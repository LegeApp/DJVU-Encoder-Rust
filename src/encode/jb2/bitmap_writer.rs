// src/jb2/bitmap_writer.rs

use std::io::Write;

// use crate::encode::jb2::context::Context;  // Comment out until Context is properly defined

use crate::encode::jb2::arithmetic_coder::ArithmeticEncoder;

use image::GrayImage;
use crate::encode::jb2::error::Jb2Error;
// use crate::encode::jb2::context::{  // Fixed path
//     get_direct_context, shift_direct_context,
//     get_cross_context, shift_cross_context,
//     Bitmap, Context,
// };  // Comment out until these are defined
use crate::encode::jb2::encoder::Jb2Encoder;

pub struct BitmapWriter<W: Write> {
    zp: ArithmeticEncoder<W>,
    bitdist: [u32; 256],
    cbitdist: [u32; 256],
}

impl Jb2Encoder<std::io::Cursor<Vec<u8>>> {
    /// Direct-mode bitmap coder: port of `Encode::code_bitmap_directly`.
    pub fn code_bitmap_directly<W: Write>(
        &mut self,
        bitmap: &GrayImage,
    ) -> Result<(), Jb2Error> {
        let dw = bitmap.width() as usize;
        let dh = bitmap.height() as usize;

        // Zero row for initial padding
        let zero_row = vec![0u8; dw];
        let mut up2: &[u8] = &zero_row;
        let mut up1: &[u8] = &zero_row;
        let mut dy = dh as isize - 1;

        while dy >= 0 {
            // Build up0 row from image
            let mut row0 = Vec::with_capacity(dw);
            for x in 0..dw {
                row0.push(bitmap.get_pixel(x as u32, dy as u32).0[0]);
            }
            let up0: &[u8] = &row0;

            // Encode this row
            let mut context = get_direct_context(up2, up1, up0, 0);
            for dx in 0..dw {
                let bit = up0[dx] != 0;
                self.zp.emit(bit, &mut self.bitdist[context])?;
                context = shift_direct_context(context,
                    if bit {1} else {0}, up2, up1, up0, dx + 1);
            }

            // Advance scanlines
            dy -= 1;
            up2 = up1;
            up1 = up0;
        }

        Ok(())
    }

    /// Cross-coding bitmap coder: port of `Encode::code_bitmap_by_cross_coding`.
    pub fn code_bitmap_cross<W: Write>(
        &mut self,
        bitmap: &GrayImage,
        lib_bitmap: &GrayImage,
        xd2c: isize,
        mut cy: isize,
    ) -> Result<(), Jb2Error> {
        let dw = bitmap.width() as usize;
        let dh = bitmap.height() as usize;

        // Prepare zero row and initial pointers
        let zero_row = vec![0u8; dw];
        let mut up1 = zero_row.clone();
        let mut up0 = (0..dw)
            .map(|x| bitmap.get_pixel(x as u32, (dh - 1) as u32).0[0])
            .collect::<Vec<u8>>();
        let mut xup1 = zero_row.clone();
        let mut xup0 = (0..dw)
            .map(|x| {
                if cy >= 0 && (cy as u32) < lib_bitmap.height() {
                    let xx = (x as isize + xd2c) as u32;
                    lib_bitmap.get_pixel(xx, cy as u32).0[0]
                } else { 0 }
            })
            .collect::<Vec<u8>>();
        let mut xdn1 = (0..dw)
            .map(|x| {
                if cy > 0 && ((cy - 1) as u32) < lib_bitmap.height() {
                    let xx = (x as isize + xd2c) as u32;
                    lib_bitmap.get_pixel(xx, (cy - 1) as u32).0[0]
                } else { 0 }
            })
            .collect::<Vec<u8>>();

        let mut dy = dh as isize - 1;
        while dy >= 0 {
            // Encode this row
            let mut context = get_cross_context(
                &up1, &up0, &xup1, &xup0, &xdn1, 0);
            for dx in 0..dw {
                let bit = up0[dx] != 0;
                self.zp.emit(bit, &mut self.cbitdist[context])?;
                context = shift_cross_context(
                    context,
                    if bit {1} else {0},
                    &up1, &up0, &xup1, &xup0, &xdn1,
                    dx + 1,
                );
            }

            // Advance scanlines
            dy -= 1;
            up1 = up0;
            up0 = (0..dw)
                .map(|x| bitmap.get_pixel(x as u32, dy as u32).0[0])
                .collect();
            xup1 = xup0;
            xup0 = xdn1;
            cy -= 1;
            xdn1 = (0..dw)
                .map(|x| {
                    if cy > 0 && ((cy - 1) as u32) < lib_bitmap.height() {
                        let xx = (x as isize + xd2c) as u32;
                        lib_bitmap.get_pixel(xx, (cy - 1) as u32).0[0]
                    } else { 0 }
                })
                .collect();
        }

        Ok(())
    }
}
