use std::simd::{LaneCount, Simd, SupportedLaneCount};

use super::constants::IW_SHIFT;
use ::image::GrayImage;

// Use a type alias for Simd<i16, 4>
type I16x4 = Simd<i16, 4>;

/// Saturating conversion from i32 to i16 to prevent overflow
#[inline]
fn sat16(x: i32) -> i16 {
    if x > 32_767 { 
        32_767 
    } else if x < -32_768 { 
        -32_768 
    } else { 
        x as i16 
    }
}

pub struct Encode;

impl Encode {
    /// Forward wavelet transform using the lifting scheme.
    /// Port of `IW44Image::Transform::Encode::forward` from DjVuLibre.
    /// Uses vertical then horizontal filtering per scale, as per DjVu specification.
    /// This order is critical for preserving DC coefficients in solid color images.
    ///
    /// # Arguments
    /// * `buf` - Coefficient data to transform (modified in-place)
    /// * `w` - Image width
    /// * `h` - Image height
    /// * `levels` - Number of decomposition levels
    pub fn forward<const LANES: usize>(
        buf: &mut [i32],
        w: usize,
        h: usize,
        levels: usize,
    ) where
        LaneCount<LANES>: SupportedLaneCount,
    {
        // Work on progressively smaller low-pass rectangles
        let mut cur_w = w;
        let mut cur_h = h;

        for _level in 0..levels {
            let scale = 1; // After splitting each level operates on a decimated area

            // DjVu's IW44 applies a horizontal pass followed by a vertical
            // pass for each decomposition level.  After each pass, the
            // coefficients are split so that the low-pass portion occupies the
            // top-left corner for the next level.
            fwt_horizontal_inplace_single_level::<LANES>(buf, w, cur_w, cur_h, scale);
            split_horizontal::<LANES>(buf, w, cur_w, cur_h, scale);

            fwt_vertical_inplace_single_level::<LANES>(buf, w, cur_w, cur_h, scale);
            split_vertical::<LANES>(buf, w, cur_w, cur_h, scale);

            // Next level operates on the even samples only
            cur_w = (cur_w + 1) / 2;
            cur_h = (cur_h + 1) / 2;
        }
    }
    
    /// Prepare image data for wavelet transform with proper pixel shifting and centering.
    /// This handles the conversion from various input formats to the i32 buffer format expected by the transform.
    pub fn prepare_and_transform<F>(
        data32: &mut [i32], 
        w: usize, 
        h: usize, 
        pixel_fn: F
    ) where F: Fn(usize, usize) -> i32 
    {
        // Copy pixels with proper shifting
        for y in 0..h {
            for x in 0..w {
                data32[y * w + x] = pixel_fn(x, y);
            }
        }
        
        // Debug: Check input data before transform
        let sample_indices = [0, w/4, w/2, 3*w/4, w-1];
        let mut input_samples = Vec::new();
        let mut min_val = i32::MAX;
        let mut max_val = i32::MIN;
        let mut unique_vals = std::collections::HashSet::new();
        
        for y in 0..std::cmp::min(h, 5) { // Check first 5 rows
            for x in &sample_indices {
                if *x < w {
                    let val = data32[y * w + *x];
                    input_samples.push(val);
                    min_val = min_val.min(val);
                    max_val = max_val.max(val);
                    if unique_vals.len() < 10 {
                        unique_vals.insert(val);
                    }
                }
            }
        }
        
        println!("DEBUG Transform input (after {} shift):", IW_SHIFT);
        println!("  Input range: {} to {} (span: {})", min_val, max_val, max_val - min_val);
        println!("  Unique values: {:?}", unique_vals);
        println!("  Sample values: {:?}", input_samples);
        
        // Apply forward transform with appropriate number of levels
        // For WxH image, max levels = floor(log2(min(W, H)))
        let max_levels = ((w.min(h) as f64).log2().floor() as usize).max(1);
        Self::forward::<4>(data32, w, h, max_levels);
        
        // Debug: Check coefficients after transform
        let mut coeff_min = i32::MAX;
        let mut coeff_max = i32::MIN;
        let mut coeff_samples = Vec::new();
        let mut zero_count = 0;
        let mut nonzero_count = 0;
        
        for y in 0..std::cmp::min(h, 5) { // Check first 5 rows
            for x in &sample_indices {
                if *x < w {
                    let coeff = data32[y * w + *x];
                    coeff_samples.push(coeff);
                    coeff_min = coeff_min.min(coeff);
                    coeff_max = coeff_max.max(coeff);
                    if coeff == 0 {
                        zero_count += 1;
                    } else {
                        nonzero_count += 1;
                    }
                }
            }
        }
        
        // Check DC coefficient (should be average of image for solid color)
        let dc_coeff = data32[0];
        
        println!("DEBUG Transform output:");
        println!("  DC coefficient (0,0): {}", dc_coeff);
        println!("  Coeff range: {} to {} (span: {})", coeff_min, coeff_max, coeff_max as i32 - coeff_min as i32);
        println!("  Sample coefficients: {:?}", coeff_samples);
        println!("  Zero coeffs: {}, Nonzero coeffs: {}", zero_count, nonzero_count);
        
        // For solid color, we expect mostly zeros except for DC
        if zero_count > nonzero_count * 10 {
            println!("  *** SPARSE COEFFICIENTS - good for compression! ***");
        } else {
            println!("  *** WARNING: Too many non-zero coefficients for solid color! ***");
        }
    }
    
    /// Helper for unsigned 8-bit image data (like GrayImage)
    pub fn from_u8_image(img: &GrayImage, data32: &mut [i32], w: usize, h: usize) {
        Self::prepare_and_transform(data32, w, h, |x, y| {
            let pixel_u8 = img.get_pixel(x as u32, y as u32)[0] as i32;
            pixel_u8 << IW_SHIFT
        });
    }
    
    /// Helper for signed 8-bit channel data (Y, Cb, Cr)
    pub fn from_i8_channel(channel_buf: &[i8], data32: &mut [i32], w: usize, h: usize) {
        let img_w = if channel_buf.len() >= w * h { w } else { 
            // For smaller images, compute actual width
            let pixels = channel_buf.len();
            if pixels > 0 && h > 0 { 
                std::cmp::min(w, pixels / h)
            } else { 
                w 
            }
        };
        let img_h = if channel_buf.len() >= w * h { h } else {
            // For smaller images, compute actual height
            let pixels = channel_buf.len();
            if pixels > 0 && img_w > 0 { 
                std::cmp::min(h, pixels / img_w)
            } else { 
                h 
            }
        };
        
        Self::prepare_and_transform(data32, w, h, |x, y| {
            // Use mirroring for coordinates outside the actual image bounds
            let mirror_x = if x >= img_w {
                mirror(x as isize, img_w)
            } else {
                x
            };
            let mirror_y = if y >= img_h {
                mirror(y as isize, img_h)
            } else {
                y
            };
            
            let idx = mirror_y * img_w + mirror_x;
            if idx < channel_buf.len() {
                let pixel_i8 = channel_buf[idx];
                (pixel_i8 as i32) << IW_SHIFT
            } else {
                // Fallback: use last pixel value
                if channel_buf.len() > 0 {
                    (channel_buf[channel_buf.len() - 1] as i32) << IW_SHIFT
                } else {
                    0
                }
            }
        });
    }
}

/// Mirror index for boundaries: even symmetry around 0 and around size-1
/// Ported from DjVuLibre.
#[inline]
fn mirror(mut k: isize, size: usize) -> usize {
    if size == 0 {
        return 0;
    }
    let size = size as isize;
    if k < 0 {
        k = -k;
    }
    let period = (size - 1) * 2;
    k %= period;
    if k >= size {
        k = period - k;
    }
    k as usize
}

/// Convenience wrapper: *horizontal → vertical* forward IW-44 transform.
///
/// Call this instead of the two passes if you just want the full 2-D
/// decomposition in one line.
///
/// # Example
/// ```rust
/// fwt_forward_inplace::<8>(&mut image, w, h, /*levels=*/5);
/// quantise_inplace(&mut image, w, h, quant_table);
/// ```

/// In-place *vertical* pass for **one** decomposition level.
///
/// * `buf`  – pixel buffer (row-major).
/// * `full_w` – full image width (for row addressing).
/// * `work_w/work_h` – working rectangle dimensions to process.
/// * `scale` – current scale (1 << level).
///
/// Processes only the current level at the given scale.
pub fn fwt_vertical_inplace_single_level<const LANES: usize>(
    buf: &mut [i32],
    full_w: usize,
    work_w: usize,
    work_h: usize,
    scale: usize,
) where
    LaneCount<LANES>: SupportedLaneCount,
{
    let h0 = ((work_h - 1) / scale) + 1;
    if h0 < 2 {
        return;
    }

    let rs = full_w;

    // Predict step on odd rows
    for y in (1..h0).step_by(2) {
        for x in 0..work_w {
            let xe1 = buf[mirror(y as isize - 1, h0) * scale * rs + x];
            let xe0 = buf[mirror(y as isize + 1, h0) * scale * rs + x];
            let xe_1 = buf[mirror(y as isize - 3, h0) * scale * rs + x];
            let xe2 = buf[mirror(y as isize + 3, h0) * scale * rs + x];
            let pred = (-xe_1 + 9 * xe1 + 9 * xe0 - xe2 + 8) >> 4;
            let idx = y * scale * rs + x;
            buf[idx] -= pred;
        }
    }

    // Update step on even rows
    for y in (0..h0).step_by(2) {
        for x in 0..work_w {
            let d_1 = buf[mirror(y as isize - 1, h0) * scale * rs + x];
            let d0 = buf[mirror(y as isize + 1, h0) * scale * rs + x];
            let d_2 = buf[mirror(y as isize - 3, h0) * scale * rs + x];
            let d1 = buf[mirror(y as isize + 3, h0) * scale * rs + x];
            let upd = (-d_2 + 9 * d_1 + 9 * d0 - d1 + 16) >> 5;
            let idx = y * scale * rs + x;
            buf[idx] += upd;
        }
    }
}

/// In-place *horizontal* pass for **one** decomposition level.
///
/// * `buf`   – image plane, row-major, mutable so we can overwrite in place.
/// * `full_w` – full image width (for row addressing).
/// * `work_w/work_h` – working rectangle dimensions to process.
/// * `scale` – current scale (1 << level).
///
/// Processes only the current level at the given scale.
pub fn fwt_horizontal_inplace_single_level<const LANES: usize>(
    buf: &mut [i32],
    full_w: usize,
    work_w: usize,
    work_h: usize,
    scale: usize,
) where
    LaneCount<LANES>: SupportedLaneCount,
{
    let w0 = ((work_w - 1) / scale) + 1;
    if w0 < 2 {
        return;
    }

    let sc = scale;

    for row in 0..work_h {
        let base = row * full_w;

        // Predict step on odd columns
        for x in (1..w0).step_by(2) {
            let xe1 = buf[base + mirror(x as isize - 1, w0) * sc];
            let xe0 = buf[base + mirror(x as isize + 1, w0) * sc];
            let xe_1 = buf[base + mirror(x as isize - 3, w0) * sc];
            let xe2 = buf[base + mirror(x as isize + 3, w0) * sc];
            let pred = (-xe_1 + 9 * xe1 + 9 * xe0 - xe2 + 8) >> 4;
            let idx = base + x * sc;
            buf[idx] -= pred;
        }

        // Update step on even columns
        for x in (0..w0).step_by(2) {
            let d_1 = buf[base + mirror(x as isize - 1, w0) * sc];
            let d0 = buf[base + mirror(x as isize + 1, w0) * sc];
            let d_2 = buf[base + mirror(x as isize - 3, w0) * sc];
            let d1 = buf[base + mirror(x as isize + 3, w0) * sc];
            let upd = (-d_2 + 9 * d_1 + 9 * d0 - d1 + 16) >> 5;
            let idx = base + x * sc;
            buf[idx] += upd;
        }
    }
}

/// Rearrange coefficients after the horizontal pass so that even (low-pass)
/// samples come first followed by the odd (high-pass) samples. This matches
/// the layout expected by subsequent transform levels.
pub fn split_horizontal<const LANES: usize>(
    buf: &mut [i32],
    full_w: usize,
    work_w: usize,
    work_h: usize,
    scale: usize,
) where
    LaneCount<LANES>: SupportedLaneCount,
{
    let w0 = ((work_w - 1) / scale) + 1;
    if w0 < 2 {
        return;
    }

    let mut temp = vec![0i32; work_w];
    let step = scale * 2;

    for row in 0..work_h {
        let start = row * full_w;
        let row_slice = &mut buf[start..start + work_w];
        temp.copy_from_slice(row_slice);

        let mut idx = 0;
        let mut x = 0;
        while x < work_w {
            row_slice[idx] = temp[x];
            idx += 1;
            x += step;
        }

        x = scale;
        while x < work_w {
            row_slice[idx] = temp[x];
            idx += 1;
            x += step;
        }
    }
}

/// Rearrange coefficients after the vertical pass so that even (low-pass)
/// rows are stored first followed by the odd (high-pass) rows.
pub fn split_vertical<const LANES: usize>(
    buf: &mut [i32],
    full_w: usize,
    work_w: usize,
    work_h: usize,
    scale: usize,
) where
    LaneCount<LANES>: SupportedLaneCount,
{
    let h0 = ((work_h - 1) / scale) + 1;
    if h0 < 2 {
        return;
    }

    let step = scale * 2;
    let mut temp_col = vec![0i32; work_h];

    for col in 0..work_w {
        // gather column
        for y in 0..work_h {
            temp_col[y] = buf[y * full_w + col];
        }

        let mut idx = 0;
        let mut y = 0;
        while y < work_h {
            buf[idx * full_w + col] = temp_col[y];
            idx += 1;
            y += step;
        }

        y = scale;
        while y < work_h {
            buf[idx * full_w + col] = temp_col[y];
            idx += 1;
            y += step;
        }
    }
}
