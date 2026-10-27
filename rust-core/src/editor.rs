//! Exact polyline hit testing; no point reduction or changes to stored ink.
fn hits(points: &[f64], x: f64, y: f64, radius: f64) -> bool {
    let radius_sq = radius * radius;
    points.windows(4).step_by(2).any(|segment| {
        let dx = segment[2] - segment[0];
        let dy = segment[3] - segment[1];
        let denominator = dx * dx + dy * dy;
        let t = if denominator == 0.0 { 0.0 } else {
            (((x - segment[0]) * dx + (y - segment[1]) * dy) / denominator).clamp(0.0, 1.0)
        };
        let ex = x - (segment[0] + t * dx);
        let ey = y - (segment[1] + t * dy);
        ex * ex + ey * ey <= radius_sq
    })
}
/// Host-owned aligned f64 x/y pairs, retained unchanged across hit queries.
#[no_mangle]
pub unsafe extern "C" fn editor_stroke_hit(ptr: *const f64, count: usize, x: f64, y: f64, radius: f64) -> i32 {
    if ptr.is_null() || count < 2 || count > 262144 || !x.is_finite() || !y.is_finite() || !radius.is_finite() || radius < 0.0 {
        return -1;
    }
    hits(std::slice::from_raw_parts(ptr, count * 2), x, y, radius) as i32
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn segment_endpoints_and_zero_length() {
        assert!(hits(&[0.,0.,10.,0.],5.,2.,2.));
        assert!(!hits(&[0.,0.,10.,0.],5.,2.01,2.));
        assert!(hits(&[0.,0.,0.,0.],0.,0.,0.));
        assert!(!hits(&[0.,0.,0.,0.],1.,0.,0.));
        assert!(hits(&[0.,0.,0.,10.,10.,10.],8.,10.,0.));
    }
}
