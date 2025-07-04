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
    pub quant_hi: [i32; 10],
    pub quant_lo: [i32; 16],

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

        for (dst, src) in quant_lo.iter_mut().zip(IW_QUANT.iter()) {
        *dst = *src >> IW_SHIFT; // Scale only by IW_SHIFT
        }
        for (dst, src) in quant_hi[1..].iter_mut().zip(IW_QUANT[1..10].iter()) {
        *dst = *src >> IW_SHIFT; // Scale only by IW_SHIFT
        }

        // Find maximum coefficient value to determine starting bit-plane
        let mut max_coeff = 0i32;
        for block in &map.blocks {
            for bucket_idx in 0..64 {
                if let Some(bucket) = block.get_bucket(bucket_idx) {
                    for &coeff in bucket {
                        max_coeff = max_coeff.max((coeff as i32).abs());
                    }
                }
            }
        }

        // Start from the highest bit-plane that contains information
        // For 8-bit images with IW_SHIFT=6, coefficients can be ~2^13, so start from bit 13
        let cur_bit = if max_coeff > 0 {
            (max_coeff as f32).log2().floor() as i32
        } else {
            1 // Fallback for empty images
        };

        let emap = CoeffMap::new(map.iw, map.ih);

        Self {
            map,
            emap,
            cur_band: 0,
            cur_bit, // Start from the highest significant bit
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

        // Debug: Print quantization info
        if self.slice_count <= 5 {
            println!(
                "DEBUG IW44: Slice {}, band={}, bit={}",
                self.slice_count, self.cur_band, self.cur_bit
            );
            if self.cur_band == 0 {
                println!("DEBUG IW44: Band 0 quant_lo: {:?}", &self.quant_lo[..4]);
            } else {
                println!(
                    "DEBUG IW44: Band {} quant_hi: {}",
                    self.cur_band, self.quant_hi[self.cur_band]
                );
            }
        }

        // First, do the detailed null check WITHOUT computing neighbor activity
        if self.is_null_slice(self.cur_band, self.cur_bit, &mut coeff_state) {
            if self.slice_count <= 5 {
                println!("DEBUG IW44: Slice {} is NULL, finishing", self.slice_count);
            }
            let more = self.finish_slice();
            return Ok(more);
        }

        if self.slice_count <= 5 {
            println!(
                "DEBUG IW44: Slice {} is NOT NULL, encoding data",
                self.slice_count
            );
        }

        // Calculate block layout dimensions
        let blocks_w = self.map.bw / 32; // 32 pixels per block
        let blocks_h = self.map.bh / 32;
        
        // Create neighbor activity array: [block][bucket] -> bool
        let mut neighbor_active = vec![vec![false; 64]; self.map.num_blocks];

        // Only if slice is NOT null, compute expensive neighbor activity
        for blk in 0..self.map.num_blocks {
            let bx = blk % blocks_w;
            let by = blk / blocks_w;
            for bucket in 0..64 {
                let mut active = false;
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        if dx == 0 && dy == 0 {
                            continue;
                        }
                        if bx as isize + dx < 0 || by as isize + dy < 0 {
                            continue;
                        }
                        let nb = (by as isize + dy) * blocks_w as isize + (bx as isize + dx);
                        if nb < 0 || nb as usize >= self.map.num_blocks {
                            continue;
                        }
                        active |= self.emap.blocks[nb as usize]
                            .get_bucket(bucket as u8)
                            .map_or(false, |b| b.iter().any(|&c| c != 0));
                        if active {
                            break;
                        }
                    }
                    if active {
                        break;
                    }
                }
                neighbor_active[blk][bucket] = active;
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

        // Return whether more slices are available (don't advance state here)
        // State advancement is now handled by the encoder
        Ok(self.cur_bit >= 0)
    }

    fn finish_slice(&mut self) -> bool {
        // Reduce quantization threshold for next round (divide by √2 ≈ 1.414)
        // This follows the IW44 spec: threshold decay should be more gradual than divide-by-2
        self.quant_hi[self.cur_band] = (self.quant_hi[self.cur_band] as f32 / 1.414) as i32;
        if self.cur_band == 0 {
            for q in self.quant_lo.iter_mut() {
                *q = (*q as f32 / 1.414) as i32;
            }
        }

        self.cur_band += 1;
        if self.cur_band >= BAND_BUCKETS.len() {
            self.cur_band = 0;
            self.cur_bit -= 1; // Decrement bit-plane after completing all bands

            // Debug: Print bit-plane transitions
            if self.slice_count <= 20 || self.cur_bit <= 2 {
                println!(
                    "DEBUG IW44: Completed all bands for bit-plane {}, moving to bit-plane {}",
                    self.cur_bit + 1,
                    self.cur_bit
                );
            }

            // Stop when we reach bit-plane -1 (have processed bit-plane 0)
            if self.cur_bit < 0 {
                return false;
            }

            // Also check if all quantization thresholds are 0 (alternative stop condition)
            let all_zero =
                self.quant_hi.iter().all(|&q| q == 0) && self.quant_lo.iter().all(|&q| q == 0);
            if all_zero {
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

            // Debug: Show band 0 coefficients (DC and low frequency)
            let significant_coeffs: Vec<(usize, i16, i32)> = (0..16)
                .filter_map(|i| {
                    let thres = quant_lo[i];
                    let coeff = pcoeff[i];
                    if (coeff as i32).abs() >= thres {
                        Some((i, coeff, thres))
                    } else {
                        None
                    }
                })
                .collect();

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
                            let is_new = cstate_slice[i].contains(CoeffState::NEW);

                            // Use a dedicated context for coefficient activation
                            let ctx_idx = if band == 0 {
                                i.min(15) // Band 0: use coefficient index as context
                            } else {
                                band.min(15) // Other bands: use band number as context
                            };

                            zp.encode(is_new, &mut self.ctx_start[ctx_idx])?;

                            if is_new {
                                // Encode the residual sign bit (XOR of original and predicted)
                                let residual_sign = (pcoeff[i] ^ epcoeff[i]) < 0;
                                zp.encode(residual_sign, &mut self.ctx_mant)?;

                                // Get the step size for this coefficient
                                let step_size = if band == 0 {
                                    self.quant_lo[i]
                                } else {
                                    self.quant_hi[band]
                                };

                                // C++ code: epcoeff[i] = thres + (thres >> 1);
                                // This sets the encoded value to 1.5 * step_size
                                let initial_val = step_size + (step_size >> 1);
                                epcoeff[i] = if pcoeff[i] < 0 {
                                    -(initial_val as i16)
                                } else {
                                    initial_val as i16
                                };
                            }
                        }
                    }
                }
            }
        }

        // Code mantissa bits (for already active coefficients)
        if bbstate.contains(CoeffState::ACTIVE) {
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
                            let step_size = if band == 0 {
                                self.quant_lo[i]
                            } else {
                                self.quant_hi[band]
                            };

                            if step_size > 0 {
                                let coeff_abs = (pcoeff[i] as i32).abs();
                                let ecoeff_abs = (epcoeff[i] as i32).abs();

                                // C++ logic: pix = (coeff >= ecoeff) ? 1 : 0
                                let should_increase = coeff_abs >= ecoeff_abs;

                                // Encode mantissa bit based on threshold
                                if ecoeff_abs <= 3 * step_size {
                                    zp.encode(should_increase, &mut self.ctx_mant)?;
                                } else {
                                    // Use IW encoder for higher values (raw bit)
                                    zp.encode(should_increase, &mut self.ctx_mant)?;
                                }

                                // C++ adjustment: epcoeff[i] = ecoeff - (pix ? 0 : thres) + (thres >> 1)
                                let adjustment = if should_increase { 0 } else { step_size };
                                let new_abs = ecoeff_abs - adjustment + (step_size >> 1);

                                epcoeff[i] = if pcoeff[i] < 0 {
                                    -(new_abs as i16)
                                } else {
                                    new_abs as i16
                                };
                            }
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
