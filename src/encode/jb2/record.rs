//! Handles the JB2 record stream state machine.
//!
//! This module is responsible for encoding the sequence of symbol instances
//! that make up the content of a page.

use crate::arithmetic_coder::ArithmeticEncoder;
use crate::encode::jb2::context;
use crate::encode::jb2::error::Jb2Error;
use crate::encode::jb2::num_coder::NumCoder;
use crate::encode::jb2::relative::{self, RelLocPredictor};
use crate::encode::jb2::symbol_dict::{BitImage, ConnectedComponent};
use std::io::Write;

/// JB2 record types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecordType {
    /// A symbol that is an instance of a dictionary symbol.
    SymbolInstance = 1,
    /// A symbol encoded as a refinement of a dictionary symbol.
    SymbolRefinement = 2,
}

/// Encodes the stream of symbol instance records for a page.
pub struct RecordStreamEncoder {
    nc: NumCoder,
    rlp: RelLocPredictor,
    refinement_base_context: u32,
    // Context handles for the NumCoder
    ctx_handle_rec_type: u32,
    ctx_handle_sym_id: u32,
    ctx_handle_rel_loc: u32,
}

impl RecordStreamEncoder {
    /// Creates a new record stream encoder.
    /// It requires a base context index to ensure its contexts don't overlap
    /// with other components.
        pub fn new(base_context_index: u32, max_contexts: u32, refinement_base_context: u32) -> Self {
        // Partition the available contexts between the relative location predictor
        // and the general-purpose number coder.
        let rlp_contexts = relative::NUM_CONTEXTS;
        let nc_contexts = max_contexts - rlp_contexts;
        let nc_base_index = base_context_index + rlp_contexts;

        let mut nc = NumCoder::new(nc_base_index, nc_contexts);

        // Allocate context handles from the number coder.
        let ctx_handle_rec_type = nc.alloc_context();
        let ctx_handle_sym_id = nc.alloc_context();
        let ctx_handle_rel_loc = nc.alloc_context();

        Self {
            nc,
            rlp: RelLocPredictor::new(base_context_index),
            refinement_base_context,
            ctx_handle_rec_type,
            ctx_handle_sym_id,
            ctx_handle_rel_loc,
        }
    }

    /// Encodes a single connected component as a record, potentially as a refinement.
    pub fn code_record<W: Write, const N: usize>(
        &mut self,
        ac: &mut ArithmeticEncoder<W, N>,
        component: &ConnectedComponent,
        dictionary: &[BitImage],
        is_refinement: bool,
    ) -> Result<(), Jb2Error> {
        let rec_type = if is_refinement {
            RecordType::SymbolRefinement
        } else {
            RecordType::SymbolInstance
        };

        // 1. Encode the record type.
        self.code_rec_type(ac, rec_type)?;

        // 2. Encode the symbol ID.
        let sym_id = component.dict_symbol_index.unwrap_or(0);
        let mut ctx_handle = self.ctx_handle_sym_id;
        self.nc.code_int(ac, sym_id as i32, &mut ctx_handle)?;
        self.ctx_handle_sym_id = ctx_handle;

        // 3. Encode the location (and get the relative offset for refinement).
        // Get the predicted location
        let (pred_dx, pred_dy) = self.rlp.predict(component.bounds.x as i32, component.bounds.y as i32, sym_id, dictionary);
        
        // Encode the difference between actual and predicted location
        let dx = component.bounds.x as i32 - pred_dx;
        let dy = component.bounds.y as i32 - pred_dy;
        
        // Encode the relative location
        let mut ctx_handle = self.ctx_handle_rel_loc;
        self.nc.code_int(ac, dx, &mut ctx_handle)?;
        self.nc.code_int(ac, dy, &mut ctx_handle)?;
        self.ctx_handle_rel_loc = ctx_handle;

        // 4. If it's a refinement, encode the actual bitmap differences.
        if is_refinement {
            let reference_symbol = &dictionary[sym_id];
            context::encode_bitmap_refine(
                ac,
                &component.bitmap,
                reference_symbol,
                dx,
                dy,
                self.refinement_base_context as usize,
            )?;
        }

        Ok(())
    }

    /// Encodes the record type using the number coder.
    fn code_rec_type<W: Write, const N: usize>(
        &mut self,
        ac: &mut ArithmeticEncoder<W, N>,
        rec_type: RecordType,
    ) -> Result<(), Jb2Error> {
        // We use a simple binary encoding for the two record types.
        let bit = match rec_type {
            RecordType::SymbolInstance => false,
            RecordType::SymbolRefinement => true,
        };
        self.nc.code_int(ac, if bit { 1 } else { 0 }, &mut self.ctx_handle_rec_type)
    }
}
