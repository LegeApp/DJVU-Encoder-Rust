// src/jb2/mod.rs
pub mod encoder;
pub mod jb2_image;
pub mod jb2_shape;
pub mod jb2_dict;
pub mod jb2_blit;
pub mod arithmetic_coder;

mod context;
mod num_coder;
mod error;

use image::GrayImage;
use std::ops::{Deref, DerefMut};
use std::rc::Rc;
use crate::zp_codec::ZpEncoder; // Using the ZP-coder from the previous step.

/// A single blit operation, instructing where to place a shape.
#[derive(Debug, Clone, Copy, Default)]
pub struct Jb2Blit {
    pub left: u16,
    pub bottom: u16,
    pub shape_index: u32,
}

/// A single shape, typically representing a character or symbol.
#[derive(Debug, Clone)]
pub struct Jb2Shape {
    /// The index of the parent shape this one is a refinement of.
    pub parent: Option<usize>,
    /// The bitmap data for the shape.
    pub bits: Option<GrayImage>,
}

/// A dictionary of shapes that can be shared across pages.
#[derive(Debug, Default)]
pub struct Jb2Dict {
    pub shapes: Vec<Jb2Shape>,
    pub inherited_dict: Option<Rc<Jb2Dict>>,
    pub comment: String,
}

impl Jb2Dict {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn get_shape(&self, shape_index: usize) -> Option<&Jb2Shape> {
        if let Some(inherited) = &self.inherited_dict {
            if shape_index < inherited.shapes.len() {
                return inherited.get_shape(shape_index);
            }
            self.shapes.get(shape_index - inherited.shapes.len())
        } else {
            self.shapes.get(shape_index)
        }
    }
    
    pub fn add_shape(&mut self, shape: Jb2Shape) -> usize {
        let inherited_count = self.inherited_dict.as_ref().map_or(0, |d| d.shapes.len());
        self.shapes.push(shape);
        inherited_count + self.shapes.len() - 1
    }

    pub fn shape_count(&self) -> usize {
        let inherited_count = self.inherited_dict.as_ref().map_or(0, |d| d.shapes.len());
        inherited_count + self.shapes.len()
    }
}

/// A full JB2 image, containing a shape dictionary and a list of blits.
#[derive(Debug, Default)]
pub struct Jb2Image {
    dict: Jb2Dict,
    blits: Vec<Jb2Blit>,
    width: u32,
    height: u32,
}

impl Jb2Image {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            ..Default::default()
        }
    }
    
    pub fn add_blit(&mut self, blit: Jb2Blit) {
        self.blits.push(blit);
    }
}

// Allow `Jb2Image` to act like a `Jb2Dict` for convenience.
impl Deref for Jb2Image {
    type Target = Jb2Dict;
    fn deref(&self) -> &Self::Target {
        &self.dict
    }
}

impl DerefMut for Jb2Image {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.dict
    }
}