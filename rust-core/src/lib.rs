//! Editor kernels and folder scene planning. Rendering, cancellation and
//! persistence remain with the host. Buffers are supplied by the private JS adapter.
mod checkpoint;
mod editor;
mod document_work;
mod pdf_ink;
#[no_mangle]
pub extern "C" fn core_abi_version() -> u32 { 1 }

#[cfg_attr(target_arch = "wasm32", no_mangle)]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    if len == 0 || len > 128 * 1024 * 1024 { return std::ptr::null_mut(); }
    // Host buffers also carry f64 values; their alignment must be guaranteed,
    // not inferred from a particular allocator's usual byte-buffer addresses.
    let layout = std::alloc::Layout::from_size_align(len, 8).unwrap();
    unsafe {
        let pointer = std::alloc::alloc(layout);
        if !pointer.is_null() { std::ptr::write_bytes(pointer, 255, len); }
        pointer
    }
}

/// Pointer/length must be the pair returned by alloc, freed exactly once.
// Native tests must not override libc's one-argument free with this WASM ABI.
#[cfg_attr(target_arch = "wasm32", no_mangle)]
pub unsafe extern "C" fn free(ptr: *mut u8, len: usize) {
    if !ptr.is_null() && len > 0 && len <= 128 * 1024 * 1024 {
        std::alloc::dealloc(ptr, std::alloc::Layout::from_size_align(len, 8).unwrap());
    }
}

#[no_mangle]
pub unsafe extern "C" fn split_rgba(src: *const u8, rgb: *mut u8, alpha: *mut u8, pixels: usize) -> u32 {
    let input = std::slice::from_raw_parts(src, pixels * 4);
    let color = std::slice::from_raw_parts_mut(rgb, pixels * 3);
    let mask = std::slice::from_raw_parts_mut(alpha, pixels);
    let mut transparent = 0;
    for p in 0..pixels {
        color[p*3..p*3+3].copy_from_slice(&input[p*4..p*4+3]);
        mask[p] = input[p*4+3];
        transparent |= (mask[p] != 255) as u32;
    }
    transparent
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn alpha_is_lossless() {
        let src = [1,2,3,255, 254,253,252,17]; let mut rgb = [0;6]; let mut alpha = [0;2];
        assert_eq!(unsafe { split_rgba(src.as_ptr(),rgb.as_mut_ptr(),alpha.as_mut_ptr(),2) },1);
        assert_eq!(rgb,[1,2,3,254,253,252]); assert_eq!(alpha,[255,17]);
    }
}
