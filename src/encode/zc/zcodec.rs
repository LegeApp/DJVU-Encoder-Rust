use super::table::{DEFAULT_ZP_TABLE, ZpTableEntry};
use thiserror::Error;

/// A single byte representing the statistical context for encoding a bit.
pub type BitContext = u8;

/// Errors that can occur during Z-Coder encoding.
#[derive(Error, Debug)]
pub enum ZCodecError {
    #[error("I/O error during write operation")]
    Io(#[from] std::io::Error),
    #[error("Attempted to encode after the stream was finished")]
    Finished,
}

/// An adaptive quasi-arithmetic encoder implementing the public-domain Z-Coder algorithm.
///
/// This encoder compresses a stream of binary decisions (`true`/`false`) into a byte stream.
/// It uses a set of statistical contexts (`BitContext`) to adapt to the probabilities of
/// the input data, achieving high compression ratios for predictable data.
///
/// The algorithm is a direct implementation of the one described in U.S. Patent 5,059,976,
/// which is now in the public domain. It matches the `ZCODER` logic from the DjVuLibre
/// open-source library.
///
/// # Usage
///
/// ```
/// use std::io::Cursor;
/// // Assuming ZEncoder and BitContext are in the current scope
/// let mut encoder = ZEncoder::new(Cursor::new(Vec::new()), true).unwrap();
/// let mut context = 0 as BitContext;
///
/// // Encode a sequence of bits
/// encoder.encode(false, &mut context).unwrap();
/// encoder.encode(true, &mut context).unwrap();
///
/// // Finalize the stream and retrieve the compressed data
/// let compressed_data = encoder.finish().unwrap().into_inner();
/// ```
use std::io::Write;

pub struct ZEncoder<W: Write> {
    writer: Option<W>,
    // Z-Coder state variables, matching the C++ implementation.
    a: u16,      // range register (16-bit as per original)
    subend: u16, // subrange end (16-bit as per original)
    buffer: u32, // 24-bit buffer for zemit (needs to be u32)
    nrun: u16,   // run length for zemit (16-bit should be enough)
    byte: u8,
    scount: u8,
    finished: bool,
    table: &'static [ZpTableEntry; 256],
}

impl<W: Write> ZEncoder<W> {
    /// Creates a new Z-Coder encoder that writes to the given writer.
    pub fn new(writer: W, _djvu_compat: bool) -> Result<Self, ZCodecError> {
        Ok(ZEncoder {
            writer: Some(writer),
            // Initialize state as per the C++ ZPCodec::Encode::init() function
            a: 0x8000, // Initialize range register to full range
            subend: 0,
            buffer: 0xffffff,
            nrun: 0,
            byte: 0,
            scount: 0,
            finished: false,
            table: &DEFAULT_ZP_TABLE,
        })
    }

    /// Encodes a single bit using the provided statistical context.
    ///
    /// # Arguments
    /// * `bit` - The boolean value to encode (`true` for 1, `false` for 0).
    /// * `ctx` - A mutable reference to the `BitContext` for this decision. The context
    ///   will be updated by this call to adapt to the data.
    #[inline(always)]
    pub fn encode(&mut self, bit: bool, ctx: &mut BitContext) -> Result<(), ZCodecError> {
        if self.finished {
            return Err(ZCodecError::Finished);
        }

        // Calculate the probability interval for the LPS (use u32 for intermediate calculation)
        let mut z = self.a as u32 + self.table[*ctx as usize].p as u32;

        // Apply the core Z-Coder (patented) interval reversion rule.
        if z >= 0x8000 {
            z = 0x4000 + (z >> 1);
        }

        if bit != (*ctx & 1 != 0) {
            // LPS (Less Probable Symbol) path
            self.encode_lps(ctx, z as u16)?;
        } else {
            // MPS (More Probable Symbol) path
            self.encode_mps(ctx, z as u16)?;
        }
        Ok(())
    }

    /// Internal function to handle the MPS encoding logic.
    #[inline(always)]
    fn encode_mps(&mut self, ctx: &mut u8, z: u16) -> Result<(), ZCodecError> {
        // MPS adaptation: move to 'up' state if the interval 'a' is large enough.
        if self.a >= self.table[*ctx as usize].m {
            *ctx = self.table[*ctx as usize].up;
        }

        // Renormalization: code the MPS by setting the interval 'a'.
        self.a = z;

        // Export bits if the interval 'a' exceeds the halfway point.
        if self.a >= 0x8000 {
            self.zemit(1 - (self.subend >> 15) as u32)?;
            self.subend = (self.subend << 1) & 0xffff;
            self.a = (self.a << 1) & 0xffff;
        }
        Ok(())
    }

    /// Internal function to handle the LPS encoding logic.
    #[inline(always)]
    fn encode_lps(&mut self, ctx: &mut u8, z: u16) -> Result<(), ZCodecError> {
        // LPS adaptation: always move to the 'down' state.
        *ctx = self.table[*ctx as usize].dn;

        // Code the LPS by adjusting both 'a' and the carry tracker 'subend'.
        // Use u32 for intermediate calculation to avoid overflow
        let z_adjusted = 0x10000u32 - z as u32;
        self.subend = (self.subend as u32 + z_adjusted) as u16;
        self.a = (self.a as u32 + z_adjusted) as u16;

        // Export bits while the interval 'a' exceeds the halfway point.
        while self.a >= 0x8000 {
            self.zemit(1 - (self.subend >> 15) as u32)?;
            self.subend = (self.subend << 1) & 0xffff;
            self.a = (self.a << 1) & 0xffff;
        }
        Ok(())
    }

    /// Emits a bit to the output buffer, handling carry propagation.
    /// This is the "bit stuffing" core of the algorithm.
    #[inline(always)]
    fn zemit(&mut self, b: u32) -> Result<(), ZCodecError> {
        self.buffer = (self.buffer << 1) | b;
        let out_byte = (self.buffer >> 24) as u8;
        self.buffer &= 0xffffff;

        match out_byte {
            1 => { // WN&C upper renormalization
                self.outbit(1)?;
                while self.nrun > 0 {
                    self.outbit(0)?;
                    self.nrun -= 1;
                }
            }
            0xff => { // WN&C lower renormalization
                self.outbit(0)?;
                while self.nrun > 0 {
                    self.outbit(1)?;
                    self.nrun -= 1;
                }
            }
            0 => { // WN&C central renormalization
                self.nrun += 1;
            }
            _ => unreachable!("zemit logic guarantees out_byte is 0, 1, or 0xff"),
        }
        Ok(())
    }

    /// Outputs a single bit to the underlying writer.
    #[inline(always)]
    fn outbit(&mut self, bit: u8) -> Result<(), ZCodecError> {
        if let Some(ref mut writer) = self.writer {
            self.byte = (self.byte << 1) | bit;
            self.scount += 1;

            if self.scount == 8 {
                writer.write_all(&[self.byte])?;
                self.scount = 0;
                self.byte = 0;
            }
        }
        Ok(())
    }

    /// Flushes all internal buffers, writes pending bits, and terminates the stream correctly.
    /// This must be called to produce a valid compressed stream.
    fn eflush(&mut self) -> Result<(), ZCodecError> {
        // Adjust subend for final carry propagation
        if self.subend > 0x8000 {
            self.subend = 0; // 0x10000 & 0xFFFF = 0 (wrap-around for u16)
        } else if self.subend > 0 {
            self.subend = 0x8000;
        }

        // Emit final MPS bits to clear the buffer
        while self.buffer != 0xffffff || self.subend != 0 {
            self.zemit(1 - (self.subend >> 15) as u32)?;
            self.subend = (self.subend << 1) & 0xffff;
        }

        // Emit the final pending run of bits
        self.outbit(1)?;
        while self.nrun > 0 {
            self.outbit(0)?;
            self.nrun -= 1;
        }

        // Pad the last byte with 0s to align to a full byte
        while self.scount > 0 {
            self.outbit(0)?;
        }

        Ok(())
    }

    /// Finalizes the encoding process, flushes all buffers, and returns the underlying writer.
    /// This is the recommended way to complete encoding.
    pub fn finish(mut self) -> Result<W, ZCodecError> {
        if !self.finished {
            self.eflush()?;
            self.finished = true;
        }
        self.writer.take().ok_or(ZCodecError::Finished)
    }
}

impl<W: Write> Drop for ZEncoder<W> {
    /// Ensures the encoder is flushed when it goes out of scope.
    ///
    /// Note: This will panic if flushing fails, as `drop` cannot return a `Result`.
    /// For recoverable error handling, call the `finish()` method explicitly.
    fn drop(&mut self) {
        if !self.finished {
            if let Err(e) = self.eflush() {
                // In a library, it's often better to avoid printing or panicking.
                // The explicit `finish()` is the proper way to handle errors.
                // For debugging, a panic can be useful.
                panic!("ZEncoder failed to flush on drop: {}", e);
            }
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_encode_simple_sequence() {
        let mut encoder = ZEncoder::new(Cursor::new(Vec::new()), true).unwrap();
        let mut ctx = 0;

        // Encode an alternating sequence
        for i in 0..100 {
            encoder.encode(i % 2 == 0, &mut ctx).unwrap();
        }

        let writer = encoder.finish().unwrap();
        let data = writer.into_inner();

        // We expect some data to be produced and reasonable compression
        assert!(!data.is_empty());
        assert!(data.len() > 0 && data.len() < 50); // Reasonable compression for 100 bits
        
        // Check that the output is deterministic - the specific pattern should always produce the same result
        let expected = [255, 255, 255, 199, 64, 175, 57, 156, 76, 226, 103, 19, 56, 153, 196, 206, 38];
        assert_eq!(data, expected, "Z-Coder output should be deterministic for alternating sequence");
    }
    
    #[test]
    fn test_encode_highly_probable_sequence() {
        let mut encoder = ZEncoder::new(Cursor::new(Vec::new()), true).unwrap();
        let mut ctx = 0;

        // Encode a sequence with a strong bias
        for _ in 0..1000 {
            encoder.encode(false, &mut ctx).unwrap();
        }
        // Throw in one LPS
        encoder.encode(true, &mut ctx).unwrap();

        let data = encoder.finish().unwrap().into_inner();
        
        // This should be very highly compressed. 1001 bits should take far less than 126 bytes.
        assert!(data.len() < 20);
    }
}
