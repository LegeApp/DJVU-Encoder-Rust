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
    pub slices: Option<usize>, // maximum number of wavelet slices
    pub bytes: Option<usize>,  // maximum total bytes (including headers)
    pub decibels: Option<f32>, // target SNR in dB
    pub crcb_mode: CrcbMode,   // chroma handling
    pub db_frac: f32,          // decibel update fraction
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
}

impl IWEncoder {
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
        })
    }

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
        })
    }

    /// Produce one BM44/PM44 chunk (with headers). Returns `(chunk_bytes, more_chunks?)`.
    pub fn encode_chunk(&mut self) -> Result<(Vec<u8>, bool), EncoderError> {
        // require at least one stopping condition
        if self.params.slices.is_none()
            && self.params.bytes.is_none()
            && self.params.decibels.is_none()
        {
            return Err(EncoderError::NeedStopCondition);
        }

        // check image non‐empty
        let (w, h) = {
            let map = &self.y_codec.map;
            let w = map.width();
            let h = map.height();
            if w == 0 || h == 0 {
                return Err(EncoderError::EmptyObject);
            }
            (w, h)
        };

        // setup arithmetic coder
        let mut zp: ZpEncoder<std::io::Cursor<Vec<u8>>> = ZpEncoder::new(std::io::Cursor::new(Vec::new()), true)?;        
        let mut more = true;
        let mut est_db = -1.0;

        let mut loop_count = 0;
        
        // process slices until a stop condition trips
        while more {
            loop_count += 1;
            
            // decibel stop
            if let Some(db_target) = self.params.decibels {
                if est_db >= db_target {
                    more = false;
                    break;
                }
            }
            // byte stop (approximate: exclude header size)
            if let Some(byte_target) = self.params.bytes {
                if self.total_bytes >= byte_target {
                    more = false;
                    break;
                }
            }
            // slice count stop
            if let Some(slice_target) = self.params.slices {
                if self.total_slices >= slice_target {
                    more = false;
                    break;
                }
            }

            // encode Y slice - use actual encoding
            more = self.y_codec.encode_slice(&mut zp)?;
            
            // encode Cb/Cr slice if present
            if let (Some(ref mut cb), Some(ref mut cr)) = (self.cb_codec.as_mut(), self.cr_codec.as_mut()) {
                more |= cb.encode_slice(&mut zp)?;
                more |= cr.encode_slice(&mut zp)?;
            }

            self.total_slices += 1;

            // update estimated dB if needed
            if let Some(db_target) = self.params.decibels {
                if more && (self.y_codec.cur_band == 0 || est_db >= db_target - DECIBEL_PRUNE) {
                    est_db = self.y_codec.estimate_decibel(self.params.db_frac);
                }
            }
            
            // Safety check to prevent infinite loops
            let max_slices = self.params.max_slices.unwrap_or(10_000);
            if loop_count > max_slices {
                eprintln!("Warning: Slice cap {} reached, aborting chunk early", max_slices);
                more = false;
                break;
            }
        }

        // finish arithmetic payload
        let payload = zp.finish()?.into_inner();
        let payload_len = payload.len();

        // The encoder's job is to produce the IW44 stream (secondary header + payload).
        // The caller is responsible for wrapping this in a BG44/FG44 IFF chunk.
        let is_pm44 = self.cb_codec.is_some();
        let mut iw44_stream = Vec::with_capacity(payload_len + 4);

        // Write secondary header (4 bytes)
        // Order: slice_count (1), version (1), chunk_len (2)
        let version = 0x02u8;
        let slice_count = if is_pm44 { 3 } else { 1 };
        // The chunk_len in the secondary header is the payload size ONLY.
        let secondary_chunk_len = payload_len as u16;

        iw44_stream.push(slice_count as u8);
        iw44_stream.push(version);
        iw44_stream.extend_from_slice(&secondary_chunk_len.to_be_bytes());

        // Append the actual compressed data
        iw44_stream.extend_from_slice(&payload);

        self.total_bytes += iw44_stream.len();

        Ok((iw44_stream, more))
    }
}
