// src/iw44/transform.rs

use image::{GrayImage, Luma, Rgb, RgbImage};

// YCbCr color conversion constants from C++ rgb_to_ycc
const RGB_TO_YCC: [[f32; 3]; 3] = [
    [0.304348, 0.608696, 0.086956],  // Y
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

/// Converts RGB image to Y (luminance) channel using fixed-point arithmetic.
/// Outputs to a mutable i8 slice for signed values (-128 to 127).
pub fn rgb_to_y(img: &RgbImage, out: &mut [i8]) {
    let (w, h) = img.dimensions();
    let (rmul, gmul, bmul) = precompute_tables();

    let mut idx = 0;
    for y in 0..h {
        for x in 0..w {
            let Rgb([r, g, b]) = img.get_pixel(x, y);
            let r_idx = *r as usize;
            let g_idx = *g as usize;
            let b_idx = *b as usize;
            // Fixed-point computation: sum, round, shift, and offset
            let y_val = rmul[r_idx] + gmul[g_idx] + bmul[b_idx] + 32768;
            out[idx] = ((y_val >> 16) - 128) as i8;
            idx += 1;
        }
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
pub fn rgb_to_cb(img: &RgbImage, out: &mut [i8]) {
    let (w, h) = img.dimensions();
    let (rmul, gmul, bmul) = precompute_cb_tables();

    let mut idx = 0;
    for y in 0..h {
        for x in 0..w {
            let Rgb([r, g, b]) = img.get_pixel(x, y);
            let r_idx = *r as usize;
            let g_idx = *g as usize;
            let b_idx = *b as usize;
            let cb_val = rmul[r_idx] + gmul[g_idx] + bmul[b_idx] + 32768;
            // Clamp to [-128, 127] as per C++ max/min
            out[idx] = (cb_val >> 16).clamp(-128, 127) as i8;
            idx += 1;
        }
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

/// Converts RGB image to Cr (red-difference) channel using fixed-point arithmetic.
pub fn rgb_to_cr(img: &RgbImage, out: &mut [i8]) {
    let (w, h) = img.dimensions();
    let (rmul, gmul, bmul) = precompute_cr_tables();

    let mut idx = 0;
    for y in 0..h {
        for x in 0..w {
            let Rgb([r, g, b]) = img.get_pixel(x, y);
            let r_idx = *r as usize;
            let g_idx = *g as usize;
            let b_idx = *b as usize;
            let cr_val = rmul[r_idx] + gmul[g_idx] + bmul[b_idx] + 32768;
            out[idx] = (cr_val >> 16).clamp(-128, 127) as i8;
            idx += 1;
        }
    }
}

/// Replicates the C++ `filter_fv` (Forward Vertical)
pub fn filter_fv(p_slice: &mut [i16], w: usize, h: usize, rowsize: usize, scale: usize) {
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
                let a = p_slice[q_idx - s] as i32 + p_slice[q_idx + s] as i32;
                let b = p_slice[q_idx - s3] as i32 + p_slice[q_idx + s3] as i32;
                p_slice[q_idx] -= (((a << 3) + a - b + 8) >> 4) as i16;
            }
        } else {
            // Special cases near boundaries
            for x_idx in (0..w).step_by(scale) {
                let q_idx = p_offset + x_idx;
                let prev_s = p_slice[q_idx - s];
                let next_s = if y_idx + 1 < effective_h { p_slice[q_idx + s] } else { prev_s };
                let a = prev_s as i32 + next_s as i32;
                p_slice[q_idx] -= ((a + 1) >> 1) as i16;
            }
        }
        
        // 2-Update
        let p_update_offset = p_offset - s3;
        if y_idx >= 6 && y_idx < effective_h {
             // Generic case
            for x_idx in (0..w).step_by(scale) {
                let q_idx = p_update_offset + x_idx;
                let a = p_slice[q_idx - s] as i32 + p_slice[q_idx + s] as i32;
                let b = p_slice[q_idx - s3] as i32 + p_slice[q_idx + s3] as i32;
                p_slice[q_idx] += (((a << 3) + a - b + 16) >> 5) as i16;
            }
        } else if y_idx >= 3 {
            // Special cases for update
            for x_idx in (0..w).step_by(scale) {
                let q_idx = p_update_offset + x_idx;
                let a = (if y_idx >= 4 { p_slice.get(q_idx - s).copied() } else { None }).unwrap_or(0) as i32 +
                        (if y_idx - 2 < effective_h { p_slice.get(q_idx + s).copied() } else { None }).unwrap_or(0) as i32;
                let b = (if y_idx >= 6 { p_slice.get(q_idx - s3).copied() } else { None }).unwrap_or(0) as i32 +
                        (if y_idx < effective_h { p_slice.get(q_idx + s3).copied() } else { None }).unwrap_or(0) as i32;
                
                p_slice[q_idx] += (((a << 3) + a - b + 16) >> 5) as i16;
            }
        }
    }
}

/// Forward horizontal filter, matching C++ filter_fh
pub fn filter_fh(p_slice: &mut [i16], w: usize, h: usize, rowsize: usize, scale: usize) {
    let s = scale;
    let s3 = 3 * s;
    let mut y = 0;

    while y < h {
        let row_start = y * rowsize;
        let p_row = &mut p_slice[row_start..row_start + w];
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
            if q_idx + s < e { a2 = p_row[q_idx + s]; }
            if q_idx + s3 < e { a3 = p_row[q_idx + s3]; }
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
            p_row[q_idx - s3] += update;
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
                let b_sum = b1 as i32 + b2 as i32;
                let update = (((b_sum << 3) + b_sum - b0 as i32 - b3 as i32 + 16) >> 5) as i16;
                p_row[q_idx - s3] += update;
            }
            q_idx += 2 * s;
        }

        // Special case: w <= x < w+3
        while q_idx - s3 < e {
            b0 = b1;
            b1 = b2;
            b2 = b3;
            b3 = 0;
            if q_idx >= s3 {
                let b_sum = b1 as i32 + b2 as i32;
                let update = (((b_sum << 3) + b_sum - b0 as i32 - b3 as i32 + 16) >> 5) as i16;
                p_row[q_idx - s3] += update;
            }
            q_idx += 2 * s;
        }

        y += scale;
    }
}

/// Combined forward wavelet transform (unchanged from original, included for completeness)
pub fn forward(p_slice: &mut [i16], w: usize, h: usize, rowsize: usize, begin_scale: usize, end_scale: usize) {
    let mut scale = begin_scale;
    while scale < end_scale {
        filter_fh(p_slice, w, h, rowsize, scale);
        filter_fv(p_slice, w, h, rowsize, scale);
        scale *= 2;
    }
}