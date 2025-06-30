// src/iw44/coeff_map.rs
use super::constants::{ZIGZAG_LOC, IW_SHIFT};
use super::transform;
use image::{GrayImage, Luma};
use std::collections::HashMap;

/// Replaces `IW44Image::Block`, storing coefficients for a 32x32 image block.
/// Uses a HashMap to sparsely store buckets of 16 coefficients.
#[derive(Debug, Clone, Default)]
pub struct Block {
    // bucket_index -> 16 coefficients
    buckets: HashMap<u8, [i16; 16]>,
}

impl Block {
    pub fn read_liftblock(&mut self, liftblock: &[i16; 1024]) {
        for (i, &loc) in ZIGZAG_LOC.iter().enumerate() {
            let coeff = liftblock[loc];
            if coeff != 0 {
                let bucket_idx = (i / 16) as u8;
                let coeff_idx_in_bucket = i % 16;
                let bucket = self.buckets.entry(bucket_idx).or_insert([0; 16]);
                bucket[coeff_idx_in_bucket] = coeff;
            }
        }
    }
    
    pub fn get_bucket(&self, bucket_idx: u8) -> Option<&[i16; 16]> {
        self.buckets.get(&bucket_idx)
    }

    pub fn get_bucket_mut(&mut self, bucket_idx: u8) -> &mut [i16; 16] {
        self.buckets.entry(bucket_idx).or_insert([0; 16])
    }

    pub fn zero_bucket(&mut self, bucket_idx: u8) {
        self.buckets.remove(&bucket_idx);
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
    
    /// Create coefficients from an image. Corresponds to `Map::Encode::create`.
    pub fn create_from_image(
        img: &GrayImage,
        mask: Option<&GrayImage>,
    ) -> Self {
        let (w, h) = img.dimensions();
        let mut map = Self::new(w as usize, h as usize);

        // Allocate decomposition buffer (padded)
        let mut data16 = vec![0i16; map.bw * map.bh];

        // Copy pixels from signed GrayImage to i16 buffer, shifting up.
        for y in 0..map.ih {
            for x in 0..map.iw {
                // The C++ code uses signed char (-128 to 127). Our GrayImage from
                // color conversion also produces signed values, cast to u8.
                let pixel_val = img.get_pixel(x as u32, y as u32)[0] as i8;
                data16[y * map.bw + x] = (pixel_val as i16) << IW_SHIFT;
            }
        }
        
        // TODO: Implement the complex masking logic (`interpolate_mask`, `forward_mask`)
        if let Some(_mask_img) = mask {
            log::warn!("Masking is not yet fully implemented in this port. Encoding without mask.");
        }
        
        // Perform traditional wavelet decomposition
        transform::forward(&mut data16, map.iw, map.ih, map.bw, 1, 32);

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
                    liftblock[dst_offset..dst_offset + 32].copy_from_slice(&data16[src_offset..src_offset + 32]);
                }
                
                map.blocks[block_idx].read_liftblock(&liftblock);
            }
        }
        
        map
    }

    pub fn slash_res(&mut self, res: usize) {
        let min_bucket = match res {
            0..=1 => return,
            2..=3 => 16,
            4..=7 => 4,
            _ => 1,
        };
        for block in self.blocks.iter_mut() {
            for buckno in min_bucket..64 {
                block.zero_bucket(buckno as u8);
            }
        }
    }
}