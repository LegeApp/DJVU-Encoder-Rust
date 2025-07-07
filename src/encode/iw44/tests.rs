#[cfg(test)]
mod tests {
    use crate::encode::iw44::encoder::{rgb_to_ycbcr_planes, ycbcr_from_rgb, EncoderParams, CrcbMode};
    use image::{RgbImage, ImageBuffer, Rgb, Luma};

    /// Test color conversion with known values
    #[test]
    fn test_rgb_to_ycbcr_conversion() {
        // Test pure red (255, 0, 0)
        let red_rgb = [255u8, 0, 0];
        let mut y = [0i8; 1];
        let mut cb = [0i8; 1];
        let mut cr = [0i8; 1];

        rgb_to_ycbcr_planes(&red_rgb, &mut y, &mut cb, &mut cr);

        // Expected values for pure red using ITU-R BT.601 conversion
        // Y = 0.299*255 + 0.587*0 + 0.114*0 = 76.245 -> 76 - 128 = -52
        // Cb = -0.168736*255 - 0.331264*0 + 0.5*0 = -43.028 -> -43
        // Cr = 0.5*255 - 0.418688*0 - 0.081312*0 = 127.5 -> 127
        
        assert_eq!(y[0], -52, "Y component for pure red");
        assert_eq!(cb[0], -43, "Cb component for pure red");
        assert_eq!(cr[0], 127, "Cr component for pure red");
    }

    #[test]
    fn test_rgb_to_ycbcr_green() {
        // Test pure green (0, 255, 0)
        let green_rgb = [0u8, 255, 0];
        let mut y = [0i8; 1];
        let mut cb = [0i8; 1];
        let mut cr = [0i8; 1];

        rgb_to_ycbcr_planes(&green_rgb, &mut y, &mut cb, &mut cr);

        // Expected values for pure green
        // Y = 0.299*0 + 0.587*255 + 0.114*0 = 149.685 -> 150 - 128 = 22
        // Cb = -0.168736*0 - 0.331264*255 + 0.5*0 = -84.472 -> -84
        // Cr = 0.5*0 - 0.418688*255 - 0.081312*0 = -106.765 -> -107
        
        assert_eq!(y[0], 22, "Y component for pure green");
        assert_eq!(cb[0], -84, "Cb component for pure green");
        assert_eq!(cr[0], -107, "Cr component for pure green");
    }

    #[test]
    fn test_rgb_to_ycbcr_blue() {
        // Test pure blue (0, 0, 255)
        let blue_rgb = [0u8, 0, 255];
        let mut y = [0i8; 1];
        let mut cb = [0i8; 1];
        let mut cr = [0i8; 1];

        rgb_to_ycbcr_planes(&blue_rgb, &mut y, &mut cb, &mut cr);

        // Expected values for pure blue
        // Y = 0.299*0 + 0.587*0 + 0.114*255 = 29.07 -> 29 - 128 = -99
        // Cb = -0.168736*0 - 0.331264*0 + 0.5*255 = 127.5 -> 127
        // Cr = 0.5*0 - 0.418688*0 - 0.081312*255 = -20.735 -> -21
        
        assert_eq!(y[0], -99, "Y component for pure blue");
        assert_eq!(cb[0], 127, "Cb component for pure blue");
        assert_eq!(cr[0], -21, "Cr component for pure blue");
    }

    #[test]
    fn test_rgb_to_ycbcr_white() {
        // Test white (255, 255, 255)
        let white_rgb = [255u8, 255, 255];
        let mut y = [0i8; 1];
        let mut cb = [0i8; 1];
        let mut cr = [0i8; 1];

        rgb_to_ycbcr_planes(&white_rgb, &mut y, &mut cb, &mut cr);

        // Expected values for white (with rounding adjustments for fixed-point math)
        // Y = 0.299*255 + 0.587*255 + 0.114*255 = 255 -> 255 - 128 = 127
        // Cb and Cr should be very close to 0, but may have small rounding errors
        
        assert_eq!(y[0], 127, "Y component for white");
        assert!(cb[0].abs() <= 1, "Cb component for white should be close to 0, got {}", cb[0]);
        assert!(cr[0].abs() <= 1, "Cr component for white should be close to 0, got {}", cr[0]);
    }

    #[test]
    fn test_rgb_to_ycbcr_black() {
        // Test black (0, 0, 0)
        let black_rgb = [0u8, 0, 0];
        let mut y = [0i8; 1];
        let mut cb = [0i8; 1];
        let mut cr = [0i8; 1];

        rgb_to_ycbcr_planes(&black_rgb, &mut y, &mut cb, &mut cr);

        // Expected values for black
        // Y = 0 -> 0 - 128 = -128
        // Cb = 0 (close to)
        // Cr = 0 (close to)
        
        assert_eq!(y[0], -128, "Y component for black");
        assert!(cb[0].abs() <= 1, "Cb component for black should be close to 0, got {}", cb[0]);
        assert!(cr[0].abs() <= 1, "Cr component for black should be close to 0, got {}", cr[0]);
    }

    #[test]
    fn test_ycbcr_from_rgb_image() {
        // Create a small test image with known colors
        let mut img: RgbImage = ImageBuffer::new(2, 2);
        
        // Set pixels: red, green, blue, white
        img.put_pixel(0, 0, Rgb([255, 0, 0]));   // red
        img.put_pixel(1, 0, Rgb([0, 255, 0]));   // green
        img.put_pixel(0, 1, Rgb([0, 0, 255]));   // blue
        img.put_pixel(1, 1, Rgb([255, 255, 255])); // white

        let (y_buf, cb_buf, cr_buf) = ycbcr_from_rgb(&img);

        assert_eq!(y_buf.len(), 4);
        assert_eq!(cb_buf.len(), 4);
        assert_eq!(cr_buf.len(), 4);

        // Check red pixel
        assert_eq!(y_buf[0], -52);
        assert_eq!(cb_buf[0], -43);
        assert_eq!(cr_buf[0], 127);

        // Check green pixel
        assert_eq!(y_buf[1], 22);
        assert_eq!(cb_buf[1], -84);
        assert_eq!(cr_buf[1], -107);

        // Check blue pixel
        assert_eq!(y_buf[2], -99);
        assert_eq!(cb_buf[2], 127);
        assert_eq!(cr_buf[2], -21);

        // Check white pixel
        assert_eq!(y_buf[3], 127);
        assert_eq!(cb_buf[3], 0);
        assert_eq!(cr_buf[3], 0);
    }

    #[test]
    fn test_rgb_planes_length_mismatch() {
        let rgb_data = [255u8, 0, 0, 0, 255, 0]; // 2 pixels
        let mut y = [0i8; 1];  // Wrong length
        let mut cb = [0i8; 2];
        let mut cr = [0i8; 2];

        // This should panic due to assertion - testing in a different way to avoid UnwindSafe issues
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            rgb_to_ycbcr_planes(&rgb_data, &mut y, &mut cb, &mut cr);
        }));
        
        assert!(result.is_err(), "Should panic on length mismatch");
    }

    #[test]
    fn test_rgb_input_not_multiple_of_3() {
        let rgb_data = [255u8, 0]; // Not divisible by 3
        let mut y = [0i8; 1];
        let mut cb = [0i8; 1];
        let mut cr = [0i8; 1];

        // This should panic due to assertion
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            rgb_to_ycbcr_planes(&rgb_data, &mut y, &mut cb, &mut cr);
        }));
        
        assert!(result.is_err(), "Should panic on invalid RGB data length");
    }

    #[test]
    fn test_encoder_params_default() {
        let params = EncoderParams::default();
        assert_eq!(params.decibels, Some(90.0));
        assert!(matches!(params.crcb_mode, CrcbMode::Full));
        assert_eq!(params.db_frac, 0.35);
    }

    #[test]
    fn test_crcb_mode_values() {
        // Test enum variants exist
        let _none = CrcbMode::None;
        let _half = CrcbMode::Half;
        let _normal = CrcbMode::Normal;
        let _full = CrcbMode::Full;
        
        // Test default
        let default_mode = CrcbMode::default();
        assert!(matches!(default_mode, CrcbMode::None));
    }
}

#[cfg(test)]
mod integration_tests {
    use crate::encode::iw44::encoder::{IWEncoder, EncoderParams, CrcbMode};
    use image::{RgbImage, GrayImage, ImageBuffer, Rgb, Luma};

    #[test]
    fn test_encoder_from_grayscale() {
        let img: GrayImage = ImageBuffer::from_fn(32, 32, |x, y| {
            Luma([((x + y) % 256) as u8])
        });

        let params = EncoderParams {
            decibels: Some(80.0),
            crcb_mode: CrcbMode::None,
            db_frac: 0.35,
        };

        let result = IWEncoder::from_gray(&img, None, params);
        assert!(result.is_ok(), "Should create encoder from grayscale image");
    }

    #[test]
    fn test_encoder_from_rgb() {
        let img: RgbImage = ImageBuffer::from_fn(32, 32, |x, y| {
            Rgb([
                ((x * 4) % 256) as u8,
                ((y * 4) % 256) as u8,
                (((x + y) * 2) % 256) as u8,
            ])
        });

        let params = EncoderParams {
            decibels: Some(85.0),
            crcb_mode: CrcbMode::Full,
            db_frac: 0.35,
        };

        let result = IWEncoder::from_rgb(&img, None, params);
        assert!(result.is_ok(), "Should create encoder from RGB image");
    }

    #[test]
    fn test_encode_chunk_progression() {
        let img: GrayImage = ImageBuffer::from_fn(64, 64, |x, y| {
            Luma([((x ^ y) % 256) as u8])
        });

        let params = EncoderParams::default();
        let mut encoder = IWEncoder::from_gray(&img, None, params).unwrap();

        // Encode first chunk
        let (chunk1, has_more1) = encoder.encode_chunk(10).unwrap();
        assert!(!chunk1.is_empty(), "First chunk should not be empty");

        // If there's more data, encode another chunk
        if has_more1 {
            let (chunk2, _has_more2) = encoder.encode_chunk(10).unwrap();
            // Second chunk might be empty if we've encoded all meaningful data
        }
    }
}

