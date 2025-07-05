use djvu_encoder::encode::jb2::symbol_dict::BitImage;
use djvu_encoder::encode::zc::ZEncoder;
use djvu_encoder::doc::DocumentEncoder;
use djvu_encoder::doc::page_encoder::PageComponents;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Testing ZP-based JB2 encoding...");
    
    // Create a simple test image
    let test_width = 100;
    let test_height = 100;
    let mut image = BitImage::new(test_width as u32, test_height as u32)?;
    
    // Create a simple pattern - a cross
    for x in 0..test_width {
        image.set_usize(x, test_height / 2, true); // Horizontal line
    }
    for y in 0..test_height {
        image.set_usize(test_width / 2, y, true); // Vertical line
    }
    
    println!("Created test image: {}x{}", test_width, test_height);
    
    // Count black pixels for verification
    let mut black_pixels = 0;
    for y in 0..test_height {
        for x in 0..test_width {
            if image.get_pixel_unchecked(x, y) {
                black_pixels += 1;
            }
        }
    }
    println!("Black pixels: {}", black_pixels);
    
    // Test basic ZP encoding
    println!("\n=== Testing basic ZP encoder ===");
    let mut zp_buffer = Vec::new();
    let mut zc = ZEncoder::new(&mut zp_buffer, true)?; // DjVu compatible mode
    
    // Encode a simple pattern
    let mut ctx = 0u8;
    for i in 0..10 {
        let bit = i % 2 == 0;
        zc.encode(bit, &mut ctx)?;
        println!("Encoded bit {}: {}", i, bit);
    }
    
    let writer = zc.finish()?;
    println!("Z codec encoded {} bits into {} bytes", 10, writer.len());
    
    // Test with actual image using document encoder
    println!("\n=== Testing with document encoder ===");
    let mut document_encoder = DocumentEncoder::new();
    
    // Create page with the cross pattern
    let page = PageComponents::new()
        .with_mask(image)?;
    document_encoder.add_page(page)?;
    
    // Write to file
    let mut output = Vec::new();
    document_encoder.write_to(&mut output)?;
    std::fs::write("zc_output.djvu", &output)?;
    
    println!("Created zc_output.djvu ({} bytes)", output.len());
    
    // Test with djvudump
    println!("\n=== Testing with djvudump ===");
    let output = std::process::Command::new("djvudump")
        .arg("zc_output.djvu")
        .output();
    
    match output {
        Ok(output) => {
            if output.status.success() {
                println!("djvudump output:");
                println!("{}", String::from_utf8_lossy(&output.stdout));
            } else {
                println!("djvudump error: {}", String::from_utf8_lossy(&output.stderr));
            }
        }
        Err(e) => {
            println!("djvudump not available: {}", e);
            println!("Skipping djvudump test");
        }
    }
    
    // Test rendering with ddjvu
    println!("\n=== Testing visibility with ddjvu ===");
    let output = std::process::Command::new("ddjvu")
        .args(&["-format=pbm", "-page=1", "zc_output.djvu", "zc_test_output.pbm"])
        .output();
    
    match output {
        Ok(output) => {
            if output.status.success() {
                println!("Successfully converted to PBM");
                
                if let Ok(metadata) = std::fs::metadata("zc_test_output.pbm") {
                    println!("Output PBM size: {} bytes", metadata.len());
                    
                    // Quick check for black pixels in output
                    if let Ok(content) = std::fs::read_to_string("zc_test_output.pbm") {
                        let black_count = content.matches('1').count();
                        println!("Output contains {} '1' characters (potential black pixels)", black_count);
                        
                        if black_count > 0 {
                            println!("✅ SUCCESS: Output contains visible content!");
                        } else {
                            println!("❌ ISSUE: Output appears blank");
                        }
                    }
                }
            } else {
                println!("ddjvu error: {}", String::from_utf8_lossy(&output.stderr));
            }
        }
        Err(e) => {
            println!("ddjvu not available: {}", e);
            println!("Skipping ddjvu test");
        }
    }
    
    Ok(())
}
