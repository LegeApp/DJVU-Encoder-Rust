// examples/minimal_test.rs
use djvu_encoder::doc::document_encoder::DocumentEncoder;
use djvu_encoder::doc::page_encoder::PageComponents;
use image::RgbImage;
use std::fs::File;
use std::io::BufWriter;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Creating minimal DjVu test...");

    // Create a tiny 4x4 red image
    let mut img = RgbImage::new(4, 4);
    for pixel in img.pixels_mut() {
        *pixel = image::Rgb([255, 0, 0]); // Red
    }

    println!("Created 4x4 red image");

    // Create a single page
    let page_components = PageComponents::new().with_background(img)?;

    // Create document with just one page
    let mut doc_encoder = DocumentEncoder::new().with_color(true);
    doc_encoder.add_page(page_components)?;

    // Write to file
    let output_file = File::create("minimal_test.djvu")?;
    let writer = BufWriter::new(output_file);
    doc_encoder.write_to(writer)?;

    println!("✅ Created minimal_test.djvu");

    Ok(())
}
