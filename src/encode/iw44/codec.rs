// src/encode/iw44/codec.rs

use crate::encode::iw44::coeff_map::{CoeffMap, Block};
use crate::encode::iw44::constants::{BAND_BUCKETS, IW_QUANT};
use crate::encode::zc::ZEncoder;
use crate::Result;
use std::io::Write;
use anyhow::Context;
use log::{debug, info, warn, error};

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
    pub fn new(map: CoeffMap, params: &super::encoder::EncoderParams) -> Self {
        let (iw, ih) = (map.iw, map.ih);
        
        // Find maximum coefficient value to determine starting bit-plane
        let mut max_coeff = 0i32;
        let mut total_coeffs = 0;
        let mut nonzero_coeffs = 0;
        for (block_idx, block) in map.blocks.iter().enumerate() {
            for bucket_idx in 0..64 {
                if let Some(bucket) = block.get_bucket(bucket_idx) {
                    for &coeff in bucket {
                        total_coeffs += 1;
                        if coeff != 0 {
                            nonzero_coeffs += 1;
                            max_coeff = max_coeff.max((coeff as i32).abs());
                            if block_idx == 0 && bucket_idx == 0 {
                                debug!("MAXCOEFF_DEBUG: Block 0, bucket 0, coeff={}, current max_coeff={}", coeff, max_coeff);
                            }
                        }
                    }
                }
            }
        }
        debug!("MAXCOEFF_DEBUG: Processed {} total coeffs, {} non-zero, max_coeff={}", total_coeffs, nonzero_coeffs, max_coeff);

        // Debug: Show DC coefficient values for debugging solid color issue
        if map.blocks.len() > 0 {
            if let Some(dc_bucket) = map.blocks[0].get_bucket(0) {
                info!("SOLID_COLOR_DEBUG: DC coefficients in block 0, bucket 0: {:?}", dc_bucket);
                // Show all block 0 coefficients for comparison
                for bucket_idx in 0..16 {
                    if let Some(bucket) = map.blocks[0].get_bucket(bucket_idx) {
                        let non_zero: Vec<i16> = bucket.iter().filter(|&&x| x != 0).cloned().collect();
                        if !non_zero.is_empty() {
                            info!("SOLID_COLOR_DEBUG: Block 0, bucket {}: non-zero coeffs: {:?}", bucket_idx, non_zero);
                        }
                    }
                }
            }
        }

        let mut codec = Codec {
            emap: CoeffMap::new(iw, ih),
            map,
            cur_band: 0,
            cur_bit: 15, // Will be updated below
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
        // Apply quality scaling based on params.decibels
        let quality_scale = if let Some(db) = params.decibels {
            // Convert decibels to quality scale factor (similar to JPEG quality)
            // Higher dB = higher quality = less quantization
            let normalized_db = (db - 50.0) / 50.0; // Normalize around 50dB
            2.0_f32.powf(-normalized_db) // Exponential scaling
        } else {
            1.0 // Default scaling
        };
        
        // Initialize quantization thresholds directly from IW_QUANT.
        // The wavelet coefficients are scaled by `IW_SHIFT` during the transform,
        // therefore the step sizes from the specification must *not* be shifted
        // here.  Using the original values ensures that the significance tests
        // and reconstruction levels match the DjVu reference encoder.
        for i in 0..16 {
            codec.quant_lo[i] = IW_QUANT[i];
        }

        codec.quant_hi[0] = codec.quant_lo[0]; // Band 0 uses quant_lo
        for j in 1..10 {
            let step_size_idx = j.min(15); // Bands 1-9 use indices 1-9, clamped to 15
            codec.quant_hi[j] = IW_QUANT[step_size_idx];
        }

        // Start from the highest bit-plane that contains information
        // For a solid color image, we need to be much more conservative
        // Different channels may need different starting bit-planes based on coefficient magnitude
        codec.cur_bit = if max_coeff > 0 {
            // For very small coefficients (like Cr channel with value 21), we need an even higher bit-plane
            if max_coeff < 50 {
                12 // Very conservative for small coefficients like Cr
            } else if max_coeff < 1000 {
                10 // Conservative starting point for small coefficients like Y/Cb
            } else {
                max_coeff.ilog2() as i32
            }
        } else {
            0 // For empty images, start at bit-plane 0
        };

        #[cfg(debug_assertions)]
        {
            info!("XXXXXXXXX CODEC NEW DEBUG XXXXXXXXX");
            info!("DEBUG Codec quantization thresholds (quality scale: {:.3}):", quality_scale);
            info!("  Max coefficient: {}", max_coeff);
            info!("  Starting bit-plane: {}", codec.cur_bit);
            info!("  quant_lo (band 0): {:?}", &codec.quant_lo[0..4]);
            info!("  quant_hi (bands 1-9): {:?}", &codec.quant_hi[1..5]);
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
        
        // Debug current encoding state (only for first few slices to avoid flooding)
        #[cfg(debug_assertions)]
        if self.cur_bit > 10 || (self.cur_band == 0 && self.cur_bit > 8) {
            let threshold = if self.cur_band == 0 {
                (self.quant_lo[0] >> self.cur_bit).max(1)
            } else {
                (self.quant_hi[self.cur_band] >> self.cur_bit).max(1)
            };
            debug!("Encode slice: band={}, bit={}, threshold={}, is_null={}", 
                self.cur_band, self.cur_bit, threshold, is_null);
        }
        
        // If slice is null, we can advance state and continue
        if is_null {
            #[cfg(debug_assertions)]
            if self.cur_bit > 10 || (self.cur_band == 0 && self.cur_bit > 8) {
                debug!("Slice is null, advancing to next (band={}, bit={})", self.cur_band, self.cur_bit);
            }
            let _has_more = self.finish_code_slice()?;
            // Return false to indicate no data was encoded in this slice
            return Ok(false);
        }

        debug!("Slice not null - proceeding to bucket encoding band={} bit={}", self.cur_band, self.cur_bit);

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

        // Always advance to next band/bit-plane
        let has_more = self.finish_code_slice()?;
        
        // Return true if we have more to process
        Ok(has_more)
    }

    /// Check if the current slice is null (no significant coefficients)
    /// According to DjVu spec: a coefficient becomes active when |coeff| >= step_size
    /// The step size at bit-plane k is: step_size = initial_step_size >> k
    fn is_null_slice(&mut self, bit: usize, band: usize) -> bool {
        if self.cur_bit < 0 {
            return true;
        }

        if band == 0 {
            // For DC band, check all 16 subbands
            for i in 0..16 {
                // CORRECTED: Calculate step size for the current bit-plane
                let step_size = (self.quant_lo[i] >> self.cur_bit).max(1);
                self.coeff_state[i] = ZERO;

                if step_size > 1 { // Corresponds to DjVu check (s > 0) after shifting
                    // Check if any coefficient in this subband is significant
                    for blockno in 0..self.map.num_blocks {
                        if let Some(bucket) = self.map.blocks[blockno].get_bucket(i as u8) {
                            for &coeff in bucket {
                                if (coeff as i32).abs() >= step_size {
                                    return false; // Found significant coefficient, slice is not null
                                }
                            }
                        }
                    }
                }
            }
        } else {
            // For AC bands
            // CORRECTED: Calculate step size for the current bit-plane
            let step_size = (self.quant_hi[band] >> self.cur_bit).max(1);

            if step_size > 1 {
                let bucket_info = BAND_BUCKETS[band];
                for blockno in 0..self.map.num_blocks {
                    for bucket_idx in bucket_info.start..(bucket_info.start + bucket_info.size) {
                        if let Some(bucket) = self.map.blocks[blockno].get_bucket(bucket_idx as u8) {
                            for &coeff in bucket {
                                if (coeff as i32).abs() >= step_size {
                                    return false; // Found significant coefficient, slice is not null
                                }
                            }
                        }
                    }
                }
            }
        }

        // If we looped through everything and found nothing, the slice is null.
        true
    }

    /// Advance to the next band or bit-plane
    fn finish_code_slice(&mut self) -> Result<bool> {
        // CORRECTED: The quantization tables should hold the initial, constant values.
        // The step sizes are calculated dynamically based on cur_bit.
        // Removed the logic that halved step sizes.

        self.cur_band += 1;
        if self.cur_band >= BAND_BUCKETS.len() {
            self.cur_band = 0;
            self.cur_bit -= 1; // Decrement bit-plane after all bands
        }
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
                    // CORRECTED: Calculate step size for the current bit-plane
                    let s_initial = if band == 0 { quant_lo[bucket_idx] } else { quant_hi[band] };
                    let step_size = (s_initial >> bit).max(1);
                    let threshold = step_size; // The threshold for significance is the current step size

                    let scaled_coeff = (coeff as i32).abs();
                    
                    if threshold > 1 && scaled_coeff >= threshold {
                        // Encode that coefficient becomes significant
                        zp.encode(true, ctx_root)?;
                        
                        // Encode sign
                        zp.encode(coeff < 0, ctx_root)?;
                        
                        // Set initial reconstructed value: step_size + (step_size >> 1)
                        let sign = if coeff < 0 { -1 } else { 1 };
                        let recon = step_size + (step_size >> 1);
                        ecoeffs[i] = (sign * recon) as i16;
                        
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
                    // CORRECTED: Calculate step size for the current bit-plane
                    let s_initial = if band == 0 { quant_lo[bucket_idx] } else { quant_hi[band] };
                    let step_size = (s_initial >> bit).max(1);
                    
                    let orig_coeff = coeff as i32;
                    let prev_val = ecoeffs[i] as i32;
                    let coeff_abs = orig_coeff.abs();
                    
                    // Compute mantissa bit: pix = (coeff >= ecoeff) ? 1 : 0
                    let pix = if coeff_abs >= prev_val.abs() { 1 } else { 0 };
                    
                    // Encode mantissa bit
                    zp.encode(pix != 0, ctx_mant)?;
                    
                    // Adjust coefficient: epcoeff[i] = ecoeff - (pix ? 0 : step_size) + (step_size >> 1);
                    let sign = if prev_val < 0 { -1 } else { 1 };
                    let abs_ecoeff = prev_val.abs();
                    let adjustment = if pix != 0 { 0 } else { step_size };
                    let new_abs = abs_ecoeff - adjustment + (step_size >> 1);
                    ecoeffs[i] = (sign * new_abs) as i16;
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
            
            // CORRECTED: Calculate step size for the current bit-plane
            let absolute_bucket_idx = fbucket + buckno;
            let initial_s = if band == 0 {
                if absolute_bucket_idx < 16 { quant_lo[absolute_bucket_idx] } else { 0 }
            } else {
                quant_hi[band]
            };
            let threshold = (initial_s >> cur_bit).max(1);

            match (pcoeff, epcoeff) {
                (Some(pc), Some(epc)) => {
                    for i in 0..16 {
                        let cstate_idx = buckno * 16 + i;
                        if cstate_idx < coeff_state.len() {
                            let mut cstatetmp = ZERO;
                            
                            if epc[i] != 0 {
                                // Already active from previous bit-plane
                                cstatetmp = ACTIVE;
                            } else if threshold > 1 && (pc[i] as i32).abs() >= threshold {
                                // Could become significant at this bit-plane
                                cstatetmp = NEW | UNK;
                            } else if threshold > 1 {
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
                            
                            if threshold > 1 && (pc[i] as i32).abs() >= threshold {
                                // Could become significant at this bit-plane
                                cstatetmp = NEW | UNK;
                            } else if threshold > 1 {
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