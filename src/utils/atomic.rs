// src/atomic.rs

//! A module for atomic operations.
//!
//! This replaces the C++ `atomic.h` and its platform-specific implementations.
//! Rust's standard library provides safe, efficient, and portable atomic types
//! that compile down to the best available atomic instructions on the target CPU.

// Re-export the most common atomic types for convenience.
pub use std::sync::atomic::{
    AtomicI32, AtomicIsize, AtomicPtr, AtomicU32, AtomicUsize, Ordering,
};