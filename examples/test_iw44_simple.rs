// examples/test_iw44_simple.rs
//! Simple IW44 encoding test to diagnose the gray output issue

use djvu_encoder::encode::iw44::encoder::{IWEncoder, EncoderParams, CrcbMode};
use djvu_encoder::doc::{DocumentEncoder, PageComponents};
use image::{GrayImage, RgbImage};
use std::fs::File;
use std::io::Write;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("IW44 Simple Encoding Test");
    println!("========================");

    // Test 1: Very simple grayscale pattern
    test_simple_pattern()?;
    
    // Test 2: Check raw IW44 chunks  
    test_raw_iw44_chunks()?;
    
    println!("\n✅ All tests completed!");
    
    Ok(())
}

fn test_simple_pattern() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Test 1: Simple Pattern ===");
    
    // Create a very simple 64x64 test image with clear patterns
    let width = 64;
    let height = 64;
    let mut img_data = vec![128u8; (width * height) as usize];
    
    // Create a clear checkerboard pattern
    for y in 0..height {
        for x in 0..width {
            let checkerboard = ((x / 8) + (y / 8)) % 2 == 0;
            img_data[(y * width + x) as usize] = if checkerboard { 200 } else { 50 };
        }
    }
    
    println!("Created {}x{} checkerboard pattern", width, height);
    
    // Save as grayscale image first for reference
    let img = GrayImage::from_raw(width, height, img_data.clone()).unwrap();
    
    // Convert to RGB for DjVu encoding
    let rgb_img = RgbImage::from_fn(width, height, |x, y| {
        let gray_val = img_data[(y * width + x) as usize];
        image::Rgb([gray_val, gray_val, gray_val])
    });
    
    // Create DjVu document with this simple pattern
    let mut encoder = DocumentEncoder::new();
    let page = PageComponents::new().with_background(rgb_img)?;
    encoder.add_page(page)?;
    
    // Write the document
    let mut output = Vec::new();
    encoder.write_to(&mut output)?;
    
    let output_path = "test_simple_pattern.djvu";
    let mut file = File::create(output_path)?;
    file.write_all(&output)?;
    
    println!("✅ Created DjVu file: {} ({} bytes)", output_path, output.len());
    println!("   Try opening this file to see if the checkerboard pattern is visible");
    
    Ok(())
}

fn test_raw_iw44_chunks() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Test 2: Raw IW44 Chunks Analysis ===");
    
    // Create a simple gradient image
    let width = 32;
    let height = 32;
    let mut img_data = vec![0u8; (width * height) as usize];
    
    // Simple left-to-right gradient
    for y in 0..height {
        for x in 0..width {
            let val = (x * 255 / (width - 1)) as u8;
            img_data[(y * width + x) as usize] = val;
        }
    }
    
    let img = GrayImage::from_raw(width, height, img_data).unwrap();
    println!("Created {}x{} gradient image", width, height);
    
    // Test IW44 encoder directly
    let params = EncoderParams {
        decibels: Some(30.0), // Target quality to limit chunks
        crcb_mode: CrcbMode::None,
        db_frac: 0.9,
    };
    
    let mut encoder = IWEncoder::from_gray(&img, None, params)?;
    
    let mut chunk_count = 0;
    let mut total_bytes = 0;
    
    // Encode and analyze each chunk
    loop {
        let result = encoder.encode_chunk(10)?; // 10 slices per chunk
        let (chunk, more) = result;
        
        if chunk.is_empty() {
            break;
        }
        
        chunk_count += 1;
        total_bytes += chunk.len();
        
        println!("Chunk {}: {} bytes, more: {}", chunk_count, chunk.len(), more);
        
        // Analyze first few bytes of chunk
        if chunk.len() >= 8 {
            print!("  Header: ");
            for i in 0..8.min(chunk.len()) {
                print!("{:02X} ", chunk[i]);
            }
            println!();
        }
        
        if !more || chunk_count >= 10 {
            break;
        }
    }
    
    println!("Total: {} chunks, {} bytes", chunk_count, total_bytes);
    
    // Save raw chunks for analysis
    let mut all_chunks = Vec::new();
    let params2 = EncoderParams {
        decibels: Some(25.0),
        crcb_mode: CrcbMode::None,
        db_frac: 0.9,
    };
    let mut encoder2 = IWEncoder::from_gray(&img, None, params2)?;
    
    loop {
        let result = encoder2.encode_chunk(5)?; // 5 slices per chunk
        let (chunk, more) = result;
        
        if chunk.is_empty() {
            break;
        }
        
        all_chunks.extend_from_slice(&chunk);
        
        if !more {
            break;
        }
    }
    
    let mut file = File::create("test_raw_iw44.dat")?;
    file.write_all(&all_chunks)?;
    
    println!("✅ Saved raw IW44 data: test_raw_iw44.dat ({} bytes)", all_chunks.len());
    
    Ok(())
}
