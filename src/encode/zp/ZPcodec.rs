mod table;

use std::io::Write;
use table::DEFAULT_ZP_TABLE;
use thiserror::Error;

pub type BitContext = u8;

#[derive(Error, Debug)]
pub enum ZpCodecError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Encoder already finished")]
    Finished,
}

/// ZP Encoder that matches the C++ implementation exactly
pub struct ZpEncoder<W: Write> {
    writer: Option<W>,
    // ZP state variables (matching C++ ZPCodec)
    a: u32,      // range register (changed to u32 to handle overflow)
    subend: u32, // subrange end (changed to u32 to handle overflow)
    buffer: u32, // 24-bit buffer for zemit
    nrun: u32,   // run length for zemit
    byte: u8,    // current output byte being built
    scount: u8,  // bit count in current byte (0-7)
    delay: u8,   // delay counter
    // State tables
    p: [u16; 256], // probability estimates
    m: [u16; 256], // minimum intervals
    up: [u8; 256], // up transitions
    dn: [u8; 256], // down transitions
    finished: bool,
}

impl<W: Write> ZpEncoder<W> {
    pub fn new(writer: W, _djvu_compat: bool) -> Result<Self, ZpCodecError> {
        // Copy the state tables from DEFAULT_ZP_TABLE
        let mut p = [0u16; 256];
        let mut m = [0u16; 256];
        let mut up = [0u8; 256];
        let mut dn = [0u8; 256];

        for i in 0..256 {
            p[i] = DEFAULT_ZP_TABLE[i].p;
            m[i] = DEFAULT_ZP_TABLE[i].m;
            up[i] = DEFAULT_ZP_TABLE[i].up;
            dn[i] = DEFAULT_ZP_TABLE[i].dn;
        }

        Ok(Self {
            writer: Some(writer),
            // Initialize ZP state (matching C++ zinit_encoder)
            a: 0,
            subend: 0,
            buffer: 0xffffff, // 24-bit buffer initialized to all 1s
            nrun: 0,
            byte: 0,
            scount: 0,
            delay: 25, // initial delay
            p,
            m,
            up,
            dn,
            finished: false,
        })
    }

    /// Main encoding function (matches C++ encoder inline function)
    pub fn encode(&mut self, bit: bool, ctx: &mut BitContext) -> Result<(), ZpCodecError> {
        if self.finished {
            return Err(ZpCodecError::Finished);
        }

        let bit = if bit { 1 } else { 0 };
        let z = self.a as u32 + self.p[*ctx as usize] as u32;

        if bit != (*ctx & 1) as i32 {
            // LPS path
            self.encode_lps(ctx, z)?;
        } else if z >= 0x8000 {
            // MPS path with renormalization
            self.encode_mps(ctx, z)?;
        } else {
            // MPS path without renormalization
            self.a = z as u32;
        }

        Ok(())
    }

    /// Encode for raw/IW contexts (pass-through encoding)
    pub fn encode_raw(&mut self, bit: bool) -> Result<(), ZpCodecError> {
        if self.finished {
            return Err(ZpCodecError::Finished);
        }

        let bit_val = if bit { 1 } else { 0 };
        self.outbit(bit_val)?;
        Ok(())
    }

    /// LPS encoding (matches C++ encode_lps)
    fn encode_lps(&mut self, ctx: &mut BitContext, z: u32) -> Result<(), ZpCodecError> {
        let mut z = z;

        // Avoid interval reversion (ZPCODER variant)
        let d = 0x6000 + ((z + self.a as u32) >> 2);
        if z > d {
            z = d;
        }

        // Adaptation
        *ctx = self.dn[*ctx as usize];

        // Code LPS
        z = 0x10000 - z;
        self.subend = self.subend.wrapping_add(z as u32);
        self.a = self.a.wrapping_add(z as u32);

        // Export bits
        while self.a >= 0x8000 {
            self.zemit(1 - ((self.subend >> 15) & 1) as i32)?;
            self.subend = (self.subend << 1) & 0xFFFF; // Keep in 16-bit range
            self.a = (self.a << 1) & 0xFFFF; // Keep in 16-bit range
        }

        Ok(())
    }

    /// MPS encoding (matches C++ encode_mps)
    fn encode_mps(&mut self, ctx: &mut BitContext, z: u32) -> Result<(), ZpCodecError> {
        let mut z = z;

        // Avoid interval reversion (ZPCODER variant)
        let d = 0x6000 + ((z + self.a as u32) >> 2);
        if z > d {
            z = d;
        }

        // Adaptation
        if self.a >= self.m[*ctx as usize] as u32 {
            *ctx = self.up[*ctx as usize];
        }

        // Code MPS
        self.a = z as u32;

        // Export bits
        if self.a >= 0x8000 {
            self.zemit(1 - ((self.subend >> 15) & 1) as i32)?;
            self.subend = (self.subend << 1) & 0xFFFF; // Keep in 16-bit range
            self.a = (self.a << 1) & 0xFFFF; // Keep in 16-bit range
        }

        Ok(())
    }

    /// Bit emission with run-length encoding (matches C++ zemit)
    fn zemit(&mut self, b: i32) -> Result<(), ZpCodecError> {
        // Shift new bit into 3-byte buffer
        self.buffer = (self.buffer << 1) + b as u32;

        // Examine bit going out of the 3-byte buffer
        let out_bit = (self.buffer >> 24) & 1;
        self.buffer &= 0xffffff;

        match out_bit {
            1 => {
                // Upper renormalization
                self.outbit(1)?;
                while self.nrun > 0 {
                    self.outbit(0)?;
                    self.nrun -= 1;
                }
                self.nrun = 0;
            }
            0xff => {
                // Lower renormalization
                self.outbit(0)?;
                while self.nrun > 0 {
                    self.outbit(1)?;
                    self.nrun -= 1;
                }
                self.nrun = 0;
            }
            0 => {
                // Central renormalization
                self.nrun += 1;
            }
            _ => unreachable!("out_bit can only be 0, 1, or 0xff"),
        }

        Ok(())
    }

    /// Output a single bit (matches C++ outbit)
    fn outbit(&mut self, bit: i32) -> Result<(), ZpCodecError> {
        if self.delay > 0 {
            if self.delay < 0xff {
                // delay=0xff suspends emission forever
                self.delay -= 1;
            }
        } else {
            // Insert a bit
            self.byte = (self.byte << 1) | (bit as u8);
            self.scount += 1;

            // Output a byte when we have 8 bits
            if self.scount == 8 {
                if let Some(ref mut writer) = self.writer {
                    writer.write_all(&[self.byte])?;
                }
                self.scount = 0;
                self.byte = 0;
            }
        }

        Ok(())
    }

    /// Flush the encoder (matches C++ eflush)
    pub fn flush(&mut self) -> Result<(), ZpCodecError> {
        if self.finished {
            return Ok(());
        }

        // Adjust subend
        if self.subend > 0x8000 {
            self.subend = 0x10000;
        } else if self.subend > 0 {
            self.subend = 0x8000;
        }

        // Emit many MPS bits
        while self.buffer != 0xffffff || self.subend != 0 {
            self.zemit(1 - ((self.subend >> 15) & 1) as i32)?;
            self.subend = (self.subend << 1) & 0xFFFF; // Keep in 16-bit range
        }

        // Emit pending run
        self.outbit(1)?;
        while self.nrun > 0 {
            self.outbit(0)?;
            self.nrun -= 1;
        }

        // Emit 1 until full byte
        while self.scount > 0 {
            self.outbit(1)?;
        }

        // Prevent further emission (critical for DjVu compatibility)
        self.delay = 0xff;

        // Flush writer
        if let Some(ref mut writer) = self.writer {
            writer.flush()?;
        }

        Ok(())
    }

    /// Finish encoding and return the writer
    pub fn finish(&mut self) -> Result<W, ZpCodecError> {
        if self.finished {
            return Err(ZpCodecError::Finished);
        }

        self.flush()?;
        self.finished = true;

        self.writer.take().ok_or(ZpCodecError::Finished)
    }

    /// Legacy wrapper for compatibility
    pub fn iw_encoder(&mut self, bit: bool) -> Result<(), ZpCodecError> {
        self.encode_raw(bit)
    }
}

impl<W: Write> Drop for ZpEncoder<W> {
    fn drop(&mut self) {
        if !self.finished && !std::thread::panicking() {
            let _ = self.flush();
        }
    }
}
