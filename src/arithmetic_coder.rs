use std::io::Write;
use thiserror::Error;

/// Represents a state in the arithmetic coder's probability estimation table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct State {
    pub qe: u16,      // Probability estimate for the LPS
    pub nmps: usize,  // Next state if MPS is coded
    pub nlps: usize,  // Next state if LPS is coded
    pub switch: bool, // Whether to toggle MPS after coding an LPS (JB2-specific)
}

#[derive(Error, Debug)]
pub enum ArithmeticError {
    #[error("Invalid context index provided: {0}")]
    InvalidContext(usize),
    #[error("Invalid state index encountered: {0}")]
    InvalidStateIndex(usize),
    #[error("I/O error during arithmetic coding: {0}")]
    Io(#[from] std::io::Error),
    #[error("Cannot access writer after it has been finished or taken")]
    WriterUnavailable,
}

/// Generic arithmetic encoder parameterized by context count.
pub struct ArithmeticEncoder<W: Write, const N: usize> {
    writer: Option<W>,
    a: u32,                        // Range register
    c: u32,                        // Code register
    ct: i32,                       // Countdown register
    b: u8,                         // Current byte being built
    b_pos: isize,                  // Byte position (JB2-style buffering)
    context: [usize; N],           // Context states (indices into state_table)
    state_table: &'static [State], // Configurable state table
    finished: bool,
    is_jb2: bool, // Flag to toggle JB2-specific logic
}

impl<W: Write, const N: usize> ArithmeticEncoder<W, N> {
    // ---------------------------------------------------------------------
    // Compatibility helpers for legacy ZP/IW44 code paths ------------------
    /// Legacy wrapper: identical to `encode_bit` but with argument order
    /// matching the historical `encode(bit, ctx)` signature that existed in
    /// the old `ArithmeticEncoder` implementation.
    pub fn encode(&mut self, bit: bool, ctx: usize) -> Result<(), ArithmeticError> {
        self.encode_bit(ctx, bit)
    }

    /// Context-less bit coder used by the IW44 path.  This implementation
    /// simply re-uses context 0, which quickly converges to an approximately
    /// uniform model in practice.  It keeps the public signature expected by
    /// `ZpEncoder::iw_encoder`.
    pub fn encode_iw(&mut self, bit: bool) -> Result<(), ArithmeticError> {
        self.encode_bit(0, bit)
    }

    /// Returns the current value of the range register – exposed for the ZP
    /// probability update logic.
    pub fn get_a(&self) -> u32 {
        self.a
    }

    /// Creates a new encoder with the specified writer, state table, and mode.
    pub fn new(
        writer: W,
        state_table: &'static [State],
        num_contexts: usize,
        is_jb2: bool,
    ) -> Result<Self, ArithmeticError> {
        if num_contexts != N {
            return Err(ArithmeticError::InvalidContext(num_contexts));
        }
        Ok(Self {
            writer: Some(writer),
            a: 0x8000,
            c: 0,
            ct: 12,
            b: 0,
            b_pos: 7,
            context: [0; N], // Initialize all contexts to state 0
            state_table,
            finished: false,
            is_jb2,
        })
    }

    /// Encodes a single bit in the given context with an explicit MPS value.
    pub fn encode_bit(&mut self, ctx: usize, d: bool) -> Result<(), ArithmeticError> {
        if ctx >= N {
            return Err(ArithmeticError::InvalidContext(ctx));
        }
        let state_idx = self.context[ctx];
        if state_idx >= self.state_table.len() {
            return Err(ArithmeticError::InvalidStateIndex(state_idx));
        }
        let state = &self.state_table[state_idx];
        let qe = state.qe as u32;

        if self.is_jb2 {
            // JB2-specific logic: MPS is inferred from state index parity
            let mps_val = (state_idx & 1) != 0;
            self.a -= qe;

            if d != mps_val {
                // LPS path
                if self.a < qe {
                    self.c += self.a;
                    self.a = qe;
                }
                self.context[ctx] = if state.switch {
                    state.nlps ^ 1
                } else {
                    state.nlps
                };
            } else {
                // MPS path
                self.c += qe;
                self.context[ctx] = state.nmps;
            }
        } else {
            // ZP/IW44 logic: MPS is provided explicitly
            let a_mps = self.a.saturating_sub(qe);
            if !d {
                // LPS path (d = false means LPS in ZP)
                self.c = self.c.saturating_add(a_mps);
                self.a = qe;
                self.context[ctx] = state.nlps;
            } else {
                // MPS path
                self.a = a_mps;
                self.context[ctx] = state.nmps;
            }
        }

        if self.a < 0x8000 {
            self.renorm_step()?;
        }
        Ok(())
    }

    /// Renormalizes the encoder state.
    fn renorm_step(&mut self) -> Result<(), ArithmeticError> {
        self.a <<= 1;
        self.c <<= 1;
        if self.c >= self.a {
            self.c -= self.a;
        }
        self.b_pos -= 1;
        if self.b_pos < 0 {
            self.byte_out(false)?;
        }
        Ok(())
    }

    /// Outputs a byte, with JB2 or ZP-specific handling.
    fn byte_out(&mut self, is_final: bool) -> Result<(), ArithmeticError> {
        let writer = self
            .writer
            .as_mut()
            .ok_or(ArithmeticError::WriterUnavailable)?;

        let mut temp_b = self.b as u32;
        if is_final {
            temp_b += self.c >> 13;
        }

        if temp_b > 0xFF {
            writer.write_all(&[0xFF])?;
            self.b = (temp_b & 0xFF) as u8;
            self.b_pos = if self.is_jb2 { 6 } else { 7 };
        } else {
            writer.write_all(&[temp_b as u8])?;
            self.b = 0;
            self.b_pos = if self.is_jb2 { 7 } else { 8 };
        }
        Ok(())
    }

    /// Flushes the encoder, with mode-specific finalization.
    pub fn flush(&mut self, with_marker: bool) -> Result<(), ArithmeticError> {
        if self.finished {
            return Ok(());
        }
        if self.is_jb2 {
            let temp_c = self.c + self.a;
            self.c |= 0xFFFF;
            if self.c >= temp_c {
                self.c -= 0x8000;
            }
            self.byte_out(true)?;
            self.byte_out(true)?;
            if with_marker {
                self.writer
                    .as_mut()
                    .ok_or(ArithmeticError::WriterUnavailable)?
                    .write_all(&[0xFF, 0xAC])?;
            }
        } else {
            for _ in 0..18 {
                self.byte_out(false)?;
            }
            self.byte_out(true)?;
        }
        self.writer
            .as_mut()
            .ok_or(ArithmeticError::WriterUnavailable)?
            .flush()?;
        self.finished = true;
        Ok(())
    }

    /// Returns the underlying writer.
    pub fn into_inner(mut self) -> Result<W, ArithmeticError> {
        self.flush(true)?;
        self.writer.take().ok_or(ArithmeticError::WriterUnavailable)
    }
}

impl<W: Write, const N: usize> Drop for ArithmeticEncoder<W, N> {
    fn drop(&mut self) {
        if !self.finished && !std::thread::panicking() {
            let _ = self.flush(false);
        }
    }
}

// The standard JB2 state transition table (see JBIG2 spec, Annex A).
// The actual MPS value is determined by `state_index & 1`.
const JB2_STATE_TABLE: [State; 94] = [
    // MPS = 0 states (even indices)
    /* 0*/
    State {
        qe: 0x5555,
        nlps: 1,
        nmps: 2,
        switch: true,
    },
    /* 1*/
    State {
        qe: 0x5555,
        nlps: 0,
        nmps: 47,
        switch: true,
    }, // MPS = 1
    /* 2*/
    State {
        qe: 0x2ABF,
        nlps: 3,
        nmps: 4,
        switch: false,
    },
    /* 3*/
    State {
        qe: 0x2ABF,
        nlps: 2,
        nmps: 49,
        switch: false,
    },
    /* 4*/
    State {
        qe: 0x1560,
        nlps: 5,
        nmps: 6,
        switch: false,
    },
    /* 5*/
    State {
        qe: 0x1560,
        nlps: 4,
        nmps: 51,
        switch: false,
    },
    /* 6*/
    State {
        qe: 0x0AD0,
        nlps: 7,
        nmps: 8,
        switch: false,
    },
    /* 7*/
    State {
        qe: 0x0AD0,
        nlps: 6,
        nmps: 53,
        switch: false,
    },
    /* 8*/
    State {
        qe: 0x0568,
        nlps: 9,
        nmps: 10,
        switch: false,
    },
    /* 9*/
    State {
        qe: 0x0568,
        nlps: 8,
        nmps: 55,
        switch: false,
    },
    /*10*/
    State {
        qe: 0x02B4,
        nlps: 11,
        nmps: 12,
        switch: false,
    },
    /*11*/
    State {
        qe: 0x02B4,
        nlps: 10,
        nmps: 57,
        switch: false,
    },
    /*12*/
    State {
        qe: 0x015A,
        nlps: 13,
        nmps: 14,
        switch: false,
    },
    /*13*/
    State {
        qe: 0x015A,
        nlps: 12,
        nmps: 59,
        switch: false,
    },
    /*14*/
    State {
        qe: 0x00AD,
        nlps: 15,
        nmps: 16,
        switch: false,
    },
    /*15*/
    State {
        qe: 0x00AD,
        nlps: 14,
        nmps: 61,
        switch: false,
    },
    /*16*/
    State {
        qe: 0x0057,
        nlps: 17,
        nmps: 18,
        switch: false,
    },
    /*17*/
    State {
        qe: 0x0057,
        nlps: 16,
        nmps: 63,
        switch: false,
    },
    /*18*/
    State {
        qe: 0x002B,
        nlps: 19,
        nmps: 20,
        switch: false,
    },
    /*19*/
    State {
        qe: 0x002B,
        nlps: 18,
        nmps: 65,
        switch: false,
    },
    /*20*/
    State {
        qe: 0x0016,
        nlps: 21,
        nmps: 22,
        switch: false,
    },
    /*21*/
    State {
        qe: 0x0016,
        nlps: 20,
        nmps: 67,
        switch: false,
    },
    /*22*/
    State {
        qe: 0x000B,
        nlps: 23,
        nmps: 24,
        switch: false,
    },
    /*23*/
    State {
        qe: 0x000B,
        nlps: 22,
        nmps: 69,
        switch: false,
    },
    /*24*/
    State {
        qe: 0x0005,
        nlps: 25,
        nmps: 26,
        switch: false,
    },
    /*25*/
    State {
        qe: 0x0005,
        nlps: 24,
        nmps: 71,
        switch: false,
    },
    /*26*/
    State {
        qe: 0x0003,
        nlps: 27,
        nmps: 28,
        switch: false,
    },
    /*27*/
    State {
        qe: 0x0003,
        nlps: 26,
        nmps: 73,
        switch: false,
    },
    /*28*/
    State {
        qe: 0x0001,
        nlps: 29,
        nmps: 30,
        switch: false,
    },
    /*29*/
    State {
        qe: 0x0001,
        nlps: 28,
        nmps: 75,
        switch: false,
    },
    /*30*/
    State {
        qe: 0x0001,
        nlps: 31,
        nmps: 32,
        switch: false,
    },
    /*31*/
    State {
        qe: 0x0001,
        nlps: 30,
        nmps: 77,
        switch: false,
    },
    /*32*/
    State {
        qe: 0x0001,
        nlps: 33,
        nmps: 34,
        switch: false,
    },
    /*33*/
    State {
        qe: 0x0001,
        nlps: 32,
        nmps: 79,
        switch: false,
    },
    /*34*/
    State {
        qe: 0x0001,
        nlps: 35,
        nmps: 36,
        switch: false,
    },
    /*35*/
    State {
        qe: 0x0001,
        nlps: 34,
        nmps: 81,
        switch: false,
    },
    /*36*/
    State {
        qe: 0x0001,
        nlps: 37,
        nmps: 38,
        switch: false,
    },
    /*37*/
    State {
        qe: 0x0001,
        nlps: 36,
        nmps: 83,
        switch: false,
    },
    /*38*/
    State {
        qe: 0x0001,
        nlps: 39,
        nmps: 40,
        switch: false,
    },
    /*39*/
    State {
        qe: 0x0001,
        nlps: 38,
        nmps: 85,
        switch: false,
    },
    /*40*/
    State {
        qe: 0x0001,
        nlps: 41,
        nmps: 42,
        switch: false,
    },
    /*41*/
    State {
        qe: 0x0001,
        nlps: 40,
        nmps: 87,
        switch: false,
    },
    /*42*/
    State {
        qe: 0x0001,
        nlps: 43,
        nmps: 44,
        switch: false,
    },
    /*43*/
    State {
        qe: 0x0001,
        nlps: 42,
        nmps: 89,
        switch: false,
    },
    /*44*/
    State {
        qe: 0x0001,
        nlps: 45,
        nmps: 46,
        switch: false,
    },
    /*45*/
    State {
        qe: 0x0001,
        nlps: 44,
        nmps: 91,
        switch: false,
    },
    /*46*/
    State {
        qe: 0x0001,
        nlps: 46,
        nmps: 46,
        switch: false,
    },
    // MPS = 1 states (odd indices)
    /*47*/
    State {
        qe: 0x5555,
        nlps: 48,
        nmps: 49,
        switch: true,
    },
    /*48*/
    State {
        qe: 0x5555,
        nlps: 47,
        nmps: 2,
        switch: true,
    },
    /*49*/
    State {
        qe: 0x2ABF,
        nlps: 50,
        nmps: 51,
        switch: false,
    },
    /*50*/
    State {
        qe: 0x2ABF,
        nlps: 49,
        nmps: 4,
        switch: false,
    },
    /*51*/
    State {
        qe: 0x1560,
        nlps: 52,
        nmps: 53,
        switch: false,
    },
    /*52*/
    State {
        qe: 0x1560,
        nlps: 51,
        nmps: 6,
        switch: false,
    },
    /*53*/
    State {
        qe: 0x0AD0,
        nlps: 54,
        nmps: 55,
        switch: false,
    },
    /*54*/
    State {
        qe: 0x0AD0,
        nlps: 53,
        nmps: 8,
        switch: false,
    },
    /*55*/
    State {
        qe: 0x0568,
        nlps: 56,
        nmps: 57,
        switch: false,
    },
    /*56*/
    State {
        qe: 0x0568,
        nlps: 55,
        nmps: 10,
        switch: false,
    },
    /*57*/
    State {
        qe: 0x02B4,
        nlps: 58,
        nmps: 59,
        switch: false,
    },
    /*58*/
    State {
        qe: 0x02B4,
        nlps: 57,
        nmps: 12,
        switch: false,
    },
    /*59*/
    State {
        qe: 0x015A,
        nlps: 60,
        nmps: 61,
        switch: false,
    },
    /*60*/
    State {
        qe: 0x015A,
        nlps: 59,
        nmps: 14,
        switch: false,
    },
    /*61*/
    State {
        qe: 0x00AD,
        nlps: 62,
        nmps: 63,
        switch: false,
    },
    /*62*/
    State {
        qe: 0x00AD,
        nlps: 61,
        nmps: 16,
        switch: false,
    },
    /*63*/
    State {
        qe: 0x0057,
        nlps: 64,
        nmps: 65,
        switch: false,
    },
    /*64*/
    State {
        qe: 0x0057,
        nlps: 63,
        nmps: 18,
        switch: false,
    },
    /*65*/
    State {
        qe: 0x002B,
        nlps: 66,
        nmps: 67,
        switch: false,
    },
    /*66*/
    State {
        qe: 0x002B,
        nlps: 65,
        nmps: 20,
        switch: false,
    },
    /*67*/
    State {
        qe: 0x0016,
        nlps: 68,
        nmps: 69,
        switch: false,
    },
    /*68*/
    State {
        qe: 0x0016,
        nlps: 67,
        nmps: 22,
        switch: false,
    },
    /*69*/
    State {
        qe: 0x000B,
        nlps: 70,
        nmps: 71,
        switch: false,
    },
    /*70*/
    State {
        qe: 0x000B,
        nlps: 69,
        nmps: 24,
        switch: false,
    },
    /*71*/
    State {
        qe: 0x0005,
        nlps: 72,
        nmps: 73,
        switch: false,
    },
    /*72*/
    State {
        qe: 0x0005,
        nlps: 71,
        nmps: 26,
        switch: false,
    },
    /*73*/
    State {
        qe: 0x0003,
        nlps: 74,
        nmps: 75,
        switch: false,
    },
    /*74*/
    State {
        qe: 0x0003,
        nlps: 73,
        nmps: 28,
        switch: false,
    },
    /*75*/
    State {
        qe: 0x0001,
        nlps: 76,
        nmps: 77,
        switch: false,
    },
    /*76*/
    State {
        qe: 0x0001,
        nlps: 75,
        nmps: 30,
        switch: false,
    },
    /*77*/
    State {
        qe: 0x0001,
        nlps: 78,
        nmps: 79,
        switch: false,
    },
    /*78*/
    State {
        qe: 0x0001,
        nlps: 77,
        nmps: 32,
        switch: false,
    },
    /*79*/
    State {
        qe: 0x0001,
        nlps: 80,
        nmps: 81,
        switch: false,
    },
    /*80*/
    State {
        qe: 0x0001,
        nlps: 79,
        nmps: 34,
        switch: false,
    },
    /*81*/
    State {
        qe: 0x0001,
        nlps: 82,
        nmps: 83,
        switch: false,
    },
    /*82*/
    State {
        qe: 0x0001,
        nlps: 81,
        nmps: 36,
        switch: false,
    },
    /*83*/
    State {
        qe: 0x0001,
        nlps: 84,
        nmps: 85,
        switch: false,
    },
    /*84*/
    State {
        qe: 0x0001,
        nlps: 83,
        nmps: 38,
        switch: false,
    },
    /*85*/
    State {
        qe: 0x0001,
        nlps: 86,
        nmps: 87,
        switch: false,
    },
    /*86*/
    State {
        qe: 0x0001,
        nlps: 85,
        nmps: 40,
        switch: false,
    },
    /*87*/
    State {
        qe: 0x0001,
        nlps: 88,
        nmps: 89,
        switch: false,
    },
    /*88*/
    State {
        qe: 0x0001,
        nlps: 87,
        nmps: 42,
        switch: false,
    },
    /*89*/
    State {
        qe: 0x0001,
        nlps: 90,
        nmps: 91,
        switch: false,
    },
    /*90*/
    State {
        qe: 0x0001,
        nlps: 89,
        nmps: 44,
        switch: false,
    },
    /*91*/
    State {
        qe: 0x0001,
        nlps: 92,
        nmps: 93,
        switch: false,
    },
    /*92*/
    State {
        qe: 0x0001,
        nlps: 91,
        nmps: 46,
        switch: false,
    },
    /*93*/
    State {
        qe: 0x0001,
        nlps: 93,
        nmps: 92,
        switch: false,
    },
];
