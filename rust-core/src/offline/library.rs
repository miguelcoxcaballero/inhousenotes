//! Offline library catalogue: the reload merge (alias map, remote-newer overlay, parent aliasing), the
//! per-directory index, `compareEntries` / `compareRecents` ordering, folder listing and the Home views.
//!
//! The JavaScript side keeps the entry objects (cards need them) and talks in indexes: the table below is
//! loaded once per reload from a columnar projection, and queries answer with arrays of `u32` indexes into the
//! entry array. Foreign rows (network results the caller passes in) are indexed after the table rows.
//! All integers on the wire are little endian `u32`; `u32::MAX` means "none".

use serde::Deserialize;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashMap as StdHashMap;
use std::hash::{BuildHasherDefault, Hasher};

/// Multiplicative string hasher (FxHash): ids are short and not attacker chosen, SipHash would dominate the load.
#[derive(Default)]
struct Fx(u64);
impl Hasher for Fx {
    fn finish(&self) -> u64 { self.0 }
    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut word = [0u8; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            self.0 = (self.0.rotate_left(5) ^ u64::from_le_bytes(word)).wrapping_mul(0x517c_c1b7_2722_0a95);
        }
    }
    fn write_u8(&mut self, byte: u8) { self.0 = (self.0.rotate_left(5) ^ byte as u64).wrapping_mul(0x517c_c1b7_2722_0a95); }
}
type HashMap<K, V> = StdHashMap<K, V, BuildHasherDefault<Fx>>;

const NONE: u32 = u32::MAX;
const TRASHED: u8 = 1;
const FOLDER: u8 = 2;
const PDF: u8 = 4;
const PENDING: u8 = 8;
const STARRED: u8 = 16;
const NOT_OWNED: u8 = 32;
const RECENT_LIMIT: usize = 12;
const SHARED_LIMIT: usize = 24;

#[derive(Default)]
struct Table {
    id: Vec<String>,
    /// remoteId, empty when there is none
    rid: Vec<String>,
    /// last opened time per document id and per document remote id (first document wins an alias)
    doc_by_id: HashMap<String, f64>,
    doc_by_remote: HashMap<String, f64>,
    flags: Vec<u8>,
    modified: Vec<f64>,
    opened: Vec<f64>,
    lname: Vec<Option<String>>,
    rank: Vec<u32>,
    children: HashMap<String, Vec<u32>>,
    active: Vec<u32>,
    /// active PDFs that are pending or have a `local-` id: `recentItems` always lists them
    unsynced: Vec<u32>,
    /// Saved Home views (recents, starred, shared) already resolved to table indexes; `None` when not an array.
    views: [Option<Vec<u32>>; 3],
}

thread_local! { static TABLE: RefCell<Table> = RefCell::new(Table::default()); }

fn is_local(id: &str) -> bool { id.starts_with("local-") }

/// `a < b ? -1 : a > b ? 1 : 0` on JavaScript strings, which compares UTF-16 code units.
fn cmp_keys(a: &str, b: &str) -> Ordering {
    if a.is_ascii() && b.is_ascii() { a.cmp(b) } else { a.encode_utf16().cmp(b.encode_utf16()) }
}
/// JavaScript truthiness of a number (`||` falls through on 0 and NaN).
fn truthy(value: f64) -> bool { value != 0.0 && !value.is_nan() }
fn sign(value: f64) -> Ordering { if value < 0.0 { Ordering::Less } else { Ordering::Greater } }

/// One row of `compareEntries`: folders first, folders by collation rank, files newest first, then by key.
struct EntryRow<'a> { folder: bool, rank: u32, modified: f64, key: &'a str }
fn compare_entries(a: &EntryRow, b: &EntryRow) -> Ordering {
    if a.folder != b.folder { return if a.folder { Ordering::Less } else { Ordering::Greater }; }
    let order = if a.folder { a.rank.cmp(&b.rank) } else {
        let delta = b.modified - a.modified;
        if truthy(delta) { sign(delta) } else { Ordering::Equal }
    };
    order.then_with(|| cmp_keys(a.key, b.key))
}

/// One row of `compareRecents`: last opened first, then newest, then by key.
#[derive(Clone, Copy)]
struct Recent<'a> { at: u32, key: &'a str, modified: f64, opened: f64 }
fn compare_recents(a: &Recent, b: &Recent) -> Ordering {
    let mut delta = b.opened - a.opened;
    if !truthy(delta) { delta = b.modified - a.modified; }
    if truthy(delta) { sign(delta) } else { cmp_keys(a.key, b.key) }
}

impl Table {
    fn len(&self) -> usize { self.id.len() }
    /// `record(id)?.lastOpenedAt || 0`: the document by id, else by remote id.
    fn opened_of(&self, id: &str) -> f64 { self.doc_by_id.get(id).or_else(|| self.doc_by_remote.get(id)).copied().unwrap_or(0.0) }
    /// `String(remoteId || id)`, the tie-break key of every ordering.
    fn key(&self, i: usize) -> &str { if self.rid[i].is_empty() { &self.id[i] } else { &self.rid[i] } }
    fn entry_row(&self, i: u32) -> EntryRow<'_> {
        let i = i as usize;
        EntryRow { folder: self.flags[i] & FOLDER != 0, rank: self.rank[i], modified: self.modified[i], key: self.key(i) }
    }
    fn recent(&self, i: u32) -> Recent<'_> {
        let at = i as usize;
        Recent { at: i, key: self.key(at), modified: self.modified[at], opened: self.opened[at] }
    }
    fn sorted_entries(&self, mut list: Vec<u32>) -> Vec<u32> {
        list.sort_by(|&a, &b| compare_entries(&self.entry_row(a), &self.entry_row(b)));
        list
    }
    fn sorted_recents(&self, list: &[u32]) -> Vec<u32> {
        let mut rows: Vec<Recent> = list.iter().map(|&i| self.recent(i)).collect();
        rows.sort_by(compare_recents);
        rows.into_iter().map(|row| row.at).collect()
    }
}

/// A JSON value that is text, null, or anything else. The host sends raw record fields; only text can match.
enum Loose { Null, Text(String), Number(f64), Other }
impl Default for Loose { fn default() -> Self { Loose::Null } }
impl<'de> Deserialize<'de> for Loose {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visit;
        impl<'de> serde::de::Visitor<'de> for Visit {
            type Value = Loose;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result { formatter.write_str("any JSON value") }
            fn visit_unit<E>(self) -> Result<Loose, E> { Ok(Loose::Null) }
            fn visit_none<E>(self) -> Result<Loose, E> { Ok(Loose::Null) }
            fn visit_str<E>(self, value: &str) -> Result<Loose, E> { Ok(Loose::Text(value.to_string())) }
            fn visit_string<E>(self, value: String) -> Result<Loose, E> { Ok(Loose::Text(value)) }
            fn visit_bool<E>(self, _: bool) -> Result<Loose, E> { Ok(Loose::Other) }
            fn visit_i64<E>(self, value: i64) -> Result<Loose, E> { Ok(Loose::Number(value as f64)) }
            fn visit_u64<E>(self, value: u64) -> Result<Loose, E> { Ok(Loose::Number(value as f64)) }
            fn visit_f64<E>(self, value: f64) -> Result<Loose, E> { Ok(Loose::Number(value)) }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Loose, A::Error> {
                while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
                Ok(Loose::Other)
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Loose, A::Error> {
                while map.next_entry::<serde::de::IgnoredAny, serde::de::IgnoredAny>()?.is_some() {}
                Ok(Loose::Other)
            }
        }
        deserializer.deserialize_any(Visit)
    }
}
impl Loose {
    fn text(self) -> Option<String> { if let Loose::Text(value) = self { Some(value) } else { None } }
    /// `Number(value || 0)`: text counts as a number when it parses, anything else is NaN.
    fn opened(&self) -> f64 {
        match self {
            Loose::Null => 0.0,
            Loose::Number(value) => *value,
            Loose::Text(text) => if text.is_empty() { 0.0 } else { text.trim().parse().unwrap_or(f64::NAN) },
            Loose::Other => f64::NAN,
        }
    }
}
/// Parent lists (and saved views) arrive raw: anything but an array is `None`, and elements that are not text
/// can never match an id.
#[derive(Default)]
struct ParentList(Option<Vec<Option<String>>>);
impl<'de> Deserialize<'de> for ParentList {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visit;
        impl<'de> serde::de::Visitor<'de> for Visit {
            type Value = ParentList;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result { formatter.write_str("a parent list or null") }
            fn visit_unit<E>(self) -> Result<ParentList, E> { Ok(ParentList(None)) }
            fn visit_none<E>(self) -> Result<ParentList, E> { Ok(ParentList(None)) }
            fn visit_str<E>(self, _: &str) -> Result<ParentList, E> { Ok(ParentList(None)) }
            fn visit_bool<E>(self, _: bool) -> Result<ParentList, E> { Ok(ParentList(None)) }
            fn visit_i64<E>(self, _: i64) -> Result<ParentList, E> { Ok(ParentList(None)) }
            fn visit_u64<E>(self, _: u64) -> Result<ParentList, E> { Ok(ParentList(None)) }
            fn visit_f64<E>(self, _: f64) -> Result<ParentList, E> { Ok(ParentList(None)) }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<ParentList, A::Error> {
                while map.next_entry::<serde::de::IgnoredAny, serde::de::IgnoredAny>()?.is_some() {}
                Ok(ParentList(None))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<ParentList, A::Error> {
                let mut list = Vec::with_capacity(seq.size_hint().unwrap_or(2).min(64));
                while let Some(item) = seq.next_element::<Loose>()? { list.push(item.text()); }
                Ok(ParentList(Some(list)))
            }
        }
        deserializer.deserialize_any(Visit)
    }
}

fn words(values: &[u32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 4);
    for value in values { out.extend_from_slice(&value.to_le_bytes()); }
    out
}
fn parse<'a, T: Deserialize<'a>>(input: &'a [u8]) -> Result<T, String> {
    serde_json::from_slice(input).map_err(|error| format!("Invalid input: {error}"))
}

// ---- reload merge ---------------------------------------------------------------------------------------------

#[derive(Deserialize)]
struct MergeIn {
    /// ids
    i: Vec<String>,
    /// `(position, remoteId)` for the records that have a remote id
    r: Vec<(u32, String)>,
    /// positions whose `pending` is truthy
    p: Vec<u32>,
    /// `(position, Date.parse(modifiedTime) || 0)` for every record the remote-newer test can compare
    m: Vec<(u32, f64)>,
    /// own `parents` array per record (null when absent or not an array)
    a: Vec<ParentList>,
    /// positions without an own `parents` property (an overlay copies the property even when undefined)
    np: Vec<u32>,
    root: String,
}

/// What `merge` hands to the following `load`: the surviving records in order, with their final parents.
struct Merged { ids: Vec<String>, rids: Vec<String>, parents: Vec<Vec<Option<String>>> }
thread_local! { static MERGED: RefCell<Option<Merged>> = const { RefCell::new(None) }; }

#[derive(Clone, Copy)]
enum Alias { Item(u32), Root }

/// Decides which stored records survive, which remote copy overlays each, and which parent ids are aliases.
/// Output: `[kept, patches, identity]`, then (unless identity) `kept` source indexes, `kept` overlay indexes (or NONE), then `patches` triples
/// `(keptPosition, parentPosition, aliasTarget)` where the target is an item index, or NONE for `root`.
/// The aliased parent lists stay here for the next `load`.
fn merge(input: &[u8]) -> Result<Vec<u8>, String> {
    let MergeIn { i: mut ids, r, p, m, a, np, root } = parse(input)?;
    let mut parents: Vec<Option<Vec<Option<String>>>> = a.into_iter().map(|list| list.0).collect();
    let count = ids.len();
    let position = |at: u32| -> Result<usize, String> { if (at as usize) < count { Ok(at as usize) } else { Err("Merge position out of range".to_string()) } };
    if count >= NONE as usize || parents.len() != count { return Err("Inconsistent merge columns".into()); }
    let mut remote_ids = vec![String::new(); count];
    for (at, id) in r { let at = position(at)?; remote_ids[at] = id; }
    let mut pending = vec![false; count];
    for at in p { pending[position(at)?] = true; }
    let mut modified = vec![0.0; count];
    for (at, value) in m { modified[position(at)?] = value; }
    let mut own_parents = vec![true; count];
    for at in np { own_parents[position(at)?] = false; }

    let mut aliases: HashMap<&str, Alias> = HashMap::default();
    for index in 0..count {
        if !remote_ids[index].is_empty() && remote_ids[index] != ids[index] { aliases.insert(&remote_ids[index], Alias::Item(index as u32)); }
    }
    // Most catalogues hold only plain Drive records: no stored record has a remote id, so no map is needed.
    let has_remote = remote_ids.iter().any(|id| !id.is_empty());
    let mut by_id: HashMap<&str, usize> = HashMap::default();
    if has_remote { by_id.reserve(count); for index in 0..count { by_id.insert(&ids[index], index); } }
    if !root.is_empty() { aliases.insert(&root, Alias::Root); }
    let (mut kept, mut overlay, mut sources, mut patches) = (Vec::with_capacity(count), Vec::with_capacity(count), Vec::with_capacity(count), Vec::<u32>::new());
    let fallback: Vec<Option<String>> = vec![Some("root".into())];
    for index in 0..count {
        if !aliases.is_empty() && aliases.contains_key(ids[index].as_str()) { continue; }
        let remote = if !pending[index] && !remote_ids[index].is_empty() { by_id.get(remote_ids[index].as_str()).copied() } else { None };
        let newer = remote.filter(|&r| modified[r] >= modified[index]);
        let source = match newer { Some(r) if own_parents[r] => r, _ => index };
        let position = kept.len() as u32;
        if !aliases.is_empty() {
            for (slot, parent) in parents[source].as_ref().unwrap_or(&fallback).iter().enumerate() {
                let Some(parent) = parent else { continue };
                match aliases.get(parent.as_str()) {
                    Some(Alias::Item(target)) => patches.extend_from_slice(&[position, slot as u32, *target]),
                    Some(Alias::Root) => patches.extend_from_slice(&[position, slot as u32, NONE]),
                    None => {}
                }
            }
        }
        kept.push(index as u32);
        overlay.push(newer.map_or(NONE, |r| r as u32));
        sources.push(source);
    }
    drop(aliases); drop(by_id);
    // Final parent lists: a record's own list moves, an overlay's list is copied, then aliased entries are patched.
    let mut final_parents: Vec<Vec<Option<String>>> = Vec::with_capacity(kept.len());
    for (slot, &source) in sources.iter().enumerate() {
        let list = if source == kept[slot] as usize { parents[source].take() } else { parents[source].clone() };
        final_parents.push(list.unwrap_or_else(|| fallback.clone()));
    }
    for patch in patches.chunks_exact(3) {
        let value = if patch[2] == NONE { "root".to_string() } else { ids[patch[2] as usize].clone() };
        final_parents[patch[0] as usize][patch[1] as usize] = Some(value);
    }
    let merged = Merged { ids: kept.iter().map(|&at| std::mem::take(&mut ids[at as usize])).collect(),
        rids: kept.iter().map(|&at| std::mem::take(&mut remote_ids[at as usize])).collect(), parents: final_parents };
    MERGED.with(|cell| *cell.borrow_mut() = Some(merged));
    // Nothing dropped, overlaid or aliased (the usual Drive-only catalogue): the host keeps its records as they are.
    let identity = kept.len() == count && patches.is_empty() && overlay.iter().all(|&at| at == NONE);
    if identity { return Ok(words(&[count as u32, 0, 1])); }
    let mut out = words(&[kept.len() as u32, (patches.len() / 3) as u32, 0]);
    out.extend(words(&kept)); out.extend(words(&overlay)); out.extend(words(&patches));
    Ok(out)
}

// ---- table ------------------------------------------------------------------------------------------------------

/// A `document-index` row as stored; only the fields the catalogue needs are read.
#[derive(Deserialize)]
struct DocRow {
    id: String,
    #[serde(rename = "remoteId", default)]
    remote_id: Loose,
    #[serde(rename = "lastOpenedAt", default)]
    opened: Loose,
}

#[derive(Deserialize)]
struct LoadIn {
    /// flags of the records `merge` kept, in order
    fl: Vec<u8>,
    /// `modifiedTime` text (null when missing) of the same records
    t: Vec<Loose>,
    docs: Vec<DocRow>,
    /// the saved Home views object `{recents, starred, shared}`
    views: Views,
}
#[derive(Deserialize, Default)]
struct Views { #[serde(default)] recents: ParentList, #[serde(default)] starred: ParentList, #[serde(default)] shared: ParentList }

fn days_in_month(year: i64, month: i64) -> i64 {
    match month { 2 => if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) { 29 } else { 28 }, 4 | 6 | 9 | 11 => 30, _ => 31 }
}
/// `Date.parse` of the canonical `YYYY-MM-DDTHH:MM:SS(.mmm)Z` form (what Drive and `toISOString` produce); `None`
/// leaves every other text, and every invalid date, to JavaScript.
fn iso_millis(text: &str) -> Option<f64> {
    let bytes = text.as_bytes();
    if bytes.len() != 20 && bytes.len() != 24 { return None; }
    let digits = |from: usize, to: usize| -> Option<i64> {
        bytes[from..to].iter().try_fold(0i64, |sum, &byte| byte.is_ascii_digit().then(|| sum * 10 + (byte - b'0') as i64))
    };
    if bytes[4] != b'-' || bytes[7] != b'-' || bytes[10] != b'T' || bytes[13] != b':' || bytes[16] != b':' || bytes[bytes.len() - 1] != b'Z' { return None; }
    let (year, month, day, hour, minute, second) = (digits(0, 4)?, digits(5, 7)?, digits(8, 10)?, digits(11, 13)?, digits(14, 16)?, digits(17, 19)?);
    let millis = if bytes.len() == 24 { if bytes[19] != b'.' { return None; } digits(20, 23)? } else { 0 };
    if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) || hour > 23 || minute > 59 || second > 59 { return None; }
    // days since 1970-01-01 in the proleptic Gregorian calendar
    let shifted = if month <= 2 { year - 1 } else { year };
    let era = shifted.div_euclid(400);
    let year_of_era = shifted - era * 400;
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146097 + day_of_era - 719468;
    Some(((days * 86400 + hour * 3600 + minute * 60 + second) * 1000 + millis) as f64)
}

/// Builds the table from the records of the last `merge`. Answers the positions whose timestamp text JavaScript
/// has to parse (`touch` them afterwards).
fn load(input: &[u8]) -> Result<Vec<u8>, String> {
    let LoadIn { fl, t, docs, views } = parse(input)?;
    let n = fl.len();
    let merged = MERGED.with(|cell| cell.borrow_mut().take());
    let Merged { ids, rids, parents } = match merged {
        Some(merged) if n > 0 => merged,
        _ if n == 0 => Merged { ids: Vec::new(), rids: Vec::new(), parents: Vec::new() }, // an empty catalogue needs no merge
        _ => return Err("Catalogue load without a merge".into()),
    };
    if t.len() != n || ids.len() != n { return Err("Inconsistent catalogue columns".into()); }
    let views = [views.recents.0, views.starred.0, views.shared.0];
    let mut table = Table { rank: vec![0; n], ..Table::default() };
    // Saved Home views historically resolve the first matching ID or remote alias: look up only the ids views name.
    let mut wanted: HashMap<&str, Option<u32>> = HashMap::default();
    for list in views.iter().flatten() { for name in list.iter().flatten() { wanted.insert(name, None); } }
    for index in 0..n {
        if fl[index] & TRASHED != 0 { continue; }
        let at = index as u32;
        table.active.push(at);
        if fl[index] & PDF != 0 && (fl[index] & PENDING != 0 || is_local(&ids[index])) { table.unsynced.push(at); }
        if !wanted.is_empty() {
            if let Some(found) = wanted.get_mut(ids[index].as_str()) { found.get_or_insert(at); }
            if !rids[index].is_empty() { if let Some(found) = wanted.get_mut(rids[index].as_str()) { found.get_or_insert(at); } }
        }
        for (slot, parent) in parents[index].iter().enumerate() {
            let Some(parent) = parent else { continue };
            if parents[index][..slot].iter().any(|earlier| earlier.as_ref() == Some(parent)) { continue; }
            match table.children.get_mut(parent.as_str()) {
                Some(list) => list.push(at),
                None => { table.children.insert(parent.clone(), vec![at]); }
            }
        }
    }
    table.views = std::array::from_fn(|slot| views[slot].as_ref().map(|list| list.iter().flatten().filter_map(|name| wanted.get(name.as_str()).copied().flatten()).collect()));
    drop(wanted);
    // `documents.get(id) || documentAliases.get(remoteId)`: the first document with a remote id wins an alias.
    let mut by_id: HashMap<&str, f64> = HashMap::default();
    let mut by_remote: HashMap<&str, f64> = HashMap::default();
    for doc in &docs {
        let opened = doc.opened.opened();
        by_id.insert(&doc.id, opened);
        if let Loose::Text(remote) = &doc.remote_id { if !remote.is_empty() { by_remote.entry(remote).or_insert(opened); } }
    }
    table.opened = ids.iter().enumerate().map(|(index, id)| if fl[index] & TRASHED != 0 { 0.0 } else { by_id.get(id.as_str()).or_else(|| by_remote.get(id.as_str())).copied().unwrap_or(0.0) }).collect();
    table.doc_by_id = by_id.into_iter().map(|(id, opened)| (id.to_string(), opened)).collect();
    table.doc_by_remote = by_remote.into_iter().map(|(id, opened)| (id.to_string(), opened)).collect();
    let mut unparsed = Vec::new();
    table.modified = t.iter().enumerate().map(|(index, stamp)| match stamp {
        Loose::Null => 0.0,
        Loose::Text(text) if text.is_empty() => 0.0,
        Loose::Text(text) => iso_millis(text).unwrap_or_else(|| { if fl[index] & TRASHED == 0 { unparsed.push(index as u32); } 0.0 }),
        Loose::Number(_) | Loose::Other => { if fl[index] & TRASHED == 0 { unparsed.push(index as u32); } 0.0 }
    }).collect();
    table.flags = fl;
    table.id = ids; table.rid = rids;
    TABLE.with(|cell| *cell.borrow_mut() = table);
    Ok(words(&unparsed))
}

#[derive(Deserialize)]
struct Touch(u32, f64, bool);
/// Observed records can change their timestamp or star before the next reload.
fn touch(input: &[u8]) -> Result<Vec<u8>, String> {
    let changes: Vec<Touch> = parse(input)?;
    TABLE.with(|cell| {
        let mut table = cell.borrow_mut();
        for Touch(index, modified, starred) in changes {
            let index = index as usize;
            if index >= table.len() { return Err("Catalogue index out of range".to_string()); }
            table.modified[index] = modified;
            if starred { table.flags[index] |= STARRED; } else { table.flags[index] &= !STARRED; }
        }
        Ok(Vec::new())
    })
}

/// Collation ranks of the folder names (equal rank == `localeCompare` returns 0), one per table row.
fn ranks(input: &[u8]) -> Result<Vec<u8>, String> {
    let values: Vec<u32> = parse(input)?;
    TABLE.with(|cell| {
        let mut table = cell.borrow_mut();
        if values.len() != table.len() { return Err("Inconsistent rank column".to_string()); }
        table.rank = values;
        Ok(Vec::new())
    })
}

/// Lower-cased names (JavaScript `toLowerCase`), one per table row, for the search filter.
fn names(input: &[u8]) -> Result<Vec<u8>, String> {
    let values: Vec<Option<String>> = parse(input)?;
    TABLE.with(|cell| {
        let mut table = cell.borrow_mut();
        if values.len() != table.len() { return Err("Inconsistent name column".to_string()); }
        table.lname = values;
        Ok(Vec::new())
    })
}

// ---- queries ----------------------------------------------------------------------------------------------------

#[derive(Deserialize)]
struct FolderIn { parent: String, needle: String, local: bool }

/// `folder(id, search)`: children of one directory, filtered by lower-case name, in `compareEntries` order.
/// With `local` only pending and `local-` records, which `combine` merges with the network listing.
fn folder(input: &[u8]) -> Result<Vec<u8>, String> {
    let FolderIn { parent, needle, local } = parse(input)?;
    TABLE.with(|cell| {
        let table = cell.borrow();
        if !needle.is_empty() && table.lname.len() != table.len() { return Err("Names are not loaded".to_string()); }
        let found: Vec<u32> = table.children.get(&parent).map(|list| list.as_slice()).unwrap_or(&[]).iter().copied().filter(|&i| {
            let at = i as usize;
            (needle.is_empty() || table.lname[at].as_deref().is_some_and(|name| name.contains(&needle)))
                && (!local || table.flags[at] & PENDING != 0 || is_local(&table.id[at]))
        }).collect();
        Ok(words(&table.sorted_entries(found)))
    })
}

#[derive(Deserialize)]
struct SortIn {
    /// rows before this one are local; later rows whose id matches a local id or remote id are dropped
    local: usize,
    i: Vec<String>,
    r: Vec<String>,
    k: Vec<String>,
    f: Vec<bool>,
    m: Vec<f64>,
    n: Vec<u32>,
}

/// `combine`'s final merge: local rows first, network rows minus the ones a local row already stands for, then
/// `compareEntries`. Rows are indexed in input order.
fn sort(input: &[u8]) -> Result<Vec<u8>, String> {
    let SortIn { local, i, r, k, f, m, n } = parse(input)?;
    let count = i.len();
    if local > count || [r.len(), k.len(), f.len(), m.len(), n.len()].iter().any(|&len| len != count) { return Err("Inconsistent row columns".into()); }
    let mut taken: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for row in 0..local { taken.insert(&i[row]); taken.insert(&r[row]); }
    let mut rows: Vec<(u32, EntryRow)> = (0..count).filter(|&row| row < local || !taken.contains(i[row].as_str()))
        .map(|row| (row as u32, EntryRow { folder: f[row], rank: n[row], modified: m[row], key: &k[row] })).collect();
    rows.sort_by(|a, b| compare_entries(&a.1, &b.1));
    Ok(words(&rows.iter().map(|row| row.0).collect::<Vec<_>>()))
}

fn dedupe_recents<'a>(candidates: impl IntoIterator<Item = Recent<'a>>) -> Vec<Recent<'a>> {
    let mut by_key: HashMap<&str, usize> = HashMap::default();
    let mut values: Vec<Recent> = Vec::new();
    for candidate in candidates {
        match by_key.get(candidate.key) {
            Some(&slot) => values[slot] = candidate,
            None => { by_key.insert(candidate.key, values.len()); values.push(candidate); }
        }
    }
    values
}

/// `recentItems`: the listed entries keyed by remote id or id, pending PDFs from the catalogue win, newest 12.
fn recent_items<'a>(table: &'a Table, listed: impl IntoIterator<Item = Recent<'a>>) -> Vec<u32> {
    let pending = table.unsynced.iter().map(|&i| table.recent(i));
    let mut rows = dedupe_recents(listed.into_iter().chain(pending));
    rows.sort_by(compare_recents);
    rows.truncate(RECENT_LIMIT);
    rows.into_iter().map(|row| row.at).collect()
}

#[derive(Deserialize)]
struct RecentsIn { i: Vec<String>, k: Vec<String>, m: Vec<f64> }

/// Foreign rows are numbered after the table rows (`table.len() + position`); their last opened time comes from
/// the document table, like the table rows'.
fn recents(input: &[u8]) -> Result<Vec<u8>, String> {
    let RecentsIn { i, k, m } = parse(input)?;
    if i.len() != k.len() || i.len() != m.len() { return Err("Inconsistent row columns".into()); }
    TABLE.with(|cell| {
        let table = cell.borrow();
        let base = table.len() as u32;
        let listed = (0..k.len()).map(|row| Recent { at: base + row as u32, key: &k[row], modified: m[row], opened: table.opened_of(&i[row]) });
        Ok(words(&recent_items(&table, listed)))
    })
}

#[derive(Deserialize)]
struct HomeIn { quick: Vec<u32> }

/// `home()`: recents, starred and shared; saved views keep their saved order, otherwise the newest entries.
fn home(input: &[u8]) -> Result<Vec<u8>, String> {
    let HomeIn { quick } = parse(input)?;
    TABLE.with(|cell| {
        let table = cell.borrow();
        let newest = |keep: &dyn Fn(u8) -> bool, limit: usize| {
            let mut found = table.sorted_recents(&table.active.iter().copied().filter(|&i| keep(table.flags[i as usize])).collect::<Vec<_>>());
            found.truncate(limit);
            found
        };
        let recents = match &table.views[0] {
            Some(saved) => saved.clone(),
            None => {
                let quick: std::collections::HashSet<u32> = quick.iter().copied().collect();
                let files: Vec<u32> = table.active.iter().copied().filter(|i| table.flags[*i as usize] & PDF != 0 && !quick.contains(i)).collect();
                let mut found = table.sorted_recents(&files);
                found.truncate(RECENT_LIMIT);
                found
            }
        };
        let recents = recent_items(&table, recents.iter().map(|&i| table.recent(i)));
        let starred = table.views[1].clone().unwrap_or_else(|| newest(&|flags| flags & STARRED != 0, RECENT_LIMIT));
        let shared = table.views[2].clone().unwrap_or_else(|| newest(&|flags| flags & NOT_OWNED != 0, SHARED_LIMIT));
        let mut out = words(&[recents.len() as u32, starred.len() as u32, shared.len() as u32]);
        out.extend(words(&recents)); out.extend(words(&starred)); out.extend(words(&shared));
        Ok(out)
    })
}

pub fn call(method: &str, input: &[u8]) -> Option<Result<Vec<u8>, String>> {
    Some(match method {
        "offline.library_merge" => merge(input),
        "offline.library_load" => load(input),
        "offline.library_touch" => touch(input),
        "offline.library_ranks" => ranks(input),
        "offline.library_names" => names(input),
        "offline.library_folder" => folder(input),
        "offline.library_sort" => sort(input),
        "offline.library_recents" => recents(input),
        "offline.library_home" => home(input),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn run(method: &str, value: Value) -> Vec<u8> { call(method, value.to_string().as_bytes()).unwrap().unwrap() }
    fn read(bytes: &[u8]) -> Vec<u32> { bytes.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect() }
    /// A record `(id, remoteId, flags, parents, lowercase name, seconds after the epoch, last opened)`.
    type Row<'a> = (&'a str, &'a str, u8, &'a [&'a str], Option<&'a str>, u32, f64);
    fn stamp(seconds: u32) -> String { format!("1970-01-01T00:{:02}:{:02}Z", seconds / 60, seconds % 60) }
    /// Merge then load a catalogue the way the JavaScript side does (nothing is aliased here).
    fn table(rows: &[Row], views: Value) {
        run("offline.library_merge", json!({ "i": rows.iter().map(|r| r.0).collect::<Vec<_>>(), "r": rows.iter().enumerate().filter(|(_, r)| !r.1.is_empty()).map(|(at, r)| json!([at, r.1])).collect::<Vec<_>>(),
            "p": [], "m": [], "a": rows.iter().map(|r| r.3).collect::<Vec<_>>(), "np": [], "root": "" }));
        run("offline.library_load", json!({ "fl": rows.iter().map(|r| r.2).collect::<Vec<_>>(), "t": rows.iter().map(|r| stamp(r.5)).collect::<Vec<_>>(),
            "docs": rows.iter().filter(|r| r.6 != 0.0).map(|r| json!({ "id": r.0, "lastOpenedAt": r.6 })).collect::<Vec<_>>(), "views": views }));
        run("offline.library_names", json!(rows.iter().map(|r| r.4).collect::<Vec<_>>()));
    }
    const NO_VIEWS: fn() -> Value = || json!({});
    fn merge_input(root: &str, items: Value) -> Value {
        let items = items.as_array().unwrap();
        let column = |name: &str| Value::Array(items.iter().map(|item| item[name].clone()).collect());
        let at = |name: &str, keep: &dyn Fn(&Value) -> bool| Value::Array(items.iter().enumerate().filter(|(_, item)| keep(&item[name])).map(|(at, _)| json!(at)).collect());
        json!({ "root": root, "i": column("i"),
            "r": items.iter().enumerate().filter(|(_, item)| item["r"] != "").map(|(at, item)| json!([at, item["r"]])).collect::<Vec<_>>(),
            "p": at("p", &|v| v == &json!(true)), "m": items.iter().enumerate().map(|(at, item)| json!([at, item["m"]])).collect::<Vec<_>>(),
            "a": column("a"), "np": at("h", &|v| v == &json!(false)) })
    }
    fn folder_of(parent: &str, needle: &str, local: bool) -> Vec<u32> { read(&run("offline.library_folder", json!({ "parent": parent, "needle": needle, "local": local }))) }

    #[test] fn merge_overlays_newer_remote_and_aliases_parents() {
        let out = read(&run("offline.library_merge", merge_input("drive-root", json!([
            { "i": "local-parent", "r": "remote-parent", "p": true, "m": 5.0, "a": ["drive-root"], "h": true },
            { "i": "remote-parent", "r": "", "p": false, "m": 9.0, "a": null, "h": false },
            { "i": "doc", "r": "", "p": false, "m": 1.0, "a": ["remote-parent", "other"], "h": true },
            { "i": "drive-root", "r": "", "p": false, "m": 0.0, "a": null, "h": false },
            { "i": "local-doc", "r": "remote-doc", "p": false, "m": 1.0, "a": ["x"], "h": true },
            { "i": "remote-doc", "r": "", "p": false, "m": 3.0, "a": ["y"], "h": true },
        ]))));
        // kept, patches, kept[], overlay[], patches[]
        assert_eq!(&out[..3], &[3, 2, 0]);
        // remote-parent, drive-root and remote-doc are aliases of local entries or the root: only local-parent, doc, local-doc stay
        assert_eq!(&out[3..6], &[0, 2, 4]);
        assert_eq!(&out[6..9], &[NONE, NONE, 5]);
        // local-parent's parent drive-root -> root; doc's remote-parent -> local-parent (item 0)
        assert_eq!(&out[9..], &[0, 0, NONE, 1, 0, 0]);
    }
    #[test] fn merge_ignores_pending_and_older_remote_copies() {
        let out = read(&run("offline.library_merge", merge_input("", json!([
            { "i": "a", "r": "b", "p": true, "m": 1.0, "a": null, "h": false },
            { "i": "b", "r": "", "p": false, "m": 99.0, "a": null, "h": false },
            { "i": "c", "r": "d", "p": false, "m": 50.0, "a": null, "h": false },
            { "i": "d", "r": "", "p": false, "m": 10.0, "a": null, "h": false },
        ]))));
        assert_eq!(&out[..3], &[2, 0, 0]);
        assert_eq!(&out[3..5], &[0, 2]);
        assert_eq!(&out[5..7], &[NONE, NONE]);
    }
    #[test] fn a_plain_catalogue_is_reported_as_the_identity() {
        let out = read(&run("offline.library_merge", merge_input("", json!([
            { "i": "a", "r": "", "p": false, "m": 0.0, "a": ["root", 5, null, ["x"]], "h": true },
            { "i": "b", "r": "", "p": false, "m": 0.0, "a": null, "h": false },
        ]))));
        assert_eq!(out, vec![2, 0, 1]);
    }
    #[test] fn merge_default_parent_is_root_alias() {
        let out = read(&run("offline.library_merge", merge_input("", json!([
            { "i": "folder", "r": "root", "p": false, "m": 0.0, "a": null, "h": false },
            { "i": "file", "r": "", "p": false, "m": 0.0, "a": null, "h": false },
        ]))));
        assert_eq!(out[1], 2); // both default 'root' parents resolve to the local alias owner
        assert_eq!(&out[out.len() - 6..], &[0, 0, 0, 1, 0, 0]);
    }
    #[test] fn an_overlay_without_its_own_parents_keeps_the_stored_ones_and_a_parent_free_overlay_does_not() {
        // remote copy newer: it has parents -> the aliased remote parents win; it has none (property missing) -> the stored list stays
        let with = read(&run("offline.library_merge", merge_input("", json!([
            { "i": "local", "r": "remote", "p": false, "m": 1.0, "a": ["old"], "h": true },
            { "i": "remote", "r": "", "p": false, "m": 2.0, "a": ["fresh"], "h": true },
            { "i": "old", "r": "x", "p": false, "m": 0.0, "a": null, "h": false },
            { "i": "fresh", "r": "", "p": false, "m": 0.0, "a": null, "h": false },
        ]))));
        let kept = with[0] as usize;
        assert_eq!(kept, 3); // local, old, fresh ('remote' is an alias target, dropped)
        let without = read(&run("offline.library_merge", merge_input("", json!([
            { "i": "local", "r": "remote", "p": false, "m": 1.0, "a": ["old"], "h": true },
            { "i": "remote", "r": "", "p": false, "m": 2.0, "a": null, "h": false },
            { "i": "old", "r": "x", "p": false, "m": 0.0, "a": null, "h": false },
        ]))));
        // 'old' is an alias of nothing; but 'remote' as overlay without own parents leaves ["old"] -> 'old' is an alias key of item 2? it is the id of item 2, not a remoteId: no patch
        assert_eq!(&without[..2], &[2, 0]);
    }
    #[test] fn merge_then_load_uses_the_aliased_parents() {
        run("offline.library_merge", merge_input("", json!([
            { "i": "local-parent", "r": "remote-parent", "p": true, "m": 5.0, "a": [], "h": true },
            { "i": "child", "r": "", "p": false, "m": 1.0, "a": ["remote-parent"], "h": true },
        ])));
        run("offline.library_load", json!({ "fl": [FOLDER, PDF], "t": [null, null], "docs": [], "views": {} }));
        assert_eq!(folder_of("local-parent", "", false), vec![1]);
        assert!(folder_of("remote-parent", "", false).is_empty());
    }
    #[test] fn load_requires_the_merge_it_follows() {
        let reset = json!({ "fl": [], "t": [], "docs": [], "views": {} });
        run("offline.library_load", reset.clone()); // an empty catalogue needs no merge
        assert!(call("offline.library_load", br#"{"fl":[0],"t":[null],"docs":[],"views":{}}"#).unwrap().is_err());
    }
    #[test] fn folders_sort_by_rank_then_files_newest_first_then_key() {
        table(&[("z", "", FOLDER, &["root"], Some("zeta"), 0, 0.0), ("a", "", FOLDER, &["root"], Some("alpha"), 0, 0.0),
            ("old", "", PDF, &["root"], Some("old"), 1, 0.0), ("new", "", PDF, &["root"], Some("new"), 9, 0.0),
            ("tie-b", "", PDF, &["root"], Some("b"), 5, 0.0), ("tie-a", "", PDF, &["root"], Some("a"), 5, 0.0),
            ("gone", "", PDF | TRASHED, &["root"], Some("gone"), 59, 0.0)], NO_VIEWS());
        run("offline.library_ranks", json!([1, 0, 0, 0, 0, 0, 0]));
        assert_eq!(folder_of("root", "", false), vec![1, 0, 3, 5, 4, 2]);
        assert_eq!(folder_of("root", "ph", false), vec![1]);
        assert!(folder_of("nowhere", "", false).is_empty());
    }
    #[test] fn equal_ranks_fall_back_to_the_key_and_the_sort_is_stable() {
        table(&[("b", "", FOLDER, &["root"], Some("x"), 0, 0.0), ("a", "", FOLDER, &["root"], Some("X"), 0, 0.0)], NO_VIEWS());
        run("offline.library_ranks", json!([0, 0]));
        assert_eq!(folder_of("root", "", false), vec![1, 0]);
        table(&[("k", "", PDF, &["root"], None, 1, 0.0), ("k", "", PDF, &["root"], None, 1, 0.0)], NO_VIEWS());
        assert_eq!(folder_of("root", "", false), vec![0, 1]);
    }
    #[test] fn searching_needs_the_names() {
        run("offline.library_merge", merge_input("", json!([{ "i": "a", "r": "", "p": false, "m": 0.0, "a": ["root"], "h": true }])));
        run("offline.library_load", json!({ "fl": [PDF], "t": [null], "docs": [], "views": {} }));
        assert_eq!(folder_of("root", "", false), vec![0]);
        assert!(call("offline.library_folder", br#"{"parent":"root","needle":"a","local":false}"#).unwrap().is_err());
    }
    #[test] fn keys_compare_in_utf16_order() {
        // U+FF5E (BMP, above the surrogate range) sorts after U+1F600 (surrogate pair D83D) in JavaScript, before it in UTF-8
        assert_eq!(cmp_keys("\u{FF5E}", "\u{1F600}"), Ordering::Greater);
        assert_eq!(cmp_keys("a", "b"), Ordering::Less);
        assert_eq!(cmp_keys("same", "same"), Ordering::Equal);
    }
    #[test] fn a_parent_listed_twice_indexes_the_child_once() {
        table(&[("c", "", PDF, &["p", "p", "q"], None, 0, 0.0)], NO_VIEWS());
        assert_eq!(folder_of("p", "", false), vec![0]);
        assert_eq!(folder_of("q", "", false), vec![0]);
    }
    #[test] fn local_filter_keeps_pending_and_local_ids() {
        table(&[("local-1", "", PDF, &["root"], None, 0, 0.0), ("p", "", PDF | PENDING, &["root"], None, 0, 0.0), ("r", "", PDF, &["root"], None, 0, 0.0)], NO_VIEWS());
        let mut found = folder_of("root", "", true);
        found.sort();
        assert_eq!(found, vec![0, 1]);
    }
    #[test] fn touch_changes_timestamp_and_star() {
        table(&[("a", "", PDF | STARRED, &["root"], None, 1, 0.0), ("b", "", PDF, &["root"], None, 2, 0.0)], NO_VIEWS());
        assert_eq!(folder_of("root", "", false), vec![1, 0]);
        run("offline.library_touch", json!([[0, 5000.0, false]]));
        assert_eq!(folder_of("root", "", false), vec![0, 1]);
        assert!(read(&run("offline.library_home", json!({ "quick": [] })))[1] == 0, "unstarred");
        assert!(call("offline.library_touch", b"[[9,1,true]]").unwrap().is_err());
    }
    #[test] fn sort_drops_remote_rows_a_local_row_stands_for() {
        let out = read(&run("offline.library_sort", json!({ "local": 1, "i": ["local-x", "dup", "keep", "r"], "r": ["dup", "", "", ""],
            "k": ["dup", "dup", "keep", "r"], "f": [false, false, false, false], "m": [1.0, 1.0, 3.0, 2.0], "n": [0, 0, 0, 0] })));
        assert_eq!(out, vec![2, 3, 0]);
    }
    #[test] fn home_defaults_saved_views_and_recents_order() {
        table(&[("a", "", PDF, &["root"], None, 10, 200.0), ("b", "", PDF, &["root"], None, 20, 100.0), ("c", "", PDF | STARRED, &["root"], None, 5, 0.0),
            ("local", "", PDF | PENDING, &["root"], None, 1, 0.0), ("s", "", PDF | NOT_OWNED, &["root"], None, 1, 0.0), ("quick", "", PDF, &["root"], None, 59, 999.0),
            ("gone", "", PDF | TRASHED, &["root"], None, 59, 999.0)], NO_VIEWS());
        let out = read(&run("offline.library_home", json!({ "quick": [5] })));
        assert_eq!(&out[..3], &[5, 1, 1]);
        // opened desc (a 200, b 100), then newest (c 5, s 1, local 1: tie by key 'local' < 's'), the pending PDF is kept
        assert_eq!(&out[3..8], &[0, 1, 2, 3, 4]);
        assert_eq!(&out[8..], &[2, 4]);
    }
    #[test] fn saved_views_keep_their_order_resolve_aliases_and_skip_missing() {
        table(&[("local-doc", "remote-doc", PDF, &["root"], None, 1, 0.0), ("second", "", PDF, &["root"], None, 1, 0.0), ("removed", "", PDF | TRASHED, &["root"], None, 1, 0.0), ("dir", "", FOLDER, &["root"], None, 0, 0.0)],
            json!({ "recents": ["second", "remote-doc", "removed", "unknown", 7], "starred": ["dir"], "shared": [] }));
        let out = read(&run("offline.library_home", json!({ "quick": [] })));
        assert_eq!(&out[..3], &[2, 1, 0]);
        assert_eq!(&out[3..], &[0, 1, 3]); // recents re-sorted by key: local doc ('remote-doc') before 'second'
    }
    #[test] fn recents_merge_foreign_rows_by_key_and_prefer_pending_catalogue_entries() {
        table(&[("local-1", "remote-1", PDF | PENDING, &["root"], None, 1, 0.0), ("plain", "", PDF, &["root"], None, 1, 0.0)], NO_VIEWS());
        // foreign rows 2..: the network copy of remote-1 loses to the catalogue entry
        let out = read(&run("offline.library_recents", json!({ "i": ["remote-1", "x", "y"], "k": ["remote-1", "x", "y"], "m": [50000.0, 7000.0, 7000.0] })));
        assert_eq!(out, vec![3, 4, 0]);
        let many = (0..20).map(|i| (19 - i) as f64).collect::<Vec<_>>();
        let keys = (0..20).map(|i| format!("k{i:02}")).collect::<Vec<_>>();
        let out = read(&run("offline.library_recents", json!({ "i": keys, "k": keys, "m": many })));
        assert_eq!(out.len(), RECENT_LIMIT);
    }
    #[test] fn foreign_rows_use_documents_by_id_then_by_remote_id() {
        run("offline.library_merge", merge_input("", json!([])));
        run("offline.library_load", json!({ "fl": [], "t": [], "docs": [{ "id": "d1", "remoteId": "shared", "lastOpenedAt": 100 }, { "id": "d2", "remoteId": "shared", "lastOpenedAt": 300 }, { "id": "d3", "lastOpenedAt": 0 }], "views": {} }));
        // 'shared' resolves to the first document with that remote id (100), 'd3' was never opened (falls to modified)
        let out = read(&run("offline.library_recents", json!({ "i": ["shared", "d2", "d3", "none"], "k": ["a", "b", "c", "d"], "m": [0.0, 0.0, 9.0, 1.0] })));
        assert_eq!(out, vec![1, 0, 2, 3]);
    }
    #[test] fn canonical_timestamps_parse_like_date_parse() {
        for (text, expected) in [("1970-01-01T00:00:00Z", 0.0), ("2026-10-01T00:00:00.000Z", 1_790_812_800_000.0), ("2000-02-29T23:59:59.999Z", 951_868_799_999.0),
            ("1969-12-31T23:59:59.999Z", -1.0), ("0000-01-01T00:00:00Z", -62_167_219_200_000.0), ("9999-12-31T23:59:59.999Z", 253_402_300_799_999.0)] {
            assert_eq!(iso_millis(text), Some(expected), "{text}");
        }
        // everything else, and every invalid date, is JavaScript's call
        for text in ["", "2026-10-01", "2026-10-01T00:00:00", "2026-10-01T00:00:00+02:00", "2026-02-29T00:00:00Z", "2026-13-01T00:00:00Z", "2026-10-01T24:00:00Z", "2026-10-01T00:00:60Z",
            "2026-10-01T00:00:00.1Z", "2026-10-01t00:00:00z", "Wed, 03 Oct 2026 00:00:00 GMT", "２０２６-10-01T00:00:00Z", "2026-10-01T00:00:00.000X"] {
            assert_eq!(iso_millis(text), None, "{text}");
        }
    }
    #[test] fn load_reports_timestamps_javascript_must_parse() {
        run("offline.library_merge", merge_input("", json!([{ "i": "a", "r": "", "p": false, "m": 0.0, "a": ["root"], "h": true }, { "i": "b", "r": "", "p": false, "m": 0.0, "a": ["root"], "h": true },
            { "i": "c", "r": "", "p": false, "m": 0.0, "a": ["root"], "h": true }])));
        let pending = read(&run("offline.library_load", json!({ "fl": [PDF, PDF, PDF | TRASHED], "t": ["2026-10-01", "2026-10-01T00:00:00Z", "junk"], "docs": [], "views": {} })));
        assert_eq!(pending, vec![0]); // trashed records are never read, so never reported
    }
    #[test] fn odd_views_and_document_fields_are_tolerated() {
        run("offline.library_merge", merge_input("", json!([{ "i": "a", "r": "", "p": false, "m": 0.0, "a": ["root"], "h": true }])));
        run("offline.library_load", json!({ "fl": [PDF], "t": ["junk"], "views": { "recents": "not-an-array", "starred": null, "shared": [["x"], null, "a"], "other": 1 },
            "docs": [{ "id": "a", "lastOpenedAt": "7" }, { "id": "b", "remoteId": 5, "lastOpenedAt": { "x": 1 } }, { "id": "c" }] }));
        let out = read(&run("offline.library_home", json!({ "quick": [] })));
        assert_eq!(&out[..3], &[1, 0, 1]); // recents falls back to the newest PDF, no entry is starred, the shared view lists a
    }
    #[test] fn rejects_inconsistent_columns() {
        assert!(call("offline.library_sort", br#"{"local":3,"i":[],"r":[],"k":[],"f":[],"m":[],"n":[]}"#).unwrap().is_err());
        assert!(call("offline.library_merge", b"not json").unwrap().is_err());
        assert!(call("offline.library_merge", br#"{"i":["a"],"r":[[5,"x"]],"p":[],"m":[],"a":[],"np":[],"root":""}"#).unwrap().is_err());
        assert!(call("offline.library_ranks", b"[1,2]").unwrap().is_err());
    }
    #[test] fn empty_table_answers_empty_lists() {
        table(&[], NO_VIEWS());
        assert_eq!(read(&run("offline.library_home", json!({ "quick": [] }))), vec![0, 0, 0]);
        assert!(folder_of("root", "", false).is_empty());
    }
}
