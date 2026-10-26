//! Editor input geometry: the resident stroke store and the stroke-eraser hit queries.
//!
//! Strokes are uploaded once (Float64 x/y pairs) and stay in WebAssembly memory; queries take scalars
//! and slot lists and return indices. All arithmetic reproduces the original JavaScript operation order,
//! including V8's `Math.hypot`, so every boolean decision is identical to the pre-port code.
use std::cell::RefCell;

pub fn call(_method: &str, _input: &[u8]) -> Option<Result<Vec<u8>, String>> { None }

/// Total retained coordinates (x/y pairs). Exceeding it makes the host drop the store and re-upload lazily.
const MAX_STORED_POINTS: usize = 6 * 1024 * 1024;
/// A single stroke can be at most this long.
const MAX_STROKE_POINTS: usize = 4 * 1024 * 1024;
/// Scratch buffers larger than this are released after use.
const SCRATCH_KEEP_POINTS: usize = 64 * 1024;

#[derive(Default)]
struct Slot {
    live: bool,
    /// minX, minY, maxX, maxY exactly as the original `getStrokeBounds` produced them.
    bounds: [f64; 4],
    /// Retained x/y pairs; valid only while `resident == current generation id`.
    points: Vec<f64>,
    resident: u32,
}

#[derive(Default)]
struct Store {
    slots: Vec<Slot>,
    free: Vec<u32>,
    scratch: Vec<f64>,
    stored: usize,
}

thread_local! { static STORE: RefCell<Store> = RefCell::new(Store::default()); }

/// JavaScript `Math.max(a, b)`: NaN poisons the result.
#[inline] fn js_max(a: f64, b: f64) -> f64 { if a.is_nan() || b.is_nan() { f64::NAN } else if a > b { a } else { b } }
/// JavaScript `Math.min(a, b)`.
#[inline] fn js_min(a: f64, b: f64) -> f64 { if a.is_nan() || b.is_nan() { f64::NAN } else if a < b { a } else { b } }

/// V8's two-argument `Math.hypot` (scaled, Kahan-compensated), bit for bit.
pub fn v8_hypot(a: f64, b: f64) -> f64 {
    let (x, y) = (a.abs(), b.abs());
    let mut max = 0.0;
    if x > max { max = x; }
    if y > max { max = y; }
    if max == f64::INFINITY { return f64::INFINITY; }
    if a.is_nan() || b.is_nan() { return f64::NAN; }
    if max == 0.0 { return 0.0; }
    let (mut sum, mut compensation) = (0.0f64, 0.0f64);
    for value in [x, y] {
        let n = value / max;
        let summand = n * n - compensation;
        let preliminary = sum + summand;
        compensation = (preliminary - sum) - summand;
        sum = preliminary;
    }
    sum.sqrt() * max
}

/// Original `distancePointToSegment(...) <= radius`, with a squared-distance shortcut that only decides
/// cases far from the threshold; anything within 1e-9 (relative) of it is settled by the exact formula.
#[inline]
fn within(d_x: f64, d_y: f64, radius: f64) -> bool {
    let d2 = d_x * d_x + d_y * d_y;
    let r2 = radius * radius;
    if d2 > 1e-250 && d2 < 1e250 && r2 > 1e-250 && r2 < 1e250 {
        if d2 < r2 * (1.0 - 1e-9) { return true; }
        if d2 > r2 * (1.0 + 1e-9) { return false; }
    }
    v8_hypot(d_x, d_y) <= radius
}

#[inline]
fn segment_within(px: f64, py: f64, x1: f64, y1: f64, x2: f64, y2: f64, radius: f64) -> bool {
    let dx = x2 - x1;
    let dy = y2 - y1;
    if dx == 0.0 && dy == 0.0 { return within(px - x1, py - y1, radius); }
    let t = ((px - x1) * dx + (py - y1) * dy) / (dx * dx + dy * dy);
    let clamped = js_max(0.0, js_min(1.0, t));
    let cx = x1 + clamped * dx;
    let cy = y1 + clamped * dy;
    within(px - cx, py - cy, radius)
}

/// Exact original `distancePointToSegment`.
pub fn segment_distance(px: f64, py: f64, x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
    let dx = x2 - x1;
    let dy = y2 - y1;
    if dx == 0.0 && dy == 0.0 { return v8_hypot(px - x1, py - y1); }
    let t = ((px - x1) * dx + (py - y1) * dy) / (dx * dx + dy * dy);
    let clamped = js_max(0.0, js_min(1.0, t));
    let cx = x1 + clamped * dx;
    let cy = y1 + clamped * dy;
    v8_hypot(px - cx, py - cy)
}

/// Original `getStrokeBounds` scan. NaN coordinates are skipped by the comparisons, as before.
pub fn compute_bounds(points: &[f64]) -> [f64; 4] {
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for pair in points.chunks_exact(2) {
        let (x, y) = (pair[0], pair[1]);
        if x < min_x { min_x = x; }
        if y < min_y { min_y = y; }
        if x > max_x { max_x = x; }
        if y > max_y { max_y = y; }
    }
    [min_x, min_y, max_x, max_y]
}

/// Original `strokeMayIntersectCircle` given precomputed bounds and half the stroke width.
#[inline]
fn may_intersect(bounds: &[f64; 4], x: f64, y: f64, radius: f64, extra: f64, half_width: f64) -> bool {
    let pad = js_max(0.0, radius + extra + half_width);
    !(x < bounds[0] - pad || x > bounds[2] + pad || y < bounds[1] - pad || y > bounds[3] + pad)
}

/// Original `strokeHitsPoint` exact phase over retained points.
fn exact_hit(points: &[f64], x: f64, y: f64, radius: f64) -> bool {
    match points.len() {
        0 | 1 => false,
        2 => within(points[0] - x, points[1] - y, radius),
        _ => points.windows(4).step_by(2).any(|s| segment_within(x, y, s[0], s[1], s[2], s[3], radius)),
    }
}

impl Store {
    fn slot(&self, id: u32) -> Option<&Slot> { self.slots.get(id as usize).filter(|s| s.live && id != 0) }
    fn alloc(&mut self) -> u32 {
        if self.slots.is_empty() { self.slots.push(Slot::default()); } // id 0 is never handed out
        if let Some(id) = self.free.pop() {
            self.slots[id as usize] = Slot { live: true, ..Slot::default() };
            return id;
        }
        self.slots.push(Slot { live: true, ..Slot::default() });
        (self.slots.len() - 1) as u32
    }
    fn release(&mut self, id: u32) {
        if id == 0 { return; }
        if let Some(slot) = self.slots.get_mut(id as usize) {
            if slot.live {
                self.stored -= slot.points.len() / 2;
                *slot = Slot::default();
                self.free.push(id);
            }
        }
    }
    fn clear(&mut self) { *self = Store::default(); }
    fn reserve(&mut self, count: usize) -> *mut f64 {
        if count == 0 || count > MAX_STROKE_POINTS { return std::ptr::null_mut(); }
        self.scratch.clear();
        self.scratch.resize(count * 2, 0.0);
        self.scratch.as_mut_ptr()
    }
    /// Computes bounds from the scratch buffer; keeps the points when `generation` is non-zero.
    fn commit(&mut self, id: u32, count: usize, generation: u32) -> bool {
        if self.slot(id).is_none() || count == 0 || self.scratch.len() != count * 2 { return false; }
        let bounds = compute_bounds(&self.scratch);
        let (retained, new_len) = (self.slots[id as usize].points.len() / 2, if generation != 0 { count } else { 0 });
        if self.stored - retained + new_len > MAX_STORED_POINTS { return false; }
        let slot = &mut self.slots[id as usize];
        slot.bounds = bounds;
        slot.points.clear();
        if generation != 0 { slot.points.extend_from_slice(&self.scratch); } else { slot.points.shrink_to_fit(); }
        slot.resident = generation;
        self.stored = self.stored - retained + new_len;
        if self.scratch.capacity() > SCRATCH_KEEP_POINTS * 2 { self.scratch = Vec::new(); }
        true
    }
}

/// Outcome of a circle query over a list of strokes.
#[derive(Debug, Default, PartialEq)]
pub struct Query { pub hits: Vec<u32>, pub pending: Vec<u32> }

/// `slots[i]` is the stroke's slot (0 = skip). With `exact`, hits are listed in descending index order
/// (the order the eraser removes them) and strokes that pass the bounds test but have no fresh retained
/// points are listed in `pending`; without it, hits are the bounds candidates in ascending order.
fn circle_query(store: &Store, slots: &[u32], half_widths: &[f64], x: f64, y: f64, radius: f64, extra: f64, generation: u32, exact: bool) -> Query {
    let mut query = Query::default();
    let mut visit = |i: usize| {
        let Some(slot) = store.slot(slots[i]) else { return };
        if !may_intersect(&slot.bounds, x, y, radius, extra, half_widths[i]) { return; }
        if !exact { query.hits.push(i as u32); return; }
        if slot.resident != generation { query.pending.push(i as u32); return; }
        if exact_hit(&slot.points, x, y, radius) { query.hits.push(i as u32); }
    };
    if exact { for i in (0..slots.len()).rev() { visit(i); } } else { for i in 0..slots.len() { visit(i); } }
    query
}

fn with_store<R>(f: impl FnOnce(&mut Store) -> R) -> R { STORE.with(|s| f(&mut s.borrow_mut())) }

#[no_mangle] pub extern "C" fn editor_input_version() -> u32 { 1 }
#[no_mangle] pub extern "C" fn editor_input_slot_alloc() -> u32 { with_store(|s| s.alloc()) }
#[no_mangle] pub extern "C" fn editor_input_slot_free(id: u32) { with_store(|s| s.release(id)) }
#[no_mangle] pub extern "C" fn editor_input_store_clear() { with_store(|s| s.clear()) }
#[no_mangle] pub extern "C" fn editor_input_stored_points() -> usize { with_store(|s| s.stored) }
/// Scratch buffer for `count` x/y pairs, or null when `count` is invalid. Valid until the next call into the store.
#[no_mangle] pub extern "C" fn editor_input_scratch(count: usize) -> *mut f64 { with_store(|s| s.reserve(count)) }
/// Commit the scratch pairs to `slot`: bounds always, points retained for `generation` (0 = bounds only).
/// Returns 0 on failure (invalid slot or capacity) and 1 on success.
#[no_mangle] pub extern "C" fn editor_input_commit(slot: u32, count: usize, generation: u32) -> i32 {
    with_store(|s| s.commit(slot, count, generation)) as i32
}
/// Writes minX, minY, maxX, maxY to `out` (4 f64). Returns 0 when the slot is not live.
#[no_mangle] pub unsafe extern "C" fn editor_input_bounds(slot: u32, out: *mut f64) -> i32 {
    if out.is_null() { return 0; }
    with_store(|s| match s.slot(slot) {
        Some(slot) => { std::ptr::copy_nonoverlapping(slot.bounds.as_ptr(), out, 4); 1 }
        None => 0,
    })
}
/// Original `strokeMayIntersectCircle` for one slot (1/0, or -1 for an invalid slot).
#[no_mangle] pub extern "C" fn editor_input_may_intersect(slot: u32, x: f64, y: f64, radius: f64, extra: f64, half_width: f64) -> i32 {
    with_store(|s| match s.slot(slot) { Some(slot) => may_intersect(&slot.bounds, x, y, radius, extra, half_width) as i32, None => -1 })
}
/// Exact test of the `count` scratch pairs just uploaded (a stroke the store could not retain): 1 hit, 0 miss, -1 invalid.
#[no_mangle] pub extern "C" fn editor_input_scratch_hit(count: usize, x: f64, y: f64, radius: f64) -> i32 {
    with_store(|s| if count == 0 || s.scratch.len() != count * 2 { -1 } else { exact_hit(&s.scratch, x, y, radius) as i32 })
}
/// Original `distancePointToSegment`.
#[no_mangle] pub extern "C" fn editor_input_segment_distance(px: f64, py: f64, x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
    segment_distance(px, py, x1, y1, x2, y2)
}
/// Circle query over `n` strokes. `out` receives hits then pending as u32 lists of capacity `n` each:
/// `out[0..hits]` and `out[n..n+pending]`; `counts` receives [hits, pending]. Returns 0 on bad arguments.
#[no_mangle]
pub unsafe extern "C" fn editor_input_circle_query(slots: *const u32, half_widths: *const f64, n: usize, x: f64, y: f64, radius: f64,
    extra: f64, generation: u32, exact: i32, out: *mut u32, counts: *mut u32) -> i32 {
    if slots.is_null() || half_widths.is_null() || out.is_null() || counts.is_null() || n == 0 || n > 16 * 1024 * 1024 { return 0; }
    let query = with_store(|s| circle_query(s, std::slice::from_raw_parts(slots, n), std::slice::from_raw_parts(half_widths, n),
        x, y, radius, extra, generation, exact != 0));
    std::ptr::copy_nonoverlapping(query.hits.as_ptr(), out, query.hits.len());
    std::ptr::copy_nonoverlapping(query.pending.as_ptr(), out.add(n), query.pending.len());
    *counts = query.hits.len() as u32;
    *counts.add(1) = query.pending.len() as u32;
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn upload(store: &mut Store, pts: &[(f64, f64)], generation: u32) -> u32 {
        let id = store.alloc();
        let ptr = store.reserve(pts.len());
        let buffer = unsafe { std::slice::from_raw_parts_mut(ptr, pts.len() * 2) };
        for (i, p) in pts.iter().enumerate() { buffer[i * 2] = p.0; buffer[i * 2 + 1] = p.1; }
        assert!(store.commit(id, pts.len(), generation));
        id
    }

    #[test]
    fn hypot_matches_known_v8_values() {
        assert_eq!(v8_hypot(3.0, 4.0), 5.0);
        assert_eq!(v8_hypot(0.0, 0.0), 0.0);
        assert_eq!(v8_hypot(f64::NAN, f64::INFINITY), f64::INFINITY);
        assert!(v8_hypot(f64::NAN, 1.0).is_nan());
        assert_eq!(v8_hypot(-5.0, 12.0), 13.0);
    }

    #[test]
    fn bounds_skip_nan_like_the_original_comparisons() {
        assert_eq!(compute_bounds(&[1.0, 5.0, -2.0, f64::NAN, 4.0, 0.0]), [-2.0, 0.0, 4.0, 5.0]);
        let all_nan = compute_bounds(&[f64::NAN, f64::NAN]);
        assert_eq!(all_nan, [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY]);
    }

    #[test]
    fn segment_endpoints_and_zero_length() {
        assert!(segment_within(5.0, 2.0, 0.0, 0.0, 10.0, 0.0, 2.0));
        assert!(!segment_within(5.0, 2.01, 0.0, 0.0, 10.0, 0.0, 2.0));
        assert!(segment_within(0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0));
        assert!(!segment_within(1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0));
        assert!(segment_within(8.0, 10.0, 0.0, 0.0, 0.0, 10.0, 0.0) == false);
        assert!(segment_within(8.0, 10.0, 0.0, 10.0, 10.0, 10.0, 0.0));
    }

    #[test]
    fn shortcut_agrees_with_exact_formula_on_threshold_cases() {
        let mut state = 1u64;
        let mut next = || { state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407); ((state >> 11) as f64) / ((1u64 << 53) as f64) };
        for _ in 0..200_000 {
            let (x1, y1, x2, y2) = (next() * 100.0, next() * 100.0, next() * 100.0, next() * 100.0);
            let (px, py) = (next() * 100.0, next() * 100.0);
            let exact = segment_distance(px, py, x1, y1, x2, y2);
            for r in [exact, f64::from_bits(exact.to_bits() + 1), f64::from_bits(exact.to_bits().saturating_sub(1)), exact * 1.000000001, exact * 0.999999999, next() * 10.0] {
                assert_eq!(segment_within(px, py, x1, y1, x2, y2, r), exact <= r, "{px} {py} {x1} {y1} {x2} {y2} r={r}");
            }
        }
    }

    #[test]
    fn query_orders_hits_and_reports_stale_strokes() {
        let mut store = Store::default();
        let a = upload(&mut store, &[(0.0, 0.0), (100.0, 0.0)], 7);
        let b = upload(&mut store, &[(0.0, 50.0), (100.0, 50.0)], 7);
        let c = upload(&mut store, &[(0.0, 2.0), (100.0, 2.0)], 7);
        let slots = [a, 0, b, c];
        let widths = [0.0; 4];
        let q = circle_query(&store, &slots, &widths, 50.0, 1.0, 1.5, 2.0, 7, true);
        assert_eq!(q, Query { hits: vec![3, 0], pending: vec![] });
        let q = circle_query(&store, &slots, &widths, 50.0, 1.0, 1.5, 2.0, 8, true);
        assert_eq!(q, Query { hits: vec![], pending: vec![3, 0] });
        let q = circle_query(&store, &slots, &widths, 50.0, 1.0, 1.5, 2.0, 7, false);
        assert_eq!(q.hits, vec![0, 3]);
    }

    #[test]
    fn single_point_stroke_uses_point_distance_and_bounds_only_slots_are_pending() {
        let mut store = Store::default();
        let a = upload(&mut store, &[(10.0, 10.0)], 3);
        let b = upload(&mut store, &[(10.0, 10.0), (11.0, 10.0)], 0);
        let q = circle_query(&store, &[a, b], &[0.0, 0.0], 10.0, 13.0, 3.0, 2.0, 3, true);
        assert_eq!(q, Query { hits: vec![0], pending: vec![1] });
        let q = circle_query(&store, &[a], &[0.0], 10.0, 13.1, 3.0, 2.0, 3, true);
        assert!(q.hits.is_empty());
    }

    #[test]
    fn release_reuses_slots_and_counts_points() {
        let mut store = Store::default();
        let a = upload(&mut store, &[(0.0, 0.0), (1.0, 1.0)], 1);
        assert_eq!(store.stored, 2);
        store.release(a);
        assert_eq!(store.stored, 0);
        assert!(store.slot(a).is_none());
        let b = store.alloc();
        assert_eq!(a, b);
        assert!(!store.commit(0, 1, 1));
        assert!(store.reserve(0).is_null());
    }

    #[test]
    fn capacity_limit_rejects_commit_without_corrupting_the_store() {
        let mut store = Store::default();
        let id = store.alloc();
        store.stored = MAX_STORED_POINTS;
        store.reserve(10);
        assert!(!store.commit(id, 10, 1));
        assert!(store.commit(id, 10, 0));
    }
}
