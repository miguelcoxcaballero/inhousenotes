//! The PDF export: who writes it and when (SYNC §14), the upload loop with the same-head fast retry (§14.2), the
//! known-export table (§14.3–14.4), head classification (§15.1) and the document-name rule (§14.7). Owner: W2.7.
//!
//! Building a PDF and merging a downloaded revision are host work (export worker, pdf.js, `v5::import`): the part
//! asks for them through [`Desk`] (`Core::export`) and the session binding answers there, then calls `tick`.

use crate::*;
use ihn_core::doc::ExportPhase;
use ihn_core::id::fnv1a64;

pub const IDLE_MS: f64 = 10_000.0;
pub const MIN_INTERVAL_MS: f64 = 60_000.0;
pub const BUSY_INTERVAL_MS: f64 = 300_000.0;
pub const LOOPS: u32 = 3;
pub const FAST_RETRIES: u32 = 10;
pub const LOOP_PAUSE_MS: f64 = 30_000.0;
/// A new head nobody described yet is downloaded only after this grace: the descriptor usually arrives first
/// through `ExportDone` or the exporter's next relay write (§14.3).
pub const UNKNOWN_HEAD_GRACE_MS: f64 = 6_000.0;

/// Test command hook: `__IHN_TEST_API__.cmd('sync.<module>.<name>', args)` (JSON text in, JSON text out).
pub fn test_cmd(name: &str, args: &str) -> Option<String> {
    match name {
        "classify" => Some(format!("\"{:?}\"", classify(args.trim_matches('"')))),
        _ => None,
    }
}

/// This module's protocol part for a session.
pub fn part() -> Option<Box<dyn crate::Part>> { Some(Box::new(Export::default())) }

/// §15.1, from a revision's PDF `Keywords`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Class {
    /// Contains `;IHN6:`: `load_snapshot` (merge).
    V6,
    /// Starts with `STROKES_`: `v5::import` with the revision's import actor, then `receive`.
    V5,
    /// Neither: a new document imports its pages; an existing v6 document keeps its state and saves the revision as
    /// `"<name> (recovered Drive version).pdf"` before the next export.
    Plain,
}

pub fn classify(keywords: &str) -> Class {
    if keywords.contains(";IHN6:") {
        Class::V6
    } else if keywords.starts_with("STROKES_") {
        Class::V5
    } else {
        Class::Plain
    }
}

/// Name of the sibling copy that preserves a foreign revision (§15.1).
pub fn recovered_name(name: &str) -> String { format!("{} (recovered Drive version).pdf", name.strip_suffix(".pdf").unwrap_or(name)) }

/// §14.7: an export pushes the Drive `name` only when the `DocName` register is newer than the stamp at which
/// `docs.drive_name` was observed (a rename made outside the editor is never reverted).
pub fn push_name(register: &str, register_at: Stamp, drive_name: &Option<(String, Stamp)>) -> bool {
    match drive_name {
        Some((v, at)) => v != register && register_at > *at,
        None => true,
    }
}

/// `appProperties.ihn6_vv`: compact hash of the exported VV (ARCHITECTURE §8.2).
pub fn vv_tag(vv: &VersionVector) -> String {
    let mut w = Writer::default();
    vv.encode(&mut w);
    format!("{:016x}", fnv1a64(&w.buf))
}

/// Requests to the host and their answers. Out = set by the part, In = set by the binding (then `tick`).
#[derive(Default, Debug)]
pub struct Desk {
    /// In: export now (user Export, or the headless page found `export_pending`); cleared when that export is done.
    pub force: bool,
    /// Out: build the export of the current state (job number).
    pub build: Option<u32>,
    /// In: `(job, Some((bytes, VV the bytes hold)))`, or `None` when building failed.
    pub built: Option<(u32, Option<(Vec<u8>, VersionVector)>)>,
    /// Out: `(job, head, modifiedTime ms, bytes)`: classify (§15.1, [`classify`]) and merge this revision; for
    /// [`Class::Plain`] on an existing document also save the sibling copy ([`recovered_name`]).
    pub merge: Option<(u32, String, u64, Vec<u8>)>,
    /// In: `(job, Some(VV the revision holds))` (an import actor's batches for 5.x, empty for a plain PDF), or
    /// `None` when it could not be read.
    pub merged: Option<(u32, Option<VersionVector>)>,
    /// Out: the Drive name changed outside the editor (§14.7): emit a `DocName` op stamped now if the register
    /// differs. The binding takes it.
    pub rename: Option<String>,
    /// Out: head revision of the newest export this session uploaded (export to the user waits for it).
    pub done: Option<String>,
}

#[derive(Clone, PartialEq, Debug)]
enum St {
    Idle,
    Build(u32),
    Check(u32),
    Fetch { req: u32, head: String, modified: u64 },
    Merge { job: u32, head: String },
    Put(u32),
    /// After a 412: the head and etag again.
    Recheck(u32),
}

/// Why the current job runs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Why {
    Export,
    /// Only a download (a relay gap or an unknown head); no upload follows.
    Fetch,
}

pub struct Export {
    st: St,
    why: Why,
    job: u32,
    bytes: Vec<u8>,
    vv: VersionVector,
    /// Head the current build is based on.
    base: String,
    /// Head of the last CheckHead (fast retries need it unchanged).
    checked: String,
    loops: u32,
    fast: u32,
    fails: u32,
    retry_at: f64,
    leaving: bool,
    /// Local time of the latest remote batch (export interval under remote activity).
    remote_at: f64,
    seen: u64,
    /// Unknown head and the local time it was first seen.
    unknown: Option<(String, f64)>,
}

impl Default for Export {
    fn default() -> Export {
        Export {
            st: St::Idle,
            why: Why::Export,
            job: 0,
            bytes: Vec::new(),
            vv: VersionVector::default(),
            base: String::new(),
            checked: String::new(),
            loops: 0,
            fast: 0,
            fails: 0,
            retry_at: f64::NEG_INFINITY,
            leaving: false,
            remote_at: f64::NEG_INFINITY,
            seen: 0,
            unknown: None,
        }
    }
}

fn vv_sum(vv: &VersionVector) -> u64 { vv.iter().map(|(_, s)| s as u64).sum::<u64>() + vv.extras().count() as u64 }

impl Export {
    fn head(cx: &Core) -> Option<String> { cx.meta.as_ref().map(|m| m.0.head.clone()).filter(|h| !h.is_empty()) }
    fn known(cx: &Core, head: &str) -> Option<VersionVector> { cx.stored.exports.iter().rev().find(|(h, _)| h == head).map(|(_, vv)| vv.clone()) }
    /// The replica holds batches the newest known export lacks.
    fn needed(cx: &Core, r: &dyn Replica) -> bool {
        let rv = r.version_vector();
        !rv.is_empty() && !cx.stored.export_vv().is_some_and(|vv| vv.covers(rv))
    }

    fn idle(&mut self, cx: &mut Core) {
        self.st = St::Idle;
        cx.export.build = None;
        cx.status.export = ExportPhase::Idle;
    }

    fn fail(&mut self, cx: &mut Core, code: &str) {
        self.fails += 1;
        self.retry_at = cx.now + cx.backoff(2_000.0, 300_000.0, self.fails);
        cx.wake(self.retry_at);
        if self.why == Why::Export {
            cx.persist(SyncRecord::Flags { relay_pending: None, export_pending: Some(true), created: None });
            cx.status.export = ExportPhase::Failed { code: code.into(), detail: String::new() };
        }
        self.st = St::Idle;
        cx.export.build = None;
    }

    fn build(&mut self, cx: &mut Core) {
        self.job += 1;
        self.base = Self::head(cx).unwrap_or_default();
        self.st = St::Build(self.job);
        cx.export.build = Some(self.job);
        cx.export.built = None;
        cx.status.export = ExportPhase::Building;
    }

    fn check(&mut self, cx: &mut Core) {
        self.st = St::Check(cx.http(HttpReq::Meta { file: cx.doc_id.clone() }));
        cx.status.export = ExportPhase::Checking;
    }

    fn fetch(&mut self, cx: &mut Core, head: String, modified: u64) {
        let req = cx.http(HttpReq::Media { file: cx.doc_id.clone() });
        self.st = St::Fetch { req, head, modified };
        cx.status.export = ExportPhase::Merging;
    }

    fn put(&mut self, cx: &mut Core, etag: &str) {
        let app = vec![("ihn6".into(), "1".into()), ("ihn6_vv".into(), vv_tag(&self.vv))];
        let req = HttpReq::Put { file: cx.doc_id.clone(), if_match: etag.into(), bytes: self.bytes.clone(), app };
        self.st = St::Put(cx.http(req));
        cx.status.export = ExportPhase::Uploading;
    }

    /// CheckHead's answer (also used after a 412 whose head changed).
    fn head_answer(&mut self, cx: &mut Core, meta: &DriveMeta, etag: &str) {
        self.checked = meta.head.clone();
        let covered = Self::known(cx, &meta.head).is_some_and(|vv| self.vv.covers(&vv));
        if meta.head == self.base || covered {
            return self.put(cx, etag);
        }
        self.loops += 1;
        if self.loops > LOOPS {
            self.loops = 0;
            self.retry_at = cx.now + LOOP_PAUSE_MS;
            cx.wake(self.retry_at);
            return self.idle(cx);
        }
        self.fetch(cx, meta.head.clone(), meta.modified_ms);
    }

    fn done(&mut self, cx: &mut Core, head: String, etag: &str) {
        let vv = std::mem::take(&mut self.vv);
        cx.persist(SyncRecord::KnownExport { head: head.clone(), vv: vv.clone() });
        cx.persist(SyncRecord::LastExport { head: head.clone(), vv: vv.clone(), etag: etag.into(), at: cx.now.max(0.0) as u64 });
        cx.persist(SyncRecord::Flags { relay_pending: None, export_pending: Some(false), created: None });
        cx.send_all(&Frame::ExportDone { head: head.clone(), vv });
        // The descriptor goes into our next relay write (§14.2 Done), so relay-only readers need no download.
        cx.flush = cx.flush.max(Flush::Soon);
        cx.export.done = Some(head);
        cx.export.force = false;
        (self.fails, self.loops, self.fast) = (0, 0, 0);
        self.bytes = Vec::new();
        self.leaving = false;
        self.idle(cx);
    }

    /// §14.1: start an export or a download when one is due.
    fn decide(&mut self, cx: &mut Core, r: &dyn Replica) {
        if !cx.networking() || !cx.polled || cx.now < self.retry_at {
            if cx.now < self.retry_at {
                cx.wake(self.retry_at);
            }
            return;
        }
        let head = Self::head(cx);
        // Downloads: a relay gap (§8.6 step 3) or an unknown head after its grace (§14.3).
        let unknown_due = self.unknown.as_ref().filter(|(h, _)| Some(h) == head.as_ref() && Self::known(cx, h).is_none()).map(|(_, at)| at + UNKNOWN_HEAD_GRACE_MS);
        if cx.need_export || unknown_due.is_some_and(|at| cx.now >= at) {
            if let Some(head) = head.clone() {
                self.why = Why::Fetch;
                let modified = cx.meta.as_ref().map_or(0, |m| m.0.modified_ms);
                return self.fetch(cx, head, modified);
            }
        }
        if let Some(at) = unknown_due {
            cx.wake(at);
        }
        if !cx.can_edit || cx.mode == Mode::Viewer || cx.contact {
            return;
        }
        let needed = Self::needed(cx, r);
        if cx.export.force && !needed && head.as_ref().is_some_and(|h| Self::known(cx, h).is_some()) {
            // Nothing to add: the head already is a complete export.
            cx.export.done = head;
            cx.export.force = false;
            return;
        }
        let auto = || {
            let last = cx.stored.last_export.as_ref().map_or(f64::NEG_INFINITY, |l| l.3 as f64);
            let interval = if cx.now - self.remote_at < IDLE_MS { BUSY_INTERVAL_MS } else { MIN_INTERVAL_MS };
            (last + interval).max(cx.last_input_at + IDLE_MS)
        };
        let at = if cx.export.force || (self.leaving && needed && cx.mode == Mode::Alone) {
            cx.now
        } else if needed && cx.is_scribe() && cx.visible {
            auto()
        } else {
            return;
        };
        if cx.now >= at {
            self.why = Why::Export;
            self.loops = 0;
            // Never build over a revision nobody merged yet (a 5.x save, a foreign app): merge it first (§15.3).
            if let Some(head) = head.filter(|h| Self::known(cx, h).is_none()) {
                let modified = cx.meta.as_ref().map_or(0, |m| m.0.modified_ms);
                return self.fetch(cx, head, modified);
            }
            self.build(cx);
        } else {
            cx.wake(at);
        }
    }
}

impl Part for Export {
    fn local_batch(&mut self, _cx: &mut Core, _b: &EncodedBatch, _r: &mut dyn Replica) { self.seen += 1 }

    fn meta(&mut self, cx: &mut Core, _r: &mut dyn Replica) {
        let Some((meta, _)) = cx.meta.as_ref() else { return };
        let (head, name) = (meta.head.clone(), meta.name.clone());
        if !head.is_empty() && Self::known(cx, &head).is_none() && self.unknown.as_ref().is_none_or(|u| u.0 != head) {
            self.unknown = Some((head, cx.now));
        }
        // §14.7: a Drive name we did not push or observe is a rename made outside the editor.
        if !name.is_empty() && cx.stored.drive_name.as_ref().is_none_or(|(v, _)| *v != name) {
            let at = Stamp::new(cx.now.max(0.0) as u64, cx.actor, 0);
            // The first observation of a document only records the name (it opened from that name's state).
            if cx.stored.drive_name.is_some() {
                cx.export.rename = Some(name.clone());
            }
            cx.persist(SyncRecord::DriveName { v: name, observed_at: at });
        }
    }

    fn http(&mut self, cx: &mut Core, id: u32, status: u16, etag: &str, body: &[u8], _r: &mut dyn Replica) {
        match self.st.clone() {
            St::Check(req) | St::Recheck(req) if req == id => {
                let Some(meta) = (status == 200).then(|| DriveMeta::decode(body)).flatten() else {
                    return if matches!(status, 403 | 404) { self.idle(cx) } else { self.fail(cx, &status.to_string()) };
                };
                if matches!(self.st, St::Recheck(_)) && meta.head == self.checked {
                    // Metadata churn (another device's record write moved the etag): the same bytes at once.
                    self.fast += 1;
                    if self.fast <= FAST_RETRIES {
                        return self.put(cx, etag);
                    }
                    self.fast = 0;
                    return self.fail(cx, "412");
                }
                self.head_answer(cx, &meta, etag);
            }
            St::Fetch { req, head, modified } if req == id => {
                if status != 200 {
                    return if status == 404 { self.idle(cx) } else { self.fail(cx, &status.to_string()) };
                }
                self.job += 1;
                cx.export.merge = Some((self.job, head.clone(), modified, body.to_vec()));
                cx.export.merged = None;
                self.st = St::Merge { job: self.job, head };
            }
            St::Put(req) if req == id => match status {
                s if (200..300).contains(&s) => {
                    let head = String::from_utf8_lossy(body).trim().to_string();
                    self.done(cx, head, etag);
                }
                412 => {
                    self.st = St::Recheck(cx.http(HttpReq::Meta { file: cx.doc_id.clone() }));
                    cx.status.export = ExportPhase::Checking;
                }
                403 => self.idle(cx),
                s => self.fail(cx, &s.to_string()),
            },
            _ => {}
        }
    }

    fn frame(&mut self, cx: &mut Core, _from: PeerRef, f: &Frame, _r: &mut dyn Replica) {
        if let Frame::ExportDone { head, vv } = f {
            if !head.is_empty() && Self::known(cx, head).is_none() {
                cx.persist(SyncRecord::KnownExport { head: head.clone(), vv: vv.clone() });
            }
        }
    }

    fn change(&mut self, cx: &mut Core, c: Change, r: &mut dyn Replica) {
        match c {
            // An accepted canvas contact aborts the job before its upload (§14.1); it stays due.
            Change::Contact if matches!(self.st, St::Build(_) | St::Check(_)) && self.why == Why::Export => self.idle(cx),
            Change::Leave | Change::Suspend => {
                self.leaving = true;
            }
            Change::PageHide if cx.mode == Mode::Alone && cx.can_edit && Self::needed(cx, r) => {
                // `pagehide` never exports: the headless page or the next session does it (§14.5).
                cx.persist(SyncRecord::Flags { relay_pending: None, export_pending: Some(true), created: None });
            }
            Change::Visible => self.leaving = false,
            Change::CanEdit(false) if self.why == Why::Export && self.st != St::Idle => self.idle(cx),
            _ => {}
        }
    }

    fn tick(&mut self, cx: &mut Core, r: &mut dyn Replica) {
        let sum = vv_sum(r.version_vector());
        if sum > self.seen {
            self.remote_at = cx.now;
        }
        self.seen = sum;
        match self.st.clone() {
            St::Build(job) => {
                if let Some((j, out)) = cx.export.built.take() {
                    if j != job {
                        return;
                    }
                    cx.export.build = None;
                    match out {
                        Some((bytes, vv)) => {
                            (self.bytes, self.vv) = (bytes, vv);
                            self.check(cx);
                        }
                        None => self.fail(cx, "build"),
                    }
                }
            }
            St::Merge { job, head } => {
                if let Some((j, vv)) = cx.export.merged.take() {
                    if j != job {
                        return;
                    }
                    cx.export.merge = None;
                    match vv {
                        Some(vv) => {
                            cx.persist(SyncRecord::KnownExport { head, vv });
                            cx.need_export = false;
                            self.seen = vv_sum(r.version_vector());
                            if self.why == Why::Export {
                                // Merged: build again on the new head (counts as a loop, §14.2).
                                self.build(cx);
                            } else {
                                self.idle(cx);
                            }
                        }
                        None => self.fail(cx, "merge"),
                    }
                }
            }
            St::Idle => self.decide(cx, r),
            _ => {}
        }
    }
}
