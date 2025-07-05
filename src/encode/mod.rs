pub mod iw44;
pub mod jb2;
pub mod zc;

// Re-export commonly used encoding functionality
pub use jb2::*;
pub use zc::*;

// Re-export error types for convenience
pub use crate::utils::error::{DjvuError, Result};
