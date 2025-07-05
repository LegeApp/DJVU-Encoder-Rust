// Remove unused imports
use rayon::prelude::*;
use std::simd::Simd;

// Use a type alias for Simd<i16, 4>
type I16x4 = Simd<i16, 4>;

pub struct Encode;

impl Encode {
    /// Forward wavelet transform using the lifting scheme.
    /// Port of `IW44Image::Transform::Encode::forward` from DjVuLibre.
    /// Uses horizontal then vertical filtering per scale, as per DjVu specification.
    ///
    /// # Arguments
    /// * `p` - Coefficient data to transform (modified in-place)
    /// * `w` - Image width
    /// * `h` - Image height
    /// * `rowsize` - Row stride 
    /// * `begin` - Starting scale index (0 = scale 1, 1 = scale 2, etc.)
    /// * `end` - Ending scale index (exclusive)
    pub fn forward(p: &mut [i16], w: usize, h: usize, rowsize: usize, begin: usize, end: usize) {
        #[cfg(debug_assertions)]
        {
            println!("DEBUG: Starting wavelet transform. First 3 before: {}, {}, {}", 
                     p[0], p[1], p[2]);
        }
        
        for i in begin..end {
            let scale = 1 << i;
            #[cfg(debug_assertions)]
            println!("DEBUG: Transform scale {}", scale);
            filter_fv(p, w, h, rowsize, scale);
            filter_fh_parallel(p, w, h, rowsize, scale);
            
            #[cfg(debug_assertions)]
            if scale <= 8 {  // Only log first few scales
                println!("DEBUG: After scale {} - first 3: {}, {}, {}", 
                         scale, p[0], p[1], p[2]);
            }
        }

        #[cfg(debug_assertions)]
        println!("DEBUG: Wavelet transform complete. First 3 after: {}, {}, {}", 
                 p[0], p[1], p[2]);
    }
}
/// Optimized symmetric boundary extension (mirroring) for wavelet transforms.
/// This function handles out-of-bounds indices by reflecting them back into the valid range.
/// This is essential for the lifting scheme used in IW44 wavelets to avoid boundary artifacts.
///
/// # Arguments
/// * `val` - The potentially out-of-bounds index
/// * `max` - The maximum valid index (exclusive)
///
/// # Returns
/// A valid index within [0, max) using symmetric mirroring
#[inline]
pub fn mirror(i: isize, max: isize) -> usize {
    if max <= 0 {
        return 0; // Safe fallback for invalid max values
    }
    let mut v = i;
    
    // Handle negative indices by reflection: -1 -> 0, -2 -> 1, etc.
    if v < 0 {
        v = -v - 1;
    }
    
    // Handle indices >= max by reflection
    if v >= max {
        v = 2 * max - 1 - v;
    }
    
    // Clamp to ensure valid range [0, max)
    if v < 0 || v >= max {
        v = 0; // Fallback for edge cases
    }
    
    v as usize
}

/// Vertical lifting filter for forward transform.
/// Implements the Deslauriers-Dubuc (4,4) interpolating wavelet lifting scheme.
/// Uses symmetric boundary extension via the mirror function.
///
/// Predict:  d[n] = x_o[n] - (-x_e[n-1] + 9*x_e[n] + 9*x_e[n+1] - x_e[n+2]) / 16
/// Update:   x_e[n] += (-d[n-1] + 9*d[n] + 9*d[n+1] - d[n+2]) / 32
///
/// # Arguments
/// * `p` - Coefficient data (modified in-place)
/// * `w` - Image width
/// * `h` - Image height
/// * `rowsize` - Row stride
/// * `scale` - Current scale (1, 2, 4, 8, 16, ...)
// Define SIMD types for 16-bit integers (4 lanes to match MMX)
// Type alias removed (moved to top of file)

/// Vertical filter function for wavelet transform, refactored to use SIMD and parallelization.
///
/// # Arguments
/// - `p`: Mutable slice of i16 representing the 2D image buffer.
/// - `w`: Width of the image in pixels.
/// - `h`: Height of the image in pixels.
/// - `rowsize`: Number of elements (shorts) per row.
/// - `scale`: Scaling factor for the transform.
fn filter_fv(p: &mut [i16], w: usize, h: usize, rowsize: usize, scale: usize) {
    let s = scale * rowsize;
    let s3 = 3 * s;
    let mut y = 0;
    let mut p_idx = 0;
    let h_scaled = ((h - 1) / scale) + 1;

    while y < h_scaled {
        // 1. Lifting Step
        {
            let row_y = p_idx;
            let row_start = row_y;
            let row_end = row_start + w;

            if y >= 3 && y + 3 < h_scaled {
                // Generic case: Use SIMD and parallelization when scale == 1
                if scale == 1 {
                    // Process in chunks for better cache locality
                    let chunk_size = 64;
                    let chunks: Vec<_> = (0..w).collect();
                    let chunks = chunks.chunks(chunk_size).collect::<Vec<_>>();
                    let chunk_results: Vec<Vec<i16>> = chunks.par_iter().map(|chunk| {
                        let start = chunk[0];
                        let end = chunk[chunk.len() - 1] + 1;
                        let mut temp = vec![0i16; end - start];
                        let mut i = start;
                        // SIMD processing for blocks of 4 columns
                        while i + 4 <= end && i + 4 <= w {
                            unsafe {
                                let b = I16x4::from_slice(&p[(row_y - s + i)..(row_y - s + i + 4)]);
                                let c = I16x4::from_slice(&p[(row_y + s + i)..(row_y + s + i + 4)]);
                                let a = I16x4::from_slice(&p[(row_y - s3 + i)..(row_y - s3 + i + 4)]);
                                let d = I16x4::from_slice(&p[(row_y + s3 + i)..(row_y + s3 + i + 4)]);

                                let sum_bc = b + c;
                                let sum_ad = a + d;

                                // Calculate the new values using SIMD operations
                                let temp_simd = {
                                    let sum_bc = b + c;
                                    let sum_ad = a + d;
                                    let temp = (sum_bc * Simd::splat(9) - sum_ad + Simd::splat(16)) >> 5;
                                    let q = I16x4::from_slice(&p[row_y + i..row_y + i + 4]);
                                    q - temp
                                };

                                // Store SIMD result back to memory
                                for j in 0..4 {
                                    temp[(i + j) - start] = temp_simd[j];
                                }
                            }
                            i += 4;
                        }

                        // Remaining elements in the chunk (less than 4)
                        while i < end {
                            let a = p[row_y - s + i] as i32 + p[row_y + s + i] as i32;
                            let b = p[row_y - s3 + i] as i32 + p[row_y + s3 + i] as i32;
                            temp[i - start] = p[row_y + i] - (((a * 9) - b + 16) >> 5) as i16;
                            i += 1;
                        }
                        temp
                    }).collect();
                    // Copy results back to original array
                    for (i, val) in chunk_results.iter().enumerate() {
                        for (j, &v) in val.iter().enumerate() {
                            if i * chunk_size + j < w {
                                p[row_y + i * chunk_size + j] = v;
                            }
                        }
                    }
                } else {
                    // Scalar code for scale != 1
                    let mut i = row_start;
                    while i < row_end {
                        let a = p[i - s] as i32 + p[i + s] as i32;
                        let b = p[i - s3] as i32 + p[i + s3] as i32;
                        p[i] -= (((a * 9) - b + 16) >> 5) as i16;
                        i += scale;
                    }
                }
            } else if y < h_scaled {
                // Boundary cases: Scalar processing
                let mut i = row_start;
                let q1_offset = if y + 1 < h_scaled { s } else { 0 };
                let q3_offset = if y + 3 < h_scaled { s3 } else { 0 };

                if y >= 3 {
                    while i < row_end {
                        let a = p[i - s] as i32 + if q1_offset != 0 { p[i + q1_offset] as i32 } else { 0 };
                        let b = p[i - s3] as i32 + if q3_offset != 0 { p[i + q3_offset] as i32 } else { 0 };
                        p[i] -= (((a * 9) - b + 16) >> 5) as i16;
                        i += scale;
                    }
                } else if y >= 1 {
                    while i < row_end {
                        let a = p[i - s] as i32 + if q1_offset != 0 { p[i + q1_offset] as i32 } else { 0 };
                        let b = if q3_offset != 0 { p[i + q3_offset] as i32 } else { 0 };
                        p[i] -= (((a * 9) - b + 16) >> 5) as i16;
                        i += scale;
                    }
                } else {
                    while i < row_end {
                        let a = if q1_offset != 0 { p[i + q1_offset] as i32 } else { 0 };
                        let b = if q3_offset != 0 { p[i + q3_offset] as i32 } else { 0 };
                        p[i] -= (((a * 9) - b + 16) >> 5) as i16;
                        i += scale;
                    }
                }
            }
        }

        // 2. Interpolation Step
        {
            let row_y = p_idx.saturating_sub(s3);
            let row_start = row_y;
            let row_end = row_start + w;

            if y >= 6 && y < h_scaled {
                // Generic case: Use SIMD and parallelization when scale == 1
                if scale == 1 {
                    // Process in chunks for better cache locality
                    let chunk_size = 64;
                    let chunks: Vec<_> = (0..w).collect();
                    let chunks = chunks.chunks(chunk_size).collect::<Vec<_>>();
                    let chunk_results: Vec<Vec<i16>> = chunks.par_iter().map(|chunk| {
                        let start = chunk[0];
                        let end = chunk[chunk.len() - 1] + 1;
                        let mut temp = vec![0i16; end - start];
                        let mut i = start;
                        // SIMD processing for blocks of 4 columns
                        while i + 4 <= end && i + 4 <= w {
                            unsafe {
                                let b = I16x4::from_slice(&p[(row_y - s + i)..(row_y - s + i + 4)]);
                                let c = I16x4::from_slice(&p[(row_y + s + i)..(row_y + s + i + 4)]);
                                let a = I16x4::from_slice(&p[(row_y - s3 + i)..(row_y - s3 + i + 4)]);
                                let d = I16x4::from_slice(&p[(row_y + s3 + i)..(row_y + s3 + i + 4)]);

                                let sum_bc = b + c;
                                let sum_ad = a + d;

                                // Calculate the new values using SIMD operations
                                let result = {
                                    let sum_bc = b + c;
                                    let sum_ad = a + d;
                                    let temp = (sum_bc * Simd::splat(9) - sum_ad + Simd::splat(8)) >> 4;
                                    let q = I16x4::from_slice(&p[row_y + i..row_y + i + 4]);
                                    q + temp
                                };

                                // Store SIMD result back to memory
                                for j in 0..4 {
                                    temp[(i + j) - start] = result[j];
                                }
                            }
                            i += 4;
                        }

                        // Remaining elements in the chunk (less than 4)
                        while i < end {
                            let a = p[row_y - s + i] as i32 + p[row_y + s + i] as i32;
                            let b = p[row_y - s3 + i] as i32 + p[row_y + s3 + i] as i32;
                            temp[i - start] = p[row_y + i] + (((a * 9) - b + 8) >> 4) as i16;
                            i += 1;
                        }
                        temp
                    }).collect();
                    // Copy results back to original array
                    for (i, val) in chunk_results.iter().enumerate() {
                        for (j, &v) in val.iter().enumerate() {
                            if i * chunk_size + j < w {
                                p[row_y + i * chunk_size + j] = v;
                            }
                        }
                    }
                } else {
                    // Scalar code for scale != 1
                    let mut i = row_start;
                    while i < row_end {
                        let a = p[i - s] as i32 + p[i + s] as i32;
                        let b = p[i - s3] as i32 + p[i + s3] as i32;
                        p[i] += (((a * 9) - b + 8) >> 4) as i16;
                        i += scale;
                    }
                }
            } else if y >= 4 {
                // Boundary cases: Scalar processing
                let mut i = row_start;
                // Convert to isize to allow negative offsets
                let q1_offset = if y >= 2 && y - 2 < h_scaled { 
                    s 
                } else if y < 2 { 
                    // Convert to isize for negative offset, then back to usize with bounds checking
                    (y as isize - s as isize).max(0) as usize 
                } else { 
                    0 
                };
                let q3_offset = if y + 2 < h_scaled { s3 } else { 0 };
                while i < row_end {
                    let a = p[i - s] as i32 + p[i + q1_offset] as i32;
                    p[i] += ((a + 1) >> 1) as i16;
                    i += scale;
                }
            }
        }

        y += 2;
        p_idx += 2 * s;
    }
}


// Define a common SIMD type alias for clarity, matching the provided code.


/// Horizontal filter for the IW44 wavelet transform.
///
/// This function applies the lifting and update steps of the wavelet transform horizontally
/// across each specified row of the image buffer. It is parallelized using Rayon to process
/// multiple rows concurrently.
///
/// The logic from the original C code, which interleaves two dependent computation steps,
/// has been separated into a clearer two-pass system (Lifting, then Update) to ensure
/// data correctness in a parallel context and improve code clarity.
///
/// Due to the non-contiguous (strided) memory access pattern inherent in the horizontal
/// filter (e.g., accessing `p[x-s]`, `p[x]`, `p[x+s]`), this implementation uses a scalar
/// approach within each row. Unlike the vertical filter, standard SIMD loads are not
/// effective here without complex, and often inefficient, gather/shuffle operations. The
/// primary performance gain comes from processing rows in parallel.
///
/// # Arguments
/// - `p`: Mutable slice of `i16` representing the 2D image buffer.
/// - `w`: Width of the image in pixels.
/// - `h`: Height of the image in pixels.
/// - `rowsize`: The number of `i16` elements per row in memory (stride).
/// - `scale`: The scaling factor `s` for the current wavelet level.
fn filter_fh_parallel(p: &mut [i16], w: usize, h: usize, rowsize: usize, scale: usize) {
    if w == 0 || h == 0 {
        return;
    }

    // Process rows in parallel. The C code iterates `y += scale`, so we process
    // blocks of `scale` rows and operate on the first row of each block.
    // `par_chunks_mut` provides safe, mutable, and disjoint slices for each thread.
    p.par_chunks_mut(rowsize * scale)
        .take(h / scale) // Ensure we don't process padding rows if h is not a multiple of scale
        .for_each(|block| {
            // Each thread operates on a single row from its assigned block.
            let row = &mut block[0..w];
            filter_fh(row, w, scale);
        });
}

/// Processes a single row with the horizontal wavelet filter logic.
///
/// This function contains the core two-pass algorithm:
/// 1.  **Lifting Pass**: Computes the new values for the even-indexed coefficients based on
///     their odd-indexed neighbors. The results are stored in a temporary buffer to prevent
///     modifying the row before the next pass has read all original values.
/// 2.  **Update Pass**: Writes the new even coefficients back to the row and then calculates
///     the update for the odd-indexed coefficients using these new values.
fn filter_fh(row: &mut [i16], w: usize, scale: usize) {
    let s = scale;
    let s2 = 2 * s;
    let s3 = 3 * s;

    // A temporary buffer to hold the new values for the even-indexed coefficients (`b` values).
    // This de-interleaves the calculation, which is crucial for correctness and clarity.
    let num_even = (w + s2 - 1) / s2;
    let mut b_coeffs = Vec::with_capacity(num_even);

    // --- Pass 1: Lifting Step ---
    // Calculate new coefficients for all even positions `x = 0, 2s, 4s, ...`
    // We read only from the original `row` and store results in `b_coeffs`.
    for i in 0..num_even {
        let x = i * s2;

        // Boundary-safe reads from the original row, mimicking the C code's zero-padding.
        let p_x_minus_s3 = if x >= s3 { row[x - s3] } else { 0 };
        let p_x_minus_s = if x >= s { row[x - s] } else { 0 };
        // The check `x + s < w` is sufficient as `x` is always less than `w` for the last element.
        let p_x_plus_s = if x + s < w { row[x + s] } else { 0 };
        let p_x_plus_s3 = if x + s3 < w { row[x + s3] } else { 0 };

        let neighbors_1s = p_x_minus_s as i32 + p_x_plus_s as i32;
        let neighbors_3s = p_x_minus_s3 as i32 + p_x_plus_s3 as i32;

        // The lifting formula from the IW44 update step.
        let delta = (((neighbors_1s * 9) - neighbors_3s + 16) >> 5) as i16;
        b_coeffs.push(row[x].wrapping_sub(delta));
    }

    // --- Pass 2: Update Step ---
    // First, write the new `b` coefficients back to the even positions in the row.
    for i in 0..num_even {
        let x = i * s2;
        if x < w {
            row[x] = b_coeffs[i];
        }
    }

    // Now, update the odd-indexed positions using the new `b` coefficients.
    // The C code uses different formulas at the start, middle, and end of the row.

    // Generic updates for the main part of the row.
    // An update at `x_update = (i * 2s) - 3s` requires `b` coeffs from `i-3` to `i`.
    for i in 3..num_even {
        let x_update = (i * s2) - s3;
        if x_update < w {
            let b0 = b_coeffs[i - 3] as i32;
            let b1 = b_coeffs[i - 2] as i32;
            let b2 = b_coeffs[i - 1] as i32;
            let b3 = b_coeffs[i] as i32;

            let b_sum_12 = b1 + b2;
            let b_sum_03 = b0 + b3;
            // The interpolation formula from the C code's generic case.
            let update_val = (((b_sum_12 * 9) - b_sum_03 + 8) >> 4) as i16;
            row[x_update] = row[x_update].wrapping_add(update_val);
        }
    }

    // Boundary updates at the start and end of the row, which use a simplified formula.
    // This logic corresponds to the special-case loops in the C implementation.
    let simplified_update = |b1, b2| (((b1 as i32 + b2 as i32 + 1) >> 1) as i16);

    // Update for position `s` (corresponds to `q` at `4s` in C, so `i=2`).
    if 2 < num_even {
        let x_update = s;
        if x_update < w {
            let b1 = b_coeffs[0]; // from x=0
            let b2 = b_coeffs[1]; // from x=2s
            row[x_update] = row[x_update].wrapping_add(simplified_update(b1, b2));
        }
    }
    
    // Trailing updates for odd positions near the end of the row.
    // This corresponds to the `while (q - s3 < e)` loop in the C code.
    for i in num_even..(num_even + 2) {
        let x_update = (i * s2) - s3;
        if x_update < w {
            // These `b` values are conceptually outside the main `b_coeffs` array.
            // We get as many as are available and assume the rest are zero.
            let b1 = b_coeffs.get(i - 2).copied().unwrap_or(0);
            let b2 = b_coeffs.get(i - 1).copied().unwrap_or(0);
            row[x_update] = row[x_update].wrapping_add(simplified_update(b1, b2));
        }
    }
}
