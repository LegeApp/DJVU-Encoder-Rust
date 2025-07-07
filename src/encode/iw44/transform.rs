use std::simd::{LaneCount, SupportedLaneCount};



/// Saturating conversion from i32 to i16 to prevent overflow
#[inline]
fn _sat16(x: i32) -> i16 {
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
    /// Fill data32 from a GrayImage (u8), centering values as needed for IW44.
    pub fn from_u8_image(img: &::image::GrayImage, data32: &mut [i32], w: usize, h: usize) {
        for y in 0..h {
            for x in 0..w {
                let px = if x < img.width() as usize && y < img.height() as usize {
                    img.get_pixel(x as u32, y as u32)[0]
                } else {
                    0
                };
                // Center pixel value for IW44: 0..255 -> -128..127
                data32[y * w + x] = (px as i32) - 128;
            }
        }
    }

    /// Fill data32 from a GrayImage (u8), centering values as needed for IW44.
    /// 
    /// # Arguments
    /// * `img` - Input grayscale image
    /// * `data32` - Output buffer (must be at least stride * h in size)
    /// * `w` - Image width (actual, not padded)
    /// * `h` - Image height (actual, not padded) 
    /// * `stride` - Row stride in the output buffer (typically padded width)
    pub fn from_u8_image_with_stride(img: &::image::GrayImage, data32: &mut [i32], w: usize, h: usize, stride: usize) {
        // Clear the buffer first to ensure padding is zero
        data32.fill(0);
        
        for y in 0..h {
            for x in 0..w {
                let px = if x < img.width() as usize && y < img.height() as usize {
                    img.get_pixel(x as u32, y as u32)[0]
                } else {
                    0
                };
                // Center pixel value for IW44: 0..255 -> -128..127
                data32[y * stride + x] = (px as i32) - 128;
            }
        }
    }

    /// Fill data32 from a signed i8 buffer, casting to i32.
    /// 
    /// # Arguments
    /// * `channel_buf` - Input buffer (must be at least w * h in size)
    /// * `data32` - Output buffer (must be at least stride * h in size)
    /// * `w` - Image width (actual, not padded)
    /// * `h` - Image height (actual, not padded)
    /// * `stride` - Row stride in the output buffer (typically padded width)
    pub fn from_i8_channel_with_stride(channel_buf: &[i8], data32: &mut [i32], w: usize, h: usize, stride: usize) {
        // Clear the buffer first to ensure padding is zero
        data32.fill(0);
        
        for y in 0..h {
            for x in 0..w {
                let idx = y * w + x;  // Index in input buffer (packed)
                let out_idx = y * stride + x;  // Index in output buffer (strided)
                let val = if idx < channel_buf.len() {
                    channel_buf[idx] as i32
                } else {
                    0
                };
                data32[out_idx] = val;
            }
        }
    }
    /// Forward wavelet transform using the lifting scheme.
    /// Port of `IW44Image::Transform::Encode::forward` from DjVuLibre.
    /// DjVu's IW44 performs the horizontal filter first, then the
    /// vertical filter for each decomposition level.
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

        for level in 0..levels {
            let _scale = 1 << level; // not used with packed implementation

            // DjVu's IW44 performs the horizontal filter first, then the
            // vertical filter for each decomposition level.
            fwt_horizontal_inplace_single_level::<LANES>(buf, w, cur_w, cur_h);
            fwt_vertical_inplace_single_level::<LANES>(buf, w, cur_w, cur_h);

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
        sample_indices.iter().for_each(|&x| {
            sample_indices.iter().for_each(|&y| {
                if x < w && y < h {
                    let val = data32[y * w + x];
                    input_samples.push(((x, y), val));
                    min_val = min_val.min(val);
                    max_val = max_val.max(val);
                }
            });
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

/// Forward Deslauriers-Dubuc (4,4) lifting on a single 1-D line.
fn forward_lift_line(line: &mut [i32]) {
    let n = line.len();
    if n < 2 { return; }

    let mut tmp = vec![0i32; n];

    // Predict step on odd indices
    for i in (1..n).step_by(2) {
        let xm1 = line[mirror(i as isize - 1, n)];
        let xp1 = line[mirror(i as isize + 1, n)];
        let xm3 = line[mirror(i as isize - 3, n)];
        let xp3 = line[mirror(i as isize + 3, n)];
        let pred = (-xm3 + 9 * xm1 + 9 * xp1 - xp3 + 8) >> 4;
        tmp[i] = line[i] - pred;
    }

    // Update step on even indices
    for i in (0..n).step_by(2) {
        let dm1 = tmp[mirror(i as isize - 1, n)];
        let dp1 = tmp[mirror(i as isize + 1, n)];
        let dm3 = tmp[mirror(i as isize - 3, n)];
        let dp3 = tmp[mirror(i as isize + 3, n)];
        let upd = (-dm3 + 9 * dm1 + 9 * dp1 - dp3 + 16) >> 5;
        tmp[i] = line[i] + upd;
    }

    // Pack: low-pass (even) then high-pass (odd)
    let mut j = 0;
    for i in (0..n).step_by(2) { line[j] = tmp[i]; j += 1; }
    for i in (1..n).step_by(2) { line[j] = tmp[i]; j += 1; }
}

/// In-place *vertical* pass for **one** decomposition level.
///
/// * `buf`  – pixel buffer (row-major).
/// * `full_w` – full image width (for row addressing).
/// * `work_w/work_h` – working rectangle dimensions to process.
/// Processes only the current level within the given rectangle.
pub fn fwt_vertical_inplace_single_level<const LANES: usize>(
    buf: &mut [i32],
    full_w: usize,
    work_w: usize,
    work_h: usize,
) where
    LaneCount<LANES>: SupportedLaneCount,
{
    if work_h < 2 { return; }

    // Process each column separately using a temporary buffer
    for x in 0..work_w {
        let mut column: Vec<i32> = (0..work_h).map(|y| buf[y * full_w + x]).collect();
        forward_lift_line(&mut column);
        for y in 0..work_h {
            buf[y * full_w + x] = column[y];
        }
    }
}

/// In-place *horizontal* pass for **one** decomposition level.
///
/// * `buf`   – image plane, row-major, mutable so we can overwrite in place.
/// * `full_w` – full image width (for row addressing).
/// * `work_w/work_h` – working rectangle dimensions to process.
/// Processes only the current level within the given rectangle.
pub fn fwt_horizontal_inplace_single_level<const LANES: usize>(
    buf: &mut [i32],
    full_w: usize,
    work_w: usize,
    work_h: usize,
) where
    LaneCount<LANES>: SupportedLaneCount,
{
    if work_w < 2 { return; }

    for row in 0..work_h {
        let start = row * full_w;
        forward_lift_line(&mut buf[start..start + work_w]);
    }
}
