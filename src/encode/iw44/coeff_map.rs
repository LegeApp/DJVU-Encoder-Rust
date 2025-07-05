use super::constants::ZIGZAG_LOC;
use super::masking;
use super::transform::Encode;
use ::image::GrayImage;

/// Replaces `IW44Image::Block`, storing coefficients for a 32x32 image block.
/// Uses fixed arrays instead of HashMap for maximum performance.
#[derive(Debug, Clone)]
pub struct Block {
    // 64 optional buckets (1024 coeffs / 16 per bucket); None == bucket all-zero
    buckets: [Option<[i16; 16]>; 64],
}

impl Default for Block {
    fn default() -> Self {
        Self {
            buckets: [None; 64],
        }
    }
}

impl Block {
    pub fn read_liftblock(&mut self, liftblock: &[i16; 1024]) {
        for (i, &loc) in ZIGZAG_LOC.iter().enumerate() {
            let coeff = liftblock[loc as usize];
            if coeff != 0 {
                let bucket_idx = (i / 16) as u8;
                let coeff_idx_in_bucket = i % 16;

                // Ensure bucket exists
                if self.buckets[bucket_idx as usize].is_none() {
                    self.buckets[bucket_idx as usize] = Some([0; 16]);
                }

                self.buckets[bucket_idx as usize].as_mut().unwrap()[coeff_idx_in_bucket] = coeff;
            }
        }
    }

    #[inline]
    pub fn get_bucket(&self, bucket_idx: u8) -> Option<&[i16; 16]> {
        self.buckets[bucket_idx as usize].as_ref()
    }

    #[inline]
    pub fn get_bucket_mut(&mut self, bucket_idx: u8) -> &mut [i16; 16] {
        if self.buckets[bucket_idx as usize].is_none() {
            self.buckets[bucket_idx as usize] = Some([0; 16]);
        }
        self.buckets[bucket_idx as usize].as_mut().unwrap()
    }

    pub fn zero_bucket(&mut self, bucket_idx: u8) {
        self.buckets[bucket_idx as usize] = None;
    }

    /// Set a bucket directly (used for encoded map)
    #[inline]
    pub fn set_bucket(&mut self, bucket_idx: u8, val: [i16; 16]) {
        self.buckets[bucket_idx as usize] = Some(val);
    }
}

/// Replaces `IW44Image::Map`. Owns all the coefficient blocks for one image component (Y, Cb, or Cr).
#[derive(Debug, Clone)]
pub struct CoeffMap {
    pub blocks: Vec<Block>,
    pub iw: usize, // Image width
    pub ih: usize, // Image height
    pub bw: usize, // Padded block width
    pub bh: usize, // Padded block height
    pub num_blocks: usize,
}

impl CoeffMap {
    pub fn new(width: usize, height: usize) -> Self {
        let bw = (width + 31) & !31;
        let bh = (height + 31) & !31;
        let num_blocks = (bw * bh) / (32 * 32);
        CoeffMap {
            blocks: vec![Block::default(); num_blocks],
            iw: width,
            ih: height,
            bw,
            bh,
            num_blocks,
        }
    }

    pub fn width(&self) -> usize {
        self.iw
    }

    pub fn height(&self) -> usize {
        self.ih
    }

    /// Private helper to copy a 32x32 block from the transform buffer to a liftblock
    fn copy_block_data(
        liftblock: &mut [i16; 1024], 
        data16: &[i16], 
        bw: usize, 
        block_x: usize, 
        block_y: usize
    ) {
        let data_start_x = block_x * 32;
        let data_start_y = block_y * 32;

        for i in 0..32 {
            let src_y = data_start_y + i;
            let src_offset = src_y * bw + data_start_x;
            let dst_offset = i * 32;
            liftblock[dst_offset..dst_offset + 32]
                .copy_from_slice(&data16[src_offset..src_offset + 32]);
        }
    }

    /// Private helper that does the core work: allocate buffer, transform, populate blocks
    fn create_from_transform<F>(
        width: usize,
        height: usize,
        mask: Option<&GrayImage>,
        transform_fn: F
    ) -> Self 
    where F: FnOnce(&mut [i16], usize, usize, usize)
    {
        let mut map = Self::new(width, height);
        
        // Allocate decomposition buffer (padded)
        let mut data16 = vec![0i16; map.bw * map.bh];

        // Apply transform function to populate data16
        transform_fn(&mut data16, map.iw, map.ih, map.bw);

        // Apply masking logic if mask is provided
        if let Some(mask_img) = mask {
            // Convert mask image to signed i8 array using masking helper
            let mask8 = masking::image_to_mask8(mask_img, map.bw, map.ih);
            
            // Apply interpolate_mask to fill masked pixels with neighbor averages
            masking::interpolate_mask(&mut data16, map.iw, map.ih, map.bw, &mask8, map.bw);

            // Apply forward_mask for multiscale masked wavelet decomposition
            masking::forward_mask(&mut data16, map.iw, map.ih, map.bw, 1, 32, &mask8, map.bw);
        }

        // Copy transformed coefficients into blocks
        let blocks_w = map.bw / 32;
        for block_y in 0..(map.bh / 32) {
            for block_x in 0..blocks_w {
                let block_idx = block_y * blocks_w + block_x;
                let mut liftblock = [0i16; 1024];

                Self::copy_block_data(&mut liftblock, &data16, map.bw, block_x, block_y);
                map.blocks[block_idx].read_liftblock(&liftblock);
            }
        }

        map
    }

    /// Create coefficients from an image. Corresponds to `Map::Encode::create`.
    pub fn create_from_image(img: &GrayImage, mask: Option<&GrayImage>) -> Self {
        let (w, h) = img.dimensions();
        Self::create_from_transform(w as usize, h as usize, mask, |data16, w, h, bw| {
            Encode::from_u8_image(img, data16, w, h, bw);
        })
    }

    /// Create a CoeffMap from signed Y channel data (centered around 0)
    pub fn create_from_signed_y_buffer(
        y_buf: &[i8], 
        width: u32, 
        height: u32, 
        mask: Option<&GrayImage>
    ) -> Self {
        Self::create_from_transform(width as usize, height as usize, mask, |data16, w, h, bw| {
            Encode::from_i8_channel(y_buf, data16, w, h, bw);
        })
    }

    /// Create a CoeffMap from signed i8 channel data (Y, Cb, or Cr)
    /// The input data should be centered around 0 (range approximately -128 to +127)
    pub fn create_from_signed_channel(
        channel_buf: &[i8], 
        width: u32, 
        height: u32, 
        mask: Option<&GrayImage>,
        _channel_name: &str  // Keep for API compatibility but don't use for debug
    ) -> Self {
        Self::create_from_transform(width as usize, height as usize, mask, |data16, w, h, bw| {
            Encode::from_i8_channel(channel_buf, data16, w, h, bw);
        })
    }

    pub fn slash_res(&mut self, res: usize) {
        // Halve the image dimensions
        self.iw = (self.iw + res - 1) / res;
        self.ih = (self.ih + res - 1) / res;
        // Update padded dimensions
        self.bw = (self.iw + 31) & !31;
        self.bh = (self.ih + 31) & !31;
        // Update number of blocks
        self.num_blocks = (self.bw * self.bh) / (32 * 32);

        let min_bucket = match res {
            0..=1 => return,
            2..=3 => 16,
            4..=7 => 4,
            _ => 1,
        };
        // Adjust blocks vector size
        self.blocks.resize(self.num_blocks, Block::default());

        for block in self.blocks.iter_mut() {
            for buckno in min_bucket..64 {
                block.zero_bucket(buckno as u8);
            }
        }
    }
}
