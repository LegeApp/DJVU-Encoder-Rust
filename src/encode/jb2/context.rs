// src/jb2/context.rs

// JB2 adaptive context functions (ported from JB2Image.h)

/// Compute the initial direct coding context.
#[inline]
pub fn get_direct_context(
    up2: &[u8],
    up1: &[u8],
    up0: &[u8],
    column: usize,
) -> usize {
    ((up2[column - 1] as usize) << 9)
        | ((up2[column] as usize) << 8)
        | ((up2[column + 1] as usize) << 7)
        | ((up1[column - 2] as usize) << 6)
        | ((up1[column - 1] as usize) << 5)
        | ((up1[column] as usize) << 4)
        | ((up1[column + 1] as usize) << 3)
        | ((up1[column + 2] as usize) << 2)
        | ((up0[column - 2] as usize) << 1)
        | ((up0[column - 1] as usize) << 0)
}

/// Update the direct coding context with the next pixel.
#[inline]
pub fn shift_direct_context(
    context: usize,
    next: u8,
    up2: &[u8],
    up1: &[u8],
    up0: &[u8],
    column: usize,
) -> usize {
    ((context << 1) & 0x37a)
        | ((up1[column + 2] as usize) << 2)
        | ((up2[column + 1] as usize) << 7)
        | ((next as usize) << 0)
}

/// Compute the initial cross coding context.
#[inline]
pub fn get_cross_context(
    up1: &[u8],
    up0: &[u8],
    xup1: &[u8],
    xup0: &[u8],
    xdn1: &[u8],
    column: usize,
) -> usize {
    ((up1[column - 1] as usize) << 10)
        | ((up1[column] as usize) << 9)
        | ((up1[column + 1] as usize) << 8)
        | ((up0[column - 1] as usize) << 7)
        | ((xup1[column] as usize) << 6)
        | ((xup0[column - 1] as usize) << 5)
        | ((xup0[column] as usize) << 4)
        | ((xup0[column + 1] as usize) << 3)
        | ((xdn1[column - 1] as usize) << 2)
        | ((xdn1[column] as usize) << 1)
        | ((xdn1[column + 1] as usize) << 0)
}

/// Update the cross coding context with the next pixel.
#[inline]
pub fn shift_cross_context(
    context: usize,
    n: u8,
    up1: &[u8],
    up0: &[u8],
    xup1: &[u8],
    xup0: &[u8],
    xdn1: &[u8],
    column: usize,
) -> usize {
    ((context << 1) & 0x636)
        | ((up1[column + 1] as usize) << 8)
        | ((xup1[column] as usize) << 6)
        | ((xup0[column + 1] as usize) << 3)
        | ((xdn1[column + 1] as usize) << 0)
        | ((n as usize) << 7)
}

/// Wrapper for get_direct_context that works with GrayImage
pub fn get_direct_context_image(
    image: &image::GrayImage,
    x: u32,
    y: u32,
) -> Result<usize, crate::encode::jb2::error::Jb2Error> {
    let width = image.width();
    let height = image.height();
    
    // Ensure we have enough padding for context window
    if x < 2 || y < 2 || x >= width - 2 || y >= height - 2 {
        return Ok(0); // Use default context for edge cases
    }
    
    // Extract row data as slices
    let up2_row: Vec<u8> = (0..width).map(|col| image.get_pixel(col, y - 2).0[0]).collect();
    let up1_row: Vec<u8> = (0..width).map(|col| image.get_pixel(col, y - 1).0[0]).collect();
    let up0_row: Vec<u8> = (0..width).map(|col| image.get_pixel(col, y).0[0]).collect();
    
    // Convert binary pixels (0 or 255) to binary (0 or 1)
    let up2_binary: Vec<u8> = up2_row.iter().map(|&p| if p > 127 { 1 } else { 0 }).collect();
    let up1_binary: Vec<u8> = up1_row.iter().map(|&p| if p > 127 { 1 } else { 0 }).collect();
    let up0_binary: Vec<u8> = up0_row.iter().map(|&p| if p > 127 { 1 } else { 0 }).collect();
    
    Ok(get_direct_context(&up2_binary, &up1_binary, &up0_binary, x as usize))
}

/// Wrapper for get_cross_context that works with GrayImage
pub fn get_cross_context_image(
    current_image: &image::GrayImage,
    reference_image: &image::GrayImage,
    x: u32,
    y: u32,
    xd2c: i32,
    cy: i32,
) -> Result<usize, crate::encode::jb2::error::Jb2Error> {
    let width = current_image.width();
    let height = current_image.height();
    
    // Ensure we have enough padding for context window
    if x < 1 || y < 1 || x >= width - 1 || y >= height - 1 {
        return Ok(0); // Use default context for edge cases
    }
    
    // Calculate reference coordinates with offset
    let ref_x = (x as i32 + xd2c).max(0).min((reference_image.width() - 1) as i32) as u32;
    let ref_y = (y as i32 + cy).max(0).min((reference_image.height() - 1) as i32) as u32;
    
    // Extract current image rows
    let up1_row: Vec<u8> = (0..width).map(|col| {
        if y > 0 { 
            let p = current_image.get_pixel(col, y - 1).0[0];
            if p > 127 { 1 } else { 0 }
        } else { 0 }
    }).collect();
    
    let up0_row: Vec<u8> = (0..width).map(|col| {
        let p = current_image.get_pixel(col, y).0[0];
        if p > 127 { 1 } else { 0 }
    }).collect();
    
    // Extract reference image rows
    let ref_width = reference_image.width();
    let ref_height = reference_image.height();
    
    let xup1_row: Vec<u8> = (0..ref_width).map(|col| {
        if ref_y > 0 {
            let p = reference_image.get_pixel(col, ref_y - 1).0[0];
            if p > 127 { 1 } else { 0 }
        } else { 0 }
    }).collect();
    
    let xup0_row: Vec<u8> = (0..ref_width).map(|col| {
        let p = reference_image.get_pixel(col, ref_y).0[0];
        if p > 127 { 1 } else { 0 }
    }).collect();
    
    let xdn1_row: Vec<u8> = (0..ref_width).map(|col| {
        if ref_y + 1 < ref_height {
            let p = reference_image.get_pixel(col, ref_y + 1).0[0];
            if p > 127 { 1 } else { 0 }
        } else { 0 }
    }).collect();
    
    // Ensure reference coordinates are within bounds for context calculation
    let safe_ref_x = ref_x.min(ref_width - 2).max(1) as usize;
    
    Ok(get_cross_context(
        &up1_row, 
        &up0_row, 
        &xup1_row, 
        &xup0_row, 
        &xdn1_row, 
        safe_ref_x
    ))
}

pub struct Context {
    pub direct: usize,
    pub cross: usize,
}
