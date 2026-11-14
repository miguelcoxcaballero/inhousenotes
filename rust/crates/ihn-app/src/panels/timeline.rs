//! Timeline panel (`#timeline-panel`, spec/documents.md §6.5–6.6): the version list grouped by month and 1-hour
//! clusters, the preview with diff highlights, zoom, and "Revert to this version" through the slide-to-confirm
//! modal. Owned by W5.3. Entries come from the document (`history::entries`): versions captured by every editor
//! travel with the document's operations; a preview needs the version's snapshot asset (`doc::versions`).
// v5: app-v5.js:openTimelinePanel / renderTimelinePanelContents / renderTimelineList / groupTimelineEntriesForDisplay
// / selectTimelineVersion / renderTimelinePreview / drawTimelineDiffHighlights / updateTimelineRevertButtonState /
// buildOriginalVersionEntry / setupTimelineModalEvents / closeTimelinePanel / restoreVersion (dialog texts)

use super::slider;
use crate::abi::{ety, ev, listen, mode, ret};
use crate::doc::{session, versions};
use crate::editor::pages;
use crate::home::cards::esc;
use crate::shell::{self, Status};
use crate::{tag, App, Event, Host, IntlOp, Mod, Node, Prop, Ui};
use ihn_core::doc::Doc;
use ihn_core::history::{self, Hint, Mark, Record, SnapPage, Snapshot};
use ihn_core::id::Id;
use ihn_core::jsfmt::num;
use ihn_core::jsmath::{self, round};
use ihn_core::op::{Background, Body, Template, Tool};
use ihn_core::render::cmd::{Join, Src, Surf};
use ihn_core::render::page::{images as draw_images, strokes as draw_strokes};
use ihn_core::render::{ImageRef, StrokeRef};

const L_OPEN: u32 = tag(Mod::PanelsTimeline, 1);
const L_CLOSE: u32 = tag(Mod::PanelsTimeline, 2);
const L_REVERT: u32 = tag(Mod::PanelsTimeline, 3);
const L_LIST: u32 = tag(Mod::PanelsTimeline, 4);
const L_KEY: u32 = tag(Mod::PanelsTimeline, 5);
const L_WHEEL: u32 = tag(Mod::PanelsTimeline, 6);
const L_TOUCH: u32 = tag(Mod::PanelsTimeline, 7);
const L_TOUCH_MOVE: u32 = tag(Mod::PanelsTimeline, 8);
const L_TOUCH_END: u32 = tag(Mod::PanelsTimeline, 9);
const L_TOUCH_CANCEL: u32 = tag(Mod::PanelsTimeline, 10);
const T_RETRY: u32 = tag(Mod::PanelsTimeline, 1);

const ORIGINAL: &str = "__original__";
const ZOOM_MIN: f64 = 0.2;
const ZOOM_MAX: f64 = 3.0;
/// v5 `TIMELINE_CLUSTER_GAP_MS`.
const CLUSTER_GAP_MS: f64 = 3_600_000.0;
const A4: (u32, u32) = (794, 1123);
const AVATAR: &str = "data:image/svg+xml;utf8,<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"%23ccc\"><path d=\"M12 12c2.21 0 4-1.79 4-4s-1.79-4-4-4-4 1.79-4 4 1.79 4 4 4zm0 2c-2.67 0-8 1.34-8 4v2h16v-2c0-2.66-5.33-4-8-4z\"/></svg>";
const CHEVRON: &str = "<svg class=\"timeline-group-chevron\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2.4\" stroke-linecap=\"round\" stroke-linejoin=\"round\" aria-hidden=\"true\"><polyline points=\"6 9 12 15 18 9\"></polyline></svg>";

/// One row of the display history (5.18.0 `getDisplayVersionHistory`, original first).
#[derive(Clone)]
struct Entry {
    /// `None` = the synthetic Original entry.
    id: Option<Id>,
    rec: Record,
}

impl Entry {
    fn key(&self) -> String { self.id.map_or(ORIGINAL.into(), |i| i.hex()) }
    fn author(&self) -> &str { if self.rec.name.is_empty() { if self.rec.email.is_empty() { "Unknown" } else { &self.rec.email } } else { &self.rec.name } }
}

struct Group {
    header: Node,
    body: Node,
    key: String,
    collapsed: bool,
}

#[derive(Default)]
pub struct State {
    open: bool,
    /// The list is waiting for snapshot reads (`Loading timeline…`).
    loading: bool,
    selected: Option<String>,
    zoom: f64,
    /// v5 `_timelineCollapsedMonths` / `_timelineCollapsedClusters` (`!key` = explicitly expanded).
    months_collapsed: Vec<String>,
    clusters_collapsed: Vec<String>,
    /// Rendered rows: (li, entry key).
    rows: Vec<(Node, String)>,
    months: Vec<Group>,
    clusters: Vec<Group>,
    /// Keys of the rendered entries (to notice remote changes).
    shown: Vec<(String, u64)>,
    /// Preview pages: (container, canvas, surface, base display width).
    preview: Vec<(Node, Node, Surf, f64)>,
    /// Preview repaints still waiting for an image decode.
    retries: u32,
    pinch: (f64, f64),
    /// The slider is open for this entry.
    restoring: Option<String>,
}

fn st(app: &mut App) -> &mut State { &mut app.st.panels_timeline }

pub fn is_open(app: &App) -> bool { app.st.panels_timeline.open }

// ---- entries ------------------------------------------------------------------------------------------------------

/// The source PDF of the document's PDF pages, if any (5.18.0 `state.cleanOriginalPdfBytes`).
fn pdf_source(d: &Doc) -> Option<ihn_core::id::Hash> {
    d.order().iter().find_map(|id| match d.pages.get(id)?.background.v {
        Background::Pdf { asset, .. } => Some(asset),
        _ => None,
    })
}

/// v5 `buildOriginalVersionEntry()`: the document without annotations, as a snapshot.
fn original(d: &Doc) -> Option<(Record, Snapshot)> {
    let page = |n: usize, template, size, background| SnapPage { id: Id::legacy(&format!("original-page-{n}")), template, size, background, side_panel: None, items: Vec::new() };
    let live: Vec<&ihn_core::doc::Page> = d.order().iter().filter_map(|id| d.pages.get(id)).collect();
    let (pages, summary) = match pdf_source(d) {
        Some(asset) => {
            // The source's pages, by the highest page shown (sizes from the pages that show them).
            let n = live.iter().filter_map(|p| match p.background.v {
                Background::Pdf { asset: a, page } if a == asset => Some(page),
                _ => None,
            }).max().unwrap_or(1);
            let size_of = |k: u32| live.iter().find(|p| p.background.v == Background::Pdf { asset, page: k }).map_or(A4, |p| p.size.v);
            ((1..=n).map(|k| page(k as usize, Template::Default, size_of(k), Background::Pdf { asset, page: k })).collect(), "Unmodified PDF without any annotations")
        }
        None => (
            live.iter().enumerate().map(|(i, p)| page(i + 1, p.template.v, p.size.v, if matches!(p.background.v, Background::Pdf { .. }) { p.background.v } else { Background::Template })).collect::<Vec<_>>(),
            "Blank document with no annotations",
        ),
    };
    if pages.is_empty() {
        return None;
    }
    let rec = Record { ts: 0.0, name: "Original document".into(), summary: summary.into(), kind: "original".into(), milestone: true, ..Record::default() };
    Some((rec, Snapshot { name: String::new(), pages }))
}

/// Original first, then the listed versions oldest first.
fn display(app: &App) -> Vec<Entry> {
    let Some(s) = session::get(app) else { return Vec::new() };
    let mut out: Vec<Entry> = original(&s.doc).map(|(rec, _)| Entry { id: None, rec }).into_iter().collect();
    out.extend(history::entries(&s.doc).into_iter().map(|(id, rec)| Entry { id: Some(id), rec }));
    out
}

fn current_key(app: &App) -> Option<String> { session::get(app).and_then(|s| history::entries(&s.doc).pop()).map(|e| e.0.hex()) }

/// The snapshot shown and restored for an entry (`None`: its asset is not on this device).
fn snapshot_of(app: &App, e: &Entry) -> Option<Snapshot> {
    match e.id {
        None => session::get(app).and_then(|s| original(&s.doc)).map(|o| o.1),
        Some(_) => versions::snapshot(app, &e.rec.snapshot),
    }
}

// ---- dates --------------------------------------------------------------------------------------------------------

fn date(host: &mut dyn Host, ms: f64, method: &str, options: &str) -> String { host.intl(IntlOp::Date, &format!("{}\0{method}\0\0{options}", num(ms))) }

/// Local `(year, month 1-12, day)`.
fn ymd(host: &mut dyn Host, ms: f64) -> (i64, i64, i64) {
    let s = host.intl(IntlOp::Date, &format!("{}\0date\0\"en-CA\"\0", num(ms)));
    let mut it = s.split('-').map(|p| p.trim().parse::<i64>().unwrap_or(0));
    (it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0))
}

/// Days since 1970-01-01 of a civil date.
fn days(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468
}

fn hm(host: &mut dyn Host, ts: f64) -> String { if ts > 0.0 { date(host, ts, "time", r#"{"hour":"numeric","minute":"2-digit"}"#) } else { String::new() } }

/// v5 `getTimelineDayLabel`.
fn day_label(host: &mut dyn Host, ts: f64) -> String {
    if ts <= 0.0 {
        return String::new();
    }
    let now = host.time();
    let (a, b) = (ymd(host, ts), ymd(host, now));
    match days(b.0, b.1, b.2) - days(a.0, a.1, a.2) {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        _ => date(host, ts, "date", r#"{"weekday":"short","month":"short","day":"numeric"}"#),
    }
}

// ---- list ---------------------------------------------------------------------------------------------------------

fn entry_html(host: &mut dyn Host, e: &Entry, current: Option<&str>) -> String {
    let key = e.key();
    let is_current = current == Some(key.as_str());
    let orig = e.id.is_none();
    let name = esc(e.author());
    let mut row = format!("<span class=\"timeline-entry-user\">{name}</span>");
    if !orig {
        let abs = date(host, e.rec.ts, "string", "");
        let when = hm(host, e.rec.ts);
        let when = if when.is_empty() { date(host, e.rec.ts, "date", "") } else { when };
        row.push_str(&format!("<span class=\"timeline-entry-when\" title=\"{}\">{}</span>", esc(&abs), esc(&when)));
    }
    if is_current {
        row.push_str("<span class=\"timeline-entry-current-tag\">Current</span>");
    }
    match e.rec.kind.as_str() {
        "restore" => row.push_str("<span class=\"timeline-entry-milestone-tag\">Restored</span>"),
        "revert-original" => row.push_str("<span class=\"timeline-entry-milestone-tag\">Reverted</span>"),
        _ => {}
    }
    if orig {
        row.push_str("<span class=\"timeline-entry-milestone-tag\">Original</span>");
    }
    let photo = if e.rec.photo.is_empty() { AVATAR } else { &e.rec.photo };
    format!(
        "<li class=\"timeline-entry{}{}\" data-version-id=\"{}\" tabindex=\"0\" role=\"button\" aria-label=\"Preview version by {}\"><img class=\"timeline-entry-avatar\" alt=\"\" src=\"{}\"><div class=\"timeline-entry-body\"><div class=\"timeline-entry-row\">{row}</div><div class=\"timeline-entry-summary\">{}</div></div></li>",
        if is_current { " is-current" } else { "" },
        if orig { " is-original" } else { "" },
        esc(&key),
        esc(if e.rec.name.is_empty() { "unknown" } else { &e.rec.name }),
        esc(photo),
        esc(if e.rec.summary.is_empty() { "Saved" } else { &e.rec.summary }),
    )
}

fn plural(n: usize, word: &str) -> String { format!("{n} {word}{}", if n != 1 { "s" } else { "" }) }

/// v5 `renderTimelineList()`.
fn render_list(app: &mut App, host: &mut dyn Host) {
    let list = display(app);
    let ts = st(app);
    ts.rows.clear();
    ts.months.clear();
    ts.clusters.clear();
    ts.shown = list.iter().map(|e| (e.key(), e.rec.ts.to_bits())).collect();
    if list.is_empty() {
        let out = host.out();
        out.html(Ui::TIMELINE_LIST, "");
        out.prop(Ui::TIMELINE_EMPTY, "hidden", Prop::Bool(false));
        return;
    }
    let current = current_key(app);
    // v5 groupTimelineEntriesForDisplay over the newest-first list: months, then clusters of ≤ 1 h gaps.
    struct M {
        key: String,
        label: String,
        count: usize,
        clusters: Vec<Vec<Entry>>,
    }
    let mut months: Vec<M> = Vec::new();
    let mut prev_ts: Option<f64> = None;
    for e in list.into_iter().rev() {
        let orig = e.id.is_none();
        let key = if orig || e.rec.ts <= 0.0 {
            "original".to_string()
        } else {
            let (y, m, _) = ymd(host, e.rec.ts);
            format!("{y}-{}", m - 1)
        };
        if months.last().is_none_or(|m| m.key != key) {
            let label = if orig || e.rec.ts <= 0.0 { "Original".into() } else { date(host, e.rec.ts, "string", r#"{"year":"numeric","month":"long"}"#) };
            months.push(M { key, label, count: 0, clusters: Vec::new() });
            prev_ts = None;
        }
        let m = months.last_mut().expect("month");
        let fresh = orig || m.clusters.is_empty() || prev_ts.is_none_or(|p| p - e.rec.ts > CLUSTER_GAP_MS);
        prev_ts = Some(e.rec.ts);
        m.count += 1;
        if fresh {
            m.clusters.push(vec![e]);
        } else {
            m.clusters.last_mut().expect("cluster").push(e);
        }
    }
    let mut html = String::new();
    let mut groups: Vec<(String, bool, Vec<Option<(String, bool)>>)> = Vec::new();
    for (mi, m) in months.iter().enumerate() {
        let collapsed = app.st.panels_timeline.months_collapsed.contains(&m.key);
        html.push_str(&format!(
            "<li class=\"timeline-month-group\" data-month-key=\"{}\"><button type=\"button\" class=\"timeline-group-header{}\" aria-expanded=\"{}\">{CHEVRON}<span class=\"timeline-month-label\">{}</span><span class=\"timeline-group-count\">{}</span></button><div class=\"timeline-cluster-group\"{}>",
            esc(&m.key),
            if collapsed { " is-collapsed" } else { "" },
            !collapsed,
            esc(&m.label),
            plural(m.count, "edit"),
            if collapsed { " hidden=\"\"" } else { "" },
        ));
        let mut clusters = Vec::new();
        for (ci, c) in m.clusters.iter().enumerate() {
            if c.len() == 1 && c[0].id.is_none() {
                html.push_str(&entry_html(host, &c[0], current.as_deref()));
                clusters.push(None);
                continue;
            }
            let key = format!("{}-{}", m.key, c[0].key());
            let st = &app.st.panels_timeline;
            let collapsed = if st.clusters_collapsed.contains(&key) { true } else if st.clusters_collapsed.contains(&format!("!{key}")) { false } else { !(mi == 0 && ci == 0) };
            let (head, tail) = (c[0].rec.ts, c[c.len() - 1].rec.ts);
            let day = day_label(host, head);
            let (from, to) = (hm(host, tail), hm(host, head));
            let span = if from == to { to } else { format!("{from}–{to}") };
            let title = if day.is_empty() { span } else { format!("{day}, {span}") };
            let mut authors: Vec<&str> = Vec::new();
            for e in c {
                let a = if e.rec.name.is_empty() { &e.rec.email } else { &e.rec.name };
                if !a.is_empty() && !authors.contains(&a.as_str()) {
                    authors.push(a);
                }
            }
            let who = match authors.len() {
                0 => String::new(),
                1 => format!(" · {}", authors[0]),
                n => format!(" · {} +{}", authors[0], n - 1),
            };
            let mut avatars = String::new();
            let mut seen: Vec<&str> = Vec::new();
            for e in c {
                let k = if e.rec.email.is_empty() { &e.rec.name } else { &e.rec.email };
                if seen.contains(&k.as_str()) {
                    continue;
                }
                seen.push(k);
                avatars.push_str(&format!("<img src=\"{}\" alt=\"\">", esc(if e.rec.photo.is_empty() { AVATAR } else { &e.rec.photo })));
                if seen.len() >= 3 {
                    break;
                }
            }
            html.push_str(&format!(
                "<div data-cluster-key=\"{}\"><button type=\"button\" class=\"timeline-cluster-header{}\" aria-expanded=\"{}\">{CHEVRON}<div class=\"timeline-cluster-info\"><div class=\"timeline-cluster-title\">{}</div><div class=\"timeline-cluster-meta\">{}</div></div><div class=\"timeline-cluster-avatars\">{avatars}</div></button><ol class=\"timeline-cluster-entries\" style=\"list-style: none; padding: 0px; margin: 0px;\"{}>",
                esc(&key),
                if collapsed { " is-collapsed" } else { "" },
                !collapsed,
                esc(&title),
                esc(&format!("{}{who}", plural(c.len(), "edit"))),
                if collapsed { " hidden=\"\"" } else { "" },
            ));
            for e in c {
                html.push_str(&entry_html(host, e, current.as_deref()));
            }
            html.push_str("</ol></div>");
            clusters.push(Some((key, collapsed)));
        }
        html.push_str("</div></li>");
        groups.push((m.key.clone(), collapsed, clusters));
    }
    let out = host.out();
    out.prop(Ui::TIMELINE_EMPTY, "hidden", Prop::Bool(true));
    out.html(Ui::TIMELINE_LIST, &html);
    let (mut rows, mut mg, mut cg) = (Vec::new(), Vec::new(), Vec::new());
    for (mi, (mkey, mc, clusters)) in groups.into_iter().enumerate() {
        let li = out.query(Ui::TIMELINE_LIST, &format!(":scope > li:nth-child({})", mi + 1));
        let body = out.query(li, ":scope > .timeline-cluster-group");
        mg.push(Group { header: out.query(li, ":scope > .timeline-group-header"), body, key: mkey, collapsed: mc });
        for (ci, c) in clusters.into_iter().enumerate() {
            if let Some((ckey, collapsed)) = c {
                let node = out.query(body, &format!(":scope > :nth-child({})", ci + 1));
                let entries = out.query(node, ":scope > ol");
                cg.push(Group { header: out.query(node, ":scope > button"), body: entries, key: ckey, collapsed });
                out.release(node);
            }
        }
        out.release(li);
    }
    let keys: Vec<String> = months.iter().flat_map(|m| m.clusters.iter().flatten()).map(Entry::key).collect();
    for key in keys {
        rows.push((out.query(Ui::TIMELINE_LIST, &format!(".timeline-entry[data-version-id=\"{key}\"]")), key));
    }
    let st = st(app);
    (st.rows, st.months, st.clusters) = (rows, mg, cg);
}

fn release_list(app: &mut App, host: &mut dyn Host) {
    let st = st(app);
    let nodes: Vec<Node> = st.rows.drain(..).map(|r| r.0).chain(st.months.drain(..).chain(st.clusters.drain(..)).flat_map(|g| [g.header, g.body])).collect();
    let out = host.out();
    nodes.into_iter().filter(|n| !n.is_none()).for_each(|n| out.release(n));
}

// ---- preview ------------------------------------------------------------------------------------------------------

fn clear_preview(app: &mut App, host: &mut dyn Host) {
    let pages_ = std::mem::take(&mut st(app).preview);
    pages::with_gfx(app, |_, g| pages_.iter().for_each(|p| g.cmds.drop_surf(p.2)));
    pages::flush(app, host);
    let out = host.out();
    for (container, canvas, _, _) in pages_ {
        out.remove(container);
        out.release(container);
        out.release(canvas);
    }
}

fn draw_highlights(c: &mut ihn_core::render::cmd::Cmds, list: &[history::Highlight]) {
    if list.is_empty() {
        return;
    }
    c.save();
    c.line_join(Join::Round);
    for h in list {
        let removed = h.mark == Mark::Removed;
        c.stroke_style(if removed { "#ef4444" } else { "#f59e0b" });
        c.fill_style(if removed { "rgba(239, 68, 68, 0.08)" } else { "rgba(245, 158, 11, 0.12)" });
        c.line_width(2.5);
        c.line_dash(if removed { &[8.0, 5.0] } else { &[] });
        c.fill_rect(h.x, h.y, h.w, h.h);
        c.stroke_rect(h.x, h.y, h.w, h.h);
    }
    c.restore();
}

/// v5 `renderTimelinePreview(entry)`. Pages whose background or images are still decoding are painted again.
fn render_preview(app: &mut App, host: &mut dyn Host) {
    clear_preview(app, host);
    let list = display(app);
    let sel = app.st.panels_timeline.selected.clone();
    let at = sel.as_ref().and_then(|k| list.iter().position(|e| e.key() == *k));
    let snap = at.and_then(|i| snapshot_of(app, &list[i]));
    let Some((i, snap)) = at.zip(snap).filter(|(_, s)| !s.pages.is_empty()) else {
        let out = host.out();
        out.prop(Ui::TIMELINE_PREVIEW_EMPTY, "hidden", Prop::Bool(false));
        out.prop(Ui::TIMELINE_PREVIEW_META, "hidden", Prop::Bool(true));
        return;
    };
    let e = &list[i];
    let prev = if i > 0 { snapshot_of(app, &list[i - 1]) } else { None };
    let when = date(host, e.rec.ts, "string", "");
    let n = snap.pages.len();
    let out = host.out();
    out.prop(Ui::TIMELINE_PREVIEW_EMPTY, "hidden", Prop::Bool(true));
    out.prop(Ui::TIMELINE_PREVIEW_META, "hidden", Prop::Bool(false));
    out.text(Ui::TIMELINE_PREVIEW_META, &format!("{} · {when} · {}", e.author(), plural(n, "page")));
    let r = host.rect(Ui::TIMELINE_PREVIEW_WRAP);
    let tw = (if r.w != 0.0 { r.w } else { 480.0 } - 32.0).max(160.0);
    let th = (if r.h != 0.0 { r.h } else { 480.0 } - 32.0).max(160.0);
    let wide = snap.pages.iter().map(|p| p.size.0 as f64).fold(0.0, f64::max);
    let tall = snap.pages.iter().map(|p| p.size.1 as f64).fold(0.0, f64::max);
    let fit = (tw / if wide > 0.0 { wide } else { A4.0 as f64 }).min(th / if tall > 0.0 { tall } else { A4.1 as f64 });
    let rs = (fit * 1.5).min(1.6).max(0.4);
    let base = fit.min(1.4).max(0.25);
    let zoom = app.st.panels_timeline.zoom;
    let mut sizes = Vec::with_capacity(n);
    let mut nodes = Vec::with_capacity(n);
    for (k, p) in snap.pages.iter().enumerate() {
        let (pw, ph) = (p.size.0 as f64, p.size.1 as f64);
        let (cw, ch) = (round(pw * rs).max(1.0), round(ph * rs).max(1.0));
        let bw = round(pw * base).max(1.0);
        let html = format!(
            "<div class=\"timeline-preview-page\"><canvas width=\"{}\" height=\"{}\" data-base-width=\"{}\" style=\"width: {}px; height: auto;\"></canvas><div class=\"timeline-preview-page-label\">Page {}</div></div>",
            num(cw), num(ch), num(bw), num(bw * zoom), k + 1
        );
        let out = host.out();
        let container = out.insert_html(Ui::TIMELINE_PREVIEW_WRAP, Node::NONE, &html);
        nodes.push((container, out.child(container, &[0])));
        sizes.push((cw as u32, ch as u32, bw));
    }
    // What lies under the ink: the live page at the same index for photo backgrounds, a live page showing the same
    // PDF page for PDF pages (its pdf.js raster), the template otherwise.
    let live: Vec<(Id, Background)> = session::get(app).map_or(Vec::new(), |s| s.doc.order().iter().filter_map(|id| Some((*id, s.doc.pages.get(id)?.background.v))).collect());
    let mut bgs = Vec::with_capacity(n);
    let mut waiting = false;
    for (k, p) in snap.pages.iter().enumerate() {
        let bg = match p.background {
            Background::Custom { .. } => match live.get(k).map(|l| l.1) {
                Some(Background::Custom { asset }) => {
                    let img = pages::image_of(app, asset);
                    waiting |= img.is_none();
                    img.map(|i| (i, None))
                }
                _ => None,
            },
            Background::Pdf { .. } => live.iter().find(|l| l.1 == p.background).and_then(|l| pages::pdf_raster_of(app, l.0)).map(|i| (i, Some((p.size.0 as f64, p.size.1 as f64)))),
            Background::Template => None,
        };
        let imgs: Vec<ImageRef> = p
            .items
            .iter()
            .filter_map(|it| match it.body {
                Body::Image { asset, cx, cy, w, h, rotation } => {
                    let img = pages::image_of(app, asset);
                    waiting |= img.is_none();
                    Some(ImageRef { img, cx, cy, w, h, rotation })
                }
                _ => None,
            })
            .collect();
        bgs.push((bg, imgs));
    }
    let diff = history::diff(prev.as_ref(), &snap);
    let surfs = pages::with_gfx(app, |_, g| {
        let mut surfs = Vec::with_capacity(n);
        for (k, p) in snap.pages.iter().enumerate() {
            let c = &mut g.cmds;
            let (cw, ch, _) = sizes[k];
            let s = c.bind(nodes[k].1 .0);
            c.ctx(s, false);
            c.set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0);
            c.fill_style("white");
            c.fill_rect(0.0, 0.0, cw as f64, ch as f64);
            match (&bgs[k].0, p.background) {
                (_, Background::Template) => {
                    c.save();
                    c.set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0);
                    g.tpl.draw(c, Some(s), cw, ch, p.template);
                    c.restore();
                }
                (Some((img, None)), _) => c.draw_image(Src::Img(*img), &[0.0, 0.0, cw as f64, ch as f64]),
                (Some((img, Some((w, h)))), _) => {
                    // v5: the pdf.js page fitted and centred in the canvas.
                    let f = (cw as f64 / w).min(ch as f64 / h);
                    let (dw, dh) = (w * f, h * f);
                    c.draw_image(Src::Img(*img), &[(cw as f64 - dw) / 2.0, (ch as f64 - dh) / 2.0, dw, dh]);
                }
                _ => {}
            }
            let c = &mut g.cmds;
            c.save();
            c.set_transform(rs, 0.0, 0.0, rs, 0.0, 0.0);
            draw_images(c, &bgs[k].1);
            let strokes: Vec<StrokeRef> = p
                .items
                .iter()
                .filter_map(|it| match &it.body {
                    Body::Stroke { tool, color, width, xy, .. } if *tool != Tool::LegacyCover => Some(StrokeRef::new(*tool, color, *width, xy)),
                    _ => None,
                })
                .collect();
            draw_strokes(c, &mut g.paths, &strokes);
            draw_highlights(c, &diff[k]);
            c.restore();
            surfs.push(s);
        }
        surfs
    });
    pages::flush(app, host);
    let st = st(app);
    st.preview = nodes.into_iter().zip(surfs).zip(sizes).map(|(((a, b), s), z)| (a, b, s, z.2)).collect();
    if waiting && st.retries < 100 {
        st.retries += 1;
        app.sched.timer(host.out(), T_RETRY, 60.0);
    }
}

/// v5 `applyTimelineZoomToCanvases()`.
fn apply_zoom(app: &mut App, host: &mut dyn Host) {
    let st = &app.st.panels_timeline;
    let out = host.out();
    for p in &st.preview {
        out.style(p.1, "width", Some(&format!("{}px", num(p.3 * st.zoom))));
        out.style(p.1, "height", Some("auto"));
    }
}

/// v5 `updateTimelineRevertButtonState()`.
fn update_revert(app: &mut App, host: &mut dyn Host) {
    let sel = app.st.panels_timeline.selected.clone();
    let can = sel.as_ref().is_some_and(|k| Some(k) != current_key(app).as_ref() && display(app).iter().find(|e| e.key() == *k).and_then(|e| snapshot_of(app, e)).is_some_and(|s| !s.pages.is_empty()));
    host.out().prop(Ui::BTN_REVERT_TO_VERSION, "disabled", Prop::Bool(!can));
}

/// v5 `selectTimelineVersion(versionId)`.
fn select(app: &mut App, host: &mut dyn Host, key: Option<String>) {
    let st = st(app);
    st.selected = key.clone();
    st.zoom = 1.0;
    st.retries = 0;
    let out = host.out();
    for (li, k) in &st.rows {
        out.class(*li, "is-selected", key.as_ref() == Some(k));
    }
    render_preview(app, host);
    update_revert(app, host);
}

/// v5 `renderTimelinePanelContents()`: the list, then the current version selected.
fn render_contents(app: &mut App, host: &mut dyn Host) {
    st(app).loading = false;
    release_list(app, host);
    render_list(app, host);
    let cur = current_key(app);
    select(app, host, cur);
}

// ---- open / close -------------------------------------------------------------------------------------------------

/// v5 `openTimelinePanel()`. Edits not captured yet are captured first; the list waits for the snapshot reads.
pub fn open(app: &mut App, host: &mut dyn Host) {
    versions::flush(app, host);
    st(app).open = true;
    let out = host.out();
    out.attr(Ui::TIMELINE_PANEL, "aria-hidden", Some("false"));
    out.class(Ui::TIMELINE_PANEL, "visible", true);
    if versions::load(app, host) {
        render_contents(app, host);
    } else {
        st(app).loading = true;
        release_list(app, host);
        host.out().text(Ui::TIMELINE_LIST, "Loading timeline…");
    }
}

/// v5 `closeTimelinePanel()`.
pub fn close(app: &mut App, host: &mut dyn Host) {
    let st = st(app);
    st.open = false;
    st.loading = false;
    st.selected = None;
    let out = host.out();
    out.class(Ui::TIMELINE_PANEL, "visible", false);
    out.attr(Ui::TIMELINE_PANEL, "aria-hidden", Some("true"));
}

/// `doc::versions`: the snapshot reads finished.
pub fn assets_loaded(app: &mut App, host: &mut dyn Host) {
    if is_open(app) && app.st.panels_timeline.loading {
        render_contents(app, host);
    }
}

/// `doc::versions`: entries may have changed (own capture, remote batches). The open list is rebuilt when they did.
pub fn versions_changed(app: &mut App, host: &mut dyn Host) {
    if !is_open(app) || app.st.panels_timeline.loading {
        return;
    }
    let now: Vec<(String, u64)> = display(app).iter().map(|e| (e.key(), e.rec.ts.to_bits())).collect();
    if now != app.st.panels_timeline.shown {
        if !versions::load(app, host) {
            return; // rebuilt by `assets_loaded`
        }
        let sel = app.st.panels_timeline.selected.clone();
        release_list(app, host);
        render_list(app, host);
        let keep = sel.filter(|k| now.iter().any(|e| e.0 == *k));
        let zoom = app.st.panels_timeline.zoom;
        select(app, host, keep.or_else(|| current_key(app)));
        st(app).zoom = zoom;
        apply_zoom(app, host);
    }
}

// ---- restore ------------------------------------------------------------------------------------------------------

fn texts(app: &App, e: &Entry, when: &str) -> (String, String, &'static str) {
    let clean = session::get(app).and_then(|s| pdf_source(&s.doc)).is_some();
    if e.id.is_none() {
        let title = if clean { "Revert to the original PDF?" } else { "Revert to the original document?" };
        (title.into(), "All annotations across the document will be removed. A milestone entry will be added so you can roll forward to the current state again later.".into(), "Slide to revert")
    } else {
        let who = if e.rec.name.is_empty() { "unknown" } else { &e.rec.name };
        ("Restore this version?".into(), format!("Roll the document back to the version saved by {who} on {when}.\nA new milestone entry will be added so you can roll forward again later."), "Slide to restore")
    }
}

fn when_of(app: &App, host: &mut dyn Host, e: &Entry) -> String {
    let clean = session::get(app).and_then(|s| pdf_source(&s.doc)).is_some();
    match e.id {
        None if clean => "the unmodified original PDF".into(),
        None => "the unmodified original document".into(),
        Some(_) => date(host, e.rec.ts, "string", ""),
    }
}

/// The Revert button: the slider, then `versions::restore`.
fn ask_restore(app: &mut App, host: &mut dyn Host) {
    let Some(key) = app.st.panels_timeline.selected.clone() else { return };
    let Some(e) = display(app).into_iter().find(|e| e.key() == key) else { return };
    if snapshot_of(app, &e).is_none_or(|s| s.pages.is_empty()) {
        shell::status(app, host, "Could not restore: version data unavailable.", Status::Error);
        return;
    }
    host.out().prop(Ui::BTN_REVERT_TO_VERSION, "disabled", Prop::Bool(true));
    let when = when_of(app, host, &e);
    let (title, message, text) = texts(app, &e, &when);
    st(app).restoring = Some(key);
    slider::show(app, host, &title, &message, text, answered);
}

fn answered(app: &mut App, host: &mut dyn Host, ok: bool) {
    let key = st(app).restoring.take();
    let e = key.and_then(|k| display(app).into_iter().find(|e| e.key() == k));
    let done = match (ok, e) {
        (true, Some(e)) => match snapshot_of(app, &e) {
            Some(target) => {
                let when = when_of(app, host, &e);
                let clean = session::get(app).and_then(|s| pdf_source(&s.doc)).is_some();
                let orig = e.id.is_none();
                let summary = if orig {
                    (if clean { "Reverted to clean original PDF" } else { "Reverted to clean original document" }).to_string()
                } else if e.rec.name.is_empty() {
                    format!("Restored version from {when}")
                } else {
                    format!("Restored version from {when} ({})", e.rec.name)
                };
                let hint = Hint { milestone: true, kind: (if orig { "revert-original" } else { "restore" }).into(), summary };
                let ok = versions::restore(app, host, &target, &when, hint);
                if ok {
                    let text = match (orig, clean) {
                        (true, true) => "Reverted to original PDF.",
                        (true, false) => "Reverted to original document.",
                        _ => "Document restored.",
                    };
                    shell::status(app, host, text, Status::Info);
                }
                ok
            }
            None => false,
        },
        _ => false,
    };
    host.out().prop(Ui::BTN_REVERT_TO_VERSION, "disabled", Prop::Bool(false));
    if done {
        close(app, host);
    } else if is_open(app) {
        update_revert(app, host);
    }
}

// ---- module contract --------------------------------------------------------------------------------------------

pub fn init(app: &mut App, host: &mut dyn Host) {
    app.st.panels_timeline.zoom = 1.0;
    if app.cfg.has(mode::HEADLESS) {
        return;
    }
    let out = host.out();
    out.listen(Ui::BTN_OPEN_TIMELINE, "click", L_OPEN, 0);
    out.listen(Ui::BTN_CLOSE_TIMELINE, "click", L_CLOSE, 0);
    out.listen(Ui::BTN_REVERT_TO_VERSION, "click", L_REVERT, 0);
    out.listen(Ui::TIMELINE_LIST, "click", L_LIST, 0);
    out.listen(Ui::TIMELINE_LIST, "keydown", L_KEY, 0);
    out.listen(Ui::TIMELINE_PREVIEW_WRAP, "wheel", L_WHEEL, listen::ACTIVE);
    out.listen(Ui::TIMELINE_PREVIEW_WRAP, "touchstart", L_TOUCH, listen::PASSIVE);
    out.listen(Ui::TIMELINE_PREVIEW_WRAP, "touchmove", L_TOUCH_MOVE, listen::ACTIVE);
    out.listen(Ui::TIMELINE_PREVIEW_WRAP, "touchend", L_TOUCH_END, 0);
    out.listen(Ui::TIMELINE_PREVIEW_WRAP, "touchcancel", L_TOUCH_CANCEL, 0);
}

fn toggle(host: &mut dyn Host, g: &Group, collapsed_now: bool) {
    let out = host.out();
    out.class(g.header, "is-collapsed", collapsed_now);
    out.attr(g.header, "aria-expanded", Some(if collapsed_now { "false" } else { "true" }));
    out.prop(g.body, "hidden", Prop::Bool(collapsed_now));
}

fn list_click(app: &mut App, host: &mut dyn Host, path: &crate::abi::Handles) {
    let st = &mut app.st.panels_timeline;
    if let Some(g) = st.clusters.iter_mut().find(|g| path.contains(g.header)) {
        g.collapsed = !g.collapsed;
        let (key, now) = (g.key.clone(), g.collapsed);
        toggle(host, g, now);
        st.clusters_collapsed.retain(|k| *k != key && *k != format!("!{key}"));
        st.clusters_collapsed.push(if now { key } else { format!("!{key}") });
        return;
    }
    if let Some(g) = st.months.iter_mut().find(|g| path.contains(g.header)) {
        g.collapsed = !g.collapsed;
        let (key, now) = (g.key.clone(), g.collapsed);
        toggle(host, g, now);
        st.months_collapsed.retain(|k| *k != key);
        if now {
            st.months_collapsed.push(key);
        }
        return;
    }
    if let Some(key) = st.rows.iter().find(|r| path.contains(r.0)).map(|r| r.1.clone()) {
        select(app, host, Some(key));
    }
}

pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    match event {
        Event::Dom(d) => match d.listener {
            L_OPEN => open(app, host),
            L_CLOSE => close(app, host),
            L_REVERT if app.st.panels_timeline.selected.is_some() => ask_restore(app, host),
            L_LIST if d.ty == ety::CLICK => list_click(app, host, &d.path),
            L_KEY if d.key == "Enter" || d.key == " " => {
                if let Some(key) = app.st.panels_timeline.rows.iter().find(|r| d.path.contains(r.0)).map(|r| r.1.clone()) {
                    select(app, host, Some(key));
                    return ret::PREVENT_DEFAULT;
                }
            }
            L_WHEEL if d.flags & ev::CTRL != 0 => {
                let st = st(app);
                st.zoom = (st.zoom * jsmath::exp(-d.dy * 0.01)).clamp(ZOOM_MIN, ZOOM_MAX);
                apply_zoom(app, host);
                return ret::PREVENT_DEFAULT;
            }
            L_TOUCH if d.touches.len() == 2 => {
                let t: Vec<_> = d.touches.iter().collect();
                let st = st(app);
                st.pinch = (jsmath::hypot(t[0].x - t[1].x, t[0].y - t[1].y), st.zoom);
            }
            L_TOUCH_MOVE if d.touches.len() == 2 && app.st.panels_timeline.pinch.0 > 0.0 => {
                let t: Vec<_> = d.touches.iter().collect();
                let dist = jsmath::hypot(t[0].x - t[1].x, t[0].y - t[1].y);
                if dist > 0.0 {
                    let st = st(app);
                    st.zoom = (st.pinch.1 * dist / st.pinch.0).clamp(ZOOM_MIN, ZOOM_MAX);
                    apply_zoom(app, host);
                }
                return ret::PREVENT_DEFAULT;
            }
            L_TOUCH_END | L_TOUCH_CANCEL => st(app).pinch.0 = 0.0,
            _ => {}
        },
        Event::Timer(T_RETRY) if is_open(app) => {
            let zoom = app.st.panels_timeline.zoom;
            render_preview(app, host);
            st(app).zoom = zoom;
        }
        Event::Msg(m) => match m.chan {
            shell::sig::VIEW if m.data != b"editor" && is_open(app) => close(app, host),
            session::sig::CLOSED => {
                if is_open(app) {
                    close(app, host);
                }
                clear_preview(app, host);
                release_list(app, host);
            }
            _ => {}
        },
        _ => {}
    }
    0
}

/// `panels.timeline.state` → `{open, loading, selected, rows, months, clusters, collapsed, zoom, preview}`.
pub fn test_cmd(app: &mut App, _host: &mut dyn Host, name: &str, _args: &str) -> Option<String> {
    let st = &app.st.panels_timeline;
    match name {
        "state" => Some(format!(
            "{{\"open\":{},\"loading\":{},\"selected\":{},\"rows\":{},\"months\":{},\"clusters\":{},\"zoom\":{},\"preview\":{}}}",
            st.open,
            st.loading,
            st.selected.as_deref().map_or("null".into(), crate::testapi::json_str),
            st.rows.len(),
            st.months.len(),
            st.clusters.len(),
            num(st.zoom),
            st.preview.len(),
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_days() {
        assert_eq!(days(1970, 1, 1), 0);
        assert_eq!(days(2026, 10, 30) - days(2026, 10, 29), 1);
        assert_eq!(days(2026, 3, 1) - days(2026, 2, 28), 1);
        assert_eq!(days(2024, 3, 1) - days(2024, 2, 28), 2);
        assert_eq!(days(2027, 1, 1) - days(2026, 12, 31), 1);
    }
}
