// src/iw44/codec.rs
use super::coeff_map::{Block, CoeffMap};
use super::constants::{BAND_BUCKETS, IW_NORM, IW_QUANT, IW_SHIFT};
use crate::encode::zp::ZpEncoder;
use bitflags::bitflags;

// Represents a ZPCodec context. In the C++ code this is a single byte.
pub type BitContext = u8;

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct CoeffState: u8 {
        const ZERO   = 1 << 0; // This coeff is known to be zero for this bitplane
        const ACTIVE = 1 << 1; // This coeff is already active from a previous bitplane
        const NEW    = 1 << 2; // This coeff becomes active in this bitplane
        const UNK    = 1 << 3; // This coeff might become active
    }
}

pub struct Codec {
    pub map: CoeffMap,
    pub emap: CoeffMap, // "encoded map" to track state

    pub cur_band: usize,
    pub cur_bit: i32,
    pub slice_count: usize, // Track total slices for safety

    // Quantization tables, mutable as they are shifted down each bitplane
    quant_hi: [i32; 10],
    quant_lo: [i32; 16],

    // Coding contexts
    ctx_start: [BitContext; 32],
    ctx_bucket: [[BitContext; 8]; 10],
    ctx_mant: BitContext,
    ctx_root: BitContext,
}

impl Codec {
    pub fn new(map: CoeffMap) -> Self {
        let mut quant_lo = [0; 16];
        let mut quant_hi = [0; 10];

        // Initialize quantization tables from constants, but scale them down by the
        // same left-shift (2^IW_SHIFT) applied to coefficients so that the first
        // bit-plane corresponds to bit 0 of the scaled value. Without this scaling
        // the thresholds are 64× too large which makes most slices appear null.
        for (dst, src) in quant_lo.iter_mut().zip(IW_QUANT.iter()) {
            *dst = *src >> IW_SHIFT; // divide by 64
        }
        // Band-wise high thresholds (bands 1..9)
        for (dst, src) in quant_hi[1..].iter_mut().zip(IW_QUANT[1..10].iter()) {
            *dst = *src >> IW_SHIFT; // divide by 64
        }

        let emap = CoeffMap::new(map.iw, map.ih);

        Self {
            map,
            emap,
            cur_band: 0,
            cur_bit: 1, // C++ starts at 1
            slice_count: 0,
            quant_hi,
            quant_lo,
            ctx_start: [0; 32],
            ctx_bucket: [[0; 8]; 10],
            ctx_mant: 0,
            ctx_root: 0,
        }
    }

    /// Corresponds to `is_null_slice`.
    fn is_null_slice(&self, band: usize, _bit: i32, coeff_state: &mut [CoeffState]) -> bool {
        if band == 0 {
            let mut is_null = true;
            for i in 0..16 {
                let threshold = self.quant_lo[i];
                coeff_state[i] = CoeffState::ZERO;
                if threshold > 0 && threshold < 0x8000 {
                    coeff_state[i] = CoeffState::UNK;
                    is_null = false;
                }
            }
            is_null
        } else {
            let threshold = self.quant_hi[band];
            threshold <= 0 || threshold >= 0x8000
        }
    }

    /// Fast check if slice is null without computing neighbor activity
    /// This performs a simplified check that should catch most null slices
    fn slice_is_null_fast(&self) -> bool {
        if self.cur_band == 0 {
            // For band 0, if all quantization thresholds are too high, slice is null
            let all_too_high = self.quant_lo.iter().all(|&threshold| threshold >= 0x8000);
            let all_too_low = self.quant_lo.iter().all(|&threshold| threshold <= 0);
            
            all_too_high || all_too_low
        } else {
            // For other bands, check the band's quantization threshold
            let threshold = self.quant_hi[self.cur_band];
            let result = threshold <= 0 || threshold >= 0x8000;
            
            result
        }
    }

    /// The main entry point to encode one "slice" of data.
    pub fn encode_slice<W: std::io::Write>(
        &mut self,
        zp: &mut ZpEncoder<W>,
    ) -> Result<bool, crate::encode::zp::ZpCodecError> {
        if self.cur_bit < 0 {
            return Ok(false); // finished all bit-planes
        }

        // Safety check: prevent runaway encoding
        self.slice_count += 1;
        if self.slice_count > 1000 {
            self.cur_bit = -1;
            return Ok(false);
        }

        let mut coeff_state = [CoeffState::empty(); 256];
        
        // First, do the detailed null check WITHOUT computing neighbor activity
        if self.is_null_slice(self.cur_band, self.cur_bit, &mut coeff_state) {
            let more = self.finish_slice();
            return Ok(more);
        }

        // Only if slice is NOT null, compute expensive neighbor activity
        let blocks_w = self.map.bw / 32;
        let mut neighbor_active = vec![vec![false; 64]; self.map.num_blocks]; // 64 buckets per block
        
        for blk in 0..self.map.num_blocks {
            let bx = blk % blocks_w;
            let by = blk / blocks_w;
            for bucket in 0..64 { // 64 buckets per 32×32 block
                neighbor_active[blk][bucket] = 
                    // Check left neighbor
                    (bx > 0 && 
                     self.emap.blocks[blk - 1].get_bucket(bucket as u8)
                        .map_or(false, |b| b.iter().any(|&c| c != 0))) ||
                    // Check top neighbor  
                    (by > 0 && 
                     self.emap.blocks[blk - blocks_w].get_bucket(bucket as u8)
                        .map_or(false, |b| b.iter().any(|&c| c != 0)));
            }
        }
        
        // Process the non-null slice
        for block_idx in 0..self.map.num_blocks {
            let fbucket = BAND_BUCKETS[self.cur_band].start;
            let nbucket = BAND_BUCKETS[self.cur_band].size;
            self.encode_buckets(
                zp,
                block_idx,
                &mut coeff_state,
                fbucket as usize,
                nbucket as usize,
                &neighbor_active[block_idx], // Pass pre-computed neighbor activity
            )?;
        }

        let more = self.finish_slice();
        
        Ok(more)
    }

    fn finish_slice(&mut self) -> bool {
        // Reduce quantization threshold for next round
        self.quant_hi[self.cur_band] >>= 1;
        if self.cur_band == 0 {
            for q in self.quant_lo.iter_mut() {
                *q >>= 1;
            }
        }

        self.cur_band += 1;
        if self.cur_band >= BAND_BUCKETS.len() {
            self.cur_band = 0;
            self.cur_bit += 1;
            
            // Hard safety check - prevent infinite loops
            if self.cur_bit >= 16 {
                self.cur_bit = -1;
                return false;
            }
            
            // Check if we are done - Modified termination condition
            // If all quantization thresholds are very small (< 8), consider done
            let max_quant = self.quant_hi.iter().max().copied().unwrap_or(0);
            if max_quant < 8 {
                self.cur_bit = -1;
                return false;
            }
            
            // Also check the original C++ condition - last band must be 0
            if self.quant_hi[BAND_BUCKETS.len() - 1] == 0 {
                self.cur_bit = -1;
                return false;
            }
        }
        
        true
    }

    /// Prepares states for a set of buckets within a block.
    fn prepare_bucket_states(
        quant_hi: &[i32; 10],
        quant_lo: &[i32; 16],
        block: &Block,
        eblock: &Block,
        band: usize,
        fbucket: usize,
        nbucket: usize,
        coeff_state: &mut [CoeffState], // The global one for the band
        bucket_states: &mut [CoeffState], // The per-block one
    ) -> CoeffState {
        let mut bbstate = CoeffState::empty();
        if band > 0 {
            // Band other than zero
            let thres = quant_hi[band];
            for buckno in 0..nbucket {
                let cstate_slice = &mut coeff_state[buckno * 16..(buckno + 1) * 16];
                let pcoeff = block.get_bucket((fbucket + buckno) as u8);
                let epcoeff = eblock.get_bucket((fbucket + buckno) as u8);

                let mut bstatetmp = CoeffState::empty();
                if pcoeff.is_none() {
                    bstatetmp = CoeffState::UNK;
                } else if epcoeff.is_none() {
                    let pcoeff = pcoeff.unwrap();
                    for i in 0..16 {
                        let mut cst = CoeffState::UNK;
                        if (pcoeff[i] as i32).abs() >= thres {
                            cst |= CoeffState::NEW;
                        }
                        cstate_slice[i] = cst;
                        bstatetmp |= cst;
                    }
                } else {
                    let pcoeff = pcoeff.unwrap();
                    let epcoeff = epcoeff.unwrap();
                    for i in 0..16 {
                        let mut cst = CoeffState::UNK;
                        if epcoeff[i] != 0 {
                            cst = CoeffState::ACTIVE;
                        } else if (pcoeff[i] as i32).abs() >= thres {
                            cst |= CoeffState::NEW;
                        }
                        cstate_slice[i] = cst;
                        bstatetmp |= cst;
                    }
                }
                bucket_states[buckno] = bstatetmp;
                bbstate |= bstatetmp;
            }
        } else {
            // Band zero
            let pcoeff = block.get_bucket(0).unwrap_or(&[0; 16]);
            let epcoeff = eblock.get_bucket(0).unwrap_or(&[0; 16]);
            let cstate_slice = &mut coeff_state[0..16];

            for i in 0..16 {
                let thres = quant_lo[i];
                if !cstate_slice[i].contains(CoeffState::ZERO) {
                    let mut cst = CoeffState::UNK;
                    if epcoeff[i] != 0 {
                        cst = CoeffState::ACTIVE;
                    } else if (pcoeff[i] as i32).abs() >= thres {
                        cst |= CoeffState::NEW;
                    }
                    cstate_slice[i] = cst;
                    bbstate |= cst;
                }
            }
            bucket_states[0] = bbstate;
        }
        bbstate
    }

    /// Encodes a sequence of buckets in a given block.
    fn encode_buckets<W: std::io::Write>(
        &mut self,
        zp: &mut ZpEncoder<W>,
        block_idx: usize,
        coeff_state: &mut [CoeffState],
        fbucket: usize,
        nbucket: usize,
        neighbor_active: &[bool], // Pre-computed neighbor activity for this block
    ) -> Result<(), crate::encode::zp::ZpCodecError> {
        let band = self.cur_band;
        let block = &self.map.blocks[block_idx];
        // Only borrow emap mutably for eblock, drop as soon as possible
        let eblock_ptr: *mut _ = &mut self.emap.blocks[block_idx];
        let eblock = unsafe { &mut *eblock_ptr };

        let mut bucket_states = [CoeffState::empty(); 16];
        let mut bbstate = Codec::prepare_bucket_states(
            &self.quant_hi,
            &self.quant_lo,
            block,
            eblock,
            band,
            fbucket,
            nbucket,
            coeff_state,
            &mut bucket_states,
        );

        // Code root bit
        let has_new = bbstate.contains(CoeffState::NEW);
        if nbucket < 16 || bbstate.contains(CoeffState::ACTIVE) {
            bbstate |= CoeffState::NEW;
        } else if bbstate.contains(CoeffState::UNK) {
            zp.encode(has_new, &mut self.ctx_root)?;
        }

        // Code bucket bits
        if bbstate.contains(CoeffState::NEW) {
            for buckno in 0..nbucket {
                if bucket_states[buckno].contains(CoeffState::UNK) {
                    // Calculate context properly based on activity
                    let parent_active = bbstate.contains(CoeffState::ACTIVE);
                    let neighbors_active = neighbor_active[fbucket + buckno];
                    let ctx_idx = (parent_active as usize) * 4 + (neighbors_active as usize) * 2;

                    zp.encode(
                        bucket_states[buckno].contains(CoeffState::NEW),
                        &mut self.ctx_bucket[band][ctx_idx.min(7)],
                    )?;
                }
            }
        }

        // Code new active coefficients (and their sign)
        if bbstate.contains(CoeffState::NEW) {
            let mut thres = self.quant_hi[band];
            for buckno in 0..nbucket {
                if bucket_states[buckno].contains(CoeffState::NEW) {
                    let cstate_slice = &coeff_state[buckno * 16..(buckno + 1) * 16];
                    let pcoeff_opt = block.get_bucket((fbucket + buckno) as u8);

                    if pcoeff_opt.is_none() {
                        continue;
                    }
                    let pcoeff = pcoeff_opt.unwrap();
                    let epcoeff = eblock.get_bucket_mut((fbucket + buckno) as u8);
                    for i in 0..16 {
                        if cstate_slice[i].contains(CoeffState::UNK) {
                            // Calculate context based on coefficient activity
                            let ctx_idx = {
                                let parent_active = bucket_states[buckno].contains(CoeffState::ACTIVE);
                                let coeff_pos_ctx = if band > 0 {
                                    // Higher bands: context from parent activity and position
                                    (i / 4) * 2
                                } else {
                                    // Band zero: more complex positional context
                                    (i & 3) + if i < 4 { 0 } else { 4 }
                                };
                                (parent_active as usize) * 8 + coeff_pos_ctx
                            };

                            let is_new = cstate_slice[i].contains(CoeffState::NEW);
                            zp.encode(is_new, &mut self.ctx_start[ctx_idx.min(31)])?;

                            if is_new {
                                zp.encode(pcoeff[i] < 0, &mut self.ctx_start[ctx_idx.min(31)])?;
                                if band == 0 {
                                    thres = self.quant_lo[i];
                                }
                                epcoeff[i] = (thres + (thres >> 1)) as i16;
                            }
                        }
                    }
                }
            }
        }

        // Code mantissa bits
        if bbstate.contains(CoeffState::ACTIVE) {
            let mut thres = self.quant_hi[band];
            for buckno in 0..nbucket {
                if bucket_states[buckno].contains(CoeffState::ACTIVE) {
                    let cstate_slice = &coeff_state[buckno * 16..(buckno + 1) * 16];
                    let pcoeff_opt = block.get_bucket((fbucket + buckno) as u8);

                    if pcoeff_opt.is_none() {
                        continue;
                    }
                    let pcoeff = pcoeff_opt.unwrap();
                    let epcoeff = eblock.get_bucket_mut((fbucket + buckno) as u8);
                    for i in 0..16 {
                        if cstate_slice[i].contains(CoeffState::ACTIVE) {
                            let coeff_abs = (pcoeff[i] as i32).abs();
                            let ecoeff = epcoeff[i] as i32;
                            if band == 0 {
                                thres = self.quant_lo[i];
                            }

                            let mut val = coeff_abs - ecoeff;
                            let mut quant = thres;
                            while quant > 0 && val >= quant {
                                zp.encode(true, &mut self.ctx_mant)?;
                                val -= quant;
                                quant >>= 1;
                            }
                            if val > 0 {
                                zp.encode(true, &mut self.ctx_mant)?;
                            } else {
                                zp.encode(false, &mut self.ctx_mant)?;
                            }

                            epcoeff[i] = (ecoeff + val + (quant >> 1)) as i16;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Estimate encoding error in decibels.
    pub fn estimate_decibel(&self, frac: f32) -> f32 {
        let mut norm_lo = [0.0; 16];
        let mut norm_hi = [0.0; 10];
        norm_lo.copy_from_slice(&IW_NORM);
        norm_hi[1..].copy_from_slice(&IW_NORM[1..10]);

        let mut mse_per_block: Vec<f32> = (0..self.map.num_blocks)
            .map(|block_idx| {
                let mut mse = 0.0;
                let block = &self.map.blocks[block_idx];
                let eblock = &self.emap.blocks[block_idx];

                for bandno in 0..10 {
                    let fbucket = BAND_BUCKETS[bandno].start;
                    let nbucket = BAND_BUCKETS[bandno].size;
                    let mut norm = norm_hi[bandno];

                    for buckno in 0..nbucket {
                        let pcoeff_opt = block.get_bucket((fbucket + buckno) as u8);
                        let epcoeff_opt = eblock.get_bucket((fbucket + buckno) as u8);

                        if let Some(pcoeff) = pcoeff_opt {
                            if let Some(epcoeff) = epcoeff_opt {
                                for i in 0..16 {
                                    if bandno == 0 {
                                        norm = norm_lo[i];
                                    }
                                    let delta = (pcoeff[i] as f32).abs() - epcoeff[i] as f32;
                                    mse += norm * delta * delta;
                                }
                            } else {
                                for i in 0..16 {
                                    if bandno == 0 {
                                        norm = norm_lo[i];
                                    }
                                    let delta = pcoeff[i] as f32;
                                    mse += norm * delta * delta;
                                }
                            }
                        }
                    }
                }
                mse / 1024.0
            })
            .collect();

        // Compute partition point
        mse_per_block.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let m = self.map.num_blocks - 1;
        let p = ((m as f32) * (1.0 - frac) + 0.5) as usize;
        let p = p.clamp(0, m);

        let avg_mse: f32 =
            mse_per_block[p..].iter().sum::<f32>() / ((self.map.num_blocks - p) as f32);

        if avg_mse <= 0.0 {
            return 99.9;
        }

        let factor = 255.0 * (1 << IW_SHIFT) as f32;
        10.0 * (factor * factor / avg_mse).log10()
    }
}
