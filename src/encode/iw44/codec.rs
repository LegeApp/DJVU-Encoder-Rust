// src/encode/iw44/codec.rs

use crate::encode::iw44::coeff_map::{CoeffMap, Block};
use crate::encode::iw44::constants::{BAND_BUCKETS, IW_QUANT};
use crate::encode::zc::ZEncoder;

use crate::encode::iw44::EncoderParams;
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
    pub fn new(map: CoeffMap, params: &EncoderParams) -> Self {
        let (iw, ih) = (map.iw, map.ih);

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

        // TODO: Correctly apply quality settings to quantization tables.
        // For now, use the default tables to avoid over-quantization.
        let quality_factor = 1.0; // Placeholder

        let mut quant_lo = [0i32; 16];
        let mut quant_hi = [0i32; 10];

        for i in 0..16 {
            quant_lo[i] = IW_QUANT[i] >> 14;
        }

        quant_hi[0] = quant_lo[0];
        quant_hi[1] = quant_lo[1];
        quant_hi[2] = quant_lo[3];
        quant_hi[3] = quant_lo[3];
        quant_hi[4] = quant_lo[6];
        quant_hi[5] = quant_lo[6];
        quant_hi[6] = quant_lo[6];
        quant_hi[7] = quant_lo[12];
        quant_hi[8] = quant_lo[12];
        quant_hi[9] = quant_lo[12];

        // Determine starting bit-plane based on max coefficient.
        // Use ilog2 to find the most significant bit.
        // Use max(1) to handle case where max_coeff is 0, preventing panic.
        let cur_bit = max_coeff.max(1).ilog2() as i32;

        // Debug quantization thresholds
        println!(
            "DEBUG Codec quantization thresholds (quality: {:?}, factor: {:.2}):",
            params.decibels,
            quality_factor
        );
        println!("  Max coefficient: {}", max_coeff);
        println!("  Starting bit-plane: {}", cur_bit);
        println!("  quant_lo (band 0): {:?}", quant_lo);
        println!("  quant_hi (bands 1-9): {:?}", quant_hi);

        Codec {
            emap: CoeffMap::new(iw, ih),
            map,
            cur_band: 0,
            cur_bit,
            quant_hi,
            quant_lo,
            coeff_state: [UNK; 256],
            bucket_state: [0; 16],
            ctx_start: [128; 32],
            ctx_bucket: [[128; 8]; 10],
            ctx_mant: 128,
            ctx_root: 128,
        }
    }


    /// Encode a single slice (current band at current bit-plane)
    pub fn encode_slice<W: Write>(&mut self, zp: &mut ZEncoder<W>) -> Result<bool> {
        if self.cur_bit < 0 {
            return Ok(false); // No more bit-planes to process
        }

        // Check if this slice contains any significant data
        let is_null = self.is_null_slice(self.cur_bit as usize, self.cur_band);
        
        // Debug current encoding state
        let threshold = if self.cur_band == 0 {
            self.quant_lo[0] << self.cur_bit
        } else {
            self.quant_hi[self.cur_band] << self.cur_bit
        };
        
        println!("DEBUG Encode slice: band={}, bit={}, threshold={}, is_null={}", 
                 self.cur_band, self.cur_bit, threshold, is_null);
        
        // If slice is null, we can advance state and continue
        if is_null {
            println!("  Slice is null, advancing to next");
            let _has_more = self.finish_code_slice()?;
            // Return false to indicate no data was encoded in this slice
            return Ok(false);
        }

        // Count active coefficients for debugging
        let _active_coeffs = 0;
        let _encoded_bits = 0;
        
        for blockno in 0..self.map.num_blocks {
            let bucket_info = BAND_BUCKETS[self.cur_band];
            
            // Debug first block's coefficient distribution
            if blockno == 0 {
                let input_block = &self.map.blocks[blockno];
                let mut coeff_count = 0;
                let mut max_coeff = 0i16;
                
                for bucket_idx in bucket_info.start..(bucket_info.start + bucket_info.size) {
                    if let Some(bucket) = input_block.get_bucket(bucket_idx as u8) {
                        for &coeff in bucket {
                            if coeff != 0 {
                                coeff_count += 1;
                                // Use safe abs to handle overflow of i16::MIN
                                let abs_coeff = if coeff == i16::MIN {
                                    32767i16 // Clamp to max positive i16
                                } else {
                                    coeff.abs()
                                };
                                max_coeff = max_coeff.max(abs_coeff);
                            }
                        }
                    }
                }
                if blockno == 0 {
                    println!("  Block 0: {} non-zero coeffs, max magnitude: {}", coeff_count, max_coeff);
                }
            }
            
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

        // Always advance to next band/bit-plane
        let has_more = self.finish_code_slice()?;
        
        // Return true if we have more to process
        Ok(has_more)
    }

    /// Check if the current slice is null (no significant coefficients)
    /// According to DjVu spec: a coefficient becomes active when |coeff| >= 2*step_size
    /// The step size at bit-plane k is: step_size = initial_step_size / 2^k
    fn is_null_slice(&mut self, bit: usize, band: usize) -> bool {
        if band == 0 {
            // For the DC band (band 0), we check all 16 of its sub-bands.
            let mut is_slice_null = true;
            for i in 0..16 {
                // The activation threshold is 2 * initial_step_size. We compare it against
                // the coefficient magnitude, both scaled by the current bit-plane.
                // This is equivalent to `|coeff| >= 2 * (initial_step_size / 2^bit)`
                // but avoids floating point or integer division issues.
                let activation_threshold = self.quant_lo[i] << 1;
                self.coeff_state[i] = ZERO;

                let mut has_significant_coeff = false;
                for blockno in 0..self.map.num_blocks {
                    if let Some(bucket) = self.map.blocks[blockno].get_bucket(i as u8) {
                        for &coeff in bucket {
                            if ((coeff as i32).abs() << bit) >= activation_threshold {
                                has_significant_coeff = true;
                                break;
                            }
                        }
                        if has_significant_coeff {
                            break;
                        }
                    }
                }

                if has_significant_coeff {
                    self.coeff_state[i] = UNK;
                    is_slice_null = false;
                }
            }
            is_slice_null
        } else {
            // For AC bands, the logic is simpler. We just need to find one significant
            // coefficient in the entire band to consider the slice not null.
            let activation_threshold = self.quant_hi[band] << 1;
            let bucket_info = BAND_BUCKETS[band];

            for blockno in 0..self.map.num_blocks {
                for bucket_idx in bucket_info.start..(bucket_info.start + bucket_info.size) {
                    if let Some(bucket) = self.map.blocks[blockno].get_bucket(bucket_idx as u8) {
                        for &coeff in bucket {
                            if ((coeff as i32).abs() << bit) >= activation_threshold {
                                return false; // Found significant coefficient, slice is not null.
                            }
                        }
                    }
                }
            }
            true // No significant coefficients found, slice is null.
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
                    let step_size = if band == 0 {
                        quant_lo[i] >> bit
                    } else {
                        quant_hi[band] >> bit
                    };

                    // According to DjVu spec: coefficient becomes active when |coeff| >= 2*step_size
                    let activation_threshold = 2 * step_size;
                    let coeff_abs = (coeff as i32).abs();
                    
                    if step_size >= 1 && coeff_abs >= activation_threshold {
                        // Encode that coefficient becomes significant
                        zp.encode(true, ctx_root)?;
                        
                        // Encode sign
                        zp.encode(coeff < 0, ctx_root)?;
                        
                        // Set initial reconstructed value at this bit-plane
                        // According to DjVu spec, coefficients are fixed-point with 6 fractional bits
                        // Initial value should be step_size * 1.5 * 64 to account for fractional bits
                        let sign = if coeff < 0 { -1 } else { 1 };
                        let initial_value = step_size + (step_size >> 1); // 1.5 * step_size
                        let scaled_value = (initial_value << 6) as i32; // Scale by 2^6
                        ecoeffs[i] = (sign * scaled_value) as i16;
                        
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
                    
                    // Fixed: Correct handling of negative coefficients with fractional bits
                    let prev_val = ecoeffs[i];
                    if prev_val == 0 {
                        // This shouldn't happen for ACTIVE coefficients
                        continue;
                    }
                    
                    // Calculate step size for refinement
                    let refine_step_size = if band == 0 {
                        quant_lo[i] >> bit
                    } else {
                        quant_hi[band] >> bit
                    };
                    
                    let sign = if prev_val < 0 { -1i16 } else { 1i16 };
                    // Use safe abs to handle overflow of i16::MIN
                    let abs_val = if prev_val == i16::MIN {
                        32767u16 // Clamp to max positive value that fits in u16
                    } else {
                        prev_val.abs() as u16
                    };
                    
                    // Update coefficient with refined bit
                    // The step size needs to be scaled by 64 for fractional bits
                    let step_adjustment = ((refine_step_size >> 1) << 6) as u16; // (step_size/2) * 64
                    let new_abs = if bit_val != 0 {
                        abs_val + step_adjustment
                    } else {
                        abs_val.saturating_sub(step_adjustment)
                    };
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
            
            // Calculate step size and activation threshold for this bucket/band
            let step_size = if band == 0 {
                if buckno < 16 {
                    quant_lo[buckno] >> cur_bit
                } else {
                    0 // Invalid bucket for band 0
                }
            } else {
                quant_hi[band] >> cur_bit
            };
            
            // According to DjVu spec: coefficient becomes active when |coeff| >= 2*step_size
            let activation_threshold = 2 * step_size;

            match (pcoeff, epcoeff) {
                (Some(pc), Some(epc)) => {
                    for i in 0..16 {
                        let cstate_idx = buckno * 16 + i;
                        if cstate_idx < coeff_state.len() {
                            let mut cstatetmp = ZERO;
                            
                            if epc[i] != 0 {
                                // Already active from previous bit-plane
                                cstatetmp = ACTIVE;
                            } else if step_size >= 1 && (pc[i] as i32).abs() >= activation_threshold {
                                // Could become significant at this bit-plane
                                cstatetmp = NEW | UNK;
                            } else if step_size >= 1 {
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
                            
                            if step_size >= 1 && (pc[i] as i32).abs() >= activation_threshold {
                                // Could become significant at this bit-plane
                                cstatetmp = NEW | UNK;
                            } else if step_size >= 1 {
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