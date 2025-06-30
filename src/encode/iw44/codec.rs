// src/iw44/codec.rs
use super::coeff_map::{Block, CoeffMap};
use super::constants::{BAND_BUCKETS, IW_NORM, IW_QUANT};
use crate::encode::zp::ZPCodec;
use crate::image::coefficients::IW_SHIFT;
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
        
        // Initialize quantization tables from constants
        quant_lo.copy_from_slice(&IW_QUANT[..]);
        quant_hi[1..].copy_from_slice(&IW_QUANT[1..10]);

        let emap = CoeffMap::new(map.iw, map.ih);

        Self {
            map,
            emap,
            cur_band: 0,
            cur_bit: 1, // C++ starts at 1
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
    
    /// The main entry point to encode one "slice" of data.
    pub fn encode_slice(&mut self, zp: &mut ZPCodec) -> bool {
        if self.cur_bit < 0 { return false; }
        
        let mut coeff_state = [CoeffState::empty(); 256];
        if !self.is_null_slice(self.cur_band, self.cur_bit, &mut coeff_state) {
            for block_idx in 0..self.map.num_blocks {
                let fbucket = BAND_BUCKETS[self.cur_band].start;
                let nbucket = BAND_BUCKETS[self.cur_band].size;
                self.encode_buckets(zp, block_idx, &mut coeff_state, fbucket, nbucket);
            }
        }

        self.finish_slice()
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
            // Check if we are done
            if self.quant_hi.iter().all(|&q| q == 0) {
                self.cur_bit = -1;
                return false;
            }
        }
        true
    }

    /// Prepares states for a set of buckets within a block.
    fn prepare_bucket_states(
        &self,
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
            let thres = self.quant_hi[band];
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
                let thres = self.quant_lo[i];
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
    fn encode_buckets(
        &mut self,
        zp: &mut ZPCodec,
        block_idx: usize,
        coeff_state: &mut [CoeffState],
        fbucket: usize,
        nbucket: usize,
    ) {
        let block = &self.map.blocks[block_idx];
        let eblock = &mut self.emap.blocks[block_idx];
        let band = self.cur_band;

        let mut bucket_states = [CoeffState::empty(); 16];
        let mut bbstate = self.prepare_bucket_states(block, eblock, band, fbucket, nbucket, coeff_state, &mut bucket_states);
        
        // Code root bit
        let has_new = bbstate.contains(CoeffState::NEW);
        if nbucket < 16 || bbstate.contains(CoeffState::ACTIVE) {
            bbstate |= CoeffState::NEW;
        } else if bbstate.contains(CoeffState::UNK) {
            zp.encoder(has_new, &mut self.ctx_root);
        }
        
        // Code bucket bits
        if bbstate.contains(CoeffState::NEW) {
            for buckno in 0..nbucket {
                if bucket_states[buckno].contains(CoeffState::UNK) {
                    // TODO: Implement full context calculation from C++
                    let ctx_idx = if bbstate.contains(CoeffState::ACTIVE) { 4 } else { 0 };
                    zp.encoder(
                        bucket_states[buckno].contains(CoeffState::NEW), 
                        &mut self.ctx_bucket[band][ctx_idx]
                    );
                }
            }
        }
        
        // Code new active coefficients (and their sign)
        if bbstate.contains(CoeffState::NEW) {
            let mut thres = self.quant_hi[band];
            for buckno in 0..nbucket {
                if bucket_states[buckno].contains(CoeffState::NEW) {
                    let cstate_slice = &coeff_state[buckno * 16..(buckno+1)*16];
                    let pcoeff_opt = block.get_bucket((fbucket + buckno) as u8);
                    
                    if pcoeff_opt.is_none() { continue; }
                    let pcoeff = pcoeff_opt.unwrap();
                    let epcoeff = eblock.get_bucket_mut((fbucket + buckno) as u8);

                    for i in 0..16 {
                        if cstate_slice[i].contains(CoeffState::UNK) {
                            // TODO: Implement full context calc
                            let ctx_idx = if bucket_states[buckno].contains(CoeffState::ACTIVE) { 8 } else { 0 };
                            let is_new = cstate_slice[i].contains(CoeffState::NEW);
                            zp.encoder(is_new, &mut self.ctx_start[ctx_idx]);
                            
                            if is_new {
                                zp.iw_encoder(pcoeff[i] < 0);
                                if band == 0 { thres = self.quant_lo[i]; }
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
                    let cstate_slice = &coeff_state[buckno * 16..(buckno+1)*16];
                     let pcoeff_opt = block.get_bucket((fbucket + buckno) as u8);
                    
                    if pcoeff_opt.is_none() { continue; }
                    let pcoeff = pcoeff_opt.unwrap();
                    let epcoeff = eblock.get_bucket_mut((fbucket + buckno) as u8);

                    for i in 0..16 {
                        if cstate_slice[i].contains(CoeffState::ACTIVE) {
                            let coeff_abs = (pcoeff[i] as i32).abs();
                            let ecoeff = epcoeff[i] as i32;
                            if band == 0 { thres = self.quant_lo[i]; }
                            
                            let pix = coeff_abs >= ecoeff;
                            
                            if ecoeff <= 3 * thres {
                                zp.encoder(pix, &mut self.ctx_mant);
                            } else {
                                zp.iw_encoder(pix);
                            }
                            
                            epcoeff[i] = (ecoeff - (if pix { 0 } else { thres }) + (thres >> 1)) as i16;
                        }
                    }
                }
            }
        }
    }

    /// Estimate encoding error in decibels.
    pub fn estimate_decibel(&self, frac: f32) -> f32 {
        let mut norm_lo = [0.0; 16];
        let mut norm_hi = [0.0; 10];
        norm_lo.copy_from_slice(&IW_NORM);
        norm_hi[1..].copy_from_slice(&IW_NORM[1..10]);
        
        let mut mse_per_block: Vec<f32> = (0..self.map.num_blocks).map(|block_idx| {
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
                                if bandno == 0 { norm = norm_lo[i]; }
                                let delta = (pcoeff[i] as f32).abs() - epcoeff[i] as f32;
                                mse += norm * delta * delta;
                            }
                        } else {
                            for i in 0..16 {
                                if bandno == 0 { norm = norm_lo[i]; }
                                let delta = pcoeff[i] as f32;
                                mse += norm * delta * delta;
                            }
                        }
                    }
                }
            }
            mse / 1024.0
        }).collect();
        
        // Compute partition point
        mse_per_block.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let m = self.map.num_blocks -1;
        let p = ((m as f32) * (1.0 - frac) + 0.5) as usize;
        let p = p.clamp(0, m);

        let avg_mse: f32 = mse_per_block[p..].iter().sum::<f32>() / ((self.map.num_blocks - p) as f32);
        
        if avg_mse <= 0.0 { return 99.9; }

        let factor = 255.0 * (1 << IW_SHIFT) as f32;
        10.0 * (factor * factor / avg_mse).log10()
    }
}