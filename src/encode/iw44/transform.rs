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
    /// Fill data16 from a GrayImage (u8), centering values as needed for IW44.
    pub fn from_u8_image(img: &::image::GrayImage, data16: &mut [i16], w: usize, h: usize) {
        // Debug: print first 16 input pixels
        if let Ok(v) = std::env::var("IW44_INPUT_TRACE") {
            let v = v.trim();
            eprintln!("DEBUG: IW44_INPUT_TRACE env var found in from_u8_image: '{}'", v);
            if !(v.is_empty() || v == "0" || v.eq_ignore_ascii_case("false")) {
                eprint!("INPUT_TRACE rust first_16_pixels=[");
                for i in 0..16.min(w) {
                    let px = if i < img.width() as usize {
                        img.get_pixel(i as u32, 0)[0]
                    } else {
                        0
                    };
                    eprint!("{}{}", px, if i == 15 || i == w-1 { "" } else { ", " });
                }
                eprintln!("]");
            }
        }
        
        for y in 0..h {
            for x in 0..w {
                let px = if x < img.width() as usize && y < img.height() as usize {
                    img.get_pixel(x as u32, y as u32)[0]
                } else {
                    0
                };
                // Convert to signed and shift (matches C++: *p++ = (int)(row[j]) << iw_shift;)
                // Store as i16 (matching C++'s short* buffer)
                data16[y * w + x] = (((px as i32) - 128) << crate::encode::iw44::constants::IW_SHIFT) as i16;
            }
        }
    }

    /// Fill data16 from a GrayImage (u8) with stride, using border extension.
    pub fn from_u8_image_with_stride(
        img: &::image::GrayImage,
        data16: &mut [i16],
        w: usize,
        h: usize,
        stride: usize,
    ) {
        eprintln!("DEBUG: from_u8_image_with_stride called with {}x{}", w, h);
        data16.fill(0);

        // First, populate the actual image data
        for y in 0..h {
            for x in 0..w {
                let px = if x < img.width() as usize && y < img.height() as usize {
                    img.get_pixel(x as u32, y as u32)[0]
                } else {
                    0
                };
                // Convert to signed and shift (matches C++: *p++ = (int)(row[j]) << iw_shift;)
                data16[y * stride + x] = (((px as i32) - 128) << crate::encode::iw44::constants::IW_SHIFT) as i16;
            }
        }
        
        // Debug: print first 16 input pixels
        if let Ok(v) = std::env::var("IW44_INPUT_TRACE") {
            let v = v.trim();
            eprintln!("DEBUG: IW44_INPUT_TRACE env var found in from_u8_image_with_stride: '{}'", v);
            if !(v.is_empty() || v == "0" || v.eq_ignore_ascii_case("false")) {
                eprint!("INPUT_TRACE rust first_16_pixels=[");
                for i in 0..16.min(w) {
                    let px = if i < img.width() as usize {
                        img.get_pixel(i as u32, 0)[0]
                    } else {
                        0
                    };
                    eprint!("{}{}", px, if i == 15 || i == w-1 { "" } else { ", " });
                }
                eprintln!("]");
            }
        }

        // Extend borders to fill padding area (mirror/replicate edge pixels)
        let buffer_h = data16.len() / stride;

        // Extend right border (replicate rightmost column)
        for y in 0..h {
            for x in w..stride {
                data16[y * stride + x] = 0;
            }
        }

        // Extend bottom border (replicate bottom row)
        for y in h..buffer_h {
            for x in 0..stride {
                data16[y * stride + x] = 0;
            }
        }
    }

    /// Fill data16 from a signed i8 buffer with stride, using border extension.
    pub fn from_i8_channel_with_stride(
        channel_buf: &[i8],
        data16: &mut [i16],
        w: usize,
        h: usize,
        stride: usize,
    ) {
        data16.fill(0);

        // First, populate the actual image data
        // channel_buf is already signed i8, shift it (matches C++: *p++ = (int)(row[j]) << iw_shift;)
        for y in 0..h {
            for x in 0..w {
                let idx = y * w + x;
                let val = if idx < channel_buf.len() {
                    channel_buf[idx] as i32
                } else {
                    0
                };
                data16[y * stride + x] = (val << crate::encode::iw44::constants::IW_SHIFT) as i16;
            }
        }

        // Extend borders to fill padding area (mirror/replicate edge pixels)
        let buffer_h = data16.len() / stride;

        // Extend right border (replicate rightmost column)
        for y in 0..h {
            for x in w..stride {
                data16[y * stride + x] = 0;
            }
        }

        // Extend bottom border (replicate bottom row)
        for y in h..buffer_h {
            for x in 0..stride {
                data16[y * stride + x] = 0;
            }
        }
    }

    /// Forward wavelet transform using the streaming algorithm from DjVuLibre.
    /// Now operates on i16 throughout, matching C++'s short* buffer behavior.
    pub fn forward<const LANES: usize>(
        buf: &mut [i16],
        w: usize,
        h: usize,
        rowsize: usize,
        levels: usize,
    ) where
        LaneCount<LANES>: SupportedLaneCount,
    {
        let mut scale = 1;
        for _ in 0..levels {
            // Both passes operate on i16, matching C++ behavior
            filter_fh(buf, w, h, rowsize, scale);
            filter_fv(buf, w, h, rowsize, scale);
            scale <<= 1;
        }
    }

    /// Prepare image data and perform the wavelet transform.
    pub fn prepare_and_transform<F>(data16: &mut [i16], w: usize, h: usize, pixel_fn: F)
    where
        F: Fn(usize, usize) -> i32,
    {
        for y in 0..h {
            for x in 0..w {
                data16[y * w + x] = pixel_fn(x, y) as i16;
            }
        }
        Self::forward::<4>(data16, w, h, w, 5); // Default levels=5 as per DjVu spec
    }
}

/// Streaming horizontal filter - operates on i16 like C++ (port of filter_fh from IW44EncodeCodec.cpp:514)
fn filter_fh(buf: &mut [i16], w: usize, h: usize, mut rowsize: usize, scale: usize) {
    let s = scale;
    let s3 = s + s + s;
    rowsize *= scale;

    let mut y = 0usize;
    let mut p = 0usize;

    while y < h {
        let mut q = p + s;
        let e = p + w;

        // Use i32 for intermediate calculations to prevent overflow
        let mut a1 = 0i32;
        let mut a2 = 0i32;
        let mut a3 = 0i32;
        let mut b1 = 0i32;
        let mut b2 = 0i32;
        let mut b3 = 0i32;

        if q < e {
            a1 = buf[q - s] as i32;
            a2 = a1;
            a3 = a1;
            if q + s < e {
                a2 = buf[q + s] as i32;
            }
            if q + s3 < e {
                a3 = buf[q + s3] as i32;
            }
            b3 = (buf[q] as i32) - ((a1 + a2 + 1) >> 1);
            buf[q] = b3 as i16;  // Store back to i16 (plain cast, no saturation)
            q += s + s;
        }

        while q + s3 < e {
            let a0 = a1;
            a1 = a2;
            a2 = a3;
            a3 = buf[q + s3] as i32;
            let b0 = b1;
            b1 = b2;
            b2 = b3;
            // Prediction uses +8 >> 4 (matches C: ((a1+a2)<<3)+(a1+a2)-a0-a3+8)>>4)
            b3 = (buf[q] as i32) - ((((a1 + a2) << 3) + (a1 + a2) - a0 - a3 + 8) >> 4);
            buf[q] = b3 as i16;  // Store back to i16
            let idx_i = q as isize - s3 as isize;
            if idx_i >= 0 {
                let idx = idx_i as usize;
                // Update uses +16 >> 5 (matches C: ((b1+b2)<<3)+(b1+b2)-b0-b3+16)>>5)
                let updated = (buf[idx] as i32) + ((((b1 + b2) << 3) + (b1 + b2) - b0 - b3 + 16) >> 5);
                buf[idx] = updated as i16;  // Store back to i16
            }
            q += s + s;
        }

        while q < e {
            // Special case: w-3 <= x < w - use simple average for prediction
            a1 = a2;
            a2 = a3;
            let b0 = b1;
            b1 = b2;
            b2 = b3;
            // Simple average filter for prediction at boundary (matches C: ((a1+a2+1)>>1))
            b3 = (buf[q] as i32) - ((a1 + a2 + 1) >> 1);
            buf[q] = b3 as i16;
            let idx_i = q as isize - s3 as isize;
            if idx_i >= 0 {
                let idx = idx_i as usize;
                // Update still uses complex filter with +16 >> 5
                let updated = (buf[idx] as i32) + ((((b1 + b2) << 3) + (b1 + b2) - b0 - b3 + 16) >> 5);
                buf[idx] = updated as i16;
            }
            q += s + s;
        }

        while (q as isize) - (s3 as isize) < e as isize {
            // Special case: w <= x < w+3 - only update phase
            let b0 = b1;
            b1 = b2;
            b2 = b3;
            b3 = 0;
            let idx_i = q as isize - s3 as isize;
            if idx_i >= p as isize {
                let idx = idx_i as usize;
                // Complex update filter with +16 >> 5 (matches C)
                let updated = (buf[idx] as i32) + ((((b1 + b2) << 3) + (b1 + b2) - b0 - b3 + 16) >> 5);
                buf[idx] = updated as i16;
            }
            q += s + s;
        }

        y += scale;
        p += rowsize;
    }
}

/// Streaming vertical filter (port of filter_fv from IW44EncodeCodec.cpp:404)
fn filter_fv(buf: &mut [i16], w: usize, h: usize, rowsize: usize, scale: usize) {
    let s = scale * rowsize;
    let s3 = s + s + s;
    let mut y = 1usize;
    let mut p = s;
    let hlimit = ((h - 1) / scale) + 1;

    while y as isize - 3 < hlimit as isize {
        // 1-Delta (prediction)
        {
            let mut q = p;
            let e = q + w;
            if y >= 3 && y + 3 < hlimit {
                // Generic case: prediction uses +8>>4 (matches C)
                while q < e {
                    let a = buf[q - s] as i32 + buf[q + s] as i32;
                    let b = buf[q - s3] as i32 + buf[q + s3] as i32;
                    buf[q] = (buf[q] as i32 - (((a << 3) + a - b + 8) >> 4)) as i16;
                    q += scale;
                }
            } else if y < hlimit {
                // Special case: simple average
                let mut q1 = if y + 1 < hlimit { q + s } else { q - s };
                while q < e {
                    let a = buf[q - s] as i32 + buf[q1] as i32;
                    buf[q] = (buf[q] as i32 - ((a + 1) >> 1)) as i16;
                    q += scale;
                    q1 += scale;
                }
            }
        }

        // 2-Update
        {
            let q_i = p as isize - s3 as isize;
            if q_i >= 0 {
                let mut q = q_i as usize;
                let e = q + w;
                if y >= 6 && y < hlimit {
                    // Generic case: update uses +16>>5 (matches C)
                    while q < e {
                        let a = buf[q - s] as i32 + buf[q + s] as i32;
                        let b = buf[q - s3] as i32 + buf[q + s3] as i32;
                        buf[q] = (buf[q] as i32 + (((a << 3) + a - b + 16) >> 5)) as i16;
                        q += scale;
                    }
                } else if y >= 3 {
                    // Special cases with boundary handling
                    let mut q1 = if y - 2 < hlimit { Some(q + s) } else { None };
                    let mut q3 = if y < hlimit { Some(q + s3) } else { None };
                    if y >= 6 {
                        while q < e {
                            let a = buf[q - s] as i32 + q1.map_or(0, |idx| buf[idx] as i32);
                            let b = buf[q - s3] as i32 + q3.map_or(0, |idx| buf[idx] as i32);
                            // Update uses +16>>5
                            buf[q] = (buf[q] as i32 + (((a << 3) + a - b + 16) >> 5)) as i16;
                            q += scale;
                            if let Some(ref mut idx) = q1 {
                                *idx += scale;
                            }
                            if let Some(ref mut idx) = q3 {
                                *idx += scale;
                            }
                        }
                    } else if y >= 4 {
                        while q < e {
                            let a = buf[q - s] as i32 + q1.map_or(0, |idx| buf[idx] as i32);
                            let b = q3.map_or(0, |idx| buf[idx] as i32);
                            // Update uses +16>>5
                            buf[q] = (buf[q] as i32 + (((a << 3) + a - b + 16) >> 5)) as i16;
                            q += scale;
                            if let Some(ref mut idx) = q1 {
                                *idx += scale;
                            }
                            if let Some(ref mut idx) = q3 {
                                *idx += scale;
                            }
                        }
                    } else {
                        while q < e {
                            let a = q1.map_or(0, |idx| buf[idx] as i32);
                            let b = q3.map_or(0, |idx| buf[idx] as i32);
                            // Update uses +16>>5
                            buf[q] = (buf[q] as i32 + (((a << 3) + a - b + 16) >> 5)) as i16;
                            q += scale;
                            if let Some(ref mut idx) = q1 {
                                *idx += scale;
                            }
                            if let Some(ref mut idx) = q3 {
                                *idx += scale;
                            }
                        }
                    }
                }
            }
        }

        y += 2;
        p += s + s;
    }
}
/// Mirror index with even symmetry (DjVu style).
#[inline]
fn mirror(mut k: isize, size: usize) -> usize {
    if k < 0 {
        k = -k;
    }
    if k >= size as isize {
        k = (size as isize - 2) - (k - size as isize);
    }
    k as usize
}
