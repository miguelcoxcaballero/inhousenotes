//! `IH_STRUCT` and `IH_FIELDS` (collaboration-core-v5.js): the page-structure and field registers, normalised as
//! `ihnNormalizeStructureMeta` / `ihnNormalizeFieldMeta` normalise them.

use super::json::{cmp_utf16, Value};
use super::pages::V5Stamp;
use std::collections::BTreeMap;

pub const RANK_STEP: f64 = 1024.0;

#[derive(Clone, Debug, PartialEq)]
pub struct StructEntry {
    pub rank: f64,
    pub order: V5Stamp,
    pub created: V5Stamp,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Structure {
    pub clock: u64,
    pub entries: BTreeMap<String, StructEntry>,
    pub tombstones: BTreeMap<String, V5Stamp>,
}

fn sorted_keys(v: &Value) -> Vec<(&str, &Value)> {
    let mut keys: Vec<(&str, &Value)> = v.entries().collect();
    keys.reverse();
    crate::sort::by(&mut keys, |a, b| cmp_utf16(a.0, b.0));
    keys.dedup_by(|b, a| a.0 == b.0);
    keys
}

/// `ihnNormalizeStructureMeta(meta, pageIds)`.
pub fn structure(meta: &Value, page_ids: &[&str]) -> Structure {
    let mut s = Structure { clock: V5Stamp::clock_of(meta.get("clock")), ..Default::default() };
    for (index, (id, raw)) in sorted_keys(meta.get("entries")).into_iter().enumerate() {
        if id.is_empty() {
            continue;
        }
        let rank = raw.get("rank").num();
        let e = StructEntry { rank: if rank.is_finite() { rank } else { index as f64 * RANK_STEP }, order: V5Stamp::of(raw.get("orderStamp")), created: V5Stamp::of(raw.get("createdStamp")) };
        s.clock = s.clock.max(e.order.clock).max(e.created.clock);
        s.entries.insert(id.to_string(), e);
    }
    for (id, raw) in sorted_keys(meta.get("tombstones")) {
        if id.is_empty() {
            continue;
        }
        let stamp = V5Stamp::of(raw);
        s.clock = s.clock.max(stamp.clock);
        s.tombstones.insert(id.to_string(), stamp);
    }
    // ihnNormalizePageIdList: empty and repeated ids are skipped before the index is counted.
    let mut index = 0;
    let mut seen: Vec<&str> = Vec::new();
    for id in page_ids {
        if id.is_empty() || seen.contains(id) {
            continue;
        }
        seen.push(id);
        s.entries.entry(id.to_string()).or_insert(StructEntry { rank: index as f64 * RANK_STEP, order: V5Stamp::default(), created: V5Stamp::default() });
        index += 1;
    }
    s
}

impl Structure {
    /// `ihnIsPageTombstoned`: a tombstone at or after the page's creation.
    pub fn is_tombstoned(&self, page_id: &str) -> bool {
        match (self.tombstones.get(page_id), self.entries.get(page_id)) {
            (Some(t), Some(e)) => *t >= e.created,
            (Some(_), None) => true,
            _ => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Fields {
    pub clock: u64,
    pub fields: BTreeMap<String, (Value, V5Stamp)>,
}

pub const DOC_NAME_KEY: &str = "doc:exportName";

/// `ihnNormalizeFieldMeta(meta)`.
pub fn fields(meta: &Value) -> Fields {
    let mut f = Fields { clock: V5Stamp::clock_of(meta.get("clock")), ..Default::default() };
    for (key, raw) in sorted_keys(meta.get("fields")) {
        if key.is_empty() || !raw.has("value") {
            continue;
        }
        let stamp = V5Stamp::of(raw.get("stamp"));
        f.clock = f.clock.max(stamp.clock);
        let value = match raw.get("value") {
            Value::Undef => Value::Null,
            v => v.clone(),
        };
        f.fields.insert(key.to_string(), (value, stamp));
    }
    f
}

impl Fields {
    pub fn get(&self, key: &str) -> Option<&(Value, V5Stamp)> { self.fields.get(key) }
    /// `page:<pageId>:<name>` (`background`, `template`, `size`, `sidePanel`).
    pub fn page(&self, page_id: &str, name: &str) -> Option<&(Value, V5Stamp)> { self.fields.get(&format!("page:{page_id}:{name}")) }
    /// `ihnResolveFieldValue(fields, 'doc:exportName', fallback)` for the document name.
    pub fn export_name(&self) -> Option<(&str, &V5Stamp)> { self.get(DOC_NAME_KEY).and_then(|(v, s)| v.as_str().map(|n| (n, s))) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v5::json::parse;

    #[test]
    fn structure_is_normalised_like_the_collaboration_core() {
        let meta = parse(br#"{"v":1,"clock":"7","entries":{"b":{"rank":"x","orderStamp":{"clock":9.7,"actor":"p"}},"a":{"rank":2048,"createdStamp":{"clock":-4}},"":{"rank":1}},
            "tombstones":{"gone":{"clock":30,"actor":"q"},"a":{"clock":0,"actor":""}}}"#).unwrap();
        let s = structure(&meta, &["c", "", "a", "c", "d"]);
        assert_eq!(s.clock, 30);
        assert_eq!(s.entries.keys().collect::<Vec<_>>(), ["a", "b", "c", "d"]);
        // "b" is second in sorted key order ("" first): fallback rank 2 * 1024; "c" and "d" take their list index.
        assert_eq!(s.entries["b"], StructEntry { rank: 2048.0, order: V5Stamp { clock: 9, actor: "p".into() }, created: V5Stamp::default() });
        assert_eq!((s.entries["a"].rank, s.entries["a"].created.clock, s.entries["c"].rank, s.entries["d"].rank), (2048.0, 0, 0.0, 2048.0));
        assert!(s.is_tombstoned("gone") && s.is_tombstoned("a") && !s.is_tombstoned("b"));
        assert_eq!(structure(&Value::Null, &[]), Structure::default());
    }

    #[test]
    fn fields_are_normalised_like_the_collaboration_core() {
        let meta = parse(br#"{"clock":3,"fields":{"doc:exportName":{"value":"Notes","stamp":{"clock":12,"actor":"a"}},"page:p1:size":{"value":{"pageWidth":794,"pageHeight":1123}},
            "bad":{"stamp":{"clock":99}},"n":null,"page:p1:sidePanel":{"value":null,"stamp":{"clock":0,"actor":""}}}}"#).unwrap();
        let f = fields(&meta);
        assert_eq!(f.clock, 12);
        assert_eq!(f.fields.len(), 3);
        assert_eq!(f.export_name(), Some(("Notes", &V5Stamp { clock: 12, actor: "a".into() })));
        assert!(f.page("p1", "size").unwrap().1.is_zero());
        assert_eq!(f.page("p1", "sidePanel").unwrap().0, Value::Null);
        assert_eq!(fields(&Value::Arr(vec![])), Fields::default());
    }
}
