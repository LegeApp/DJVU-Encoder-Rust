// src/zp_codec/mod.rs

mod table;

use std::io::{Read, Write};
use thiserror::Error;
use table::DEFAULT_ZP_TABLE;

/// The context for a single binary prediction. It holds the state of the
/// adaptive probability model.
pub type BitContext = u8;

// Type alias for backward compatibility
pub type ZPCodec<T> = ZpEncoder<T>;

#[derive(Error, Debug)]
pub enum ZpCodecError {
    #[error("I/O error during codec operation")]
    Io(#[from] std::io::Error),
    #[error("Unexpected end of file while decoding")]
    EndOfFile,
}

/// A lookup table for fast "find first zero" bit scan.
const FFZ_TABLE: [u8; 256] = {
    let mut table = [0; 256];
    let mut i = 0;
    while i < 256 {
        let mut count = 0;
        let mut j = i;
        while (j & 0x80) != 0 {
            count += 1;
            j <<= 1;
        }
        table[i] = count;
        i += 1;
    }
    table
};

/// Holds the probability model tables for the ZP-Coder.
/// These tables are derived from the static `DEFAULT_ZP_TABLE`.
struct ZpTables {
    /// LPS probability for each state.
    p: [u16; 256],
    /// MPS adaptation threshold for each state.
    m: [u16; 256],
    /// Next state after an MPS is coded.
    up: [BitContext; 256],
    /// Next state after an LPS is coded.
    dn: [BitContext; 256],
}

impl ZpTables {
    fn new(djvu_compat: bool) -> Self {
        let mut p = [0; 256];
        let mut m = [0; 256];
        let mut up = [0; 256];
        let mut dn = [0; 256];

        for i in 0..256 {
            p[i] = DEFAULT_ZP_TABLE[i].p;
            m[i] = DEFAULT_ZP_TABLE[i].m;
            up[i] = DEFAULT_ZP_TABLE[i].up;
            dn[i] = DEFAULT_ZP_TABLE[i].dn;
        }

        // The C++ code has a patch for non-DjVu-compatible streams to improve
        // compression slightly. We replicate it here.
        if !djvu_compat {
            for j in 0..256 {
                let a = 0x10000u32 - p[j] as u32;
                let a_norm = if a >= 0x8000 { a << 1 } else { a };

                if m[j] > 0 && a + p[j] as u32 >= 0x8000 && a_norm >= m[j] as u32 {
                    let x = DEFAULT_ZP_TABLE[j].dn;
                    let y = DEFAULT_ZP_TABLE[x as usize].dn;
                    dn[j] = y;
                }
            }
        }

        Self { p, m, up, dn }
    }
}

/// A ZP-Coder for encoding a stream of binary decisions.
pub struct ZpEncoder<W: Write> {
    writer: W,
    tables: ZpTables,
    /// The base of the probability interval, scaled to 16 bits.
    a: u32,
    /// Carries for interval arithmetic.
    subend: u32,
    /// Buffer for outgoing bytes.
    byte_buf: u8,
    /// Number of bits in `byte_buf`.
    bit_count: u8,
    /// Buffer for carry propagation logic (3 bytes).
    zemit_buf: u32,
    /// Count of pending carry bits.
    nrun: u32,
    /// If true, the encoder has been finished and should not be used.
    finished: bool,
}

impl<W: Write> ZpEncoder<W> {
    /// Creates a new ZP-Coder for encoding.
    ///
    /// # Arguments
    /// * `writer`: The output stream to write encoded bytes to.
    /// * `djvu_compat`: If true, uses the exact probability tables from the DjVu reference library.
    pub fn new(writer: W, djvu_compat: bool) -> Self {
        Self {
            writer,
            tables: ZpTables::new(djvu_compat),
            a: 0,
            subend: 0,
            byte_buf: 0,
            bit_count: 0,
            zemit_buf: 0xFFFFFF,
            nrun: 0,
            finished: false,
        }
    }

    /// Encodes a single bit using an adaptive context.
    ///
    /// # Arguments
    /// * `bit`: The boolean value to encode.
    /// * `ctx`: A mutable reference to the `BitContext` for this decision.
    pub fn encode(&mut self, bit: bool, ctx: &mut BitContext) -> Result<(), ZpCodecError> {
        let z = self.a + self.tables.p[*ctx as usize] as u32;
        let mps_val = (*ctx & 1) != 0;

        if bit != mps_val {
            self.encode_lps(ctx, z)
        } else if z >= 0x8000 {
            self.encode_mps(ctx, z)
        } else {
            self.a = z;
            Ok(())
        }
    }

    /// Encodes a bit using the special non-adaptive IW44 rules.
    pub fn iw_encoder(&mut self, bit: bool) -> Result<(), ZpCodecError> {
        let z = 0x8000 + ((self.a + self.a + self.a) >> 3);
        if bit {
            self.encode_lps_simple(z)
        } else {
            self.encode_mps_simple(z)
        }
    }

    /// Flushes all internal buffers, writes final bytes, and returns the underlying writer.
    /// This method MUST be called to ensure all data is written to the stream.
    pub fn finish(mut self) -> Result<W, ZpCodecError> {
        self.flush()?;
        self.finished = true;
        Ok(self.writer)
    }

    fn encode_mps(&mut self, ctx: &mut BitContext, z: u32) -> Result<(), ZpCodecError> {
        // ZP-Coder specific rule to avoid interval reversion.
        let d = 0x6000 + ((z + self.a) >> 2);
        let z_clipped = if z > d { d } else { z };

        // Adaptation
        if self.a >= self.tables.m[*ctx as usize] as u32 {
            *ctx = self.tables.up[*ctx as usize];
        }

        // Code MPS
        self.a = z_clipped;

        // Renormalization
        if self.a >= 0x8000 {
            self.zemit( (self.subend >> 15) == 0 )?;
            self.subend <<= 1;
            self.a <<= 1;
        }
        Ok(())
    }

    fn encode_lps(&mut self, ctx: &mut BitContext, z: u32) -> Result<(), ZpCodecError> {
        // ZP-Coder specific rule
        let d = 0x6000 + ((z + self.a) >> 2);
        let z_clipped = if z > d { d } else { z };

        // Adaptation
        *ctx = self.tables.dn[*ctx as usize];

        // Code LPS
        let z_inv = 0x10000 - z_clipped;
        self.subend += z_inv;
        self.a += z_inv;

        // Renormalization
        while self.a >= 0x8000 {
            self.zemit( (self.subend >> 15) == 0 )?;
            self.subend <<= 1;
            self.a <<= 1;
        }
        Ok(())
    }

    fn encode_mps_simple(&mut self, z: u32) -> Result<(), ZpCodecError> {
        self.a = z;
        if self.a >= 0x8000 {
            self.zemit( (self.subend >> 15) == 0 )?;
            self.subend <<= 1;
            self.a <<= 1;
        }
        Ok(())
    }

    fn encode_lps_simple(&mut self, z: u32) -> Result<(), ZpCodecError> {
        let z_inv = 0x10000 - z;
        self.subend += z_inv;
        self.a += z_inv;
        while self.a >= 0x8000 {
            self.zemit( (self.subend >> 15) == 0 )?;
            self.subend <<= 1;
            self.a <<= 1;
        }
        Ok(())
    }

    /// Emits a bit into the carry-propagation buffer.
    fn zemit(&mut self, bit: bool) -> Result<(), ZpCodecError> {
        self.zemit_buf = (self.zemit_buf << 1) | (bit as u32);
        let outgoing_bit = (self.zemit_buf >> 24) & 1;
        self.zemit_buf &= 0xFFFFFF;

        match outgoing_bit {
            1 => { // Upper renormalization
                self.outbit(true)?;
                for _ in 0..self.nrun {
                    self.outbit(false)?;
                }
                self.nrun = 0;
            }
            0 => { // Central renormalization
                self.nrun += 1;
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    /// Writes a bit to the byte buffer and flushes the buffer when full.
    fn outbit(&mut self, bit: bool) -> Result<(), ZpCodecError> {
        self.byte_buf = (self.byte_buf << 1) | (bit as u8);
        self.bit_count += 1;
        if self.bit_count == 8 {
            self.writer.write_all(&[self.byte_buf])?;
            self.bit_count = 0;
            self.byte_buf = 0;
        }
        Ok(())
    }
    
    /// Flushes the encoder at the end of the stream. Replicates `eflush`.
    fn flush(&mut self) -> Result<(), ZpCodecError> {
        // Adjust subend
        self.subend = if self.subend > 0x8000 { 0x10000 } else if self.subend > 0 { 0x8000 } else { 0 };

        // Emit final MPS bits
        while self.zemit_buf != 0xFFFFFF || self.subend != 0 {
            self.zemit((self.subend >> 15) == 0)?;
            self.subend <<= 1;
        }

        // Emit pending run
        self.outbit(true)?;
        for _ in 0..self.nrun {
            self.outbit(false)?;
        }
        self.nrun = 0;

        // Pad with 1s to fill the last byte
        while self.bit_count > 0 {
            self.outbit(true)?;
        }
        Ok(())
    }
}

impl<W: Write> Drop for ZpEncoder<W> {
    fn drop(&mut self) {
        if !self.finished && !std::thread::panicking() {
            panic!("ZpEncoder dropped without calling finish(). Potential data loss.");
        }
    }
}

// NOTE: The ZpDecoder is not required for the encoder port, but is included here
// for completeness and to show the full structure. It can be removed if not needed.

/// A ZP-Coder for decoding a stream of binary decisions.
pub struct ZpDecoder<R: Read> {
    reader: R,
    tables: ZpTables,
    /// The base of the probability interval.
    a: u32,
    /// The current value from the code stream.
    code: u32,
    /// The boundary for fast MPS path.
    fence: u32,
    /// Buffer for incoming bytes.
    buffer: u32,
    /// Number of bits in `buffer`.
    scount: i32,
    /// Graceful end-of-file counter.
    delay: i32,
}

impl<R: Read> ZpDecoder<R> {
     /// Creates a new ZP-Coder for decoding.
    pub fn new(mut reader: R, djvu_compat: bool) -> Result<Self, ZpCodecError> {
        let mut byte_buf = [0u8; 2];
        reader.read_exact(&mut byte_buf)?;
        let code = ((byte_buf[0] as u32) << 8) | (byte_buf[1] as u32);
        
        let mut decoder = Self {
            reader,
            tables: ZpTables::new(djvu_compat),
            a: 0,
            code,
            fence: if code >= 0x8000 { 0x7FFF } else { code },
            buffer: 0,
            scount: 0,
            delay: 25,
        };
        decoder.preload()?;
        
        Ok(decoder)
    }

    /// Decodes a single bit using an adaptive context.
    pub fn decode(&mut self, ctx: &mut BitContext) -> Result<bool, ZpCodecError> {
        let z = self.a + self.tables.p[*ctx as usize] as u32;
        if z <= self.fence {
            self.a = z;
            Ok((*ctx & 1) != 0)
        } else {
            self.decode_sub(ctx, z)
        }
    }

    /// Decodes a bit using the special non-adaptive IW44 rules.
    pub fn iw_decoder(&mut self) -> Result<bool, ZpCodecError> {
        self.decode_sub_simple(false, 0x8000 + ((self.a + self.a + self.a) >> 3))
    }

    fn decode_sub(&mut self, ctx: &mut BitContext, mut z: u32) -> Result<bool, ZpCodecError> {
        let bit = (*ctx & 1) != 0;

        // ZP-Coder interval reversion avoidance
        let d = 0x6000 + ((z + self.a) >> 2);
        if z > d { z = d; }

        let is_lps = z > self.code;
        if is_lps {
            let z_inv = 0x10000 - z;
            self.a += z_inv;
            self.code += z_inv;
            *ctx = self.tables.dn[*ctx as usize]; // Adapt
            self.renorm_lps()?;
            Ok(!bit)
        } else {
            // MPS adaptation
            if self.a >= self.tables.m[*ctx as usize] as u32 {
                *ctx = self.tables.up[*ctx as usize];
            }
            self.renorm_mps(z)?;
            Ok(bit)
        }
    }
    
    fn decode_sub_simple(&mut self, mps_val: bool, _z: u32) -> Result<bool, ZpCodecError> {
        // This is a simplified version for IWdecoder, more logic may be needed for general case
        unimplemented!("decode_sub_simple is not fully ported, only iw_decoder's path is sketched out")
    }

    /// Renormalizes the interval after an MPS.
    fn renorm_mps(&mut self, z: u32) -> Result<(), ZpCodecError> {
        self.scount -= 1;
        self.a = z << 1;
        self.code = (self.code << 1) | ((self.buffer >> self.scount) & 1);
        self.update_fence_and_preload()
    }

    /// Renormalizes the interval after an LPS.
    fn renorm_lps(&mut self) -> Result<(), ZpCodecError> {
        let shift = Self::ffz(self.a);
        self.scount -= shift as i32;
        self.a <<= shift;
        self.code = (self.code << shift) | ((self.buffer >> self.scount) & ((1 << shift) - 1));
        self.update_fence_and_preload()
    }
    
    /// Updates the fence and preloads the buffer if needed.
    fn update_fence_and_preload(&mut self) -> Result<(), ZpCodecError> {
        self.fence = if self.code >= 0x8000 { 0x7FFF } else { self.code };
        if self.scount < 16 {
            self.preload()?;
        }
        Ok(())
    }

    fn preload(&mut self) -> Result<(), ZpCodecError> {
        while self.scount <= 24 {
            let mut byte_buf = [0u8; 1];
            match self.reader.read(&mut byte_buf) {
                Ok(0) => { // End of stream
                    self.delay -=1;
                    if self.delay < 1 { return Err(ZpCodecError::EndOfFile); }
                    byte_buf[0] = 0xFF; // Pad with 0xFF at EOF
                }
                Ok(1) => {},
                Err(e) => return Err(e.into()),
                _ => unreachable!(),
            }
            self.buffer = (self.buffer << 8) | (byte_buf[0] as u32);
            self.scount += 8;
        }
        Ok(())
    }
    
    /// Fast "Find First Zero" using a lookup table.
    fn ffz(x: u32) -> u32 {
        if x >= 0xFF00 {
            FFZ_TABLE[(x & 0xFF) as usize] as u32 + 8
        } else {
            FFZ_TABLE[((x >> 8) & 0xFF) as usize] as u32
        }
    }
}