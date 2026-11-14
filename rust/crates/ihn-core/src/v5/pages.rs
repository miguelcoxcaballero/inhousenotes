//! 5.x page entries: the elements of `STROKES_Z` / `STROKES_DATA`, IndexedDB page rows, offline snapshot pages and
//! timeline snapshot pages share one shape (spec/documents.md §3.4, spec/persistence.md §1.1.3).

use super::checkpoint;
use super::json::{self, cmp_utf16, Value};
use super::V5Error;
use crate::jsmath;
use std::cmp::Ordering;

pub const A4: (u32, u32) = (794, 1123);
pub const A5: (u32, u32) = (559, 794);

/// A 5.x causal stamp, normalised (`ihnNormalizeCollabStamp`). `{0, ""}` = no stamp.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct V5Stamp {
    pub clock: u64,
    pub actor: String,
}

impl V5Stamp {
    /// `Math.max(0, Math.floor(Number(clock) || 0))`.
    pub fn clock_of(v: &Value) -> u64 {
        let n = v.num();
        if n.is_nan() || n <= 0.0 {
            0
        } else {
            n.floor().min(9.0e15) as u64
        }
    }
    pub fn of(v: &Value) -> V5Stamp { V5Stamp { clock: Self::clock_of(v.get("clock")), actor: v.get("actor").text() } }
    pub fn is_zero(&self) -> bool { self.clock == 0 && self.actor.is_empty() }
    /// `ihnGetItemStamp`: `syncStamp`, `_syncStamp`, or `updatedClock`/`updatedBy`.
    pub fn of_item(item: &Value) -> V5Stamp {
        for key in ["syncStamp", "_syncStamp"] {
            if item.get(key).truthy() {
                return Self::of(item.get(key));
            }
        }
        if item.get("updatedClock").truthy() || item.get("updatedBy").truthy() {
            return V5Stamp { clock: Self::clock_of(item.get("updatedClock")), actor: item.get("updatedBy").text() };
        }
        V5Stamp::default()
    }
}

impl Ord for V5Stamp {
    /// `ihnCompareCollabStamps`: clock, then actor as JavaScript compares strings.
    fn cmp(&self, o: &Self) -> Ordering { self.clock.cmp(&o.clock).then_with(|| cmp_utf16(&self.actor, &o.actor)) }
}
impl PartialOrd for V5Stamp {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> { Some(self.cmp(o)) }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct V5Stroke {
    /// Empty when the stroke has none (legacy).
    pub id: String,
    pub tool: String,
    pub color: String,
    pub width: f64,
    /// `x0, y0, x1, y1, …`
    pub xy: Vec<f64>,
    /// Pressure per point, NaN where the point has none; empty when no point has one.
    pub p: Vec<f64>,
    pub stamp: V5Stamp,
    /// `__pendingOpId` of a stroke that came from the `stroke-ops` journal.
    pub pending_op: String,
}

impl V5Stroke {
    pub fn points(&self) -> usize { self.xy.len() / 2 }
    /// `pageEntryHasDrawableVectors` / the renderers: not an `eraser-stroke`, and enough points.
    pub fn drawable(&self) -> bool { self.tool != "eraser-stroke" && self.points() >= if self.tool == "highlighter" { 1 } else { 2 } }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct V5Image {
    pub id: String,
    /// A data URL in exported files; may be a `blob:` URL in local records.
    pub src: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub rotation: f64,
    pub stamp: V5Stamp,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct V5Page {
    /// Empty when missing.
    pub page_id: String,
    pub strokes: Vec<V5Stroke>,
    pub images: Vec<V5Image>,
    /// An entry without an `images` key is legacy-raster: images were flattened into the page.
    pub has_images_key: bool,
    /// `ihnNormalizeDeletionStamps(deletedStrokeStamps, deletedStrokeIds)`, sorted by id.
    pub deleted: Vec<(String, V5Stamp)>,
    /// `""` when missing.
    pub background_source: String,
    pub has_custom_background: bool,
    /// A data URL (photo page in a local record); never present in exported files.
    pub background_image: Option<String>,
    /// Normalised: `default`, `agenda` or `diary`.
    pub template_kind: &'static str,
    /// `Number.isFinite(pdfPageIndex)`.
    pub pdf_page_index: Option<f64>,
    /// Only when the stored value is a finite number.
    pub page_width: Option<f64>,
    pub page_height: Option<f64>,
    /// `sidePanel || null` as stable JSON text.
    pub side_panel: Option<String>,
    pub legacy_cover_strokes: Option<Vec<V5Stroke>>,
    /// `__sessionId` of an IndexedDB row.
    pub session_id: Option<f64>,
}

/// `normalizeTemplateKind`.
pub fn template_kind(v: &Value) -> &'static str {
    match v.as_str() {
        Some("agenda") => "agenda",
        Some("diary") => "diary",
        _ => "default",
    }
}

/// `normalizePageDimension(value, fallback)`: `max(64, Math.round(value))` for a finite number.
pub fn dimension(value: Option<f64>, fallback: u32) -> u32 {
    match value {
        Some(v) => jsmath::round(v).clamp(64.0, u32::MAX as f64) as u32,
        None => fallback,
    }
}

fn points(v: &Value, words: Option<&[u64]>) -> Result<(Vec<f64>, Vec<f64>), V5Error> {
    if let Value::Arr(list) = v {
        let mut xy = Vec::with_capacity(list.len() * 2);
        let mut p = Vec::new();
        for (i, pt) in list.iter().enumerate() {
            let (x, y, pr) = match pt {
                Value::Pt { x, y, p } => (*x, *y, *p),
                // `p` is kept when it is not null/undefined (the exporter's rule).
                o => (o.get("x").num(), o.get("y").num(), if matches!(o.get("p"), Value::Undef | Value::Null) { f64::NAN } else { o.get("p").num() }),
            };
            xy.push(x);
            xy.push(y);
            if !pr.is_nan() {
                if p.is_empty() {
                    p = vec![f64::NAN; list.len()];
                }
                p[i] = pr;
            }
        }
        return Ok((xy, p));
    }
    match (v.get("inkRange"), words) {
        (Value::Undef, _) => Ok((Vec::new(), Vec::new())),
        (r, Some(words)) => checkpoint::range(words, r),
        (_, None) => Err(V5Error::Checkpoint("Unknown ink checkpoint format")),
    }
}

fn strokes(v: &Value, words: Option<&[u64]>) -> Result<Vec<V5Stroke>, V5Error> {
    let mut out = Vec::with_capacity(v.arr().len());
    for s in v.arr() {
        if !matches!(s, Value::Obj(_)) {
            continue;
        }
        let (xy, p) = points(s.get("points"), words)?;
        out.push(V5Stroke { id: s.get("id").text(), tool: s.get("tool").text(), color: s.get("color").text(), width: s.get("width").num(), xy, p, stamp: V5Stamp::of_item(s), pending_op: s.get("__pendingOpId").text() });
    }
    Ok(out)
}

/// One stroke object (the `stroke` of a `stroke-ops` row).
pub fn stroke(v: &Value) -> Option<V5Stroke> { strokes(&Value::Arr(vec![v.clone()]), None).ok()?.pop() }

/// `ihnNormalizeDeletionStamps(stamps, legacyDeletedIds)`.
pub fn deletion_stamps(stamps: &Value, legacy_ids: &Value) -> Vec<(String, V5Stamp)> {
    let mut found: std::collections::HashMap<String, V5Stamp> = std::collections::HashMap::new();
    let mut record = |id: String, raw: &Value| {
        if id.is_empty() {
            return;
        }
        let stamp = V5Stamp::of(if raw.get("stamp").truthy() { raw.get("stamp") } else { raw });
        match found.get_mut(&id) {
            Some(old) => {
                if stamp > *old {
                    *old = stamp;
                }
            }
            None => {
                found.insert(id, stamp);
            }
        }
    };
    for value in [stamps, legacy_ids] {
        match value {
            Value::Arr(list) => {
                for entry in list {
                    if entry.is_object() {
                        record(entry.get("id").text(), if entry.get("stamp").truthy() { entry.get("stamp") } else { entry });
                    } else {
                        record(entry.text(), &Value::Null);
                    }
                }
            }
            Value::Obj(_) => {
                for (id, raw) in value.entries() {
                    record(id.to_string(), raw);
                }
            }
            _ => {}
        }
    }
    let mut out: Vec<(String, V5Stamp)> = found.into_iter().collect();
    crate::sort::by(&mut out, |a, b| cmp_utf16(&a.0, &b.0));
    out
}

/// A page entry / stored page. `__inkCheckpoint`, when present, is unpacked (`unpackInkPageFromStorage`).
pub fn page(v: &Value) -> Result<V5Page, V5Error> {
    let words = match v.get("__inkCheckpoint") {
        cp if cp.truthy() => Some(checkpoint::unpack(cp)?),
        _ => None,
    };
    let words = words.as_deref();
    let images = v
        .get("images")
        .arr()
        .iter()
        .filter(|i| matches!(i, Value::Obj(_)))
        .map(|i| V5Image {
            id: i.get("id").text(),
            src: i.get("src").text(),
            x: i.get("x").num(),
            y: i.get("y").num(),
            width: i.get("width").num(),
            height: i.get("height").num(),
            rotation: match i.get("rotation") {
                Value::Undef | Value::Null => 0.0,
                r => r.num(),
            },
            stamp: V5Stamp::of_item(i),
        })
        .collect();
    let side = v.get("sidePanel");
    Ok(V5Page {
        page_id: v.get("pageId").text(),
        strokes: strokes(v.get("strokes"), words)?,
        images,
        has_images_key: v.has("images"),
        deleted: deletion_stamps(v.get("deletedStrokeStamps"), v.get("deletedStrokeIds")),
        background_source: v.get("backgroundSource").text(),
        has_custom_background: v.get("hasCustomBackground").truthy(),
        background_image: v.get("backgroundImage").as_str().filter(|s| !s.is_empty()).map(str::to_string),
        template_kind: template_kind(v.get("templateKind")),
        pdf_page_index: v.get("pdfPageIndex").finite(),
        page_width: v.get("pageWidth").finite(),
        page_height: v.get("pageHeight").finite(),
        side_panel: side.truthy().then(|| json::stable(side)),
        legacy_cover_strokes: match v.get("legacyCoverStrokes") {
            c @ Value::Arr(_) => Some(strokes(c, words)?),
            _ => None,
        },
        session_id: v.get("__sessionId").finite(),
    })
}

impl V5Page {
    /// `pageEntryNeedsPdfBackground`.
    pub fn needs_pdf_background(&self) -> bool {
        matches!(self.background_source.as_str(), "pdf" | "custom") || self.has_custom_background || self.pdf_page_index.is_some()
    }
    /// `pageEntryHasDrawableVectors`.
    pub fn has_drawable_vectors(&self) -> bool { self.strokes.iter().any(V5Stroke::drawable) || !self.images.is_empty() }
    /// `normalizeLegacyPage`'s source: stored value, else `pdfPageIndex` → pdf, `backgroundImage` → custom.
    pub fn source(&self) -> &str {
        if !self.background_source.is_empty() {
            &self.background_source
        } else if self.pdf_page_index.is_some_and(|i| i != 0.0) {
            "pdf"
        } else if self.background_image.is_some() {
            "custom"
        } else {
            "template"
        }
    }
    /// Stored size with the template kind's default (`getTemplateKindDefaults`).
    pub fn size(&self) -> (u32, u32) {
        let d = if self.template_kind == "default" { A4 } else { A5 };
        (dimension(self.page_width, d.0), dimension(self.page_height, d.1))
    }
}

/// The array of page entries of a `STROKES_*` payload. `None` when the JSON is not an array.
pub fn entries(payload: &Value) -> Result<Option<Vec<Option<V5Page>>>, V5Error> {
    let Value::Arr(list) = payload else { return Ok(None) };
    // A falsy entry (`null`) means "no data for this PDF page".
    list.iter().map(|e| if e.truthy() { page(e).map(Some) } else { Ok(None) }).collect::<Result<Vec<_>, _>>().map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v5::json::parse;

    #[test]
    fn reads_a_page_entry() {
        let v = parse(
            br##"{"pageId":"pg_1","strokes":[{"id":"a","tool":"pen","color":"#1A1A1A","width":2,"points":[{"x":1,"y":2,"p":0.5},{"x":3.25,"y":4}],"syncStamp":{"clock":10,"actor":"peer"}},
            {"tool":"highlighter","color":"rgba(255,235,59,0.35)","width":18,"points":[{"y":2,"x":"1"}]},null,{"id":"e","tool":"eraser-stroke","points":[{"x":1,"y":1},{"x":2,"y":2}]}],
            "images":[{"id":"i","src":"data:image/png;base64,AAAA","x":10,"y":20,"width":30,"height":40}],
            "deletedStrokeIds":["z","b"],"deletedStrokeStamps":{"b":{"clock":5,"actor":"q"},"c":{"stamp":{"clock":7,"actor":"r"}}},
            "hasCustomBackground":false,"backgroundSource":"template","templateKind":"weird","pdfPageIndex":null,"pageWidth":793.5,"pageHeight":"1123","sidePanel":{"title":"t","html":"<b>"}}"##,
        )
        .unwrap();
        let p = page(&v).unwrap();
        assert_eq!(p.page_id, "pg_1");
        assert_eq!(p.strokes.len(), 3);
        assert_eq!(p.strokes[0].xy, [1.0, 2.0, 3.25, 4.0]);
        assert!(p.strokes[0].p[0] == 0.5 && p.strokes[0].p[1].is_nan());
        assert_eq!(p.strokes[0].stamp, V5Stamp { clock: 10, actor: "peer".into() });
        assert_eq!((p.strokes[1].id.as_str(), p.strokes[1].xy.as_slice(), p.strokes[1].p.len()), ("", &[1.0, 2.0][..], 0));
        assert!(p.strokes[1].drawable() && !p.strokes[2].drawable() && p.has_drawable_vectors());
        assert_eq!(p.images[0].rotation, 0.0);
        assert_eq!(p.deleted, [("b".to_string(), V5Stamp { clock: 5, actor: "q".into() }), ("c".to_string(), V5Stamp { clock: 7, actor: "r".into() }), ("z".to_string(), V5Stamp::default())]);
        assert_eq!((p.template_kind, p.size(), p.source(), p.needs_pdf_background()), ("default", (794, 1123), "template", false));
        assert_eq!(p.side_panel.as_deref(), Some(r#"{"html":"<b>","title":"t"}"#));
        assert!(p.has_images_key && p.legacy_cover_strokes.is_none());

        let legacy = page(&parse(br#"{"strokes":[],"hasCustomBackground":true,"templateKind":"diary"}"#).unwrap()).unwrap();
        assert!(!legacy.has_images_key && legacy.needs_pdf_background() && legacy.size() == A5 && legacy.page_id.is_empty());
        assert_eq!(dimension(Some(10.0), 5), 64);
        assert_eq!(dimension(Some(100.5), 5), 101);
        assert_eq!(entries(&parse(b"[null,{}]").unwrap()).unwrap().unwrap().iter().map(Option::is_some).collect::<Vec<_>>(), [false, true]);
        assert_eq!(entries(&parse(b"{}").unwrap()), Ok(None));
    }

    #[test]
    fn stamps_order_like_the_collaboration_core() {
        let s = |clock, actor: &str| V5Stamp { clock, actor: actor.into() };
        assert!(s(2, "") > s(1, "z") && s(1, "b") > s(1, "a") && s(0, "").is_zero() && !s(0, "a").is_zero());
        assert_eq!(V5Stamp::of(&parse(br#"{"clock":"12.9","actor":7}"#).unwrap()), s(12, "7"));
        assert_eq!(V5Stamp::of_item(&parse(br#"{"updatedClock":4,"updatedBy":"u"}"#).unwrap()), s(4, "u"));
        assert_eq!(V5Stamp::of_item(&parse(br#"{"_syncStamp":{"clock":3}}"#).unwrap()), s(3, ""));
    }
}
