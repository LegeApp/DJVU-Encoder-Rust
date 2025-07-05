use djvu_encoder::encode::iw44::{encoder::IWEncoder, encoder::EncoderParams};
use image::{GrayImage, RgbImage};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a simple test image
    let img = GrayImage::new(64, 64);
    
    // Create encoder
    let params = EncoderParams::default();
    let mut encoder = IWEncoder::from_gray(&img, None, params)?;
    
    // Encode a chunk
    let (chunk_data, has_more) = encoder.encode_chunk(10)?;
    
    println!("Encoded {} bytes, has_more: {}", chunk_data.len(), has_more);
    
    Ok(())
}
