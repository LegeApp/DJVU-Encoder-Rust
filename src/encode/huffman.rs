// src/encode/huffman.rs

//! Huffman coding implementation for IW44 compression.
//!
//! This module provides Huffman encoding and decoding functionality
//! used in the IW44 wavelet compression algorithm.

use std::collections::{HashMap, BinaryHeap};
use std::cmp::{Ordering, Reverse};
use std::io::{Read, Write, Result as IoResult};

/// Node in a Huffman tree.
#[derive(Debug, Clone)]
enum HuffmanNode {
    Leaf { symbol: u16, frequency: u32 },
    Internal { left: Box<HuffmanNode>, right: Box<HuffmanNode>, frequency: u32 },
}

impl HuffmanNode {
    fn frequency(&self) -> u32 {
        match self {
            HuffmanNode::Leaf { frequency, .. } => *frequency,
            HuffmanNode::Internal { frequency, .. } => *frequency,
        }
    }
}

impl PartialEq for HuffmanNode {
    fn eq(&self, other: &Self) -> bool {
        self.frequency() == other.frequency()
    }
}

impl Eq for HuffmanNode {}

impl PartialOrd for HuffmanNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HuffmanNode {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse ordering for min-heap behavior
        other.frequency().cmp(&self.frequency())
    }
}

/// A Huffman decoder for reading compressed data.
pub struct HuffmanDecoder {
    root: Option<HuffmanNode>,
    codes: HashMap<u16, (u32, u8)>, // symbol -> (code, bit_length)
}

impl HuffmanDecoder {
    /// Creates a new Huffman decoder.
    pub fn new() -> Self {
        Self {
            root: None,
            codes: HashMap::new(),
        }
    }

    /// Builds a Huffman tree from frequency data.
    pub fn build_from_frequencies(&mut self, frequencies: &[(u16, u32)]) {
        if frequencies.is_empty() {
            return;
        }

        let mut heap = BinaryHeap::new();

        // Create leaf nodes
        for &(symbol, freq) in frequencies {
            heap.push(Reverse(HuffmanNode::Leaf { symbol, frequency: freq }));
        }

        // Build the tree
        while heap.len() > 1 {
            let left = heap.pop().unwrap().0;
            let right = heap.pop().unwrap().0;
            let combined_freq = left.frequency() + right.frequency();
            
            heap.push(Reverse(HuffmanNode::Internal {
                left: Box::new(left),
                right: Box::new(right),
                frequency: combined_freq,
            }));
        }

        self.root = heap.pop().map(|r| r.0);
        self.generate_codes();
    }

    /// Generates the Huffman codes from the tree.
    fn generate_codes(&mut self) {
        self.codes.clear();
        if let Some(ref root) = self.root {
            self.generate_codes_recursive(root, 0, 0);
        }
    }

    /// Recursively generates codes for each symbol.
    fn generate_codes_recursive(&mut self, node: &HuffmanNode, code: u32, depth: u8) {
        match node {
            HuffmanNode::Leaf { symbol, .. } => {
                self.codes.insert(*symbol, (code, depth));
            }
            HuffmanNode::Internal { left, right, .. } => {
                self.generate_codes_recursive(left, code << 1, depth + 1);
                self.generate_codes_recursive(right, (code << 1) | 1, depth + 1);
            }
        }
    }

    /// Decodes a symbol from the bit stream.
    pub fn decode_symbol<R: Read>(&self, reader: &mut crate::encode::bit_stream::BitReader<R>) -> IoResult<Option<u16>> {
        let mut current = match &self.root {
            Some(root) => root,
            None => return Ok(None),
        };

        loop {
            match current {
                HuffmanNode::Leaf { symbol, .. } => return Ok(Some(*symbol)),
                HuffmanNode::Internal { left, right, .. } => {
                    let bit = reader.read_bit()?;
                    current = if bit { right } else { left };
                }
            }
        }
    }

    /// Gets the code for a specific symbol.
    pub fn get_code(&self, symbol: u16) -> Option<(u32, u8)> {
        self.codes.get(&symbol).copied()
    }
}

impl Default for HuffmanDecoder {
    fn default() -> Self {
        Self::new()
    }
}

/// A Huffman encoder for writing compressed data.
pub struct HuffmanEncoder {
    decoder: HuffmanDecoder,
}

impl HuffmanEncoder {
    /// Creates a new Huffman encoder.
    pub fn new() -> Self {
        Self {
            decoder: HuffmanDecoder::new(),
        }
    }

    /// Builds a Huffman tree from frequency data.
    pub fn build_from_frequencies(&mut self, frequencies: &[(u16, u32)]) {
        self.decoder.build_from_frequencies(frequencies);
    }

    /// Encodes a symbol to the bit stream.
    pub fn encode_symbol<W: Write>(&self, symbol: u16, writer: &mut crate::encode::bit_stream::BitWriter<W>) -> IoResult<()> {
        if let Some((code, bit_length)) = self.decoder.get_code(symbol) {
            writer.write_bits(code, bit_length)?;
        }
        Ok(())
    }

    /// Gets the code for a specific symbol.
    pub fn get_code(&self, symbol: u16) -> Option<(u32, u8)> {
        self.decoder.get_code(symbol)
    }

    /// Returns the underlying decoder for reading operations.
    pub fn decoder(&self) -> &HuffmanDecoder {
        &self.decoder
    }
}

impl Default for HuffmanEncoder {
    fn default() -> Self {
        Self::new()
    }
}

/// Predefined Huffman tables for IW44 compression.
pub mod tables {
    /// Default frequency table for IW44 coefficients.
    pub const IW44_FREQUENCIES: &[(u16, u32)] = &[
        (0, 1000), (1, 500), (2, 300), (3, 200), (4, 150),
        (5, 120), (6, 100), (7, 80), (8, 70), (9, 60),
        (10, 50), (11, 40), (12, 35), (13, 30), (14, 25),
        (15, 20), (16, 18), (17, 16), (18, 14), (19, 12),
        (20, 10), (21, 9), (22, 8), (23, 7), (24, 6),
        (25, 5), (26, 4), (27, 3), (28, 2), (29, 1),
    ];
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode::bit_stream::{BitWriter, BitReader};
    use std::io::Cursor;

    #[test]
    fn test_huffman_basic() {
        let frequencies = vec![(65, 5), (66, 9), (67, 12), (68, 13), (69, 16), (70, 45)];
        
        let mut encoder = HuffmanEncoder::new();
        encoder.build_from_frequencies(&frequencies);
        
        // Test that we can get codes for all symbols
        for &(symbol, _) in &frequencies {
            assert!(encoder.get_code(symbol).is_some());
        }
    }

    #[test]
    fn test_huffman_roundtrip() {
        let frequencies = vec![(1, 10), (2, 20), (3, 30)];
        
        let mut encoder = HuffmanEncoder::new();
        encoder.build_from_frequencies(&frequencies);
        
        // Encode some data
        let mut buffer = Vec::new();
        {
            let mut bit_writer = BitWriter::new(&mut buffer);
            encoder.encode_symbol(1, &mut bit_writer).unwrap();
            encoder.encode_symbol(2, &mut bit_writer).unwrap();
            encoder.encode_symbol(3, &mut bit_writer).unwrap();
            bit_writer.flush().unwrap();
        }
        
        // Decode the data
        let mut decoder = HuffmanDecoder::new();
        decoder.build_from_frequencies(&frequencies);
        
        let cursor = Cursor::new(buffer);
        let mut bit_reader = BitReader::new(cursor);
        
        assert_eq!(decoder.decode_symbol(&mut bit_reader).unwrap(), Some(1));
        assert_eq!(decoder.decode_symbol(&mut bit_reader).unwrap(), Some(2));
        assert_eq!(decoder.decode_symbol(&mut bit_reader).unwrap(), Some(3));
    }
}
