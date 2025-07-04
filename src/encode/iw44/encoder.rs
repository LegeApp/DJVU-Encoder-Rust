// src/iw44/encoder.rs
//! Production-ready IW44 encoder: chunked ZP coding with automatic headers,
//! slice/byte/decibel stopping, and optional chroma handling.

use super::codec::Codec;
use super::coeff_map::CoeffMap;
use super::constants::DECIBEL_PRUNE;
use super::transform;
use crate::encode::zp::ZpEncoder;
use ::image::{GrayImage, RgbImage};
use std::io::Cursor;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum EncoderError {
    #[error("At least one stop condition (slices, bytes, or decibels) must be set.")]
    NeedStopCondition,
    #[error("Input image is empty or invalid.")]
    EmptyObject,
    #[error("ZP codec error: {0}")]
    ZpCodec(#[from] crate::encode::zp::ZpCodecError),
}

/// Chrominance mode for IW44 encoding.
#[derive(Debug, Clone, Copy, Default)]
pub enum CrcbMode {
    #[default]
    None, // Y only
    Half,   // chroma at half resolution
    Normal, // full resolution with delay
    Full,   // full resolution, no delay
}

#[derive(Debug, Clone, Copy)]
pub struct EncoderParams {
    pub slices: Option<usize>,     // maximum number of wavelet slices
    pub bytes: Option<usize>,      // maximum total bytes (including headers)
    pub decibels: Option<f32>,     // target SNR in dB
    pub crcb_mode: CrcbMode,       // chroma handling
    pub db_frac: f32,              // decibel update fraction
    pub max_slices: Option<usize>, // absolute maximum slices to prevent infinite loops
}

impl Default for EncoderParams {
    fn default() -> Self {
        Self {
            slices: None,
            bytes: None,
            decibels: None,
            crcb_mode: CrcbMode::default(),
            db_frac: 0.9,
            max_slices: Some(1000), // Safety limit for infinite loop prevention
        }
    }
}

/// IW44 encoder that emits complete BM44/PM44 chunks with headers.
pub struct IWEncoder {
    y_codec: Codec,
    cb_codec: Option<Codec>,
    cr_codec: Option<Codec>,
    params: EncoderParams,
    // running state
    total_slices: usize,
    total_bytes: usize,
    serial: u8,
    // Shared band/bit state for synchronization
    cur_band: usize,
    cur_bit: i32,
}

impl IWEncoder {
    /// Create from a grayscale image (with optional mask).
    pub fn from_gray(
        img: &GrayImage,
        mask: Option<&GrayImage>,
        params: EncoderParams,
    ) -> Result<Self, EncoderError> {
        let ymap = CoeffMap::create_from_image(img, mask);
        let y_codec = Codec::new(ymap);

        Ok(IWEncoder {
            y_codec,
            cb_codec: None,
            cr_codec: None,
            params,
            total_slices: 0,
            total_bytes: 0,
            serial: 0,
            cur_band: 0,
            cur_bit: 13, // Start from high bit-plane
        })
    }

    /// Create from an RGB image (with optional binary mask) and parameters.
    pub fn from_rgb(
        img: &RgbImage,
        mask: Option<&GrayImage>,
        params: EncoderParams,
    ) -> Result<Self, EncoderError> {
        let (delay, half) = match params.crcb_mode {
            CrcbMode::None => (-1, true),
            CrcbMode::Half => (10, true),
            CrcbMode::Normal => (10, false),
            CrcbMode::Full => (0, false),
        };

        // Y channel
        let yplane = transform::rgb_to_y(img);
        let ymap = CoeffMap::create_from_image(&yplane, mask);
        let y_codec = Codec::new(ymap);

        // Cb/Cr channels if enabled
        let (cb_codec, cr_codec) = if delay >= 0 {
            let cbplane = transform::rgb_to_cb(img);
            let crplane = transform::rgb_to_cr(img);
            let mut cbmap = CoeffMap::create_from_image(&cbplane, mask);
            let mut crmap = CoeffMap::create_from_image(&crplane, mask);
            if half {
                cbmap.slash_res(2);
                crmap.slash_res(2);
            }
            (Some(Codec::new(cbmap)), Some(Codec::new(crmap)))
        } else {
            (None, None)
        };

        Ok(IWEncoder {
            y_codec,
            cb_codec,
            cr_codec,
            params,
            total_slices: 0,
            total_bytes: 0,
            serial: 0,
            cur_band: 0,
            cur_bit: 13, // Start from high bit-plane
        })
    }

    /// Encode one slice of data and return the IW44 stream + whether more slices remain.
    /// This should be called repeatedly until `more` is false.
    pub fn encode_chunk(&mut self) -> Result<(Vec<u8>, bool), EncoderError> {
        // require at least one stopping condition
        if self.params.slices.is_none()
            && self.params.bytes.is_none()
            && self.params.decibels.is_none()
        {
            return Err(EncoderError::NeedStopCondition);
        }

        // check image non-empty
        let (w, h) = {
            let map = &self.y_codec.map;
            let w = map.width();
            let h = map.height();
            if w == 0 || h == 0 {
                return Err(EncoderError::EmptyObject);
            }
            (w, h)
        };

        // Check if we should stop before encoding this slice
        if self.cur_bit < 0 {
            return Ok((Vec::new(), false)); // finished all bit-planes
        }

        // decibel stop
        if let Some(db_target) = self.params.decibels {
            let est_db = self.y_codec.estimate_decibel(self.params.db_frac);
            if est_db >= db_target {
                return Ok((Vec::new(), false));
            }
        }
        // byte stop (approximate: exclude header size)
        if let Some(byte_target) = self.params.bytes {
            if self.total_bytes >= byte_target {
                return Ok((Vec::new(), false));
            }
        }
        // slice count stop
        if let Some(slice_target) = self.params.slices {
            if self.total_slices >= slice_target {
                return Ok((Vec::new(), false));
            }
        }

        // Synchronize all codecs to current band/bit
        self.y_codec.cur_band = self.cur_band;
        self.y_codec.cur_bit = self.cur_bit;
        if let Some(ref mut cb) = self.cb_codec {
            cb.cur_band = self.cur_band;
            cb.cur_bit = self.cur_bit;
        }
        if let Some(ref mut cr) = self.cr_codec {
            cr.cur_band = self.cur_band;
            cr.cur_bit = self.cur_bit;
        }

        // setup arithmetic coder for this slice
        let mut zp: ZpEncoder<Cursor<Vec<u8>>> = ZpEncoder::new(Cursor::new(Vec::new()), true)?;
        let mut more = true;
        let mut actual_slice_count = 1; // Always encode Y

        // Sync all codec states with the encoder's current state BEFORE encoding
        self.y_codec.cur_band = self.cur_band;
        self.y_codec.cur_bit = self.cur_bit;
        
        if let Some(ref mut cb) = self.cb_codec {
            cb.cur_band = self.cur_band;
            cb.cur_bit = self.cur_bit;
        }
        if let Some(ref mut cr) = self.cr_codec {
            cr.cur_band = self.cur_band;
            cr.cur_bit = self.cur_bit;
        }

        // encode Y slice
        more = self.y_codec.encode_slice(&mut zp)?;

        // encode Cb/Cr slice if present (with delay logic)
        if let (Some(ref mut cb), Some(ref mut cr)) =
            (self.cb_codec.as_mut(), self.cr_codec.as_mut())
        {
            // Simple delay: only encode chroma after a few Y slices
            let crcb_delay = 10; // Delay chroma by 10 slices
            if self.total_slices >= crcb_delay {
                more |= cb.encode_slice(&mut zp)?;
                more |= cr.encode_slice(&mut zp)?;
                actual_slice_count = 3; // Y + Cb + Cr were encoded
            }
        }

        self.total_slices += 1;

        // Advance to next band/bit-plane (synchronized across all codecs)
        // Note: Don't call finish_slice() on codecs as that advances their state
        self.advance_band_bit();

        // Safety check to prevent infinite loops
        let max_slices = self.params.max_slices.unwrap_or(10_000);
        if self.total_slices > max_slices {
            eprintln!("Warning: Slice cap {} reached, aborting", max_slices);
            more = false;
        }

        // finish arithmetic payload
        let payload = zp.finish()?.into_inner();
        let payload_len = payload.len();

        // Calculate slice count for this chunk based on what was actually encoded
        let slice_count = actual_slice_count;

        // The encoder's job is to produce ONLY the raw IW44 stream (secondary header + payload).
        // The caller is responsible for wrapping this in the appropriate IFF chunk (BG44/FG44/PM44/etc.)
        let is_pm44 = self.cb_codec.is_some();
        let mut iw44_stream = Vec::with_capacity(payload_len + 9);

        // Write data header as specified in the DjVu spec
        iw44_stream.push(self.serial);
        iw44_stream.push(slice_count as u8);

        if self.serial == 0 {
            let color_bit = if is_pm44 { 0 } else { 1 };
            let major = (color_bit << 7) | 1; // major version 1
            iw44_stream.push(major);
            iw44_stream.push(2); // minor version
            iw44_stream.extend_from_slice(&(w as u16).to_be_bytes());
            iw44_stream.extend_from_slice(&(h as u16).to_be_bytes());

            let delay = match self.params.crcb_mode {
                CrcbMode::Half | CrcbMode::Normal => 10,
                _ => 0,
            } as u8;
            iw44_stream.push(0x80 | (delay & 0x7F));
        }

        iw44_stream.extend_from_slice(&payload);
        self.serial = self.serial.wrapping_add(1);

        self.total_bytes += iw44_stream.len();

        // Return whether more slices are available
        Ok((iw44_stream, self.cur_bit >= 0))
    }

    /// Advance to the next band/bit-plane in a synchronized manner
    fn advance_band_bit(&mut self) {
        println!(
            "DEBUG: Before advance: band={}, bit={}",
            self.cur_band, self.cur_bit
        );
        self.cur_band += 1;
        if self.cur_band >= super::constants::BAND_BUCKETS.len() {
            self.cur_band = 0;
            self.cur_bit -= 1;
            println!(
                "DEBUG: Advanced to new bit-plane: band={}, bit={}",
                self.cur_band, self.cur_bit
            );

            // Only update quantization thresholds if we're still encoding
            if self.cur_bit >= 0 {
                // Reduce quantization thresholds for next bit-plane (use √2 decay)
                for q in self.y_codec.quant_hi.iter_mut() {
                    *q = (*q as f32 / 1.414) as i32;
                }
                for q in self.y_codec.quant_lo.iter_mut() {
                    *q = (*q as f32 / 1.414) as i32;
                }

                // Sync chroma codecs if present
                if let Some(ref mut cb) = self.cb_codec {
                    for q in cb.quant_hi.iter_mut() {
                        *q = (*q as f32 / 1.414) as i32;
                    }
                    for q in cb.quant_lo.iter_mut() {
                        *q = (*q as f32 / 1.414) as i32;
                    }
                }
                if let Some(ref mut cr) = self.cr_codec {
                    for q in cr.quant_hi.iter_mut() {
                        *q = (*q as f32 / 1.414) as i32;
                    }
                    for q in cr.quant_lo.iter_mut() {
                        *q = (*q as f32 / 1.414) as i32;
                    }
                }
            }
        } else {
            println!(
                "DEBUG: Advanced to next band: band={}, bit={}",
                self.cur_band, self.cur_bit
            );
        }
    }
}
