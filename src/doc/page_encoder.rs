//! Page encoding functionality for DjVu documents

use crate::encode::{
    iw44::encoder::{EncoderParams as IW44EncoderParams, IWEncoder},
    jb2::encoder::JB2Encoder,
    symbol_dict::BitImage,
};
use crate::iff::iff::IffWriter;
use crate::{DjvuError, Result};
use byteorder::{BigEndian, WriteBytesExt};
use image::RgbImage;
use lutz::Image;
use std::io::{self, Write};

/// Configuration for page encoding
#[derive(Debug, Clone)]
pub struct PageEncodeParams {
    /// Dots per inch (default: 300)
    pub dpi: u32,
    /// Background quality (0-100, higher is better quality)
    pub bg_quality: u8,
    /// Foreground quality (0-100, higher is better quality)
    pub fg_quality: u8,
    /// Whether to use IW44 for background (true) or JB2 (false)
    pub use_iw44: bool,
    /// Whether to encode in color (true) or grayscale (false)
    pub color: bool,
}

impl Default for PageEncodeParams {
    fn default() -> Self {
        Self {
            dpi: 300,
            bg_quality: 90,
            fg_quality: 90,
            use_iw44: true, // Default to IW44 for background
            color: true,    // Default to color encoding
        }
    }
}

/// Represents a single page's components for encoding.
///
/// Use `PageComponents::new()` to create an empty page, then add components
/// like background, foreground, and mask using the `with_*` methods.
/// The dimensions of the first image added will set the dimensions for the page.
#[derive(Default)]
pub struct PageComponents {
    /// Page width in pixels
    width: u32,
    /// Page height in pixels
    height: u32,
    /// Optional background image data (for IW44)
    pub background: Option<RgbImage>,
    /// Optional foreground image data (for JB2)
    pub foreground: Option<BitImage>,
    /// Optional mask data (bitonal)
    pub mask: Option<BitImage>,
    /// Optional text/annotations
    pub text: Option<String>,
}

impl PageComponents {
    /// Creates a new, empty page.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the dimensions of the page.
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Checks and sets the page dimensions if they are not already set.
    /// Returns an error if the new dimensions conflict with existing ones.
    fn check_and_set_dimensions(&mut self, new_dims: (u32, u32)) -> Result<()> {
        if self.width == 0 && self.height == 0 {
            self.width = new_dims.0;
            self.height = new_dims.1;
        } else if self.width != new_dims.0 || self.height != new_dims.1 {
            return Err(DjvuError::InvalidOperation(format!(
                "Dimension mismatch: expected {}x{}, got {}x{}",
                self.width, self.height, new_dims.0, new_dims.1
            )));
        }
        Ok(())
    }

    /// Adds a background image to the page.
    pub fn with_background(mut self, image: RgbImage) -> Result<Self> {
        self.check_and_set_dimensions(image.dimensions())?;
        self.background = Some(image);
        Ok(self)
    }

    /// Adds a foreground image to the page.
    pub fn with_foreground(mut self, image: BitImage) -> Result<Self> {
        self.check_and_set_dimensions((image.width(), image.height()))?;
        self.foreground = Some(image);
        Ok(self)
    }

    /// Adds a mask to the page.
    pub fn with_mask(mut self, image: BitImage) -> Result<Self> {
        self.check_and_set_dimensions((image.width(), image.height()))?;
        self.mask = Some(image);
        Ok(self)
    }

    /// Adds text/annotations to the page.
    pub fn with_text(mut self, text: String) -> Self {
        self.text = Some(text);
        self
    }

    /// Encodes the page to a byte vector using the given parameters
    pub fn encode(
        &self,
        params: &PageEncodeParams,
        page_num: u32,
        dpm: u32,
        rotation: u8,       // 1=0°, 6=90°CCW, 2=180°, 5=90°CW
        gamma: Option<f32>, // If None, use 2.2
    ) -> Result<Vec<u8>> {
        let mut output = Vec::new();
        {
            let mut cursor = io::Cursor::new(&mut output);
            let mut writer = IffWriter::new(&mut cursor);

            // Write AT&T magic bytes first
            writer.write_magic_bytes()?;

            // Start the FORM:DJVU chunk
            writer.put_chunk("FORM:DJVU")?;

            // Write INFO chunk (required for all pages)
            self.write_info_chunk(
                &mut writer,
                params.dpi as u16,
                page_num,
                dpm,
                rotation,
                gamma,
            )?;

            // Encode and write background if present
            if let Some(bg_img) = &self.background {
                if params.use_iw44 {
                    self.encode_iw44_background(bg_img, &mut writer, params)?
                } else {
                    // JB2 background encoding from RGB is not supported.
                    // A bitonal image should be provided as a foreground component instead.
                    return Err(DjvuError::InvalidOperation(
                        "JB2 background encoding requires a bitonal image. Use foreground instead."
                            .to_string(),
                    ));
                }
            }

            // Encode and write foreground if present
            if let Some(fg_img) = &self.foreground {
                self.encode_jb2_foreground(fg_img, &mut writer, params.fg_quality)?;
            }

            // Encode and write mask if present
            if let Some(mask_img) = &self.mask {
                self.encode_jb2_mask(mask_img, &mut writer)?;
            }

            // Write text/annotations if present
            if let Some(text) = &self.text {
                self.write_text_chunk(text, &mut writer)?;
            }

            // Close the FORM:DJVU chunk
            writer.close_chunk()?;
        }
        Ok(output)
    }

    /// Writes the INFO chunk as per DjVu spec (10 bytes)
    /// Format: width(2,BE) height(2,BE) minor_ver(1) major_ver(1) dpi(2,LE) gamma(1) flags(1)
    fn write_info_chunk(
        &self,
        writer: &mut IffWriter,
        dpi: u16,
        _page_num: u32,
        _dpm: u32,
        rotation: u8,       // 1=0°, 6=90°CCW, 2=180°, 5=90°CW
        gamma: Option<f32>, // If None, use 2.2
    ) -> Result<()> {
        use byteorder::LittleEndian;

        writer.put_chunk("INFO")?;

        // Width and height (2 bytes each, big-endian)
        writer.write_u16::<BigEndian>(self.width as u16)?;
        writer.write_u16::<BigEndian>(self.height as u16)?;

        // Minor version (1 byte, currently 26 per spec)
        writer.write_u8(26)?;

        // Major version (1 byte, currently 0 per spec)
        writer.write_u8(0)?;

        // DPI (2 bytes, little-endian per spec)
        writer.write_u16::<LittleEndian>(dpi)?;

        // Gamma (1 byte, gamma * 10)
        let gamma_val = gamma.map_or(22, |g| (g * 10.0 + 0.5) as u8); // Default gamma = 2.2
        writer.write_u8(gamma_val)?;

        // Flags (1 byte: bits 0-2 = rotation, bits 3-7 = reserved)
        let flags = rotation & 0x07; // Ensure only bottom 3 bits are used
        writer.write_u8(flags)?;

        writer.close_chunk()?;
        Ok(())
    }

    /// Encodes the background using IW44 (wavelet)
    fn encode_iw44_background(
        &self,
        img: &RgbImage,
        writer: &mut IffWriter,
        params: &PageEncodeParams,
    ) -> Result<()> {
        let crcb_mode = if params.color {
            crate::encode::iw44::encoder::CrcbMode::Full
        } else {
            crate::encode::iw44::encoder::CrcbMode::None
        };

        // Debug: Check input image properties
        let (w, h) = img.dimensions();
        let raw_data = img.as_raw();
        println!("DEBUG: Input image {}x{}, {} bytes", w, h, raw_data.len());

        // Check some sample pixels
        if raw_data.len() >= 9 {
            println!(
                "DEBUG: First 3 pixels: RGB({},{},{}) RGB({},{},{}) RGB({},{},{})",
                raw_data[0],
                raw_data[1],
                raw_data[2],
                raw_data[3],
                raw_data[4],
                raw_data[5],
                raw_data[6],
                raw_data[7],
                raw_data[8]
            );
        }

        // Configure IW44 encoder with proper quality-based parameters
        // Calculate reasonable slice count based on quality (10-100 slices for normal quality)
        let slice_count = 50; // Enough for multiple bit-planes
        println!(
            "DEBUG: Configuring IW44 encoder with {} slices for quality {}",
            slice_count, params.bg_quality
        );

        let iw44_params = IW44EncoderParams {
            slices: Some(slice_count),
            bytes: None,
            decibels: None,
            crcb_mode,
            max_slices: Some(200), // Reasonable safety limit
            ..Default::default()
        };

        // Encode the image
        let mut encoder = IWEncoder::from_rgb(img, None, iw44_params)
            .map_err(|e| DjvuError::EncodingError(e.to_string()))?;

        // Choose the correct chunk type for IW44 background images:
        // - BG44 for background layer (the main use case for IW44 in DjVu pages)
        // - FG44 for foreground layer (has mask)
        // Note: PM44/BM44 are for standalone IW44 files, not DjVu page backgrounds
        let iw_chunk_id = if self.mask.is_some() {
            "FG44"
        } else {
            "BG44" // Use BG44 for background images in DjVu pages
        };

        // Encode all slices, writing one chunk per slice with the proper 'more' flag
        let mut chunk_count = 0;
        loop {
            let (iw44_stream, more) = encoder
                .encode_chunk()
                .map_err(|e| DjvuError::EncodingError(e.to_string()))?;

            // Don't write empty slices - just break out
            if iw44_stream.is_empty() {
                println!("DEBUG: Encoder returned empty slice, stopping");
                break;
            }

            // Write this slice as a separate BG44/FG44 chunk
            writer.put_chunk(iw_chunk_id)?;
            writer.write_all(&iw44_stream)?;
            writer.close_chunk()?;

            chunk_count += 1;
            println!(
                "DEBUG: Writing {} chunk {}, size: {} bytes, more: {}",
                iw_chunk_id,
                chunk_count,
                iw44_stream.len(),
                more
            );

            if !more {
                break;
            }

            // Safety check to prevent infinite loops
            if chunk_count >= 100 {
                eprintln!(
                    "Warning: Too many IW44 chunks generated ({}), stopping",
                    chunk_count
                );
                break;
            }
        }

        println!("DEBUG: Total IW44 chunks written: {}", chunk_count);

        Ok(())
    }

    /// Encodes the foreground using JB2
    fn encode_jb2_foreground(
        &self,
        img: &BitImage,
        writer: &mut IffWriter,
        _quality: u8,
    ) -> Result<()> {
        // Create JB2 encoder and encode
        let mut jb2_encoder = JB2Encoder::new(Vec::new());
        let encoded = jb2_encoder.encode_page(img, 0)?;

        // Write FGbz chunk
        writer.put_chunk("FGbz")?;
        writer.write_all(&encoded)?;
        writer.close_chunk()?;

        Ok(())
    }

    /// Encodes the mask using JB2
    fn encode_jb2_mask(&self, img: &BitImage, writer: &mut IffWriter) -> Result<()> {
        // Create JB2 encoder and encode
        let mut jb2_encoder = JB2Encoder::new(Vec::new());
        let encoded = jb2_encoder.encode_page(img, 0)?;

        // Write Sjbz chunk
        writer.put_chunk("Sjbz")?;
        writer.write_all(&encoded)?;
        writer.close_chunk()?;

        Ok(())
    }

    /// Writes the text/annotations chunk
    fn write_text_chunk(&self, text: &str, writer: &mut IffWriter) -> Result<()> {
        writer.put_chunk("TXTa")?;
        writer.write_all(text.as_bytes())?;
        writer.close_chunk()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode::symbol_dict::BitImage;
    use image::{Rgb, RgbImage};

    #[test]
    fn test_page_encoding_with_builder() {
        // Create a simple white background image
        let bg_image = RgbImage::from_pixel(100, 200, Rgb([255, 255, 255]));

        // Use the builder pattern to create the page
        let page = PageComponents::new()
            .with_background(bg_image)
            .unwrap()
            .with_text("Hello, DjVu!".to_string());

        assert_eq!(page.dimensions(), (100, 200));

        // Encode with default parameters
        let params = PageEncodeParams::default();
        let result = page.encode(&params, 1, 300, 1, Some(2.2));

        assert!(result.is_ok());
        let encoded = result.unwrap();

        // Basic validation of the encoded data
        assert!(!encoded.is_empty());
        // Check for FORM:DJVU header
        assert_eq!(&encoded[0..8], b"AT&TFORM");
        // Check for INFO chunk
        assert!(encoded.windows(4).any(|w| w == b"INFO"));
        // Check for PM44 chunk (since that's the default for color images)
        assert!(encoded.windows(4).any(|w| w == b"PM44"));
        // Check for text chunk
        assert!(encoded.windows(4).any(|w| w == b"TXTa"));
    }

    #[test]
    fn test_dimension_mismatch() {
        let bg_image = RgbImage::new(100, 200);
        let fg_image = BitImage::new(101, 201); // Different dimensions

        let result = PageComponents::new()
            .with_background(bg_image)
            .unwrap()
            .with_foreground(fg_image.unwrap());

        assert!(result.is_err());
        if let Err(DjvuError::InvalidOperation(msg)) = result {
            assert!(msg.contains("Dimension mismatch"));
        } else {
            panic!("Expected a DimensionMismatch error");
        }
    }
}
