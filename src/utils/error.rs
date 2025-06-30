// src/error.rs

use std::fmt;

/// The primary error type for all operations in the DjVu encoder library.
#[derive(Debug)]
pub enum DjvuError {
    /// An error occurred during I/O operations (e.g., file not found, permission denied).
    Io(std::io::Error),
    /// An error occurred during encoding or decoding of data streams.
    Stream(String),
    /// An invalid argument was provided to a function.
    InvalidArg(String),
    /// The requested operation is not supported or invalid in the current context.
    InvalidOperation(String),
    /// A required component or resource was not found.
    NotFound(String),
}

// Implement the standard Error trait to be a good citizen in the Rust ecosystem.
impl std::error::Error for DjvuError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DjvuError::Io(err) => Some(err),
            _ => None,
        }
    }
}

// Implement Display for user-friendly error messages.
impl fmt::Display for DjvuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DjvuError::Io(err) => write!(f, "I/O error: {}", err),
            DjvuError::Stream(msg) => write!(f, "Stream error: {}", msg),
            DjvuError::InvalidArg(msg) => write!(f, "Invalid argument: {}", msg),
            DjvuError::InvalidOperation(msg) => write!(f, "Invalid operation: {}", msg),
            DjvuError::NotFound(msg) => write!(f, "Not found: {}", msg),
        }
    }
}

// Allow easy conversion from `std::io::Error` into our `DjvuError`.
impl From<std::io::Error> for DjvuError {
    fn from(err: std::io::Error) -> Self {
        DjvuError::Io(err)
    }
}

/// A specialized `Result` type for DjVu operations.
pub type Result<T> = std::result::Result<T, DjvuError>;