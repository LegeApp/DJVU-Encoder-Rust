// src/encode/zp/model.rs
//! ZP context model that uses the shared arithmetic encoder.
//! 
//! This module contains the Zero Prediction context modeling logic
//! separated from the arithmetic coding engine.

use crate::encode::arith::ArithmeticEncoder;
use crate::encode::zp::table::DEFAULT_ZP_TABLE;
use std::io::Write;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ZpModelError {
    #[error("Arithmetic encoder error: {0}")]
    ArithmeticEncoder(#[from] anyhow::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// The context for a single binary prediction. It holds the state of the
/// adaptive probability model.
pub type BitContext = u8;

/// ZP probability model that delegates arithmetic coding to the shared encoder.
pub struct ZpModel {
    /// The probability model tables for the ZP-Coder.
    p: [u16; 256],    // LPS probability for each state
    m: [u16; 256],    // MPS adaptation threshold for each state  
    up: [BitContext; 256],  // Next state after an MPS is coded
    dn: [BitContext; 256],  // Next state after an LPS is coded
}

impl ZpModel {
    /// Creates a new ZP model with the specified compatibility mode.
    pub fn new(djvu_compat: bool) -> Self {
        let mut p = [0; 256];
        let mut m = [0; 256];
        let mut up = [0; 256];
        let mut dn = [0; 256];

        // Initialize from the default table
        for i in 0..256 {
            p[i] = DEFAULT_ZP_TABLE[i].p;
            m[i] = DEFAULT_ZP_TABLE[i].m;
            up[i] = DEFAULT_ZP_TABLE[i].up;
            dn[i] = DEFAULT_ZP_TABLE[i].dn;
        }

        // Apply the non-DjVu compatibility patch if needed
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

    /// Encodes a single bit using an adaptive context with ZP-specific logic.
    /// This method handles the ZP context adaptation and delegates bit encoding
    /// to the arithmetic encoder.
    pub fn encode_adaptive<W: Write>(
        &self,
        arith: &mut ArithmeticEncoder<W>,
        bit: bool,
        ctx: &mut BitContext,
    ) -> Result<(), ZpModelError> {
        let ctx_idx = *ctx as usize;
        
        // ZP-specific context adaptation logic
        let z = 0x10000u32; // Simplified for now - in real ZP this would be calculated
        let p = self.p[ctx_idx] as u32;
        let m = self.m[ctx_idx] as u32;
        let mps_val = z >= 0x8000;

        // Determine if this is MPS or LPS and update context accordingly
        if bit == mps_val {
            // MPS path
            if z >= m {
                *ctx = self.up[ctx_idx];
            }
        } else {
            // LPS path  
            if z < m {
                *ctx = self.dn[ctx_idx];
            }
        }

        // Delegate actual bit encoding to the arithmetic encoder
        // For ZP, we use context 0 and let the arithmetic encoder handle the probability
        arith.encode_bit(ctx_idx, bit)?;
        
        Ok(())
    }

    /// Encodes a bit using the special non-adaptive IW44 rules.
    /// This is used for raw coefficient bits that don't use adaptive context.
    pub fn encode_raw<W: Write>(
        &self,
        arith: &mut ArithmeticEncoder<W>,
        bit: bool,
    ) -> Result<(), ZpModelError> {
        // For raw encoding, use a fixed context (context 0) with equal probability
        arith.encode_bit(0, bit)?;
        Ok(())
    }
}

/// ZP encoder that combines the ZP model with the shared arithmetic encoder.
pub struct ZpEncoder<W: Write> {
    model: ZpModel,
    arith: ArithmeticEncoder<W>,
}

impl<W: Write> ZpEncoder<W> {
    /// Creates a new ZP encoder.
    pub fn new(writer: W, djvu_compat: bool) -> Self {
    let tables = ZpTables::new(djvu_compat);
    let state_table = if djvu_compat { &ZP_STATE_TABLE } else { &*ZP_STATE_TABLE_PATCHED };
    let ac = ZpArithmeticEncoder::new(writer, state_table);
    Self {
        ac,
        tables,
        a: 0,
        subend: 0,
        finished: false,
    }


    /// Encodes a single bit using an adaptive context.
    pub fn encode(&mut self, bit: bool, ctx: &mut BitContext) -> Result<(), ZpModelError> {
        self.model.encode_adaptive(&mut self.arith, bit, ctx)
    }

    /// Encodes a bit using the special non-adaptive IW44 rules.
    pub fn encode_raw(&mut self, bit: bool) -> Result<(), ZpModelError> {
        self.model.encode_raw(&mut self.arith, bit)
    }

    /// Alias for encode_raw to maintain compatibility.
    pub fn iw_encoder(&mut self, bit: bool) -> Result<(), ZpModelError> {
        self.encode_raw(bit)
    }

    /// Finalizes the encoding and returns the writer.
    pub fn finish(self) -> Result<W, ZpModelError> {
        Ok(self.arith.finish()?)
    }
}

// For backward compatibility, create a type alias
pub type ZPCodec<W> = ZpEncoder<W>;
