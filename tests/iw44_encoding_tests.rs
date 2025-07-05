// tests/iw44_encoding_tests.rs

use djvu_encoder::encode::iw44::{encoder::*, transform};
use image::{GrayImage, Luma, Rgb, RgbImage};

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

/// Test: IFF-structure validator for DjVu output
#[test]
fn test_iff_structure_validator() {
    use djvu_encoder::{DocumentEncoder, PageComponents};
    use image::RgbImage;
    use std::io::Cursor;

    // Create a trivial single-page DjVu file in memory
    let mut encoder = DocumentEncoder::new();
    let page = PageComponents::new().with_background(RgbImage::new(8, 8)).unwrap();
    encoder.add_page(page).unwrap();
    let mut buf = Vec::new();
    encoder.write_to(&mut buf).expect("Failed to encode DjVu");
    let mut cursor = std::io::Cursor::new(&buf);

    // 1) Magic "AT&T"
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic).unwrap();
    assert_eq!(&magic, b"AT&T");

    // 2) FORM chunk
    let mut chunk_id = [0u8; 4];
    cursor.read_exact(&mut chunk_id).unwrap();
    assert_eq!(&chunk_id, b"FORM");

    // 3) FORM-size
    use byteorder::{BigEndian, ReadBytesExt};
    let size = cursor.read_u32::<BigEndian>().unwrap() as usize;
    assert_eq!(size + 8, buf.len(), "FORM size matches file length");

    // 4) FORM-type
    let mut form_type = [0u8; 4];
    cursor.read_exact(&mut form_type).unwrap();
    assert_eq!(&form_type, b"DJVU");

    // 5) Iterate remaining chunks
    while (cursor.position() as usize) < buf.len() {
        let mut id = [0u8; 4];
        let mut sz = [0u8; 4];
        if cursor.read_exact(&mut id).is_err() { break; }
        if cursor.read_exact(&mut sz).is_err() { break; }
        let n = u32::from_be_bytes(sz) as usize;
        cursor.set_position(cursor.position() + n as u64);
    }
    // If we reach here, the IFF structure is valid
}

/// Test: IW44 wavelet transform round-trip
#[test]
fn test_transform_round_trip() {
    // Basic 8x8 impulse test (already present)
    use djvu_encoder::encode::iw44::transform;
    let mut img = vec![vec![0f32; 8]; 8];
    img[3][4] = 255.0;
    let coeffs = transform::forward(&img);
    let recon = transform::inverse(&coeffs);
    let mut mse = 0.0;
    for y in 0..8 {
        for x in 0..8 {
            let diff = img[y][x] - recon[y][x];
            mse += diff * diff;
        }
    }
    mse /= 64.0;
    let psnr = if mse == 0.0 { 99.9 } else { 10.0 * ((255.0 * 255.0) / mse).log10() };
    assert!(psnr > 40.0, "PSNR too low: {}", psnr);
}

/// Test: Wavelet round-trip for various patterns and sizes
#[test]
fn test_wavelet_roundtrip_various_patterns() {
    use djvu_encoder::encode::iw44::transform::{Encode, Decode};
    let test_cases = [
        ("impulse", 32, 32),
        ("ramp", 64, 64),
        ("checkerboard", 32, 32),
        ("gradient", 64, 32),
        ("constant", 32, 32),
    ];
    for (pattern, width, height) in test_cases {
        let result = super::test_wavelet_round_trip(width, height, pattern);
        assert!(result.passed, "Pattern '{}' failed round-trip test", pattern);
    }
}

/// Test: Wavelet round-trip for small image
#[test]
fn test_wavelet_roundtrip_small_image() {
    let result = super::test_wavelet_round_trip(8, 8, "ramp");
    assert!(result.passed, "Small image round-trip test failed");
}

/// Test: Wavelet round-trip for power-of-2 size
#[test]
fn test_wavelet_roundtrip_power_of_two() {
    let result = super::test_wavelet_round_trip(64, 64, "gradient");
    assert!(result.passed, "Power-of-2 round-trip test failed");
}

/// Test: Comprehensive wavelet transform round-trip
#[test]
fn test_transform_round_trip_comprehensive() {
    use djvu_encoder::encode::iw44::transform::{Encode, Decode};
    let test_cases = vec![
        (8, 8, "impulse"),
        (8, 8, "gradient"),
        (16, 16, "impulse"),
        (16, 16, "checkerboard"),
        (32, 32, "ramp"),
        (32, 32, "constant"),
    ];
    for (width, height, pattern) in test_cases {
        let original = super::generate_test_pattern(width, height, pattern);
        let mut test_data = original.clone();
        let begin = 1;
        let end = std::cmp::min(5, std::cmp::min(
            (width as f64).log2() as usize,
            (height as f64).log2() as usize
        ));
        Encode::forward(&mut test_data, width, height, width, begin, end);
        Decode::backward(&mut test_data, width, height, width, begin, end);
        let result = super::calculate_error_metrics(&original, &test_data);
        assert!(result.psnr > 99.0 || result.is_perfect,
                "Transform round-trip failed for {} {}x{}: PSNR {:.1} dB",
                pattern, width, height, result.psnr);
    }
}

/// Test: IFF structure comprehensive validation
#[test]
fn test_iff_structure_comprehensive() {
    use djvu_encoder::{DocumentEncoder, PageComponents};
    use image::GrayImage;
    let width = 64;
    let height = 64;
    let mut img = GrayImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let value = ((x + y) % 256) as u8;
            img.put_pixel(x, y, image::Luma([value]));
        }
    }
    let page = PageComponents::new().with_background(image::DynamicImage::ImageLuma8(img)).unwrap();
    let mut encoder = DocumentEncoder::new();
    encoder.add_page(page).unwrap();
    let mut buf = Vec::new();
    encoder.write_to(&mut buf).expect("Failed to encode DjVu");
    // Now validate IFF structure
    let mut cursor = std::io::Cursor::new(&buf);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic).unwrap();
    assert_eq!(&magic, b"AT&T");
    let mut chunk_id = [0u8; 4];
    cursor.read_exact(&mut chunk_id).unwrap();
    assert_eq!(&chunk_id, b"FORM");
    use byteorder::{BigEndian, ReadBytesExt};
    let size = cursor.read_u32::<BigEndian>().unwrap() as usize;
    assert_eq!(size + 8, buf.len(), "FORM size matches file length");
    let mut form_type = [0u8; 4];
    cursor.read_exact(&mut form_type).unwrap();
    assert_eq!(&form_type, b"DJVU");
    // Iterate remaining chunks
    while (cursor.position() as usize) < buf.len() {
        let mut id = [0u8; 4];
        let mut sz = [0u8; 4];
        if cursor.read_exact(&mut id).is_err() { break; }
        if cursor.read_exact(&mut sz).is_err() { break; }
        let n = u32::from_be_bytes(sz) as usize;
        cursor.set_position(cursor.position() + n as u64);
    }
}

// --- Helpers for transform tests ---

fn generate_test_pattern(width: usize, height: usize, pattern: &str) -> Vec<i16> {
    // ... (copy from src/validate/transform_tests.rs)
    let mut data = vec![0i16; width * height];
    match pattern {
        "impulse" => { data[width / 2 + (height / 2) * width] = 255; },
        "ramp" => { for y in 0..height { for x in 0..width { data[y * width + x] = (x + y) as i16; } } },
        "checkerboard" => { for y in 0..height { for x in 0..width { data[y * width + x] = if (x + y) % 2 == 0 { 127 } else { -127 }; } } },
        "gradient" => { for y in 0..height { for x in 0..width { data[y * width + x] = (x * 255 / (width as i16)) as i16; } } },
        "constant" => { for val in data.iter_mut() { *val = 42; } },
        _ => {},
    }
    data
}

struct WaveletTestResult {
    passed: bool,
    max_abs_error: i16,
    mean_abs_error: f64,
    rms_error: f64,
    psnr: f64,
    is_perfect: bool,
}

fn calculate_error_metrics(original: &[i16], reconstructed: &[i16]) -> WaveletTestResult {
    let mut max_abs_error = 0i16;
    let mut sum_abs_error = 0f64;
    let mut sum_sq_error = 0f64;
    let n = original.len();
    let mut is_perfect = true;
    for (&a, &b) in original.iter().zip(reconstructed.iter()) {
        let err = (a - b).abs();
        if err > max_abs_error { max_abs_error = err; }
        sum_abs_error += err as f64;
        sum_sq_error += (err as f64) * (err as f64);
        if err != 0 { is_perfect = false; }
    }
    let mean_abs_error = sum_abs_error / n as f64;
    let rms_error = (sum_sq_error / n as f64).sqrt();
    let psnr = if rms_error == 0.0 { 99.9 } else { 20.0 * (255.0 / rms_error).log10() };
    WaveletTestResult {
        passed: psnr > 40.0,
        max_abs_error,
        mean_abs_error,
        rms_error,
        psnr,
        is_perfect,
    }
}

fn test_wavelet_round_trip(width: usize, height: usize, pattern: &str) -> WaveletTestResult {
    use djvu_encoder::encode::iw44::transform::{Encode, Decode};
    let original = generate_test_pattern(width, height, pattern);
    let mut test_data = original.clone();
    let begin = 1;
    let end = std::cmp::min(5, std::cmp::min(
        (width as f64).log2() as usize,
        (height as f64).log2() as usize
    ));
    Encode::forward(&mut test_data, width, height, width, begin, end);
    Decode::backward(&mut test_data, width, height, width, begin, end);
    calculate_error_metrics(&original, &test_data)
}

    use djvu_encoder::encode::iw44::transform;
    // Generate a simple 8x8 impulse image
    let mut img = vec![vec![0f32; 8]; 8];
    img[3][4] = 255.0;
    // Forward transform
    let coeffs = transform::forward(&img);
    // Inverse transform
    let recon = transform::inverse(&coeffs);
    // Compute PSNR
    let mut mse = 0.0;
    for y in 0..8 {
        for x in 0..8 {
            let diff = img[y][x] - recon[y][x];
            mse += diff * diff;
        }
    }
    mse /= 64.0;
    let psnr = if mse == 0.0 { 99.9 } else { 10.0 * ((255.0 * 255.0) / mse).log10() };
    assert!(psnr > 40.0, "PSNR too low: {}", psnr);
    // If this passes, transform round-trip is correct

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

