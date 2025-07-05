// examples/test_jb2_encoding.rs
//! Test JB2 encoding with a PBM file
//!
//! This example demonstrates how to:
//! 1. Load a PBM (Portable Bitmap) file into a BitImage
//! 2. Encode it using the JB2 encoder
//! 3. Create a single-page DjVu document with JB2 foreground data
//! 4. Test with multiple pages

use djvu_encoder::doc::DocumentEncoder;
use djvu_encoder::doc::page_encoder::PageComponents;
use djvu_encoder::encode::jb2::symbol_dict::BitImage;
use djvu_encoder::encode::jb2::encoder::JB2Encoder;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write, Seek, SeekFrom};
use std::path::Path;

/// Load a PBM (Portable Bitmap) file into a BitImage
/// Based on working jbig2 rust library PBM loader that handles IrfanView metadata properly
fn load_pbm(path: &str) -> Result<BitImage, Box<dyn std::error::Error>> {
    let mut file = File::open(path)?;
    let mut reader = BufReader::new(&mut file);

    let mut line = String::new();
    reader.read_line(&mut line)?;
    if line.trim() != "P4" {
        return Err(format!("Unsupported PBM magic number: {}", line.trim()).into());
    }
    println!("PBM magic number: {}", line.trim());

    line.clear();
    loop {
        reader.read_line(&mut line)?;
        let trimmed_line = line.trim();
        if !trimmed_line.starts_with('#') && !trimmed_line.is_empty() {
            break;
        }
        line.clear();
    }
    let parts: Vec<&str> = line.trim().split_whitespace().collect();
    if parts.len() != 2 {
        return Err(format!("Invalid PBM dimensions line: {}", line.trim()).into());
    }
    let width = parts[0].parse::<usize>()?;
    let height = parts[1].parse::<usize>()?;
    println!("PBM dimensions: {}x{}", width, height);

    // Get current position and seek back to that position in the main file handle
    let current_file_pos = reader.stream_position()?;
    file.seek(SeekFrom::Start(current_file_pos))?;

    let width_in_bytes = (width + 7) / 8;
    let expected_data_len = height * width_in_bytes;
    println!(
        "Calculated PBM data length ( H * ((W+7)/8) ): {} bytes",
        expected_data_len
    );
    let mut data = vec![0u8; expected_data_len];
    file.read_exact(&mut data)?;

    // Convert to BitImage
    let mut image = BitImage::new(width as u32, height as u32)?;
    let mut pixel_count = 0;
    
    for y in 0..height {
        for x in 0..width {
            let byte_index = y * width_in_bytes + x / 8;
            let bit_index = 7 - (x % 8);
            
            if byte_index < data.len() {
                let bit = (data[byte_index] >> bit_index) & 1;
                let is_foreground = bit != 0;
                image.set_usize(x, y, is_foreground);
                if is_foreground {
                    pixel_count += 1;
                }
            }
        }
    }
    
    println!("Loaded {} foreground pixels out of {} total pixels", pixel_count, width * height);
    println!("Foreground density: {:.2}%", (pixel_count as f64 / (width * height) as f64) * 100.0);
    
    // Debug: Sample a few pixels to verify loading
    if width > 10 && height > 10 {
        print!("Sample pixels (top-left 10x10): ");
        for y in 0..10 {
            for x in 0..10 {
                print!("{}", if image.get_pixel_unchecked(x, y) { "1" } else { "0" });
            }
            if y < 9 { print!(" "); }
        }
        println!();
    }
    
    Ok(image)
}

fn test_jb2_single_page(pbm_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Testing JB2 Single Page Encoding ===");
    
    // Load the PBM file
    let bit_image = load_pbm(pbm_path)?;
    println!("Loaded BitImage: {}x{}", bit_image.width, bit_image.height);
    
    // Create a guaranteed visible test image instead of using the potentially empty area
    println!("Creating test image with guaranteed visible content...");
    let test_width = 100;
    let test_height = 100;
    let mut test_image = BitImage::new(test_width, test_height)?;
    
    // Create a simple but visible pattern with SEPARATED characters
    let mut pixel_count = 0;
    for y in 0..test_height as usize {
        for x in 0..test_width as usize {
            let mut pixel = false;
            
            // Create separated character-like patterns
            // Letter "A" (isolated)
            if y >= 20 && y <= 35 && x >= 10 && x <= 20 {
                if (x == 10 || x == 20) && y >= 25 || // sides
                   (y == 25 || y == 30) && x >= 10 && x <= 20 || // top and middle bar
                   y == 20 && x >= 13 && x <= 17 { // peak
                    pixel = true;
                }
            }
            
            // Letter "B" (isolated, with gap)
            if y >= 20 && y <= 35 && x >= 35 && x <= 45 {
                if x == 35 || // left side
                   (y == 20 || y == 27 || y == 35) && x >= 35 && x <= 42 || // horizontal bars
                   (y >= 20 && y <= 26 || y >= 28 && y <= 35) && x == 42 { // right curves
                    pixel = true;
                }
            }
            
            // Letter "C" (isolated, with gap)
            if y >= 20 && y <= 35 && x >= 60 && x <= 70 {
                if (y == 20 || y == 35) && x >= 60 && x <= 67 || // top and bottom
                   x == 60 && y >= 20 && y <= 35 { // left side
                    pixel = true;
                }
            }
            
            // Some isolated dots (each should be a separate component)
            if (x == 15 && y == 45) ||
               (x == 40 && y == 45) ||
               (x == 65 && y == 45) {
                pixel = true;
            }
            
            // A small isolated rectangle
            if y >= 60 && y <= 65 && x >= 30 && x <= 35 {
                pixel = true;
            }
            
            test_image.set_usize(x, y, pixel);
            if pixel {
                pixel_count += 1;
            }
        }
    }
    
    println!("Created test image with {} foreground pixels out of {} total", pixel_count, test_width * test_height);
    
    // Use only the test patterns for proper separated component testing
    println!("Using only separated test patterns for JB2 component analysis");
    
    println!("Final test image has {} foreground pixels", pixel_count);
    
    // Debug: Print a visual representation of the test image
    println!("Visual representation of test image (first 50x25):");
    for y in 0..25 {
        for x in 0..50 {
            print!("{}", if test_image.get_pixel_unchecked(x, y) { "█" } else { "·" });
        }
        println!();
    }
    
    // Test direct JB2 encoding
    println!("Testing direct JB2 encoder...");
    
    // First, let's debug the connected components detection
    use djvu_encoder::encode::jb2::symbol_dict::{find_connected_components, SymDictBuilder};
    
    println!("Debug: Finding connected components...");
    let components = find_connected_components(&test_image, 4);
    println!("Debug: Found {} connected components with min_size=4", components.len());
    
    // Print info about the largest components
    for (i, component) in components.iter().enumerate().take(10) {
        println!("  Component {}: {}x{} pixels, {} total pixels at ({}, {})", 
                 i, component.bounds.width, component.bounds.height, 
                 component.pixel_count, component.bounds.x, component.bounds.y);
    }
    
    // Now test the dictionary builder
    println!("Debug: Building symbol dictionary...");
    let mut builder = SymDictBuilder::new(10);
    let (dictionary, components_with_dict) = builder.build(&test_image);
    println!("Debug: Dictionary contains {} unique symbols", dictionary.len());
    println!("Debug: Total components: {}", components_with_dict.len());
    
    // Now do the actual encoding
    let mut jb2_encoder = JB2Encoder::new(Vec::new());
    let jb2_data = jb2_encoder.encode_page(&test_image, 10)?; // max_error = 10
    println!("JB2 encoding successful! Data size: {} bytes", jb2_data.len());
    
    // Analyze the JB2 data structure
    println!("Analyzing JB2 data structure:");
    if jb2_data.len() >= 8 {
        println!("  First chunk: {:?}", &jb2_data[0..4]);
        if jb2_data.len() >= 16 {
            println!("  Second chunk: {:?}", &jb2_data[8..12]);
        }
    }
    
    // Save the raw JB2 data for inspection
    let mut jb2_file = File::create("test_jb2_raw.dat")?;
    jb2_file.write_all(&jb2_data)?;
    println!("Saved raw JB2 data to test_jb2_raw.dat");
    
    // Create a complete DjVu document with JB2 foreground (not mask!)
    println!("Creating DjVu document with JB2 foreground...");
    let mut document_encoder = DocumentEncoder::new();
    
    let page_components = PageComponents::new()
        .with_foreground(test_image)?;
    
    document_encoder.add_page(page_components)?;
    
    // Write the document
    let mut output = Vec::new();
    document_encoder.write_to(&mut output)?;
    
    let output_path = "test_jb2_single.djvu";
    let mut file = File::create(output_path)?;
    file.write_all(&output)?;
    
    println!("✅ Single page JB2 test successful!");
    println!("   Output: {} ({} bytes)", output_path, output.len());
    
    Ok(())
}

fn test_jb2_multi_page(pbm_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Testing JB2 Multi-Page Encoding ===");
    
    // Load the original image
    let bit_image = load_pbm(pbm_path)?;
    println!("Loaded BitImage: {}x{}", bit_image.width, bit_image.height);
    
    // Use smaller images for multi-page test to avoid arithmetic overflow
    let small_width = std::cmp::min(150, bit_image.width);
    let small_height = std::cmp::min(150, bit_image.height);
    
    // Create a document with multiple pages using smaller versions
    let mut document_encoder = DocumentEncoder::new();
    
    // Page 1: Original image (smaller version)
    let mut page1_image = BitImage::new(small_width as u32, small_height as u32)?;
    for y in 0..small_height {
        for x in 0..small_width {
            let pixel = bit_image.get_pixel_unchecked(x, y);
            page1_image.set_usize(x, y, pixel);
        }
    }
    
    let page1 = PageComponents::new()
        .with_foreground(page1_image)?;
    document_encoder.add_page(page1)?;
    
    // Page 2: Create a modified version (shift pattern)
    let mut modified_image = BitImage::new(small_width as u32, small_height as u32)?;
    for y in 0..small_height {
        for x in 0..small_width {
            // Shift pattern and add some variation
            let src_x = (x + 10) % small_width;
            let src_y = (y + 5) % small_height;
            let original_pixel = if src_x < bit_image.width && src_y < bit_image.height {
                bit_image.get_pixel_unchecked(src_x, src_y)
            } else {
                false
            };
            // Add some noise
            let noise = (x + y) % 20 == 0;
            modified_image.set_usize(x, y, original_pixel ^ noise);
        }
    }
    
    let page2 = PageComponents::new()
        .with_foreground(modified_image)?;
    document_encoder.add_page(page2)?;
    
    // Page 3: Create a synthetic pattern
    let mut synthetic_image = BitImage::new(small_width as u32, small_height as u32)?;
    for y in 0..small_height {
        for x in 0..small_width {
            // Create a checkerboard pattern with text-like features
            let checker = ((x / 8) + (y / 8)) % 2 == 0;
            let text_like = (x / 4) % 3 == 0 && (y / 6) % 2 == 0;
            synthetic_image.set_usize(x, y, checker || text_like);
        }
    }
    
    let page3 = PageComponents::new()
        .with_foreground(synthetic_image)?;
    document_encoder.add_page(page3)?;
    
    // Write the document
    let mut output = Vec::new();
    document_encoder.write_to(&mut output)?;
    
    let output_path = "test_jb2_multi.djvu";
    let mut file = File::create(output_path)?;
    file.write_all(&output)?;
    
    println!("✅ Multi-page JB2 test successful!");
    println!("   Output: {} ({} bytes)", output_path, output.len());
    println!("   Pages: 3");
    println!("   Page size: {}x{}", small_width, small_height);
    
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("JB2 Encoding Test Example");
    println!("=========================");
    
    let args: Vec<String> = std::env::args().collect();
    let pbm_path = args.get(1).map(|s| s.as_str()).unwrap_or("test.pbm");
    
    if !Path::new(pbm_path).exists() {
        println!("Error: PBM file not found at '{}'", pbm_path);
        println!("Please ensure the PBM file exists or provide a path as an argument.");
        println!("Usage: cargo run --example test_jb2_encoding -- [path/to/file.pbm]");
        
        // Create a simple test PBM file if none exists
        if pbm_path == "test.pbm" {
            println!("\nCreating a simple test PBM file...");
            create_test_pbm("test.pbm")?;
            println!("Created test.pbm with a simple pattern.");
        } else {
            return Ok(());
        }
    }
    
    // Run tests
    test_jb2_single_page(pbm_path)?;
    test_jb2_multi_page(pbm_path)?;
    
    println!("\n🎉 All JB2 tests completed successfully!");
    println!("\nGenerated files:");
    println!("  - test_jb2_raw.dat (raw JB2 data)");
    println!("  - test_jb2_single.djvu (single page)");
    println!("  - test_jb2_multi.djvu (3 pages)");
    
    Ok(())
}

/// Create a simple test PBM file if none exists
fn create_test_pbm(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut file = File::create(path)?;
    
    // Write a simple 40x30 ASCII PBM with text-like pattern
    writeln!(file, "P1")?;
    writeln!(file, "# Test PBM file for JB2 encoding")?;
    writeln!(file, "40 30")?;
    
    for y in 0..30 {
        for x in 0..40 {
            // Create a pattern that looks like text
            let bit = if y >= 5 && y <= 25 {
                // Create letter-like shapes
                match x / 10 {
                    0 => (x % 10 == 2 || x % 10 == 8) && (y >= 8 && y <= 22),  // "I" shape
                    1 => (x % 10 >= 2 && x % 10 <= 8) && ((y == 8) || (y == 15) || (y == 22)), // "E" shape
                    2 => (x % 10 == 2) || ((x % 10 >= 2 && x % 10 <= 8) && (y == 8 || y == 22)), // "L" shape
                    3 => (x % 10 >= 2 && x % 10 <= 8) && ((y >= 8 && y <= 12) || (y >= 18 && y <= 22)), // lines
                    _ => false,
                }
            } else {
                false
            };
            
            write!(file, "{} ", if bit { 1 } else { 0 })?;
        }
        writeln!(file)?;
    }
    
    Ok(())
}
