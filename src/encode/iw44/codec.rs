// src/encode/iw44/codec.rs

use crate::encode::iw44::coeff_map::{CoeffMap, Block};
use crate::encode::iw44::constants::{BAND_BUCKETS, IW_QUANT};
use crate::encode::zc::ZEncoder;
use crate::Result;
use std::io::Write;

// Coefficient states
pub const ZERO: u8 = 1;
pub const ACTIVE: u8 = 2;
pub const NEW: u8 = 4;
pub const UNK: u8 = 8;

pub struct Codec {
    pub map: CoeffMap,        // Input coefficients
    pub emap: CoeffMap,       // Encoded coefficients
    pub cur_band: usize,      // Current band index
    pub cur_bit: i32,         // Current bit-plane (decrements)
    pub quant_hi: [i32; 10],  // High-frequency quantization thresholds
    pub quant_lo: [i32; 16],  // Low-frequency quantization thresholds
    coeff_state: [u8; 256],   // Coefficient states per block
    bucket_state: [u8; 16],   // Bucket states
    ctx_start: [u8; 32],      // Context for Z-Encoder
    ctx_bucket: [[u8; 8]; 10], // Bucket contexts
    ctx_mant: u8,             // Mantissa context
    ctx_root: u8,             // Root context
}

impl Codec {
    /// Initialize a new Codec instance for a given coefficient map
    pub fn new(map: CoeffMap) -> Self {
        let (iw, ih) = (map.iw, map.ih);
        let mut codec = Codec {
            emap: CoeffMap::new(iw, ih),
            map,
            cur_band: 0,
            cur_bit: 15, // Start at most significant bit-plane (16-bit coeffs)
            quant_hi: [0; 10],
            quant_lo: [0; 16],
            coeff_state: [0; 256],
            bucket_state: [0; 16],
            ctx_start: [0; 32],
            ctx_bucket: [[0; 8]; 10],
            ctx_mant: 0,
            ctx_root: 0,
        };

        // Initialize quantization thresholds from IW_QUANT
        // quant_lo corresponds to step size table indices 0-15 (Table 3 in DjVu spec)
        // quant_hi corresponds to bands 1-9 (indices 7-15 in step size table)
        codec.quant_lo = IW_QUANT;  // Direct copy of all 16 step sizes
        
        // For high-frequency bands, use the appropriate step sizes
        // Band 0 uses IW_QUANT[0], bands 1-9 use indices 7-15
        codec.quant_hi[0] = IW_QUANT[0];  // Band 0 -> step size index 0
        for j in 1..10 {
            let step_size_idx = match j {
                1 => 7,   // Band 1 -> step size index 7
                2 => 8,   // Band 2 -> step size index 8
                3 => 9,   // Band 3 -> step size index 9
                4 => 10,  // Band 4 -> step size index 10
                5 => 11,  // Band 5 -> step size index 11
                6 => 12,  // Band 6 -> step size index 12
                7 => 13,  // Band 7 -> step size index 13
                8 => 14,  // Band 8 -> step size index 14
                9 => 15,  // Band 9 -> step size index 15
                _ => 15,  // Fallback
            };
            codec.quant_hi[j] = IW_QUANT[step_size_idx];
        }

        codec
    }

    /// Encode a single slice (current band at current bit-plane)
    pub fn encode_slice<W: Write>(&mut self, zp: &mut ZEncoder<W>) -> Result<bool> {
        if self.cur_bit < 0 {
            return Ok(false); // No more bit-planes to process
        }

        // Check if this slice contains any significant data
        let is_null = self.is_null_slice(self.cur_bit as usize, self.cur_band);
        
        if !is_null {
            for blockno in 0..self.map.num_blocks {
                let bucket_info = BAND_BUCKETS[self.cur_band];
                
                // Extract the blocks we need to avoid borrowing issues
                let input_block = &self.map.blocks[blockno];
                let output_block = &mut self.emap.blocks[blockno];
                
                // Call encode_buckets as a static function to avoid borrowing self
                Self::encode_buckets_static(
                    zp,
                    self.cur_bit as usize,
                    self.cur_band,
                    input_block,
                    output_block,
                    bucket_info.start,
                    bucket_info.size,
                    &mut self.coeff_state,
                    &mut self.bucket_state,
                    &mut self.ctx_start,
                    &mut self.ctx_bucket,
                    &mut self.ctx_root,
                    &mut self.ctx_mant,
                    &self.quant_lo,
                    &self.quant_hi,
                )?;
            }
        }

        // Always advance to next band/bit-plane
        let has_more = self.finish_code_slice()?;
        
        // Return true if we encoded data and have more to process
        Ok(!is_null && has_more)
    }

    /// Check if the current slice is null (no significant coefficients)
    fn is_null_slice(&mut self, bit: usize, band: usize) -> bool {
        if band == 0 {
            let mut is_null = true;
            for i in 0..16 {
                let threshold = self.quant_lo[i] >> bit;
                self.coeff_state[i] = ZERO;
                if threshold > 0 && threshold < 0x8000 {
                    self.coeff_state[i] = UNK;
                    is_null = false;
                }
            }
            is_null
        } else {
            let threshold = self.quant_hi[band] >> bit;
            !(threshold > 0 && threshold < 0x8000)
        }
    }

    /// Advance to the next band or bit-plane
    fn finish_code_slice(&mut self) -> Result<bool> {
        self.cur_band += 1;
        if self.cur_band >= BAND_BUCKETS.len() {
            self.cur_band = 0;
            self.cur_bit -= 1; // Decrement bit-plane after all bands
        }
        // Return true as long as we have more bit-planes to process
        Ok(self.cur_bit >= 0)
    }

    /// Encode buckets for a block in the current slice
    fn encode_buckets_static<W: Write>(
        zp: &mut ZEncoder<W>,
        bit: usize,
        band: usize,
        blk: &Block,
        eblk: &mut Block,
        fbucket: usize,
        nbucket: usize,
        coeff_state: &mut [u8; 256],
        bucket_state: &mut [u8; 16],
        ctx_start: &mut [u8; 32],
        ctx_bucket: &mut [[u8; 8]; 10],
        ctx_root: &mut u8,
        ctx_mant: &mut u8,
        quant_lo: &[i32; 16],
        quant_hi: &[i32; 10],
    ) -> Result<()> {
        let bbstate = Self::encode_prepare_static(
            band, fbucket, nbucket, blk, eblk, bit,
            coeff_state, bucket_state, quant_lo, quant_hi
        );
        if bbstate == 0 {
            return Ok(());
        }

        // Encode bucket-level decisions
        for buckno in 0..nbucket {
            let bstate = bucket_state[buckno];
            
            // Encode whether this bucket is active
            if (bstate & (NEW | ACTIVE)) != 0 {
                let ctx_idx = if band == 0 {
                    &mut ctx_start[buckno.min(31)]
                } else {
                    &mut ctx_bucket[(band - 1).min(9)][buckno.min(7)]
                };
                zp.encode(true, ctx_idx)?;

                // Encode coefficient-level data for active buckets
                // Pass relative bucket index to fix state indexing
                Self::encode_bucket_coeffs_static(
                    zp, bit, band, blk, eblk, fbucket + buckno, buckno,
                    coeff_state, ctx_root, ctx_mant, quant_lo, quant_hi
                )?;
            } else {
                // Bucket is inactive - encode "false" bit
                let ctx_idx = if band == 0 {
                    &mut ctx_start[buckno.min(31)]
                } else {
                    &mut ctx_bucket[(band - 1).min(9)][buckno.min(7)]
                };
                zp.encode(false, ctx_idx)?;
            }
        }

        Ok(())
    }

    /// Encode individual coefficients within a bucket
    fn encode_bucket_coeffs_static<W: Write>(
        zp: &mut ZEncoder<W>,
        bit: usize,
        band: usize,
        blk: &Block,
        eblk: &mut Block,
        bucket_idx: usize,
        relative_bucket_idx: usize,  // Added: relative bucket index within band
        coeff_state: &mut [u8; 256],
        ctx_root: &mut u8,
        ctx_mant: &mut u8,
        quant_lo: &[i32; 16],
        quant_hi: &[i32; 10],
    ) -> Result<()> {
        if let Some(coeffs) = blk.get_bucket(bucket_idx as u8) {
            let mut ecoeffs = eblk.get_bucket(bucket_idx as u8)
                .map(|prev| *prev)
                .unwrap_or([0; 16]);
            
            for (i, &coeff) in coeffs.iter().enumerate() {
                // Fixed: Use relative bucket index to prevent state collisions
                let cstate_idx = relative_bucket_idx * 16 + i;
                let cstate = if cstate_idx < coeff_state.len() {
                    coeff_state[cstate_idx]
                } else {
                    UNK
                };

                if (cstate & NEW) != 0 {
                    // New significant coefficient - encode activation decision
                    let threshold = if band == 0 {
                        quant_lo[i]
                    } else {
                        quant_hi[band]
                    } >> bit;

                    if (coeff as i32).abs() >= threshold {
                        // Encode that coefficient becomes significant
                        zp.encode(true, ctx_root)?;
                        
                        // Encode sign
                        zp.encode(coeff < 0, ctx_root)?;
                        
                        // Set initial reconstructed value at this bit-plane
                        let sign = if coeff < 0 { -1 } else { 1 };
                        ecoeffs[i] = (sign * (threshold + (threshold >> 1))) as i16;
                        
                        // Update state: NEW -> ACTIVE for next bit-plane
                        if cstate_idx < coeff_state.len() {
                            coeff_state[cstate_idx] = ACTIVE;
                        }
                    } else {
                        // Coefficient not significant at this bit-plane
                        zp.encode(false, ctx_root)?;
                        // Keep as NEW for lower bit-planes
                    }
                } else if (cstate & ACTIVE) != 0 {
                    // Refinement of already significant coefficient
                    let orig_abs = (coeff as i32).abs();
                    
                    // Check if current bit is set in original coefficient
                    let bit_val = (orig_abs >> bit) & 1;
                    zp.encode(bit_val != 0, ctx_mant)?;
                    
                    // Fixed: Correct handling of negative coefficients
                    let prev_val = ecoeffs[i];
                    if prev_val == 0 {
                        // This shouldn't happen for ACTIVE coefficients
                        continue;
                    }
                    
                    let sign = if prev_val < 0 { -1i16 } else { 1i16 };
                    let abs_val = prev_val.abs() as u16;
                    let new_abs = abs_val | ((bit_val as u16) << bit);
                    ecoeffs[i] = sign * (new_abs as i16);
                }
                // Note: ZERO coefficients are not encoded
            }
            
            eblk.set_bucket(bucket_idx as u8, ecoeffs);
        }

        Ok(())
    }

    /// Prepare states for encoding buckets
    fn encode_prepare_static(
        band: usize,
        fbucket: usize,
        nbucket: usize,
        blk: &Block,
        eblk: &Block,
        cur_bit: usize,
        coeff_state: &mut [u8; 256],
        bucket_state: &mut [u8; 16],
        quant_lo: &[i32; 16],
        quant_hi: &[i32; 10],
    ) -> u8 {
        let mut bbstate = 0;
        
        for buckno in 0..nbucket {
            let pcoeff = blk.get_bucket((fbucket + buckno) as u8);
            let epcoeff = eblk.get_bucket((fbucket + buckno) as u8);
            let mut bstatetmp = 0;
            
            // Calculate threshold for this bucket/band
            let thres = if band == 0 {
                if buckno < 16 {
                    quant_lo[buckno] >> cur_bit
                } else {
                    0 // Invalid bucket for band 0
                }
            } else {
                quant_hi[band] >> cur_bit
            };

            match (pcoeff, epcoeff) {
                (Some(pc), Some(epc)) => {
                    for i in 0..16 {
                        let cstate_idx = buckno * 16 + i;
                        if cstate_idx < coeff_state.len() {
                            let mut cstatetmp = ZERO;
                            
                            if epc[i] != 0 {
                                // Already active from previous bit-plane
                                cstatetmp = ACTIVE;
                            } else if (pc[i] as i32).abs() >= thres && thres > 0 {
                                // Could become significant at this bit-plane
                                cstatetmp = NEW | UNK;
                            } else if thres > 0 {
                                // Not significant yet, but could be at lower bit-planes
                                cstatetmp = UNK;
                            }
                            
                            coeff_state[cstate_idx] = cstatetmp;
                            bstatetmp |= cstatetmp;
                        }
                    }
                }
                (Some(pc), None) => {
                    for i in 0..16 {
                        let cstate_idx = buckno * 16 + i;
                        if cstate_idx < coeff_state.len() {
                            let mut cstatetmp = ZERO;
                            
                            if (pc[i] as i32).abs() >= thres && thres > 0 {
                                // Could become significant at this bit-plane
                                cstatetmp = NEW | UNK;
                            } else if thres > 0 {
                                // Not significant yet, but could be at lower bit-planes
                                cstatetmp = UNK;
                            }
                            
                            coeff_state[cstate_idx] = cstatetmp;
                            bstatetmp |= cstatetmp;
                        }
                    }
                }
                _ => bstatetmp = 0,
            }
            
            bucket_state[buckno] = bstatetmp;
            bbstate |= bstatetmp;
        }
        
        bbstate
    }
}