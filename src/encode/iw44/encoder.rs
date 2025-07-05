// src/encode/iw44/encoder.rs

use super::codec::Codec;
use super::coeff_map::CoeffMap;
use super::transform;
use crate::encode::zc::ZEncoder;
use ::image::{GrayImage, RgbImage};
use bytemuck;
use std::io::{Cursor, Write};
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
            decibels: None,
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
        let mut cb_table = [[0i32; 256]; 3];  // [R, G, B] components for Cb
        let mut cr_table = [[0i32; 256]; 3];  // [R, G, B] components for Cr
        
        for i in 0..256 {
            let v = i as i32;
            
            // Y coefficients (no offset, full 0-255 range)
            y_table[0][i] = (19595 * v) >> 16;  // 0.299 * 65536
            y_table[1][i] = (38469 * v) >> 16;  // 0.587 * 65536  
            y_table[2][i] = (7471 * v) >> 16;   // 0.114 * 65536
            
            // Cb coefficients (centered on 128)
            cb_table[0][i] = ((-11059 * v) >> 16) + 128;  // -0.168736 * 65536
            cb_table[1][i] = ((-21709 * v) >> 16) + 128;  // -0.331264 * 65536
            cb_table[2][i] = ((32768 * v) >> 16) + 128;   //  0.500000 * 65536
            
            // Cr coefficients (centered on 128)
            cr_table[0][i] = ((32768 * v) >> 16) + 128;   //  0.500000 * 65536
            cr_table[1][i] = ((-27439 * v) >> 16) + 128;  // -0.418688 * 65536
            cr_table[2][i] = ((-5329 * v) >> 16) + 128;   // -0.081312 * 65536
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
        
        // Cb/Cr: centered on 128, then subtract 128 to get signed range
        let cb = cb_table[0][r as usize] + cb_table[1][g as usize] + cb_table[2][b as usize];
        let cr = cr_table[0][r as usize] + cr_table[1][g as usize] + cr_table[2][b as usize];
        
        out_cb[i] = (cb - 128).clamp(i8::MIN as i32, i8::MAX as i32) as i8;
        out_cr[i] = (cr - 128).clamp(i8::MIN as i32, i8::MAX as i32) as i8;
        
        // Collect samples for debugging
        if sample_indices.contains(&i) {
            y_samples.push((r, g, b, y_val, out_y[i]));
            cb_samples.push(out_cb[i]);
            cr_samples.push(out_cr[i]);
        }
    }
    
    // Print debug info for YCbCr conversion
    println!("YCbCr conversion samples (RGB → Y_uint8 → Y_i8):");
    for (idx, &sample_i) in sample_indices.iter().enumerate() {
        if idx < y_samples.len() {
            let (r, g, b, y_uint, y_i8) = y_samples[idx];
            println!("  Pixel {}: RGB({},{},{}) → Y_uint={} → Y_i8={}, Cb={}, Cr={}", 
                     sample_i, r, g, b, y_uint, y_i8, cb_samples[idx], cr_samples[idx]);
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

        // Convert Y channel to GrayImage (signed i8 -> unsigned u8)
        let y_plane = GrayImage::from_raw(
            width,
            height,
            y_buf.iter().map(|&v| (v as i32 + 128) as u8).collect(),
        ).unwrap();
        
        let ymap = CoeffMap::create_from_image(&y_plane, mask);
        let y_codec = Codec::new(ymap);

        let (cb_codec, cr_codec) = if delay >= 0 {
            // Convert Cb/Cr channels to GrayImage (signed i8 -> unsigned u8)
            let cb_plane = GrayImage::from_raw(
                width,
                height,
                cb_buf.iter().map(|&v| (v as i32 + 128) as u8).collect(),
            ).unwrap();
            
            let cr_plane = GrayImage::from_raw(
                width,
                height,
                cr_buf.iter().map(|&v| (v as i32 + 128) as u8).collect(),
            ).unwrap();

            let mut cbmap = CoeffMap::create_from_image(&cb_plane, mask);
            let mut crmap = CoeffMap::create_from_image(&cr_plane, mask);

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
        let mut slice_count = 0u8;
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
        
        // Encode up to max_slices
        while slices_encoded < max_slices && self.cur_bit >= 0 {
            let y_has_data = self.y_codec.encode_slice(&mut zp)?;
            
            // Debug: Log slice encoding status
            if slices_encoded % 50 == 0 || !y_has_data {
                println!("Slice {}: bit={}, band={}, y_has_data={}", 
                         self.total_slices, self.cur_bit, self.y_codec.cur_band, y_has_data);
            }
            
            // Always count Y slice
            slices_encoded += 1;
            
            // Handle chrominance delay for Cb/Cr components
            let mut cb_cr_encoded = false;
            if let (Some(ref mut cb), Some(ref mut cr)) = (&mut self.cb_codec, &mut self.cr_codec) {
                let crcb_delay = match self.params.crcb_mode {
                    CrcbMode::Half | CrcbMode::Normal => 10,
                    _ => 0,
                };
                
                if self.total_slices >= crcb_delay {
                    cb.encode_slice(&mut zp)?;
                    cr.encode_slice(&mut zp)?;
                    cb_cr_encoded = true;
                }
            }
            
            // Update total slice count and synchronize state
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
            
            // Calculate actual slice count for the chunk header
            if cb_cr_encoded {
                slice_count += 3; // Y + Cb + Cr
            } else {
                slice_count += 1; // Just Y
            }
            
            // Break if we've exhausted all bit-planes
            if self.cur_bit < 0 {
                break;
            }
        }

        // Write chunk header
        chunk_data.push(self.serial);
        chunk_data.push(slice_count);

        if self.serial == 0 {
            let is_pm44 = self.cb_codec.is_some();
            let color_bit = if is_pm44 { 0 } else { 1 };
            let major = (color_bit << 7) | 1;
            chunk_data.push(major);
            chunk_data.push(2); // Minor version
            chunk_data.extend_from_slice(&(w as u16).to_be_bytes());
            chunk_data.extend_from_slice(&(h as u16).to_be_bytes());

            let delay = match self.params.crcb_mode {
                CrcbMode::Half | CrcbMode::Normal => 10,
                _ => 0,
            } as u8;
            chunk_data.push(0x80 | (delay & 0x7F));
        }

        let zp_data = zp.finish()?.into_inner();
        chunk_data.extend_from_slice(&zp_data);

        self.serial = self.serial.wrapping_add(1);
        self.total_bytes += chunk_data.len();

        Ok((chunk_data, self.cur_bit >= 0))
    }
}