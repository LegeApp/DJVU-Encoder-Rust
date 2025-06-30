//! # DjVu Encoder Library
//!
//! A Rust port of the DjVu encoder library, providing safe and efficient
//! encoding and decoding of DjVu document format files.
//!
//! This library is organized into several modules:
//! - `utils`: Core utilities including error handling, data structures, and threading
//! - `core`: Core DjVu functionality including global settings and file handling
//! - `iff`: IFF (Interchange File Format) reading and writing
//! - `image`: Image processing and format handling
//! - `encode`: Various encoding algorithms (IW44, JB2, ZP)
//! - `doc`: Document structure handling
//! - `annotations`: Text annotations and hidden text support

// Re-export commonly used types at the crate root
pub use utils::error::{DjvuError, Result};

// Core modules
pub mod utils {
    pub mod error;
    pub mod arrays;
    pub mod atomic;
    pub mod geom;
    pub mod log;
    pub mod smartpointer;
    pub mod string;
    pub mod threads;
}

pub mod core {
    #[path = "DJVUFile.rs"]
    pub mod djvu_file;
    #[path = "DJVUGlobal.rs"]
    pub mod djvu_global;
    #[path = "DJVUMessage.rs"]
    pub mod djvu_message;
}

pub mod iff {
    pub mod iff;
    pub mod bzz;
    pub mod chunk_tree;
    pub mod data_pool;
}

pub mod image {
    pub mod image_formats;
    pub mod palette;
    pub mod coefficients;
}

pub mod encode {
    pub mod huffman;
    pub mod iw44;
    
    pub mod jb2 {
        pub mod arithmetic_coder;
        pub mod bitmap_writer;
        pub mod context;
        pub mod encoder;
        pub mod error;
        pub mod num_coder;
        pub mod relative_and_state;
        pub mod types;
        
        // Re-export the types and encoder contents
        pub use self::encoder::*;
        pub use self::types::*;
    }
    
    pub mod zp {
        pub mod table;
        #[path = "ZPcodec.rs"]
        pub mod zp_codec;
        
        // Re-export everything from zp_codec
        pub use zp_codec::*;
    }
}

pub mod doc {
    #[path = "DJVUDir.rs"]
    pub mod djvu_dir;
    #[path = "DJVUDoceditor.rs"]
    pub mod djvu_doceditor;
    #[path = "DJVUDocument.rs"]
    pub mod djvu_document;
    #[path = "DJVUNav.rs"]
    pub mod djvu_nav;
}

pub mod annotations {
    pub mod annotations;
    pub mod hidden_text;
}

// Public API exports
pub use core::djvu_global::*;
pub use core::djvu_message::DjVuMessage;
pub use crate::iff::iff::{Chunk};

// Constants
pub const DJVU_VERSION: &str = "0.1.0";
pub const DJVU_MAGIC: [u8; 4] = [0x41, 0x54, 0x26, 0x54]; // "AT&T"

/// Initialize the DjVu library with default settings
pub fn init() -> Result<()> {
    // TODO: Implement proper initialization
    // core::djvu_global::init_global_settings()?;
    Ok(())
}

/// Initializes the logging system.
pub fn init_logging() -> Result<()> {
    // TODO: Implement proper logging initialization
    // crate::utils::log::init_logging()?;
    Ok(())
}

/// Cleans up the DjVu library.
pub fn cleanup() {
    // TODO: Implement proper cleanup
    // core::djvu_global::cleanup_global_settings();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_init_cleanup() {
        assert!(init().is_ok());
        cleanup();
    }

    #[test]
    fn test_version() {
        assert_eq!(DJVU_VERSION, "0.1.0");
    }

    #[test]
    fn test_magic() {
        assert_eq!(DJVU_MAGIC, [0x41, 0x54, 0x26, 0x54]);
    }
}
