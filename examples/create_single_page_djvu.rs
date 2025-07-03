// examples/create_single_page_djvu.rs
use djvu_encoder::doc::page_encoder::{PageComponents, PageEncodeParams};
use image::RgbImage;
use std::fs::File;
use std::io::Write;
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Creating a single-page DjVu file from an image...");

    let args: Vec<String> = std::env::args().collect();

    // Find the image path (first non-flag argument after the executable)
    let image_path = args
        .iter()
        .skip(1)
        .find(|arg| !arg.starts_with("--"))
        .map(|s| s.as_str())
        .unwrap_or("test.png");

    // Check for the --grayscale flag
    let use_grayscale = args.iter().any(|arg| arg == "--grayscale");
    let color_mode_str = if use_grayscale { "Grayscale" } else { "Color" };

    if !Path::new(image_path).exists() {
        println!("Error: Image file not found at '{}'", image_path);
        println!("Please ensure the image file exists or provide a path as an argument.");
        println!("Usage: cargo run --example create_single_page_djvu -- [path/to/image.png]");
        return Ok(());
    }

    // Load the image
    println!("Loading image '{}'...", image_path);
    let img: RgbImage = image::open(image_path)?.to_rgb8();
    let (width, height) = img.dimensions();
    println!("Loaded image with dimensions: {}x{}", width, height);

    // Create page components. For a simple photo, we just add it as a background.
    println!("Building page components...");
    let page_components = PageComponents::new().with_background(img)?;

    // Create encoding parameters
    let mut params = PageEncodeParams::default();
    params.color = !use_grayscale;
    
    if use_grayscale {
        println!("Encoding in grayscale mode.");
    } else {
        println!("Encoding in color mode (default). Use --grayscale to disable.");
    }

    println!("Encoding single page...");
    // Encode the page directly as a single-page DjVu document
    let page_data = page_components.encode(&params, 1, 300, 1, Some(2.2))?;
    println!("Page encoded successfully.");

    // Save the document
    let output_path = "output_single.djvu";
    let mut file = File::create(output_path)?;

    println!("Writing document to '{}'...", output_path);
    file.write_all(&page_data)?;

    println!("\n✅ Successfully created single-page DjVu document: {}", output_path);
    println!("   File size: {} bytes", std::fs::metadata(output_path)?.len());
    println!("   Pages: 1");
    println!("   Source Image: {}", image_path);
    println!("   Image Dimensions: {}x{}", width, height);
    println!("   Color Mode: {}", color_mode_str);

    Ok(())
}
