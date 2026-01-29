//! Handles the JB2 record stream state machine.
//!
//! This module is responsible for encoding the sequence of symbol instances
//! that make up the content of a page.

use crate::encode::jb2::context;
use crate::encode::jb2::error::Jb2Error;
use crate::encode::jb2::num_coder::{NumCoder, NumContext, BIG_NEGATIVE, BIG_POSITIVE};
use crate::encode::jb2::relative::RelLocPredictor;
use crate::encode::jb2::symbol_dict::{BitImage, ConnectedComponent};
use crate::encode::zc::ZEncoder;
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
    // NumContext handles for tree-based encoding
    ctx_sym_id: NumContext,
    ctx_rel_loc_x: NumContext,
    ctx_rel_loc_y: NumContext,
    ctx_rec_type: NumContext,
}

impl RecordStreamEncoder {
    /// Creates a new record stream encoder.
    /// It requires a base context index to ensure its contexts don't overlap
    /// with other components.
    pub fn new(base_context_index: u32, _max_contexts: u32, refinement_base_context: u32) -> Self {
        Self {
            nc: NumCoder::new(),
            rlp: RelLocPredictor::new(base_context_index),
            refinement_base_context,
            ctx_sym_id: 0,
            ctx_rel_loc_x: 0,
            ctx_rel_loc_y: 0,
            ctx_rec_type: 0,
        }
    }

    /// Encodes a single connected component as a record, potentially as a refinement.
    pub fn code_record<W: Write>(
        &mut self,
        ac: &mut ZEncoder<W>,
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
        self.nc.code_num(
            ac,
            &mut self.ctx_sym_id,
            0,
            dictionary.len() as i32 - 1,
            sym_id as i32,
        )?;

        // 3. Encode the location (and get the relative offset for refinement).
        // Get the predicted location
        let (pred_dx, pred_dy) = self.rlp.predict(
            component.bounds.x as i32,
            component.bounds.y as i32,
            sym_id,
            dictionary,
        );

        // Encode the difference between actual and predicted location
        let dx = component.bounds.x as i32 - pred_dx;
        let dy = component.bounds.y as i32 - pred_dy;

        // Encode the relative location using reasonable bounds
        self.nc.code_num(ac, &mut self.ctx_rel_loc_x, BIG_NEGATIVE, BIG_POSITIVE, dx)?;
        self.nc.code_num(ac, &mut self.ctx_rel_loc_y, BIG_NEGATIVE, BIG_POSITIVE, dy)?;

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
    fn code_rec_type<W: Write>(
        &mut self,
        ac: &mut ZEncoder<W>,
        rec_type: RecordType,
    ) -> Result<(), Jb2Error> {
        // Encode record type as integer (1 for SymbolInstance, 2 for SymbolRefinement)
        self.nc.code_num(ac, &mut self.ctx_rec_type, 1, 2, rec_type as i32)
    }
}
