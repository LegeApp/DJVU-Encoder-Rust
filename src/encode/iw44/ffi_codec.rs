// Alternative codec implementation using C++ FFI
// This is a drop-in replacement for your problematic Rust codec

use crate::encode::iw44::coeff_map::CoeffMap;
use crate::encode::zc::ZEncoder;
use crate::ffi::simple_iw44_ffi::SimpleIW44Codec;
use crate::Result;
use std::io::Write;
use log::info;

pub struct FfiCodec {
    pub map: CoeffMap,        // Expose map field like original Codec
    pub cur_bit: i32,         // Expose cur_bit field like original Codec  
    inner: SimpleIW44Codec,
}

impl FfiCodec {
    /// Create new codec using C++ FFI - matches your original Codec::new signature
    pub fn new(map: CoeffMap, _params: &super::encoder::EncoderParams) -> Self {
        // Convert your CoeffMap to flat coefficient array
        let mut coeffs = Vec::new();
        for block in &map.blocks {
            for bucket_idx in 0..64 {
                if let Some(bucket) = block.get_bucket(bucket_idx) {
                    coeffs.extend_from_slice(bucket);
                }
            }
        }
        
        // Set up quantization tables (use your existing logic)
        let mut quant_lo = [0i32; 16];
        let mut quant_hi = [0i32; 10];
        
        // Copy from your constants - adjust as needed
        const IW_QUANT: [i32; 16] = [
            0x1000, 0x1000, 0x2000, 0x2000,
            0x3000, 0x3000, 0x4000, 0x4000,
            0x5000, 0x5000, 0x6000, 0x6000,
            0x7000, 0x7000, 0x8000, 0x8000,
        ];
        
        for i in 0..16 {
            quant_lo[i] = IW_QUANT[i];
        }
        for i in 0..10 {
            quant_hi[i] = IW_QUANT[i.min(15)];
        }
        
        // Determine starting bit plane
        let max_coeff = coeffs.iter().map(|&c| (c as i32).abs()).max().unwrap_or(0);
        let starting_bit = if max_coeff > 0 {
            if max_coeff < 50 {
                12
            } else if max_coeff < 1000 {
                10
            } else {
                max_coeff.ilog2() as i32
            }
        } else {
            0
        };
        
        let inner = SimpleIW44Codec::new(
            coeffs,
            map.iw as i32,
            map.ih as i32,
            quant_lo,
            quant_hi,
            starting_bit,
        );
        
        info!("FfiCodec created with {} coefficients, starting bit: {}", inner.coeffs.len(), starting_bit);
        
        Self { 
            map,
            cur_bit: starting_bit,
            inner 
        }
    }
    
    /// Drop-in replacement for your encode_slice method
    pub fn encode_slice<W: Write>(&mut self, zp: &mut ZEncoder<W>) -> Result<bool> {
        let result = self.inner.encode_slice(zp)?;
        // Update our cur_bit field to match the inner codec
        self.cur_bit = self.inner.cur_bit;
        Ok(result)
    }
}
