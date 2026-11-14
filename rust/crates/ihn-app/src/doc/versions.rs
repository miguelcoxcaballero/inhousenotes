//! Timeline capture trigger (ARCHITECTURE §4.6), the version-snapshot assets this tab reads, restore, and the
//! conversion of imported 5.x timelines. Owned by W5.3; the rules are `ihn-core::history` (W2.9).
//!
//! * Capture: every local batch (`session::sig::COMMITTED`) marks the document dirty. A capture that would start a new
//!   entry runs after 180 ms of quiet input (5.18.0 `DRIVE_BUILD_IDLE_WINDOW_MS`, the moment 5.18.0 built its next
//!   PDF and captured); one that would coalesce into this tab's newest entry runs at most every [`THROTTLE_MS`]
//!   (SYNC §14.6: ≤ 12 version assets per hour of writing). Pending edits are captured at once on Home / pagehide /
//!   hidden, when the Timeline opens and before a restore.
//! * Assets: a capture stores its snapshot as an asset (SHA-256 in Rust, `blob_new`, `session::put_asset`) and keeps
//!   the bytes here. Other snapshots are read from their blobs on demand ([`load`]): at open the chains the next
//!   capture needs, everything when the Timeline opens.
//! * Restore (5.18.0 `restoreVersion`): `pre-restore` milestone, `plan_restore` in one batch, `restore` /
//!   `revert-original` milestone.
// v5: app-v5.js:captureVersionSnapshot / captureTimelineRecoveryPoint / restoreVersion / applyRestoredPages /
// getCurrentVersionAuthor

use crate::abi::{life, tag, Event};
use crate::app::Mod;
use crate::doc::session;
use crate::editor::{history as undo, pages, viewport};
use crate::panels::{pages_panel, timeline};
use crate::shell::{self, fx, Status};
use crate::testapi::json_str;
use crate::{auth, App, Host};
use ihn_core::history::{self, Hint, Record, SnapItem, SnapPage, Snapshot, Who, CHECKPOINT_EVERY};
use ihn_core::id::{Hash, Id, IdMap};
use ihn_core::op::{Background, Batch, Body, Color, Op, Template, Tool, VersionMeta, Z};
use ihn_core::stamp::Stamp;
use ihn_core::v5::import::{Asset, Import};
use ihn_core::v5::pages::{dimension, A4};
use ihn_core::v5::sha256::sha256;
use ihn_core::v5::timeline::V5Version;

/// v5 `DRIVE_BUILD_IDLE_WINDOW_MS`.
pub const QUIET_MS: f64 = 180.0;
/// Shortest interval between two captures that rewrite the same (coalesced) entry; below the 5-minute window.
pub const THROTTLE_MS: f64 = 290_000.0;
/// MIME type of version-snapshot assets.
pub const MIME: &str = "application/x-ihn6-version";

const W_CAPTURE: u32 = tag(Mod::DocVersions, 1);
const T_CAPTURE: u32 = tag(Mod::DocVersions, 1);

#[derive(Default)]
pub struct State {
    /// Snapshot asset bytes known to this tab.
    cache: Vec<(Hash, Vec<u8>)>,
    /// Blob reads in flight: (request, hash).
    reads: Vec<(u32, Hash)>,
    /// Local edits since the last capture.
    dirty: bool,
    /// This tab's seq the last capture (or check) covered: a commit above it is an edit not captured yet.
    covered: u32,
    timer: bool,
    /// Captures made (test counter).
    captures: u32,
}

fn st(app: &mut App) -> &mut State { &mut app.st.doc_versions }

/// The bytes of a snapshot asset, if this tab holds them.
pub fn bytes<'a>(app: &'a App, h: &Hash) -> Option<&'a [u8]> { app.st.doc_versions.cache.iter().find(|e| e.0 == *h).map(|e| e.1.as_slice()) }

/// The materialised snapshot of a version (`None` while an asset of its chain is not here).
pub fn snapshot(app: &App, h: &Hash) -> Option<Snapshot> { history::materialize(h, &|x| bytes(app, x)) }

/// The snapshot and its delta bases, from the records (pruned ones included).
fn chain(app: &App, h: Hash) -> Vec<Hash> {
    let Some(s) = session::get(app) else { return Vec::new() };
    let mut out = vec![h];
    while out.len() < 64 {
        let last = out[out.len() - 1];
        let base = s.doc.versions.values().find_map(|v| Record::decode(&v.data.v).filter(|r| r.snapshot == last)?.base);
        match base {
            Some(b) if !out.contains(&b) => out.push(b),
            _ => break,
        }
    }
    out
}

/// Starts blob reads for `hashes` that are neither cached nor in flight. True when nothing is being read.
fn want(app: &mut App, host: &mut dyn Host, hashes: Vec<Hash>) -> bool {
    for h in hashes {
        let known = bytes(app, &h).is_some() || st(app).reads.iter().any(|r| r.1 == h);
        let Some(blob) = session::get(app).and_then(|s| s.asset_blob(&h)).filter(|_| !known) else { continue };
        let req = app.req(Mod::DocVersions);
        st(app).reads.push((req, h));
        fx::blob_read(host.out(), req, blob, 0.0, -1.0);
    }
    st(app).reads.is_empty()
}

/// Reads every snapshot the timeline lists (with their chains). True when all readable ones are here.
pub fn load(app: &mut App, host: &mut dyn Host) -> bool {
    let need = session::get(app).map(|s| history::needed_assets(&s.doc)).unwrap_or_default();
    want(app, host, need)
}

/// The chains the next capture of this tab reads: the newest entry's and this actor's tail.
fn preload(app: &mut App, host: &mut dyn Host) {
    let Some(s) = session::get(app) else { return };
    let actor = s.author.as_ref().map(|a| a.actor.0);
    let list = history::entries(&s.doc);
    let mine = s.doc.versions.iter().rev().filter(|(id, _)| Some((id.0 >> 64) as u64) == actor).find_map(|(_, v)| Record::decode(&v.data.v));
    let mut need = Vec::new();
    for h in list.last().map(|e| e.1.snapshot).into_iter().chain(mine.map(|r| r.snapshot)) {
        need.extend(chain(app, h));
    }
    want(app, host, need);
}

fn who(app: &App) -> (String, String, String) {
    let p = auth::profile(app);
    let email = p.map_or("", |p| p.email.as_str()).to_string();
    let name = p.map(|p| p.name.as_str()).filter(|n| !n.is_empty()).map_or_else(|| if email.is_empty() { "You".to_string() } else { email.clone() }, str::to_string);
    (email, name, p.map_or("", |p| p.photo.as_str()).to_string())
}

/// Captures the document now (5.18.0 `captureVersionSnapshot`): `None` when nothing was written (no writable
/// session, no pages, or the content equals the newest entry and `hint` is not a milestone).
pub fn capture(app: &mut App, host: &mut dyn Host, hint: Option<&Hint>, ts: Option<f64>) -> Option<Id> {
    st(app).dirty = false;
    let ts = ts.unwrap_or_else(|| host.time());
    let (email, name, photo) = who(app);
    let s = session::get(app).filter(|s| s.writable())?;
    let actor = s.author.as_ref()?.actor;
    let device = format!("{:016x}", actor.0);
    let w = Who { email: &email, name: &name, photo: &photo, device: &device };
    let mut planned = history::plan_capture(&s.doc, actor, &w, ts, hint, &|h| bytes(app, h))?;
    let asset = std::mem::take(&mut planned.asset);
    let hash = sha256(&asset);
    let obj = fx::new_obj(app);
    fx::blob_new(host.out(), obj, MIME, &asset);
    session::put_asset(app, host, hash, obj, MIME, asset.len() as f64);
    let vs = st(app);
    if !vs.cache.iter().any(|e| e.0 == hash) {
        vs.cache.push((hash, asset));
    }
    let mut id = None;
    let e = session::edit(app, host, |tx| {
        let d = tx.doc();
        id = Some(planned.commit(hash, d, tx));
    });
    e.as_ref()?;
    cover(app);
    st(app).captures += 1;
    timeline::versions_changed(app, host);
    id
}

/// This tab's newest batch in the document.
fn own_seq(app: &App) -> u32 { session::get(app).and_then(|s| Some(s.doc.vv.get(s.author.as_ref()?.actor))).unwrap_or(0) }

fn cover(app: &mut App) {
    let seq = own_seq(app);
    let vs = st(app);
    vs.covered = vs.covered.max(seq);
    vs.dirty = false;
}

/// An automatic capture. Edits that leave every page as the newest entry shows it (a Drive-side rename the session
/// applied, an undo back to it) are not captured on their own.
fn auto(app: &mut App, host: &mut dyn Host) {
    let same = session::get(app).and_then(|s| {
        let (_, last) = history::entries(&s.doc).pop()?;
        Some(snapshot(app, &last.snapshot)?.pages == Snapshot::of(&s.doc).pages)
    });
    if same == Some(true) {
        cover(app);
    } else {
        capture(app, host, None, None);
    }
}

/// Captures pending local edits now (Home, pagehide, Timeline open).
pub fn flush(app: &mut App, host: &mut dyn Host) {
    if st(app).dirty {
        auto(app, host);
    }
}

/// When the next capture may run: after the quiet window, or (for one that would coalesce into this tab's newest
/// entry) once the throttle interval since that entry passed.
fn schedule(app: &mut App, host: &mut dyn Host) {
    if st(app).timer {
        return;
    }
    let now = host.time();
    let wait = session::get(app).and_then(|s| {
        let actor = s.author.as_ref()?.actor.0;
        let (id, last) = history::entries(&s.doc).pop()?;
        let mine = (id.0 >> 64) as u64 == actor && !last.milestone && now - last.ts < history::COALESCE_MS;
        mine.then(|| (last.ts + THROTTLE_MS - now).max(0.0))
    });
    match wait {
        Some(ms) if ms > 0.0 => {
            st(app).timer = true;
            app.sched.timer(host.out(), T_CAPTURE, ms);
        }
        _ => {
            app.sched.defer(W_CAPTURE, QUIET_MS, true);
        }
    }
}

// ---- restore ------------------------------------------------------------------------------------------------------

/// v5 `restoreVersion` after the slider: the `pre-restore` milestone (`Before restoring <when>`), the restore batch,
/// the editor reset, then the `restore` / `revert-original` milestone. False when nothing was restored.
pub fn restore(app: &mut App, host: &mut dyn Host, target: &Snapshot, when: &str, hint: Hint) -> bool {
    let pre = Hint { milestone: true, kind: "pre-restore".into(), summary: format!("Before restoring {when}") };
    if capture(app, host, Some(&pre), None).is_none() {
        shell::status(app, host, "Restore cancelled: a safe recovery point could not be created.", Status::Error);
        return false;
    }
    let Some(ops) = session::get(app).map(|s| history::plan_restore(&s.doc, target)) else { return false };
    match session::edit(app, host, |tx| ops.into_iter().for_each(|op| tx.push(op))) {
        Some(e) => {
            undo::clear(app, host);
            pages::changed(app, host, &e.changes);
            pages::set_active_page(app, 0);
            viewport::center_on_top(app, host);
            pages_panel::refresh(app, host);
        }
        None => {}
    }
    capture(app, host, Some(&hint), None);
    true
}

// ---- 5.x timelines --------------------------------------------------------------------------------------------------

fn stroke_body(tool: &str, color: &str, width: f64, xy: &[f64]) -> Option<Body> {
    let tool = match tool {
        "eraser-stroke" => return None,
        "highlighter" => Tool::Highlighter,
        "eraser-area" => Tool::LegacyEraseArea,
        _ => Tool::Pen,
    };
    Some(Body::Stroke { tool, color: Color::parse(color), width: if width.is_finite() { width } else { 2.0 }, xy: xy.to_vec(), p: None })
}

/// One 5.x timeline entry as a v6 snapshot. `z`: draw positions of the items the import created (an item absent
/// from the import follows the item before it); `bgs`: backgrounds of the imported pages in order.
fn v5_snapshot(v: &V5Version, z: &IdMap<Z>, bgs: &[Background], assets: &mut Vec<Asset>) -> Snapshot {
    let pdf = bgs.iter().find_map(|b| matches!(b, Background::Pdf { .. }).then_some(*b));
    let mut seen = std::collections::HashSet::new();
    let pages = v
        .pages
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let pid = if p.page_id.is_empty() { format!("legacy-page-{}", i + 1) } else { p.page_id.clone() };
            let source = if p.background_source.is_empty() { if p.pdf_page_index.is_some() { "pdf" } else { "template" } } else { p.background_source.as_str() };
            let background = match (source, pdf, bgs.get(i)) {
                ("pdf", Some(Background::Pdf { asset, .. }), _) => Background::Pdf { asset, page: p.pdf_page_index.unwrap_or(1.0).max(1.0) as u32 },
                ("custom", _, Some(b @ Background::Custom { .. })) => *b,
                _ => Background::Template,
            };
            let template = match p.template_kind {
                "agenda" => Template::Agenda,
                "diary" => Template::Diary,
                _ => Template::Default,
            };
            let mut item = |id: &str| if seen.insert(id.to_string()) { Id::legacy(id) } else { Id::legacy(&format!("{pid}:{id}")) };
            let mut runs: [Vec<SnapItem>; 2] = [Vec::new(), Vec::new()];
            for s in &p.strokes {
                if let Some(body) = stroke_body(&s.tool, &s.color, s.width, &s.xy) {
                    runs[1].push(SnapItem { id: item(&s.id), z: Z::default(), body });
                }
            }
            for im in &p.images {
                let Some((mime, data)) = im.src.strip_prefix("data:").and_then(|r| r.split_once(',')) else { continue };
                let Some(bytes) = mime.strip_suffix(";base64").and_then(|_| ihn_core::codec::b64_decode(data.trim().as_bytes())) else { continue };
                let asset = sha256(&bytes);
                if !assets.iter().any(|a| a.hash == asset) {
                    assets.push(Asset { hash: asset, mime: mime.trim_end_matches(";base64").into(), bytes });
                }
                let f = |v: f64| if v.is_finite() { v } else { 0.0 };
                let body = Body::Image { asset, cx: f(im.x), cy: f(im.y), w: f(im.width), h: f(im.height), rotation: f(im.rotation) };
                runs[0].push(SnapItem { id: item(&im.id), z: Z::default(), body });
            }
            for run in &mut runs {
                let mut prev = Z::default();
                for it in run.iter_mut() {
                    it.z = z.get(&it.id).copied().unwrap_or(Z { at: prev.at, sub: prev.sub.wrapping_add(1) });
                    prev = it.z;
                }
                ihn_core::sort::by_key(run, |it| (it.z, it.id));
            }
            let [images, strokes] = runs;
            SnapPage {
                id: Id::legacy(&pid),
                template,
                size: (dimension(p.page_width, A4.0), dimension(p.page_height, A4.1)),
                background,
                side_panel: p.side_panel.as_ref().map(|s| s.as_bytes().to_vec()),
                items: images.into_iter().chain(strokes).collect(),
            }
        })
        .collect();
    Snapshot { name: v.export_name.clone(), pages }
}

/// Adds the imported 5.x timeline (`Import::versions`) to an import: one more batch of the import actor with a
/// `VersionAdd` per entry (ids `Id::legacy(5.x id)`, so every device importing the same file writes the same
/// versions) and the snapshot assets (delta chains, a checkpoint every 8). Runs where the import runs (the export
/// worker for large files); `doc::v5open` and the migration call it after `v5::import`.
pub fn add_v5(imp: &mut Import) {
    let versions = std::mem::take(&mut imp.versions);
    let Some(last) = imp.batches.last().filter(|_| !versions.is_empty()) else { return };
    let (actor, seq, clock) = (last.actor, last.seq + 1, last.clock);
    let mut z = IdMap::default();
    let mut bgs = Vec::new();
    for op in imp.batches.iter().flat_map(|b| &b.ops) {
        match op {
            Op::ItemPut { item, z: at, .. } => {
                z.insert(*item, *at);
            }
            Op::PageNew { background, .. } => bgs.push(*background),
            _ => {}
        }
    }
    let (mut ops, mut stamps) = (Vec::new(), Vec::new());
    let mut prev: Option<(Hash, Snapshot, u8)> = None;
    for (i, v) in versions.iter().enumerate() {
        let snap = v5_snapshot(v, &z, &bgs, &mut imp.assets);
        let base = prev.as_ref().filter(|p| p.2 + 1 < CHECKPOINT_EVERY);
        let bytes = snap.encode(base.map(|p| (&p.0, &p.1)));
        let hash = sha256(&bytes);
        let name = if v.author_name.is_empty() { if v.author_email.is_empty() { "Unknown editor" } else { &v.author_email } } else { &v.author_name };
        let record = Record {
            ts: v.ts,
            email: v.author_email.clone(),
            name: name.to_string(),
            photo: v.author_photo.clone(),
            device: v.device_id.clone(),
            kind: v.kind.clone(),
            milestone: v.is_milestone,
            summary: v.summary.clone(),
            content: snap.content_hash(),
            snapshot: hash,
            base: base.map(|p| p.0),
            depth: base.map_or(0, |p| p.2 + 1),
        };
        ops.push(Op::VersionAdd { meta: VersionMeta { id: Id::legacy(&v.id), data: record.encode() } });
        stamps.push(Stamp::new(clock, actor, 0x4000_0000 + i as u32));
        if !imp.assets.iter().any(|a| a.hash == hash) {
            imp.assets.push(Asset { hash, mime: MIME.into(), bytes });
        }
        prev = Some((hash, snap, record.depth));
    }
    imp.batches.push(Batch { actor, seq, clock, ops, stamps: Some(stamps) });
}

// ---- module contract --------------------------------------------------------------------------------------------

pub fn init(_app: &mut App, _host: &mut dyn Host) {}

pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    match event {
        Event::Msg(m) => match m.chan {
            session::sig::OPENED => {
                *st(app) = State::default();
                cover(app);
                preload(app, host);
            }
            session::sig::COMMITTED => {
                if own_seq(app) > st(app).covered {
                    st(app).dirty = true;
                    schedule(app, host);
                }
            }
            session::sig::CHANGED => timeline::versions_changed(app, host),
            session::sig::CLOSED => *st(app) = State::default(),
            shell::sig::VIEW if m.data != b"editor" => flush(app, host),
            _ => {}
        },
        Event::Life(l) if l.code == life::PAGEHIDE || (l.code == life::VISIBILITY && l.a != 0.0) => flush(app, host),
        Event::Timer(T_CAPTURE) => {
            st(app).timer = false;
            app.sched.defer(W_CAPTURE, QUIET_MS, true);
        }
        Event::Work { id: W_CAPTURE, stale: false } if app.st.doc_versions.dirty => {
            // Wait for the snapshot reads the capture uses (its delta base, the summary's previous entry).
            if st(app).reads.is_empty() {
                auto(app, host);
            } else {
                st(app).timer = true;
                app.sched.timer(host.out(), T_CAPTURE, 100.0);
            }
        }
        Event::Io(io) => {
            let vs = st(app);
            if let Some(at) = vs.reads.iter().position(|r| r.0 == io.req) {
                let (_, h) = vs.reads.remove(at);
                if io.status >= 0 && sha256(io.data) == h && !vs.cache.iter().any(|e| e.0 == h) {
                    vs.cache.push((h, io.data.to_vec()));
                }
                if vs.reads.is_empty() {
                    timeline::assets_loaded(app, host);
                }
            }
        }
        _ => {}
    }
    0
}

/// `doc.versions.state` → the listed entries (oldest first) and the capture counters; `doc.versions.capture`
/// `{ts?, milestone?, kind?, summary?}` → captures now (`{id}` or `{id:null}`).
pub fn test_cmd(app: &mut App, host: &mut dyn Host, name: &str, args: &str) -> Option<String> {
    match name {
        "state" => {
            let list = session::get(app).map(|s| history::entries(&s.doc)).unwrap_or_default();
            let entries: Vec<String> = list
                .iter()
                .map(|(id, r)| {
                    format!(
                        "{{\"id\":\"{}\",\"ts\":{},\"kind\":{},\"milestone\":{},\"summary\":{},\"name\":{},\"email\":{},\"device\":{},\"depth\":{},\"delta\":{},\"cached\":{},\"stored\":{}}}",
                        id.hex(),
                        ihn_core::jsfmt::num(r.ts),
                        json_str(&r.kind),
                        r.milestone,
                        json_str(&r.summary),
                        json_str(&r.name),
                        json_str(&r.email),
                        json_str(&r.device),
                        r.depth,
                        r.base.is_some(),
                        bytes(app, &r.snapshot).is_some(),
                        session::get(app).is_some_and(|s| s.asset_blob(&r.snapshot).is_some()),
                    )
                })
                .collect();
            let st = &app.st.doc_versions;
            Some(format!("{{\"entries\":[{}],\"captures\":{},\"dirty\":{},\"reads\":{}}}", entries.join(","), st.captures, st.dirty, st.reads.len()))
        }
        "capture" => {
            let j = crate::shell::json::Json::parse(args).unwrap_or(crate::shell::json::Json::Null);
            let s = |k: &str| j.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
            let milestone = matches!(j.get("milestone"), Some(crate::shell::json::Json::Bool(true)));
            let hint = (milestone || !s("kind").is_empty() || !s("summary").is_empty()).then(|| Hint { milestone, kind: s("kind"), summary: s("summary") });
            let ts = j.get("ts").and_then(|v| v.as_f64());
            let id = capture(app, host, hint.as_ref(), ts);
            Some(format!("{{\"id\":{}}}", id.map_or("null".into(), |i| format!("\"{}\"", i.hex()))))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ihn_core::v5::pages::{V5Page, V5Stroke};

    fn v5(id: &str, ts: f64, strokes: &[&str]) -> V5Version {
        let page = V5Page {
            page_id: "pg1".into(),
            template_kind: "default",
            strokes: strokes.iter().map(|s| V5Stroke { id: s.to_string(), tool: "pen".into(), color: "#1a1a1a".into(), width: 2.0, xy: vec![1.0, 2.0, 3.0, 4.0], ..Default::default() }).collect(),
            ..Default::default()
        };
        V5Version { id: id.into(), ts, author_name: "Ana".into(), summary: "+1 stroke".into(), kind: "autosave".into(), pages: vec![page], ..Default::default() }
    }

    #[test]
    fn imported_timeline_becomes_one_batch_of_versions_with_delta_chains() {
        let a = ihn_core::id::ActorId::import("r");
        let mut imp = Import { batches: vec![Batch { actor: a, seq: 1, clock: 7, ops: vec![], stamps: Some(vec![]) }], ..Default::default() };
        imp.versions = (0..10).map(|i| v5(&format!("v{i}"), 1000.0 * i as f64, &(0..=i).map(|k| ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"][k]).collect::<Vec<_>>())).collect();
        add_v5(&mut imp);
        assert_eq!(imp.batches.len(), 2);
        let b = &imp.batches[1];
        assert_eq!((b.actor, b.seq, b.ops.len()), (a, 2, 10));
        let mut doc = ihn_core::doc::Doc::default();
        doc.apply(&imp.batches[0]);
        doc.apply(b);
        let list = history::entries(&doc);
        assert_eq!(list.len(), 10);
        assert_eq!(list[0].0, Id::legacy("v0"));
        assert_eq!(list.iter().map(|e| e.1.depth).collect::<Vec<_>>(), [0, 1, 2, 3, 4, 5, 6, 7, 0, 1]);
        let get = |h: &Hash| imp.assets.iter().find(|a| a.hash == *h).map(|a| a.bytes.as_slice());
        let snap = history::materialize(&list[9].1.snapshot, &get).expect("chain readable");
        assert_eq!(snap.pages.len(), 1);
        assert_eq!(snap.pages[0].id, Id::legacy("pg1"));
        assert_eq!(snap.pages[0].items.len(), 10);
        assert_eq!(list[9].1.content, snap.content_hash());
        assert_eq!(list[3].1.name, "Ana");
    }
}
