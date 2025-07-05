// examples/create_djvu.rs
use djvu_encoder::doc::document_encoder::DocumentEncoder;
use djvu_encoder::doc::page_encoder::PageComponents;
use image::RgbImage;
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Creating a DjVu document from an image...");

    let args: Vec<String> = std::env::args().collect();

    // A simple helper to find the value of a command-line argument.
    let find_arg_value = |flag: &str| -> Option<&str> {
        args.iter()
            .position(|arg| arg == flag)
            .and_then(|i| args.get(i + 1))
            .map(|s| s.as_str())
    };

    // Get input and output paths from arguments.
    let image_path = find_arg_value("--input").unwrap_or("test.png");
    let output_path = find_arg_value("--output").unwrap_or("output.djvu");

    // Check for the --grayscale flag
    let use_grayscale = args.iter().any(|arg| arg == "--grayscale");
    let decibels_str = find_arg_value("--decibels");
    let decibels: Option<f32> = decibels_str.and_then(|s| s.parse().ok());
    let color_mode_str = if use_grayscale { "Grayscale" } else { "Color" };

    if !Path::new(image_path).exists() {
        println!("Error: Image file not found at '{}'", image_path);
        println!("Please ensure the image file exists or provide a path as an argument.");
        println!("Usage: cargo run --example create_djvu -- --input [path/to/image.png] --output [path/to/output.djvu]");
        return Ok(());
    }

    // Load the image
    println!("Loading image '{}'...", image_path);
    let img: RgbImage = image::open(image_path)?.to_rgb8();
    let (width, height) = img.dimensions();
    println!("Loaded image with dimensions: {}x{}", width, height);

    // Create page components for one page (for comparison).
    println!("Building page components for page 1...");
    let page_components1 = PageComponents::new().with_background(img.clone())?;

    // Create a document encoder and set color mode based on the flag.
    let mut doc_encoder = DocumentEncoder::new();

    if let Some(db) = decibels {
        doc_encoder = doc_encoder.with_decibels(db);
        println!("Encoding with target decibels: {}", db);
    }

    if use_grayscale {
        doc_encoder = doc_encoder.with_color(false);
        println!("Encoding in grayscale mode.");
    } else {
        println!("Encoding in color mode (default). Use --grayscale to disable.");
    }

    // Add just one page to compare with working format
    println!("Adding page 1 to document...");
    doc_encoder.add_page(page_components1)?;
    println!("Page 1 added successfully.");

    // Save the document
    let file = File::create(output_path)?;
    let mut writer = BufWriter::new(file);

    println!("Writing document to '{}'...", output_path);
    doc_encoder.write_to(&mut writer)?;

    println!("\n✅ Successfully created DjVu document: {}", output_path);
    println!(
        "   File size: {} bytes",
        std::fs::metadata(output_path)?.len()
    );
    println!("   Pages: {}", doc_encoder.page_count());
    println!("   Source Image: {}", image_path);
    println!("   Image Dimensions: {}x{}", width, height);
    println!("   Color Mode: {}", color_mode_str);

    Ok(())
}
