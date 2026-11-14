//! The 5.x version timeline: the `IH_VERSIONS` archive (schema 3: checkpoints and deltas with integrity hashes,
//! timeline-core-v5.js) and the legacy array of full snapshots, normalised as `normalizeTimelineEntry` does.
//! Pruning and the conversion to v6 versions belong to `ihn-core::history`.

use super::json::{self, js_slice, Value};
use super::pages::{self, V5Page};
use super::{simple_hash, to_base36};

pub const ARCHIVE_SCHEMA_VERSION: f64 = 3.0;
pub const MAX_ARCHIVE_ENTRIES: usize = 200;
pub const MAX_ARCHIVE_PAGES: usize = 2000;
const MAX_PAGE_ID_LENGTH: usize = 256;

#[derive(Clone, Debug, PartialEq, Default)]
pub struct V5Version {
    pub id: String,
    pub origin_id: String,
    pub ts: f64,
    pub author_email: String,
    pub author_name: String,
    pub author_photo: String,
    pub summary: String,
    pub kind: String,
    pub is_milestone: bool,
    pub parent_id: Option<String>,
    pub content_hash: String,
    pub device_id: String,
    pub export_name: String,
    pub pages: Vec<V5Page>,
}

/// timeline-core `hashString`: FNV-1a 32 over each UTF-16 unit as two bytes (low, high), base 36 padded to 7.
pub fn hash_string(s: &str) -> String {
    let mut h = 0x811c_9dc5u32;
    for u in s.encode_utf16() {
        h = (h ^ (u & 0xff) as u32).wrapping_mul(0x0100_0193);
        h = (h ^ (u >> 8) as u32).wrapping_mul(0x0100_0193);
    }
    format!("{:0>7}", to_base36(h as u64))
}

struct Snapshot {
    pages: Vec<Value>,
    calendar: Value,
    export_name: String,
}

fn null_if_undef(v: &Value) -> Value {
    match v {
        Value::Undef => Value::Null,
        v => v.clone(),
    }
}

impl Snapshot {
    fn hash(&self) -> String {
        let mut text = String::from("{\"calendarPageConfig\":");
        text.push_str(&json::stable(&self.calendar));
        text.push_str(",\"exportName\":");
        json::push_str(&mut text, &self.export_name);
        text.push_str(",\"pages\":[");
        for (i, p) in self.pages.iter().enumerate() {
            if i > 0 {
                text.push(',');
            }
            text.push_str(&json::stable(p));
        }
        text.push_str("]}");
        hash_string(&text)
    }
}

fn page_identity(page: &Value) -> Option<&str> { page.get("pageId").as_str().filter(|id| !id.is_empty() && id.encode_utf16().count() <= MAX_PAGE_ID_LENGTH) }

fn valid_id(v: &Value) -> Option<&str> { v.as_str().filter(|id| id.encode_utf16().count() <= MAX_PAGE_ID_LENGTH) }

fn apply_delta(prev: &Snapshot, delta: &Value) -> Option<Snapshot> {
    if !matches!(delta, Value::Obj(_)) {
        return None;
    }
    // Insertion-ordered map id → page, as the JavaScript `Map`.
    let mut map: Vec<(String, Value)> = Vec::with_capacity(prev.pages.len());
    for p in &prev.pages {
        let id = page_identity(p)?;
        if map.iter().any(|(i, _)| i == id) {
            return None;
        }
        map.push((id.to_string(), p.clone()));
    }
    let prev_order: Vec<String> = map.iter().map(|(i, _)| i.clone()).collect();
    match delta.get("removed") {
        Value::Undef => {}
        Value::Arr(list) if list.len() <= MAX_ARCHIVE_PAGES => {
            for id in list {
                let id = valid_id(id)?;
                map.retain(|(i, _)| i != id);
            }
        }
        _ => return None,
    }
    match delta.get("upserts") {
        Value::Undef => {}
        Value::Arr(list) if list.len() <= MAX_ARCHIVE_PAGES => {
            for pair in list {
                let [id, page] = pair.arr() else { return None };
                let id = valid_id(id).filter(|id| page_identity(page) == Some(id))?;
                match map.iter_mut().find(|(i, _)| i == id) {
                    Some(slot) => slot.1 = page.clone(),
                    None => map.push((id.to_string(), page.clone())),
                }
            }
        }
        _ => return None,
    }
    let order: Vec<String> = match delta.get("order") {
        Value::Undef => {
            let mut order: Vec<String> = prev_order.into_iter().filter(|id| map.iter().any(|(i, _)| i == id)).collect();
            for (id, _) in &map {
                if !order.contains(id) {
                    order.push(id.clone());
                }
            }
            order
        }
        Value::Arr(list) if list.len() <= MAX_ARCHIVE_PAGES => {
            let mut order: Vec<String> = Vec::with_capacity(list.len());
            for id in list {
                let id = valid_id(id).filter(|id| !order.iter().any(|o| o == id) && map.iter().any(|(i, _)| i == id))?;
                order.push(id.to_string());
            }
            if order.len() != map.len() {
                return None;
            }
            order
        }
        _ => return None,
    };
    let patch = delta.get("document");
    let patch = if matches!(patch, Value::Obj(_)) { patch } else { &Value::Undef };
    let mut take = |id: &String| map.iter_mut().find(|(i, _)| i == id).map(|(_, p)| std::mem::take(p));
    Some(Snapshot {
        pages: order.iter().filter_map(&mut take).collect(),
        calendar: if patch.has("calendarPageConfig") { null_if_undef(patch.get("calendarPageConfig")) } else { prev.calendar.clone() },
        export_name: if patch.has("exportName") { js_slice(&patch.get("exportName").text(), 220).to_string() } else { prev.export_name.clone() },
    })
}

/// One materialised entry: its metadata object and its snapshot.
pub struct Materialized<'a> {
    pub meta: &'a Value,
    pub export_name: String,
    pub pages: Vec<Value>,
}

/// timeline-core `materializeArchive`. `Ok(None)` = not an archive this version reads (5.18.0 returns `null`);
/// `Err` = the archive is damaged (5.18.0 throws; the timeline then stays empty).
pub fn materialize(value: &Value) -> Result<Option<Vec<Materialized<'_>>>, &'static str> {
    if let Value::Arr(list) = value {
        // Legacy format: full snapshots, accepted as they are.
        return Ok(Some(list.iter().map(|e| Materialized { meta: e, export_name: e.get("exportName").text(), pages: e.get("pages").arr().to_vec() }).collect()));
    }
    if !matches!(value, Value::Obj(_)) || value.get("schema").as_str() != Some("inhouse-timeline") || value.get("schemaVersion").num() != ARCHIVE_SCHEMA_VERSION {
        return Ok(None);
    }
    let Value::Arr(records) = value.get("entries") else { return Err("Invalid timeline archive length") };
    if records.len() > MAX_ARCHIVE_ENTRIES {
        return Err("Invalid timeline archive length");
    }
    let mut out = Vec::with_capacity(records.len());
    let mut current: Option<Snapshot> = None;
    let mut current_hash = String::new();
    for record in records {
        let kind = record.get("type").as_str();
        let hash = record.get("hash").as_str().filter(|h| h.encode_utf16().count() <= 64);
        let (Value::Obj(_), Some("checkpoint" | "delta"), Some(hash)) = (record, kind, hash) else { return Err("Invalid timeline record") };
        let next = if kind == Some("checkpoint") {
            let snap = record.get("snapshot");
            if !matches!(snap, Value::Obj(_)) {
                return Err("Invalid timeline checkpoint");
            }
            let s = Snapshot { pages: snap.get("pages").arr().to_vec(), calendar: null_if_undef(snap.get("calendarPageConfig")), export_name: js_slice(&snap.get("exportName").text(), 220).to_string() };
            if s.pages.len() > MAX_ARCHIVE_PAGES {
                return Err("Timeline checkpoint has too many pages");
            }
            s
        } else {
            let base = current.as_ref().filter(|_| record.get("baseHash").as_str() == Some(current_hash.as_str())).ok_or("Timeline delta chain is broken")?;
            apply_delta(base, record.get("delta")).ok_or("Invalid timeline delta")?
        };
        current_hash = next.hash();
        if current_hash != hash {
            return Err("Timeline integrity check failed");
        }
        out.push(Materialized { meta: record.get("meta"), export_name: next.export_name.clone(), pages: next.pages.clone() });
        current = Some(next);
    }
    Ok(Some(out))
}

fn obj(fields: Vec<(&str, Value)>) -> Value { Value::Obj(fields.into_iter().map(|(k, v)| (k.into(), v)).collect()) }

/// `buildTimelineSnapshotFromPages` for one page, as a value in 5.18.0's key order (hashed by `contentHash`).
fn snapshot_page(page: &Value, index: usize) -> Value {
    let p = |k: &str| page.get(k);
    let source = if p("backgroundSource").truthy() { p("backgroundSource").clone() } else { Value::Str(if p("pdfPageIndex").truthy() { "pdf" } else { "template" }.into()) };
    let portable = source.as_str() == Some("custom") && p("backgroundImage").as_str().is_some_and(|s| !s.starts_with("blob:"));
    let arr = |v: &Value| if v.is_arr() { v.clone() } else { Value::Arr(Vec::new()) };
    let or_null = |v: &Value| if v.truthy() { v.clone() } else { Value::Null };
    let dim = |v: &Value, d: u32| Value::Num(pages::dimension(v.finite(), d) as f64);
    obj(vec![
        ("pageId", if p("pageId").truthy() { p("pageId").clone() } else { Value::Str(format!("legacy-page-{}", index + 1).into()) }),
        ("strokes", arr(p("strokes"))),
        ("images", arr(p("images"))),
        ("deletedStrokeIds", arr(p("deletedStrokeIds"))),
        ("deletedStrokeStamps", if p("deletedStrokeStamps").truthy() { p("deletedStrokeStamps").clone() } else { Value::Obj(Vec::new()) }),
        ("backgroundImage", if portable { p("backgroundImage").clone() } else { Value::Null }),
        ("backgroundSource", source),
        ("templateKind", Value::Str(pages::template_kind(p("templateKind")).into())),
        ("pdfPageIndex", p("pdfPageIndex").finite().map_or(Value::Null, Value::Num)),
        ("pageWidth", dim(p("pageWidth"), pages::A4.0)),
        ("pageHeight", dim(p("pageHeight"), pages::A4.1)),
        ("sidePanel", or_null(p("sidePanel"))),
        ("legacyCoverStrokes", or_null(p("legacyCoverStrokes"))),
    ])
}

/// `timelineSnapshotHash(pages, exportName)`.
pub fn content_hash(pages: &[Value], export_name: &str) -> String {
    let text = json::stringify(&obj(vec![("pages", Value::Arr(pages.to_vec())), ("calendarPageConfig", Value::Null), ("exportName", Value::Str(export_name.into()))]));
    format!("{}:{}", simple_hash(&text), to_base36(text.encode_utf16().count() as u64))
}

const MILESTONE_KINDS: [&str; 5] = ["baseline", "restore", "revert-original", "pre-restore", "manual"];

/// The id without a legacy branch suffix: `/^(.*)~[a-z0-9]+(?:_\d+)?$/i`.
fn origin_of(id: &str) -> &str {
    let alnum = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric());
    let suffix_ok = |s: &str| alnum(s) || s.rsplit_once('_').is_some_and(|(a, d)| alnum(a) && !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()));
    // Greedy `(.*)`: the last `~` whose tail matches.
    id.rmatch_indices('~').find(|(i, _)| suffix_ok(&id[i + 1..])).map_or(id, |(i, _)| &id[..i])
}

/// `normalizeTimelineEntry(rawEntry, index)`; `None` when the entry has no pages. `now_ms` replaces an invalid `ts`
/// (5.18.0 uses `Date.now()`; importers pass the revision time so the result is deterministic).
pub fn normalize(m: &Materialized, index: usize, now_ms: f64) -> Option<V5Version> {
    let raw = m.meta;
    if !raw.is_object() || m.pages.is_empty() {
        return None;
    }
    let cut = |v: &Value, n: usize| js_slice(&v.text(), n).to_string();
    let ts = Some(raw.get("ts").num()).filter(|t| t.is_finite()).map_or(now_ms, |t| t.max(0.0));
    let export_name = js_slice(&m.export_name, 220).to_string();
    let snapshot: Vec<Value> = m.pages.iter().enumerate().map(|(i, p)| snapshot_page(p, i)).collect();
    let content_hash = if raw.get("contentHash").truthy() { raw.get("contentHash").text() } else { content_hash(&snapshot, &export_name) };
    let kind = if raw.get("kind").truthy() { cut(raw.get("kind"), 48) } else { "autosave".into() };
    let id = if raw.get("id").truthy() { cut(raw.get("id"), 160) } else { js_slice(&format!("recovered_{}_{content_hash}_{index}", to_base36(ts as u64)), 160).to_string() };
    let origin_id = if raw.get("originId").truthy() { cut(raw.get("originId"), 160) } else { js_slice(origin_of(&id), 160).to_string() };
    let author = raw.get("author");
    let email = cut(author.get("email"), 256);
    let name = [author.get("name"), author.get("email")].into_iter().find(|v| v.truthy()).map_or("Unknown editor".to_string(), |v| cut(v, 160));
    Some(V5Version {
        is_milestone: raw.get("isMilestone").truthy() || MILESTONE_KINDS.contains(&kind.as_str()),
        id,
        origin_id,
        ts,
        author_email: email,
        author_name: name,
        author_photo: cut(author.get("photo"), 1024),
        summary: if raw.get("summary").truthy() { cut(raw.get("summary"), 320) } else { "Saved".into() },
        kind,
        parent_id: raw.get("parentId").truthy().then(|| cut(raw.get("parentId"), 160)),
        content_hash,
        device_id: cut(raw.get("deviceId"), 96),
        export_name,
        pages: snapshot.iter().map(pages::page).collect::<Result<_, _>>().ok()?,
    })
}

/// A parsed archive (or legacy array) → normalised entries in archive order. A damaged or unknown archive gives an
/// empty timeline, as in 5.18.0.
pub fn read(archive: &Value, now_ms: f64) -> Vec<V5Version> {
    match materialize(archive) {
        Ok(Some(entries)) => entries.iter().enumerate().filter_map(|(i, m)| normalize(m, i, now_ms)).collect(),
        _ => Vec::new(),
    }
}

/// The JSON text of an archive (an inflated `IH_VERSIONS` section, or `historyArchiveJson` of a stored row).
pub fn read_json(text: &[u8], now_ms: f64) -> Vec<V5Version> { json::parse(text).map_or_else(Vec::new, |v| read(&v, now_ms)) }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v5::json::parse;

    #[test]
    fn hash_string_matches_timeline_core() {
        // tests/timeline-core-v5.test.mjs pins hashString(stableStringify(snapshot)) of its fixture; these are
        // direct values of the same function.
        assert_eq!(hash_string(""), "0ztntfp");
        assert_eq!(hash_string("abc"), "1cb8by5");
        assert_eq!(hash_string("é😀"), "14tndg7");
    }

    fn archive() -> Value {
        let page = |id: &str, n: u32| format!(r#"{{"pageId":"{id}","strokes":[{{"id":"s{n}","tool":"pen","color":"#111111","width":2,"points":[{{"x":1,"y":2,"p":0.5}},{{"x":3,"y":4,"p":0.5}}]}}],"images":[]}}"#);
        let snap = |pages: &[String], name: &str| Snapshot { pages: pages.iter().map(|p| parse(p.as_bytes()).unwrap()).collect(), calendar: Value::Null, export_name: name.into() };
        let s0 = snap(&[page("a", 1), page("b", 2)], "Doc");
        let s1 = snap(&[page("b", 3), page("c", 4)], "Doc 2");
        let text = format!(
            r#"{{"schema":"inhouse-timeline","schemaVersion":3,"checkpointInterval":8,"entries":[
            {{"type":"checkpoint","meta":{{"id":"v_1","ts":1000,"summary":"+2 pages","kind":"baseline","author":{{"email":"a@x","name":""}}}},"hash":"{h0}",
              "snapshot":{{"pages":[{pa},{pb}],"calendarPageConfig":null,"exportName":"Doc"}}}},
            {{"type":"delta","meta":{{"id":"v_2~ab12_3","ts":"bad","deviceId":"dev"}},"baseHash":"{h0}","hash":"{h1}",
              "delta":{{"order":["b","c"],"upserts":[["b",{pb3}],["c",{pc}]],"removed":["a"],"document":{{"exportName":"Doc 2"}}}}}}]}}"#,
            h0 = s0.hash(),
            h1 = s1.hash(),
            pa = page("a", 1),
            pb = page("b", 2),
            pb3 = page("b", 3),
            pc = page("c", 4)
        );
        parse(text.as_bytes()).unwrap()
    }

    #[test]
    fn materialises_checkpoints_and_deltas() {
        let a = archive();
        let v = read(&a, 777.0);
        assert_eq!(v.len(), 2);
        assert_eq!((v[0].id.as_str(), v[0].origin_id.as_str(), v[0].ts, v[0].is_milestone, v[0].kind.as_str()), ("v_1", "v_1", 1000.0, true, "baseline"));
        assert_eq!((v[0].author_email.as_str(), v[0].author_name.as_str(), v[0].summary.as_str()), ("a@x", "a@x", "+2 pages"));
        assert_eq!((v[1].id.as_str(), v[1].origin_id.as_str(), v[1].ts, v[1].kind.as_str(), v[1].summary.as_str()), ("v_2~ab12_3", "v_2", 777.0, "autosave", "Saved"));
        assert_eq!((v[1].author_name.as_str(), v[1].device_id.as_str(), v[1].export_name.as_str(), v[1].parent_id.as_deref()), ("Unknown editor", "dev", "Doc 2", None));
        assert_eq!(v[1].pages.iter().map(|p| (p.page_id.as_str(), p.strokes[0].id.as_str())).collect::<Vec<_>>(), [("b", "s3"), ("c", "s4")]);
        assert!(v[1].content_hash.contains(':'));

        // Fail closed: a wrong hash, a broken chain, an unknown schema.
        let text = json::stringify(&a);
        let broken = |from: &str, to: &str| parse(text.replacen(from, to, 1).as_bytes()).unwrap();
        assert_eq!(materialize(&broken("\"s1\"", "\"sX\"")).err(), Some("Timeline integrity check failed"));
        assert_eq!(materialize(&broken("\"baseHash\":\"", "\"baseHash\":\"x")).err(), Some("Timeline delta chain is broken"));
        assert_eq!(materialize(&broken("[\"b\",\"c\"]", "[\"b\",\"zz\"]")).err(), Some("Invalid timeline delta"));
        assert!(read(&broken("\"s1\"", "\"sX\""), 0.0).is_empty());
        assert!(matches!(materialize(&broken("\"schemaVersion\":3", "\"schemaVersion\":2")), Ok(None)));
        assert!(read(&Value::Null, 0.0).is_empty());
    }

    #[test]
    fn legacy_arrays_are_accepted_as_they_are() {
        let v = read_json(br#"[{"ts":5,"pages":[{"strokes":[],"pdfPageIndex":2}],"exportName":"Old"},{"id":"x","pages":[]},7]"#, 9.0);
        assert_eq!(v.len(), 1);
        assert_eq!((v[0].pages[0].page_id.as_str(), v[0].pages[0].background_source.as_str(), v[0].pages[0].size()), ("legacy-page-1", "pdf", pages::A4));
        let hash = &v[0].content_hash;
        assert_eq!(v[0].id, format!("recovered_5_{hash}_0"));
        assert_eq!(origin_of("v_1~abc"), "v_1");
        assert_eq!(origin_of("a~b~c_12"), "a~b");
        assert_eq!(origin_of("a~b_c"), "a~b_c");
        assert_eq!(origin_of("plain"), "plain");
    }
}
