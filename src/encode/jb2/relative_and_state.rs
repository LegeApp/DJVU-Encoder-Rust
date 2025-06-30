// src/jb2/relative_and_state.rs

use std::io::Write;
use crate::encode::zp::zp_codec::ZpEncoder;
use crate::encode::jb2::error::Jb2Error;
use crate::encode::jb2::num_coder::{NumCoder, BIG_NEGATIVE, BIG_POSITIVE};
use crate::encode::jb2::encoder::{RecordType, Jb2Encoder};
use image::GrayImage;

impl<W: Write> Jb2Encoder<W> {
    // === 4. Relative-location predictor ===

    /// Update the short-list ring buffer with a new delta.
    pub fn update_short_list(&mut self, delta: i32) {
        self.rel_loc_idx = (self.rel_loc_idx + 1) % self.rel_loc_short_list.len();
        self.rel_loc_short_list[self.rel_loc_idx] = delta;
    }

    /// Code the relative location of a new shape at (x, y).
    pub fn code_relative_location(
        &mut self,
        num_coder: &mut NumCoder,
        zp: &mut ZpEncoder<W>,
        x: i32,
        y: i32,
    ) -> Result<(), Jb2Error> {
        // Branch: same row vs new row
        let same_row = y == self.last_y;
        zp.encode(same_row, &mut self.bitdist[self.ctx_rel_loc_same_row])?;

        if same_row {
            // Delta X on same row
            let dx = x - self.last_x;
            num_coder.code_num(zp, dx, BIG_NEGATIVE, BIG_POSITIVE, &mut self.ctx_rel_loc_x_current)?;
            let last_dx = self.rel_loc_short_list[self.rel_loc_idx];
            num_coder.code_num(zp, dx - last_dx, BIG_NEGATIVE, BIG_POSITIVE, &mut self.ctx_rel_loc_x_last)?;
            self.update_short_list(dx);
        } else {
            // New row: Y then X
            let dy = y - self.last_y;
            num_coder.code_num(zp, dy, BIG_NEGATIVE, BIG_POSITIVE, &mut self.ctx_rel_loc_y_current)?;
            let last_dy = self.rel_loc_short_list[self.rel_loc_idx];
            num_coder.code_num(zp, dy - last_dy, BIG_NEGATIVE, BIG_POSITIVE, &mut self.ctx_rel_loc_y_last)?;
            // Absolute X on new row
            num_coder.code_num(zp, x, 0, BIG_POSITIVE, &mut self.ctx_rel_loc_x_current)?;
            self.update_short_list(dy);
        }

        self.last_x = x;
        self.last_y = y;
        Ok(())
    }

    // === 5. Record-type state machine ===

    /// Top-level record emitter, mirroring the C++ switch over RecordType.
    pub fn code_record(
        &mut self,
        num_coder: &mut NumCoder,
        zp: &mut ZpEncoder<W>,
        rec: RecordType,
    ) -> Result<(), Jb2Error> {
        // Emit the record-type codeword
        self.code_record_type(rec)?;

        match rec {
            // Image-only variants: bitmap only
            RecordType::NewImage | RecordType::NewRefineImage => {
                let (bitmap, lib_bitmap, xd2c, cy) = self.next_image();
                self.code_bitmap_directly(&bitmap)?;
            }

            // Library-only variants: only add to library
            RecordType::NewMarkLibraryOnly | RecordType::MatchedRefineLibraryOnly => {
                let (shape_index, bitmap) = self.next_shape();
                self.add_to_library(shape_index, &bitmap)?;
            }

            // Combined: cross-coded bitmap + add to library
            _ => {
                let (bitmap, lib_bitmap, xd2c, cy) = self.next_image();
                self.code_bitmap_cross(&bitmap, &lib_bitmap, xd2c, cy)?;
                let (shape_index, bitmap) = self.next_shape();
                self.add_to_library(shape_index, &bitmap)?;
            }
        }

        Ok(())
    }
}
