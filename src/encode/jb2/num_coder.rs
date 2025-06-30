// src/jb2/num_coder.rs

use crate::encode::zp::zp_codec::{BitContext, ZpEncoder};
use crate::encode::jb2::error::Jb2Error;
use std::io::Write;

/// Maximum number of contexts before a reset is needed (for fuzz safety).
const CELLCHUNK: usize = 20_000;
/// Bounds for signed integer coding (to catch invalid inputs).
pub const BIG_POSITIVE: i32 = 262_142;
pub const BIG_NEGATIVE: i32 = -262_143;

#[derive(Default)]
struct NumCodeNode {
    bit_ctx: BitContext,
    left_child: u32,
    right_child: u32,
}

/// Adaptive integer coder with dynamic context allocation.
pub struct NumCoder {
    nodes: Vec<NumCodeNode>,
}

impl NumCoder {
    /// Create a fresh NumCoder with a dummy root context.
    pub fn new() -> Self {
        let mut nodes = Vec::with_capacity(CELLCHUNK + 1);
        nodes.push(NumCodeNode::default()); // index 0 is the "null" context
        Self { nodes }
    }

    /// Clear all contexts, returning to initial state.
    pub fn reset(&mut self) {
        self.nodes.clear();
        self.nodes.push(NumCodeNode::default());
    }

    /// Encode an integer `value` in the range [low, high].
    /// `ctx_handle` holds the entry context index, and is updated to the final context for reuse.
    pub fn code_num<W: Write>(
        &mut self,
        zp: &mut ZpEncoder<W>,
        mut value: i32,
        mut low: i32,
        mut high: i32,
        ctx_handle: &mut u32,
    ) -> Result<(), Jb2Error> {
        // Range check
        if value < low || value > high {
            return Err(Jb2Error::InvalidNumber(
                format!("Value {} outside of [{}, {}]", value, low, high)
            ));
        }

        let mut ctx_idx = *ctx_handle as usize;

        // === Phase 1: Sign bit ===
        let negative = value < 0;
        if low < 0 && high >= 0 {
            self.alloc_ctx(ctx_idx)?;
            let node = &mut self.nodes[ctx_idx];
            zp.encode(negative, &mut node.bit_ctx)?;
            ctx_idx = if negative { node.left_child as usize } else { node.right_child as usize };
        }
        if negative {
            // Mirror range for magnitude coding
            value = -value - 1;
            let temp = -low - 1;
            low = -high - 1;
            high = temp;
        }

        // === Phases 2 & 3: Magnitude bits ===
        let mut cutoff = 1;
        while low < high {
            self.alloc_ctx(ctx_idx)?;
            let node = &mut self.nodes[ctx_idx];
            let bit = value >= cutoff;
            zp.encode(bit, &mut node.bit_ctx)?;
            ctx_idx = if bit { node.right_child as usize } else { node.left_child as usize };

            // Narrow range
            if !bit {
                high = cutoff - 1;
            } else {
                low = cutoff;
            }
            cutoff = (low + high + 1) / 2;
        }

        *ctx_handle = ctx_idx as u32;
        Ok(())
    }

    /// Check if a reset is needed due to context overflow
    pub fn needs_reset(&self) -> bool {
        self.nodes.len() >= CELLCHUNK
    }

    /// Ensure the context at `index` and its children exist.
    fn alloc_ctx(&mut self, index: usize) -> Result<(), Jb2Error> {
        // Check for potential overflow before allocation
        if self.nodes.len() + 3 > CELLCHUNK {
            return Err(Jb2Error::ContextOverflow);
        }
        
        // Allocate this node if missing
        if index >= self.nodes.len() {
            // Must grow sequentially: only allow next index
            if index != self.nodes.len() {
                return Err(Jb2Error::BadNumber(
                    format!("Non-sequential context allocation: {} vs {}", index, self.nodes.len())
                ));
            }
            self.nodes.push(NumCodeNode::default());
        }
        let node = &mut self.nodes[index];
        // Allocate left child if first visit
        if node.left_child == 0 {
            let child_idx = self.nodes.len() as u32;
            node.left_child = child_idx;
            self.nodes.push(NumCodeNode::default());
        }
        // Allocate right child
        if node.right_child == 0 {
            let child_idx = self.nodes.len() as u32;
            node.right_child = child_idx;
            self.nodes.push(NumCodeNode::default());
        }
        Ok(())
    }
}
