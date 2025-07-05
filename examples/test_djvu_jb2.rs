// examples/test_djvu_jb2.rs
//! Test the new DjVu-compatible JB2 encoder
//!
//! This tests the JB2 encoder that follows the official DjVu specification
//! and produces proper Sjbz chunks.

use djvu_encoder::encode::jb2::djvu_jb2_encoder::DjvuJb2Encoder;
use djvu_encoder::encode::jb2::symbol_dict::BitImage;
use std::fs::File;
use std::io::Write;

/// Create a simple test pattern
fn create_test_pattern(width: usize, height: usize, pattern_type: &str) -> Result<BitImage, Box<dyn std::error::Error>> {
    let mut image = BitImage::new(width as u32, height as u32)?;
    let mut pixel_count = 0;
    
    match pattern_type {
        "single_pixel" => {
            // Single pixel in center
            let center_x = width / 2;
            let center_y = height / 2;
            image.set_usize(center_x, center_y, true);
            pixel_count = 1;
        },
        "cross" => {
            // Cross pattern
            let center_x = width / 2;
            let center_y = height / 2;
            
            for x in 0..width {
                image.set_usize(x, center_y, true);
                pixel_count += 1;
            }
            for y in 0..height {
                image.set_usize(center_x, y, true);
                pixel_count += 1;
            }
            pixel_count -= 1; // Don't double-count center
        },
        "border" => {
            // Border rectangle
            for x in 0..width {
                image.set_usize(x, 0, true);
                image.set_usize(x, height - 1, true);
                pixel_count += 2;
            }
            for y in 1..height-1 {
                image.set_usize(0, y, true);
                image.set_usize(width - 1, y, true);
                pixel_count += 2;
            }
        },
        "text" => {
            // Simple text-like pattern
            for y in height/4..3*height/4 {
                for x in width/6..5*width/6 {
                    if (x % 8 == 0) || (y % 12 == 0) || (x % 20 == 0 && y % 8 == 0) {
                        image.set_usize(x, y, true);
                        pixel_count += 1;
                    }
                }
            }
        },
        _ => {
            return Err("Unknown pattern type".into());
        }
    }
    
    println!("Created '{}' pattern ({}x{}) with {} pixels", pattern_type, width, height, pixel_count);
    
    // Show visual representation for small patterns
    if width <= 20 && height <= 15 {
        println!("Visual representation:");
        for y in 0..height {
            for x in 0..width {
                print!("{}", if image.get_pixel_unchecked(x, y) { "█" } else { "·" });
            }
            println!();
        }
    }
    
    Ok(image)
}

fn test_pattern(name: &str, pattern: &str, width: usize, height: usize) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Testing {} ({}) ===", name, pattern);
    
    // Create test pattern
    let image = create_test_pattern(width, height, pattern)?;
    
    // Create DjVu JB2 encoder
    let mut encoder = DjvuJb2Encoder::new(Vec::new());
    
    // Encode the image
    let jb2_data = encoder.encode_single_page(&image)?;
    println!("✅ DjVu JB2 encoding successful! Size: {} bytes", jb2_data.len());
    
    // Analyze the data
    if jb2_data.len() >= 10 {
        println!("First 10 bytes: {:02X?}", &jb2_data[0..10]);
        
        // Check if it starts with expected arithmetic-coded data (not "JB2D")
        let starts_with_jb2d = jb2_data.len() >= 4 && 
            &jb2_data[0..4] == b"JB2D";
        
        if starts_with_jb2d {
            println!("⚠️  Still starts with JB2D - need to fix encoder");
        } else {
            println!("✅ Starts with arithmetic-coded data (not JB2D)");
        }
    }
    
    // Create a complete DjVu file with proper Sjbz chunk
    let mut djvu_file = Vec::new();
    
    // FORM:DJVU header
    djvu_file.extend_from_slice(b"FORM");
    let file_size = 4 + 4 + 10 + 4 + 3 + jb2_data.len(); // DJVU + INFO chunk + Sjbz header + data
    djvu_file.extend_from_slice(&(file_size as u32).to_be_bytes());
    djvu_file.extend_from_slice(b"DJVU");
    
    // INFO chunk
    djvu_file.extend_from_slice(b"INFO");
    djvu_file.extend_from_slice(&10u32.to_be_bytes()); // INFO chunk size
    djvu_file.extend_from_slice(&(width as u16).to_be_bytes());
    djvu_file.extend_from_slice(&(height as u16).to_be_bytes());
    djvu_file.push(26); // version
    djvu_file.extend_from_slice(&(300u16).to_be_bytes()); // dpi as 16-bit
    djvu_file.push(220); // gamma * 10 = 2.2 * 100 = 220
    djvu_file.push(0); // orientation
    
    // Sjbz chunk
    djvu_file.extend_from_slice(b"Sjbz");
    djvu_file.extend_from_slice(&(jb2_data.len() as u32).to_be_bytes());
    djvu_file.extend_from_slice(&jb2_data);
    
    // Pad to even length if needed
    if djvu_file.len() % 2 == 1 {
        djvu_file.push(0);
    }
    
    // Write to file
    let filename = format!("djvu_jb2_{}.djvu", name);
    let mut file = File::create(&filename)?;
    file.write_all(&djvu_file)?;
    
    println!("✅ Created DjVu file: {} ({} bytes)", filename, djvu_file.len());
    
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("DjVu-Compatible JB2 Encoder Test");
    println!("=================================");
    
    // Test various patterns with different sizes
    test_pattern("single_pixel_10x10", "single_pixel", 10, 10)?;
    test_pattern("cross_20x15", "cross", 20, 15)?;
    test_pattern("border_30x20", "border", 30, 20)?;
    test_pattern("text_50x30", "text", 50, 30)?;
    
    println!("\n🎉 All DjVu JB2 tests completed!");
    println!("\nGenerated files:");
    println!("  - djvu_jb2_single_pixel_10x10.djvu");
    println!("  - djvu_jb2_cross_20x15.djvu");
    println!("  - djvu_jb2_border_30x20.djvu");
    println!("  - djvu_jb2_text_50x30.djvu");
    println!("\n🔍 Test these files in WinDjView and ddjvu to see if they work!");
    
    Ok(())
}
