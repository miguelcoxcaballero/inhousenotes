//! Variable-height grid rows: prefix sums once, then two binary searches per visible-window question.
//! Implemented by W2.5 (BUILD-PLAN.md).
// v5: rust-core-v5.js:folderRows

/// The measured row tracks of a CSS grid holding `count` items in `columns` columns.
#[derive(Clone, Debug, PartialEq)]
pub struct Rows {
    tops: Vec<f64>,
    bottoms: Vec<f64>,
    columns: usize,
    count: usize,
}

impl Rows {
    /// `heights` may hold extra trailing tracks (the New PDF button's row); they never invent items beyond `count`.
    pub fn new(heights: &[f64], gap: f64, columns: usize, count: usize) -> Option<Rows> {
        if columns == 0 || heights.len() > 1_000_000 || !gap.is_finite() || gap < 0.0 {
            return None;
        }
        let required = count.div_ceil(columns);
        if required > heights.len() || heights.iter().any(|h| !h.is_finite() || *h <= 0.0) {
            return None;
        }
        let (mut tops, mut bottoms, mut top) = (Vec::with_capacity(required), Vec::with_capacity(required), 0.0);
        for height in &heights[..required] {
            let bottom = top + height;
            if !bottom.is_finite() || !(bottom + gap).is_finite() {
                return None;
            }
            tops.push(top);
            bottoms.push(bottom);
            top = bottom + gap;
        }
        Some(Rows { tops, bottoms, columns, count })
    }

    /// Item indices `start..end` of the whole rows that touch `top - overscan ..= bottom + overscan`.
    pub fn range(&self, top: f64, bottom: f64, overscan: f64) -> Option<(usize, usize)> {
        let (first, last) = (top - overscan, bottom + overscan);
        if ![top, bottom, overscan, first, last].iter().all(|v| v.is_finite()) || bottom < top || overscan < 0.0 {
            return None;
        }
        let start = self.bottoms.partition_point(|b| *b < first);
        let end = self.tops.partition_point(|t| *t <= last).max(start);
        Some(((start * self.columns).min(self.count), (end * self.columns).min(self.count)))
    }

    /// Height of rows `first..end` without the gaps outside them (NaN for a range that does not exist).
    pub fn span(&self, first: usize, end: usize) -> f64 {
        if first > end || end > self.tops.len() {
            return f64::NAN;
        }
        if first == end { 0.0 } else { self.bottoms[end - 1] - self.tops[first] }
    }

    pub fn rows(&self) -> usize { self.tops.len() }
    pub fn columns(&self) -> usize { self.columns }
    pub fn count(&self) -> usize { self.count }
}

/// The numbers of a computed track list (`"138px 138px"`, `"repeat(…)"` never reaches here: computed values are
/// resolved). v5: `style.gridTemplateRows.match(/[\d.]+px/g)`
pub fn px_list(css: &str) -> Vec<f64> {
    let (bytes, mut out, mut i) = (css.as_bytes(), Vec::new(), 0);
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() || bytes[i] == b'.' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            if css[i..].starts_with("px") {
                // `parseFloat` stops at a second dot.
                let text = &css[start..i];
                let cut = text.match_indices('.').nth(1).map_or(text.len(), |(at, _)| at);
                if let Ok(v) = text[..cut].parse::<f64>() {
                    out.push(v);
                }
                i += 2;
            }
        } else {
            i += 1;
        }
    }
    out
}

/// Test command hook: `__IHN_TEST_API__.cmd('core.rows.<name>', args)`.
pub fn test_cmd(name: &str, args: &str) -> Option<String> {
    match name {
        // "h1,h2,…|gap|columns|count|top|bottom|overscan" → "start,end"
        "range" => {
            let f: Vec<&str> = args.trim_matches('"').split('|').collect();
            let n = |i: usize| f.get(i).and_then(|s| s.parse::<f64>().ok()).unwrap_or(f64::NAN);
            let heights: Vec<f64> = f.first()?.split(',').filter_map(|s| s.parse().ok()).collect();
            let rows = Rows::new(&heights, n(1), n(2) as usize, n(3) as usize)?;
            let (a, b) = rows.range(n(4), n(5), n(6))?;
            Some(format!("\"{a},{b}\""))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_cover_whole_rows_and_clamp_to_the_count() {
        let rows = Rows::new(&[100.0, 120.0, 100.0, 80.0], 10.0, 3, 10).unwrap();
        assert_eq!(rows.rows(), 4);
        assert_eq!(rows.range(0.0, 50.0, 0.0), Some((0, 3)));
        assert_eq!(rows.range(105.0, 109.0, 0.0), Some((3, 3))); // inside a gap: nothing
        assert_eq!(rows.range(105.0, 109.0, 10.0), Some((0, 6)));
        assert_eq!(rows.range(230.0, 1000.0, 0.0), Some((3, 10)));
        assert_eq!(rows.range(0.0, 1000.0, 240.0), Some((0, 10)));
        assert_eq!(rows.range(5000.0, 6000.0, 0.0), Some((10, 10)));
        assert_eq!(rows.range(10.0, 0.0, 0.0), None);
        assert_eq!(rows.range(0.0, 10.0, -1.0), None);
    }

    #[test]
    fn span_is_the_height_without_outside_gaps() {
        let rows = Rows::new(&[100.0, 120.0, 100.0], 10.0, 2, 6).unwrap();
        assert_eq!(rows.span(0, 0), 0.0);
        assert_eq!(rows.span(0, 1), 100.0);
        assert_eq!(rows.span(0, 3), 340.0);
        assert_eq!(rows.span(1, 3), 230.0);
        assert!(rows.span(2, 1).is_nan() && rows.span(0, 4).is_nan());
    }

    #[test]
    fn invalid_tracks_are_refused_and_extra_tracks_ignored() {
        assert!(Rows::new(&[100.0], 0.0, 0, 1).is_none());
        assert!(Rows::new(&[100.0], -1.0, 1, 1).is_none());
        assert!(Rows::new(&[100.0], 0.0, 1, 2).is_none());
        assert!(Rows::new(&[100.0, 0.0], 0.0, 1, 2).is_none());
        let rows = Rows::new(&[100.0, 100.0, 300.0], 0.0, 2, 3).unwrap();
        assert_eq!(rows.rows(), 2);
        assert_eq!(rows.range(0.0, 9999.0, 0.0), Some((0, 3)));
        assert_eq!(Rows::new(&[], 0.0, 4, 0).unwrap().range(0.0, 10.0, 0.0), Some((0, 0)));
    }

    #[test]
    fn computed_track_lists_are_read_like_the_js_regex() {
        assert_eq!(px_list("138px 140.5px 138px"), [138.0, 140.5, 138.0]);
        assert_eq!(px_list("none"), Vec::<f64>::new());
        assert_eq!(px_list("[a] 10px [b] 2.5px"), [10.0, 2.5]);
    }
}
