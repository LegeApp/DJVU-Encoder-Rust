use crate::doc::djvu_dir::{DjVmDir, File, FileType};
use crate::doc::page_encoder::PageComponents;
use crate::{PageEncodeParams, Result};
use byteorder::{BigEndian, WriteBytesExt};
use std::io::{Cursor, Write};

/// A high-level encoder for creating multi-page DjVu documents.
///
/// This struct provides a builder-like interface for assembling a document
/// from multiple pages and then writing it to a stream.
#[derive(Default)]
pub struct DocumentEncoder {
    pages: Vec<Vec<u8>>,
    params: PageEncodeParams,
    dpi: u32,
    gamma: Option<f32>,
}

impl DocumentEncoder {
    /// Creates a new `DocumentEncoder` with default parameters.
    pub fn new() -> Self {
        Self {
            pages: Vec::new(),
            params: PageEncodeParams::default(),
            dpi: 300,
            gamma: Some(2.2),
        }
    }

    /// Sets the default encoding parameters for all subsequent pages.
    pub fn with_params(mut self, params: PageEncodeParams) -> Self {
        self.params = params;
        self
    }

    /// Sets the DPI for all subsequent pages.
    pub fn with_dpi(mut self, dpi: u32) -> Self {
        self.dpi = dpi;
        self
    }
    
    /// Sets the gamma correction value for all subsequent pages.
        pub fn with_gamma(mut self, gamma: Option<f32>) -> Self {
        self.gamma = gamma;
        self
    }

    /// Adds a new page to the document.
    ///
    /// The page is encoded using the parameters set on the `DocumentEncoder`.
    pub fn add_page(&mut self, page_components: PageComponents) -> Result<()> {
        let page_num = (self.pages.len() + 1) as u32;
        let dpm = (self.dpi * 100 / 254) as u32; // Dots per meter
        let rotation = 1; // Default rotation

        let mut encoded_page_bytes =
            page_components.encode(&self.params, page_num, dpm, rotation, self.gamma)?;

        // For multi-page documents we store the inner FORM:DJVU without the
        // leading AT&T magic. Single page documents expect "AT&TFORM", but
        // within a DJVM container only the FORM is stored.
        if encoded_page_bytes.starts_with(b"AT&TFORM") {
            encoded_page_bytes = encoded_page_bytes.split_off(4);
        }

        self.pages.push(encoded_page_bytes);
        Ok(())
    }

    /// Assembles the final DjVu document and writes it to the provided writer.
    pub fn write_to<W: Write>(&self, mut writer: W) -> Result<()> {
        // 1. Create the DIRM component data with correct offsets
        let mut dirm = DjVmDir::new();
        
        // Calculate offsets for each page
        let header_size = 12_usize; // "AT&TFORM" + size + "DJVM"
        let mut current_offset = header_size;
        
        // Account for DIRM chunk (we'll calculate its size first with dummy offsets)
        let mut temp_dirm = DjVmDir::new();
        for i in 0..self.pages.len() {
            let page_id = format!("p{:04}", i + 1);
            let file = File::new(&page_id, &page_id, "", FileType::Page);
            temp_dirm.insert_file(file, -1)?;
        }
        let mut temp_dirm_stream = crate::iff::byte_stream::MemoryStream::new();
        temp_dirm.encode_explicit(&mut temp_dirm_stream, false, true)?; // bundled=false for temp calculation
        let temp_dirm_bytes = temp_dirm_stream.into_vec();
        let dirm_chunk_size = 8 + temp_dirm_bytes.len() + (temp_dirm_bytes.len() % 2); // ID + size + data + padding
        current_offset += dirm_chunk_size;
        
        // Now create the final DIRM with correct offsets
        for i in 0..self.pages.len() {
            let page_id = format!("p{:04}", i + 1);
            let file = File::new_with_offset(
                &page_id, 
                &page_id, 
                "", 
                FileType::Page, 
                current_offset as u32, 
                self.pages[i].len() as u32
            );
            dirm.insert_file(file, -1)?;
            current_offset += self.pages[i].len();
        }
        
        // Encode the final DIRM with correct offsets
        let mut dirm_stream = crate::iff::byte_stream::MemoryStream::new();
        dirm.encode_explicit(&mut dirm_stream, true, true)?;
        let dirm_bytes = dirm_stream.into_vec();

        // 2. Calculate total size
        let final_dirm_chunk_size = 8 + dirm_bytes.len() + (dirm_bytes.len() % 2); // ID + size + data + padding
        let pages_total_size: usize = self.pages.iter().map(|p| p.len()).sum();
        let total_size = 4 + final_dirm_chunk_size + pages_total_size; // "DJVM" + DIRM chunk + pages

        // 3. Write FORM:DJVM header
        writer.write_all(b"AT&TFORM")?;
        writer.write_u32::<BigEndian>(total_size as u32)?;
        writer.write_all(b"DJVM")?;

        // 4. Write DIRM chunk
        writer.write_all(b"DIRM")?;
        writer.write_u32::<BigEndian>(dirm_bytes.len() as u32)?;
        writer.write_all(&dirm_bytes)?;
        if dirm_bytes.len() % 2 != 0 {
            writer.write_u8(0)?; // Padding
        }

        // 5. Write each page
        for page_data in &self.pages {
            writer.write_all(page_data)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::page_encoder::PageComponents;
    use image::RgbImage;

    #[test]
    fn test_document_encoder() -> Result<()> {
        let mut encoder = DocumentEncoder::new();
        let page1 = PageComponents::new().with_background(RgbImage::new(10, 10))?;
        encoder.add_page(page1)?;
        let page2 = PageComponents::new().with_background(RgbImage::new(20, 20))?;
        encoder.add_page(page2)?;
        
        let mut buffer = Cursor::new(Vec::new());
        encoder.write_to(&mut buffer)?;

        let data = buffer.into_inner();
        assert!(data.len() > 20); // Sanity check
        assert_eq!(&data[0..8], b"AT&TFORM");
        assert_eq!(&data[12..16], b"DJVM");
        // A simple search for the DIRM and the nested FORM chunks
        assert!(data.windows(4).any(|w| w == b"DIRM"));
        assert!(data.windows(8).any(|w| w == b"AT&TFORM")); // The nested page form
        assert!(data.windows(4).any(|w| w == b"DJVU"));

        Ok(())
    }
}
