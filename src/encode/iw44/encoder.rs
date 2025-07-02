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
    crcb_delay: isize,
    first_chunk: bool,
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
            crcb_delay: delay,
            first_chunk: true,
        })
    }

    /// Create from a grayscale image (with optional mask).
    pub fn from_gray(
        img: &GrayImage,
        mask: Option<&GrayImage>,
        params: EncoderParams,
    ) -> Result<Self, EncoderError> {
        if std::env::var("DJVU_VERBOSE_LOG").unwrap_or_default() == "1" {
            println!("IWEncoder::from_gray - Creating coefficient map...");
        }
        
        let ymap = CoeffMap::create_from_image(img, mask);
        
        if std::env::var("DJVU_VERBOSE_LOG").unwrap_or_default() == "1" {
            println!("IWEncoder::from_gray - Creating codec...");
        }
        
        let y_codec = Codec::new(ymap);
        
        if std::env::var("DJVU_VERBOSE_LOG").unwrap_or_default() == "1" {
            println!("IWEncoder::from_gray - Encoder created successfully");
        }
        
        Ok(IWEncoder {
            y_codec,
            cb_codec: None,
            cr_codec: None,
            params,
            total_slices: 0,
            total_bytes: 0,
            crcb_delay: -1,
            first_chunk: true,
        })
    }

    /// Produce one BM44/PM44 chunk (with headers). Returns `(chunk_bytes, more_chunks?)`.
    pub fn encode_chunk(&mut self) -> Result<(Vec<u8>, bool), EncoderError> {
        // Verbose logging can be enabled by setting DJVU_VERBOSE_LOG=1
        if std::env::var("DJVU_VERBOSE_LOG").unwrap_or_default() == "1" {
            println!("IWEncoder::encode_chunk - Starting...");
        }
        
        // require at least one stopping condition
        if self.params.slices.is_none()
            && self.params.bytes.is_none()
            && self.params.decibels.is_none()
        {
            return Err(EncoderError::NeedStopCondition);
        }

        if std::env::var("DJVU_VERBOSE_LOG").unwrap_or_default() == "1" {
            println!("IWEncoder::encode_chunk - Checking image dimensions...");
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

        if std::env::var("DJVU_VERBOSE_LOG").unwrap_or_default() == "1" {
            println!("IWEncoder::encode_chunk - Creating ZP codec...");
        }

        // setup arithmetic coder
        let mut zp: ZpEncoder<std::io::Cursor<Vec<u8>>> = ZpEncoder::new(std::io::Cursor::new(Vec::new()), true);        
        let mut more = true;
        let mut est_db = -1.0;

        if std::env::var("DJVU_VERBOSE_LOG").unwrap_or_default() == "1" {
            println!("IWEncoder::encode_chunk - Starting encoding loop...");
        }

        let mut loop_count = 0;
        
        // process slices until a stop condition trips
        while more {
            loop_count += 1;
            
            if std::env::var("DJVU_VERBOSE_LOG").unwrap_or_default() == "1" && loop_count % 10 == 1 {
                println!("IWEncoder::encode_chunk - Loop iteration {}, total_slices: {}, total_bytes: {}, est_db: {:.2}",
                         loop_count, self.total_slices, self.total_bytes, est_db);
            }
            
            // decibel stop
            if let Some(db_target) = self.params.decibels {
                if est_db >= db_target {
                    if std::env::var("DJVU_VERBOSE_LOG").unwrap_or_default() == "1" {
                        println!("IWEncoder::encode_chunk - Stopping due to decibel target reached");
                    }
                    more = false;
                    break;
                }
            }
            // byte stop (approximate: exclude header size)
            if let Some(byte_target) = self.params.bytes {
                if self.total_bytes >= byte_target {
                    if std::env::var("DJVU_VERBOSE_LOG").unwrap_or_default() == "1" {
                        println!("IWEncoder::encode_chunk - Stopping due to byte target reached");
                    }
                    more = false;
                    break;
                }
            }
            // slice count stop
            if let Some(slice_target) = self.params.slices {
                if self.total_slices >= slice_target {
                    if std::env::var("DJVU_VERBOSE_LOG").unwrap_or_default() == "1" {
                        println!("IWEncoder::encode_chunk - Stopping due to slice target reached");
                    }
                    more = false;
                    break;
                }
            }

            // encode Y slice - use actual encoding
            more = self.y_codec.encode_slice(&mut zp)?;
            
            if std::env::var("DJVU_VERBOSE_LOG").unwrap_or_default() == "1" {
                println!("IWEncoder::encode_chunk - Y slice encoded, more: {}", more);
            }
            // encode Cb/Cr slice when due
            if let (Some(ref mut cb), Some(ref mut cr)) = (self.cb_codec.as_mut(), self.cr_codec.as_mut()) {
                if self.total_slices as isize >= self.crcb_delay {
                    more |= cb.encode_slice(&mut zp)?;
                    more |= cr.encode_slice(&mut zp)?;
                }
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
                if std::env::var("DJVU_VERBOSE_LOG").unwrap_or_default() == "1" {
                    println!("IWEncoder::encode_chunk - Safety break after {} iterations (max_slices={})", loop_count, max_slices);
                }
                eprintln!("Warning: Slice cap {} reached, aborting chunk early", max_slices);
                more = false;
                break;
            }
        }

        if std::env::var("DJVU_VERBOSE_LOG").unwrap_or_default() == "1" {
            println!("IWEncoder::encode_chunk - Encoding loop completed, finishing ZP codec...");
        }

        // finish arithmetic payload
        let payload = zp.finish()?.into_inner();

        if std::env::var("DJVU_VERBOSE_LOG").unwrap_or_default() == "1" {
            println!("IWEncoder::encode_chunk - ZP codec finished, payload size: {} bytes", payload.len());
        }

        // === IFF header generation ===
        const IW_MAJOR: u8 = 4;
        const IW_MINOR: u8 = 0;
        let mut chunk = Vec::with_capacity(8 + payload.len());

        if std::env::var("DJVU_VERBOSE_LOG").unwrap_or_default() == "1" {
            println!("IWEncoder::encode_chunk - Starting header generation...");
        }

        if self.first_chunk {
            // FORM header for first chunk
            chunk.extend_from_slice(b"FORM");
            // Placeholder for length (will be calculated and filled later)
            let length_placeholder_pos = chunk.len();
            chunk.extend_from_slice(&[0, 0, 0, 0]);
            // Chunk type based on color mode
            chunk.extend_from_slice(match self.params.crcb_mode {
                CrcbMode::None => b"BM44",
                _ => b"PM44",
            });

            // Calculate and fill in the FORM length
            let form_length = 4 + 9 + payload.len(); // "BM44"/"PM44" + headers + payload
            let length_bytes = (form_length as u32).to_be_bytes();
            chunk[length_placeholder_pos..length_placeholder_pos + 4]
                .copy_from_slice(&length_bytes);

            self.first_chunk = false;
        }

        // Primary header: serial=0, slices = total_slices mod 256
        chunk.push(0);
        chunk.push((self.total_slices & 0xFF) as u8);

        // Secondary header: (major|0x80), minor
        chunk.push(IW_MAJOR | 0x80);
        chunk.push(IW_MINOR);

        // Tertiary header: width_hi,width_lo,height_hi,height_lo, crcb_delay
        chunk.push(((w >> 8) & 0xFF) as u8);
        chunk.push((w & 0xFF) as u8);
        chunk.push(((h >> 8) & 0xFF) as u8);
        chunk.push((h & 0xFF) as u8);
        chunk.push(self.crcb_delay as u8);

        // append payload
        chunk.extend_from_slice(&payload);

        // update total_bytes with full chunk (excluding the IFF size header)
        self.total_bytes += chunk.len();

        if std::env::var("DJVU_VERBOSE_LOG").unwrap_or_default() == "1" {
            println!("IWEncoder::encode_chunk - Completed successfully, chunk size: {} bytes", chunk.len());
        }

        Ok((chunk, more))
    }
}
