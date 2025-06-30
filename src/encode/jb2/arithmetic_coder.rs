// src/encode/jb2/arithmetic_coder.rs

//! Arithmetic coder for JB2 compression.
//!
//! This module provides arithmetic coding functionality used in JB2 bilevel
//! image compression.

use std::io::{Write, Result as IoResult};
use crate::encode::jb2::error::Jb2Error;

/// An arithmetic encoder for JB2 compression.
pub struct ArithmeticEncoder<W: Write> {
    writer: W,
    low: u32,
    high: u32,
    pending_bits: u32,
}

impl<W: Write> ArithmeticEncoder<W> {
    /// Creates a new arithmetic encoder.
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            low: 0,
            high: 0xFFFFFFFF,
            pending_bits: 0,
        }
    }

    /// Encodes a bit with a given probability.
    pub fn encode_bit(&mut self, bit: bool, probability: u32) -> Result<(), Jb2Error> {
        let range = self.high - self.low + 1;
        let threshold = self.low + (range * probability / 0x10000);

        if bit {
            self.high = threshold - 1;
        } else {
            self.low = threshold;
        }

        // Renormalization
        while self.high < 0x80000000 || self.low >= 0x80000000 {
            if self.high < 0x80000000 {
                self.output_bit(false)?;
                self.output_pending_bits(true)?;
            } else {
                self.output_bit(true)?;
                self.output_pending_bits(false)?;
            }

            self.low = (self.low << 1) & 0xFFFFFFFF;
            self.high = ((self.high << 1) | 1) & 0xFFFFFFFF;
        }

        // Handle pending bits
        while self.low >= 0x40000000 && self.high < 0xC0000000 {
            self.pending_bits += 1;
            self.low = ((self.low - 0x40000000) << 1) & 0xFFFFFFFF;
            self.high = (((self.high - 0x40000000) << 1) | 1) & 0xFFFFFFFF;
        }

        Ok(())
    }

    /// Encodes a symbol using a frequency table.
    pub fn encode_symbol(&mut self, symbol: u32, cumulative_freq: &[u32]) -> Result<(), Jb2Error> {
        let total_freq = cumulative_freq[cumulative_freq.len() - 1];
        let range = self.high - self.low + 1;
        
        let low_bound = if symbol == 0 { 0 } else { cumulative_freq[symbol as usize - 1] };
        let high_bound = cumulative_freq[symbol as usize];

        self.high = self.low + (range * high_bound / total_freq) - 1;
        self.low = self.low + (range * low_bound / total_freq);

        // Renormalization (same as in encode_bit)
        while self.high < 0x80000000 || self.low >= 0x80000000 {
            if self.high < 0x80000000 {
                self.output_bit(false)?;
                self.output_pending_bits(true)?;
            } else {
                self.output_bit(true)?;
                self.output_pending_bits(false)?;
            }

            self.low = (self.low << 1) & 0xFFFFFFFF;
            self.high = ((self.high << 1) | 1) & 0xFFFFFFFF;
        }

        while self.low >= 0x40000000 && self.high < 0xC0000000 {
            self.pending_bits += 1;
            self.low = ((self.low - 0x40000000) << 1) & 0xFFFFFFFF;
            self.high = (((self.high - 0x40000000) << 1) | 1) & 0xFFFFFFFF;
        }

        Ok(())
    }

    /// Finishes encoding and flushes any remaining bits.
    pub fn finish(&mut self) -> Result<(), Jb2Error> {
        self.pending_bits += 1;
        if self.low < 0x40000000 {
            self.output_bit(false)?;
            self.output_pending_bits(true)?;
        } else {
            self.output_bit(true)?;
            self.output_pending_bits(false)?;
        }
        self.writer.flush().map_err(|e| Jb2Error::Io(e))?;
        Ok(())
    }

    /// Outputs a single bit to the writer.
    fn output_bit(&mut self, bit: bool) -> Result<(), Jb2Error> {
        // This is a simplified implementation - in practice, you'd accumulate
        // bits into bytes before writing
        let byte = if bit { 1u8 } else { 0u8 };
        self.writer.write_all(&[byte]).map_err(|e| Jb2Error::Io(e))?;
        Ok(())
    }

    /// Outputs all pending bits.
    fn output_pending_bits(&mut self, bit: bool) -> Result<(), Jb2Error> {
        for _ in 0..self.pending_bits {
            self.output_bit(bit)?;
        }
        self.pending_bits = 0;
        Ok(())
    }

    /// Gets the underlying writer.
    pub fn into_inner(self) -> W {
        self.writer
    }
}

/// An arithmetic decoder for JB2 decompression.
pub struct ArithmeticDecoder<R> {
    reader: R,
    low: u32,
    high: u32,
    code: u32,
}

// Note: Decoder implementation would be similar but for reading
// For now, we'll provide a basic structure as it's not immediately needed
// for the encoder port.
