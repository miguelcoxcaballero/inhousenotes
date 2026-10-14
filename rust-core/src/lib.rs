//! Numeric pixel kernels only. Geometry validation, cancellation and persistence
//! remain with the host. All dimensions/pointers are supplied by the private JS adapter.
#[no_mangle]
pub extern "C" fn core_abi_version() -> u32 { 1 }

#[no_mangle]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    if len == 0 || len > 128 * 1024 * 1024 { return std::ptr::null_mut(); }
    let mut buffer = Vec::<u8>::new();
    if buffer.try_reserve_exact(len).is_err() { return std::ptr::null_mut(); }
    buffer.resize(len, 255);
    Box::into_raw(buffer.into_boxed_slice()) as *mut u8
}

/// Pointer/length must be the pair returned by alloc, freed exactly once.
#[no_mangle]
pub unsafe extern "C" fn free(ptr: *mut u8, len: usize) {
    if !ptr.is_null() && len > 0 { drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len))); }
}

// Match Uint8ClampedArray's round-to-nearest, ties-to-even (not f64::round).
fn clamp_byte(value: f64) -> u8 { value.round_ties_even().clamp(0.0, 255.0) as u8 }

#[no_mangle]
pub unsafe extern "C" fn warp_cell(src: *const u8, sw: usize, sh: usize,
    dst: *mut u8, dw: usize, dh: usize, left: f64, right: f64, top: f64, bottom: f64,
    tlx: f64, tly: f64, trx: f64, try_: f64, blx: f64, bly: f64, brx: f64, bry: f64) {
    if sw < 2 || sh < 2 || dw == 0 || dh == 0 || sw > 16384 || sh > 16384 || dw > 16384 || dh > 16384
        || src.is_null() || dst.is_null() || right <= left || bottom <= top
        || ![left,right,top,bottom,tlx,tly,trx,try_,blx,bly,brx,bry].iter().all(|x| x.is_finite()) { return; }
    let input = std::slice::from_raw_parts(src, sw * sh * 4);
    let output = std::slice::from_raw_parts_mut(dst, dw * dh * 4);
    let x0 = (left - 0.5).ceil().clamp(0.0, dw as f64) as usize;
    let x1 = (right - 0.5).ceil().clamp(0.0, dw as f64) as usize;
    let y0 = (top - 0.5).ceil().clamp(0.0, dh as f64) as usize;
    let y1 = (bottom - 0.5).ceil().clamp(0.0, dh as f64) as usize;
    let width = right - left;
    let first_u = (x0 as f64 + 0.5 - left) / width;
    for y in y0..y1 {
        let v = (y as f64 + 0.5 - top) / (bottom - top);
        let lx = tlx + (blx - tlx) * v;
        let ly = tly + (bly - tly) * v;
        let rx = trx + (brx - trx) * v;
        let ry = try_ + (bry - try_) * v;
        let dx = (rx - lx) / width;
        let dy = (ry - ly) / width;
        let mut sx = lx + (rx - lx) * first_u - 0.5;
        let mut sy = ly + (ry - ly) * first_u - 0.5;
        for x in x0..x1 {
            let dest = (y * dw + x) * 4;
            if sx < 0.0 || sy < 0.0 || sx >= (sw - 1) as f64 || sy >= (sh - 1) as f64 {
                output[dest..dest+4].fill(255);
            } else {
                let ix = sx.floor() as usize;
                let iy = sy.floor() as usize;
                let fx = sx - ix as f64;
                let fy = sy - iy as f64;
                let first = (iy * sw + ix) * 4;
                let second = first + sw * 4;
                for c in 0..3 {
                    let upper = input[first+c] as f64 * (1.0-fx) + input[first+4+c] as f64 * fx;
                    let lower = input[second+c] as f64 * (1.0-fx) + input[second+4+c] as f64 * fx;
                    output[dest+c] = clamp_byte(upper * (1.0-fy) + lower * fy);
                }
                output[dest+3] = 255;
            }
            sx += dx; sy += dy;
        }
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
    #[test] fn clamped_array_rounding() {
        assert_eq!(clamp_byte(12.5), 12); assert_eq!(clamp_byte(13.5), 14);
        assert_eq!(clamp_byte(256.0), 255); assert_eq!(clamp_byte(-1.0), 0);
    }
    #[test] fn alpha_is_lossless() {
        let src = [1,2,3,255, 254,253,252,17]; let mut rgb = [0;6]; let mut alpha = [0;2];
        assert_eq!(unsafe { split_rgba(src.as_ptr(),rgb.as_mut_ptr(),alpha.as_mut_ptr(),2) },1);
        assert_eq!(rgb,[1,2,3,254,253,252]); assert_eq!(alpha,[255,17]);
    }
}
