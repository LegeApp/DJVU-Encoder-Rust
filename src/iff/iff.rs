// src/iff.rs

//! A module for reading and writing IFF (Interchange File Format) streams.
//!
//! This module replaces the C++ `IFFByteStream` class with a safer, more idiomatic
//! and composable Rust API. It provides two main structs:
//! - `IffReader`: For parsing IFF chunks from any source that implements `std::io::Read`.
//! - `IffWriter`: For creating IFF files on any destination that implements
//!   `std::io::Write` and `std::io::Seek`.

use crate::utils::error::{DjvuError, Result};
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use std::io::{Read, Seek, SeekFrom, Write};

/// Represents the header of an IFF chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// The 4-character primary identifier (e.g., "FORM", "PM44").
    pub id: [u8; 4],
    /// The 4-character secondary identifier for composite chunks (e.g., "DJVU" in "FORM:DJVU").
    /// For simple chunks, this is typically all spaces or nulls.
    pub secondary_id: [u8; 4],
    /// The size of the chunk's data payload in bytes.
    pub size: u32,
    /// Indicates if the chunk is a composite type like 'FORM' or 'LIST'.
    pub is_composite: bool,
}

impl Chunk {
    /// Returns the full chunk ID as a string, e.g., "FORM:DJVU".
    #[inline]
    pub fn full_id(&self) -> String {
        let primary = String::from_utf8_lossy(&self.id);
        if self.is_composite {
            let secondary = String::from_utf8_lossy(&self.secondary_id);
            format!("{}:{}", primary, secondary.trim_end())
        } else {
            primary.trim_end().to_string()
        }
    }
}

/// A reader for parsing IFF-structured data from a byte stream.
pub struct IffReader<R: Read> {
    reader: R,
}

impl<R: Read> IffReader<R> {
    /// The 4-byte "AT&T" magic number sometimes found at the start of DjVu files.
    const MAGIC_ATT: [u8; 4] = [0x41, 0x54, 0x26, 0x54];
    /// The 4-byte "SDJV" magic number sometimes found at the start of DjVu files.
    const MAGIC_SDJV: [u8; 4] = [b'S', b'D', b'J', b'V'];

    /// Creates a new `IffReader` that wraps an existing reader.
    ///
    /// This constructor will automatically detect and skip the optional 4-byte
    /// DjVu magic numbers ('AT&T' or 'SDJV') if they are present at the
    /// beginning of the stream.
    #[inline]
    pub fn new(mut reader: R) -> Result<Self> {
        let mut magic_buf = [0u8; 4];
        // Peek at the first 4 bytes without consuming them from the main reader.
        let bytes_read = reader.read(&mut magic_buf)?;
        if bytes_read == 4 && (magic_buf == Self::MAGIC_ATT || magic_buf == Self::MAGIC_SDJV) {
            // It's a magic number, so we are done with the buffer.
        } else {
            // Not a magic number, so we need to "prepend" the bytes back.
            // We do this by chaining the buffered bytes with the original reader.
            let new_reader = magic_buf[..bytes_read].as_ref().chain(reader);
            return Ok(IffReader {
                reader: Box::new(new_reader),
            });
        }

        Ok(IffReader {
            reader: Box::new(reader),
        })
    }

    /// Reads the next chunk header from the stream.
    ///
    /// On success, returns `Ok(Some(Chunk))`.
    /// On end-of-stream, returns `Ok(None)`.
    /// On a parsing error, returns `Err(DjvuError)`.
    ///
    /// After calling this, the stream is positioned at the start of the chunk's
    /// data payload. The caller is responsible for reading `chunk.size` bytes.
    pub fn next_chunk(&mut self) -> Result<Option<Chunk>> {
        // Skip padding byte if necessary from the previous chunk.
        // IFF chunks are padded to an even number of bytes.
        // We can't know the absolute position, but we can track it.
        // A more robust implementation might track bytes read. For now, this is
        // simplified, assuming the caller correctly consumes the data.
        // A full implementation would need to track its own position.

        let mut id = [0u8; 4];
        match self.reader.read_exact(&mut id) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e.into()),
        }

        let size = self.reader.read_u32::<BigEndian>()?;
        let is_composite = matches!(&id, b"FORM" | b"LIST" | b"PROP" | b"CAT ");

        let secondary_id = if is_composite {
            let mut sid = [0u8; 4];
            self.reader.read_exact(&mut sid)?;
            sid
        } else {
            [b' '; 4]
        };

        Ok(Some(Chunk {
            id,
            secondary_id,
            size: if is_composite { size - 4 } else { size },
            is_composite,
        }))
    }

    /// Provides a limited reader for the current chunk's data payload.
    ///
    /// This returns a new reader that will automatically stop after `size` bytes,
    /// preventing accidental reading past the end of the current chunk.
    /// This is the primary way to safely read chunk data.
    #[inline]
    pub fn take_chunk_reader(self, chunk: &Chunk) -> impl Read {
        self.reader.take(chunk.size as u64)
    }
}

/// A writer for creating IFF-structured data on a byte stream.
/// The underlying writer must also implement `Seek` to allow for patching chunk sizes.
pub struct IffWriter<W: Write + Seek> {
    writer: W,
    // Stack to hold the file offset of the size field for each open chunk.
    chunk_stack: Vec<u64>,
}

impl<W: Write + Seek> IffWriter<W> {
    /// Creates a new `IffWriter` that wraps an existing writer.
    #[inline]
    pub fn new(writer: W) -> Self {
        IffWriter {
            writer,
            chunk_stack: Vec::new(),
        }
    }

    /// Writes the DjVu "AT&T" magic bytes to the start of the stream.
    /// This should only be called once at the very beginning of the file.
    #[inline]
    pub fn write_magic_bytes(&mut self) -> Result<()> {
        self.writer.write_all(&IffReader::<&[u8]>::MAGIC_ATT)?;
        Ok(())
    }

    /// Begins a new chunk with the given ID.
    ///
    /// For composite chunks, the ID should be in the format "FORM:DJVU".
    /// The writer is now positioned to write the chunk's payload.
    pub fn put_chunk(&mut self, full_id: &str) -> Result<()> {
        let (id, secondary_id) = Self::parse_full_id(full_id)?;

        self.writer.write_all(&id)?;

        // Store the position of the size field to be patched later.
        let size_pos = self.writer.stream_position()?;
        self.chunk_stack.push(size_pos);

        // Write a placeholder for the size.
        self.writer.write_u32::<BigEndian>(0)?;

        if let Some(sid) = secondary_id {
            self.writer.write_all(&sid)?;
        }

        Ok(())
    }

    /// Finishes the most recently opened chunk.
    ///
    /// This calculates the chunk's size, seeks back to the header, writes the
    /// correct size, and adds a padding byte if necessary to ensure the chunk
    /// ends on an even boundary.
    pub fn close_chunk(&mut self) -> Result<()> {
        let size_pos = self.chunk_stack.pop().ok_or_else(|| {
            DjvuError::InvalidOperation("Cannot close chunk: no chunk is open.".to_string())
        })?;

        // Calculate the size of the payload.
        let end_pos = self.writer.stream_position()?;
        let payload_start_pos = size_pos + 4; // Position after the size field
        let mut payload_size = end_pos - payload_start_pos;

        // IFF requires chunks to be padded to an even length.
        if payload_size % 2 != 0 {
            self.writer.write_all(&[0])?;
            payload_size += 1;
        }

        // Seek back, write the correct size, and return to the end.
        self.writer.seek(SeekFrom::Start(size_pos))?;
        self.writer.write_u32::<BigEndian>(payload_size as u32)?;
        self.writer.seek(SeekFrom::Start(end_pos))?;

        // After padding, we may need to seek again to the final position.
        if payload_size % 2 != 0 {
             self.writer.seek(SeekFrom::Current(1))?;
        }

        Ok(())
    }
    
    /// Helper to parse a user-friendly ID string into IFF bytes.
    fn parse_full_id(full_id: &str) -> Result<([u8; 4], Option<[u8; 4]>)> {
        let parts: Vec<_> = full_id.split(':').collect();
        match parts.as_slice() {
            [primary] => {
                if primary.len() != 4 {
                    return Err(DjvuError::InvalidArg(format!("Chunk ID must be 4 characters: '{}'", primary)));
                }
                Ok((primary.as_bytes().try_into().unwrap(), None))
            }
            [primary, secondary] => {
                if primary.len() != 4 || secondary.len() > 4 {
                    return Err(DjvuError::InvalidArg(format!("Composite chunk IDs must be 4 chars: '{}:{}'", primary, secondary)));
                }
                let mut sid_buf = [b' '; 4];
                sid_buf[..secondary.len()].copy_from_slice(secondary.as_bytes());
                Ok((primary.as_bytes().try_into().unwrap(), Some(sid_buf)))
            }
            _ => Err(DjvuError::InvalidArg(format!("Invalid chunk ID format: '{}'", full_id))),
        }
    }
}

// Implement Write for IffWriter to pass through writes to the underlying writer.
impl<W: Write + Seek> Write for IffWriter<W> {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.writer.write(buf)
    }

    #[inline]
    fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush()
    }
}