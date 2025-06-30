// src/chunk_tree.rs

//! An in-memory representation of an IFF file structure.
//!
//! This module replaces the C++ `GIFFManager` and `GIFFChunk` classes. It provides
//! a tree-like data structure, `IffChunk`, that can be loaded from a stream,
//! manipulated in memory, and saved back to a stream.

use crate::utils::error::{DjvuError, Result};
use crate::iff::iff::{IffReader, IffWriter};
use std::io::{Read, Seek, Write};

/// Represents the data payload of an `IffChunk`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChunkPayload {
    /// A raw byte buffer for simple chunks (e.g., "BG44", "INFO").
    Raw(Vec<u8>),
    /// A list of child chunks for composite chunks (e.g., "FORM", "LIST").
    Composite {
        /// The 4-character secondary identifier (e.g., "DJVU" in "FORM:DJVU").
        secondary_id: [u8; 4],
        /// The vector of child chunks.
        children: Vec<IffChunk>,
    },
}

/// Represents a single chunk in an IFF file, which can be either a leaf or a node in a tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IffChunk {
    /// The 4-character primary identifier (e.g., "FORM", "PM44").
    pub id: [u8; 4],
    /// The chunk's data, which can be raw bytes or a collection of sub-chunks.
    pub payload: ChunkPayload,
}

impl IffChunk {
    /// Creates a new raw data chunk.
    #[inline]
    pub fn new_raw(id: [u8; 4], data: Vec<u8>) -> Self {
        IffChunk { id, payload: ChunkPayload::Raw(data) }
    }

    /// Creates a new, empty composite chunk.
    #[inline]
    pub fn new_composite(id: [u8; 4], secondary_id: [u8; 4]) -> Self {
        IffChunk {
            id,
            payload: ChunkPayload::Composite {
                secondary_id,
                children: Vec::new(),
            },
        }
    }

    /// Returns `true` if this is a composite chunk.
    #[inline]
    pub fn is_composite(&self) -> bool {
        matches!(self.payload, ChunkPayload::Composite { .. })
    }

    /// Returns the chunk's primary ID as a string slice.
    #[inline]
    pub fn id_as_str(&self) -> &str {
        std::str::from_utf8(&self.id).unwrap_or("????")
    }

    /// Recursively writes this chunk and its children to the `IffWriter`.
    fn write<W: Write + Seek>(&self, writer: &mut IffWriter<W>) -> Result<()> {
        match &self.payload {
            ChunkPayload::Raw(data) => {
                let id_str = std::str::from_utf8(&self.id).unwrap_or("????");
                writer.put_chunk(id_str)?;
                writer.write_all(data)?;
            }
            ChunkPayload::Composite { secondary_id, children } => {
                let id_str = std::str::from_utf8(&self.id).unwrap_or("????");
                let secondary_str = std::str::from_utf8(secondary_id).unwrap_or("????");
                let full_id = format!("{}:{}", id_str, secondary_str.trim_end());
                writer.put_chunk(&full_id)?;
                for child in children {
                    child.write(writer)?;
                }
            }
        }
        writer.close_chunk()?;
        Ok(())
    }
}

/// Represents an entire IFF document as a tree of chunks.
/// This is the main entry point for creating, loading, and saving IFF files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IffDocument {
    /// The root chunk of the document, typically a "FORM" chunk.
    pub root: IffChunk,
}

impl IffDocument {
    /// Creates a new IFF document with a specified root chunk.
    #[inline]
    pub fn new(root_chunk: IffChunk) -> Self {
        IffDocument { root: root_chunk }
    }

    /// Parses an entire IFF stream from a reader into an `IffDocument`.
    pub fn from_reader<R: Read>(reader: R) -> Result<Self> {
        let mut iff_reader = IffReader::new(reader)?;
        
        // Read the root chunk
        let root_chunk_header = iff_reader.next_chunk()?.ok_or_else(|| {
            DjvuError::Stream("Cannot create document from empty stream.".to_string())
        })?;

        if !root_chunk_header.is_composite {
            return Err(DjvuError::Stream("Root chunk of a document must be a composite type (e.g., FORM).".to_string()));
        }

        // Recursively read the children
        let root_data_reader = iff_reader.take_chunk_reader(&root_chunk_header);
        let children = Self::read_chunk_tree(root_data_reader, root_chunk_header.size as u64)?;

        let root = IffChunk {
            id: root_chunk_header.id,
            payload: ChunkPayload::Composite {
                secondary_id: root_chunk_header.secondary_id,
                children,
            },
        };

        Ok(IffDocument { root })
    }
    
    /// A recursive helper to read a tree of chunks from a limited reader.
    fn read_chunk_tree(reader: impl Read, mut bytes_to_read: u64) -> Result<Vec<IffChunk>> {
        let mut children = Vec::new();
        let mut iff_reader = IffReader::new(reader)?;

        while bytes_to_read > 0 {
            let chunk_header = match iff_reader.next_chunk()? {
                Some(ch) => ch,
                None => break, // Clean end of stream within the composite chunk
            };

            let header_size = if chunk_header.is_composite { 12 } else { 8 };
            let chunk_total_size = header_size + chunk_header.size as u64;
            let padded_size = if chunk_total_size % 2 != 0 { chunk_total_size + 1 } else { chunk_total_size };

            if padded_size > bytes_to_read {
                return Err(DjvuError::Stream("Child chunk size exceeds parent's boundary.".to_string()));
            }

            let mut chunk_data_reader = iff_reader.take_chunk_reader(&chunk_header);

            let chunk = if chunk_header.is_composite {
                let sub_children = Self::read_chunk_tree(chunk_data_reader, chunk_header.size as u64)?;
                IffChunk {
                    id: chunk_header.id,
                    payload: ChunkPayload::Composite {
                        secondary_id: chunk_header.secondary_id,
                        children: sub_children,
                    },
                }
            } else {
                let mut data = Vec::with_capacity(chunk_header.size as usize);
                chunk_data_reader.read_to_end(&mut data)?;
                IffChunk::new_raw(chunk_header.id, data)
            };

            children.push(chunk);
            bytes_to_read -= padded_size;
        }

        Ok(children)
    }

    /// Writes the entire IFF document to the given writer.
    pub fn write<W: Write + Seek>(&self, writer: W) -> Result<()> {
        let mut iff_writer = IffWriter::new(writer);
        iff_writer.write_magic_bytes()?;
        self.root.write(&mut iff_writer)
    }
}