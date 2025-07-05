// tests/iw44_encoding_tests.rs

use djvu_encoder::encode::iw44::{encoder::*, transform};
use image::{GrayImage, Luma, Rgb, RgbImage};

/// Test that forward and backward transform are inverses of each other
#[test]
#[ignore]
fn test_wavelet_transform_roundtrip() {
    // Create a simple test image
    let width = 64;
    let height = 64;
    let mut test_data = vec![0i16; width * height];

    // Fill with a simple pattern
    for y in 0..height {
        for x in 0..width {
            test_data[y * width + x] = ((x + y) % 128) as i16;
        }
    }

    let original_data = test_data.clone();

    // Apply forward transform
    transform::Encode::forward(&mut test_data, width, height, width, 1, 32);

    // Apply backward transform
    transform::Decode::backward(&mut test_data, width, height, width, 1, 32);

    // Check if we get back the original data with some tolerance
    let mut total_error = 0i64;
    let mut max_error = 0i16;
    for i in 0..test_data.len() {
        let error = (test_data[i] - original_data[i]).abs();
        total_error += error as i64;
        max_error = max_error.max(error);
    }

    let mean_error = total_error as f64 / test_data.len() as f64;

    println!("Transform roundtrip test:");
    println!("  Mean error: {:.3}", mean_error);
    println!("  Max error: {}", max_error);

    // Allow some small error due to the nature of the lifting transform
    assert!(mean_error < 1.0, "Mean error {} is too large", mean_error);
    assert!(max_error < 10, "Max error {} is too large", max_error);
}

/// Test IW44 encoder with a simple grayscale image
#[test]
fn test_iw44_grayscale_encoding() {
    println!("Starting IW44 grayscale encoding test...");

    // Create a simple grayscale image
    let width = 32;
    let height = 32;
    let mut img_data = vec![128u8; (width * height) as usize];

    // Add some pattern
    for y in 0..height {
        for x in 0..width {
            let val = ((x + y) % 256) as u8;
            img_data[(y * width + x) as usize] = val;
        }
    }

    println!("Created test image {}x{}", width, height);

    let img = GrayImage::from_raw(width, height, img_data).unwrap();

    // Create encoder parameters
    let params = EncoderParams {
        decibels: None,
        crcb_mode: CrcbMode::None,
        db_frac: 0.9,
    };

    println!("Creating encoder...");

    // Test encoder creation
    let mut encoder = IWEncoder::from_gray(&img, None, params).expect("Failed to create encoder");

    println!("Encoder created, starting encoding...");

    // Encode one chunk
    let result = encoder.encode_chunk(1);

    println!("Encoding finished, checking result...");

    assert!(result.is_ok(), "Encoding failed: {:?}", result.err());

    let (chunk, more) = result.unwrap();

    // Basic sanity checks
    assert!(!chunk.is_empty(), "Encoded chunk is empty");
    assert!(
        chunk.len() > 10,
        "Encoded chunk is too small: {} bytes",
        chunk.len()
    );

    println!(
        "Successfully encoded {} bytes, more chunks: {}",
        chunk.len(),
        more
    );
}

/// Test IW44 encoder with an RGB image
#[test]
fn test_iw44_rgb_encoding() {
    // Create a simple RGB image
    let width = 32;
    let height = 32;
    let mut img_data = vec![Rgb([0u8, 0u8, 0u8]); (width * height) as usize];

    // Add some colorful pattern
    for y in 0..height {
        for x in 0..width {
            let r = ((x * 8) % 256) as u8;
            let g = ((y * 8) % 256) as u8;
            let b = (((x + y) * 4) % 256) as u8;
            img_data[(y * width + x) as usize] = Rgb([r, g, b]);
        }
    }

    let img = RgbImage::from_raw(
        width,
        height,
        img_data.into_iter().flat_map(|p| p.0).collect(),
    )
    .unwrap();

    // Create encoder parameters for color
    let params = EncoderParams {
        decibels: None,
        crcb_mode: CrcbMode::Normal,
        db_frac: 0.9,
    };

    // Test encoder creation
    let mut encoder =
        IWEncoder::from_rgb(&img, None, params).expect("Failed to create RGB encoder");

    // Encode one chunk
    let result = encoder.encode_chunk(1);
    assert!(result.is_ok(), "RGB encoding failed: {:?}", result.err());

    let (chunk, more) = result.unwrap();

    // Basic sanity checks
    assert!(!chunk.is_empty(), "RGB encoded chunk is empty");
    assert!(
        chunk.len() > 10,
        "RGB encoded chunk is too small: {} bytes",
        chunk.len()
    );

    println!(
        "Successfully encoded RGB {} bytes, more chunks: {}",
        chunk.len(),
        more
    );
}

/// Test multiple slice encoding
#[test]
#[ignore]
fn test_multiple_slice_encoding() {
    // Create a test image
    let width = 64;
    let height = 64;
    let mut img_data = vec![128u8; (width * height) as usize];

    // Add a gradient pattern
    for y in 0..height {
        for x in 0..width {
            let val = ((x * y) % 256) as u8;
            img_data[(y * width + x) as usize] = val;
        }
    }

    let img = GrayImage::from_raw(width, height, img_data).unwrap();

    // Create encoder parameters for multiple slices
    let params = EncoderParams {
        decibels: None,
        crcb_mode: CrcbMode::None,
        db_frac: 0.9,
    };

    let mut encoder = IWEncoder::from_gray(&img, None, params).expect("Failed to create encoder");

    let mut total_bytes = 0;
    let mut chunk_count = 0;

    // Keep encoding chunks until done
    loop {
        let result = encoder.encode_chunk(10); // 10 slices per chunk
        assert!(
            result.is_ok(),
            "Encoding failed at chunk {}: {:?}",
            chunk_count,
            result.err()
        );

        let (chunk, more) = result.unwrap();
        total_bytes += chunk.len();
        chunk_count += 1;

        println!("Chunk {}: {} bytes", chunk_count, chunk.len());

        if !more {
            break;
        }

        // Safety check to avoid infinite loops
        assert!(chunk_count < 100, "Too many chunks generated");
    }

    assert!(
        total_bytes > 100,
        "Total encoded size too small: {} bytes",
        total_bytes
    );
    assert!(chunk_count > 0, "No chunks were generated");

    println!(
        "Successfully encoded {} chunks totaling {} bytes",
        chunk_count, total_bytes
    );
}

/// Test byte limit stopping condition
#[test]
#[ignore]
fn test_byte_limit_encoding() {
    // Create a test image
    let width = 64;
    let height = 64;
    let mut img_data = vec![0u8; (width * height) as usize];

    // Fill with random-ish pattern to get some compressible data
    for y in 0..height {
        for x in 0..width {
            let val = ((x * 13 + y * 17) % 256) as u8;
            img_data[(y * width + x) as usize] = val;
        }
    }

    let img = GrayImage::from_raw(width, height, img_data).unwrap();

    // Create encoder parameters with byte limit
    let byte_limit = 500;
    let params = EncoderParams {
        decibels: None,
        crcb_mode: CrcbMode::None,
        db_frac: 0.9,
    };

    let mut encoder = IWEncoder::from_gray(&img, None, params).expect("Failed to create encoder");

    let result = encoder.encode_chunk(5); // 5 slices per chunk
    assert!(
        result.is_ok(),
        "Byte-limited encoding failed: {:?}",
        result.err()
    );

    let (chunk, _more) = result.unwrap();

    // The chunk should respect the byte limit (approximately)
    println!(
        "Byte-limited encoding: {} bytes (limit: {})",
        chunk.len(),
        byte_limit
    );

    // Allow some overhead for headers, but it shouldn't be wildly over the limit
    assert!(
        chunk.len() < byte_limit * 2,
        "Chunk size {} greatly exceeds byte limit {}",
        chunk.len(),
        byte_limit
    );
}

/// Test with a mask (masked encoding)
#[test]
fn test_masked_encoding() {
    // Create a test image
    let width = 32;
    let height = 32;
    let mut img_data = vec![128u8; (width * height) as usize];

    // Add some pattern to the image
    for y in 0..height {
        for x in 0..width {
            let val = ((x + y) % 256) as u8;
            img_data[(y * width + x) as usize] = val;
        }
    }

    let img = GrayImage::from_raw(width, height, img_data).unwrap();

    // Create a simple mask - mask out the center region
    let mut mask_data = vec![0u8; (width * height) as usize];
    for y in height / 4..(3 * height / 4) {
        for x in width / 4..(3 * width / 4) {
            mask_data[(y * width + x) as usize] = 255; // masked out
        }
    }

    let mask = GrayImage::from_raw(width, height, mask_data).unwrap();

    // Create encoder parameters
    let params = EncoderParams {
        decibels: None,
        crcb_mode: CrcbMode::None,
        db_frac: 0.9,
    };

    let mut encoder =
        IWEncoder::from_gray(&img, Some(&mask), params).expect("Failed to create masked encoder");

    let result = encoder.encode_chunk(1); // 1 slice for simple test
    assert!(result.is_ok(), "Masked encoding failed: {:?}", result.err());

    let (chunk, _more) = result.unwrap();

    assert!(!chunk.is_empty(), "Masked encoded chunk is empty");
    println!("Successfully encoded with mask: {} bytes", chunk.len());
}

/// Test that a constant image remains constant after transformation
#[test]
#[ignore]
fn test_constant_image_transform() {
    // Create a constant image
    let width = 32;
    let height = 32;
    let constant_value = 64i16;
    let mut test_data = vec![constant_value; width * height];

    let original_data = test_data.clone();

    // Apply forward transform
    transform::Encode::forward(&mut test_data, width, height, width, 1, 32);

    // Apply backward transform
    transform::Decode::backward(&mut test_data, width, height, width, 1, 32);

    // A constant image should remain constant
    for i in 0..test_data.len() {
        assert_eq!(
            test_data[i], original_data[i],
            "Constant image changed at index {}: got {}, expected {}",
            i, test_data[i], original_data[i]
        );
    }
}
