// src/encode/iw44/constants.rs

//! Constants for IW44 encoding
//! 
//! These constants define various parameters and thresholds used
//! throughout the IW44 wavelet-based encoding process.

/// Number of buckets for organizing frequency bands
pub const BAND_BUCKETS: usize = 64;

/// IW normalization factor for coefficient processing
pub const IW_NORM: f32 = 1.0;

/// IW quantization step size
pub const IW_QUANT: f32 = 1.0;

/// Bit shift amount for IW processing 
pub const IW_SHIFT: i32 = 6;

/// Decibel pruning threshold for encoder
pub const DECIBEL_PRUNE: f32 = -10.0;

/// Coefficient processing parameters
pub const MAX_REFINEMENT_PASSES: usize = 12;
pub const MIN_COEFFICIENT_THRESHOLD: f32 = 0.001;

/// Zigzag ordering constants - re-exported from zigzag module
pub use super::zigzag::ZIGZAG_LOC;
