// src/zp_codec/mod.rs
//! ZP Codec implementation using the refactored arithmetic encoder

use crate::arithmetic_coder::{ZpArithmeticEncoder, ArithmeticError};
use crate::arithtable::ZP_STATE_TABLE;
use std::io::Write;
use thiserror::Error;

pub mod table;

#[derive(Error, Debug)]
pub enum ZpCodecError {
    #[error("Arithmetic coding error: {0:?}")]
    Arithmetic(ArithmeticError),
    #[error("Encoder already finished")]
    Finished,
}

impl From<ArithmeticError> for ZpCodecError {
    fn from(err: ArithmeticError) -> Self {
        ZpCodecError::Arithmetic(err)
    }
}

/// ZP Encoder wrapper around ZpArithmeticEncoder
pub struct ZpEncoder<W: Write> {
    inner: ZpArithmeticEncoder<W>,
    finished: bool,
}

impl<W: Write> ZpEncoder<W> {
    /// Create a new ZP encoder
    pub fn new(writer: W, _djvu_compat: bool) -> Self {
        Self {
            inner: ZpArithmeticEncoder::new(writer, &ZP_STATE_TABLE),
            finished: false,
        }
    }

    /// Encode a bit in the given context
    pub fn encode(&mut self, bit: bool, context: &mut u8) -> Result<(), ZpCodecError> {
        if self.finished {
            return Err(ZpCodecError::Finished);
        }
        self.inner.encode_bit(*context as usize, bit)?;
        Ok(())
    }

    /// Flush and finish the encoder, returning the writer
    pub fn finish(mut self) -> Result<W, ZpCodecError> {
        if self.finished {
            return Err(ZpCodecError::Finished);
        }
        self.finished = true;
        self.inner.flush(false)?;
        Ok(self.inner.into_inner()?)
    }
}
