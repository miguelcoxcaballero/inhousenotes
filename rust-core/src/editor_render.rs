//! Editor drawing engine: stroke point bounds and legacy cover-stroke widening.
//! Canvas 2D calls and Path2D construction stay in the host; this module only
//! produces numbers from host-owned typed arrays.
pub fn call(_method: &str, _input: &[u8]) -> Option<Result<Vec<u8>, String>> { None }

/// Largest stroke the host may pack (points). Matches the host scratch limit.
pub const MAX_STROKE_POINTS: usize = 4_194_304;

/// Axis-aligned bounds of interleaved x,y pairs with the exact comparison
/// semantics of the original JS loop: strict `<` / `>` starting from
/// +/-Infinity, so NaN coordinates never update a bound.
fn stroke_bounds(coords: &[f64]) -> [f64; 4] {
    let (mut min_x, mut min_y) = (f64::INFINITY, f64::INFINITY);
    let (mut max_x, mut max_y) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for pair in coords.chunks_exact(2) {
        let (x, y) = (pair[0], pair[1]);
        if x < min_x { min_x = x; }
        if y < min_y { min_y = y; }
        if x > max_x { max_x = x; }
        if y > max_y { max_y = y; }
    }
    [min_x, min_y, max_x, max_y]
}

/// `ptr`: `count` x,y f64 pairs. `out`: four f64 (minX, minY, maxX, maxY).
/// Returns 1 on success, -1 for invalid arguments.
#[no_mangle]
pub unsafe extern "C" fn editor_render_stroke_bounds(ptr: *const f64, count: usize, out: *mut f64) -> i32 {
    if ptr.is_null() || out.is_null() || count == 0 || count > MAX_STROKE_POINTS
        || (ptr as usize) % 8 != 0 || (out as usize) % 8 != 0 { return -1; }
    let bounds = stroke_bounds(std::slice::from_raw_parts(ptr, count * 2));
    std::ptr::copy_nonoverlapping(bounds.as_ptr(), out, 4);
    1
}

#[allow(dead_code)] // the host sends 0 for every non-highlighter, non-eraser stroke
pub const COVER_PEN: u8 = 0;
pub const COVER_HIGHLIGHTER: u8 = 1;
pub const COVER_ERASER_STROKE: u8 = 2;
/// Written for strokes the cover pass must skip. Every drawn width is >= 1 or NaN.
pub const COVER_SKIP: f64 = -1.0;

/// Cover width for one legacy stroke. `width` is already the host's
/// `stroke.width || 1` number. `count` is the point count, 0 when the points
/// are not an array. The opaque-white cover is slightly wider than the burnt-in
/// ink so anti-aliasing is hidden even at high zoom.
fn cover_width(tool: u8, width: f64, count: u32) -> f64 {
    if tool == COVER_ERASER_STROKE { return COVER_SKIP; }
    let highlighter = tool == COVER_HIGHLIGHTER;
    if count < if highlighter { 1 } else { 2 } { return COVER_SKIP; }
    let widened = width * if highlighter { 1.05 } else { 1.35 };
    // Math.max(1, x): NaN propagates, unlike f64::max.
    if widened.is_nan() { widened } else if widened < 1.0 { 1.0 } else { widened }
}

/// Per-stroke cover widths. `tools`: n codes (0 pen/other, 1 highlighter,
/// 2 eraser-stroke or missing stroke), `widths`: n f64, `counts`: n u32,
/// `out`: n f64. Returns 1 on success, -1 for invalid arguments.
#[no_mangle]
pub unsafe extern "C" fn editor_render_cover_widths(tools: *const u8, widths: *const f64, counts: *const u32, n: usize, out: *mut f64) -> i32 {
    if tools.is_null() || widths.is_null() || counts.is_null() || out.is_null() || n == 0 || n > 1 << 24
        || (widths as usize) % 8 != 0 || (counts as usize) % 4 != 0 || (out as usize) % 8 != 0 { return -1; }
    let (tools, widths, counts) = (std::slice::from_raw_parts(tools, n), std::slice::from_raw_parts(widths, n), std::slice::from_raw_parts(counts, n));
    let out = std::slice::from_raw_parts_mut(out, n);
    for i in 0..n {
        if tools[i] > COVER_ERASER_STROKE { return -1; }
        out[i] = cover_width(tools[i], widths[i], counts[i]);
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_match_js_loop_semantics() {
        assert_eq!(stroke_bounds(&[3., 4., -1., 9., 2., -5.]), [-1., -5., 3., 9.]);
        assert_eq!(stroke_bounds(&[7., 7.]), [7., 7., 7., 7.]);
        // NaN never updates a bound; an all-NaN stroke keeps the +/-Infinity seeds.
        assert_eq!(stroke_bounds(&[f64::NAN, 2., 5., f64::NAN, 1., 8.]), [1., 2., 5., 8.]);
        let all_nan = stroke_bounds(&[f64::NAN, f64::NAN]);
        assert_eq!(all_nan, [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY]);
        // Signed zero: the first value seen wins because -0 is neither < nor > +0.
        let zero = stroke_bounds(&[0., -0., -0., 0.]);
        assert!(zero[0].is_sign_positive() && zero[1].is_sign_negative() && zero[2].is_sign_positive() && zero[3].is_sign_negative());
        assert_eq!(stroke_bounds(&[f64::INFINITY, f64::NEG_INFINITY]), [f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY]);
    }

    #[test]
    fn bounds_boundary_rejects_bad_arguments() {
        let mut out = [0.0f64; 4];
        let coords = [1.0f64, 2.0, 3.0, 4.0];
        unsafe {
            assert_eq!(editor_render_stroke_bounds(std::ptr::null(), 2, out.as_mut_ptr()), -1);
            assert_eq!(editor_render_stroke_bounds(coords.as_ptr(), 0, out.as_mut_ptr()), -1);
            assert_eq!(editor_render_stroke_bounds(coords.as_ptr(), MAX_STROKE_POINTS + 1, out.as_mut_ptr()), -1);
            assert_eq!(editor_render_stroke_bounds(coords.as_ptr(), 2, std::ptr::null_mut()), -1);
            assert_eq!(editor_render_stroke_bounds(coords.as_ptr(), 2, out.as_mut_ptr()), 1);
        }
        assert_eq!(out, [1., 2., 3., 4.]);
    }

    #[test]
    fn cover_widths_follow_legacy_rules() {
        assert_eq!(cover_width(COVER_ERASER_STROKE, 5., 10), COVER_SKIP);
        assert_eq!(cover_width(COVER_PEN, 5., 1), COVER_SKIP);
        assert_eq!(cover_width(COVER_PEN, 5., 0), COVER_SKIP);
        assert_eq!(cover_width(COVER_HIGHLIGHTER, 12., 0), COVER_SKIP);
        assert_eq!(cover_width(COVER_PEN, 4., 2), 4. * 1.35);
        assert_eq!(cover_width(COVER_HIGHLIGHTER, 12., 1), 12. * 1.05);
        assert_eq!(cover_width(COVER_PEN, 0.5, 2), 1.0);
        assert_eq!(cover_width(COVER_PEN, -3., 2), 1.0);
        assert_eq!(cover_width(COVER_PEN, f64::INFINITY, 2), f64::INFINITY);
        assert!(cover_width(COVER_PEN, f64::NAN, 2).is_nan());
        assert_eq!(cover_width(COVER_PEN, f64::NEG_INFINITY, 2), 1.0);
    }

    #[test]
    fn cover_boundary_rejects_bad_arguments() {
        let (tools, widths, counts, mut out) = ([0u8, 1, 2, 0], [2.0f64, 10.0, 3.0, 1.0], [5u32, 1, 9, 1], [0.0f64; 4]);
        unsafe {
            assert_eq!(editor_render_cover_widths(tools.as_ptr(), widths.as_ptr(), counts.as_ptr(), 0, out.as_mut_ptr()), -1);
            assert_eq!(editor_render_cover_widths(std::ptr::null(), widths.as_ptr(), counts.as_ptr(), 4, out.as_mut_ptr()), -1);
            assert_eq!(editor_render_cover_widths([9u8, 0, 0, 0].as_ptr(), widths.as_ptr(), counts.as_ptr(), 4, out.as_mut_ptr()), -1);
            assert_eq!(editor_render_cover_widths(tools.as_ptr(), widths.as_ptr(), counts.as_ptr(), 4, out.as_mut_ptr()), 1);
        }
        assert_eq!(out, [2.0 * 1.35, 10.0 * 1.05, COVER_SKIP, COVER_SKIP]);
    }
}
