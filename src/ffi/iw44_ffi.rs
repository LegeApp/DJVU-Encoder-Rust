// Rust FFI bindings for IW44 C++ encoder
use std::ffi::c_void;
use std::ptr;

#[repr(C)]
pub struct IW44Encoder {
    _private: [u8; 0],
}

#[repr(C)]
pub struct IW44EncodeParms {
    pub slices: i32,     // Target number of slices
    pub bytes: i32,      // Target file size in bytes  
    pub decibels: f32,   // Target quality in decibels
}

extern "C" {
    fn iw44_encoder_new_grayscale(
        image_data: *const u8,
        width: i32,
        height: i32,
        mask_data: *const u8,
    ) -> *mut IW44Encoder;
    
    fn iw44_encoder_new_color(
        image_data: *const u8,
        width: i32,
        height: i32,
        mask_data: *const u8,
    ) -> *mut IW44Encoder;
    
    fn iw44_encode_chunk(
        encoder: *mut IW44Encoder,
        parms: *const IW44EncodeParms,
        output_data: *mut *mut u8,
        output_size: *mut usize,
    ) -> i32;
    
    fn iw44_get_slices(encoder: *mut IW44Encoder) -> i32;
    fn iw44_get_bytes(encoder: *mut IW44Encoder) -> i32;
    fn iw44_encoder_free(encoder: *mut IW44Encoder);
    fn iw44_free_output(data: *mut u8);
}

pub struct IW44EncoderWrapper {
    encoder: *mut IW44Encoder,
}

impl IW44EncoderWrapper {
    /// Create encoder for grayscale image
    pub fn new_grayscale(image_data: &[u8], width: i32, height: i32, mask: Option<&[u8]>) -> Option<Self> {
        let mask_ptr = mask.map_or(ptr::null(), |m| m.as_ptr());
        
        let encoder = unsafe {
            iw44_encoder_new_grayscale(image_data.as_ptr(), width, height, mask_ptr)
        };
        
        if encoder.is_null() {
            None
        } else {
            Some(Self { encoder })
        }
    }
    
    /// Create encoder for color image (RGB format)
    pub fn new_color(image_data: &[u8], width: i32, height: i32, mask: Option<&[u8]>) -> Option<Self> {
        let mask_ptr = mask.map_or(ptr::null(), |m| m.as_ptr());
        
        let encoder = unsafe {
            iw44_encoder_new_color(image_data.as_ptr(), width, height, mask_ptr)
        };
        
        if encoder.is_null() {
            None
        } else {
            Some(Self { encoder })
        }
    }
    
    /// Encode a chunk with the given parameters
    pub fn encode_chunk(&self, parms: &IW44EncodeParms) -> Result<Vec<u8>, String> {
        let mut output_data: *mut u8 = ptr::null_mut();
        let mut output_size: usize = 0;
        
        let result = unsafe {
            iw44_encode_chunk(
                self.encoder,
                parms,
                &mut output_data,
                &mut output_size,
            )
        };
        
        if result <= 0 {
            return Err(format!("IW44 encoding failed with code: {}", result));
        }
        
        if output_data.is_null() || output_size == 0 {
            return Ok(Vec::new());
        }
        
        // Copy data to Rust Vec and free C++ memory
        let rust_data = unsafe {
            let slice = std::slice::from_raw_parts(output_data, output_size);
            let vec = slice.to_vec();
            iw44_free_output(output_data);
            vec
        };
        
        Ok(rust_data)
    }
    
    /// Get current number of slices encoded
    pub fn get_slices(&self) -> i32 {
        unsafe { iw44_get_slices(self.encoder) }
    }
    
    /// Get current encoded size in bytes
    pub fn get_bytes(&self) -> i32 {
        unsafe { iw44_get_bytes(self.encoder) }
    }
}

impl Drop for IW44EncoderWrapper {
    fn drop(&mut self) {
        if !self.encoder.is_null() {
            unsafe {
                iw44_encoder_free(self.encoder);
            }
        }
    }
}

unsafe impl Send for IW44EncoderWrapper {}
unsafe impl Sync for IW44EncoderWrapper {}
