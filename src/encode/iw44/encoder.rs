// src/iw44/encoder.rs

use super::coeff_map::CoeffMap;
use super::codec::Codec;
use super::constants::DECIBEL_PRUNE;
use super::transform;
use std::io::Write;

use crate::encode::zp::ZPCodec;
use crate::encode::huffman::{HuffmanDecoder, HuffmanEncoder};

use image::{GrayImage, RgbImage};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum EncoderError {
    #[error("No stopping condition was provided for the encoder.")]
    NeedStopCondition,
    #[error("The image to encode is empty or invalid.")]
    EmptyObject,
}

#[derive(Debug, Clone, Copy)]
pub enum CrcbMode {
    None,   // Grayscale only
    Half,   // Chrominance at half resolution
    Normal, // Full resolution with delay
    Full,   // Full resolution, no delay
}

#[derive(Debug, Clone, Copy, Default)]
pub struct EncoderParams {
    pub slices: Option<usize>,
    pub bytes: Option<usize>,
    pub decibels: Option<f32>,
    pub crcb_mode: CrcbMode,
    pub db_frac: f32,
}

pub struct IWEncoder {
    y_codec: Codec,
    cb_codec: Option<Codec>,
    cr_codec: Option<Codec>,
    params: EncoderParams,
    
    // Encoder state
    total_slices: usize,
    total_bytes: usize,
    crcb_delay: isize,
}

impl IWEncoder {
    pub fn from_rgb(img: &RgbImage, mask: Option<&GrayImage>, params: EncoderParams) -> Self {
        let (w, h) = img.dimensions();

        let (crcb_delay, crcb_half) = match params.crcb_mode {
            CrcbMode::None => (-1, true),
            CrcbMode::Half => (10, true),
            CrcbMode::Normal => (10, false),
            CrcbMode::Full => (0, false),
        };
        
        let y_img = transform::rgb_to_y(img);
        let mut y_map = CoeffMap::create_from_image(&y_img, mask);
        let y_codec = Codec::new(y_map);

        let (cb_codec, cr_codec) = if crcb_delay >= 0 {
            let cb_img = transform::rgb_to_cb(img);
            let cr_img = transform::rgb_to_cr(img);
            let mut cb_map = CoeffMap::create_from_image(&cb_img, mask);
            let mut cr_map = CoeffMap::create_from_image(&cr_img, mask);
            if crcb_half {
                cb_map.slash_res(2);
                cr_map.slash_res(2);
            }
            (Some(Codec::new(cb_map)), Some(Codec::new(cr_map)))
        } else {
            (None, None)
        };
        
        Self {
            y_codec,
            cb_codec,
            cr_codec,
            params,
            total_slices: 0,
            total_bytes: 0,
            crcb_delay,
        }
    }

    pub fn from_grayscale(img: &GrayImage, mask: Option<&GrayImage>, params: EncoderParams) -> Self {
        // Prepare gray level conversion (0-255 -> -128 to 127)
        let mut signed_img = GrayImage::new(img.width(), img.height());
        for (x,y,p) in img.enumerate_pixels() {
            signed_img.put_pixel(x, y, image::Luma([ (p[0] as i16 - 128) as u8 ]));
        }

        let y_map = CoeffMap::create_from_image(&signed_img, mask);
        let y_codec = Codec::new(y_map);
        Self {
            y_codec,
            cb_codec: None,
            cr_codec: None,
            params,
            total_slices: 0,
            total_bytes: 0,
            crcb_delay: -1,
        }
    }

    /// Encodes one data chunk. Returns `false` if encoding is complete.
    pub fn encode_chunk(&mut self) -> Result<(Vec<u8>, bool), EncoderError> {
        if self.params.slices.is_none() && self.params.bytes.is_none() && self.params.decibels.is_none() {
            return Err(EncoderError::NeedStopCondition);
        }

        let mut zp = ZPCodec::create_encoder();
        let mut flag = true;
        let mut slices_in_chunk = 0;
        let mut est_db = -1.0;

        while flag {
            // Check stopping conditions
            if let Some(target_db) = self.params.decibels {
                if est_db >= target_db { break; }
            }
            if let Some(target_bytes) = self.params.bytes {
                // This is a rough check; real check would need current ZP buffer size
                if self.total_bytes >= target_bytes { break; }
            }
            if let Some(target_slices) = self.params.slices {
                if self.total_slices >= target_slices { break; }
            }

            flag = self.y_codec.encode_slice(&mut zp);
            
            if let (Some(cb_codec), Some(cr_codec)) = (self.cb_codec.as_mut(), self.cr_codec.as_mut()) {
                if self.total_slices as isize >= self.crcb_delay {
                    flag |= cb_codec.encode_slice(&mut zp);
                    flag |= cr_codec.encode_slice(&mut zp);
                }
            }
            
            slices_in_chunk += 1;
            self.total_slices += 1;

            if let Some(target_db) = self.params.decibels {
                if flag && (self.y_codec.cur_band == 0 || est_db >= target_db - DECIBEL_PRUNE) {
                    est_db = self.y_codec.estimate_decibel(self.params.db_frac);
                }
            }
        }
        
        let chunk_data = zp.finish();
        self.total_bytes += chunk_data.len(); // Plus headers, which we omit here.

        Ok((chunk_data, flag))
    }
}