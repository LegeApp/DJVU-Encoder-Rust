// src/bzz.rs

//! A module for BZZ (bzip2) compression and decompression.
//!
//! This module replaces the C++ `BSByteStream`, `BSEncodeByteStream`, and their
//! complex internal sorting and coding logic. It acts as a simple wrapper around
//! the `bzip2` crate, which implements the same underlying Burrows-Wheeler
//! Transform algorithm.
//!
//! This provides a robust, performant, and well-tested compression solution
//! without needing to reimplement the algorithm from scratch.

use crate::utils::error::{DjvuError, Result};
use bzip2::read::BzDecoder;
use bzip2::write::BzEncoder;
use bzip2::Compression;
use std::io::{Read, Write};

/// Compresses a byte slice using the BZZ (bzip2) algorithm.
///
/// This function is the replacement for creating a `BSByteStream` in encoding mode.
///
/// # Arguments
/// * `data` - The raw byte slice to compress.
/// * `level` - The compression level, from 1 (fastest) to 9 (best compression).
///   A level of 6 is a good default balance.
///
/// # Returns
/// A `Result` containing the compressed data as a `Vec<u8>`.
#[inline]
pub fn bzz_compress(data: &[u8], level: u32) -> Result<Vec<u8>> {
    // Ensure the compression level is valid for the bzip2 crate (1-9).
    let compression_level = match level {
        1..=9 => Compression::new(level),
        _ => Compression::default(), // Defaults to 6
    };

    let mut encoder = BzEncoder::new(Vec::new(), compression_level);
    encoder.write_all(data)?;
    let compressed_data = encoder.finish()?;
    Ok(compressed_data)
}

/// Decompresses a byte slice that was compressed with the BZZ (bzip2) algorithm.
///
/// This function is the replacement for creating a `BSByteStream` in decoding mode.
/// It is included for completeness but is not strictly necessary for an encoder-only library.
///
/// # Arguments
/// * `compressed_data` - The compressed byte slice.
///
/// # Returns
/// A `Result` containing the decompressed data as a `Vec<u8>`.
#[inline]
pub fn bzz_decompress(compressed_data: &[u8]) -> Result<Vec<u8>> {
    let mut decoder = BzDecoder::new(compressed_data);
    let mut decompressed_data = Vec::new();
    decoder.read_to_end(&mut decompressed_data)?;
    Ok(decompressed_data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compression_decompress_roundtrip() {
        let original_data = b"Hello, this is a test of the bzz compression system. It should handle repeated patterns very well. hello hello hello.";
        let compression_level = 6;

        // Compress the data
        let compressed = bzz_compress(original_data, compression_level).unwrap();

        // The compressed data should be smaller than the original.
        assert!(compressed.len() < original_data.len());
        println!(
            "Original size: {}, Compressed size: {}",
            original_data.len(),
            compressed.len()
        );

        // Decompress the data
        let decompressed = bzz_decompress(&compressed).unwrap();

        // The result should match the original data.
        assert_eq!(original_data, decompressed.as_slice());
    }
    
    #[test]
    fn test_compress_empty_data() {
        let original_data = b"";
        let compressed = bzz_compress(original_data, 6).unwrap();
        let decompressed = bzz_decompress(&compressed).unwrap();
        assert_eq!(decompressed, original_data);
        // bzip2 has a small header/footer, so empty input is not zero bytes.
        assert!(!compressed.is_empty());
    }

    #[test]
    fn test_highly_compressible_data() {
        let original_data = vec![b'a'; 10_000];
        let compressed = bzz_compress(&original_data, 9).unwrap();
        
        // Should compress extremely well
        assert!(compressed.len() < 100); 
        println!(
            "Original size: {}, Compressed size: {}",
            original_data.len(),
            compressed.len()
        );
        
        let decompressed = bzz_decompress(&compressed).unwrap();
        assert_eq!(original_data, decompressed);
    }
}