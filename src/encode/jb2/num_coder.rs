//! Adaptive integer coder compatible with a fixed-context arithmetic coder.

use crate::arithmetic_coder::ArithmeticEncoder;
use crate::encode::jb2::error::Jb2Error;
use std::io::Write;

/// Bounds for signed integer coding.
pub const BIG_POSITIVE: i32 = 262_142;
pub const BIG_NEGATIVE: i32 = -262_143;

#[derive(Default, Clone, Copy)]
struct NumCodeNode {
    // This node's context index within the main Jb2ArithmeticEncoder.
    context_index: u32,
    // Indices into the `nodes` vector for children.
    left_child: u32,
    right_child: u32,
}

/// Adaptive integer coder that uses a pre-allocated slice of contexts
/// from a `Jb2ArithmeticEncoder`.
pub struct NumCoder {
    nodes: Vec<NumCodeNode>,
    next_context: u32,
    max_contexts: u32,
}

impl NumCoder {
    /// Creates a new NumCoder that will use a specific range of contexts.
    pub fn new(base_context_index: u32, max_contexts: u32) -> Self {
        let mut nodes = Vec::with_capacity(256);
        // Node 0 is the root for any new coding tree.
        nodes.push(NumCodeNode {
            context_index: base_context_index,
            left_child: 0,
            right_child: 0,
        });
        Self {
            nodes,
            next_context: base_context_index + 1,
            max_contexts,
        }
    }

    /// Allocates a new context and returns its handle
    pub fn alloc_context(&mut self) -> u32 {
        if self.next_context >= self.max_contexts {
            // Reuse contexts if we run out (simple round-robin)
            self.next_context = 0;
        }
        let ctx = self.next_context;
        self.next_context += 1;
        ctx
    }

    /// Encodes a signed integer using adaptive binary coding
    pub fn code_int<W: Write, const N: usize>(
        &mut self,
        ac: &mut ArithmeticEncoder<W, N>,
        value: i32,
        ctx_handle: &mut u32,
    ) -> Result<(), Jb2Error> {
        // Encode the sign bit
        let sign_bit = value < 0;
        let uvalue = value.unsigned_abs() as u32;

        // Encode the magnitude using adaptive binary coding
        let mut mask = 1u32 << 30; // Start with a high bit
        while mask > 0 && (uvalue & mask) == 0 {
            mask >>= 1;
        }

        // Allocate context if needed
        if *ctx_handle == 0 {
            *ctx_handle = self.alloc_context();
        }

        // Encode the bits
        while mask > 0 {
            let bit = (uvalue & mask) != 0;
            ac.encode_bit(*ctx_handle as usize, bit)?;
            mask >>= 1;
        }

        // Encode the sign bit if non-zero
        if uvalue != 0 {
            ac.encode_bit(*ctx_handle as usize, sign_bit)?;
        }

        Ok(())
    }

    /// Encodes an integer `value` in the range [low, high].
    /// `ctx_handle` tracks the root of the probability tree for this value type.
    pub fn code_num<W: Write, const N: usize>(
        &mut self,
        ac: &mut ArithmeticEncoder<W, N>,
        mut value: i32,
        mut low: i32,
        mut high: i32,
        ctx_handle: &mut u32,
    ) -> Result<(), Jb2Error> {
        if value < low || value > high {
            return Err(Jb2Error::InvalidNumber(format!(
                "Value {} outside of [{}, {}]",
                value, low, high
            )));
        }

        let mut node_idx = *ctx_handle as usize;

        // Phase 1: Sign bit
        if low < 0 && high >= 0 {
            self.alloc_children(node_idx)?;
            let node = &self.nodes[node_idx];
            let negative = value < 0;
            ac.encode_bit(node.context_index as usize, negative)?;
            node_idx = if negative {
                node.left_child as usize
            } else {
                node.right_child as usize
            };
            if negative {
                let temp = -low - 1;
                low = -high - 1;
                high = temp;
                value = -value - 1;
            }
        }

        // Phase 2 & 3: Magnitude bits
        let mut cutoff = 1;
        while low < high {
            self.alloc_children(node_idx)?;
            let node = &self.nodes[node_idx];
            let bit = value >= cutoff;
            ac.encode_bit(node.context_index as usize, bit)?;
            node_idx = if bit {
                node.right_child as usize
            } else {
                node.left_child as usize
            };

            if !bit {
                high = cutoff - 1;
            } else {
                low = cutoff;
            }
            cutoff = (low + high + 1) / 2;
        }

        *ctx_handle = node_idx as u32;
        Ok(())
    }

    /// Ensures the children for a given node are allocated.
    fn alloc_children(&mut self, node_idx: usize) -> Result<(), Jb2Error> {
        if self.nodes[node_idx].left_child == 0 {
            let left_idx = self.nodes.len() as u32;
            let right_idx = left_idx + 1;
            self.nodes[node_idx].left_child = left_idx;
            self.nodes[node_idx].right_child = right_idx;

            if self.next_context + 1 > self.max_contexts {
                return Err(Jb2Error::ContextOverflow);
            }

            self.nodes.push(NumCodeNode {
                context_index: self.next_context,
                left_child: 0,
                right_child: 0,
            });
            self.nodes.push(NumCodeNode {
                context_index: self.next_context + 1,
                left_child: 0,
                right_child: 0,
            });
            self.next_context += 2;
        }
        Ok(())
    }
}
