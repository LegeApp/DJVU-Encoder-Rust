// examples/debug_jb2_content.rs
//! Debug JB2 content visibility issues
//! 
//! This example creates very simple, guaranteed-visible test patterns
//! and tests them with both small and large sizes to debug why 
//! JB2 content isn't showing up in viewers.

use djvu_encoder::doc::DocumentEncoder;
use djvu_encoder::doc::page_encoder::PageComponents;
use djvu_encoder::encode::jb2::symbol_dict::BitImage;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write, Seek, SeekFrom};
use std::path::Path;

/// Load a PBM file (same as in test_jb2_encoding.rs)
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

    let current_file_pos = reader.stream_position()?;
    file.seek(SeekFrom::Start(current_file_pos))?;

    let width_in_bytes = (width + 7) / 8;
    let expected_data_len = height * width_in_bytes;
    println!("Calculated PBM data length: {} bytes", expected_data_len);
    let mut data = vec![0u8; expected_data_len];
    file.read_exact(&mut data)?;

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
    
    println!("Loaded {} foreground pixels out of {} total", pixel_count, width * height);
    Ok(image)
}

/// Create a very simple, guaranteed-visible test pattern
fn create_simple_test_pattern(width: usize, height: usize) -> Result<BitImage, Box<dyn std::error::Error>> {
    let mut image = BitImage::new(width as u32, height as u32)?;
    let mut pixel_count = 0;
    
    for y in 0..height {
        for x in 0..width {
            // Create a simple cross pattern in the center
            let center_x = width / 2;
            let center_y = height / 2;
            
            let is_foreground = 
                // Horizontal line
                (y == center_y && x >= center_x.saturating_sub(width/4) && x <= center_x + width/4) ||
                // Vertical line  
                (x == center_x && y >= center_y.saturating_sub(height/4) && y <= center_y + height/4) ||
                // Border rectangle
                (x < 5 || x >= width - 5 || y < 5 || y >= height - 5);
                
            image.set_usize(x, y, is_foreground);
            if is_foreground {
                pixel_count += 1;
            }
        }
    }
    
    println!("Created simple test pattern with {} foreground pixels", pixel_count);
    
    // Show a visual representation
    println!("Visual representation (showing center 20x20):");
    let start_x = (width / 2).saturating_sub(10);
    let start_y = (height / 2).saturating_sub(10);
    for y in start_y..std::cmp::min(start_y + 20, height) {
        for x in start_x..std::cmp::min(start_x + 20, width) {
            print!("{}", if image.get_pixel_unchecked(x, y) { "█" } else { "·" });
        }
        println!();
    }
    
    Ok(image)
}

fn test_simple_pattern() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Testing Simple Cross Pattern ===");
    
    // Test with a small, simple pattern
    let test_image = create_simple_test_pattern(50, 50)?;
    
    // Create DjVu document
    let mut document_encoder = DocumentEncoder::new();
    let page_components = PageComponents::new()
        .with_mask(test_image)?;  // Use mask instead of foreground
    
    document_encoder.add_page(page_components)?;
    
    let mut output = Vec::new();
    document_encoder.write_to(&mut output)?;
    
    let output_path = "debug_simple_pattern.djvu";
    let mut file = File::create(output_path)?;
    file.write_all(&output)?;
    
    println!("✅ Simple pattern test created: {} ({} bytes)", output_path, output.len());
    Ok(())
}

fn test_large_text_pattern() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Testing Large Text Pattern ===");
    
    // Create a larger image with text-like content
    let width = 200;
    let height = 100;
    let mut image = BitImage::new(width, height)?;
    let mut pixel_count = 0;
    
    // Create "HELLO" text pattern
    for y in 0..height as usize {
        for x in 0..width as usize {
            let is_foreground = create_letter_pattern(x, y, width as usize, height as usize);
            image.set_usize(x, y, is_foreground);
            if is_foreground {
                pixel_count += 1;
            }
        }
    }
    
    println!("Created large text pattern with {} foreground pixels", pixel_count);
    
    // Create DjVu document
    let mut document_encoder = DocumentEncoder::new();
    let page_components = PageComponents::new()
        .with_mask(image)?;
    
    document_encoder.add_page(page_components)?;
    
    let mut output = Vec::new();
    document_encoder.write_to(&mut output)?;
    
    let output_path = "debug_large_text.djvu";
    let mut file = File::create(output_path)?;
    file.write_all(&output)?;
    
    println!("✅ Large text pattern test created: {} ({} bytes)", output_path, output.len());
    Ok(())
}

fn create_letter_pattern(x: usize, y: usize, width: usize, height: usize) -> bool {
    let char_width = width / 6; // Space for 5 letters plus margins
    let char_height = height / 2;
    let start_y = height / 4;
    let start_x = width / 12;
    
    if y < start_y || y >= start_y + char_height {
        return false;
    }
    
    let local_y = y - start_y;
    let local_x = x.saturating_sub(start_x);
    let char_index = local_x / char_width;
    let char_x = local_x % char_width;
    
    match char_index {
        0 => create_h(char_x, local_y, char_width, char_height),
        1 => create_e(char_x, local_y, char_width, char_height),
        2 => create_l(char_x, local_y, char_width, char_height),
        3 => create_l(char_x, local_y, char_width, char_height),
        4 => create_o(char_x, local_y, char_width, char_height),
        _ => false,
    }
}

fn create_h(x: usize, y: usize, w: usize, h: usize) -> bool {
    (x < 3) || (x > w - 4) || (y > h/2 - 2 && y < h/2 + 2)
}

fn create_e(x: usize, y: usize, w: usize, h: usize) -> bool {
    (x < 3) || (y < 3) || (y > h - 4) || (y > h/2 - 2 && y < h/2 + 2 && x < w - 3)
}

fn create_l(x: usize, y: usize, w: usize, h: usize) -> bool {
    (x < 3) || (y > h - 4)
}

fn create_o(x: usize, y: usize, w: usize, h: usize) -> bool {
    ((x < 3) || (x > w - 4)) && (y > 2 && y < h - 3) ||
    ((y < 3) || (y > h - 4)) && (x > 2 && x < w - 3)
}

fn test_original_pbm_full_size() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Testing Original PBM (Full Size) ===");
    
    if !Path::new("test.pbm").exists() {
        println!("test.pbm not found, skipping full size test");
        return Ok(());
    }
    
    let image = load_pbm("test.pbm")?;
    println!("Loaded full-size PBM: {}x{}", image.width, image.height);
    
    // Find a region with content
    let mut found_content = false;
    let mut sample_x = 0;
    let mut sample_y = 0;
    
    'outer: for y in 0..std::cmp::min(image.height, 100) {
        for x in 0..std::cmp::min(image.width, 100) {
            if image.get_pixel_unchecked(x, y) {
                sample_x = x;
                sample_y = y;
                found_content = true;
                break 'outer;
            }
        }
    }
    
    if !found_content {
        println!("No content found in first 100x100 region, trying middle region...");
        let mid_x = image.width / 2;
        let mid_y = image.height / 2;
        
        'outer2: for y in mid_y.saturating_sub(50)..std::cmp::min(mid_y + 50, image.height) {
            for x in mid_x.saturating_sub(50)..std::cmp::min(mid_x + 50, image.width) {
                if image.get_pixel_unchecked(x, y) {
                    sample_x = x;
                    sample_y = y;
                    found_content = true;
                    break 'outer2;
                }
            }
        }
    }
    
    if found_content {
        println!("Found content at ({}, {}), extracting that region...", sample_x, sample_y);
        
        // Extract a 200x200 region around the found content
        let extract_size = 200;
        let start_x = sample_x.saturating_sub(extract_size / 2);
        let start_y = sample_y.saturating_sub(extract_size / 2);
        let end_x = std::cmp::min(start_x + extract_size, image.width);
        let end_y = std::cmp::min(start_y + extract_size, image.height);
        
        let extract_width = end_x - start_x;
        let extract_height = end_y - start_y;
        
        let mut extracted_image = BitImage::new(extract_width as u32, extract_height as u32)?;
        let mut pixel_count = 0;
        
        for y in 0..extract_height {
            for x in 0..extract_width {
                let src_x = start_x + x;
                let src_y = start_y + y;
                let pixel = image.get_pixel_unchecked(src_x, src_y);
                extracted_image.set_usize(x, y, pixel);
                if pixel {
                    pixel_count += 1;
                }
            }
        }
        
        println!("Extracted {}x{} region with {} foreground pixels", 
                extract_width, extract_height, pixel_count);
        
        // Create DjVu document
        let mut document_encoder = DocumentEncoder::new();
        let page_components = PageComponents::new()
            .with_mask(extracted_image)?;
        
        document_encoder.add_page(page_components)?;
        
        let mut output = Vec::new();
        document_encoder.write_to(&mut output)?;
        
        let output_path = "debug_pbm_content.djvu";
        let mut file = File::create(output_path)?;
        file.write_all(&output)?;
        
        println!("✅ PBM content test created: {} ({} bytes)", output_path, output.len());
    } else {
        println!("No content found in PBM file!");
    }
    
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("JB2 Content Debugging Tool");
    println!("===========================");
    
    // Test 1: Very simple cross pattern
    test_simple_pattern()?;
    
    // Test 2: Larger text pattern
    test_large_text_pattern()?;
    
    // Test 3: Extract content from original PBM
    test_original_pbm_full_size()?;
    
    println!("\n🔍 Debug tests completed!");
    println!("\nGenerated files:");
    println!("  - debug_simple_pattern.djvu (simple cross)");
    println!("  - debug_large_text.djvu (HELLO text)");
    println!("  - debug_pbm_content.djvu (extracted PBM content)");
    println!("\nTry opening these in WinDjView to see which ones display content.");
    
    Ok(())
}
