// src/iw44/transform.rs

use ::image::{GrayImage, RgbImage};
use bytemuck;

// Helper function to convert signed to unsigned with offset
#[inline]
fn signed_to_unsigned_u8(signed_val: i8) -> u8 {
    (signed_val as i16 + 128) as u8
}

fn convert_signed_buffer_to_grayscale(buffer: &[i8], w: u32, h: u32) -> GrayImage {
    let unsigned_buffer: Vec<u8> = buffer
        .iter()
        .map(|&signed_val| signed_to_unsigned_u8(signed_val))
        .collect();

    // Debug: check first few converted values
    if unsigned_buffer.len() >= 10 {
        let unsigned_samples: Vec<u8> = unsigned_buffer.iter().take(10).cloned().collect();
        println!("DEBUG: First 10 unsigned Y values: {:?}", unsigned_samples);
    }

    GrayImage::from_raw(w, h, unsigned_buffer).unwrap()
}

// YCbCr color conversion constants from C++ rgb_to_ycc
const RGB_TO_YCC: [[f32; 3]; 3] = [
    [0.304348, 0.608696, 0.086956],   // Y
    [0.463768, -0.405797, -0.057971], // Cr
    [-0.173913, -0.347826, 0.521739], // Cb
];

// Precompute multiplication tables for each channel
fn precompute_tables() -> ([i32; 256], [i32; 256], [i32; 256]) {
    let mut rmul = [0; 256];
    let mut gmul = [0; 256];
    let mut bmul = [0; 256];
    for k in 0..256 {
        rmul[k] = (k as f32 * 65536.0 * RGB_TO_YCC[0][0]) as i32;
        gmul[k] = (k as f32 * 65536.0 * RGB_TO_YCC[0][1]) as i32;
        bmul[k] = (k as f32 * 65536.0 * RGB_TO_YCC[0][2]) as i32;
    }
    (rmul, gmul, bmul)
}

/// Wrapper that returns GrayImage from RGB luminance conversion
pub fn rgb_to_y(img: &RgbImage) -> GrayImage {
    let (w, h) = img.dimensions();
    let mut buffer = vec![0i8; (w * h) as usize];
    rgb_to_y_buffer(img, &mut buffer);

    // Debug: check first few Y values
    if buffer.len() >= 10 {
        let y_samples: Vec<i8> = buffer.iter().take(10).cloned().collect();
        println!("DEBUG: First 10 Y values: {:?}", y_samples);
    }

    convert_signed_buffer_to_grayscale(&buffer, w, h)
}

/// Wrapper that returns GrayImage from RGB Cb conversion
pub fn rgb_to_cb(img: &RgbImage) -> GrayImage {
    let (w, h) = img.dimensions();
    let mut buffer = vec![0i8; (w * h) as usize];
    rgb_to_cb_buffer(img, &mut buffer);
    convert_signed_buffer_to_grayscale(&buffer, w, h)
}

/// Wrapper that returns GrayImage from RGB Cr conversion
pub fn rgb_to_cr(img: &RgbImage) -> GrayImage {
    let (w, h) = img.dimensions();
    let mut buffer = vec![0i8; (w * h) as usize];
    rgb_to_cr_buffer(img, &mut buffer);
    convert_signed_buffer_to_grayscale(&buffer, w, h)
}

/// Converts RGB image to Y (luminance) channel using fixed-point arithmetic.
/// Optimized version using bytemuck for efficient RGB triplet processing.
pub fn rgb_to_y_buffer(img: &RgbImage, out: &mut [i8]) {
    let (rmul, gmul, bmul) = precompute_tables();

    // Get raw pixel data as u8 slice
    let raw_pixels = img.as_raw();

    // Use bytemuck to cast RGB triplets for efficient processing
    // RGB pixels are stored as [R, G, B, R, G, B, ...]
    let rgb_triplets: &[[u8; 3]] = bytemuck::cast_slice(raw_pixels);

    // Process each RGB triplet directly
    for (idx, &[r, g, b]) in rgb_triplets.iter().enumerate() {
        let y_val = rmul[r as usize] + gmul[g as usize] + bmul[b as usize] + 32768;
        out[idx] = ((y_val >> 16) - 128) as i8;
    }
}

/// Precompute tables for Cb channel
fn precompute_cb_tables() -> ([i32; 256], [i32; 256], [i32; 256]) {
    let mut rmul = [0; 256];
    let mut gmul = [0; 256];
    let mut bmul = [0; 256];
    for k in 0..256 {
        rmul[k] = (k as f32 * 65536.0 * RGB_TO_YCC[2][0]) as i32;
        gmul[k] = (k as f32 * 65536.0 * RGB_TO_YCC[2][1]) as i32;
        bmul[k] = (k as f32 * 65536.0 * RGB_TO_YCC[2][2]) as i32;
    }
    (rmul, gmul, bmul)
}

/// Converts RGB image to Cb (blue-difference) channel using fixed-point arithmetic.
/// Optimized version using bytemuck for efficient RGB triplet processing.
pub fn rgb_to_cb_buffer(img: &RgbImage, out: &mut [i8]) {
    let (rmul, gmul, bmul) = precompute_cb_tables();

    let raw_pixels = img.as_raw();
    let rgb_triplets: &[[u8; 3]] = bytemuck::cast_slice(raw_pixels);

    for (idx, &[r, g, b]) in rgb_triplets.iter().enumerate() {
        let cb_val = rmul[r as usize] + gmul[g as usize] + bmul[b as usize] + 32768;
        out[idx] = (cb_val >> 16).clamp(-128, 127) as i8;
    }
}

/// Precompute tables for Cr channel
fn precompute_cr_tables() -> ([i32; 256], [i32; 256], [i32; 256]) {
    let mut rmul = [0; 256];
    let mut gmul = [0; 256];
    let mut bmul = [0; 256];
    for k in 0..256 {
        rmul[k] = (k as f32 * 65536.0 * RGB_TO_YCC[1][0]) as i32;
        gmul[k] = (k as f32 * 65536.0 * RGB_TO_YCC[1][1]) as i32;
        bmul[k] = (k as f32 * 65536.0 * RGB_TO_YCC[1][2]) as i32;
    }
    (rmul, gmul, bmul)
}

/// Converts RGB image to Cr (chrominance) channel using fixed-point arithmetic.
/// Optimized version using bytemuck for efficient RGB triplet processing.
pub fn rgb_to_cr_buffer(img: &RgbImage, out: &mut [i8]) {
    let (rmul, gmul, bmul) = precompute_cr_tables();

    // Get raw pixel data as u8 slice
    let raw_pixels = img.as_raw();
    let rgb_triplets: &[[u8; 3]] = bytemuck::cast_slice(raw_pixels);

    // Process each RGB triplet directly
    for (idx, &[r, g, b]) in rgb_triplets.iter().enumerate() {
        let cr_val = rmul[r as usize] + gmul[g as usize] + bmul[b as usize] + 32768;
        out[idx] = ((cr_val >> 16) - 128) as i8;
    }
}

pub struct Encode;

impl Encode {
    /// Forward wavelet transform.
    /// Port of `IW44Image::Transform::Encode::forward`
    pub fn forward(p: &mut [i16], w: usize, h: usize, rowsize: usize, begin: usize, end: usize) {
        for i in begin..end {
            let scale = 1 << i;
            filter_fv(p, w, h, rowsize, scale);
            filter_fh(p, w, h, rowsize, scale);
        }
    }
}

// Port of `filter_fv` from `IW44EncodeCodec.cpp`
fn filter_fv(p: &mut [i16], w: usize, h: usize, rowsize: usize, scale: usize) {
    let s = scale * rowsize;
    let s3 = 3 * s;
    let effective_h = ((h - 1) / scale) + 1;

    for y_idx in (1..effective_h).step_by(2) {
        let p_offset = y_idx * s;

        // 1-Delta
        if y_idx >= 3 && y_idx + 3 < effective_h {
            // Generic case - safe from boundary checks
            for x_idx in (0..w).step_by(scale) {
                let q_idx = p_offset + x_idx;
                let a = p[q_idx - s] as i32 + p[q_idx + s] as i32;
                let b = p[q_idx - s3] as i32 + p[q_idx + s3] as i32;
                p[q_idx] = p[q_idx].saturating_sub((((a << 3) + a - b + 8) >> 4) as i16);
            }
        } else {
            // Special cases near boundaries
            for x_idx in (0..w).step_by(scale) {
                let q_idx = p_offset + x_idx;
                let prev_s = p[q_idx - s];
                let next_s = if y_idx + 1 < effective_h {
                    p[q_idx + s]
                } else {
                    prev_s
                };
                let a = prev_s as i32 + next_s as i32;
                p[q_idx] = p[q_idx].saturating_sub(((a + 1) >> 1) as i16);
            }
        }

        // 2-Update
        let p_update_offset = p_offset.saturating_sub(s3);
        if y_idx >= 6 && y_idx < effective_h {
            // Generic case
            for x_idx in (0..w).step_by(scale) {
                let q_idx = p_update_offset + x_idx;
                let a = p[q_idx - s] as i32 + p[q_idx + s] as i32;
                let b = p[q_idx - s3] as i32 + p[q_idx + s3] as i32;
                p[q_idx] += (((a << 3) + a - b + 16) >> 5) as i16;
            }
        } else if y_idx >= 3 {
            // Special cases for update
            for x_idx in (0..w).step_by(scale) {
                let q_idx = p_update_offset + x_idx;
                let a = (if y_idx >= 4 {
                    p.get(q_idx - s).copied()
                } else {
                    None
                })
                .unwrap_or(0) as i32
                    + (if y_idx - 2 < effective_h {
                        p.get(q_idx + s).copied()
                    } else {
                        None
                    })
                    .unwrap_or(0) as i32;
                let b = (if y_idx >= 6 {
                    p.get(q_idx - s3).copied()
                } else {
                    None
                })
                .unwrap_or(0) as i32
                    + (if y_idx < effective_h {
                        p.get(q_idx + s3).copied()
                    } else {
                        None
                    })
                    .unwrap_or(0) as i32;

                p[q_idx] += (((a << 3) + a - b + 16) >> 5) as i16;
            }
        }
    }
}

// Port of `filter_fh` from `IW44EncodeCodec.cpp`
fn filter_fh(p: &mut [i16], w: usize, h: usize, rowsize: usize, scale: usize) {
    let s = scale;
    let s3 = 3 * s;
    let mut y = 0;

    while y < h {
        let row_start = y * rowsize;
        let p_row = &mut p[row_start..row_start + w];
        let mut q_idx = s;
        let e = w;

        let mut a0 = 0i16;
        let mut a1 = 0i16;
        let mut a2 = 0i16;
        let mut a3 = 0i16;
        let mut b0 = 0i16;
        let mut b1 = 0i16;
        let mut b2 = 0i16;
        let mut b3 = 0i16;

        // Special case: x=1 (q_idx = s)
        if q_idx < e {
            a1 = p_row[0]; // q[-s]
            a2 = a1;
            a3 = a1;
            if q_idx + s < e {
                a2 = p_row[q_idx + s];
            }
            if q_idx + s3 < e {
                a3 = p_row[q_idx + s3];
            }
            b3 = p_row[q_idx] - ((a1 as i32 + a2 as i32 + 1) >> 1) as i16;
            p_row[q_idx] = b3;
            q_idx += 2 * s;
        }

        // Generic case: while q + s3 < e
        while q_idx + s3 < e {
            a0 = a1;
            a1 = a2;
            a2 = a3;
            a3 = p_row[q_idx + s3];
            b0 = b1;
            b1 = b2;
            b2 = b3;
            let a_sum = a1 as i32 + a2 as i32;
            let delta = (((a_sum << 3) + a_sum - a0 as i32 - a3 as i32 + 8) >> 4) as i16;
            b3 = p_row[q_idx] - delta;
            p_row[q_idx] = b3;
            let b_sum = b1 as i32 + b2 as i32;
            let update = (((b_sum << 3) + b_sum - b0 as i32 - b3 as i32 + 16) >> 5) as i16;
            // Use checked arithmetic to prevent overflow
            if let Some(index) = q_idx.checked_sub(s3) {
                p_row[index] += update;
            }
            q_idx += 2 * s;
        }

        // Special case: w-3 <= x < w
        while q_idx < e {
            a1 = a2;
            a2 = a3;
            b0 = b1;
            b1 = b2;
            b2 = b3;
            let a_sum = a1 as i32 + a2 as i32;
            b3 = p_row[q_idx] - ((a_sum + 1) >> 1) as i16;
            p_row[q_idx] = b3;
            if q_idx >= s3 {
                // Use checked arithmetic to prevent overflow
                if let Some(index) = q_idx.checked_sub(s3) {
                    let b_sum = b1 as i32 + b2 as i32;
                    let update = (((b_sum << 3) + b_sum - b0 as i32 - b3 as i32 + 16) >> 5) as i16;
                    p_row[index] += update;
                }
            }
            q_idx += 2 * s;
        }

        // Special case: w <= x < w+3
        while q_idx < e + s3 {
            b0 = b1;
            b1 = b2;
            b2 = b3;
            b3 = 0;
            if q_idx >= s3 {
                // Use checked arithmetic to prevent overflow
                if let Some(index) = q_idx.checked_sub(s3) {
                    if index < p_row.len() {
                        let b_sum = b1 as i32 + b2 as i32;
                        let update =
                            (((b_sum << 3) + b_sum - b0 as i32 - b3 as i32 + 16) >> 5) as i16;
                        p_row[index] += update;
                    }
                }
            }
            q_idx += 2 * s;
        }

        y += scale;
    }
}

pub fn forward(
    p_slice: &mut [i16],
    w: usize,
    h: usize,
    rowsize: usize,
    begin_scale: usize,
    end_scale: usize,
) {
    let mut scale = begin_scale;
    while scale < end_scale {
        filter_fh(p_slice, w, h, rowsize, scale);
        filter_fv(p_slice, w, h, rowsize, scale);
        scale *= 2;
    }
}
pub struct Decode;

impl Decode {
    pub fn backward(p: &mut [i16], w: usize, h: usize, rowsize: usize, begin: usize, end: usize) {
        for i in (begin..end).rev() {
            let scale = 1 << i;
            filter_ih(p, w, h, rowsize, scale);
            filter_iv(p, w, h, rowsize, scale);
        }
    }
}
fn filter_iv(p: &mut [i16], w: usize, h: usize, rowsize: usize, scale: usize) {
    let s = scale * rowsize;
    let s3 = 3 * s;
    let effective_h = ((h - 1) / scale) + 1;

    for y_idx in (1..effective_h).step_by(2).rev() {
        let p_offset = y_idx * s;

        // Undo Update (2-Σ) first
        if y_idx >= 3 {
            let p_update_offset = p_offset.saturating_sub(s3);
            if y_idx >= 6 && y_idx < effective_h {
                // Generic case
                for x_idx in (0..w).step_by(scale) {
                    let q_idx = p_update_offset + x_idx;
                    let a = p[q_idx - s] as i32 + p[q_idx + s] as i32;
                    let b = p[q_idx - s3] as i32 + p[q_idx + s3] as i32;
                    let update = (((a << 3) + a - b + 16) >> 5) as i16;
                    p[q_idx] -= update; // Subtract instead of add
                }
            } else if y_idx >= 3 {
                // Special cases near boundaries
                for x_idx in (0..w).step_by(scale) {
                    let q_idx = p_update_offset + x_idx;
                    let a = (if y_idx >= 4 {
                        p.get(q_idx - s).copied()
                    } else {
                        None
                    })
                    .unwrap_or(0) as i32
                        + (if y_idx - 2 < effective_h {
                            p.get(q_idx + s).copied()
                        } else {
                            None
                        })
                        .unwrap_or(0) as i32;
                    let b = (if y_idx >= 6 {
                        p.get(q_idx - s3).copied()
                    } else {
                        None
                    })
                    .unwrap_or(0) as i32
                        + (if y_idx < effective_h {
                            p.get(q_idx + s3).copied()
                        } else {
                            None
                        })
                        .unwrap_or(0) as i32;
                    let update = (((a << 3) + a - b + 16) >> 5) as i16;
                    p[q_idx] -= update; // Subtract instead of add
                }
            }
        }

        // Undo Predict (1-Δ)
        if y_idx >= 3 && y_idx + 3 < effective_h {
            // Generic case
            for x_idx in (0..w).step_by(scale) {
                let q_idx = p_offset + x_idx;
                let a = p[q_idx - s] as i32 + p[q_idx + s] as i32;
                let b = p[q_idx - s3] as i32 + p[q_idx + s3] as i32;
                let delta = (((a << 3) + a - b + 8) >> 4) as i16;
                p[q_idx] += delta; // Add instead of subtract
            }
        } else {
            // Special cases near boundaries
            for x_idx in (0..w).step_by(scale) {
                let q_idx = p_offset + x_idx;
                let prev_s = p[q_idx - s];
                let next_s = if y_idx + 1 < effective_h {
                    p[q_idx + s]
                } else {
                    prev_s
                };
                let a = prev_s as i32 + next_s as i32;
                let delta = ((a + 1) >> 1) as i16;
                p[q_idx] += delta; // Add instead of subtract
            }
        }
    }
}
fn filter_ih(p: &mut [i16], w: usize, h: usize, rowsize: usize, scale: usize) {
    let s = scale;
    let s3 = 3 * s;
    let mut y = 0;

    while y < h {
        let row_start = y * rowsize;
        let p_row = &mut p[row_start..row_start + w];
        let e = w;

        // Process in reverse: start from the last possible q_idx and work backwards
        let mut q_idx = ((e - 1) / (2 * s)) * 2 * s + s; // Last odd index
        if q_idx >= e {
            q_idx = q_idx.saturating_sub(2 * s);
        }

        let mut a0 = 0i16;
        let mut a1 = 0i16;
        let mut a2 = 0i16;
        let mut a3 = 0i16;
        let mut b0 = 0i16;
        let mut b1 = 0i16;
        let mut b2 = 0i16;
        let mut b3 = 0i16;

        // Initialize for reverse pass (approximate last state)
        if q_idx + s3 < e {
            a3 = p_row[q_idx + s3];
        }
        if q_idx + s < e {
            a2 = p_row[q_idx + s];
        }
        a1 = p_row.get(q_idx.saturating_sub(s)).copied().unwrap_or(0);
        a0 = p_row.get(q_idx.saturating_sub(s3)).copied().unwrap_or(0);
        b3 = p_row[q_idx];
        b2 = p_row.get(q_idx.saturating_sub(2 * s)).copied().unwrap_or(0);
        b1 = p_row.get(q_idx.saturating_sub(4 * s)).copied().unwrap_or(0);
        b0 = p_row.get(q_idx.saturating_sub(6 * s)).copied().unwrap_or(0);

        // Reverse pass
        while q_idx >= s {
            // Undo Update (if applicable)
            if q_idx >= s3 {
                if let Some(index) = q_idx.checked_sub(s3) {
                    if index < p_row.len() {
                        let b_sum = b1 as i32 + b2 as i32;
                        let update =
                            (((b_sum << 3) + b_sum - b0 as i32 - b3 as i32 + 16) >> 5) as i16;
                        p_row[index] -= update; // Subtract instead of add
                    }
                }
            }

            // Undo Predict
            if q_idx + s3 < e {
                // Generic case
                let a_sum = a1 as i32 + a2 as i32;
                let delta = (((a_sum << 3) + a_sum - a0 as i32 - a3 as i32 + 8) >> 4) as i16;
                p_row[q_idx] += delta; // Add instead of subtract
            } else if q_idx < e {
                // Special case near end
                let a_sum = a1 as i32 + a2 as i32;
                let delta = ((a_sum + 1) >> 1) as i16;
                p_row[q_idx] += delta; // Add instead of subtract
            }

            // Shift variables for next iteration
            if q_idx >= 2 * s {
                q_idx = q_idx.saturating_sub(2 * s);
                a3 = a2;
                a2 = a1;
                a1 = a0;
                a0 = p_row.get(q_idx.saturating_sub(s3)).copied().unwrap_or(0);
                b3 = b2;
                b2 = b1;
                b1 = b0;
                b0 = p_row.get(q_idx.saturating_sub(s3)).copied().unwrap_or(0);
            } else {
                break;
            }
        }

        // Handle q_idx = s separately (first odd index)
        if q_idx == s && s < e {
            // Undo Update at 0
            let b_sum = b1 as i32 + b2 as i32;
            let update = (((b_sum << 3) + b_sum - b0 as i32 - b3 as i32 + 16) >> 5) as i16;
            p_row[0] -= update;

            // Undo Predict at s
            let a_sum = a1 as i32 + a2 as i32;
            let delta = ((a_sum + 1) >> 1) as i16;
            p_row[s] += delta;
        }

        y += scale;
    }
}
