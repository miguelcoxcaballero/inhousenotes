//! 5.x IndexedDB and localStorage records (spec/persistence.md §1): the working cache `notebook-data-db-v1`
//! (`snapshots`, `pages`, `stroke-ops`, `timeline-history`) and the offline library `inhouse-offline-v1`
//! (`documents`, `outbox`). Input = `json::Value` trees of the stored records (structured clones; typed arrays as
//! `Bytes`, blobs as `Blob` handles). Read-only.

use super::json::{self, Value};
use super::pages::{self, V5Page, V5Stroke};
use super::structfields::{self, Fields, Structure};
use super::timeline::{self, V5Version};
use super::V5Error;

/// `MetaPayload`: the `data` of `snapshots/latest`, or the `notebook-data-v3` localStorage backup.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct MetaPayload {
    /// Document session that owns the `pages` / `stroke-ops` rows; absent in older payloads.
    pub session_id: Option<f64>,
    pub saved_at: f64,
    /// One per page in document order (bodies are in the `pages` store unless `legacy`).
    pub pages: Vec<V5Page>,
    /// `strokeCount` of each page meta.
    pub stroke_counts: Vec<f64>,
    /// A legacy payload whose pages carry their `strokes` inline (`notebook-data-v2`).
    pub legacy: bool,
    pub structure: Value,
    pub fields: Value,
    /// `versionHistory` of a legacy payload.
    pub versions: Vec<V5Version>,
}

/// Accepts the `snapshots` record (`{key, data, savedAt, version}`, `data` an object or a JSON string), a bare
/// payload object, or the localStorage text.
pub fn meta_payload(v: &Value) -> Result<Option<MetaPayload>, V5Error> {
    let parsed;
    let (record, data) = match v {
        Value::Str(text) => {
            parsed = json::parse(text.as_bytes()).ok_or(V5Error::Json)?;
            (&Value::Undef, &parsed)
        }
        v if v.has("data") => match v.get("data") {
            Value::Str(text) => {
                parsed = json::parse(text.as_bytes()).ok_or(V5Error::Json)?;
                (v, &parsed)
            }
            d => (v, d),
        },
        v => (&Value::Undef, v),
    };
    let Value::Arr(list) = data.get("pages") else { return Ok(None) };
    let saved_at = [data.get("savedAt"), record.get("savedAt")].into_iter().find_map(Value::finite).unwrap_or(0.0);
    Ok(Some(MetaPayload {
        session_id: data.get("sessionId").finite(),
        saved_at,
        pages: list.iter().map(pages::page).collect::<Result<_, _>>()?,
        stroke_counts: list.iter().map(|p| p.get("strokeCount").finite().unwrap_or(0.0)).collect(),
        legacy: list.iter().any(|p| p.get("strokes").is_arr()),
        structure: data.get("collabStructure").clone(),
        fields: data.get("collabFields").clone(),
        versions: timeline::read(data.get("versionHistory"), saved_at),
    }))
}

/// A `pages` row. A row whose finite `__sessionId` differs from `session` is absent (`Ok(None)`), as in 5.18.0;
/// an unreadable ink checkpoint is an error (the row is kept by the caller, never overwritten).
pub fn page_row(v: &Value, session: Option<f64>) -> Result<Option<V5Page>, V5Error> {
    if !v.is_object() {
        return Ok(None);
    }
    match (v.get("__sessionId").finite(), session) {
        (Some(row), Some(current)) if row != current => Ok(None),
        _ => pages::page(v).map(Some),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct StrokeOp {
    pub page_index: usize,
    pub stroke: V5Stroke,
    pub saved_at: f64,
}

/// The replayable rows of `stroke-ops`, in key order: `type === 'add-stroke'`, a finite `pageIndex`, a stroke, and
/// no foreign finite `sessionId`.
pub fn stroke_ops(rows: &[Value], session: Option<f64>) -> Vec<StrokeOp> {
    rows.iter()
        .filter(|r| !matches!((r.get("sessionId").finite(), session), (Some(row), Some(current)) if row != current))
        .filter(|r| r.get("type").as_str() == Some("add-stroke"))
        .filter_map(|r| {
            let index = r.get("pageIndex").finite().filter(|i| *i >= 0.0 && i.fract() == 0.0 && *i < super::MAX_PDF_PAGES as f64)?;
            Some(StrokeOp { page_index: index as usize, stroke: pages::stroke(r.get("stroke"))?, saved_at: r.get("savedAt").finite().unwrap_or(0.0) })
        })
        .collect()
}

/// The working copy 5.18.0 would show: page rows where present (else the page meta), blank A4 template pages for
/// journal entries beyond the last page, and the journal replayed (`applyPendingStrokesToPageData`).
pub fn working_copy(meta: &MetaPayload, rows: Vec<Option<V5Page>>, ops: &[StrokeOp]) -> Vec<V5Page> {
    let mut rows = rows.into_iter();
    let mut out: Vec<V5Page> = meta.pages.iter().map(|m| rows.next().flatten().unwrap_or_else(|| m.clone())).collect();
    for op in ops {
        while out.len() <= op.page_index {
            out.push(V5Page { background_source: "template".into(), template_kind: "default", has_images_key: true, page_width: Some(pages::A4.0 as f64), page_height: Some(pages::A4.1 as f64), ..Default::default() });
        }
        let page = &mut out[op.page_index];
        let s = &op.stroke;
        if s.id.is_empty() {
            let same = |o: &V5Stroke| o.id.is_empty() && o.tool == s.tool && o.color == s.color && o.width == s.width && o.xy == s.xy;
            if !page.strokes.iter().any(same) {
                page.strokes.push(s.clone());
            }
        } else if let Some(old) = page.strokes.iter_mut().find(|o| o.id == s.id) {
            // `ihnChooseConcurrentItem`: the later stamp wins.
            if s.stamp > old.stamp {
                *old = s.clone();
            }
        } else if !page.deleted.iter().any(|(id, at)| *id == s.id && *at >= s.stamp) {
            page.strokes.push(s.clone());
        }
    }
    out
}

/// A `timeline-history` row: `historyArchiveJson` (text), or the legacy `historyArchive` (object) / `history` (array).
pub fn timeline_row(v: &Value) -> Vec<V5Version> {
    let now = v.get("savedAt").finite().unwrap_or(0.0);
    match v.get("historyArchiveJson") {
        Value::Str(text) => timeline::read_json(text.as_bytes(), now),
        _ => [v.get("historyArchive"), v.get("history")].into_iter().find(|h| h.is_object()).map_or_else(Vec::new, |h| timeline::read(h, now)),
    }
}

/// The editable snapshot of an offline `documents` row (or of a `recoveries` row).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Snapshot {
    pub pages: Vec<V5Page>,
    pub structure: Structure,
    pub fields: Fields,
    pub export_name: String,
    pub versions: Vec<V5Version>,
}

pub fn snapshot(v: &Value, now_ms: f64) -> Result<Option<Snapshot>, V5Error> {
    let Value::Arr(list) = v.get("pages") else { return Ok(None) };
    let pages: Vec<V5Page> = list.iter().map(pages::page).collect::<Result<_, _>>()?;
    let ids: Vec<&str> = pages.iter().map(|p| p.page_id.as_str()).collect();
    Ok(Some(Snapshot {
        structure: structfields::structure(v.get("collabStructure"), &ids),
        fields: structfields::fields(v.get("collabFields")),
        export_name: v.get("exportName").text(),
        versions: timeline::read(v.get("versionHistory"), now_ms),
        pages,
    }))
}

/// The Drive revision a local edit is based on. 5.18.0 stores two shapes; both are read.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct BaseMeta {
    pub head_revision_id: String,
    pub version: String,
    /// ms since the epoch; 0 when unknown.
    pub modified_ms: f64,
}

/// `Date.parse` for the ISO form Drive returns (`2026-10-30T10:00:32.000Z`).
pub fn iso_ms(text: &str) -> Option<f64> {
    let b = text.as_bytes();
    let n = |a: usize, z: usize| text.get(a..z)?.parse::<i64>().ok();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let (y, m, d, h, mi, s) = (n(0, 4)?, n(5, 7)?, n(8, 10)?, n(11, 13)?, n(14, 16)?, n(17, 19)?);
    let rest = &text[19..];
    let (frac, zone) = match rest.strip_prefix('.') {
        Some(r) => {
            let end = r.bytes().position(|c| !c.is_ascii_digit()).unwrap_or(r.len());
            (format!("0.{}", &r[..end]).parse::<f64>().ok()?, &r[end..])
        }
        None => (0.0, rest),
    };
    let offset_min = match zone {
        "Z" => 0,
        z if z.len() == 6 && (z.starts_with('+') || z.starts_with('-')) => (z[1..3].parse::<i64>().ok()? * 60 + z[4..6].parse::<i64>().ok()?) * if z.starts_with('-') { -1 } else { 1 },
        _ => return None,
    };
    // Days from civil (Howard Hinnant).
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    Some(((days * 86_400 + h * 3600 + mi * 60 + s - offset_min * 60) * 1000) as f64 + (frac * 1000.0).floor())
}

fn base_meta(v: &Value) -> BaseMeta {
    let pick = |a: &str, b: &str| if v.get(a).truthy() { v.get(a).text() } else { v.get(b).text() };
    let modified_ms = match v.get("modifiedTime") {
        Value::Str(iso) => iso_ms(iso).unwrap_or(0.0),
        _ => v.get("driveLastKnownModified").finite().unwrap_or(0.0),
    };
    BaseMeta { head_revision_id: pick("headRevisionId", "driveHeadRevisionId"), version: pick("version", "driveRevisionVersion"), modified_ms }
}

/// A host blob: `(object handle, size)`.
pub type BlobRef = (u32, f64);

fn blob(v: &Value) -> Option<BlobRef> {
    match v {
        Value::Blob { obj, size } if *size > 0.0 => Some((*obj, *size)),
        _ => None,
    }
}

/// An `inhouse-offline-v1/documents` row.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct OfflineDoc {
    pub account: String,
    /// Drive file id, or `local-<uuid>`.
    pub id: String,
    pub generation: f64,
    /// Local content not yet confirmed in Drive.
    pub dirty: bool,
    pub snapshot: Option<Snapshot>,
    pub base: BaseMeta,
    /// Last known complete PDF.
    pub blob: Option<BlobRef>,
    /// Clean original PDF bytes.
    pub original_blob: Option<BlobRef>,
    pub file_name: String,
    pub parent_ids: Vec<String>,
    pub can_edit: bool,
    pub remote_id: String,
    pub offline_edited: bool,
    pub pinned: bool,
    pub updated_at: f64,
    pub last_opened_at: f64,
}

impl OfflineDoc {
    pub fn is_local(&self) -> bool { self.id.starts_with("local-") }
}

pub fn offline_document(v: &Value) -> Result<Option<OfflineDoc>, V5Error> {
    let id = v.get("id").text();
    if !matches!(v, Value::Obj(_)) || id.is_empty() {
        return Ok(None);
    }
    let details = v.get("details");
    let updated_at = v.get("updatedAt").finite().unwrap_or(0.0);
    Ok(Some(OfflineDoc {
        account: v.get("account").text(),
        generation: v.get("generation").finite().unwrap_or(0.0),
        dirty: v.get("dirty").truthy(),
        snapshot: snapshot(v.get("snapshot"), updated_at)?,
        base: base_meta(v.get("baseMeta")),
        blob: blob(v.get("blob")),
        original_blob: blob(v.get("originalBlob")),
        file_name: if details.get("fileName").truthy() { details.get("fileName").text() } else { v.get("entry").get("name").text() },
        parent_ids: details.get("parentIds").arr().iter().map(Value::text).filter(|p| !p.is_empty()).collect(),
        can_edit: !matches!(details.get("canEdit"), Value::Bool(false)),
        remote_id: v.get("remoteId").text(),
        offline_edited: v.get("offlineEdited").truthy(),
        pinned: v.get("pinned").truthy(),
        updated_at,
        last_opened_at: v.get("lastOpenedAt").finite().unwrap_or(0.0),
        id,
    }))
}

#[derive(Clone, Debug, PartialEq)]
pub struct OutboxOp {
    pub account: String,
    pub id: String,
    /// `kind === 'folder'`; anything else is a document upload.
    pub folder: bool,
    pub generation: f64,
    pub created_at: f64,
    pub attempts: f64,
    /// HTTP status or code string of the last failure (`""` = none).
    pub last_error: String,
}

/// `inhouse-offline-v1/outbox` rows, in key order.
pub fn outbox(rows: &[Value]) -> Vec<OutboxOp> {
    rows.iter()
        .filter(|r| r.get("id").truthy())
        .map(|r| OutboxOp {
            account: r.get("account").text(),
            id: r.get("id").text(),
            folder: r.get("kind").as_str() == Some("folder"),
            generation: r.get("generation").finite().unwrap_or(0.0),
            created_at: r.get("createdAt").finite().unwrap_or(0.0),
            attempts: r.get("attempts").finite().unwrap_or(0.0),
            last_error: r.get("lastError").text(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v5::json::parse;

    #[test]
    fn iso_dates() {
        assert_eq!(iso_ms("1970-01-01T00:00:00Z"), Some(0.0));
        assert_eq!(iso_ms("2026-10-30T10:00:32.000Z"), Some(1_793_354_432_000.0));
        assert_eq!(iso_ms("2024-02-29T23:59:59.123+01:00"), Some(1_709_247_599_123.0));
        assert_eq!(iso_ms("2026-10-30"), None);
        assert_eq!(base_meta(&parse(br#"{"driveHeadRevisionId":"r","driveRevisionVersion":"7","driveLastKnownModified":5}"#).unwrap()), BaseMeta { head_revision_id: "r".into(), version: "7".into(), modified_ms: 5.0 });
        assert_eq!(base_meta(&parse(br#"{"headRevisionId":"h","version":"8","modifiedTime":"1970-01-01T00:00:01Z"}"#).unwrap()).modified_ms, 1000.0);
    }

    #[test]
    fn working_copy_replays_the_journal() {
        let meta = meta_payload(&Value::Str(r#"{"sessionId":2,"savedAt":50,"pages":[{"pageId":"p1","strokeCount":1,"pageWidth":794,"pageHeight":1123}]}"#.into())).unwrap().unwrap();
        assert_eq!((meta.session_id, meta.saved_at, meta.legacy, meta.stroke_counts.as_slice()), (Some(2.0), 50.0, false, &[1.0][..]));
        let row = parse(br#"{"pageId":"p1","__sessionId":2,"strokes":[{"id":"a","tool":"pen","points":[],"syncStamp":{"clock":5,"actor":"x"}}],"deletedStrokeStamps":{"gone":{"clock":9,"actor":"x"}}}"#).unwrap();
        assert!(page_row(&row, Some(3.0)).unwrap().is_none());
        let page = page_row(&row, Some(2.0)).unwrap().unwrap();
        let rows = parse(
            br#"[{"id":1,"pageIndex":0,"type":"add-stroke","sessionId":2,"savedAt":7,"stroke":{"id":"b","tool":"pen","points":[{"x":1,"y":2}],"__pendingOpId":"2-x-1-abc"}},
            {"id":2,"pageIndex":0,"type":"add-stroke","sessionId":1,"stroke":{"id":"old","points":[]}},
            {"id":3,"pageIndex":0,"type":"add-stroke","stroke":{"id":"a","tool":"highlighter","points":[],"syncStamp":{"clock":6,"actor":"x"}}},
            {"id":4,"pageIndex":0,"type":"add-stroke","stroke":{"id":"b","points":[]}},
            {"id":5,"pageIndex":0,"type":"add-stroke","stroke":{"id":"gone","points":[],"syncStamp":{"clock":8,"actor":"x"}}},
            {"id":6,"pageIndex":2,"type":"add-stroke","stroke":{"id":"far","points":[]}},
            {"id":7,"pageIndex":0,"type":"erase","stroke":{"id":"no"}},{"id":8,"pageIndex":"0","type":"add-stroke","stroke":{"id":"no2"}}]"#,
        )
        .unwrap();
        let ops = stroke_ops(rows.arr(), Some(2.0));
        assert_eq!(ops.iter().map(|o| o.stroke.id.as_str()).collect::<Vec<_>>(), ["b", "a", "b", "gone", "far"]);
        assert_eq!(ops[0].stroke.pending_op, "2-x-1-abc");
        let doc = working_copy(&meta, vec![Some(page)], &ops);
        assert_eq!(doc.len(), 3);
        assert_eq!(doc[0].strokes.iter().map(|s| (s.id.as_str(), s.tool.as_str())).collect::<Vec<_>>(), [("a", "highlighter"), ("b", "pen")]);
        assert_eq!((doc[1].strokes.len(), doc[2].strokes[0].id.as_str(), doc[2].size()), (0, "far", pages::A4));
        // Without the row, the page meta stands in.
        assert_eq!(working_copy(&meta, vec![None], &[])[0].page_id, "p1");
    }

    #[test]
    fn offline_rows() {
        let v = parse(
            br#"{"account":"a@x","id":"local-1","generation":3,"dirty":true,"updatedAt":99,"snapshot":{"pages":[{"pageId":"p","strokes":[],"images":[]}],"exportName":"N",
            "collabStructure":{"entries":{"p":{"rank":5}}},"versionHistory":[]},"baseMeta":{"headRevisionId":"h"},"details":{"fileName":"N.pdf","parentIds":["root"],"canEdit":true}}"#,
        )
        .unwrap();
        let d = offline_document(&v).unwrap().unwrap();
        assert!(d.is_local() && d.dirty && d.can_edit && d.blob.is_none());
        let s = d.snapshot.unwrap();
        assert_eq!((s.export_name.as_str(), s.structure.entries["p"].rank, s.pages.len(), d.file_name.as_str(), d.parent_ids.len()), ("N", 5.0, 1, "N.pdf", 1));
        assert_eq!(offline_document(&Value::Null), Ok(None));
        let rows = parse(br#"[{"account":"a","id":"f","kind":"folder","generation":1,"createdAt":4},{"account":"a","id":"d","kind":"document","generation":2,"attempts":3,"lastError":503},{}]"#).unwrap();
        let o = outbox(rows.arr());
        assert_eq!(o.len(), 2);
        assert!(o[0].folder && !o[1].folder && o[1].last_error == "503" && o[1].attempts == 3.0);
        assert_eq!(timeline_row(&parse(br#"{"history":[{"ts":1,"pages":[{"pageId":"p"}]}],"savedAt":3}"#).unwrap()).len(), 1);
    }
}
