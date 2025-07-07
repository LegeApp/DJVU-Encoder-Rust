// src/encode/iw44/encoder.rs

#[cfg(not(feature = "ffi_codec"))]
use super::codec::Codec;
#[cfg(feature = "ffi_codec")]
use super::ffi_codec::FfiCodec as Codec;
use super::coeff_map::CoeffMap;
use crate::encode::zc::ZEncoder;
use ::image::{GrayImage, RgbImage};
use bytemuck;
use std::io::Cursor;
use std::sync::OnceLock;
use thiserror::Error;
use log::{debug, info, warn, error};

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
fn _signed_to_unsigned_u8(v: i8) -> u8 { (v as i16 + 128) as u8 }

fn _convert_signed_buffer_to_grayscale(buf: &[i8], w: u32, h: u32) -> GrayImage {
    let bytes: Vec<u8> = buf.iter().map(|&v| _signed_to_unsigned_u8(v)).collect();
    GrayImage::from_raw(w, h, bytes).expect("Invalid buffer dimensions")
}

// fixed-point constants, same as before
const _SCALE: i32 = 1 << 16;
const ROUND: i32 = 1 << 15;

// precompute only once
static YCC_TABLES: OnceLock<([[i32; 256]; 3], [[i32; 256]; 3], [[i32; 256]; 3])> = OnceLock::new();

fn get_ycc_tables() -> &'static ([[i32; 256]; 3], [[i32; 256]; 3], [[i32; 256]; 3]) {
    YCC_TABLES.get_or_init(|| {
        let mut y  = [[0;256]; 3];
        let mut cb = [[0;256]; 3];
        let mut cr = [[0;256]; 3];
        
        // Use EXACT coefficients from original DjVu C++ encoder
        // From IW44EncodeCodec.cpp rgb_to_ycc[3][3] matrix:
        const RGB_TO_YCC: [[f32; 3]; 3] = [
            [ 0.304348,  0.608696,  0.086956],  // Y coefficients
            [ 0.463768, -0.405797, -0.057971],  // Cr coefficients
            [-0.173913, -0.347826,  0.521739],  // Cb coefficients
        ];
        
        for k in 0..256 {
            // Exactly match C++ code: rmul[k] = (int)(k * 0x10000 * rgb_to_ycc[0][0]);
            y[0][k] = (k as f32 * 65536.0 * RGB_TO_YCC[0][0]) as i32;
            y[1][k] = (k as f32 * 65536.0 * RGB_TO_YCC[0][1]) as i32;
            y[2][k] = (k as f32 * 65536.0 * RGB_TO_YCC[0][2]) as i32;
            
            cb[0][k] = (k as f32 * 65536.0 * RGB_TO_YCC[2][0]) as i32;
            cb[1][k] = (k as f32 * 65536.0 * RGB_TO_YCC[2][1]) as i32;
            cb[2][k] = (k as f32 * 65536.0 * RGB_TO_YCC[2][2]) as i32;

            cr[0][k] = (k as f32 * 65536.0 * RGB_TO_YCC[1][0]) as i32;
            cr[1][k] = (k as f32 * 65536.0 * RGB_TO_YCC[1][1]) as i32;
            cr[2][k] = (k as f32 * 65536.0 * RGB_TO_YCC[1][2]) as i32;
        }
        (y, cb, cr)
    })
}

/// Convert an RGB-buffer (`img_raw`, length must be divisible by 3)
/// into three signed i8 planes (`out_y`, `out_cb`, `out_cr`).
pub fn rgb_to_ycbcr_planes(
    img_raw: &[u8],
    out_y:   &mut [i8],
    out_cb:  &mut [i8],
    out_cr:  &mut [i8],
) {
    assert!(img_raw.len() % 3 == 0,   "input length must be a multiple of 3");
    let npix = img_raw.len() / 3;
    assert_eq!(out_y.len(),  npix);
    assert_eq!(out_cb.len(), npix);
    assert_eq!(out_cr.len(), npix);

    let (y_tbl, cb_tbl, cr_tbl) = get_ycc_tables();

    for (i, chunk) in img_raw.chunks_exact(3).enumerate() {
        let r = chunk[0] as usize;
        let g = chunk[1] as usize;
        let b = chunk[2] as usize;

        // Exactly match C++ code calculation:
        // int y = rmul[p2->r] + gmul[p2->g] + bmul[p2->b] + 32768;
        // *out2 = (y >> 16) - 128;
        
        let y = y_tbl[0][r] + y_tbl[1][g] + y_tbl[2][b] + 32768;
        out_y[i] = ((y >> 16) - 128) as i8;

        let cb = cb_tbl[0][r] + cb_tbl[1][g] + cb_tbl[2][b] + 32768;
        out_cb[i] = (cb >> 16).clamp(-128, 127) as i8;

        let cr = cr_tbl[0][r] + cr_tbl[1][g] + cr_tbl[2][b] + 32768;
        out_cr[i] = (cr >> 16).clamp(-128, 127) as i8;
    }
}

/// Convert RgbImage to YCbCr buffers (wrapper for rgb_to_ycbcr_planes)
pub fn rgb_to_ycbcr_buffers(
    img: &RgbImage,
    out_y: &mut [i8],
    out_cb: &mut [i8],
    out_cr: &mut [i8],
) {
    let pixels: &[[u8; 3]] = bytemuck::cast_slice(img.as_raw());
    assert_eq!(out_y.len(), pixels.len());
    assert_eq!(out_cb.len(), pixels.len());
    assert_eq!(out_cr.len(), pixels.len());

    // Call the main conversion function
    rgb_to_ycbcr_planes(img.as_raw(), out_y, out_cb, out_cr);
}
/// Convert an `RgbImage` into three signed‐i8 planes (Y, Cb, Cr).
pub fn ycbcr_from_rgb(img: &RgbImage) -> (Vec<i8>, Vec<i8>, Vec<i8>) {
    let (w, h) = img.dimensions();
    let npix = (w * h) as usize;

    let mut y_buf  = vec![0i8; npix];
    let mut cb_buf = vec![0i8; npix];
    let mut cr_buf = vec![0i8; npix];

    // Re-use your core converter
    rgb_to_ycbcr_planes(img.as_raw(), &mut y_buf, &mut cb_buf, &mut cr_buf);
    (y_buf, cb_buf, cr_buf)
}

/// Build Y/Cb/Cr `Codec`s (or None for chroma) from signed‐i8 planes.
pub fn make_ycbcr_codecs(
    y_buf: &[i8],
    cb_buf: &[i8],
    cr_buf: &[i8],
    width: u32,
    height: u32,
    mask: Option<&GrayImage>,
    params: &EncoderParams,
) -> (Codec, Option<Codec>, Option<Codec>) {
    // Y is always present
    let ymap     = CoeffMap::create_from_signed_channel(y_buf, width, height, mask, "Y");
    let y_codec  = Codec::new(ymap, params);

    // Decide whether to build Cb/Cr
    let (cb_codec, cr_codec) = match params.crcb_mode {
        CrcbMode::None => (None, None),
        CrcbMode::Half | CrcbMode::Normal | CrcbMode::Full => {
            let mut cbmap = CoeffMap::create_from_signed_channel(cb_buf, width, height, mask, "Cb");
            let mut crmap = CoeffMap::create_from_signed_channel(cr_buf, width, height, mask, "Cr");
            if matches!(params.crcb_mode, CrcbMode::Half) {
                cbmap.slash_res(2);
                crmap.slash_res(2);
            }
            (Some(Codec::new(cbmap, params)), Some(Codec::new(crmap, params)))
        }
    };

    (y_codec, cb_codec, cr_codec)
}

/// High-level: build an `IWEncoder` straight from RGB.
pub fn encoder_from_rgb_with_helpers(
    img: &RgbImage,
    mask: Option<&GrayImage>,
    params: EncoderParams,
) -> Result<IWEncoder, EncoderError> {
    let (w, h) = img.dimensions();
    let (y_buf, cb_buf, cr_buf) = ycbcr_from_rgb(img);
    let (y_codec, cb_codec, cr_codec) =
        make_ycbcr_codecs(&y_buf, &cb_buf, &cr_buf, w, h, mask, &params);

    Ok(IWEncoder {
        y_codec,
        cb_codec,
        cr_codec,
        params,
        total_slices: 0,
        total_bytes: 0,
        serial: 0,
    })
}

/// And a symmetric one for gray:
pub fn encoder_from_gray_with_helpers(
    img: &GrayImage,
    mask: Option<&GrayImage>,
    params: EncoderParams,
) -> Result<IWEncoder, EncoderError> {
    let ymap    = CoeffMap::create_from_image(img, mask);
    let y_codec = Codec::new(ymap, &params);

    Ok(IWEncoder {
        y_codec,
        cb_codec: None,
        cr_codec: None,
        params,
        total_slices: 0,
        total_bytes: 0,
        serial: 0,
    })
}
pub struct IWEncoder {
    y_codec: Codec,
    cb_codec: Option<Codec>,
    cr_codec: Option<Codec>,
    params: EncoderParams,
    total_slices: usize,
    total_bytes: usize,
    serial: u8,
}

impl IWEncoder {
    pub fn from_gray(
        img: &GrayImage,
        mask: Option<&GrayImage>,
        params: EncoderParams,
    ) -> Result<Self, EncoderError> {
        encoder_from_gray_with_helpers(img, mask, params)
    }
    
    pub fn from_rgb(
        img: &RgbImage,
        mask: Option<&GrayImage>,
        params: EncoderParams,
    ) -> Result<Self, EncoderError> {
        info!("IWEncoder::from_rgb called with image {}x{}", img.width(), img.height());
        encoder_from_rgb_with_helpers(img, mask, params)
    }


    pub fn encode_chunk(&mut self, max_slices: usize) -> Result<(Vec<u8>, bool), EncoderError> {
        info!("encode_chunk called with max_slices={}", max_slices);
        info!("Y codec cur_bit={}, CB codec cur_bit={:?}, CR codec cur_bit={:?}", 
                 self.y_codec.cur_bit,
                 self.cb_codec.as_ref().map(|c| c.cur_bit),
                 self.cr_codec.as_ref().map(|c| c.cur_bit));
        
        let (w, h) = {
            let map = &self.y_codec.map;
            let w = map.width();
            let h = map.height();
            if w == 0 || h == 0 {
                return Err(EncoderError::EmptyObject);
            }
            (w, h)
        };

        // Check if all codecs are finished
        let all_finished = self.y_codec.cur_bit < 0 && 
                          self.cb_codec.as_ref().map_or(true, |c| c.cur_bit < 0) &&
                          self.cr_codec.as_ref().map_or(true, |c| c.cur_bit < 0);
        
        if all_finished {
            return Ok((Vec::new(), false));
        }

        let mut chunk_data = Vec::new();
        let mut zp = ZEncoder::new(Cursor::new(Vec::new()), true)?;

        let mut slices_encoded = 0;
        let _initial_bytes = self.total_bytes;
        
        // Encode slices according to DjVu spec: multiple slices per chunk
        // Each "slice" is one logical unit containing color bands for active components
        // Each codec maintains its own cur_bit and progresses independently
        while slices_encoded < max_slices {
            // Check if any codec still has data to encode
            let any_active = self.y_codec.cur_bit >= 0 || 
                           self.cb_codec.as_ref().map_or(false, |c| c.cur_bit >= 0) ||
                           self.cr_codec.as_ref().map_or(false, |c| c.cur_bit >= 0);
            
            debug!("Loop iteration, any_active={}, Y cur_bit={}, slices_encoded={}", 
                     any_active, self.y_codec.cur_bit, slices_encoded);
            
            if !any_active {
                debug!("No codecs active, breaking loop");
                break;
            }
            
            // A DjVu "slice" contains one color band for each active component
            // Encode Y component if it still has data
            let y_has_data = if self.y_codec.cur_bit >= 0 {
                println!("PRINTLN: About to call Y codec encode_slice, cur_bit={}", self.y_codec.cur_bit);
                debug!("Calling Y codec encode_slice, cur_bit={}", self.y_codec.cur_bit);
                let result = self.y_codec.encode_slice(&mut zp)?;
                println!("PRINTLN: Y codec encode_slice returned: {}", result);
                result
            } else {
                debug!("Y codec finished (cur_bit < 0)");
                false
            };
            
            // CORRECTED: Unconditionally encode Cb and Cr if the codecs exist and are active.
            // The faulty delay logic has been removed.
            let mut cb_has_data = false;
            if let Some(ref mut cb) = &mut self.cb_codec {
                if cb.cur_bit >= 0 {
                    cb_has_data = cb.encode_slice(&mut zp)?;
                }
            }
            
            let mut cr_has_data = false;
            if let Some(ref mut cr) = &mut self.cr_codec {
                if cr.cur_bit >= 0 {
                    cr_has_data = cr.encode_slice(&mut zp)?;
                }
            }
            
            // Only count this as a slice if we encoded meaningful data from ANY component
            if y_has_data || cb_has_data || cr_has_data {
                slices_encoded += 1;
                self.total_slices += 1;
            } else {
                // No meaningful data encoded from any component in this iteration
                // This can happen when all codecs have progressed past their useful bit-planes
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
            
            // Chrominance delay counter (1 byte) - set to 0 since we encode all components immediately
            chunk_data.push(0x80); // MSB set to 1 as per spec, delay = 0
        }

        // Append the ZP-encoded slice data
        chunk_data.extend_from_slice(&zp_data);

        // Update state
        self.serial = self.serial.wrapping_add(1);
        self.total_bytes += chunk_data.len();

        #[cfg(debug_assertions)]
        debug!("Writing IW44 chunk {}, {} slices, {} bytes", 
                 self.serial - 1, slices_encoded, chunk_data.len());
        
        // Determine if there are more slices to emit
        // 'more' is true if we hit the max_slices for this chunk AND any codec still has data to process
        let any_codec_active = self.y_codec.cur_bit >= 0 || 
                              self.cb_codec.as_ref().map_or(false, |c| c.cur_bit >= 0) ||
                              self.cr_codec.as_ref().map_or(false, |c| c.cur_bit >= 0);
        let more = any_codec_active && slices_encoded == max_slices;
        Ok((chunk_data, more))
    }
}