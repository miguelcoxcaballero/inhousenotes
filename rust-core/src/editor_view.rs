//! Editor viewport logic. Unit 1: page layout cache and visible-window planner.
//!
//! The host (JS) owns the DOM and the `state.pages` objects. It uploads each page's raw width, height and a
//! calendar-panel flag once per structure change; this module builds the vertical layout (tops, bottoms,
//! lefts, rights) and answers every per-scroll question (visible range, directly visible pages, activation
//! order, which previews and pending activations to prune). All floating point expressions keep the operation order of the original
//! JavaScript so results are bit-identical.
use std::cell::RefCell;

pub fn call(_method: &str, _input: &[u8]) -> Option<Result<Vec<u8>, String>> { None }

const MAX_PAGES: usize = 1 << 22;

/// JS `Math.round`: ties go toward +infinity.
fn js_round(x: f64) -> f64 {
    if !x.is_finite() { return x; }
    let floor = x.floor();
    if x - floor >= 0.5 { floor + 1.0 } else { floor }
}

/// `normalizePageDimension`: non-finite values (missing, NaN, non-numbers) use the fallback unchanged.
fn normalize_dimension(value: f64, fallback: f64) -> f64 {
    if !value.is_finite() { return fallback; }
    let rounded = js_round(value);
    if 64.0 > rounded { 64.0 } else { rounded }
}

/// JS truthiness of a number.
fn truthy(x: f64) -> bool { x != 0.0 && !x.is_nan() }

#[derive(Default)]
pub struct Layout {
    pages: usize,
    padding: f64,
    tops: Vec<f64>,
    bottoms: Vec<f64>,
    lefts: Vec<f64>,
    rights: Vec<f64>,
    heights: Vec<f64>,
    content_bottom: f64,
    bounds: [f64; 4],
}

impl Layout {
    /// `raw_widths` / `raw_heights` hold the unnormalised page values (NaN when the page has none).
    pub fn build(raw_widths: &[f64], raw_heights: &[f64], calendar: &[u8], padding: f64, gap: f64,
        a4_width: f64, a4_height: f64, calendar_space: f64) -> Layout {
        let count = raw_widths.len();
        let mut layout = Layout { pages: count, padding, ..Layout::default() };
        let widths: Vec<f64> = raw_widths.iter().map(|&w| normalize_dimension(w, a4_width)).collect();
        let mut max_width = a4_width;
        for &width in &widths { if width > max_width { max_width = width; } }
        let mut y = padding;
        if count == 0 {
            layout.tops.push(padding);
            layout.bottoms.push(padding + a4_height);
            layout.lefts.push(padding);
            layout.rights.push(padding + a4_width);
            layout.heights.push(a4_height);
            y = padding + a4_height;
        } else {
            layout.tops.reserve(count); layout.bottoms.reserve(count); layout.lefts.reserve(count);
            layout.rights.reserve(count); layout.heights.reserve(count);
            for index in 0..count {
                let width = widths[index];
                let height = normalize_dimension(raw_heights[index], a4_height);
                let left = padding + (max_width - width) / 2.0;
                layout.tops.push(y);
                layout.bottoms.push(y + height);
                layout.lefts.push(left);
                layout.rights.push(left + width);
                layout.heights.push(height);
                let space = if calendar[index] != 0 { calendar_space } else { 0.0 };
                y += height + space;
                if index < count - 1 { y += gap; }
            }
        }
        layout.content_bottom = y;
        layout.bounds = [padding, padding, padding + max_width, y];
        layout
    }

    /// `metrics.pageCount`: an empty document still lays out one placeholder page.
    pub fn page_count(&self) -> usize { if self.pages == 0 { 1 } else { self.pages } }

    /// `findFirstPageWhoseBottomReaches`; -1 when none.
    pub fn first_bottom_reaching(&self, y: f64) -> i64 {
        let (mut low, mut high) = (0usize, self.bottoms.len());
        while low < high {
            let mid = (low + high) >> 1;
            if self.bottoms[mid] < y { low = mid + 1; } else { high = mid; }
        }
        if low < self.bottoms.len() { low as i64 } else { -1 }
    }

    /// `findLastPageWhoseTopPrecedes`; -1 when none.
    pub fn last_top_preceding(&self, y: f64) -> i64 {
        let (mut low, mut high) = (0usize, self.tops.len());
        while low < high {
            let mid = (low + high) >> 1;
            if self.tops[mid] <= y { low = mid + 1; } else { high = mid; }
        }
        if low > 0 { low as i64 - 1 } else { -1 }
    }

    /// `getPageTopOffset` (idx clamping included); NaN when the clamped index is not a page index.
    pub fn top_offset(&self, page_index: f64, gap: f64) -> f64 {
        let upper = self.pages as f64;
        let clamped = if page_index.is_nan() { f64::NAN } else {
            let low = if upper < page_index { upper } else { page_index };
            if 0.0 > low { 0.0 } else { low }
        };
        if clamped < self.tops.len() as f64 {
            if clamped.fract() != 0.0 { return f64::NAN; }
            return self.tops[clamped as usize];
        }
        self.content_bottom + gap
    }

    /// Centre of a page in layout space: top + height / 2.
    fn center(&self, index: usize) -> f64 { self.tops[index] + self.heights[index] / 2.0 }
}

/// `getVisiblePageRange`; start > end signals an empty range.
pub fn visible_range(layout: &Layout, pan_y: f64, zoom: f64, rect_height: f64, buffer: f64) -> (f64, f64) {
    if !truthy(rect_height) { return (0.0, -1.0); }
    let view_top = (-pan_y) / zoom;
    let view_bottom = (rect_height - pan_y) / zoom;
    let first = layout.first_bottom_reaching(view_top);
    let last = layout.last_top_preceding(view_bottom);
    if first < 0 || last < first {
        if layout.pages == 0 { return (0.0, -1.0); }
        let nearest = if view_top < layout.padding { 0.0 } else { layout.pages as f64 - 1.0 };
        return (nearest, nearest);
    }
    let start = (first as f64 - buffer).max(0.0);
    let end = (last as f64 + buffer).min(layout.pages as f64 - 1.0);
    (start, end)
}

/// `getViewportVisiblePageIndices` for an extra vertical buffer of zero.
fn viewport_indices(layout: &Layout, pan_x: f64, pan_y: f64, zoom: f64, rect_width: f64, rect_height: f64) -> Vec<usize> {
    if !truthy(rect_width) || !truthy(rect_height) { return Vec::new(); }
    let extra_y = (rect_height * 0.0).max(0.0);
    let extra_x = (rect_width * 0.25).max(0.0);
    let view_top = (-pan_y - extra_y) / zoom;
    let view_bottom = (rect_height - pan_y + extra_y) / zoom;
    let view_left = (-pan_x - extra_x) / zoom;
    let view_right = (rect_width - pan_x + extra_x) / zoom;
    let first = layout.first_bottom_reaching(view_top);
    let last = layout.last_top_preceding(view_bottom);
    let mut indices = Vec::new();
    let from = first.max(0);
    let to = last.min(layout.page_count() as i64 - 1);
    let mut index = from;
    while index <= to {
        let i = index as usize;
        if layout.rights[i] >= view_left && layout.lefts[i] <= view_right { indices.push(i); }
        index += 1;
    }
    indices
}

/// `expandPageIndices`: sorted unique neighbours within `pages`.
fn expand_indices(indices: &[usize], buffer: f64, pages: usize) -> Vec<usize> {
    let safe = if buffer.is_finite() { js_round(buffer).max(0.0) } else { 0.0 };
    let mut marks = vec![false; pages];
    for &index in indices {
        let low = (index as f64 - safe).max(0.0);
        let high = (index as f64 + safe).min(pages as f64 - 1.0);
        let mut i = low;
        while i <= high { marks[i as usize] = true; i += 1.0; }
    }
    marks.iter().enumerate().filter(|(_, &m)| m).map(|(i, _)| i).collect()
}

#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    pub range_start: i32,
    pub range_end: i32,
    pub direct: Vec<i32>,
    pub ordered: Vec<i32>,
    pub preview_only: Vec<i32>,
    pub prune: Vec<i32>,
}

pub struct PlanInput<'a> {
    pub pan_x: f64, pub pan_y: f64, pub zoom: f64, pub rect_width: f64, pub rect_height: f64,
    pub buffer: f64, pub max_active: usize,
    /// NaN when the selection / active page is not a finite page index.
    pub selection: f64, pub active_page: f64,
    pub pending: &'a [i32],
}

/// Planning half of `updateVisiblePages`. `None` means nothing is visible and the update must stop.
pub fn plan(layout: &Layout, input: &PlanInput) -> Option<Plan> {
    let pages = layout.pages;
    if pages == 0 { return None; }
    let (mut range_start, mut range_end) = visible_range(layout, input.pan_y, input.zoom, input.rect_height, input.buffer);
    let direct_range = visible_range(layout, input.pan_y, input.zoom, input.rect_height, 0.0);
    let dom_direct = viewport_indices(layout, input.pan_x, input.pan_y, input.zoom, input.rect_width, input.rect_height);
    let view_center_y = (-input.pan_y + input.rect_height / 2.0) / input.zoom;
    let distance = |index: usize| (layout.center(index) - view_center_y).abs();
    let (direct, visible): (Vec<usize>, Vec<usize>);
    if !dom_direct.is_empty() {
        let mut sorted: Vec<(usize, f64)> = dom_direct.iter().map(|&i| (i, distance(i))).collect();
        sorted.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        sorted.truncate(input.max_active);
        direct = sorted.into_iter().map(|(i, _)| i).collect();
        visible = expand_indices(&direct, input.buffer, pages);
        range_start = visible[0] as f64;
        range_end = visible[visible.len() - 1] as f64;
    } else {
        if range_end < range_start { return None; }
        let mut list = Vec::new();
        let mut i = range_start;
        while i <= range_end { list.push(i as usize); i += 1.0; }
        visible = list;
        let mut list = Vec::new();
        if direct_range.1 >= direct_range.0 {
            let mut i = direct_range.0;
            while i <= direct_range.1 { list.push(i as usize); i += 1.0; }
        }
        direct = list;
    }
    if visible.is_empty() { return None; }

    let mut candidates: Vec<(usize, f64)> = visible.iter().map(|&i| (i, distance(i))).collect();
    candidates.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut ordered: Vec<i32> = Vec::new();
    let add = |ordered: &mut Vec<i32>, index: i32| { if !ordered.contains(&index) { ordered.push(index); } };
    for candidate in [input.selection, input.active_page] {
        if candidate.is_finite() && candidate >= range_start && candidate <= range_end { add(&mut ordered, candidate as i32); }
    }
    for &index in &direct {
        if ordered.contains(&(index as i32)) { continue; }
        if ordered.len() >= input.max_active { break; }
        add(&mut ordered, index as i32);
    }
    for &(index, _) in &candidates {
        if ordered.len() >= input.max_active { break; }
        add(&mut ordered, index as i32);
    }
    let preview_only = visible.iter().map(|&i| i as i32).filter(|i| !ordered.contains(i)).collect();
    let prune = input.pending.iter().copied().filter(|i| !ordered.contains(i)).collect();
    Some(Plan { range_start: range_start as i32, range_end: range_end as i32,
        direct: direct.iter().map(|&i| i as i32).collect(), ordered, preview_only, prune })
}

// ---------------------------------------------------------------------------------------------------------------
// WASM boundary. One editor per module instance, so the layout lives in module state.
// ---------------------------------------------------------------------------------------------------------------
thread_local! {
    static LAYOUT: RefCell<Layout> = RefCell::new(Layout::default());
    static PAGE_VALUES: RefCell<Vec<f64>> = const { RefCell::new(Vec::new()) };
    static CALENDAR_FLAGS: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static INT_INPUT: RefCell<Vec<i32>> = const { RefCell::new(Vec::new()) };
    static INT_OUTPUT: RefCell<Vec<i32>> = const { RefCell::new(Vec::new()) };
    static FLOAT_OUTPUT: RefCell<[f64; 2]> = const { RefCell::new([0.0; 2]) };
}

#[no_mangle]
pub extern "C" fn editor_view_version() -> u32 { 1 }

/// Reserve the page-value upload buffer: `2 * count` f64 (all widths, then all heights). Null when invalid.
#[no_mangle]
pub extern "C" fn ev_layout_values(count: usize) -> *mut f64 {
    if count > MAX_PAGES { return std::ptr::null_mut(); }
    PAGE_VALUES.with(|v| { let mut v = v.borrow_mut(); v.resize(count * 2, f64::NAN); v.as_mut_ptr() })
}

/// Reserve the per-page calendar-panel flag upload buffer (`count` bytes).
#[no_mangle]
pub extern "C" fn ev_layout_flags(count: usize) -> *mut u8 {
    if count > MAX_PAGES { return std::ptr::null_mut(); }
    CALENDAR_FLAGS.with(|v| { let mut v = v.borrow_mut(); v.resize(count, 0); v.as_mut_ptr() })
}

/// Build the layout from the uploaded buffers. Returns 1 on success.
#[no_mangle]
pub extern "C" fn ev_layout_build(count: usize, padding: f64, gap: f64, a4_width: f64, a4_height: f64, calendar_space: f64) -> u32 {
    if count > MAX_PAGES || ![padding, gap, a4_width, a4_height, calendar_space].iter().all(|v| v.is_finite()) { return 0; }
    let layout = PAGE_VALUES.with(|values| CALENDAR_FLAGS.with(|flags| {
        let (values, flags) = (values.borrow(), flags.borrow());
        if values.len() < count * 2 || flags.len() < count { return None; }
        Some(Layout::build(&values[..count], &values[count..count * 2], &flags[..count], padding, gap, a4_width, a4_height, calendar_space))
    }));
    match layout { Some(layout) => { LAYOUT.with(|l| *l.borrow_mut() = layout); 1 } None => 0 }
}

/// 0 content bottom, 1..=4 bounds left/top/right/bottom, 5 metrics page count, 6 real page count.
#[no_mangle]
pub extern "C" fn ev_layout_scalar(which: u32) -> f64 {
    LAYOUT.with(|l| { let l = l.borrow(); match which {
        0 => l.content_bottom, 1..=4 => l.bounds[which as usize - 1], 5 => l.page_count() as f64, 6 => l.pages as f64, _ => f64::NAN } })
}

#[no_mangle]
pub extern "C" fn ev_layout_top(page_index: f64, gap: f64) -> f64 { LAYOUT.with(|l| l.borrow().top_offset(page_index, gap)) }

/// Pointer to two f64: range start and end. Valid until the next call.
#[no_mangle]
pub extern "C" fn ev_range(pan_y: f64, zoom: f64, rect_height: f64, buffer: f64) -> *const f64 {
    let (start, end) = LAYOUT.with(|l| visible_range(&l.borrow(), pan_y, zoom, rect_height, buffer));
    FLOAT_OUTPUT.with(|o| { *o.borrow_mut() = [start, end]; o.borrow().as_ptr() })
}

/// Reserve the i32 input list: the pending activation keys.
#[no_mangle]
pub extern "C" fn ev_plan_input(len: usize) -> *mut i32 {
    if len > MAX_PAGES { return std::ptr::null_mut(); }
    INT_INPUT.with(|v| { let mut v = v.borrow_mut(); v.resize(len, 0); v.as_mut_ptr() })
}

/// Plan one visible-page update. Returns 0 when there is nothing to do, otherwise the number of i32 written to
/// the output block (see `ev_plan_output`): range start, range end, then the counts of direct, ordered,
/// preview-only and prune lists, followed by those lists in that order. Releasing canvases stays with the host
/// because earlier DOM steps of the same update can activate more pages.
#[no_mangle]
pub extern "C" fn ev_plan(pan_x: f64, pan_y: f64, zoom: f64, rect_width: f64, rect_height: f64, buffer: f64,
    max_active: u32, selection: f64, active_page: f64, pending_len: usize) -> u32 {
    INT_INPUT.with(|input| LAYOUT.with(|layout| {
        let input = input.borrow();
        if pending_len > input.len() { return 0; }
        let plan_input = PlanInput { pan_x, pan_y, zoom, rect_width, rect_height, buffer, max_active: max_active as usize,
            selection, active_page, pending: &input[..pending_len] };
        let Some(plan) = plan(&layout.borrow(), &plan_input) else { return 0; };
        INT_OUTPUT.with(|out| {
            let mut out = out.borrow_mut();
            out.clear();
            out.extend_from_slice(&[plan.range_start, plan.range_end, plan.direct.len() as i32, plan.ordered.len() as i32,
                plan.preview_only.len() as i32, plan.prune.len() as i32]);
            for list in [&plan.direct, &plan.ordered, &plan.preview_only, &plan.prune] { out.extend_from_slice(list); }
            out.len() as u32
        })
    }))
}

#[no_mangle]
pub extern "C" fn ev_plan_output() -> *const i32 { INT_OUTPUT.with(|o| o.borrow().as_ptr()) }

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(widths: &[f64], heights: &[f64], calendar: &[u8]) -> Layout {
        Layout::build(widths, heights, calendar, 30.0, 30.0, 794.0, 1123.0, 198.0)
    }
    fn uniform(count: usize) -> Layout { layout(&vec![794.0; count], &vec![1123.0; count], &vec![0; count]) }

    #[test] fn js_round_matches_math_round() {
        for (x, y) in [(0.5, 1.0), (1.5, 2.0), (2.5, 3.0), (-0.5, 0.0), (-1.5, -1.0), (-2.5, -2.0), (0.49999999999999994, 0.0), (793.4, 793.0), (1e20, 1e20)] {
            assert_eq!(js_round(x), y, "{x}");
        }
    }

    #[test] fn empty_document_has_placeholder_page() {
        let l = layout(&[], &[], &[]);
        assert_eq!((l.page_count(), l.pages), (1, 0));
        assert_eq!(l.tops, vec![30.0]);
        assert_eq!(l.bottoms, vec![1153.0]);
        assert_eq!(l.rights, vec![824.0]);
        assert_eq!(l.content_bottom, 1153.0);
        assert_eq!(l.bounds, [30.0, 30.0, 824.0, 1153.0]);
        assert_eq!(l.top_offset(0.0, 30.0), 30.0);
        assert_eq!(visible_range(&l, 0.0, 1.0, 800.0, 1.0), (0.0, -1.0));
    }

    #[test] fn layout_matches_hand_computation() {
        let l = layout(&[794.0, 1000.0, f64::NAN], &[1123.0, 500.4, 20.0], &[0, 1, 0]);
        assert_eq!(l.tops, vec![30.0, 1183.0, 1183.0 + 500.0 + 198.0 + 30.0]);
        assert_eq!(l.bottoms[1], 1683.0);
        // Widest page is 1000: page 0 is centred in it, the NaN width falls back to A4.
        assert_eq!(l.lefts, vec![30.0 + 103.0, 30.0, 30.0 + 103.0]);
        assert_eq!(l.heights[2], 64.0);
        assert_eq!(l.bounds[2], 1030.0);
        assert_eq!(l.content_bottom, l.tops[2] + 64.0);
    }

    #[test] fn top_offset_clamps_like_js() {
        let l = uniform(3);
        assert_eq!(l.top_offset(-5.0, 30.0), l.tops[0]);
        assert_eq!(l.top_offset(2.0, 30.0), l.tops[2]);
        assert_eq!(l.top_offset(3.0, 30.0), l.content_bottom + 30.0);
        assert_eq!(l.top_offset(99.0, 30.0), l.content_bottom + 30.0);
        assert_eq!(l.top_offset(f64::NAN, 30.0), l.content_bottom + 30.0);
    }

    #[test] fn binary_searches_find_edges() {
        let l = uniform(10);
        assert_eq!(l.first_bottom_reaching(-100.0), 0);
        assert_eq!(l.first_bottom_reaching(l.bottoms[4]), 4);
        assert_eq!(l.first_bottom_reaching(l.bottoms[4] + 0.001), 5);
        assert_eq!(l.first_bottom_reaching(1e9), -1);
        assert_eq!(l.last_top_preceding(l.tops[4]), 4);
        assert_eq!(l.last_top_preceding(l.tops[4] - 0.001), 3);
        assert_eq!(l.last_top_preceding(0.0), -1);
        assert_eq!(l.first_bottom_reaching(f64::NAN), 0);
        assert_eq!(l.last_top_preceding(f64::NAN), -1);
    }

    #[test] fn range_falls_back_to_nearest_page() {
        let l = uniform(5);
        assert_eq!(visible_range(&l, 0.0, 1.0, 0.0, 1.0), (0.0, -1.0));
        assert_eq!(visible_range(&l, 1e9, 1.0, 800.0, 1.0), (0.0, 0.0));
        assert_eq!(visible_range(&l, -1e9, 1.0, 800.0, 1.0), (4.0, 4.0));
        assert_eq!(visible_range(&l, 0.0, 1.0, 800.0, 1.0), (0.0, 1.0));
        assert_eq!(visible_range(&l, 0.0, 1.0, 800.0, 0.0), (0.0, 0.0));
    }

    fn base_input<'a>(pending: &'a [i32]) -> PlanInput<'a> {
        PlanInput { pan_x: 0.0, pan_y: 0.0, zoom: 1.0, rect_width: 900.0, rect_height: 800.0, buffer: 1.0, max_active: 3,
            selection: f64::NAN, active_page: f64::NAN, pending }
    }

    #[test] fn plan_orders_active_pages_and_lists_prunes() {
        let l = uniform(20);
        let mut input = base_input(&[0, 7, 9]);
        input.pan_y = -(l.tops[3] + 300.0);
        let plan = plan(&l, &input).unwrap();
        assert_eq!(plan.direct, vec![3]);
        assert_eq!(plan.ordered, vec![3, 4, 2]);
        assert_eq!((plan.range_start, plan.range_end), (2, 4));
        assert!(plan.preview_only.is_empty());
        assert_eq!(plan.prune, vec![0, 7, 9]);
    }

    #[test] fn selection_and_active_page_take_priority_inside_range() {
        let l = uniform(20);
        let mut input = base_input(&[]);
        input.pan_y = -(l.tops[3] + 300.0);
        input.selection = 4.0; input.active_page = 9.0; // 9 is outside [2, 4]
        let plan = plan(&l, &input).unwrap();
        assert_eq!(plan.ordered, vec![4, 3, 2]);
    }

    #[test] fn plan_stops_when_nothing_visible() {
        assert!(plan(&uniform(0), &base_input(&[])).is_none());
        let l = uniform(4);
        let mut input = base_input(&[]);
        input.rect_height = 0.0;
        assert!(plan(&l, &input).is_none());
    }

    #[test] fn zoomed_out_view_caps_active_pages_and_previews_the_rest() {
        let l = uniform(40);
        let mut input = base_input(&[]);
        input.zoom = 0.25;
        input.pan_y = -2000.0 * 0.25;
        let plan = plan(&l, &input).unwrap();
        assert_eq!(plan.ordered.len(), 3);
        assert_eq!(plan.direct.len(), 3);
        assert!(plan.preview_only.len() > 0);
        assert!(plan.preview_only.iter().all(|i| !plan.ordered.contains(i)));
    }

    #[test] fn invalid_buffers_are_rejected() {
        assert!(ev_layout_values(MAX_PAGES + 1).is_null());
        assert!(ev_layout_flags(MAX_PAGES + 1).is_null());
        assert!(ev_plan_input(MAX_PAGES + 1).is_null());
        assert_eq!(ev_layout_build(MAX_PAGES + 1, 30.0, 30.0, 794.0, 1123.0, 198.0), 0);
        assert_eq!(ev_layout_build(0, f64::NAN, 30.0, 794.0, 1123.0, 198.0), 0);
        // Reserved buffers shorter than the count are refused instead of read out of bounds.
        ev_layout_values(2); ev_layout_flags(2);
        assert_eq!(ev_layout_build(3, 30.0, 30.0, 794.0, 1123.0, 198.0), 0);
        assert_eq!(ev_plan(0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 3, f64::NAN, f64::NAN, 5), 0);
    }

    #[test] fn exported_boundary_round_trips() {
        unsafe {
            let values = std::slice::from_raw_parts_mut(ev_layout_values(2), 4);
            values.copy_from_slice(&[794.0, 794.0, 1123.0, 1123.0]);
            std::slice::from_raw_parts_mut(ev_layout_flags(2), 2).copy_from_slice(&[0, 1]);
            assert_eq!(ev_layout_build(2, 30.0, 30.0, 794.0, 1123.0, 198.0), 1);
            assert_eq!(ev_layout_scalar(6), 2.0);
            assert_eq!(ev_layout_scalar(0), 30.0 + 1123.0 + 30.0 + 1123.0 + 198.0);
            std::slice::from_raw_parts_mut(ev_plan_input(1), 1).copy_from_slice(&[5]);
            let written = ev_plan(0.0, 0.0, 1.0, 900.0, 800.0, 1.0, 3, f64::NAN, f64::NAN, 1);
            assert!(written >= 6);
            let out = std::slice::from_raw_parts(ev_plan_output(), written as usize);
            assert_eq!(out[0], 0);
            let range = std::slice::from_raw_parts(ev_range(0.0, 1.0, 800.0, 1.0), 2);
            assert_eq!(range, &[0.0, 1.0]);
        }
    }
}
