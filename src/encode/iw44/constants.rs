// src/encode/iw44/constants.rs

//! Constants for IW44 encoding - re-exports from the main image coefficients module
//!
//! This module provides access to the same constants that are defined in the
//! centralized image::coefficients module to avoid duplication.

// Re-export the constants from the centralized location
pub use crate::image::coefficients::*;

/// Additional IW44-specific constants
pub const DECIBEL_PRUNE: f32 = 0.1; // Fixed from 5.0 to +0.1 dB as per spec
