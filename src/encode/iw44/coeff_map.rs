use super::constants::{IW_SHIFT, ZIGZAG_LOC};
use super::masking;
use super::transform;
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

    /// Create coefficients from an image. Corresponds to `Map::Encode::create`.
    pub fn create_from_image(img: &GrayImage, mask: Option<&GrayImage>) -> Self {
        let (w, h) = img.dimensions();
        let mut map = Self::new(w as usize, h as usize);

        // Allocate decomposition buffer (padded)
        let mut data16 = vec![0i16; map.bw * map.bh];

        // Copy pixels from GrayImage to i16 buffer, shifting up.
        // Note: The GrayImage here comes from signed Y channel data that was
        // converted back to unsigned 0-255 range, so we can use it directly.
        for y in 0..map.ih {
            for x in 0..map.iw {
                // The GrayImage contains the Y channel in 0-255 range.
                // Apply IW_SHIFT scaling as per DjVu specification.
                let pixel_u8 = img.get_pixel(x as u32, y as u32)[0] as i16;
                data16[y * map.bw + x] = pixel_u8 << IW_SHIFT;
            }
        }

        // Debug: Print some pixel values before transform
        println!("DEBUG: Before transform - first 3 pixels: {}, {}, {}", 
                 data16[0], data16[1], data16[2]);
        println!("DEBUG: Pixel shift: original Y {} -> scaled {}", 
                 img.get_pixel(0, 0)[0], data16[0]);

        // Apply masking logic if mask is provided
        if let Some(mask_img) = mask {
            // Convert mask image to signed i8 array
            let mut mask8 = vec![0i8; map.bw * map.bh];
            for y in 0..map.ih {
                for x in 0..map.iw {
                    // Non-zero mask pixels indicate masked-out regions
                    let mask_val = mask_img.get_pixel(x as u32, y as u32)[0];
                    mask8[y * map.bw + x] = if mask_val > 0 { 1 } else { 0 };
                }
            }

            // Apply interpolate_mask to fill masked pixels with neighbor averages
            masking::interpolate_mask(&mut data16, map.iw, map.ih, map.bw, &mask8, map.bw);

            // Apply forward_mask for multiscale masked wavelet decomposition
            masking::forward_mask(&mut data16, map.iw, map.ih, map.bw, 1, 32, &mask8, map.bw);
        } else {
            // Perform traditional wavelet decomposition without masking
            // Fixed: begin=0 to include finest scale (scale=1) transform
            transform::Encode::forward(&mut data16, map.iw, map.ih, map.bw, 0, 5);
        }

        // Debug: Print some coefficient values after transform
        println!("DEBUG: After transform - first 3 coeffs: {}, {}, {}", 
                 data16[0], data16[1], data16[2]);

        // Copy transformed coefficients into blocks
        let blocks_w = map.bw / 32;
        for block_y in 0..(map.bh / 32) {
            for block_x in 0..blocks_w {
                let block_idx = block_y * blocks_w + block_x;
                let mut liftblock = [0i16; 1024];

                let data_start_x = block_x * 32;
                let data_start_y = block_y * 32;

                for i in 0..32 {
                    let src_y = data_start_y + i;
                    let src_offset = src_y * map.bw + data_start_x;
                    let dst_offset = i * 32;
                    liftblock[dst_offset..dst_offset + 32]
                        .copy_from_slice(&data16[src_offset..src_offset + 32]);
                }

                map.blocks[block_idx].read_liftblock(&liftblock);

                // Debug: Print some coefficient values for the first few blocks
                if block_idx < 3 {
                    let dc_coeff = liftblock[ZIGZAG_LOC[0] as usize];
                    let low_freq_coeffs: Vec<i16> =
                        (0..4).map(|i| liftblock[ZIGZAG_LOC[i] as usize]).collect();
                    println!(
                        "DEBUG: Block {}: DC={}, low_freq={:?}",
                        block_idx, dc_coeff, low_freq_coeffs
                    );
                    
                    // Debug: Check zigzag mapping for first 16 coefficients
                    if block_idx == 0 {
                        println!("DEBUG: First 16 zigzag mappings and values:");
                        for i in 0..16 {
                            let loc = ZIGZAG_LOC[i] as usize;
                            let val = liftblock[loc];
                            println!("  zigzag[{}] -> liftblock[{}] = {}", i, loc, val);
                        }
                    }
                }
            }
        }

        #[cfg(debug_assertions)]
        println!("CoeffMap::create_from_image - Completed successfully");

        map
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
