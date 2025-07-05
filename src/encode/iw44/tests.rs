#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode::iw44::constants::*;
    use crate::encode::iw44::transform::*;
    use crate::encode::iw44::coeff_map::*;
    use crate::encode::iw44::codec::{Codec, ZERO, ACTIVE, NEW, UNK};
    use crate::encode::iw44::masking;
    use image::{GrayImage, RgbImage, Rgb};

    /// Test negative coefficient refinement in bit-plane encoding
    #[test]
    fn test_negative_coefficient_refinement() {
        // Create a test coefficient with negative value
        let test_coeff = -1024i16; // Binary: 1111110000000000
        
        // Test refinement at different bit-planes
        let reconstructed = -512i16; // Initial approximation
        
        // Test bit-plane 8 - check bit 8 in the absolute value
        let bit = 8;
        let abs_coeff = test_coeff.abs(); // 1024 = 0000010000000000
        let bit_val = ((abs_coeff >> bit) & 1) != 0;
        assert_eq!(bit_val, false, "Bit 8 should NOT be set in abs(1024) = {}", abs_coeff);
        
        // Test bit-plane 10 - bit 10 should be set in 1024
        let bit = 10;
        let bit_val = ((abs_coeff >> bit) & 1) != 0;
        assert_eq!(bit_val, true, "Bit 10 should be set in abs(1024) = {}", abs_coeff);
        
        // Apply correct refinement logic for bit 10
        let sign = if reconstructed < 0 { -1i16 } else { 1i16 };
        let abs_val = reconstructed.abs() as u16;
        let new_abs = abs_val | ((bit_val as u16) << bit);
        let new_reconstructed = sign * (new_abs as i16);
        
        // Should be -1536 (-512 | 1024 = -1536)
        assert_eq!(new_reconstructed, -1536, "Negative coefficient refinement failed");
    }

    /// Test zigzag reordering correctness
    #[test]
    fn test_zigzag_reordering() {
        // Test that zigzag table has correct length and bounds
        assert_eq!(ZIGZAG_LOC.len(), 1024, "Zigzag table should have 1024 entries");
        
        for (i, &loc) in ZIGZAG_LOC.iter().enumerate() {
            assert!(loc < 1024, "Zigzag location {} at index {} is out of bounds", loc, i);
        }
        
        // Test DC coefficient is at position 0
        assert_eq!(ZIGZAG_LOC[0], 0, "DC coefficient should be at zigzag position 0");
        
        // Test that all positions 0-1023 are represented exactly once
        let mut used = vec![false; 1024];
        for &loc in ZIGZAG_LOC.iter() {
            assert!(!used[loc as usize], "Zigzag location {} used twice", loc);
            used[loc as usize] = true;
        }
        assert!(used.iter().all(|&x| x), "Not all positions covered by zigzag");
    }

    /// Test wavelet transform symmetry: forward then backward should reconstruct
    #[test]
    fn test_wavelet_symmetry() {
        let width = 64;
        let height = 64;
        let mut original = vec![0i16; width * height];
        
        // Create a simple test pattern
        for y in 0..height {
            for x in 0..width {
                original[y * width + x] = ((x + y) % 256) as i16;
            }
        }
        
        let mut test_data = original.clone();
        
        // Apply forward transform
        Encode::forward(&mut test_data, width, height, width, 0, 5);
        
        // Apply backward transform
        Decode::backward(&mut test_data, width, height, width, 0, 5);
        
        // Check reconstruction (allowing for small numerical errors)
        for i in 0..original.len() {
            let diff = (original[i] - test_data[i]).abs();
            assert!(diff <= 1, "Wavelet reconstruction error too large at position {}: {} vs {}", 
                   i, original[i], test_data[i]);
        }
    }

    /// Test quantization thresholds at different bit-planes
    #[test]
    fn test_quantization_thresholds() {
        let codec = Codec::new(CoeffMap::new(32, 32));
        
        // Test that quantization thresholds decrease with bit-plane
        for i in 0..16 {
            for bit in 0..15 {
                let thresh1 = codec.quant_lo[i] >> bit;
                let thresh2 = codec.quant_lo[i] >> (bit + 1);
                assert!(thresh1 >= thresh2, 
                       "Quantization threshold should decrease with higher bit-planes");
            }
        }
        
        // Test high-frequency thresholds
        for band in 1..10 {
            for bit in 0..15 {
                let thresh1 = codec.quant_hi[band] >> bit;
                let thresh2 = codec.quant_hi[band] >> (bit + 1);
                assert!(thresh1 >= thresh2, 
                       "High-freq quantization threshold should decrease with higher bit-planes");
            }
        }
    }

    /// Test mask interpolation
    #[test]
    fn test_mask_interpolation() {
        let width = 16;
        let height = 16;
        let mut data = vec![100i16; width * height];
        let mut mask = vec![0i8; width * height];
        
        // Mask out center pixel
        mask[8 * width + 8] = 1;
        data[8 * width + 8] = 0; // Will be interpolated
        
        masking::interpolate_mask(&mut data, width, height, width, &mask, width);
        
        // Center pixel should now be interpolated from neighbors
        let interpolated = data[8 * width + 8];
        assert!(interpolated > 0 && interpolated <= 100, 
               "Masked pixel should be interpolated: got {}", interpolated);
    }

    /// Test coefficient state transitions
    #[test]
    fn test_coefficient_states() {
        // Test state constants
        assert_eq!(ZERO, 1);
        assert_eq!(ACTIVE, 2);
        assert_eq!(NEW, 4);
        assert_eq!(UNK, 8);
        
        // Test state combinations
        assert_eq!(NEW | UNK, 12);
        assert!((NEW | UNK) & NEW != 0);
        assert!((NEW | UNK) & UNK != 0);
        assert!((ACTIVE) & NEW == 0);
    }

    /// Test band bucket mapping
    #[test]
    fn test_band_buckets() {
        assert_eq!(BAND_BUCKETS.len(), 10, "Should have 10 bands");
        
        // Band 0 should have 16 buckets (0-15)
        assert_eq!(BAND_BUCKETS[0].start, 0);
        assert_eq!(BAND_BUCKETS[0].size, 16);
        
        // Check that all bands are covered
        let mut total_buckets = 0;
        for band in &BAND_BUCKETS {
            total_buckets += band.size;
        }
        assert_eq!(total_buckets, 64, "All 64 buckets should be covered");
    }

    /// Test mirror function boundary handling
    #[test]
    fn test_mirror_function() {
        // Test standard cases
        assert_eq!(mirror(0, 10), 0);
        assert_eq!(mirror(5, 10), 5);
        assert_eq!(mirror(9, 10), 9);
        
        // Test boundary reflection
        assert_eq!(mirror(-1, 10), 1);   // -(-1) = 1
        assert_eq!(mirror(-2, 10), 2);   // -(-2) = 2
        assert_eq!(mirror(10, 10), 8);   // 2*10-2-10 = 8
        assert_eq!(mirror(11, 10), 7);   // 2*10-2-11 = 7
        
        // Test edge cases
        assert_eq!(mirror(-1, 1), 0);   // max=1, so mirror(-1) should be 0
        assert_eq!(mirror(1, 1), 0);    // max=1, so mirror(1) should be 0
        assert_eq!(mirror(0, 1), 0);    // Valid index
    }

    /// Test RGB to YCbCr conversion
    #[test]
    fn test_rgb_to_ycbcr_conversion() {
        // Create a simple RGB image
        let mut img = RgbImage::new(4, 4);
        img.put_pixel(0, 0, Rgb([255, 0, 0]));    // Red
        img.put_pixel(1, 0, Rgb([0, 255, 0]));    // Green  
        img.put_pixel(2, 0, Rgb([0, 0, 255]));    // Blue
        img.put_pixel(3, 0, Rgb([128, 128, 128])); // Gray
        
        let mut y_buf = vec![0i8; 16];
        let mut cb_buf = vec![0i8; 16];
        let mut cr_buf = vec![0i8; 16];
        
        rgb_to_ycbcr_buffers(&img, &mut y_buf, &mut cb_buf, &mut cr_buf);
        
        // Test that conversions are in valid range
        for i in 0..16 {
            assert!(y_buf[i] >= -128 && y_buf[i] <= 127, "Y out of range: {}", y_buf[i]);
            assert!(cb_buf[i] >= -128 && cb_buf[i] <= 127, "Cb out of range: {}", cb_buf[i]);
            assert!(cr_buf[i] >= -128 && cr_buf[i] <= 127, "Cr out of range: {}", cr_buf[i]);
        }
        
        // Gray pixel should have near-zero chrominance
        let gray_cb = cb_buf[3];
        let gray_cr = cr_buf[3];
        assert!(gray_cb.abs() <= 2, "Gray pixel should have low Cb: {}", gray_cb);
        assert!(gray_cr.abs() <= 2, "Gray pixel should have low Cr: {}", gray_cr);
    }

    /// Test coefficient magnitude calculation
    #[test]
    fn test_coefficient_magnitude() {
        // Test various coefficient values
        let test_cases = vec![
            (1024, 10, true),   // 1024 >> 10 = 1, should be significant
            (512, 10, false),   // 512 >> 10 = 0, should not be significant  
            (-1024, 10, true),  // abs(-1024) >> 10 = 1, should be significant
            (0, 5, false),      // 0 should never be significant
        ];
        
        for (coeff, bit, expected) in test_cases {
            let threshold = 1;
            let is_significant = (coeff as i32).abs() >= (threshold << bit);
            assert_eq!(is_significant, expected, 
                      "Coefficient {} at bit-plane {} significance test failed", coeff, bit);
        }
    }

    /// Integration test: encode a simple pattern
    #[test]
    fn test_encode_simple_pattern() {
        // Create a simple checkerboard pattern
        let width = 32;
        let height = 32;
        let mut img_data = vec![0u8; (width * height) as usize];
        
        for y in 0..height {
            for x in 0..width {
                let val = if (x + y) % 2 == 0 { 255 } else { 0 };
                img_data[(y * width + x) as usize] = val;
            }
        }
        
        let gray_img = GrayImage::from_raw(width, height, img_data).unwrap();
        
        // Test that coefficient map creation doesn't panic
        let coeff_map = CoeffMap::create_from_image(&gray_img, None);
        
        // Check basic properties
        assert_eq!(coeff_map.width(), width as usize);
        assert_eq!(coeff_map.height(), height as usize);
        assert_eq!(coeff_map.num_blocks, 1); // 32x32 = 1 block
        
        // Check that we have coefficients
        let block = &coeff_map.blocks[0];
        let dc_bucket = block.get_bucket(0);
        assert!(dc_bucket.is_some(), "Should have DC coefficients");
    }
}
