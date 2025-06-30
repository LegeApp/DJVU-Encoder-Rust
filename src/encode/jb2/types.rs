// src/encode/jb2/types.rs

//! Core types for JB2 compression.

use image::GrayImage;
use std::collections::HashMap;

/// A dictionary for storing JB2 shapes.
#[derive(Debug, Clone)]
pub struct Jb2Dict {
    /// The collection of bitmap shapes in the dictionary.
    pub shapes: Vec<Jb2Shape>,
    /// Index mapping for quick lookups.
    pub shape_index: HashMap<u32, usize>,
}

impl Jb2Dict {
    /// Creates a new empty JB2 dictionary.
    pub fn new() -> Self {
        Self {
            shapes: Vec::new(),
            shape_index: HashMap::new(),
        }
    }

    /// Adds a shape to the dictionary.
    pub fn add_shape(&mut self, shape: Jb2Shape) -> usize {
        let index = self.shapes.len();
        self.shapes.push(shape);
        index
    }

    /// Gets a shape by index.
    pub fn get_shape(&self, index: usize) -> Option<&Jb2Shape> {
        self.shapes.get(index)
    }

    /// Returns the number of shapes in the dictionary.
    pub fn len(&self) -> usize {
        self.shapes.len()
    }

    /// Checks if the dictionary is empty.
    pub fn is_empty(&self) -> bool {
        self.shapes.is_empty()
    }
}

impl Default for Jb2Dict {
    fn default() -> Self {
        Self::new()
    }
}

/// A single shape in the JB2 dictionary.
#[derive(Debug, Clone)]
pub struct Jb2Shape {
    /// The bitmap data for this shape.
    pub bits: Option<GrayImage>,
    /// The width of the shape in pixels.
    pub width: u32,
    /// The height of the shape in pixels.
    pub height: u32,
    /// Reference count for this shape.
    pub ref_count: u32,
}

impl Jb2Shape {
    /// Creates a new JB2 shape.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            bits: None,
            width,
            height,
            ref_count: 0,
        }
    }

    /// Creates a JB2 shape with bitmap data.
    pub fn with_bitmap(bitmap: GrayImage) -> Self {
        let (width, height) = bitmap.dimensions();
        Self {
            bits: Some(bitmap),
            width,
            height,
            ref_count: 0,
        }
    }

    /// Sets the bitmap for this shape.
    pub fn set_bitmap(&mut self, bitmap: GrayImage) {
        let (width, height) = bitmap.dimensions();
        self.width = width;
        self.height = height;
        self.bits = Some(bitmap);
    }

    /// Gets the bitmap data.
    pub fn bitmap(&self) -> Option<&GrayImage> {
        self.bits.as_ref()
    }

    /// Increments the reference count.
    pub fn add_ref(&mut self) {
        self.ref_count += 1;
    }

    /// Decrements the reference count.
    pub fn release(&mut self) {
        if self.ref_count > 0 {
            self.ref_count -= 1;
        }
    }
}

/// A blit operation that places a shape at a specific location.
#[derive(Debug, Clone)]
pub struct Jb2Blit {
    /// The x-coordinate where the shape should be placed.
    pub x: i32,
    /// The y-coordinate where the shape should be placed.
    pub y: i32,
    /// The index of the shape in the dictionary.
    pub shape_index: usize,
    /// Width of the shape for this blit.
    pub width: u32,
    /// Height of the shape for this blit.
    pub height: u32,
}

impl Jb2Blit {
    /// Creates a new blit operation.
    pub fn new(x: i32, y: i32, shape_index: usize, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            shape_index,
            width,
            height,
        }
    }

    /// Gets the bounding rectangle of this blit.
    pub fn bounds(&self) -> (i32, i32, u32, u32) {
        (self.x, self.y, self.width, self.height)
    }
}

/// A complete JB2 image containing shapes and blits.
#[derive(Debug, Clone)]
pub struct Jb2Image {
    /// The width of the full image.
    pub width: u32,
    /// The height of the full image.
    pub height: u32,
    /// The dictionary of shapes used in this image.
    pub dict: Jb2Dict,
    /// The list of blit operations that compose the image.
    pub blits: Vec<Jb2Blit>,
}

impl Jb2Image {
    /// Creates a new JB2 image.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            dict: Jb2Dict::new(),
            blits: Vec::new(),
        }
    }

    /// Adds a blit to the image.
    pub fn add_blit(&mut self, blit: Jb2Blit) {
        self.blits.push(blit);
    }

    /// Adds a shape to the dictionary and returns its index.
    pub fn add_shape(&mut self, shape: Jb2Shape) -> usize {
        self.dict.add_shape(shape)
    }

    /// Gets a shape from the dictionary.
    pub fn get_shape(&self, index: usize) -> Option<&Jb2Shape> {
        self.dict.get_shape(index)
    }

    /// Returns the number of blits in the image.
    pub fn blit_count(&self) -> usize {
        self.blits.len()
    }

    /// Returns the number of shapes in the dictionary.
    pub fn shape_count(&self) -> usize {
        self.dict.len()
    }

    /// Decodes a JB2 image from a byte stream (placeholder).
    pub fn decode(
        _bs: crate::core::djvu_file::ByteStream, 
        _fgjd: Option<Jb2Dict>
    ) -> Result<Self, crate::encode::jb2::error::Jb2Error> {
        // Placeholder implementation
        Ok(Self::new(100, 100))
    }
}

impl Default for Jb2Image {
    fn default() -> Self {
        Self::new(0, 0)
    }
}
