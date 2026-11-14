//! Deterministic conversion of 5.x data into `Batch`es (ARCHITECTURE §9.2–§9.3, §10; SYNC §15.3–§15.4).
//!
//! * [`from_pdf`]: a Drive revision (or a user-imported PDF) → import-actor batches. Two devices importing the same
//!   revision produce identical batches.
//! * [`supersede`]: a 5.x revision is a full state written by a device that did not tombstone every deletion; the
//!   batch this returns deletes what an older imported revision had and the newer one dropped, in either import
//!   order (R0 then R1, or R1 then R0, give the same document; R1 wins).
//! * [`from_local`]: unsynced 5.x local data (migration) → migration-actor batches holding only what `base` lacks.
//!
//! Stamps: a 5.x stamp `{clock, actor}` becomes `{clock, ActorId::import("v5:" + actor), 0}` (a register written by a
//! 5.x device keeps one stamp in every revision, so a later v6 edit still wins over an unchanged 5.x value);
//! stampless registers take `{rev_time_ms, import_actor, idx}`, so the newer of two revisions wins.

use super::keywords::{self, OverlayRef};
use super::pages::{self, V5Image, V5Page, V5Stamp, V5Stroke, A4};
use super::sha256::sha256;
use super::structfields::{self, Fields, Structure};
use super::timeline::{self, V5Version};
use super::{legacy_item_id, legacy_page_id, V5Error};
use crate::codec::b64_decode;
use crate::doc::Doc;
use crate::id::{fnv1a64, ActorId, Hash, Id};
use crate::op::{Background, Batch, Body, Color, Op, PageProp, Template, Tool, Z};
use crate::pos::Pos;
use crate::stamp::Stamp;
use std::collections::HashSet;

/// Which bytes a `Background::Pdf` asset of an import holds (ARCHITECTURE §4.5: `SHA-256("ihn6-src" ‖
/// SHA-256(revision bytes) ‖ u32 variant)`, variant little-endian).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// The downloaded revision as is (plain PDFs, legacy-baked 5.x files, or no clean original recoverable).
    Saved = 0,
    /// The revision with the `IH_OVERLAY_REFS` XObjects removed (export worker, pdf-lib).
    Stripped = 1,
    /// The `IH_CLEAN_ORIGINAL.pdf` attachment.
    Attachment = 2,
}

pub fn source_hash(revision_sha256: &Hash, source: Source) -> Hash {
    let mut b = Vec::with_capacity(44);
    b.extend_from_slice(b"ihn6-src");
    b.extend_from_slice(revision_sha256);
    b.extend_from_slice(&(source as u32).to_le_bytes());
    sha256(&b)
}

/// What the caller knows of the PDF besides its Keywords (from pdf.js and `files.get`).
#[derive(Clone, Debug, Default)]
pub struct PdfFacts {
    /// Drive file id (`""` for a user import: ids are then derived from the import actor).
    pub doc_id: String,
    /// The Drive file name without `.pdf`: the document name when no `doc:exportName` field exists.
    pub name: String,
    /// `round(viewport.width/height)` at scale 1 of every PDF page (1..=2000 pages).
    pub page_sizes: Vec<(u32, u32)>,
    /// SHA-256 of the downloaded bytes.
    pub revision_sha256: Hash,
    /// The PDF has an `IH_CLEAN_ORIGINAL.pdf` attachment.
    pub clean_attachment: bool,
    /// Drive, offline and shared opens: invalid STROKES/STRUCT/FIELDS abort (5.18.0 `strictCollabMetadata`).
    pub strict: bool,
}

/// An asset found inside the 5.x data (inline image, photo page background).
#[derive(Clone, Debug, PartialEq)]
pub struct Asset {
    pub hash: Hash,
    pub mime: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Import {
    pub batches: Vec<Batch>,
    pub assets: Vec<Asset>,
    /// `IH_VERSIONS` / stored timeline, normalised; converted to v6 versions by `history` (W2.9/W5.3).
    pub versions: Vec<V5Version>,
    /// The PDF carried 5.x ink (`STROKES_*` decoded); `false` = imported as background.
    pub v5: bool,
    /// Strokes restored (`'PDF imported: N pages, M strokes restored'`).
    pub strokes: usize,
    pub pages: usize,
    /// Pages with a PDF background keep the saved PDF (ink burnt in) and get cover strokes.
    pub legacy_baked: bool,
    /// 5.18.0 `needsMigrationResave`: `STROKES_DATA`, or repaired ids without `IH_NORM`.
    pub needs_resave: bool,
    /// The bytes `Background::Pdf` pages refer to, and their asset hash.
    pub source: Option<(Source, Hash)>,
    /// `IH_SYNC.contentHash` (5.18.0 `driveConfirmedContentHash`).
    pub content_hash: String,
}

// ------------------------------------------------------------------------------------- batch builder

struct Out {
    actor: ActorId,
    clock: u64,
    seq: u32,
    idx: u32,
    ops: Vec<Op>,
    stamps: Vec<Stamp>,
    imp: Import,
    asset_seen: HashSet<Hash>,
    /// 5.x item ids already used by an earlier page of this document.
    item_ids: HashSet<String>,
}

impl Out {
    fn new(actor: ActorId, clock: u64) -> Out { Out { actor, clock, seq: 0, idx: 0, ops: Vec::new(), stamps: Vec::new(), imp: Import::default(), asset_seen: HashSet::new(), item_ids: HashSet::new() } }
    fn own(&mut self) -> Stamp {
        self.idx += 1;
        Stamp::new(self.clock, self.actor, self.idx)
    }
    /// A 5.x stamp, or an own one when there is none (clock 0).
    fn stamp(&mut self, s: &V5Stamp, migration: bool) -> Stamp {
        match s.clock {
            0 => self.own(),
            c if migration => {
                self.idx += 1;
                Stamp::new(c, self.actor, self.idx)
            }
            c => Stamp::new(c, ActorId::import(&format!("v5:{}", s.actor)), 0),
        }
    }
    fn push(&mut self, op: Op, at: Stamp) {
        self.ops.push(op);
        self.stamps.push(at);
    }
    fn flush(&mut self) {
        if !self.ops.is_empty() {
            self.seq += 1;
            let (ops, stamps) = (std::mem::take(&mut self.ops), std::mem::take(&mut self.stamps));
            self.imp.batches.push(Batch { actor: self.actor, seq: self.seq, clock: self.clock, ops, stamps: Some(stamps) });
        }
    }
    /// v6 id of a 5.x item id: `Id::legacy(id)`, or, for an id an earlier page already used, `page:id`.
    fn item(&mut self, page_id: &str, id: &str) -> Id {
        if self.item_ids.insert(id.to_string()) {
            Id::legacy(id)
        } else {
            Id::legacy(&format!("{page_id}:{id}"))
        }
    }
    fn asset(&mut self, data_url: &str) -> Option<Hash> {
        let (mime, bytes) = data_url_bytes(data_url)?;
        let hash = sha256(&bytes);
        if self.asset_seen.insert(hash) {
            self.imp.assets.push(Asset { hash, mime, bytes });
        }
        Some(hash)
    }
}

/// `data:<mime>;base64,<data>` → bytes. Other URLs (`blob:` of a closed 5.x session) cannot be read.
fn data_url_bytes(url: &str) -> Option<(String, Vec<u8>)> {
    let rest = url.strip_prefix("data:")?;
    let (head, data) = rest.split_once(',')?;
    let mime = head.strip_suffix(";base64")?;
    Some((mime.to_string(), b64_decode(data.trim().as_bytes())?))
}

fn stroke_body(s: &V5Stroke, cover: bool) -> Option<Body> {
    let tool = match s.tool.as_str() {
        _ if cover => Tool::LegacyCover,
        // Never drawn by 5.18.0.
        "eraser-stroke" => return None,
        "highlighter" => Tool::Highlighter,
        "eraser-area" => Tool::LegacyEraseArea,
        _ => Tool::Pen,
    };
    Some(Body::Stroke {
        tool,
        color: Color::parse(&s.color),
        width: if s.width.is_finite() { s.width } else { 2.0 },
        xy: s.xy.clone(),
        p: (!s.p.is_empty()).then(|| s.p.iter().map(|&v| v as f32).collect()),
    })
}

fn image_body(out: &mut Out, i: &V5Image) -> Option<Body> {
    let asset = out.asset(&i.src)?;
    let f = |v: f64| if v.is_finite() { v } else { 0.0 };
    Some(Body::Image { asset, cx: f(i.x), cy: f(i.y), w: f(i.width), h: f(i.height), rotation: f(i.rotation) })
}

/// The 5.x string ids of a page's strokes and images, as 5.18.0 ends up with them: without `IH_NORM`, a repeated
/// stroke id is re-issued (5.18.0: random; here `id~pageId~index`) and identical strokes are dropped; then missing
/// ids get the legacy ids (`st_legacy_`/`im_legacy_`).
fn normalise_ids(page: &mut V5Page, doc_key: &str, normalized: bool) -> bool {
    let mut repaired = false;
    if !normalized && page.strokes.len() > 1 {
        let mut seen = HashSet::new();
        for (i, s) in page.strokes.iter_mut().enumerate() {
            if !s.id.is_empty() && !seen.insert(s.id.clone()) {
                s.id = format!("{}~{}~{i}", s.id, page.page_id);
                repaired = true;
            }
        }
        let before = page.strokes.len();
        let mut kept: Vec<V5Stroke> = Vec::with_capacity(before);
        for s in page.strokes.drain(..) {
            let same = |o: &V5Stroke| o.id.is_empty() && o.tool == s.tool && o.color == s.color && o.width.to_bits() == s.width.to_bits() && o.xy == s.xy && o.p.len() == s.p.len() && o.p.iter().zip(&s.p).all(|(a, b)| a.to_bits() == b.to_bits());
            if !(s.id.is_empty() && kept.iter().any(same)) {
                kept.push(s);
            }
        }
        repaired |= kept.len() < before;
        page.strokes = kept;
    }
    let pid = page.page_id.clone();
    for (i, s) in page.strokes.iter_mut().enumerate().filter(|(_, s)| s.id.is_empty()) {
        s.id = legacy_item_id(doc_key, &pid, false, i).unwrap_or_default();
    }
    for (i, im) in page.images.iter_mut().enumerate().filter(|(_, im)| im.id.is_empty()) {
        im.id = legacy_item_id(doc_key, &pid, true, i).unwrap_or_default();
    }
    repaired
}

/// Item ops of one page: strokes (and their covers), images, tombstones.
fn page_items(out: &mut Out, page: Id, p: &V5Page, covers: bool, migration: bool) {
    let mut live = HashSet::new();
    let mut k = 0u32;
    let mut z_of = |out: &mut Out, at: Stamp| {
        k += 1;
        // Stampless items keep their array order; 5.x-stamped ones their stamp (one value per stamp).
        if at.actor == out.actor && at.clock == out.clock { Z { at: Stamp::new(out.clock, out.actor, k), sub: 0 } } else { Z { at, sub: 0 } }
    };
    for s in &p.strokes {
        let Some(body) = stroke_body(s, false) else { continue };
        live.insert(s.id.as_str());
        let item = out.item(&p.page_id, &s.id);
        let at = out.stamp(&s.stamp, migration);
        let z = z_of(out, at);
        if covers {
            let cover = Id::legacy(&format!("cover:{}", s.id));
            let cbody = stroke_body(s, true).unwrap_or(Body::Dropped);
            out.push(Op::ItemPut { item: cover, page, z, body: cbody }, at);
        }
        out.push(Op::ItemPut { item, page, z, body }, at);
    }
    for i in &p.images {
        let Some(body) = image_body(out, i) else { continue };
        live.insert(i.id.as_str());
        let item = out.item(&p.page_id, &i.id);
        let at = out.stamp(&i.stamp, migration);
        let z = z_of(out, at);
        out.push(Op::ItemPut { item, page, z, body }, at);
    }
    for (id, at) in &p.deleted {
        if !live.contains(id.as_str()) {
            let at = out.stamp(at, migration);
            out.push(Op::ItemDel { item: Id::legacy(id), deleted: true }, at);
        }
    }
}

fn template(kind: &str) -> Template {
    match kind {
        "agenda" => Template::Agenda,
        "diary" => Template::Diary,
        _ => Template::Default,
    }
}

fn side_panel(p: &V5Page) -> Option<Vec<u8>> { p.side_panel.as_ref().map(|s| s.as_bytes().to_vec()) }

// ------------------------------------------------------------------------------------- from_pdf

/// A Drive revision or an imported PDF → batches (one per page in page order, then one for document fields).
/// `Err` only in strict mode (or for the size limits that 5.18.0 enforces before parsing).
pub fn from_pdf(kw: &str, facts: &PdfFacts, import_actor: ActorId, rev_time_ms: u64) -> Result<Import, V5Error> {
    super::check_page_count(facts.page_sizes.len() as u32)?;
    let k = keywords::parse(kw);
    let mut out = Out::new(import_actor, rev_time_ms);
    let doc_key = if facts.doc_id.trim().is_empty() { format!("import:{:016x}", import_actor.0) } else { facts.doc_id.clone() };
    macro_rules! strict {
        ($e:expr) => {
            if facts.strict {
                return Err($e);
            } else {
                None
            }
        };
    }

    // `STROKES_*` → page entries (5.18.0 catches every metadata error unless strict).
    let entries = match k.strokes {
        Some((z, payload)) => match keywords::strokes_json(z, payload).and_then(|t| super::json::parse(&t).ok_or(V5Error::Json)) {
            Ok(v) => match pages::entries(&v) {
                Ok(Some(e)) => Some(e),
                Ok(None) => strict!(V5Error::StrokesInvalid),
                Err(e) => strict!(e),
            },
            Err(e) => strict!(e),
        },
        None => None,
    };
    // A failed `STROKES_*` (not strict) skips the rest of 5.18.0's metadata block.
    let skip = k.strokes.is_some() && entries.is_none();
    let structure = match keywords::structure_value(&k) {
        _ if skip => None,
        Ok(v) => v,
        Err(e) => strict!(e),
    };
    let fields = match keywords::fields_value(&k) {
        _ if skip => None,
        Ok(v) => v.map(|f| structfields::fields(&f)),
        Err(e) => strict!(e),
    };
    out.imp.needs_resave = matches!(k.strokes, Some((false, _))) && entries.is_some();
    out.imp.content_hash = keywords::sync_envelope(&k).map(|s| s.content_hash).unwrap_or_default();
    out.imp.v5 = entries.is_some();
    out.imp.pages = facts.page_sizes.len();

    let mut entries = entries.unwrap_or_default();
    for (i, e) in entries.iter_mut().enumerate().take(facts.page_sizes.len()) {
        if let Some(p) = e {
            if p.page_id.is_empty() {
                p.page_id = legacy_page_id(&doc_key, i as u32 + 1).unwrap_or_default();
            }
            out.imp.needs_resave |= normalise_ids(p, &doc_key, k.normalized);
        }
    }
    let needs_pdf = |p: &V5Page| !p.has_images_key || !matches!(if p.background_source.is_empty() { if p.has_custom_background { "pdf" } else { "template" } } else { p.background_source.as_str() }, "template");
    let present = || entries.iter().take(facts.page_sizes.len()).flatten();
    out.imp.legacy_baked = present().any(|p| p.needs_pdf_background() && !p.has_drawable_vectors());
    let any_pdf = (0..facts.page_sizes.len()).any(|i| entries.get(i).and_then(Option::as_ref).is_none_or(needs_pdf));
    let source = match () {
        _ if !out.imp.v5 || out.imp.legacy_baked => Source::Saved,
        _ if !k.overlay_refs.is_empty() => Source::Stripped,
        _ if facts.clean_attachment => Source::Attachment,
        _ => Source::Saved,
    };
    let asset = source_hash(&facts.revision_sha256, source);
    if any_pdf {
        out.imp.source = Some((source, asset));
    }

    let mut ids = Vec::with_capacity(facts.page_sizes.len());
    for (i, &pdf_size) in facts.page_sizes.iter().enumerate() {
        let n = i as u32 + 1;
        let entry = entries.get(i).and_then(Option::as_ref);
        let (page_id, tpl, size, pdf) = match entry {
            Some(p) => {
                let pdf = needs_pdf(p);
                let fallback = if pdf { pdf_size } else { A4 };
                (p.page_id.clone(), template(p.template_kind), (pages::dimension(p.page_width, fallback.0), pages::dimension(p.page_height, fallback.1)), pdf)
            }
            None => (legacy_page_id(&doc_key, n).unwrap_or_default(), Template::Default, pdf_size, true),
        };
        let page = Id::legacy(&page_id);
        ids.push(page_id);
        let background = if pdf { Background::Pdf { asset, page: n } } else { Background::Template };
        let at = out.own();
        out.push(Op::PageNew { page, pos: Pos::rank(i as u32), template: tpl, size: (size.0.max(64), size.1.max(64)), background }, at);
        if let Some(p) = entry {
            if let Some(panel) = side_panel(p) {
                let at = out.own();
                out.push(Op::PageSet { page, prop: PageProp::SidePanel(Some(panel)) }, at);
            }
            out.imp.strokes += p.strokes.len();
            let covers = out.imp.legacy_baked && pdf;
            page_items(&mut out, page, p, covers, false);
        }
        out.flush();
    }

    // Document fields: name, pages tombstoned in `IH_STRUCT` that no entry shows.
    let id_refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    if let Some(s) = structure.map(|v| structfields::structure(&v, &id_refs)) {
        for (id, t) in &s.tombstones {
            if !ids.contains(id) && s.is_tombstoned(id) {
                let at = out.stamp(t, false);
                out.push(Op::PageDel { page: Id::legacy(id), deleted: true }, at);
            }
        }
    }
    let at;
    let name = match fields.as_ref().and_then(Fields::export_name) {
        Some((n, s)) => {
            at = out.stamp(s, false);
            n.to_string()
        }
        None => {
            at = out.own();
            facts.name.clone()
        }
    };
    out.push(Op::DocName { name }, at);
    out.flush();

    if let Some(v) = k.versions {
        // A damaged timeline leaves it empty, as in 5.18.0.
        if let Ok(text) = keywords::b64_zlib(v) {
            out.imp.versions = timeline::read_json(&text, rev_time_ms as f64);
        }
    }
    Ok(out.imp)
}

/// Pages of a v5 PDF whose overlay XObjects `Source::Stripped` removes (0-based page index → reference).
pub fn overlay_refs(kw: &str) -> Vec<(i64, OverlayRef<'_>)> { keywords::parse(kw).overlay_refs }

// ------------------------------------------------------------------------------------- supersede

fn low32(id: Id) -> u32 { (id.0 as u32) ^ ((id.0 >> 64) as u32) }

/// After importing a revision (`rev`, the batches of [`from_pdf`]) into `doc` (before or after applying them): the
/// deletions that 5.x expressed by leaving items or pages out instead of tombstoning them. `keep` = actors whose
/// registers no revision may delete (this installation's migration actors).
///
/// * `rev` is the newest imported revision: live items/pages whose newest write came from 5.x data (an import-actor
///   stamp not in `keep`) older than `rev` and that `rev` does not contain are deleted at `{rev_time, rev_actor}`.
/// * an older revision arrives after a newer one: what only the older one contains is deleted at the newer
///   revision's stamp, exactly as importing in the other order would have.
///
/// The batch is deterministic in the resulting state; its actor is derived from its content (the same deletions
/// from two devices are the same batch).
pub fn supersede(doc: &Doc, rev: &[Batch], keep: &[ActorId]) -> Option<Batch> {
    let first = rev.first()?;
    let me = Stamp::new(first.clock, first.actor, 0);
    let from_v5 = |s: Stamp| s.actor.is_import() && !keep.contains(&s.actor) && s.clock != 0;
    let (mut items, mut pages) = (HashSet::new(), HashSet::new());
    for op in rev.iter().flat_map(|b| &b.ops) {
        match op {
            Op::ItemPut { item, .. } => drop(items.insert(*item)),
            Op::PageNew { page, .. } => drop(pages.insert(*page)),
            _ => {}
        }
    }
    // Newest revision the document holds: page positions are always written with the revision's own stamp.
    let newest = doc.pages.values().map(|p| p.pos.at).filter(|&s| from_v5(s)).max().unwrap_or(Stamp::ZERO);
    let key = |s: Stamp| (s.clock, s.actor);
    let mut ops: Vec<(Op, Stamp)> = Vec::new();
    if key(newest) <= key(me) {
        for (&id, it) in doc.items.iter() {
            if it.is_live() && !items.contains(&id) && from_v5(it.body.at) && key(it.body.at) < key(me) {
                ops.push((Op::ItemDel { item: id, deleted: true }, Stamp::new(me.clock, me.actor, low32(id))));
            }
        }
        for (&id, p) in doc.pages.iter() {
            if p.is_born() && !p.deleted.v && !pages.contains(&id) && from_v5(p.pos.at) && key(p.pos.at) < key(me) {
                ops.push((Op::PageDel { page: id, deleted: true }, Stamp::new(me.clock, me.actor, low32(id))));
            }
        }
    } else {
        for &id in &items {
            if !doc.items.contains_key(&id) {
                ops.push((Op::ItemDel { item: id, deleted: true }, Stamp::new(newest.clock, newest.actor, low32(id))));
            }
        }
        for &id in &pages {
            if !doc.pages.get(&id).is_some_and(|p| p.is_born()) {
                ops.push((Op::PageDel { page: id, deleted: true }, Stamp::new(newest.clock, newest.actor, low32(id))));
            }
        }
    }
    if ops.is_empty() {
        return None;
    }
    ops.sort_by_key(|(op, s)| (s.idx, matches!(op, Op::PageDel { .. })));
    let digest = ops.iter().fold(fnv1a64(b"supersede"), |h, (op, s)| {
        let id = match op {
            Op::ItemDel { item, .. } => item.0,
            Op::PageDel { page, .. } => page.0 ^ 1,
            _ => 0,
        };
        fnv1a64(&[h.to_le_bytes(), (id as u64).to_le_bytes(), ((id >> 64) as u64).to_le_bytes(), s.clock.to_le_bytes(), s.actor.0.to_le_bytes()].concat())
    });
    let actor = ActorId::import(&format!("supersede:{digest:016x}"));
    let (ops, stamps): (Vec<Op>, Vec<Stamp>) = ops.into_iter().unzip();
    let clock = stamps.iter().map(|s| s.clock).max().unwrap_or(0);
    Some(Batch { actor, seq: 1, clock, ops, stamps: Some(stamps) })
}

// ------------------------------------------------------------------------------------- from_local

/// One source of unsynced 5.x local data for one document (ARCHITECTURE §10.1), already decoded by `v5::idb`.
#[derive(Clone, Debug, Default)]
pub struct V5LocalDoc {
    /// Drive file id or `local-<uuid>` (also the key of 5.x legacy ids).
    pub doc_id: String,
    /// The working copy in document order (`idb::working_copy`, or an offline snapshot's pages).
    pub pages: Vec<V5Page>,
    pub structure: Structure,
    pub fields: Fields,
    /// Fallback when `fields` has no `doc:exportName`.
    pub export_name: String,
    /// `savedAt` / `updatedAt` of the record: the stamp clock of stampless local data.
    pub saved_at: f64,
    /// `modifiedTime` of the Drive revision the local copy was based on (0 = unknown): base items no newer than
    /// this that the local copy lacks were deleted locally.
    pub base_ms: f64,
    /// Asset of the document's source PDF (`originalBlob`), for new local pages with a PDF background.
    pub source: Option<Hash>,
    pub versions: Vec<V5Version>,
}

impl V5LocalDoc {
    /// From an offline `documents` row (or a recovery's snapshot).
    pub fn from_snapshot(doc_id: &str, s: &super::idb::Snapshot, saved_at: f64, base_ms: f64, source: Option<Hash>) -> V5LocalDoc {
        V5LocalDoc { doc_id: doc_id.into(), pages: s.pages.clone(), structure: s.structure.clone(), fields: s.fields.clone(), export_name: s.export_name.clone(), saved_at, base_ms, source, versions: s.versions.clone() }
    }
    /// From the working cache (`snapshots/latest` + `pages` + `stroke-ops`, already merged by `idb::working_copy`).
    pub fn from_working(doc_id: &str, meta: &super::idb::MetaPayload, pages: Vec<V5Page>, export_name: &str) -> V5LocalDoc {
        let ids: Vec<&str> = pages.iter().map(|p| p.page_id.as_str()).collect();
        let structure = structfields::structure(&meta.structure, &ids);
        V5LocalDoc { doc_id: doc_id.into(), structure, fields: structfields::fields(&meta.fields), export_name: export_name.into(), saved_at: meta.saved_at, versions: meta.versions.clone(), pages, ..Default::default() }
    }
}

/// Unsynced 5.x local data → batches of `migration_actor` holding what `base` (the document built from the Drive
/// base, if any) does not already have: new pages and items, changed bodies and page properties, local deletions,
/// the name. Stamps: `{5.x clock or savedAt, migration_actor, idx}`.
pub fn from_local(records: &V5LocalDoc, migration_actor: ActorId, base: Option<&Doc>) -> Result<Import, V5Error> {
    let empty = Doc::default();
    let base = base.unwrap_or(&empty);
    let saved = if records.saved_at.is_finite() && records.saved_at > 0.0 { records.saved_at as u64 } else { 1 };
    let mut out = Out::new(migration_actor, saved);
    // Several sources (working cache, offline snapshot) and resumed runs continue the actor's seq and stamp indexes.
    out.seq = base.vv.get(migration_actor);
    let mine = |s: Stamp| if s.actor == migration_actor { s.idx } else { 0 };
    out.idx = base.items.values().map(|i| mine(i.body.at).max(mine(i.deleted.at))).chain(base.pages.values().map(|p| mine(p.pos.at).max(mine(p.size.at)).max(mine(p.deleted.at)))).chain([mine(base.name.at)]).max().unwrap_or(0);
    out.imp.v5 = true;
    out.imp.versions = records.versions.clone();
    let doc_key = records.doc_id.trim();
    let base_pdf = base.pages.values().find_map(|p| match p.background.v {
        Background::Pdf { asset, .. } => Some(asset),
        _ => None,
    });

    let mut local: Vec<(Id, V5Page)> = Vec::with_capacity(records.pages.len());
    for (i, p) in records.pages.iter().enumerate() {
        let mut p = p.clone();
        if p.page_id.is_empty() {
            p.page_id = legacy_page_id(doc_key, i as u32 + 1).unwrap_or_else(|| format!("pg_local_{i}"));
        }
        normalise_ids(&mut p, doc_key, true);
        local.push((Id::legacy(&p.page_id), p));
    }
    // Order: keep base positions when the shared pages keep their relative order, else re-key every local page.
    let born = |id: &Id| base.pages.get(id).filter(|p| p.is_born());
    let shared: Vec<Id> = local.iter().map(|(id, _)| *id).filter(|id| born(id).is_some()).collect();
    let in_base: Vec<Id> = base.order().iter().copied().filter(|id| shared.contains(id)).collect();
    let rekey = shared != in_base;
    let next_pos = |from: usize| local[from..].iter().find_map(|(id, _)| born(id).map(|p| p.pos.v.clone()));

    let mut prev: Option<Pos> = None;
    for (i, (page, p)) in local.iter().enumerate() {
        let page = *page;
        out.imp.strokes += p.strokes.len();
        out.imp.pages += 1;
        let pdf = p.source() != "template";
        let background = match p.source() {
            "template" => Background::Template,
            "custom" => p.background_image.as_deref().and_then(|u| out.asset(u)).map_or(Background::Template, |asset| Background::Custom { asset }),
            _ => match records.source.or(base_pdf) {
                Some(asset) => Background::Pdf { asset, page: p.pdf_page_index.filter(|v| *v >= 1.0).map_or(i as u32 + 1, |v| v as u32) },
                None => Background::Template,
            },
        };
        let size = p.size();
        let size = (size.0.max(64), size.1.max(64));
        let tpl = template(p.template_kind);
        match born(&page) {
            Some(b) => {
                let pos = if rekey { Pos::rank(i as u32) } else { b.pos.v.clone() };
                if pos != b.pos.v {
                    let at = out.own();
                    out.push(Op::PageMove { page, pos: pos.clone() }, at);
                }
                if b.deleted.v {
                    let at = out.own();
                    out.push(Op::PageDel { page, deleted: false }, at);
                }
                let set = |out: &mut Out, prop: PageProp| {
                    let at = out.own();
                    out.push(Op::PageSet { page, prop }, at);
                };
                if b.template.v != tpl {
                    set(&mut out, PageProp::Template(tpl));
                }
                if b.size.v != size {
                    set(&mut out, PageProp::Size(size));
                }
                // A PDF page whose source is unknown here keeps the base's background.
                let known = !(pdf && matches!(background, Background::Template));
                if known && std::mem::discriminant(&b.background.v) != std::mem::discriminant(&background) {
                    set(&mut out, PageProp::Background(background));
                }
                if b.side_panel.v != side_panel(p) {
                    set(&mut out, PageProp::SidePanel(side_panel(p)));
                }
                prev = Some(pos);
            }
            None => {
                let pos = if rekey { Pos::rank(i as u32) } else { Pos::between(prev.as_ref(), next_pos(i + 1).as_ref()) };
                let at = out.own();
                out.push(Op::PageNew { page, pos: pos.clone(), template: tpl, size, background }, at);
                if let Some(panel) = side_panel(p) {
                    let at = out.own();
                    out.push(Op::PageSet { page, prop: PageProp::SidePanel(Some(panel)) }, at);
                }
                prev = Some(pos);
            }
        }
        local_items(&mut out, base, page, p, records.base_ms);
        out.flush();
    }

    for (id, t) in &records.structure.tombstones {
        let page = Id::legacy(id);
        if base.pages.get(&page).is_some_and(|p| p.is_born() && !p.deleted.v) && !local.iter().any(|(l, _)| *l == page) && records.structure.is_tombstoned(id) {
            let at = out.stamp(t, true);
            out.push(Op::PageDel { page, deleted: true }, at);
        }
    }
    let (name, stamp) = match records.fields.export_name() {
        Some((n, s)) => (n.to_string(), s.clone()),
        None => (records.export_name.clone(), V5Stamp::default()),
    };
    if !name.is_empty() && name != base.name.v {
        let at = out.stamp(&stamp, true);
        out.push(Op::DocName { name }, at);
    }
    out.flush();
    Ok(out.imp)
}

/// Items of one local page against `base`.
fn local_items(out: &mut Out, base: &Doc, page: Id, p: &V5Page, base_ms: f64) {
    let mut sub = Out::new(out.actor, out.clock);
    std::mem::swap(&mut sub.item_ids, &mut out.item_ids);
    std::mem::swap(&mut sub.asset_seen, &mut out.asset_seen);
    sub.idx = out.idx;
    page_items(&mut sub, page, p, false, true);
    if let Some(c) = &p.legacy_cover_strokes {
        for s in c {
            let at = sub.stamp(&s.stamp, true);
            if let Some(body) = stroke_body(s, true) {
                sub.push(Op::ItemPut { item: Id::legacy(&format!("cover:{}", s.id)), page, z: Z { at, sub: 0 }, body }, at);
            }
        }
    }
    out.idx = sub.idx;
    std::mem::swap(&mut sub.item_ids, &mut out.item_ids);
    std::mem::swap(&mut sub.asset_seen, &mut out.asset_seen);
    out.imp.assets.append(&mut sub.imp.assets);

    let mut present = HashSet::new();
    for (op, at) in sub.ops.into_iter().zip(sub.stamps) {
        match op {
            Op::ItemPut { item, page, z, body } => {
                present.insert(item);
                match base.items.get(&item) {
                    // Unchanged, or already overwritten by a later write: nothing to migrate.
                    Some(b) if (b.page.v == page && b.body.v == body) || b.body.at > at => {}
                    // Changed: keep the base's draw position.
                    Some(b) if !b.body.v.is_dropped() => out.push(Op::ItemPut { item, page, z: b.z.v, body }, at),
                    _ => out.push(Op::ItemPut { item, page, z, body }, at),
                }
            }
            Op::ItemDel { item, deleted } => {
                if base.live(item).is_some() {
                    out.push(Op::ItemDel { item, deleted }, at);
                }
            }
            _ => {}
        }
    }
    // Base items of this page that the local copy dropped although it had seen them.
    if base_ms > 0.0 {
        let mut gone: Vec<Id> = base
            .page_items(page)
            .into_iter()
            .filter(|id| !present.contains(id))
            .filter(|id| base.items.get(id).is_some_and(|it| !matches!(it.body.v, Body::Stroke { tool: Tool::LegacyCover, .. }) && (it.body.at.clock as f64) <= base_ms))
            .collect();
        gone.sort_unstable();
        for item in gone {
            let at = out.own();
            out.push(Op::ItemDel { item, deleted: true }, at);
        }
    }
}

#[cfg(test)]
#[path = "import_tests.rs"]
mod tests;
