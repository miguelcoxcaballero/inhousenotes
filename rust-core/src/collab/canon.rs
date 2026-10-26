//! Canonical document hash and the structure / field-meta normalizers it needs.
//! Mirrors collaboration-core-v5.js as it was before the port: same coercions,
//! same key order, same UTF-16 string ordering and number formatting.
use super::json::*;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

const RANK_STEP: f64 = 1024.0;

/// ihnHashString over a stream of UTF-16 code units (two 32-bit FNV-style lanes).
pub struct Hasher { first: u32, second: u32, index: u32 }
impl Hasher {
    pub fn new() -> Self { Hasher { first: 0x811c_9dc5, second: 0x9e37_79b9, index: 0 } }
    pub fn finish(&self) -> String { format!("{:08x}{:08x}", self.first, self.second) }
}
impl Sink for Hasher {
    #[inline] fn put(&mut self, unit: u16) {
        let code = u32::from(unit);
        self.first ^= code;
        self.first = self.first.wrapping_mul(0x0100_0193);
        self.second ^= code.wrapping_add(self.index);
        self.second = self.second.wrapping_mul(0x85eb_ca6b);
        self.index = self.index.wrapping_add(1);
    }
}
pub fn hash_units(units: impl Iterator<Item = u16>) -> String {
    let mut h = Hasher::new();
    for u in units { h.put(u); }
    h.finish()
}

#[derive(Clone)]
pub struct Stamp { pub clock: f64, pub actor: Vec<u16> }
impl Stamp { fn zero() -> Self { Stamp { clock: 0.0, actor: Vec::new() } } }

fn get<'a>(pool: &Pool, v: Option<&'a J>, key: &str) -> Option<&'a J> { v.and_then(|j| j.get(pool, key)) }

pub fn norm_stamp(pool: &Pool, v: Option<&J>) -> Stamp {
    Stamp { clock: clamp_clock(pool, get(pool, v, "clock")), actor: str_or_empty(pool, get(pool, v, "actor")) }
}
fn compare_stamps(a: &Stamp, b: &Stamp) -> Ordering {
    if a.clock != b.clock { return if a.clock < b.clock { Ordering::Less } else { Ordering::Greater }; }
    a.actor.cmp(&b.actor)
}
fn max_stamp(a: Stamp, b: &Stamp) -> Stamp { if compare_stamps(&a, b) != Ordering::Less { a } else { b.clone() } }

fn write_stamp_sorted<S: Sink>(s: &mut S, st: &Stamp) {
    put_ascii(s, "{\"actor\":"); write_str(s, &st.actor); put_ascii(s, ",\"clock\":"); write_number(s, st.clock); s.put(0x7d);
}
fn write_stamp_plain<S: Sink>(s: &mut S, st: &Stamp) {
    put_ascii(s, "{\"clock\":"); write_number(s, st.clock); put_ascii(s, ",\"actor\":"); write_str(s, &st.actor); s.put(0x7d);
}

/// ihnNormalizePageIdList
pub fn normalize_page_ids(pool: &Pool, v: Option<&J>) -> Vec<Vec<u16>> {
    let Some(J::Arr(items)) = v else { return Vec::new(); };
    let mut seen: HashSet<Vec<u16>> = HashSet::new();
    let mut out = Vec::new();
    for item in items {
        let id = str_or_empty(pool, Some(item));
        if id.is_empty() || !seen.insert(id.clone()) { continue; }
        out.push(id);
    }
    out
}

pub struct Entry { pub rank: f64, pub order: Stamp, pub created: Stamp }
fn norm_entry(pool: &Pool, v: Option<&J>, fallback: f64) -> Entry {
    let rank = to_number(pool, get(pool, v, "rank"));
    Entry { rank: if rank.is_finite() { rank } else { fallback },
        order: norm_stamp(pool, get(pool, v, "orderStamp")), created: norm_stamp(pool, get(pool, v, "createdStamp")) }
}

pub struct Meta {
    pub clock: f64,
    pub entries: Vec<(Vec<u16>, Entry)>,
    pub tombstones: Vec<(Vec<u16>, Stamp)>,
    entry_index: HashMap<Vec<u16>, usize>,
    tomb_index: HashMap<Vec<u16>, usize>,
}

/// ihnNormalizeStructureMeta
pub fn normalize_structure(pool: &Pool, meta: Option<&J>, page_ids: &[Vec<u16>]) -> Meta {
    let mut m = Meta { clock: clamp_clock(pool, get(pool, meta, "clock")), entries: Vec::new(), tombstones: Vec::new(),
        entry_index: HashMap::new(), tomb_index: HashMap::new() };
    if let Some(raw) = get(pool, meta, "entries").filter(|v| v.is_object()) {
        for (index, (key, val)) in sorted_pairs(pool, raw).into_iter().enumerate() {
            if key.is_empty() { continue; }
            let entry = norm_entry(pool, Some(val), index as f64 * RANK_STEP);
            m.clock = m.clock.max(entry.order.clock).max(entry.created.clock);
            m.entry_index.insert(key.clone(), m.entries.len());
            m.entries.push((key, entry));
        }
    }
    if let Some(raw) = get(pool, meta, "tombstones").filter(|v| v.is_object()) {
        for (key, val) in sorted_pairs(pool, raw) {
            if key.is_empty() { continue; }
            let stamp = norm_stamp(pool, Some(val));
            m.clock = m.clock.max(stamp.clock);
            m.tomb_index.insert(key.clone(), m.tombstones.len());
            m.tombstones.push((key, stamp));
        }
    }
    for (index, id) in page_ids.iter().enumerate() {
        if !m.entry_index.contains_key(id) {
            m.entry_index.insert(id.clone(), m.entries.len());
            m.entries.push((id.clone(), Entry { rank: index as f64 * RANK_STEP, order: Stamp::zero(), created: Stamp::zero() }));
        }
    }
    m
}

fn write_entry_sorted<S: Sink>(s: &mut S, e: &Entry) {
    put_ascii(s, "{\"createdStamp\":"); write_stamp_sorted(s, &e.created);
    put_ascii(s, ",\"orderStamp\":"); write_stamp_sorted(s, &e.order);
    put_ascii(s, ",\"rank\":"); write_number(s, e.rank); s.put(0x7d);
}

/// ihnStableStringify(ihnCanonicalStructurePayload(meta, pageIds)); `ids` is the already-normalized id list or None.
pub fn write_structure_payload<S: Sink>(s: &mut S, m: &Meta, ids: Option<&[Vec<u16>]>) {
    let mut list: Vec<&Vec<u16>> = match ids {
        Some(list) => list.iter().collect(),
        None => { let mut set: HashSet<&Vec<u16>> = HashSet::new();
            for (k, _) in &m.entries { set.insert(k); } for (k, _) in &m.tombstones { set.insert(k); } set.into_iter().collect() }
    };
    list.sort(); list.dedup();
    let with_entry: Vec<&Vec<u16>> = list.iter().copied().filter(|k| m.entry_index.contains_key(*k)).collect();
    let with_tomb: Vec<&Vec<u16>> = list.iter().copied().filter(|k| m.tomb_index.contains_key(*k)).collect();
    put_ascii(s, "{\"entries\":");
    let keys: Vec<&[u16]> = with_entry.iter().map(|k| &k[..]).collect();
    write_map(s, &keys, |s, i| write_entry_sorted(s, &m.entries[m.entry_index[with_entry[i]]].1));
    put_ascii(s, ",\"tombstones\":");
    let keys: Vec<&[u16]> = with_tomb.iter().map(|k| &k[..]).collect();
    write_map(s, &keys, |s, i| write_stamp_sorted(s, &m.tombstones[m.tomb_index[with_tomb[i]]].1));
    put_ascii(s, ",\"v\":1}");
}

/// JSON of the normalized meta in JS insertion order (the host parses it back into an object).
pub fn write_structure_plain<S: Sink>(s: &mut S, m: &Meta) {
    put_ascii(s, "{\"v\":1,\"clock\":"); write_number(s, m.clock); put_ascii(s, ",\"entries\":{");
    let mut first = true;
    for (k, e) in &m.entries {
        if is_proto(k) { continue; }
        if !first { s.put(0x2c); } first = false;
        write_str(s, k); put_ascii(s, ":{\"rank\":"); write_number(s, e.rank);
        put_ascii(s, ",\"orderStamp\":"); write_stamp_plain(s, &e.order);
        put_ascii(s, ",\"createdStamp\":"); write_stamp_plain(s, &e.created); s.put(0x7d);
    }
    put_ascii(s, "},\"tombstones\":{");
    first = true;
    for (k, t) in &m.tombstones {
        if is_proto(k) { continue; }
        if !first { s.put(0x2c); } first = false;
        write_str(s, k); s.put(0x3a); write_stamp_plain(s, t);
    }
    put_ascii(s, "}}");
}

pub struct Fields<'a> { pub clock: f64, pub fields: Vec<(Vec<u16>, &'a J, Stamp)> }

/// ihnNormalizeFieldMeta
pub fn normalize_fields<'a>(pool: &Pool, meta: Option<&'a J>) -> Fields<'a> {
    let mut f = Fields { clock: clamp_clock(pool, get(pool, meta, "clock")), fields: Vec::new() };
    if let Some(raw) = get(pool, meta, "fields").filter(|v| v.is_object()) {
        for (key, rec) in sorted_pairs(pool, raw) {
            if key.is_empty() { continue; }
            let Some(value) = rec.get(pool, "value") else { continue; };
            let stamp = norm_stamp(pool, rec.get(pool, "stamp"));
            f.clock = f.clock.max(stamp.clock);
            f.fields.push((key, value, stamp));
        }
    }
    f
}

fn write_fields_sorted<S: Sink>(s: &mut S, pool: &Pool, f: &Fields) {
    let keys: Vec<&[u16]> = f.fields.iter().map(|x| &x.0[..]).collect();
    write_map(s, &keys, |s, i| {
        let (_, value, stamp) = &f.fields[i];
        put_ascii(s, "{\"stamp\":"); write_stamp_sorted(s, stamp); put_ascii(s, ",\"value\":"); write_stable(s, pool, value); s.put(0x7d);
    });
}

pub fn write_fields_plain<S: Sink>(s: &mut S, pool: &Pool, f: &Fields) {
    put_ascii(s, "{\"v\":1,\"clock\":"); write_number(s, f.clock); put_ascii(s, ",\"fields\":{");
    let mut first = true;
    for (k, value, stamp) in &f.fields {
        if is_proto(k) { continue; }
        if !first { s.put(0x2c); } first = false;
        write_str(s, k); put_ascii(s, ":{\"value\":"); write_stable(s, pool, value);
        put_ascii(s, ",\"stamp\":"); write_stamp_plain(s, stamp); s.put(0x7d);
    }
    put_ascii(s, "}}");
}

/// ihnFieldMetaHash
pub fn field_meta_hash(pool: &Pool, meta: Option<&J>) -> String {
    let f = normalize_fields(pool, meta);
    let mut h = Hasher::new();
    put_ascii(&mut h, "{\"fields\":"); write_fields_sorted(&mut h, pool, &f); put_ascii(&mut h, ",\"v\":1}");
    h.finish()
}

/// ihnStructureHash
pub fn structure_hash(pool: &Pool, meta: Option<&J>, page_ids: Option<&J>) -> String {
    let given = truthy(pool, page_ids);
    let ids = normalize_page_ids(pool, page_ids);
    let m = normalize_structure(pool, meta, if given { &ids } else { &[] });
    let mut h = Hasher::new();
    write_structure_payload(&mut h, &m, if given { Some(&ids) } else { None });
    h.finish()
}

// -- deletion stamps (consumed by the page canonicalizer) ----------------------

fn pick<'a>(pool: &Pool, v: Option<&'a J>) -> Option<&'a J> {
    match v { Some(j) => match j.get(pool, "stamp") { Some(inner) if inner.truthy(pool) => Some(inner), _ => Some(j) }, None => None }
}
fn record(map: &mut HashMap<Vec<u16>, Stamp>, id: Vec<u16>, stamp: Stamp) {
    if id.is_empty() { return; }
    let merged = match map.remove(&id) { Some(previous) => max_stamp(previous, &stamp), None => stamp };
    map.insert(id, merged);
}
fn consume(pool: &Pool, map: &mut HashMap<Vec<u16>, Stamp>, value: Option<&J>) {
    let Some(v) = value.filter(|v| v.truthy(pool)) else { return; };
    match v {
        J::Arr(items) => for entry in items {
            if entry.is_object() {
                let raw = pick(pool, Some(entry));
                record(map, str_or_empty(pool, entry.get(pool, "id")), norm_stamp(pool, pick(pool, raw)));
            } else {
                // Legacy deletedStrokeIds held bare ids: Lamport zero lets a later causal undo recreate the item.
                record(map, str_or_empty(pool, Some(entry)), Stamp::zero());
            }
        },
        J::Obj(entries) => for (k, val) in entries { record(map, pool.get(*k).to_vec(), norm_stamp(pool, pick(pool, Some(val)))); },
        _ => {}
    }
}
/// ihnNormalizeDeletionStamps (plain JSON inputs), sorted by id.
pub fn normalize_deletions(pool: &Pool, stamps: Option<&J>, legacy: Option<&J>) -> Vec<(Vec<u16>, Stamp)> {
    let mut map = HashMap::new();
    consume(pool, &mut map, stamps);
    consume(pool, &mut map, legacy);
    let mut list: Vec<(Vec<u16>, Stamp)> = map.into_iter().collect();
    list.sort_by(|a, b| a.0.cmp(&b.0));
    list
}

// -- canonical live page ---------------------------------------------------------

const VOLATILE: [&str; 4] = ["preview", "needsRedraw", "unloaded", "pendingUnload"];

fn write_item<S: Sink>(s: &mut S, pool: &Pool, item: &J) {
    if item.is_object() { write_stable_skip(s, pool, item, &VOLATILE); } else { write_stable(s, pool, item); }
}

fn write_items<S: Sink>(s: &mut S, pool: &Pool, list: Option<&J>) {
    let items: &[J] = match list { Some(J::Arr(a)) => a, _ => &[] };
    let mut keyed: Vec<(Vec<u16>, usize)> = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let key = match item.get(pool, "id") {
            Some(id) if id.truthy(pool) => to_js_string(pool, id),
            _ => { let mut k: Vec<u16> = Vec::new(); write_item(&mut k, pool, item); k }
        };
        keyed.push((key, i));
    }
    keyed.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    s.put(0x5b);
    for (n, (_, i)) in keyed.iter().enumerate() { if n > 0 { s.put(0x2c); } write_item(s, pool, &items[*i]); }
    s.put(0x5d);
}

fn write_stamp_map<S: Sink>(s: &mut S, list: &[(Vec<u16>, Stamp)]) {
    let keys: Vec<&[u16]> = list.iter().map(|x| &x.0[..]).collect();
    write_map(s, &keys, |s, i| write_stamp_sorted(s, &list[i].1));
}

/// ihnStableStringify(ihnCanonicalLivePage(page))
pub fn write_page<S: Sink>(s: &mut S, pool: &Pool, page: &J) {
    let g = |k: &str| page.get(pool, k);
    let text_or = |k: &str, default: &str| -> Vec<u16> {
        match g(k) { Some(j) if j.truthy(pool) => to_js_string(pool, j), _ => default.encode_utf16().collect() }
    };
    put_ascii(s, "{\"backgroundSource\":"); write_str(s, &text_or("backgroundSource", "template"));
    put_ascii(s, ",\"deletedStrokeIds\":[");
    let mut ids: Vec<Vec<u16>> = Vec::new();
    if let Some(J::Arr(items)) = g("deletedStrokeIds") {
        let mut seen: HashSet<Vec<u16>> = HashSet::new();
        for item in items { if item.truthy(pool) { let id = to_js_string(pool, item); if seen.insert(id.clone()) { ids.push(id); } } }
    }
    ids.sort();
    for (i, id) in ids.iter().enumerate() { if i > 0 { s.put(0x2c); } write_str(s, id); }
    put_ascii(s, "],\"deletedStrokeStamps\":");
    write_stamp_map(s, &normalize_deletions(pool, g("deletedStrokeStamps"), g("deletedStrokeIds")));
    put_ascii(s, ",\"images\":"); write_items(s, pool, g("images"));
    let dim = |k: &str| { let n = to_number(pool, g(k)); if n.is_nan() { 0.0 } else { n } };
    put_ascii(s, ",\"pageHeight\":"); write_number(s, dim("pageHeight"));
    put_ascii(s, ",\"pageId\":"); write_str(s, &str_or_empty(pool, g("pageId")));
    put_ascii(s, ",\"pageWidth\":"); write_number(s, dim("pageWidth"));
    put_ascii(s, ",\"sidePanel\":");
    match g("sidePanel").filter(|p| p.truthy(pool)) {
        None => put_ascii(s, "null"),
        Some(panel) => {
            put_ascii(s, "{\"dateKeys\":");
            match panel.get(pool, "dateKeys") {
                Some(J::Arr(keys)) => {
                    s.put(0x5b);
                    for (i, k) in keys.iter().enumerate() { if i > 0 { s.put(0x2c); } write_str(s, &to_js_string(pool, k)); }
                    s.put(0x5d);
                }
                _ => put_ascii(s, "null"),
            }
            put_ascii(s, ",\"title\":"); write_str(s, &str_or_empty(pool, panel.get(pool, "title"))); s.put(0x7d);
        }
    }
    put_ascii(s, ",\"strokes\":"); write_items(s, pool, g("strokes"));
    put_ascii(s, ",\"templateKind\":"); write_str(s, &str_or_empty(pool, g("templateKind"))); s.put(0x7d);
}

/// ihnCanonicalDocumentHash. Input: {"pages":[..],"structure":..,"calendar":..,"name":"..","fields":..}
pub fn document_hash(input: &[u8]) -> Result<String, String> {
    let mut pool = Pool::default();
    let root = parse(&mut pool, input)?;
    let pool = &pool;
    let g = |k: &str| root.get(pool, k);
    let mut h = Hasher::new();
    put_ascii(&mut h, "{\"calendarPageConfig\":");
    match g("calendar") { Some(c) if c.truthy(pool) => write_stable(&mut h, pool, c), _ => put_ascii(&mut h, "null") }
    put_ascii(&mut h, ",\"exportName\":"); write_str(&mut h, &str_or_empty(pool, g("name")));
    put_ascii(&mut h, ",\"fields\":"); write_fields_sorted(&mut h, pool, &normalize_fields(pool, g("fields")));
    put_ascii(&mut h, ",\"pages\":[");
    if let Some(J::Arr(pages)) = g("pages") {
        for (i, page) in pages.iter().enumerate() { if i > 0 { h.put(0x2c); } write_page(&mut h, pool, page); }
    }
    put_ascii(&mut h, "],\"structure\":");
    write_structure_payload(&mut h, &normalize_structure(pool, g("structure"), &[]), None);
    h.put(0x7d);
    Ok(h.finish())
}
