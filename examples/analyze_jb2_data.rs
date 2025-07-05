// examples/analyze_jb2_data.rs
//! Analyze JB2 data to understand why it's not decodable
//!
//! This example examines the raw JB2 data produced by our encoder
//! and compares it with known-good DjVu files to identify issues.

use djvu_encoder::encode::jb2::symbol_dict::BitImage;
use djvu_encoder::encode::jb2::encoder::JB2Encoder;
use std::fs::File;
use std::io::{Read, Write};

fn analyze_jb2_raw_data() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Analyzing JB2 Raw Data ===");
    
    // Create a very simple 10x10 test pattern
    let mut image = BitImage::new(10, 10)?;
    
    // Just a single pixel in the center
    image.set_usize(5, 5, true);
    
    println!("Created 10x10 image with single pixel at (5,5)");
    
    // Encode with JB2
    let mut jb2_encoder = JB2Encoder::new(Vec::new());
    let jb2_data = jb2_encoder.encode_page(&image, 10)?;
    
    println!("JB2 data size: {} bytes", jb2_data.len());
    
    // Analyze the raw JB2 data
    if jb2_data.len() >= 8 {
        println!("First 8 bytes (hex): {:02X?}", &jb2_data[..8]);
        
        // Check for expected JB2 chunk headers
        if jb2_data.len() >= 4 {
            let chunk1_id = String::from_utf8_lossy(&jb2_data[..4]);
            println!("First chunk ID: '{}'", chunk1_id);
            
            if jb2_data.len() >= 7 {
                let chunk1_size = u32::from_be_bytes([0, jb2_data[4], jb2_data[5], jb2_data[6]]) as usize;
                println!("First chunk size: {} bytes", chunk1_size);
                
                if jb2_data.len() >= 7 + chunk1_size && chunk1_size > 0 {
                    let second_chunk_start = 4 + 3 + chunk1_size;
                    if second_chunk_start + 4 <= jb2_data.len() {
                        let chunk2_id = String::from_utf8_lossy(&jb2_data[second_chunk_start..second_chunk_start + 4]);
                        println!("Second chunk ID: '{}'", chunk2_id);
                    }
                }
            }
        }
    }
    
    // Save for external analysis
    let mut file = File::create("analyze_jb2_single_pixel.dat")?;
    file.write_all(&jb2_data)?;
    println!("Saved raw JB2 data to: analyze_jb2_single_pixel.dat");
    
    // Try different sizes to see if size affects the issue
    for size in [5, 20, 50] {
        let mut test_image = BitImage::new(size, size)?;
        
        // Create a simple pattern
        for i in 0..size as usize {
            test_image.set_usize(i, i, true); // Diagonal line
        }
        
        let mut encoder = JB2Encoder::new(Vec::new());
        let data = encoder.encode_page(&test_image, 10)?;
        
        println!("{}x{} diagonal pattern: {} bytes", size, size, data.len());
        
        let filename = format!("analyze_jb2_{}x{}.dat", size, size);
        let mut file = File::create(&filename)?;
        file.write_all(&data)?;
    }
    
    Ok(())
}

fn compare_with_reference() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Comparing with Reference DjVu ===");
    
    if std::path::Path::new("working.djvu").exists() {
        println!("Extracting page 150 from working.djvu for comparison...");
        
        // Extract page 150 as a separate DjVu file for easier analysis
        let extract_result = std::process::Command::new("./djvused.exe")
            .args(&["-e", "select 150; save-page page150.djvu", "working.djvu"])
            .output();
            
        match extract_result {
            Ok(output) => {
                if output.status.success() {
                    println!("Successfully extracted page 150 to page150.djvu");
                    
                    // Now analyze the extracted page
                    if std::path::Path::new("page150.djvu").exists() {
                        analyze_djvu_page("page150.djvu")?;
                    }
                } else {
                    println!("Failed to extract page 150: {}", String::from_utf8_lossy(&output.stderr));
                    
                    // Fallback: analyze the whole working.djvu file
                    println!("Falling back to analyzing working.djvu directly...");
                    analyze_djvu_page("working.djvu")?;
                }
            }
            Err(e) => {
                println!("Could not run djvused: {}", e);
                println!("Analyzing working.djvu directly...");
                analyze_djvu_page("working.djvu")?;
            }
        }
    } else {
        println!("working.djvu not found, checking for other reference files...");
        
        let ref_files = ["minimal_test.djvu", "our_bg.iw4", "ref.iw4"];
        
        for ref_file in &ref_files {
            if std::path::Path::new(ref_file).exists() {
                println!("Analyzing reference file: {}", ref_file);
                analyze_djvu_page(ref_file)?;
                break;
            }
        }
    }
    
    Ok(())
}

fn analyze_djvu_page(filename: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("Analyzing DjVu file: {}", filename);
    
    let mut file = File::open(filename)?;
    let mut data = Vec::new();
    file.read_to_end(&mut data)?;
    
    println!("File size: {} bytes", data.len());
    
    // Look for JB2 chunks in the file
    let mut sjbz_found = 0;
    let mut jb2d_found = 0;
    
    for i in 0..data.len().saturating_sub(7) {
        let chunk_id = String::from_utf8_lossy(&data[i..i+4]);
        if chunk_id == "Sjbz" || chunk_id == "JB2D" || chunk_id == "JB2I" || chunk_id == "FGbz" {
            if chunk_id == "Sjbz" { sjbz_found += 1; }
            if chunk_id == "JB2D" { jb2d_found += 1; }
            
            println!("Found {} chunk at offset {}", chunk_id, i);
            
            // Parse the 24-bit size field correctly  
            let size_bytes = [data[i+4], data[i+5], data[i+6]];
            let chunk_size = ((size_bytes[0] as u32) << 16) | 
                           ((size_bytes[1] as u32) << 8) | 
                           (size_bytes[2] as u32);
            println!("  Chunk size: {} bytes", chunk_size);
            
            if i + 7 + chunk_size as usize <= data.len() && chunk_size > 0 && chunk_size < 100000 {
                let chunk_data = &data[i+7..i+7+chunk_size as usize];
                println!("  First 16 bytes: {:02X?}", &chunk_data[..std::cmp::min(16, chunk_data.len())]);
                
                // For Sjbz chunks, save the first one for detailed analysis
                if chunk_id == "Sjbz" && sjbz_found == 1 {
                    let mut ref_file = File::create("reference_sjbz_chunk.dat")?;
                    ref_file.write_all(chunk_data)?;
                    println!("  → Saved reference Sjbz chunk to reference_sjbz_chunk.dat");
                }
                
                // For JB2D chunks, save the first one
                if chunk_id == "JB2D" && jb2d_found == 1 {
                    let mut ref_file = File::create("reference_jb2d_chunk.dat")?;
                    ref_file.write_all(chunk_data)?;
                    println!("  → Saved reference JB2D chunk to reference_jb2d_chunk.dat");
                }
            }
        }
    }
    
    println!("Summary: Found {} Sjbz chunks, {} JB2D chunks", sjbz_found, jb2d_found);
    
    Ok(())
}

fn test_arithmetic_coder_directly() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Testing Arithmetic Coder Directly ===");
    
    use djvu_encoder::encode::zc::zcodec::ZEncoder;

    use std::io::Cursor;
    use djvu_encoder::encode::zc::zcodec::BitContext;
    
    // Test the arithmetic coder with minimal data
    let mut buffer = Cursor::new(Vec::new());
    
    let encoded_data = {
        let mut ac = ZEncoder::new(
            buffer,
            true
        )?; 
        
        // Encode a few simple bits
        let mut ctx0 = 0 as BitContext;
        let mut ctx1 = 1 as BitContext;
        let mut ctx2 = 2 as BitContext;
        let mut ctx3 = 3 as BitContext;

        ac.encode(false, &mut ctx0).unwrap();
        ac.encode(true, &mut ctx1).unwrap();
        ac.encode(false, &mut ctx2).unwrap();
        ac.encode(true, &mut ctx3).unwrap();
        
        ac.finish()?.into_inner()
    };
    println!("Arithmetic coder test: {} bytes", encoded_data.len());
    println!("Data: {:02X?}", encoded_data);
    
    // Save for analysis
    let mut file = File::create("arithmetic_coder_test.dat")?;
    file.write_all(&encoded_data)?;
    
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("JB2 Data Analysis Tool");
    println!("======================");
    
    // Analyze what our JB2 encoder produces
    analyze_jb2_raw_data()?;
    
    // Compare with reference files if available
    compare_with_reference()?;
    
    // Test arithmetic coder directly
    test_arithmetic_coder_directly()?;
    
    println!("\n🔍 Analysis complete!");
    println!("\nGenerated analysis files:");
    println!("  - analyze_jb2_single_pixel.dat");
    println!("  - analyze_jb2_5x5.dat");
    println!("  - analyze_jb2_20x20.dat");
    println!("  - analyze_jb2_50x50.dat");
    println!("  - arithmetic_coder_test.dat");
    
    println!("\nNext steps:");
    println!("1. Compare these with known-good JB2 data");
    println!("2. Check if the chunk structure matches DjVu spec");
    println!("3. Verify arithmetic coder output");
    
    Ok(())
}
