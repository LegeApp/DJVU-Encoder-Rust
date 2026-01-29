//! DjVu-compatible JB2 encoder following the official DjVu specification
//!
//! This implements the JB2 encoding as specified in Appendix 2 of the DjVu specification,
//! producing a single Sjbz chunk with arithmetically encoded records.

use crate::encode::jb2::error::Jb2Error;
use crate::encode::jb2::num_coder::{NumCoder, NumContext, BIG_POSITIVE};
use crate::encode::jb2::symbol_dict::BitImage;
use crate::encode::zc::ZEncoder;
use std::io::Write;

// Record types as per DjVu specification Table 6
const START_OF_DATA: i32 = 0;
const NEW_MARK: i32 = 1;
#[allow(dead_code)]
const NEW_MARK_LIBRARY_ONLY: i32 = 2;
#[allow(dead_code)]
const NEW_MARK_IMAGE_ONLY: i32 = 3;
#[allow(dead_code)]
const MATCHED_REFINE: i32 = 4;
#[allow(dead_code)]
const MATCHED_REFINE_LIBRARY_ONLY: i32 = 5;
#[allow(dead_code)]
const MATCHED_REFINE_IMAGE_ONLY: i32 = 6;
#[allow(dead_code)]
const MATCHED_COPY: i32 = 7;
const NON_MARK_DATA: i32 = 8;
#[allow(dead_code)]
const REQUIRED_DICT_OR_RESET: i32 = 9;
#[allow(dead_code)]
const PRESERVED_COMMENT: i32 = 10;
const END_OF_DATA: i32 = 11;

/// DjVu-compatible JB2 encoder matching DjVuLibre's exact algorithm.
pub struct DjvuJb2Encoder<W: Write> {
    _writer: W,
    image_width: u32,
    image_height: u32,
    // Number coder with tree structure
    num_coder: NumCoder,
    // NumContext variables for different number types (matching DjVuLibre)
    dist_record_type: NumContext,
    dist_match_index: NumContext,
    abs_loc_x: NumContext,
    abs_loc_y: NumContext,
    abs_size_x: NumContext,
    abs_size_y: NumContext,
    image_size_dist: NumContext,
    // Bit contexts for direct bitmap coding (1024 contexts)
    bitdist: [u8; 1024],
    // Bit context for refinement flag
    dist_refinement_flag: u8,
    // State
    gotstartrecordp: bool,
}

impl<W: Write> DjvuJb2Encoder<W> {
    /// Create a new DjVu JB2 encoder
    pub fn new(writer: W) -> Self {
        Self {
            _writer: writer,
            image_width: 0,
            image_height: 0,
            num_coder: NumCoder::new(),
            dist_record_type: 0,
            dist_match_index: 0,
            abs_loc_x: 0,
            abs_loc_y: 0,
            abs_size_x: 0,
            abs_size_y: 0,
            image_size_dist: 0,
            bitdist: [0; 1024],
            dist_refinement_flag: 0,
            gotstartrecordp: false,
        }
    }

    /// Encode a bitmap as a single-page DjVu JB2 stream
    pub fn encode_single_page(&mut self, image: &BitImage) -> Result<Vec<u8>, Jb2Error> {
        self.image_width = image.width as u32;
        self.image_height = image.height as u32;

        let buffer = Vec::new();

        // Create ZP encoder (djvu_compat = true for JB2)
        // Pass buffer by value so we can get it back from finish()
        let mut zc = ZEncoder::new(buffer, true)?;

        // Encode start of image record
        self.encode_start_of_image(&mut zc)?;

        // For simplicity, encode the entire image as a single "non-symbol data" record
        self.encode_non_symbol_data(&mut zc, image, 0, 0)?;

        // Encode end of data record
        self.encode_end_of_data(&mut zc)?;

        // Flush the encoder and get the buffer back
        let buffer = zc.finish()?;

        Ok(buffer)
    }

    /// Encode start of image record (record type 0)
    fn encode_start_of_image(
        &mut self,
        zc: &mut ZEncoder<Vec<u8>>,
    ) -> Result<(), Jb2Error> {
        // Encode record type
        self.num_coder.code_num(
            zc,
            &mut self.dist_record_type,
            START_OF_DATA,
            END_OF_DATA,
            START_OF_DATA,
        )?;

        // Encode image size: WIDTH then HEIGHT
        self.num_coder.code_num(
            zc,
            &mut self.image_size_dist,
            0,
            BIG_POSITIVE,
            self.image_width as i32,
        )?;
        self.num_coder.code_num(
            zc,
            &mut self.image_size_dist,
            0,
            BIG_POSITIVE,
            self.image_height as i32,
        )?;

        // Encode eventual image refinement flag (0 = no refinement)
        zc.encode(false, &mut self.dist_refinement_flag)?;

        self.gotstartrecordp = true;

        Ok(())
    }

    /// Encode non-symbol data record (record type 8)
    fn encode_non_symbol_data(
        &mut self,
        zc: &mut ZEncoder<Vec<u8>>,
        bitmap: &BitImage,
        abs_x: i32,
        abs_y: i32,
    ) -> Result<(), Jb2Error> {
        if !self.gotstartrecordp {
            return Err(Jb2Error::InvalidState("No start record".to_string()));
        }

        // Encode record type
        self.num_coder.code_num(
            zc,
            &mut self.dist_record_type,
            START_OF_DATA,
            END_OF_DATA,
            NON_MARK_DATA,
        )?;

        // Encode absolute symbol size
        self.num_coder.code_num(
            zc,
            &mut self.abs_size_x,
            0,
            BIG_POSITIVE,
            bitmap.width as i32,
        )?;
        self.num_coder.code_num(
            zc,
            &mut self.abs_size_y,
            0,
            BIG_POSITIVE,
            bitmap.height as i32,
        )?;

        // Encode bitmap by direct coding (matching DjVuLibre's code_bitmap_directly)
        self.encode_bitmap_directly(zc, bitmap)?;

        // Encode absolute location (1-based as per DjVuLibre)
        self.num_coder.code_num(
            zc,
            &mut self.abs_loc_x,
            1,
            self.image_width as i32,
            abs_x + 1,
        )?;
        // For NON_MARK_DATA, top = bottom + rows - 1 + 1 (adjusted for 1-based)
        let top = abs_y + bitmap.height as i32;
        self.num_coder.code_num(
            zc,
            &mut self.abs_loc_y,
            1,
            self.image_height as i32,
            top,
        )?;

        Ok(())
    }

    /// Encode end of data record (record type 11)
    fn encode_end_of_data(
        &mut self,
        zc: &mut ZEncoder<Vec<u8>>,
    ) -> Result<(), Jb2Error> {
        // Encode record type only
        self.num_coder.code_num(
            zc,
            &mut self.dist_record_type,
            START_OF_DATA,
            END_OF_DATA,
            END_OF_DATA,
        )?;
        Ok(())
    }

    /// Encode bitmap using direct coding with 10-bit context template.
    /// This matches DjVuLibre's code_bitmap_directly() exactly.
    fn encode_bitmap_directly(
        &mut self,
        zc: &mut ZEncoder<Vec<u8>>,
        bitmap: &BitImage,
    ) -> Result<(), Jb2Error> {
        let dw = bitmap.width as i32;
        let dh = bitmap.height as i32;

        // DjVuLibre scans from top row (dy = rows-1) down to bottom (dy = 0)
        // But first we need to set up row pointers with border padding

        // Create padded row access (simulating GBitmap's minborder(3))
        // We'll create a simple wrapper that returns 0 for out-of-bounds
        // NOTE: Flip Y coordinate because DjVu uses bottom-left origin (y=0 at bottom)
        // while BitImage uses top-left origin (y=0 at top)
        let get_pixel = |x: i32, y: i32| -> u8 {
            if x < 0 || y < 0 || x >= dw || y >= dh {
                0
            } else {
                // Flip Y: DjVu y=0 is at bottom, BitImage y=0 is at top
                let flipped_y = dh - 1 - y;
                bitmap.get_pixel_unchecked(x as usize, flipped_y as usize) as u8
            }
        };

        // Iterate from top row down (DjVuLibre order)
        for dy in (0..dh).rev() {
            // Get initial context for this row
            let mut context = self.get_direct_context(&get_pixel, 0, dy);

            for dx in 0..dw {
                // Get pixel value
                let n = get_pixel(dx, dy);

                // Encode the pixel
                zc.encode(n != 0, &mut self.bitdist[context])?;

                // Shift context for next pixel
                if dx + 1 < dw {
                    context = self.shift_direct_context(context, n, &get_pixel, dx + 1, dy);
                }
            }
        }

        Ok(())
    }

    /// Get the direct context for position (x, y).
    /// This matches DjVuLibre's get_direct_context() exactly.
    fn get_direct_context<F>(&self, get_pixel: &F, x: i32, y: i32) -> usize
    where
        F: Fn(i32, i32) -> u8,
    {
        // DjVuLibre uses up2, up1, up0 where up0 is current row, up1 is row above, up2 is 2 rows above
        // Since we're scanning top-down, "up" means higher y values
        // up2 = y + 2, up1 = y + 1, up0 = y
        let up2_y = y + 2;
        let up1_y = y + 1;
        // up0_y = y (current row)

        // Template positions (column offsets relative to current x):
        // up2: [x-1, x, x+1] -> bits [9, 8, 7]
        // up1: [x-2, x-1, x, x+1, x+2] -> bits [6, 5, 4, 3, 2]
        // up0: [x-2, x-1] -> bits [1, 0]

        ((get_pixel(x - 1, up2_y) as usize) << 9)
            | ((get_pixel(x, up2_y) as usize) << 8)
            | ((get_pixel(x + 1, up2_y) as usize) << 7)
            | ((get_pixel(x - 2, up1_y) as usize) << 6)
            | ((get_pixel(x - 1, up1_y) as usize) << 5)
            | ((get_pixel(x, up1_y) as usize) << 4)
            | ((get_pixel(x + 1, up1_y) as usize) << 3)
            | ((get_pixel(x + 2, up1_y) as usize) << 2)
            | ((get_pixel(x - 2, y) as usize) << 1)
            | ((get_pixel(x - 1, y) as usize) << 0)
    }

    /// Shift the direct context for the next pixel.
    /// This matches DjVuLibre's shift_direct_context() exactly.
    fn shift_direct_context<F>(
        &self,
        context: usize,
        next: u8,
        get_pixel: &F,
        x: i32,
        y: i32,
    ) -> usize
    where
        F: Fn(i32, i32) -> u8,
    {
        let up2_y = y + 2;
        let up1_y = y + 1;

        // Shift and bring in new bits
        // ((context << 1) & 0x37a) preserves bits [9,8,6,5,4,3,1] shifted left
        // Then we add: up1[x+2] at bit 2, up2[x+1] at bit 7, next at bit 0
        ((context << 1) & 0x37a)
            | ((get_pixel(x + 2, up1_y) as usize) << 2)
            | ((get_pixel(x + 1, up2_y) as usize) << 7)
            | (next as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_pattern_encoding() {
        // Create a simple 10x10 pattern
        let mut image = BitImage::new(10, 10).unwrap();

        // Set center pixel
        image.set_usize(5, 5, true);

        // Create encoder
        let mut encoder = DjvuJb2Encoder::new(Vec::new());

        // Encode
        let result = encoder.encode_single_page(&image);
        assert!(result.is_ok());

        let data = result.unwrap();
        assert!(!data.is_empty());
        println!("Encoded {} bytes for 10x10 single pixel", data.len());
    }

    #[test]
    fn test_all_black_pattern() {
        // Create a 8x8 all-black pattern
        let mut image = BitImage::new(8, 8).unwrap();
        for y in 0..8 {
            for x in 0..8 {
                image.set_usize(x, y, true);
            }
        }

        let mut encoder = DjvuJb2Encoder::new(Vec::new());
        let result = encoder.encode_single_page(&image);
        assert!(result.is_ok());

        let data = result.unwrap();
        println!("Encoded {} bytes for 8x8 all-black", data.len());
    }

    #[test]
    fn test_checkerboard_pattern() {
        // Create a 16x16 checkerboard
        let mut image = BitImage::new(16, 16).unwrap();
        for y in 0..16 {
            for x in 0..16 {
                if (x + y) % 2 == 0 {
                    image.set_usize(x, y, true);
                }
            }
        }

        let mut encoder = DjvuJb2Encoder::new(Vec::new());
        let result = encoder.encode_single_page(&image);
        assert!(result.is_ok());

        let data = result.unwrap();
        println!("Encoded {} bytes for 16x16 checkerboard", data.len());
    }
}
