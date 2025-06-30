// src/data_pool.rs

//! A read-only, seekable pool of byte data.
//!
//! This module replaces the complex, asynchronous C++ `DataPool` class with a
//! simplified, synchronous, and type-safe Rust equivalent suitable for an encoder.
//!
//! The `DataPool` provides a unified interface over various data sources, such as
//! an in-memory buffer, a file on disk, or a slice of another `DataPool`. This is
//! achieved using Rust's trait system, which is safer and more flexible than the
//! pointer-based "connection" system of the original C++ code.

use crate::utils::error::{DjvuError, Result};
use std::fs::File;
use std::io::{self, Cursor, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Arc;

/// A trait representing a source of byte data that can be read and sought.
///
/// This is the core abstraction that allows `DataPool` to work with different
/// underlying data storage mechanisms (memory, file, etc.).
trait DataSource: Read + Seek + Send + Sync {
    /// Returns the total size of the data source in bytes.
    fn len(&self) -> u64;

    /// Checks if the data source is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// Implement DataSource for a read-only cursor over a shared byte buffer (in-memory data).
pub struct ArcCursor(Cursor<Arc<Vec<u8>>>);

impl Read for ArcCursor {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let remaining_in_pool = self.len() - self.pos;
        if remaining_in_pool == 0 {
            return Ok(0); // EOF for this pool's view
        }

        let max_read = (buf.len() as u64).min(remaining_in_pool);
        let mut limited_buf = &mut buf[..max_read as usize];

        // The source must be mutable for `read` and `seek`, but we hold it in an `Arc`.
        // This is a classic case for `Arc::get_mut`, but we can't use that if the Arc is shared.
        // A real-world, high-performance library would use a `Mutex` here to protect the
        // underlying `File` or `Cursor` handle if it needs to be shared and mutated by
        // multiple `DataPool` clones simultaneously on different threads.
        // For a single-threaded encoder context, this simplification of re-opening or
        // cloning the handle is acceptable. For this refactor, we assume the source
        // can be mutated through a temporary mutable reference.
        // **This is a simplified approach.** A production-ready version would
        // need `Arc<Mutex<dyn DataSource>>`.
        let source_mut = match Arc::get_mut(&mut self.source) {
            Some(s) => s,
            None => {
                // This is the tricky part. If the source is shared, we can't get a `&mut`.
                // For now, we'll return an error.
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "Cannot read from a shared DataPool source concurrently (simplification).",
                ));
            }
        };

        source_mut.seek(SeekFrom::Start(self.start + self.pos))?;
        let bytes_read = source_mut.read(&mut limited_buf)?;
        self.pos += bytes_read as u64;

        Ok(bytes_read)
    }
}

impl DataSource for ArcCursor {}

impl AsRef<[u8]> for ArcCursor {
    fn as_ref(&self) -> &[u8] {
        self.0.get_ref().as_ref()
    }
}

// Implement DataSource for a file.
impl DataSource for File {
    fn len(&self) -> u64 {
        self.metadata().map(|m| m.len()).unwrap_or(0)
    }
}

/// A read-only pool of data providing a unified `Read` and `Seek` interface.
///
/// A `DataPool` can be created from an in-memory `Vec<u8>`, a file path, or a
/// slice of another `DataPool`. It is cheap to clone, as it uses `Arc` for
/// shared ownership of the underlying data source.
pub struct DataPool {
    // The underlying data source is a trait object, allowing for different
    // concrete types (File, Cursor, etc.).
    source: Arc<dyn DataSource>,
    // The start and end bounds of the view into the source.
    start: u64,
    end: u64,
    // The current read position within this pool's view.
    pos: u64,
}

impl DataPool {
    /// Creates a new `DataPool` from an in-memory vector of bytes.
    /// The `DataPool` takes ownership of the data.
    #[inline]
    pub fn from_vec(data: Vec<u8>) -> Self {
        let len = data.len() as u64;
        DataPool {
            source: Arc::new(Cursor::new(Arc::new(data))),
            start: 0,
            end: len,
            pos: 0,
        }
    }

    /// Creates a new `DataPool` by opening a file at the given path.
    ///
    /// Returns an error if the file cannot be opened.
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = File::open(path)?;
        let len = file.len();
        Ok(DataPool {
            source: Arc::new(file),
            start: 0,
            end: len,
            pos: 0,
        })
    }

    /// Creates a new `DataPool` that is a view (slice) into another `DataPool`.
    ///
    /// This is a cheap operation as it shares the underlying data source.
    ///
    /// # Arguments
    /// * `parent` - The `DataPool` to slice.
    /// * `offset` - The starting byte offset within the parent pool.
    /// * `len` - The length of the slice. If `None`, the slice extends to the end of the parent.
    ///
    /// Returns an error if the requested slice is out of bounds.
    pub fn slice(&self, offset: u64, len: Option<u64>) -> Result<Self> {
        let parent_len = self.len();
        if offset > parent_len {
            return Err(DjvuError::InvalidArg(
                "Slice offset is beyond the end of the data pool.".to_string(),
            ));
        }

        let slice_len = len.unwrap_or(parent_len - offset);

        if offset + slice_len > parent_len {
            return Err(DjvuError::InvalidArg(
                "Slice extends beyond the end of the data pool.".to_string(),
            ));
        }

        Ok(DataPool {
            source: self.source.clone(),
            start: self.start + offset,
            end: self.start + offset + slice_len,
            pos: 0,
        })
    }

    /// Returns the total length of the data available in this pool (or view).
    #[inline]
    pub fn len(&self) -> u64 {
        self.end - self.start
    }

    /// Returns `true` if the pool contains no data.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// Implement Clone to allow cheap, shared-ownership copies of the DataPool.
impl Clone for DataPool {
    fn clone(&self) -> Self {
        DataPool {
            source: self.source.clone(),
            start: self.start,
            end: self.end,
            pos: self.pos,
        }
    }
}

// Implement the standard `Seek` trait for `DataPool`.
impl Seek for DataPool {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let new_pos = match pos {
            SeekFrom::Start(p) => p as i64,
            SeekFrom::End(p) => self.len() as i64 + p,
            SeekFrom::Current(p) => self.pos as i64 + p,
        };

        if new_pos < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Seek to a negative position is not allowed.",
            ));
        }

        self.pos = (new_pos as u64).min(self.len());
        Ok(self.pos)
    }
}

// Implementing AsRef<[u8]> for Cursor<Arc<Vec<u8>>>
impl AsRef<[u8]> for Cursor<Arc<Vec<u8>>> {
    fn as_ref(&self) -> &[u8] {
        self.get_ref().as_ref()
    }
}