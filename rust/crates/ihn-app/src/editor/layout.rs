//! Editor: Page column metrics.
//!
//! The page column in content coordinates (5.18.0 `getPageLayoutMetrics`): page tops/bottoms/lefts/rights, the
//! content bottom and the bounds the viewport clamps against, plus the visible-range searches. The page list is
//! the editor's rows (`editor::pages` keeps it in document order); the metrics are recomputed only when it changes.

use crate::{App, Event, Host};
use ihn_core::id::Id;

// v5: app-v5.js constants
pub const A4_WIDTH: f64 = 794.0;
pub const A4_HEIGHT: f64 = 1123.0;
pub const PAGE_PADDING: f64 = 30.0;
pub const PAGE_GAP: f64 = 30.0;
pub const VISIBLE_PAGE_BUFFER: usize = 1;
pub const MAX_ACTIVE_PAGES: usize = 3;

/// One page of the column: id and logical size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dim {
    pub id: Id,
    pub w: f64,
    pub h: f64,
}

/// v5 `cachedPageLayoutMetrics`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Metrics {
    pub tops: Vec<f64>,
    pub bottoms: Vec<f64>,
    pub lefts: Vec<f64>,
    pub rights: Vec<f64>,
    pub content_bottom: f64,
    /// `[left, top, right, bottom]`
    pub bounds: [f64; 4],
    /// v5 `pageCount || 1`.
    pub count: usize,
}

#[derive(Default)]
pub struct State {
    pages: Vec<Dim>,
    metrics: Metrics,
}

/// Replaces the page list (rows rebuilt, a page added/removed/resized).
pub fn set_pages(app: &mut App, pages: Vec<Dim>) {
    let st = &mut app.st.editor_layout;
    st.metrics = compute(&pages);
    st.pages = pages;
}

pub fn pages(app: &App) -> &[Dim] { &app.st.editor_layout.pages }
pub fn len(app: &App) -> usize { app.st.editor_layout.pages.len() }
pub fn index_of(app: &App, id: Id) -> Option<usize> { app.st.editor_layout.pages.iter().position(|p| p.id == id) }
pub fn metrics(app: &App) -> &Metrics { &app.st.editor_layout.metrics }

/// v5: `getMaxPageWidth()` — never narrower than A4.
pub fn max_width(pages: &[Dim]) -> f64 { pages.iter().fold(A4_WIDTH, |m, p| if p.w > m { p.w } else { m }) }

/// v5: `getPageLayoutMetrics()` (app-v5.js:15588).
pub fn compute(pages: &[Dim]) -> Metrics {
    let (padding, max_width) = (PAGE_PADDING, max_width(pages));
    let mut m = Metrics::default();
    let mut y = padding;
    if pages.is_empty() {
        m.tops.push(padding);
        m.bottoms.push(padding + A4_HEIGHT);
        m.lefts.push(padding);
        m.rights.push(padding + A4_WIDTH);
        y = padding + A4_HEIGHT;
    }
    for (i, p) in pages.iter().enumerate() {
        let left = padding + (max_width - p.w) / 2.0;
        m.tops.push(y);
        m.bottoms.push(y + p.h);
        m.lefts.push(left);
        m.rights.push(left + p.w);
        y += p.h;
        if i + 1 < pages.len() {
            y += PAGE_GAP;
        }
    }
    m.content_bottom = y;
    m.bounds = [padding, padding, padding + max_width, y];
    m.count = pages.len().max(1);
    m
}

/// v5: `getContentHeight()`.
pub fn content_height(app: &App) -> f64 {
    if app.st.editor_layout.pages.is_empty() {
        0.0
    } else {
        app.st.editor_layout.metrics.content_bottom + PAGE_PADDING
    }
}

/// v5: `getPageTopOffset(i)`.
pub fn page_top(app: &App, i: usize) -> f64 {
    let m = metrics(app);
    let idx = i.min(len(app));
    if idx < m.tops.len() {
        m.tops[idx]
    } else {
        m.content_bottom + if m.count > 0 { PAGE_GAP } else { 0.0 }
    }
}

/// v5: `getPageDimensions(i)` (A4 for an index without a page).
pub fn dims(app: &App, i: usize) -> (f64, f64) { app.st.editor_layout.pages.get(i).map_or((A4_WIDTH, A4_HEIGHT), |p| (p.w, p.h)) }

/// v5: `getPageCenterOffset(i)`.
pub fn page_center(app: &App, i: usize) -> f64 { page_top(app, i) + dims(app, i).1 / 2.0 }

/// v5: `findFirstPageWhoseBottomReaches(metrics, y)`; -1 when none.
pub fn first_bottom_reaching(m: &Metrics, y: f64) -> isize {
    let i = m.bottoms.partition_point(|&b| b < y);
    if i < m.bottoms.len() { i as isize } else { -1 }
}

/// v5: `findLastPageWhoseTopPrecedes(metrics, y)`; -1 when none.
pub fn last_top_preceding(m: &Metrics, y: f64) -> isize { m.tops.partition_point(|&t| t <= y) as isize - 1 }

/// v5: `getVisiblePageRange(buffer)`: inclusive `(start, end)`, `end < start` when nothing (as `{0, -1}`).
/// `vh` is the viewport height (gesture geometry or a fresh rect), `pan_y`/`zoom` the transform.
pub fn visible_range(app: &App, vh: f64, pan_y: f64, zoom: f64, buffer: usize) -> (isize, isize) {
    if vh == 0.0 || vh.is_nan() {
        return (0, -1);
    }
    let n = len(app) as isize;
    let (view_top, view_bottom) = (-pan_y / zoom, (vh - pan_y) / zoom);
    let m = metrics(app);
    let (first, last) = (first_bottom_reaching(m, view_top), last_top_preceding(m, view_bottom));
    if first < 0 || last < first {
        if n == 0 {
            return (0, -1);
        }
        let nearest = if view_top < PAGE_PADDING { 0 } else { n - 1 };
        return (nearest, nearest);
    }
    let b = buffer as isize;
    ((first - b).max(0), (last + b).min(n - 1))
}

/// v5: `getViewportVisiblePageIndices(0)`: pages intersecting the viewport, x tolerance 25 % of its width.
pub fn viewport_visible(app: &App, vw: f64, vh: f64, pan_x: f64, pan_y: f64, zoom: f64) -> Vec<usize> {
    if vw == 0.0 || vh == 0.0 || vw.is_nan() || vh.is_nan() {
        return Vec::new();
    }
    let extra_x = (vw * 0.25).max(0.0);
    let (view_top, view_bottom) = (-pan_y / zoom, (vh - pan_y) / zoom);
    let (view_left, view_right) = ((-pan_x - extra_x) / zoom, (vw - pan_x + extra_x) / zoom);
    let m = metrics(app);
    let (first, last) = (first_bottom_reaching(m, view_top), last_top_preceding(m, view_bottom));
    let mut out = Vec::new();
    let mut i = first.max(0);
    while i <= last.min(m.count as isize - 1) {
        let k = i as usize;
        if k < m.rights.len() && m.rights[k] >= view_left && m.lefts[k] <= view_right && k < len(app) {
            out.push(k);
        }
        i += 1;
    }
    out
}

/// v5: `expandPageIndices(indices, buffer)`: sorted, deduplicated, within the document.
pub fn expand(n: usize, indices: &[usize], buffer: usize) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    for &i in indices {
        for k in i.saturating_sub(buffer)..=(i + buffer) {
            if k < n && !out.contains(&k) {
                out.push(k);
            }
        }
    }
    ihn_core::sort::ord(&mut out);
    out
}

/// Called once at boot, in `app::MODULES` order, after the `Ui` table is bound.
pub fn init(_app: &mut App, _host: &mut dyn Host) {}

/// Events addressed to this module (ids made with `abi::tag`) and broadcasts (`Life`, `Msg`). Returns `abi::ret` bits.
pub fn event(_app: &mut App, _host: &mut dyn Host, _event: &Event) -> u32 { 0 }

/// `__IHN_TEST_API__.cmd('editor.layout.metrics')`: the column as JSON.
pub fn test_cmd(app: &mut App, _host: &mut dyn Host, name: &str, _args: &str) -> Option<String> {
    let m = metrics(app);
    let list = |v: &[f64]| v.iter().map(|x| ihn_core::jsfmt::num(*x)).collect::<Vec<_>>().join(",");
    match name {
        "metrics" => Some(format!(
            "{{\"pages\":{},\"tops\":[{}],\"lefts\":[{}],\"contentBottom\":{},\"contentHeight\":{}}}",
            len(app),
            list(&m.tops),
            list(&m.lefts),
            ihn_core::jsfmt::num(m.content_bottom),
            ihn_core::jsfmt::num(content_height(app))
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dim(w: f64, h: f64) -> Dim { Dim { id: Id(w as u128 * 10000 + h as u128), w, h } }

    #[test]
    fn metrics_center_narrow_pages_and_stack_with_gaps() {
        let m = compute(&[dim(794.0, 1123.0), dim(559.0, 794.0)]);
        assert_eq!(m.tops, vec![30.0, 1183.0]);
        assert_eq!(m.bottoms, vec![1153.0, 1977.0]);
        assert_eq!(m.lefts, vec![30.0, 147.5]);
        assert_eq!(m.content_bottom, 1977.0);
        assert_eq!(m.bounds, [30.0, 30.0, 824.0, 1977.0]);
        let empty = compute(&[]);
        assert_eq!((empty.tops[0], empty.content_bottom, empty.count), (30.0, 1153.0, 1));
    }

    #[test]
    fn binary_searches_match_v5() {
        let m = compute(&[dim(794.0, 1123.0), dim(794.0, 1123.0), dim(794.0, 1123.0)]);
        assert_eq!(first_bottom_reaching(&m, 0.0), 0);
        assert_eq!(first_bottom_reaching(&m, 1153.0), 0);
        assert_eq!(first_bottom_reaching(&m, 1154.0), 1);
        assert_eq!(first_bottom_reaching(&m, 1e9), -1);
        assert_eq!(last_top_preceding(&m, 29.0), -1);
        assert_eq!(last_top_preceding(&m, 1183.0), 1);
        assert_eq!(expand(3, &[0, 2], 1), vec![0, 1, 2]);
    }
}
