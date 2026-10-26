//! Live snapshot cache and delta planner, ported from live-collaboration-v5.js
//! (ihnSerializeSnapshotPage, ihnRememberLiveSnapshot, ihnBuildPeerSnapshot).
//! Every page is serialised once with the stable stringifier and kept as text; a peer that
//! acknowledged an older cached snapshot receives only the pages whose text changed.
use super::json::{parse, put_ascii, str_or_empty, to_js_string, units_to_string, write_str, write_stable, Pool, J};
use std::cell::RefCell;
use std::collections::HashMap;

struct Page { bytes: Box<[u8]>, len16: usize }

/// One remembered snapshot. `slots[i]` is the entry of `pages` for page index `i`; a repeated
/// page id shares one entry (the last page wins) like the Map the JavaScript used.
struct Record { hash: Vec<u16>, slots: Vec<usize>, by_id: HashMap<Vec<u16>, usize>, pages: Vec<Page>, page_chars: usize }

#[derive(Default)]
struct Cache { records: Vec<Record> }

thread_local! { static CACHE: RefCell<Cache> = RefCell::new(Cache::default()); }

fn page_id(pool: &Pool, page: &J, index: usize) -> Vec<u16> {
    match page.get(pool, "pageId") {
        Some(id) if id.truthy(pool) => to_js_string(pool, id),
        _ => format!("legacy-page-{}", index + 1).encode_utf16().collect(),
    }
}

/// Input: `<limit>,<maxChars>\n<snapshot JSON>`. Returns "1" when remembered, "0" when the
/// snapshot has no contentHash or no pages array (the JS returned null).
pub fn remember(input: &[u8]) -> Result<Vec<u8>, String> {
    let split = input.iter().position(|&b| b == b'\n').ok_or("Missing snapshot cache header")?;
    let header = std::str::from_utf8(&input[..split]).map_err(|_| "Bad snapshot cache header")?;
    let (limit, max_chars) = header.split_once(',').ok_or("Bad snapshot cache header")?;
    let limit: usize = limit.trim().parse().map_err(|_| "Bad snapshot cache limit")?;
    let max_chars: f64 = max_chars.trim().parse().map_err(|_| "Bad snapshot cache size")?;
    let mut pool = Pool::default();
    let root = parse(&mut pool, &input[split + 1..])?;
    let pool = &pool;
    let hash = str_or_empty(pool, root.get(pool, "contentHash"));
    let Some(J::Arr(pages)) = root.get(pool, "pages") else { return Ok(b"0".to_vec()); };
    if hash.is_empty() { return Ok(b"0".to_vec()); }
    let mut record = Record { hash, slots: Vec::with_capacity(pages.len()), by_id: HashMap::new(), pages: Vec::new(), page_chars: 0 };
    let mut units: Vec<u16> = Vec::new();
    for (index, page) in pages.iter().enumerate() {
        units.clear();
        write_stable(&mut units, pool, page);
        let entry = Page { bytes: units_to_string(&units)?.into_bytes().into_boxed_slice(), len16: units.len() };
        record.page_chars += entry.len16;
        let slot = match record.by_id.get(&page_id(pool, page, index)) {
            Some(&slot) => { record.pages[slot] = entry; slot }
            None => { record.pages.push(entry); let slot = record.pages.len() - 1; record.by_id.insert(page_id(pool, page, index), slot); slot }
        };
        record.slots.push(slot);
    }
    CACHE.with(|cache| {
        let records = &mut cache.borrow_mut().records;
        records.retain(|r| r.hash != record.hash);
        records.push(record);
        // Same eviction as the JS: oldest first, by count, then by total characters while more than one remains.
        while records.len() > limit || (records.len() > 1 && records.iter().map(|r| r.page_chars as f64).sum::<f64>() > max_chars) {
            records.remove(0);
        }
    });
    Ok(b"1".to_vec())
}

pub fn reset() -> Result<Vec<u8>, String> { CACHE.with(|cache| cache.borrow_mut().records = Vec::new()); Ok(Vec::new()) }

/// Input: `{"hash":"..","ids":[..],"bases":[..],"force":bool}` where `ids` are the page ids of the
/// snapshot in order and `bases` the candidate base hashes (already filtered to non-empty and not
/// equal to the snapshot hash). Output JSON: `["missing"]` when the snapshot is not cached,
/// `["full"]` to send the whole snapshot, or `["delta","<baseHash>",[changed page indices]]`.
pub fn delta(input: &[u8]) -> Result<Vec<u8>, String> {
    let mut pool = Pool::default();
    let root = parse(&mut pool, input)?;
    let pool = &pool;
    let hash = str_or_empty(pool, root.get(pool, "hash"));
    let ids: Vec<Vec<u16>> = match root.get(pool, "ids") { Some(J::Arr(items)) => items.iter().map(|v| to_js_string(pool, v)).collect(), _ => Vec::new() };
    let bases: Vec<Vec<u16>> = match root.get(pool, "bases") { Some(J::Arr(items)) => items.iter().map(|v| to_js_string(pool, v)).collect(), _ => Vec::new() };
    let force = root.get(pool, "force").is_some_and(|v| v.truthy(pool));
    let plan = CACHE.with(|cache| {
        let cache = cache.borrow();
        let Some(current) = cache.records.iter().find(|r| r.hash == hash) else { return Plan::Missing; };
        if force { return Plan::Full; }
        let Some(base) = bases.iter().find_map(|b| cache.records.iter().find(|r| !b.is_empty() && r.hash == *b)) else { return Plan::Full; };
        // A snapshot whose page list differs from the cached one (same hash, different pages) is sent whole.
        if ids.len() != current.slots.len() || ids.iter().zip(&current.slots).any(|(id, slot)| current.by_id.get(id) != Some(slot)) { return Plan::Full; }
        let mut changed: Vec<usize> = Vec::new();
        let mut changed_chars = 0usize;
        for (index, id) in ids.iter().enumerate() {
            let page = &current.pages[current.slots[index]];
            let same = base.by_id.get(id).is_some_and(|&slot| base.pages[slot].bytes == page.bytes);
            if same { continue; }
            changed.push(index);
            changed_chars += page.len16;
        }
        if changed.len() == ids.len() || changed_chars as f64 + 2048.0 >= (current.page_chars.max(1) as f64) * 0.8 { return Plan::Full; }
        Plan::Delta(base.hash.clone(), changed)
    });
    let mut out: Vec<u16> = Vec::new();
    match plan {
        Plan::Missing => put_ascii(&mut out, "[\"missing\"]"),
        Plan::Full => put_ascii(&mut out, "[\"full\"]"),
        Plan::Delta(base_hash, changed) => {
            put_ascii(&mut out, "[\"delta\",");
            write_str(&mut out, &base_hash);
            put_ascii(&mut out, ",[");
            for (i, index) in changed.iter().enumerate() { if i > 0 { out.push(0x2c); } put_ascii(&mut out, &index.to_string()); }
            put_ascii(&mut out, "]]");
        }
    }
    Ok(units_to_string(&out)?.into_bytes())
}

enum Plan { Missing, Full, Delta(Vec<u16>, Vec<usize>) }
