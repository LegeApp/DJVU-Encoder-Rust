// src/encode/iw44/encoder.rs

use super::codec::Codec;
use super::coeff_map::CoeffMap;
use crate::encode::zc::ZEncoder;
use ::image::{GrayImage, RgbImage};
use bytemuck;
use std::io::Cursor;
use std::sync::OnceLock;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum EncoderError {
    #[error("At least one stop condition must be set")]
    NeedStopCondition,
    #[error("Input image is empty or invalid")]
    EmptyObject,
    #[error("ZP codec error: {0}")]
    ZCodec(#[from] crate::encode::zc::ZCodecError),
    #[error("General error: {0}")]
    General(#[from] crate::utils::error::DjvuError),
}

#[derive(Debug, Clone, Copy, Default)]
pub enum CrcbMode {
    #[default]
    None,
    Half,
    Normal,
    Full,
}

#[derive(Debug, Clone, Copy)]
pub struct EncoderParams {
    pub decibels: Option<f32>,
    pub crcb_mode: CrcbMode,
    pub db_frac: f32,
}

impl Default for EncoderParams {
    fn default() -> Self {
        Self {
            decibels: Some(90.0), // Default to good quality instead of None
            crcb_mode: CrcbMode::Full,
            db_frac: 0.35,
        }
    }
}
// (1) helper to go from signed i8 → unbiased u8
#[inline]
fn signed_to_unsigned_u8(v: i8) -> u8 { (v as i16 + 128) as u8 }

fn convert_signed_buffer_to_grayscale(buf: &[i8], w: u32, h: u32) -> GrayImage {
    let bytes: Vec<u8> = buf.iter().map(|&v| signed_to_unsigned_u8(v)).collect();
    GrayImage::from_raw(w, h, bytes).expect("Invalid buffer dimensions")
}

// Fixed-point constants for YCbCr conversion (Rec.601)
const SCALE: i32 = 1 << 16;
const ROUND: i32 = 1 << 15;

// Pre-computed YCbCr conversion tables (computed once)
static YCC_TABLES: OnceLock<([[i32; 256]; 3], [[i32; 256]; 3], [[i32; 256]; 3])> = OnceLock::new();

fn get_ycc_tables() -> &'static ([[i32; 256]; 3], [[i32; 256]; 3], [[i32; 256]; 3]) {
    YCC_TABLES.get_or_init(|| {
        let mut y_table = [[0i32; 256]; 3];   // [R, G, B] components for Y
        // Chrominance tables store raw coefficients without the +128 offset.
        // The bias is applied after summing the R/G/B contributions.
        let mut cb_table = [[0i32; 256]; 3];  // [R, G, B] components for Cb
        let mut cr_table = [[0i32; 256]; 3];  // [R, G, B] components for Cr
        
        for i in 0..256 {
            let v = i as i32;
            
            // Y coefficients (no offset, full 0-255 range)
            y_table[0][i] = (19595 * v) >> 16;  // 0.299 * 65536
            y_table[1][i] = (38469 * v) >> 16;  // 0.587 * 65536  
            y_table[2][i] = (7471 * v) >> 16;   // 0.114 * 65536
            
            // Cb coefficients (no offset yet)
            cb_table[0][i] = (-11059 * v) >> 16;  // -0.168736 * 65536
            cb_table[1][i] = (-21709 * v) >> 16;  // -0.331264 * 65536
            cb_table[2][i] = (32768 * v) >> 16;   //  0.500000 * 65536

            // Cr coefficients (no offset yet)
            cr_table[0][i] = (32768 * v) >> 16;   //  0.500000 * 65536
            cr_table[1][i] = (-27439 * v) >> 16;  // -0.418688 * 65536
            cr_table[2][i] = (-5329 * v) >> 16;   // -0.081312 * 65536
        }
        
        (y_table, cb_table, cr_table)
    })
}

/// Optimized RGB → YCbCr conversion with pre-computed tables.
/// Y channel: 0-255 range mapped to signed i8: -128 to +127 (0→-128, 255→+127)
/// Cb/Cr channels: centered on 0 (stored as signed i8: -128 to +127)
pub fn rgb_to_ycbcr_buffers(
    img: &RgbImage,
    out_y:  &mut [i8],
    out_cb: &mut [i8],
    out_cr: &mut [i8],
) {
    let (y_table, cb_table, cr_table) = get_ycc_tables();
    let pixels: &[[u8;3]] = bytemuck::cast_slice(img.as_raw());

    assert_eq!(out_y.len(), pixels.len());
    assert_eq!(out_cb.len(), pixels.len());
    assert_eq!(out_cr.len(), pixels.len());

    // Debug sample to check conversion
    let sample_indices = [0, pixels.len() / 4, pixels.len() / 2, 3 * pixels.len() / 4, pixels.len() - 1];
    let mut y_samples = Vec::new();
    let mut cb_samples = Vec::new();
    let mut cr_samples = Vec::new();

    for (i, &[r, g, b]) in pixels.iter().enumerate() {
        // Y: full 0-255 range, no centering
        let y = y_table[0][r as usize] + y_table[1][g as usize] + y_table[2][b as usize];
        let y_val = y.clamp(0, 255);
        // Store Y as signed but preserve full 0-255 range by subtracting 128
        // This maps 0-255 to -128-127, which is the expected format for IW44
        out_y[i] = (y_val - 128) as i8;
        
        // Cb/Cr: sum raw coefficients then apply +128 bias before converting to
        // signed representation.
        // Raw chroma values (range roughly [-128,127]). Add the +128 offset
        // only for the intermediate 8-bit representation if needed; the final
        // buffers store signed values.
        let cb_raw = cb_table[0][r as usize] + cb_table[1][g as usize] + cb_table[2][b as usize];
        let cr_raw = cr_table[0][r as usize] + cr_table[1][g as usize] + cr_table[2][b as usize];

        out_cb[i] = cb_raw.clamp(i8::MIN as i32, i8::MAX as i32) as i8;
        out_cr[i] = cr_raw.clamp(i8::MIN as i32, i8::MAX as i32) as i8;
        
        // Collect samples for debugging
        if sample_indices.contains(&i) {
            y_samples.push((r, g, b, y_val, out_y[i]));
            cb_samples.push(out_cb[i]);
            cr_samples.push(out_cr[i]);
        }
    }
}
pub struct IWEncoder {
    y_codec: Codec,
    cb_codec: Option<Codec>,
    cr_codec: Option<Codec>,
    params: EncoderParams,
    total_slices: usize,
    total_bytes: usize,
    serial: u8,
    cur_bit: i32, // Synchronized bit-plane index
}

impl IWEncoder {
    pub fn from_gray(
        img: &GrayImage,
        mask: Option<&GrayImage>,
        params: EncoderParams,
    ) -> Result<Self, EncoderError> {
        let ymap = CoeffMap::create_from_image(img, mask);
        let y_codec = Codec::new(ymap);
        let cur_bit = y_codec.cur_bit;

        Ok(IWEncoder {
            y_codec,
            cb_codec: None,
            cr_codec: None,
            params,
            total_slices: 0,
            total_bytes: 0,
            serial: 0,
            cur_bit,
        })
    }

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

        let (width, height) = img.dimensions();
        let num_pixels = (width * height) as usize;
        
        // Convert RGB to YCbCr using the corrected function
        let mut y_buf = vec![0i8; num_pixels];
        let mut cb_buf = vec![0i8; num_pixels];
        let mut cr_buf = vec![0i8; num_pixels];
        rgb_to_ycbcr_buffers(img, &mut y_buf, &mut cb_buf, &mut cr_buf);

        // Create Y coefficient map directly from signed Y buffer (keeping centered at 0)
        let ymap = CoeffMap::create_from_signed_channel(&y_buf, width, height, mask, "Y");
        let y_codec = Codec::new(ymap);

        let (cb_codec, cr_codec) = if delay >= 0 {
            // Create Cb/Cr coefficient maps directly from signed buffers (keeping centered at 0)
            let mut cbmap = CoeffMap::create_from_signed_channel(&cb_buf, width, height, mask, "Cb");
            let mut crmap = CoeffMap::create_from_signed_channel(&cr_buf, width, height, mask, "Cr");

            if half {
                cbmap.slash_res(2);
                crmap.slash_res(2);
            }
            (Some(Codec::new(cbmap)), Some(Codec::new(crmap)))
        } else {
            (None, None)
        };

        let cur_bit = y_codec.cur_bit;

        Ok(IWEncoder {
            y_codec,
            cb_codec,
            cr_codec,
            params,
            total_slices: 0,
            total_bytes: 0,
            serial: 0,
            cur_bit,
        })
    }

    pub fn encode_chunk(&mut self, max_slices: usize) -> Result<(Vec<u8>, bool), EncoderError> {
        let (w, h) = {
            let map = &self.y_codec.map;
            let w = map.width();
            let h = map.height();
            if w == 0 || h == 0 {
                return Err(EncoderError::EmptyObject);
            }
            (w, h)
        };

        if self.cur_bit < 0 {
            return Ok((Vec::new(), false));
        }

        let mut chunk_data = Vec::new();
        let mut zp = ZEncoder::new(Cursor::new(Vec::new()), true)?;

        // Synchronize bit-planes across components
        self.y_codec.cur_bit = self.cur_bit;
        if let Some(ref mut cb) = self.cb_codec {
            cb.cur_bit = self.cur_bit;
        }
        if let Some(ref mut cr) = self.cr_codec {
            cr.cur_bit = self.cur_bit;
        }

        let mut slices_encoded = 0;
        let initial_bytes = self.total_bytes;
        
        // Encode slices according to DjVu spec: multiple slices per chunk
        // Each "slice" is one logical unit containing color bands for active components
        while slices_encoded < max_slices && self.cur_bit >= 0 {
            // A DjVu "slice" contains one color band for each active component
            // Encode Y component (always present)
            let y_has_data = self.y_codec.encode_slice(&mut zp)?;
            
            // Handle chrominance components based on delay
            let crcb_delay = match self.params.crcb_mode {
                CrcbMode::Half | CrcbMode::Normal => 10,
                _ => 0,
            };
            
            if let (Some(ref mut cb), Some(ref mut cr)) = (&mut self.cb_codec, &mut self.cr_codec) {
                if self.total_slices >= crcb_delay {
                    cb.encode_slice(&mut zp)?;
                    cr.encode_slice(&mut zp)?;
                }
            }
            
            // Only count this as a slice if we encoded meaningful data
            if y_has_data {
                slices_encoded += 1;
                self.total_slices += 1;
                
                // Synchronize cur_bit from Y codec (it manages band progression)
                self.cur_bit = self.y_codec.cur_bit;
                
                // Sync chrominance codecs if they exist
                if let Some(ref mut cb) = self.cb_codec {
                    cb.cur_bit = self.cur_bit;
                }
                if let Some(ref mut cr) = self.cr_codec {
                    cr.cur_bit = self.cur_bit;
                }
            } else {
                // No meaningful data encoded, we're probably done
                break;
            }
            
            // Break if we've exhausted all bit-planes
            if self.cur_bit < 0 {
                break;
            }
        }
        
        // Finish ZP encoding
        let zp_data = zp.finish()?.into_inner();
        
        // Only create a chunk if we encoded some slices
        if slices_encoded == 0 || zp_data.is_empty() {
            return Ok((Vec::new(), false));
        }

        // Write IW44 chunk header according to DjVu spec
        
        // Serial number (1 byte)
        chunk_data.push(self.serial);
        
        // Number of slices in this chunk (1 byte)
        chunk_data.push(slices_encoded as u8);

        // Additional headers only for first chunk (serial 0)
        if self.serial == 0 {
            // Major version and color type (1 byte)
            let is_color = self.cb_codec.is_some();
            let color_bit = if is_color { 0 } else { 1 }; // 0 = color (3 components), 1 = grayscale (1 component)
            let major = (color_bit << 7) | 1; // Version 1
            chunk_data.push(major);
            
            // Minor version (1 byte)
            chunk_data.push(2);
            
            // Image width (2 bytes, big endian)
            chunk_data.extend_from_slice(&(w as u16).to_be_bytes());
            
            // Image height (2 bytes, big endian)
            chunk_data.extend_from_slice(&(h as u16).to_be_bytes());
            
            // Chrominance delay counter (1 byte)
            let delay = match self.params.crcb_mode {
                CrcbMode::Half | CrcbMode::Normal => 10,
                _ => 0,
            } as u8;
            chunk_data.push(0x80 | (delay & 0x7F)); // MSB set to 1 as per spec
        }

        // Append the ZP-encoded slice data
        chunk_data.extend_from_slice(&zp_data);

        // Update state
        self.serial = self.serial.wrapping_add(1);
        self.total_bytes += chunk_data.len();

        println!("DEBUG: Writing IW44 chunk {}, {} slices, {} bytes", 
                 self.serial - 1, slices_encoded, chunk_data.len());
        
        // Determine if there are more slices to emit
        // 'more' is true if we hit the max_slices for this chunk AND there are more bit-planes to process
        let more = self.cur_bit >= 0 && slices_encoded == max_slices;
        Ok((chunk_data, more))
    }
}