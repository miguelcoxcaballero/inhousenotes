//! Batch/op codec (SYNC §4.1–4.3, exact) and the snapshot codec (doc header + per-page chunks, SYNC §4.4).
//!
//! ```text
//! Snapshot := u8 ver=1, bytes Header, uv n, (bytes Chunk) × n            // chunks ascending by page id
//! Header   := u8 ver=1, VV, Extras, str name, Stamp, uv n, PageRec × n, uv n, VersionRec × n   // ascending ids
//! PageRec  := Id, bytes pos, Stamp, u8 deleted, Stamp, u8 template, Stamp, uv w, uv h, Stamp, Background, Stamp,
//!             u8 present, [bytes side_panel], Stamp
//! VersionRec := Id, bytes data, Stamp, u8 deleted, Stamp
//! Chunk    := u8 ver=1, Id page, uv n, ItemRec × n                        // items whose page register = page
//! ItemRec  := Id, u8 flags, Stamp body_at, [Stamp page_at], [Z], [Stamp z_at], [Stamp deleted_at], [Body]
//!   flags: 1 page_at = body_at · 2 z_at = body_at · 4 z = {body_at, 0} · 8 deleted · 16 deleted_at present · 32 body
//! ```
//! The encoding is canonical: converged replicas produce identical bytes whatever their history. The body of an
//! item deleted after its last body write is omitted (`Item::body_dropped_in_snapshot`). Decoders never panic and
//! reject truncation, overflow, trailing bytes and limit violations as a whole.

use crate::codec::{Reader, Writer};
use crate::doc::{Doc, Item, Page, Version};
use crate::id::{ActorId, Hash, Id};
use crate::op::{Background, Batch, Body, Color, Op, PageProp, Template, Tool, VersionMeta, MIN_PAGE_SIDE, Z};
use crate::pos::Pos;
use crate::stamp::{Reg, Stamp, VersionVector};

pub const CODEC_VERSION: u8 = 1;
pub const MAX_BATCH_OPS: u64 = 1_000_000;
pub const MAX_BATCH_BYTES: usize = 32 << 20;

/// A batch in its SYNC §4.2 encoding: what the journal stores and every transport carries.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EncodedBatch(pub Vec<u8>);

/// A snapshot in the encoding above; sync transports it opaque. It always carries its VV.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EncodedSnapshot(pub Vec<u8>);

/// Undecodable input (truncated, corrupt, over a limit, unknown version or tag).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Corrupt;

// ------------------------------------------------------------------------------------------------ primitives

pub fn put_bytes(w: &mut Writer, v: &[u8]) {
    w.leb(v.len() as u64);
    w.raw(v);
}
pub fn get_bytes<'a>(r: &mut Reader<'a>) -> Option<&'a [u8]> {
    let n = usize::try_from(r.leb()?).ok()?;
    r.take(n)
}
pub fn put_str(w: &mut Writer, v: &str) { put_bytes(w, v.as_bytes()) }
pub fn get_str<'a>(r: &mut Reader<'a>) -> Option<&'a str> { std::str::from_utf8(get_bytes(r)?).ok() }
pub fn put_id(w: &mut Writer, v: Id) { w.raw(&v.0.to_le_bytes()) }
pub fn get_id(r: &mut Reader) -> Option<Id> { Some(Id(u128::from_le_bytes(r.take(16)?.try_into().ok()?))) }
pub fn put_hash(w: &mut Writer, v: &Hash) { w.raw(v) }
pub fn get_hash(r: &mut Reader) -> Option<Hash> { r.take(32)?.try_into().ok() }
fn get_u32(r: &mut Reader) -> Option<u32> { u32::try_from(r.leb()?).ok() }
fn get_flag(r: &mut Reader) -> Option<bool> {
    match r.u8()? {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    }
}

pub fn put_stamp(w: &mut Writer, s: Stamp) {
    w.leb(s.clock);
    w.u64(s.actor.0);
    w.leb(s.idx as u64);
}
pub fn get_stamp(r: &mut Reader) -> Option<Stamp> { Some(Stamp { clock: r.leb()?, actor: ActorId(r.u64()?), idx: get_u32(r)? }) }

pub fn put_z(w: &mut Writer, z: Z) {
    put_stamp(w, z.at);
    w.leb(z.sub as u64);
}
pub fn get_z(r: &mut Reader) -> Option<Z> { Some(Z { at: get_stamp(r)?, sub: get_u32(r)? }) }

pub fn put_color(w: &mut Writer, c: &Color) {
    match c {
        Color::Hex(v) => {
            w.u8(0);
            w.u32(*v);
        }
        Color::Css(s) => {
            w.u8(1);
            put_str(w, s);
        }
    }
}
pub fn get_color(r: &mut Reader) -> Option<Color> {
    match r.u8()? {
        0 => Some(Color::Hex(r.u32()?)),
        1 => Some(Color::Css(get_str(r)?.into())),
        _ => None,
    }
}

fn put_background(w: &mut Writer, b: &Background) {
    match b {
        Background::Template => w.u8(0),
        Background::Pdf { asset, page } => {
            w.u8(1);
            put_hash(w, asset);
            w.leb(*page as u64);
        }
        Background::Custom { asset } => {
            w.u8(2);
            put_hash(w, asset);
        }
    }
}
fn get_background(r: &mut Reader) -> Option<Background> {
    match r.u8()? {
        0 => Some(Background::Template),
        1 => Some(Background::Pdf { asset: get_hash(r)?, page: get_u32(r).filter(|&p| p >= 1)? }),
        2 => Some(Background::Custom { asset: get_hash(r)? }),
        _ => None,
    }
}

fn put_template(w: &mut Writer, t: Template) { w.u8(t as u8) }
fn get_template(r: &mut Reader) -> Option<Template> {
    match r.u8()? {
        0 => Some(Template::Default),
        1 => Some(Template::Agenda),
        2 => Some(Template::Diary),
        _ => None,
    }
}

fn get_size(r: &mut Reader) -> Option<(u32, u32)> { Some((get_u32(r)?, get_u32(r)?)) }
fn valid_size(s: (u32, u32)) -> bool { s.0 >= MIN_PAGE_SIDE && s.1 >= MIN_PAGE_SIDE }
fn get_pos(r: &mut Reader) -> Option<Pos> { Some(Pos(get_bytes(r)?.to_vec())) }

/// SYNC §4.3 `Body`. `Body::Dropped` has no encoding in ops (tag 2 is written and rejected by the decoder).
pub fn put_body(w: &mut Writer, b: &Body) {
    match b {
        Body::Stroke { tool, color, width, xy, p } => {
            w.u8(0);
            w.u8(*tool as u8);
            put_color(w, color);
            w.f64(*width);
            let n = xy.len() / 2;
            w.leb(n as u64);
            w.buf.reserve(n * 16);
            for v in &xy[..n * 2] {
                w.f64(*v);
            }
            // Pressure is kept only when there is one value per point.
            match p.as_ref().filter(|p| p.len() == n) {
                Some(p) => {
                    w.u8(1);
                    p.iter().for_each(|v| w.f32(*v));
                }
                None => w.u8(0),
            }
        }
        Body::Image { asset, cx, cy, w: width, h, rotation } => {
            w.u8(1);
            put_hash(w, asset);
            [cx, cy, width, h, rotation].into_iter().for_each(|v| w.f64(*v));
        }
        Body::Dropped => w.u8(2),
    }
}
pub fn get_body(r: &mut Reader) -> Option<Body> {
    match r.u8()? {
        0 => {
            let tool = match r.u8()? {
                0 => Tool::Pen,
                1 => Tool::Highlighter,
                2 => Tool::LegacyEraseArea,
                3 => Tool::LegacyCover,
                _ => return None,
            };
            let (color, width) = (get_color(r)?, r.f64()?);
            let n = usize::try_from(r.leb()?).ok().filter(|&n| n <= r.buf.len() / 16)?;
            let xy = r.take(n * 16)?.chunks_exact(8).map(|c| f64::from_le_bytes(c.try_into().unwrap_or_default())).collect();
            let p = match get_flag(r)? {
                true => Some(r.take(n * 4)?.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap_or_default())).collect()),
                false => None,
            };
            Some(Body::Stroke { tool, color, width, xy, p })
        }
        1 => Some(Body::Image { asset: get_hash(r)?, cx: r.f64()?, cy: r.f64()?, w: r.f64()?, h: r.f64()?, rotation: r.f64()? }),
        _ => None,
    }
}

// ------------------------------------------------------------------------------------------------ ops, batches

pub fn put_op(w: &mut Writer, op: &Op) {
    match op {
        Op::PageNew { page, pos, template, size, background } => {
            w.u8(0x01);
            put_id(w, *page);
            put_bytes(w, &pos.0);
            put_template(w, *template);
            w.leb(size.0 as u64);
            w.leb(size.1 as u64);
            put_background(w, background);
        }
        Op::PageMove { page, pos } => {
            w.u8(0x02);
            put_id(w, *page);
            put_bytes(w, &pos.0);
        }
        Op::PageDel { page, deleted } => {
            w.u8(0x03);
            put_id(w, *page);
            w.u8(*deleted as u8);
        }
        Op::PageSet { page, prop } => {
            w.u8(0x04);
            put_id(w, *page);
            match prop {
                PageProp::Template(t) => {
                    w.u8(0);
                    put_template(w, *t);
                }
                PageProp::Size(s) => {
                    w.u8(1);
                    w.leb(s.0 as u64);
                    w.leb(s.1 as u64);
                }
                PageProp::Background(b) => {
                    w.u8(2);
                    put_background(w, b);
                }
                PageProp::SidePanel(v) => {
                    w.u8(3);
                    w.u8(v.is_some() as u8);
                    if let Some(v) = v {
                        put_bytes(w, v);
                    }
                }
            }
        }
        Op::ItemPut { item, page, z, body } => {
            w.u8(0x05);
            put_id(w, *item);
            put_id(w, *page);
            put_z(w, *z);
            put_body(w, body);
        }
        Op::ItemDel { item, deleted } => {
            w.u8(0x06);
            put_id(w, *item);
            w.u8(*deleted as u8);
        }
        Op::DocName { name } => {
            w.u8(0x07);
            put_str(w, name);
        }
        Op::VersionAdd { meta } => {
            // `bytes meta` = Id ‖ the VersionMeta record of ihn-core::history.
            w.u8(0x08);
            w.leb(16 + meta.data.len() as u64);
            put_id(w, meta.id);
            w.raw(&meta.data);
        }
        Op::VersionDel { version } => {
            w.u8(0x09);
            put_id(w, *version);
        }
    }
}

pub fn get_op(r: &mut Reader) -> Option<Op> {
    Some(match r.u8()? {
        0x01 => Op::PageNew {
            page: get_id(r)?,
            pos: get_pos(r).filter(Pos::is_valid)?,
            template: get_template(r)?,
            size: get_size(r).filter(|s| valid_size(*s))?,
            background: get_background(r)?,
        },
        0x02 => Op::PageMove { page: get_id(r)?, pos: get_pos(r).filter(Pos::is_valid)? },
        0x03 => Op::PageDel { page: get_id(r)?, deleted: get_flag(r)? },
        0x04 => Op::PageSet {
            page: get_id(r)?,
            prop: match r.u8()? {
                0 => PageProp::Template(get_template(r)?),
                1 => PageProp::Size(get_size(r).filter(|s| valid_size(*s))?),
                2 => PageProp::Background(get_background(r)?),
                3 => PageProp::SidePanel(match get_flag(r)? {
                    true => Some(get_bytes(r)?.to_vec()),
                    false => None,
                }),
                _ => return None,
            },
        },
        0x05 => Op::ItemPut { item: get_id(r)?, page: get_id(r)?, z: get_z(r)?, body: get_body(r)? },
        0x06 => Op::ItemDel { item: get_id(r)?, deleted: get_flag(r)? },
        0x07 => Op::DocName { name: get_str(r)?.to_string() },
        0x08 => {
            let mut meta = Reader::new(get_bytes(r)?);
            Op::VersionAdd { meta: VersionMeta { id: get_id(&mut meta)?, data: meta.buf.to_vec() } }
        }
        0x09 => Op::VersionDel { version: get_id(r)? },
        _ => return None, // all devices run the same release
    })
}

/// `Batch := u8 ver=1, u64 actor, uv seq, uv clock, uv n_ops, ([Stamp]? Op) × n_ops`; the stamp is present iff
/// the actor is an import actor.
pub fn encode_batch(b: &Batch) -> EncodedBatch {
    let mut w = Writer::default();
    w.u8(CODEC_VERSION);
    w.u64(b.actor.0);
    w.leb(b.seq as u64);
    w.leb(b.clock);
    w.leb(b.ops.len() as u64);
    for (i, op) in b.ops.iter().enumerate() {
        if b.actor.is_import() {
            put_stamp(&mut w, b.stamp(i));
        }
        put_op(&mut w, op);
    }
    EncodedBatch(w.buf)
}

fn batch_header(r: &mut Reader) -> Option<(ActorId, u32, u64, u64)> {
    if r.u8()? != CODEC_VERSION {
        return None;
    }
    let (actor, seq, clock, n) = (ActorId(r.u64()?), get_u32(r).filter(|&s| s >= 1)?, r.leb()?, r.leb()?);
    (n <= MAX_BATCH_OPS).then_some((actor, seq, clock, n))
}

/// Incremental batch decoder: a large remote batch is decoded a slice of ops at a time (remote-apply budget),
/// and handed out only when all of it decoded, so nothing is ever applied partially from damaged bytes.
pub struct BatchDecoder {
    at: usize,
    left: u64,
    batch: Batch,
}

impl BatchDecoder {
    pub fn new(bytes: &[u8]) -> Result<BatchDecoder, Corrupt> {
        let mut r = Reader::new(bytes);
        let (actor, seq, clock, left) = batch_header(&mut r).filter(|_| bytes.len() <= MAX_BATCH_BYTES).ok_or(Corrupt)?;
        let stamps = actor.is_import().then(Vec::new);
        Ok(BatchDecoder { at: bytes.len() - r.buf.len(), left, batch: Batch { actor, seq, clock, ops: Vec::new(), stamps } })
    }
    /// Decodes up to `max_ops` more ops of the same `bytes`. `Ok(true)` when the batch is complete.
    pub fn step(&mut self, bytes: &[u8], max_ops: usize) -> Result<bool, Corrupt> {
        let mut r = Reader::new(bytes.get(self.at..).ok_or(Corrupt)?);
        for _ in 0..self.left.min(max_ops as u64) {
            if let Some(stamps) = &mut self.batch.stamps {
                stamps.push(get_stamp(&mut r).ok_or(Corrupt)?);
            }
            self.batch.ops.push(get_op(&mut r).ok_or(Corrupt)?);
            self.left -= 1;
        }
        self.at = bytes.len() - r.buf.len();
        if self.left == 0 && !r.is_empty() {
            return Err(Corrupt); // trailing bytes
        }
        Ok(self.left == 0)
    }
    /// The decoded batch; `None` until `step` returned `Ok(true)`.
    pub fn finish(self) -> Option<Batch> { (self.left == 0).then_some(self.batch) }
}

pub fn decode_batch(bytes: &[u8]) -> Result<Batch, Corrupt> {
    let mut d = BatchDecoder::new(bytes)?;
    d.step(bytes, usize::MAX)?;
    d.finish().ok_or(Corrupt)
}

impl EncodedBatch {
    /// `(actor, seq, clock)` without decoding the ops (journal keys, dedupe, acks).
    pub fn header(&self) -> Option<(ActorId, u32, u64)> { batch_header(&mut Reader::new(&self.0)).map(|(a, s, c, _)| (a, s, c)) }
    pub fn decode(&self) -> Result<Batch, Corrupt> { decode_batch(&self.0) }
}

// ------------------------------------------------------------------------------------------------ snapshot

fn put_reg<T>(w: &mut Writer, reg: &Reg<T>, put: impl FnOnce(&mut Writer, &T)) {
    put(w, &reg.v);
    put_stamp(w, reg.at);
}
fn get_reg<T>(r: &mut Reader, get: impl FnOnce(&mut Reader) -> Option<T>) -> Option<Reg<T>> { Some(Reg { v: get(r)?, at: get_stamp(r)? }) }

/// Header: VV, name, page registers, timeline. Stored as the `docs` header chunk.
pub fn encode_header(doc: &Doc) -> Vec<u8> {
    let mut w = Writer::default();
    w.u8(CODEC_VERSION);
    doc.vv.encode(&mut w);
    doc.vv.encode_extras(&mut w);
    put_reg(&mut w, &doc.name, |w, v| put_str(w, v));
    let mut pages: Vec<(&Id, &Page)> = doc.pages.iter().collect();
    pages.sort_unstable_by_key(|(id, _)| **id);
    w.leb(pages.len() as u64);
    for (id, p) in pages {
        put_id(&mut w, *id);
        put_reg(&mut w, &p.pos, |w, v| put_bytes(w, &v.0));
        put_reg(&mut w, &p.deleted, |w, v| w.u8(*v as u8));
        put_reg(&mut w, &p.template, |w, v| put_template(w, *v));
        put_reg(&mut w, &p.size, |w, v| {
            w.leb(v.0 as u64);
            w.leb(v.1 as u64);
        });
        put_reg(&mut w, &p.background, put_background);
        put_reg(&mut w, &p.side_panel, |w, v| {
            w.u8(v.is_some() as u8);
            if let Some(v) = v {
                put_bytes(w, v);
            }
        });
    }
    w.leb(doc.versions.len() as u64);
    for (id, v) in &doc.versions {
        put_id(&mut w, *id);
        put_reg(&mut w, &v.data, |w, v| put_bytes(w, v));
        put_reg(&mut w, &v.deleted, |w, v| w.u8(*v as u8));
    }
    w.buf
}

const F_PAGE_AT: u8 = 1;
const F_Z_AT: u8 = 2;
const F_Z_NEW: u8 = 4;
const F_DELETED: u8 = 8;
const F_DEL_AT: u8 = 16;
const F_BODY: u8 = 32;

/// The chunk of one page: every item whose page register names it (live or deleted), ascending by id.
pub fn encode_chunk(doc: &Doc, page: Id) -> Vec<u8> {
    let mut w = Writer::default();
    w.u8(CODEC_VERSION);
    put_id(&mut w, page);
    let ids = doc.chunk_items(page);
    w.leb(ids.len() as u64);
    for id in ids {
        let Some(it) = doc.items.get(&id) else { continue };
        let at = it.body.at;
        let has_body = !it.body_dropped_in_snapshot();
        let z_new = it.z.v == Z { at, sub: 0 };
        let flags = (it.page.at == at) as u8 * F_PAGE_AT
            | (it.z.at == at) as u8 * F_Z_AT
            | z_new as u8 * F_Z_NEW
            | it.deleted.v as u8 * F_DELETED
            | (it.deleted.at != Stamp::ZERO) as u8 * F_DEL_AT
            | has_body as u8 * F_BODY;
        put_id(&mut w, id);
        w.u8(flags);
        put_stamp(&mut w, at);
        if flags & F_PAGE_AT == 0 {
            put_stamp(&mut w, it.page.at);
        }
        if !z_new {
            put_z(&mut w, it.z.v);
        }
        if flags & F_Z_AT == 0 {
            put_stamp(&mut w, it.z.at);
        }
        if flags & F_DEL_AT != 0 {
            put_stamp(&mut w, it.deleted.at);
        }
        if has_body {
            put_body(&mut w, &it.body.v);
        }
    }
    w.buf
}

/// The whole document: header and every chunk.
pub fn encode(doc: &Doc) -> EncodedSnapshot {
    let mut w = Writer::default();
    w.u8(CODEC_VERSION);
    put_bytes(&mut w, &encode_header(doc));
    let pages = doc.chunk_pages();
    w.leb(pages.len() as u64);
    for page in pages {
        put_bytes(&mut w, &encode_chunk(doc, page));
    }
    EncodedSnapshot(w.buf)
}

/// Builds a `Doc` from a header and chunks (the session's load path and `decode`). Each call is all-or-nothing.
#[derive(Default)]
pub struct Loader {
    doc: Doc,
}

impl Loader {
    pub fn header(&mut self, bytes: &[u8]) -> Result<(), Corrupt> {
        let decode = || {
            let mut r = Reader::new(bytes);
            if r.u8()? != CODEC_VERSION {
                return None;
            }
            let mut vv = VersionVector::decode(&mut r)?;
            vv.decode_extras(&mut r)?;
            let name = get_reg(&mut r, |r| get_str(r).map(str::to_string))?;
            let mut pages = Vec::new();
            for _ in 0..r.leb()? {
                let id = get_id(&mut r)?;
                let page = Page {
                    pos: get_reg(&mut r, get_pos).filter(|p| p.v.is_valid() || (p.v.0.is_empty() && p.at == Stamp::ZERO))?,
                    deleted: get_reg(&mut r, get_flag)?,
                    template: get_reg(&mut r, get_template)?,
                    size: get_reg(&mut r, get_size).filter(|s| valid_size(s.v) || (s.v == (0, 0) && s.at == Stamp::ZERO))?,
                    background: get_reg(&mut r, get_background)?,
                    side_panel: get_reg(&mut r, |r| match get_flag(r)? {
                        true => Some(Some(get_bytes(r)?.to_vec())),
                        false => Some(None),
                    })?,
                };
                pages.push((id, page));
            }
            let mut versions = Vec::new();
            for _ in 0..r.leb()? {
                versions.push((get_id(&mut r)?, Version { data: get_reg(&mut r, |r| get_bytes(r).map(<[u8]>::to_vec))?, deleted: get_reg(&mut r, get_flag)? }));
            }
            r.is_empty().then_some((vv, name, pages, versions))
        };
        let (vv, name, pages, versions) = decode().ok_or(Corrupt)?;
        self.doc.load_header(&vv, name, pages, versions);
        Ok(())
    }

    pub fn chunk(&mut self, bytes: &[u8]) -> Result<(), Corrupt> {
        let decode = || {
            let mut r = Reader::new(bytes);
            if r.u8()? != CODEC_VERSION {
                return None;
            }
            let page = get_id(&mut r)?;
            let mut items = Vec::new();
            for _ in 0..r.leb()? {
                let (id, flags) = (get_id(&mut r)?, r.u8()?);
                if flags >= 64 || (flags & F_DELETED != 0 && flags & F_DEL_AT == 0) {
                    return None;
                }
                let at = get_stamp(&mut r)?;
                let page_at = if flags & F_PAGE_AT != 0 { at } else { get_stamp(&mut r)? };
                let z = if flags & F_Z_NEW != 0 { Z { at, sub: 0 } } else { get_z(&mut r)? };
                let z_at = if flags & F_Z_AT != 0 { at } else { get_stamp(&mut r)? };
                let deleted_at = if flags & F_DEL_AT != 0 { get_stamp(&mut r)? } else { Stamp::ZERO };
                let body = if flags & F_BODY != 0 { get_body(&mut r)? } else { Body::Dropped };
                let item = Item { page: Reg::new(page, page_at), z: Reg::new(z, z_at), body: Reg::new(body, at), deleted: Reg::new(flags & F_DELETED != 0, deleted_at) };
                items.push((id, item));
            }
            r.is_empty().then_some(items)
        };
        self.doc.load_items(decode().ok_or(Corrupt)?);
        Ok(())
    }

    pub fn finish(mut self) -> Doc {
        self.doc.reindex();
        self.doc
    }
}

/// Decodes a whole snapshot into a fresh `Doc`; merge it with `Doc::merge` (never replace, SYNC §3.7).
pub fn decode(bytes: &[u8]) -> Result<Doc, Corrupt> {
    let mut r = Reader::new(bytes);
    let mut loader = Loader::default();
    if r.u8() != Some(CODEC_VERSION) {
        return Err(Corrupt);
    }
    loader.header(get_bytes(&mut r).ok_or(Corrupt)?)?;
    for _ in 0..r.leb().ok_or(Corrupt)? {
        loader.chunk(get_bytes(&mut r).ok_or(Corrupt)?)?;
    }
    if !r.is_empty() {
        return Err(Corrupt);
    }
    Ok(loader.finish())
}

impl EncodedSnapshot {
    pub fn decode(&self) -> Result<Doc, Corrupt> { decode(&self.0) }
    /// The snapshot's VV without decoding the document.
    pub fn version_vector(&self) -> Option<VersionVector> {
        let mut r = Reader::new(&self.0);
        let mut header = Reader::new(r.u8().filter(|&v| v == CODEC_VERSION).and_then(|_| get_bytes(&mut r))?);
        header.u8().filter(|&v| v == CODEC_VERSION)?;
        let mut vv = VersionVector::decode(&mut header)?;
        vv.decode_extras(&mut header)?;
        Some(vv)
    }
}

/// Test command hook: `__IHN_TEST_API__.cmd('core.snapshot.<name>', args)`.
pub fn test_cmd(_name: &str, _args: &str) -> Option<String> { None }

/// Random model values for the codec and convergence tests.
#[cfg(test)]
pub(crate) mod gen {
    use super::*;
    use crate::testutil::Rng;

    pub fn hash(rng: &mut Rng) -> Hash {
        let mut h = [0u8; 32];
        h.iter_mut().for_each(|b| *b = rng.next() as u8);
        h
    }
    pub fn id(rng: &mut Rng) -> Id { Id((rng.next() as u128) << 64 | rng.below(50) as u128) }
    pub fn stamp(rng: &mut Rng) -> Stamp { Stamp::new(rng.next() >> rng.below(64), ActorId(rng.next()), rng.below(1 << 20) as u32) }
    pub fn float(rng: &mut Rng) -> f64 {
        match rng.below(12) {
            0 => f64::NAN,
            1 => -0.0,
            2 => f64::INFINITY,
            3 => f64::from_bits(rng.next()), // any bit pattern, signalling NaNs included
            _ => rng.range(-50.0, 1200.0),
        }
    }
    pub fn color(rng: &mut Rng) -> Color {
        match rng.below(4) {
            0 => Color::Css(["red", "", "rgba(0, 47, 217, 0.45)", "#abc", "añil€"][rng.below(5) as usize].into()),
            _ => Color::Hex(rng.below(1 << 24) as u32),
        }
    }
    pub fn stroke(rng: &mut Rng, max_points: u64) -> Body {
        let n = rng.below(max_points + 1) as usize;
        let tool = [Tool::Pen, Tool::Pen, Tool::Highlighter, Tool::LegacyEraseArea, Tool::LegacyCover][rng.below(5) as usize];
        Body::Stroke {
            tool,
            color: color(rng),
            width: float(rng),
            xy: (0..n * 2).map(|_| float(rng)).collect(),
            p: rng.chance(0.6).then(|| (0..n).map(|_| rng.unit() as f32).collect()),
        }
    }
    pub fn body(rng: &mut Rng) -> Body {
        match rng.below(4) {
            0 => Body::Image { asset: hash(rng), cx: float(rng), cy: float(rng), w: float(rng), h: float(rng), rotation: float(rng) },
            _ => stroke(rng, 12),
        }
    }
    pub fn pos(rng: &mut Rng) -> Pos {
        let mut bytes: Vec<u8> = (0..rng.below(5)).map(|_| rng.next() as u8).collect();
        bytes.push(1 + rng.below(255) as u8);
        Pos(bytes)
    }
    pub fn background(rng: &mut Rng) -> Background {
        match rng.below(3) {
            0 => Background::Template,
            1 => Background::Pdf { asset: hash(rng), page: 1 + rng.below(2000) as u32 },
            _ => Background::Custom { asset: hash(rng) },
        }
    }
    pub fn template(rng: &mut Rng) -> Template { [Template::Default, Template::Agenda, Template::Diary][rng.below(3) as usize] }
    pub fn size(rng: &mut Rng) -> (u32, u32) { (64 + rng.below(3000) as u32, 64 + rng.below(3000) as u32) }
    pub fn op(rng: &mut Rng) -> Op {
        match rng.below(12) {
            0 => Op::PageNew { page: id(rng), pos: pos(rng), template: template(rng), size: size(rng), background: background(rng) },
            1 => Op::PageMove { page: id(rng), pos: pos(rng) },
            2 => Op::PageDel { page: id(rng), deleted: rng.chance(0.7) },
            3 => Op::PageSet {
                page: id(rng),
                prop: match rng.below(4) {
                    0 => PageProp::Template(template(rng)),
                    1 => PageProp::Size(size(rng)),
                    2 => PageProp::Background(background(rng)),
                    _ => PageProp::SidePanel(rng.chance(0.5).then(|| (0..rng.below(9)).map(|_| rng.next() as u8).collect())),
                },
            },
            4 => Op::ItemDel { item: id(rng), deleted: rng.chance(0.7) },
            5 => Op::DocName { name: ["", "Notes", "Año nuevo — 日本"][rng.below(3) as usize].to_string() },
            6 => Op::VersionAdd { meta: VersionMeta { id: id(rng), data: (0..rng.below(40)).map(|_| rng.next() as u8).collect() } },
            7 => Op::VersionDel { version: id(rng) },
            _ => Op::ItemPut { item: id(rng), page: id(rng), z: Z { at: stamp(rng), sub: rng.below(1 << 32) as u32 }, body: body(rng) },
        }
    }
    pub fn batch(rng: &mut Rng) -> Batch {
        let import = rng.chance(0.3);
        let actor = if import { ActorId::import(&format!("rev{}", rng.below(9))) } else { ActorId::tab(rng.next(), rng.below(256) as u8) };
        let ops: Vec<Op> = (0..rng.below(8)).map(|_| op(rng)).collect();
        let stamps = import.then(|| ops.iter().map(|_| stamp(rng)).collect());
        Batch { actor, seq: 1 + rng.below(1 << 20) as u32, clock: rng.next() >> rng.below(64), ops, stamps }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::{Author, Tx};
    use crate::id::IdGen;
    use crate::testutil::Rng;

    #[test]
    fn batch_bytes_are_the_sync_4_layout() {
        let actor = ActorId(0x0102_0304_0506_0700);
        let b = Batch { actor, seq: 300, clock: 5, ops: vec![Op::DocName { name: "hi".into() }, Op::ItemDel { item: Id(7), deleted: true }], stamps: None };
        let mut expect = vec![1, 0x00, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, 0xac, 0x02, 5, 2, 0x07, 2, b'h', b'i', 0x06, 7];
        expect.extend([0u8; 15]);
        expect.push(1);
        let enc = encode_batch(&b);
        assert_eq!(enc.0, expect);
        assert_eq!(enc.header(), Some((actor, 300, 5)));
        assert_eq!(enc.decode(), Ok(b));
        // Import batch: a stamp (uv clock, u64 actor, uv idx) precedes every op.
        let imp = ActorId::import("r1");
        let b = Batch { actor: imp, seq: 1, clock: 0, ops: vec![Op::PageDel { page: Id(1), deleted: false }], stamps: Some(vec![Stamp::new(200, ActorId(3), 1)]) };
        let enc = encode_batch(&b);
        assert_eq!(&enc.0[12..23], &[0xc8, 0x01, 3, 0, 0, 0, 0, 0, 0, 0, 1]);
        assert_eq!(enc.0[23], 0x03);
        assert_eq!(enc.decode(), Ok(b));
        // An ItemPut stroke: tag, ids, Z (stamp + sub), body.
        let body = Body::Stroke { tool: Tool::Highlighter, color: Color::Hex(0x00ff_aa00), width: 16.0, xy: vec![1.0, 2.0], p: Some(vec![0.5]) };
        let mut w = Writer::default();
        put_op(&mut w, &Op::ItemPut { item: Id(1), page: Id(2), z: Z { at: Stamp::new(9, ActorId(4), 0), sub: 3 }, body });
        let tail: Vec<u8> = [&[9u8, 4, 0, 0, 0, 0, 0, 0, 0, 0, 3][..], &[0, 1, 0, 0x00, 0xaa, 0xff, 0x00], &16f64.to_le_bytes(), &[1], &1f64.to_le_bytes(), &2f64.to_le_bytes(), &[1], &0.5f32.to_le_bytes()].concat();
        assert_eq!(w.buf[0], 0x05);
        assert_eq!(&w.buf[33..], &tail[..]);
    }

    #[test]
    fn random_batches_round_trip_bit_exactly() {
        let mut rng = Rng::new(5);
        for round in 0..3000 {
            let b = gen::batch(&mut rng);
            let enc = encode_batch(&b);
            let back = enc.decode().unwrap_or_else(|_| panic!("round {round}: {b:?}"));
            // NaN payloads make `==` useless: compare the re-encoding.
            assert_eq!(encode_batch(&back), enc, "round {round}");
            // Decoded a few ops at a time: the same batch; nothing is handed out before the end.
            let mut d = BatchDecoder::new(&enc.0).unwrap();
            let mut steps = 0;
            while !d.step(&enc.0, 3).unwrap() {
                steps += 1;
            }
            assert!(steps >= b.ops.len().saturating_sub(1) / 3);
            assert_eq!(encode_batch(&d.finish().unwrap()), enc);
            let mut d = BatchDecoder::new(&enc.0).unwrap();
            if !d.step(&enc.0, 1).unwrap() {
                assert!(d.finish().is_none());
            }
            assert_eq!((back.actor, back.seq, back.clock, back.ops.len(), back.stamps.is_some()), (b.actor, b.seq, b.clock, b.ops.len(), b.stamps.is_some()));
            assert!(back.is_well_formed());
        }
    }

    #[test]
    fn damaged_batches_are_rejected_whole_and_never_panic() {
        let mut rng = Rng::new(6);
        for _ in 0..400 {
            let enc = encode_batch(&gen::batch(&mut rng)).0;
            for cut in 0..enc.len() {
                assert_eq!(decode_batch(&enc[..cut]), Err(Corrupt), "truncated at {cut}");
            }
            let mut long = enc.clone();
            long.push(0);
            assert_eq!(decode_batch(&long), Err(Corrupt), "trailing byte");
            for _ in 0..20 {
                let mut bad = enc.clone();
                let at = rng.below(bad.len() as u64) as usize;
                bad[at] ^= 1 << rng.below(8);
                if let Ok(b) = decode_batch(&bad) {
                    assert!(b.is_well_formed()); // a flip inside a float or id still decodes: fine
                }
            }
            let noise: Vec<u8> = (0..rng.below(200)).map(|_| rng.next() as u8).collect();
            let _ = decode_batch(&noise);
            let _ = decode(&noise);
            let _ = EncodedSnapshot(noise.clone()).version_vector();
            let _ = EncodedBatch(noise).header();
        }
        // Version, seq 0, op-count limit, unknown tag, Dropped body, bad flags and bad values.
        let head = |seq: u8, n: &[u8]| [&[1u8, 0, 1, 0, 0, 0, 0, 0, 0, seq, 9][..], n].concat();
        assert!(decode_batch(&head(1, &[0])).is_ok());
        assert!(decode_batch(&[&[2u8][..], &head(1, &[0])[1..]].concat()).is_err());
        assert!(decode_batch(&head(0, &[0])).is_err());
        assert!(decode_batch(&head(1, &[0xc1, 0x84, 0x3d])).is_err()); // 1,000,001 ops
        assert!(decode_batch(&head(1, &[1, 0x0a])).is_err());
        let with_op = |op: &Op| {
            let mut w = Writer { buf: head(1, &[1]) };
            put_op(&mut w, op);
            decode_batch(&w.buf)
        };
        assert!(with_op(&Op::ItemPut { item: Id(1), page: Id(2), z: Z::default(), body: Body::Dropped }).is_err());
        assert!(with_op(&Op::PageMove { page: Id(1), pos: Pos(vec![]) }).is_err());
        assert!(with_op(&Op::PageMove { page: Id(1), pos: Pos(vec![5, 0]) }).is_err());
        assert!(with_op(&Op::PageSet { page: Id(1), prop: PageProp::Size((63, 800)) }).is_err());
        assert!(with_op(&Op::PageSet { page: Id(1), prop: PageProp::Background(Background::Pdf { asset: [0; 32], page: 0 }) }).is_err());
        assert!(with_op(&Op::PageSet { page: Id(1), prop: PageProp::Size((64, 64)) }).is_ok());
        // A point count larger than the input is rejected before any allocation.
        let mut w = Writer { buf: head(1, &[1]) };
        w.raw(&[0x05]);
        w.raw(&[0; 32]);
        w.raw(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        w.raw(&[0, 0, 0, 0, 0, 0, 0]);
        w.f64(1.0);
        w.raw(&[0xff, 0xff, 0xff, 0xff, 0x0f]);
        assert!(decode_batch(&w.buf).is_err());
    }

    /// A document made through `Tx` with every kind of content, deletions and an unborn page.
    fn sample(rng: &mut Rng, pages: usize, strokes: usize, points: u64) -> Doc {
        let mut doc = Doc::default();
        let mut au = Author::new(IdGen::new(ActorId::tab(0x77_0000, 1), 0), 0);
        let mut commit = |doc: &mut Doc, f: &mut dyn FnMut(&mut Tx)| {
            let mut tx = Tx::new(doc, &mut au, 1_700_000_000_000);
            f(&mut tx);
            let b = tx.commit();
            doc.apply(&b);
        };
        let mut last = None;
        for _ in 0..pages {
            commit(&mut doc, &mut |tx| last = Some(tx.page_new(last, gen::template(rng), gen::size(rng), gen::background(rng))));
        }
        for page in doc.order().to_vec() {
            for _ in 0..strokes {
                commit(&mut doc, &mut |tx| {
                    tx.add_stroke(page, Body::Stroke { tool: Tool::Pen, color: Color::Hex(0x1a1a1a), width: 2.0, xy: (0..points * 2).map(|_| rng.range(0.0, 800.0)).collect(), p: Some(vec![0.5; points as usize]) });
                });
            }
        }
        doc
    }

    #[test]
    fn snapshot_round_trip_is_canonical_and_chunked_loading_agrees() {
        let mut rng = Rng::new(8);
        let mut doc = sample(&mut rng, 4, 6, 5);
        let mut au = Author::new(IdGen::new(ActorId::tab(0x99_0000, 0), 0), 0);
        let pages = doc.order().to_vec();
        let mut tx = Tx::new(&doc, &mut au, 1_700_000_100_000);
        let victims = doc.page_items(pages[0]);
        tx.erase(&victims[..2]);
        tx.split_ranges(victims[2], &[(0, 2), (3, 5)]);
        tx.move_to_page(&victims[3..4], pages[1], 40.0);
        tx.add_image(pages[2], [7; 32], crate::op::ImageRect { cx: 100.0, cy: 90.0, w: 50.0, h: 40.0, rotation: 0.3 });
        tx.page_delete(pages[3]);
        tx.set_name("Sample ñ");
        tx.version_add(vec![1, 2, 3]);
        tx.page_set(pages[1], PageProp::SidePanel(Some(vec![9, 9])));
        tx.push(Op::ItemDel { item: Id(424242), deleted: true }); // an item known only by its deletion
        tx.push(Op::PageMove { page: Id(515151), pos: Pos(vec![9]) }); // a page not born yet
        let b = tx.commit();
        doc.apply(&b);
        doc.vv.add(ActorId(0x55_0000), 3); // an extra above a gap

        let snap = encode(&doc);
        let back = snap.decode().unwrap();
        assert_eq!(encode(&back), snap);
        assert_eq!((back.order(), &back.vv, &back.name), (doc.order(), &doc.vv, &doc.name));
        assert_eq!(snap.version_vector().as_ref(), Some(&doc.vv));
        for &page in &pages {
            assert_eq!(back.page_items(page), doc.page_items(page));
        }
        // Erased items keep their registers but lose their bodies; the document itself still has them.
        assert!(doc.items[&victims[0]].body.v.point_count() == 5 && back.items[&victims[0]].body.v.is_dropped());
        assert!(back.items[&victims[0]].deleted.v && back.items[&Id(424242)].deleted.v && !back.pages[&Id(515151)].is_born());

        // Header + chunks loaded one by one (the store's layout), in any order, give the same document.
        let mut loader = Loader::default();
        let mut chunks: Vec<Vec<u8>> = doc.chunk_pages().into_iter().map(|p| encode_chunk(&doc, p)).collect();
        rng.shuffle(&mut chunks);
        for (i, c) in chunks.iter().enumerate() {
            if i == 1 {
                loader.header(&encode_header(&doc)).unwrap();
            }
            loader.chunk(c).unwrap();
        }
        assert_eq!(encode(&loader.finish()), snap);

        // Merging into an empty document, twice, is the same again (snapshots merge, never replace).
        let mut merged = Doc::default();
        assert!(merged.merge(&back).applied);
        let again = merged.merge(&back);
        assert!(again.items.is_empty() && again.pages.is_empty() && !again.order);
        assert_eq!(encode(&merged), snap);

        // Damage: every truncation fails, nothing panics, a failed chunk leaves the loader untouched.
        for cut in 0..snap.0.len() {
            assert!(decode(&snap.0[..cut]).is_err(), "cut {cut}");
        }
        for _ in 0..3000 {
            let mut bad = snap.0.clone();
            let at = rng.below(bad.len() as u64) as usize;
            bad[at] ^= 1 << rng.below(8);
            let _ = decode(&bad);
        }
        let mut loader = Loader::default();
        assert!(loader.chunk(&chunks[0][..chunks[0].len() - 1]).is_err() && loader.header(&[1, 0]).is_err());
        assert!(loader.finish().items.is_empty());
    }

    #[test]
    fn snapshot_size_stays_near_the_raw_ink() {
        // The 20-page profile document: 20 pages × 150 strokes × 60 points, with pressure.
        let (pages, strokes, points) = (20usize, 150usize, 60u64);
        let doc = sample(&mut Rng::new(2), pages, strokes, points);
        let raw = pages * strokes * points as usize * (16 + 4);
        let size = encode(&doc).0.len();
        let overhead = size - raw;
        assert!(overhead <= pages * strokes * 56 + pages * 200 + 256, "snapshot {size} bytes, raw ink {raw}, overhead {overhead}");
        // A fresh stroke costs its points plus a small fixed record (one stamp, not five).
        let per_stroke = overhead as f64 / (pages * strokes) as f64;
        assert!(per_stroke < 56.0, "{per_stroke} bytes of overhead per stroke");
    }
}
