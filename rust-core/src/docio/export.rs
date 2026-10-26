//! PDF export bookkeeping: per-page metadata entry, page hash, export profile,
//! copyPages plan and page geometry. All inputs are validated; nothing panics.

use super::jv::*;
use super::strokes::*;

fn split_sections<const N: usize>(input: &[u8]) -> Result<[&[u8]; N], String> {
    let mut sections: [&[u8]; N] = [&[]; N];
    let mut rest = input;
    for slot in sections.iter_mut() {
        if rest.len() < 4 { return Err("Truncated docio frame".into()); }
        let len = u32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
        if len > rest.len() - 4 { return Err("Truncated docio frame".into()); }
        *slot = &rest[4..4 + len];
        rest = &rest[4 + len..];
    }
    if !rest.is_empty() { return Err("Trailing docio frame bytes".into()); }
    Ok(sections)
}

fn header(bytes: &[u8]) -> Result<V, String> {
    let v = P::document(bytes)?;
    if matches!(v, V::Obj(_)) { Ok(v) } else { Err("Header must be an object".into()) }
}

fn number_field(header: &V, key: &str) -> f64 { to_number(header.get(key.as_bytes())) }

/// `normalizePageDimension(value, fallback)`.
pub fn normalize_dimension(value: f64, fallback: f64) -> f64 {
    if value.is_finite() { 64f64.max(js_round(value)) } else { fallback }
}

// ------------------------------------------------------------------ deletion stamps

pub struct Stamps(pub Vec<(Bytes, f64, Bytes)>);

fn stamp_parts(stamp: Option<&V>) -> (f64, Bytes) {
    let clock = to_number(stamp.and_then(|s| s.get(b"clock")));
    let clock = if clock != 0.0 && !clock.is_nan() { clock } else { 0.0 };
    let clock = 0f64.max(clock.floor());
    let actor = match stamp.and_then(|s| s.get(b"actor")).filter(|a| a.truthy()) { Some(a) => to_js_string(a), None => Vec::new() };
    (clock, actor)
}

/// `ihnNormalizeDeletionStamps(stamps, legacyIds)`: sorted by UTF-16 order, `{clock, actor}` each.
pub fn normalize_deletion_stamps(stamps: Option<&V>, legacy: Option<&V>) -> Stamps {
    let mut collected: Vec<(Bytes, f64, Bytes)> = Vec::new();
    let mut index: std::collections::HashMap<Bytes, usize> = std::collections::HashMap::new();
    let mut record = |raw_id: Bytes, raw_stamp: Option<&V>| {
        if raw_id.is_empty() { return; }
        let source = match raw_stamp {
            Some(s) => match s.get(b"stamp").filter(|v| v.truthy()) { Some(inner) => Some(inner), None => Some(s) },
            None => None,
        };
        let (clock, actor) = stamp_parts(source);
        match index.get(&raw_id) {
            Some(&n) => {
                let previous = &collected[n];
                // ihnMaxCollabStamp keeps the previous stamp when it is >= the new one.
                let previous_wins = previous.1 > clock || (previous.1 == clock && cmp_utf16(&previous.2, &actor) != std::cmp::Ordering::Less);
                if !previous_wins { collected[n].1 = clock; collected[n].2 = actor; }
            }
            None => { index.insert(raw_id.clone(), collected.len()); collected.push((raw_id, clock, actor)); }
        }
    };
    let zero = V::Obj(vec![(b"clock".to_vec(), V::Num(0.0)), (b"actor".to_vec(), V::Str(Vec::new()))]);
    let mut consume = |value: Option<&V>| {
        let Some(value) = value.filter(|v| v.truthy()) else { return };
        match value {
            V::Arr(entries) => for entry in entries {
                match entry {
                    V::Obj(_) => {
                        let id = entry.get(b"id").filter(|v| v.truthy()).map(to_js_string).unwrap_or_default();
                        let stamp = entry.get(b"stamp").filter(|v| v.truthy()).unwrap_or(entry);
                        record(id, Some(stamp));
                    }
                    V::Arr(_) => {}
                    other => { let id = if other.truthy() { to_js_string(other) } else { Vec::new() }; record(id, Some(&zero)); }
                }
            },
            V::Obj(map) => for (id, stamp) in map { record(id.clone(), Some(stamp)); },
            _ => {}
        }
    };
    consume(stamps);
    consume(legacy);
    collected.sort_by(|a, b| cmp_utf16(&a.0, &b.0));
    // Integer-like ids enumerate first in a JavaScript object.
    let mut ordered: Vec<(Bytes, (f64, Bytes))> = collected.into_iter().map(|(k, c, a)| (k, (c, a))).collect();
    js_property_order(&mut ordered);
    Stamps(ordered.into_iter().map(|(k, (c, a))| (k, c, a)).collect())
}

impl Stamps {
    /// `JSON.stringify(normalized)`: `{clock, actor}` per id.
    pub fn write_plain(&self, out: &mut Bytes) {
        out.push(b'{');
        for (n, (id, clock, actor)) in self.0.iter().enumerate() {
            if n > 0 { out.push(b','); }
            write_str(id, out);
            out.extend_from_slice(b":{\"clock\":");
            write_num(*clock, out);
            out.extend_from_slice(b",\"actor\":");
            write_str(actor, out);
            out.push(b'}');
        }
        out.push(b'}');
    }
    /// `ihnStableStringify(normalized)`: keys sorted, so `actor` precedes `clock`.
    pub fn write_stable(&self, out: &mut Bytes) {
        out.push(b'{');
        for (n, (id, clock, actor)) in self.0.iter().enumerate() {
            if n > 0 { out.push(b','); }
            write_str(id, out);
            out.extend_from_slice(b":{\"actor\":");
            write_str(actor, out);
            out.extend_from_slice(b",\"clock\":");
            write_num(*clock, out);
            out.push(b'}');
        }
        out.push(b'}');
    }
}

// ------------------------------------------------------------------ page hash

/// `computePageHash(pageData)`.
pub fn page_hash(input: &[u8]) -> Result<Bytes, String> {
    let [head, strokes_json] = split_sections::<2>(input)?;
    let h = header(head)?;
    let mut out: Bytes = Vec::with_capacity(strokes_json.len() / 4 + 256);
    let mut stroke_count = 0usize;
    if !strokes_json.is_empty() {
        let items = parse_strokes(strokes_json)?;
        stroke_count = items.len();
        for (n, item) in items.iter().enumerate() {
            if n > 0 { out.push(b','); }
            stroke_fingerprint(item, &mut out);
        }
    } else if let Some(other) = h.get(b"strokes") {
        stroke_count = match other { V::Str(s) => utf16_len(s), V::Arr(a) => a.len(), _ => 0 };
    }
    out.push(b'|');
    if let Some(V::Arr(images)) = h.get(b"images") {
        for (n, image) in images.iter().enumerate() {
            if n > 0 { out.push(b','); }
            image_fingerprint(image, &mut out)?;
        }
    }
    out.push(b'|');
    let legacy = h.get(b"deletedStrokeIds");
    if let Some(V::Arr(ids)) = legacy {
        let mut first = true;
        for id in ids.iter().filter(|v| v.truthy()) {
            if !first { out.push(b','); }
            first = false;
            out.extend_from_slice(&to_js_string(id));
        }
    }
    out.push(b'|');
    normalize_deletion_stamps(h.get(b"deletedStrokeStamps"), legacy).write_stable(&mut out);
    out.push(b'|');
    if let Some(bg) = h.get(b"bgSource").filter(|v| v.truthy()) { out.extend_from_slice(&to_js_string(bg)); }
    out.push(b'|');
    if let Some(index) = h.get(b"pdfIndex").filter(|v| v.truthy()) { out.extend_from_slice(&to_js_string(index)); }
    out.push(b'|');
    let bg_len = number_field(&h, "bgLen");
    if bg_len > 0.0 {
        fmt_num(bg_len, &mut out);
        out.push(b':');
        if let Some(V::Str(head)) = h.get(b"bgHead") { simple_hash(head, &mut out); } else { out.push(b'0'); }
    }
    out.push(b'|');
    let w = normalize_dimension(h.get(b"pageWidth").map_or(f64::NAN, |v| if let V::Num(n) = v { *n } else { f64::NAN }), number_field(&h, "a4Width"));
    let ht = normalize_dimension(h.get(b"pageHeight").map_or(f64::NAN, |v| if let V::Num(n) = v { *n } else { f64::NAN }), number_field(&h, "a4Height"));
    fmt_num(w, &mut out);
    out.push(b'x');
    fmt_num(ht, &mut out);
    out.push(b'|');
    fmt_num(stroke_count as f64, &mut out);
    Ok(out)
}

// ------------------------------------------------------------------ page export (dedupe, compaction, metadata entry, bounds)

/// Returns `[keepCount u32][keep u32...][hasBounds u8][4 x f64][compactLen u32][compact][meta]`.
pub fn page_export(input: &[u8]) -> Result<Vec<u8>, String> {
    let [head, strokes_json, cached_compact] = split_sections::<3>(input)?;
    let h = header(head)?;
    let mode = match h.get(b"mode") {
        Some(m) if m.is_str("pdflib") => Mode::PdfLib,
        Some(m) if m.is_str("raster") => Mode::Raster,
        _ => return Err("Unknown export mode".into()),
    };
    let reuse = truthy(h.get(b"reuse"));
    let items = if strokes_json.is_empty() { Vec::new() } else { parse_strokes(strokes_json)? };
    let mut keep: Vec<usize> = Vec::new();
    let mut compact: Bytes = Vec::new();
    if reuse {
        compact.extend_from_slice(cached_compact);
    } else {
        keep = dedupe(&items, strokes_json)?;
        compact.reserve(strokes_json.len() / 2 + 16);
        compact.push(b'[');
        for (n, &i) in keep.iter().enumerate() {
            if n > 0 { compact.push(b','); }
            let Item::Obj(stroke) = &items[i] else { return Err("Cannot read properties of a non-object stroke".into()) };
            write_compact_stroke(stroke, mode, strokes_json, &mut compact)?;
        }
        compact.push(b']');
    }

    let images = h.get(b"images");
    let bounds = match h.get(b"bounds") {
        Some(b @ V::Obj(_)) => overlay_bounds(&items, if reuse { None } else { Some(&keep) }, images,
            number_field(b, "width"), number_field(b, "height"), number_field(b, "scale"))?,
        _ => None,
    };

    // Metadata entry, key order identical to the original object literal.
    let background = h.get(b"bgSource");
    let pdf_index = h.get(b"pdfIndex");
    let has_image = truthy(h.get(b"hasBgImage"));
    let resolved: V = match (mode, background.filter(|v| v.truthy())) {
        (_, Some(v)) => v.clone(),
        (_, None) => V::Str(if truthy(pdf_index) { b"pdf".to_vec() } else if mode == Mode::Raster && has_image { b"custom".to_vec() } else { b"template".to_vec() }),
    };
    // The pdf-lib path tests the stored source; the raster path tests the resolved one.
    let has_custom = has_image || match mode {
        Mode::PdfLib => background.is_some_and(|v| v.is_str("pdf")),
        Mode::Raster => resolved.is_str("pdf"),
    };
    let page_index = number_field(&h, "pageIndex");
    let mut meta: Bytes = Vec::with_capacity(compact.len() + 512);
    meta.push(b'{');
    if let Some(id) = h.get(b"pageId") { meta.extend_from_slice(b"\"pageId\":"); write(id, &mut meta); meta.push(b','); }
    meta.extend_from_slice(b"\"strokes\":");
    meta.extend_from_slice(&compact);
    meta.extend_from_slice(b",\"images\":");
    match images { Some(a @ V::Arr(_)) => write(a, &mut meta), _ => meta.extend_from_slice(b"[]") }
    meta.extend_from_slice(b",\"deletedStrokeIds\":");
    let legacy = h.get(b"deletedStrokeIds");
    match legacy { Some(a @ V::Arr(_)) => write(a, &mut meta), _ => meta.extend_from_slice(b"[]") }
    meta.extend_from_slice(b",\"deletedStrokeStamps\":");
    normalize_deletion_stamps(h.get(b"deletedStrokeStamps"), legacy).write_plain(&mut meta);
    meta.extend_from_slice(b",\"hasCustomBackground\":");
    meta.extend_from_slice(if has_custom { b"true" } else { b"false" });
    meta.extend_from_slice(b",\"backgroundSource\":");
    write(&resolved, &mut meta);
    meta.extend_from_slice(b",\"templateKind\":");
    match h.get(b"templateKind") { Some(t) => write(t, &mut meta), None => meta.extend_from_slice(b"null") }
    meta.extend_from_slice(b",\"pdfPageIndex\":");
    if has_custom { write_num(page_index + 1.0, &mut meta); } else { meta.extend_from_slice(b"null"); }
    meta.extend_from_slice(b",\"pageWidth\":");
    write_num(number_field(&h, "pageWidth"), &mut meta);
    meta.extend_from_slice(b",\"pageHeight\":");
    write_num(number_field(&h, "pageHeight"), &mut meta);
    meta.extend_from_slice(b",\"sidePanel\":");
    match h.get(b"sidePanel").filter(|v| v.truthy()) {
        Some(panel) if truthy(h.get(b"renderPanels")) => write(panel, &mut meta),
        _ => meta.extend_from_slice(b"null"),
    }
    meta.push(b'}');

    let mut out: Vec<u8> = Vec::with_capacity(64 + keep.len() * 4 + compact.len() + meta.len());
    out.extend_from_slice(&(keep.len() as u32).to_le_bytes());
    for &i in &keep { out.extend_from_slice(&(i as u32).to_le_bytes()); }
    match &bounds {
        Some(r) => { out.push(1); for v in [r.x, r.y, r.width, r.height] { out.extend_from_slice(&v.to_le_bytes()); } }
        None => { out.push(0); out.extend_from_slice(&[0u8; 32]); }
    }
    out.extend_from_slice(&(compact.len() as u32).to_le_bytes());
    out.extend_from_slice(&compact);
    out.extend_from_slice(&meta);
    Ok(out)
}

/// Overlay bounds alone: `[hasBounds u8][4 x f64]`.
pub fn bounds_only(input: &[u8]) -> Result<Vec<u8>, String> {
    let [head, strokes_json] = split_sections::<2>(input)?;
    let h = header(head)?;
    let items = if strokes_json.is_empty() { Vec::new() } else { parse_strokes(strokes_json)? };
    let rect = overlay_bounds(&items, None, h.get(b"images"), number_field(&h, "width"), number_field(&h, "height"), number_field(&h, "scale"))?;
    let mut out = Vec::with_capacity(33);
    match rect {
        Some(r) => { out.push(1); for v in [r.x, r.y, r.width, r.height] { out.extend_from_slice(&v.to_le_bytes()); } }
        None => { out.push(0); out.extend_from_slice(&[0u8; 32]); }
    }
    Ok(out)
}

// ------------------------------------------------------------------ export plan

/// Export profile, scale/quality selection and the copyPages plan for one export run.
pub fn export_plan(input: &[u8]) -> Result<Vec<u8>, String> {
    let h = header(input)?;
    let high = truthy(h.get(b"highQuality"));
    let optimize = truthy(h.get(b"optimizeForUpload")) && !high;
    let canvas_scale = number_field(&h, "canvasScale");
    let page_count = number_field(&h, "pageCount");
    let mut out: Bytes = Vec::new();
    let raster = match h.get(b"kind") {
        Some(k) if k.is_str("raster") => true,
        Some(k) if k.is_str("pdflib") => false,
        _ => return Err("Unknown export kind".into()),
    };
    if raster {
        let optimize_scale = truthy(h.get(b"optimizeForUpload")) && !high;
        let many = page_count >= 20.0;
        let scale = if high { canvas_scale } else if optimize_scale { if many { 1.1 } else { 1.22 } } else { 1.5 };
        let quality = if high { if many { 0.75 } else { 0.85 } }
            else if optimize_scale { if many { 0.55 } else { 0.62 } }
            else if many { 0.70 } else { 0.75 };
        let mut profile = Vec::new();
        if high { profile.extend_from_slice(b"hq:"); } else { profile.extend_from_slice(if optimize_scale { b"drv-opt:" } else { b"std:" }); }
        fmt_num(scale, &mut profile);
        profile.extend_from_slice(b":q");
        fmt_num(quality, &mut profile);
        out.extend_from_slice(b"{\"exportScale\":");
        write_num(scale, &mut out);
        out.extend_from_slice(b",\"jpegQuality\":");
        write_num(quality, &mut out);
        out.extend_from_slice(b",\"exportProfile\":");
        write_str(&profile, &mut out);
        out.push(b'}');
        return Ok(out);
    }
    let overlay_scale = if high { 3.5f64.min(canvas_scale * 1.75) } else if optimize { 2.0 } else { 2.5 };
    let mut profile = b"pdflib:".to_vec();
    fmt_num(overlay_scale, &mut profile);
    profile.push(b':');
    profile.extend_from_slice(if optimize { b"opt" } else if high { b"hq" } else { b"std" });
    profile.push(b':');
    profile.extend_from_slice(if truthy(h.get(b"rawBackend")) { b"raw" } else { b"png" });
    let original_pages = number_field(&h, "originalPageCount");
    let mut state_to_source: Vec<Option<usize>> = Vec::new();
    let mut sources: Vec<f64> = Vec::new();
    if let Some(V::Arr(pages)) = h.get(b"pages") {
        for page in pages {
            let is_pdf = page.get(b"s").is_some_and(|s| s.is_str("pdf"));
            match finite_number(page.get(b"i")) {
                Some(index) if is_pdf && index >= 1.0 && index <= original_pages => {
                    state_to_source.push(Some(sources.len()));
                    sources.push(index - 1.0);
                }
                _ => state_to_source.push(None),
            }
        }
    }
    out.extend_from_slice(b"{\"overlayScale\":");
    write_num(overlay_scale, &mut out);
    out.extend_from_slice(b",\"exportProfile\":");
    write_str(&profile, &mut out);
    out.extend_from_slice(b",\"stateToSourceIdx\":[");
    for (n, entry) in state_to_source.iter().enumerate() {
        if n > 0 { out.push(b','); }
        match entry { Some(i) => write_num(*i as f64, &mut out), None => out.extend_from_slice(b"null") }
    }
    out.extend_from_slice(b"],\"sourceIndices\":[");
    for (n, i) in sources.iter().enumerate() {
        if n > 0 { out.push(b','); }
        write_num(*i, &mut out);
    }
    out.extend_from_slice(b"]}");
    Ok(out)
}

// ------------------------------------------------------------------ geometry kernels

const PX_TO_PT: f64 = 0.75;

fn px_to_mm(px: f64) -> f64 { 10f64.max(js_round(px * 25.4 / 96.0 * 1000.0) / 1000.0) }

/// `[width, height, mmWidth, mmHeight, landscape]` for a page's stored size.
pub fn page_geometry(out: &mut [f64; 5], page_w: f64, page_h: f64, a4_w: f64, a4_h: f64) {
    let w = normalize_dimension(page_w, a4_w);
    let h = normalize_dimension(page_h, a4_h);
    let (mm_w, mm_h) = (px_to_mm(normalize_dimension(w, a4_w)), px_to_mm(normalize_dimension(h, a4_h)));
    *out = [w, h, mm_w, mm_h, if mm_w > mm_h { 1.0 } else { 0.0 }];
}

/// `[logicalW, logicalH, pxW, pxH, ptW, ptH]`; a copied source page (finite pt size) wins over the stored size.
pub fn pdflib_geometry(out: &mut [f64; 6], page_w: f64, page_h: f64, a4_w: f64, a4_h: f64, copied_w_pt: f64, copied_h_pt: f64) {
    let w = normalize_dimension(page_w, a4_w);
    let h = normalize_dimension(page_h, a4_h);
    if copied_w_pt.is_finite() && copied_h_pt.is_finite() {
        *out = [w, h, copied_w_pt / PX_TO_PT, copied_h_pt / PX_TO_PT, copied_w_pt, copied_h_pt];
    } else {
        *out = [w, h, w, h, w * PX_TO_PT, h * PX_TO_PT];
    }
}

/// `[x, y, width, height]` in PDF points for the cropped overlay image.
pub fn overlay_placement(out: &mut [f64; 4], bx: f64, by: f64, bw: f64, bh: f64, pt_w: f64, pt_h: f64, logical_w: f64, logical_h: f64) {
    let (scale_x, scale_y) = (pt_w / logical_w, pt_h / logical_h);
    *out = [bx * scale_x, pt_h - ((by + bh) * scale_y), bw * scale_x, bh * scale_y];
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(sections: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        for s in sections { out.extend_from_slice(&(s.len() as u32).to_le_bytes()); out.extend_from_slice(s); }
        out
    }
    fn split_out(out: &[u8]) -> (Vec<u32>, Option<[f64; 4]>, String, String) {
        let n = u32::from_le_bytes(out[0..4].try_into().unwrap()) as usize;
        let keep = (0..n).map(|i| u32::from_le_bytes(out[4 + i * 4..8 + i * 4].try_into().unwrap())).collect();
        let at = 4 + n * 4;
        let bounds = if out[at] == 1 { Some(std::array::from_fn(|k| f64::from_le_bytes(out[at + 1 + k * 8..at + 9 + k * 8].try_into().unwrap()))) } else { None };
        let at = at + 33;
        let clen = u32::from_le_bytes(out[at..at + 4].try_into().unwrap()) as usize;
        let compact = String::from_utf8(out[at + 4..at + 4 + clen].to_vec()).unwrap();
        (keep, bounds, compact, String::from_utf8(out[at + 4 + clen..].to_vec()).unwrap())
    }

    #[test] fn metadata_entry_follows_original_key_order() {
        let head = br#"{"mode":"pdflib","pageId":"p1","pageIndex":2,"bgSource":"pdf","pdfIndex":3,"hasBgImage":false,"templateKind":"default",
            "pageWidth":794,"pageHeight":1123,"images":[{"id":"i","src":"data:x"}],"deletedStrokeIds":["a","b"],"deletedStrokeStamps":{"a":{"clock":4,"actor":"z"}},
            "renderPanels":true,"sidePanel":{"n":1},"bounds":{"width":794,"height":1123,"scale":2.5}}"#;
        let strokes = br#"[{"id":"s1","tool":"pen","width":3,"points":[{"x":10.123,"y":20},{"x":30,"y":40.555,"p":0.5}]}]"#;
        let out = page_export(&frame(&[head, strokes, b""])).unwrap();
        let (keep, bounds, compact, meta) = split_out(&out);
        assert_eq!(keep, vec![0]);
        assert!(bounds.is_some());
        assert_eq!(compact, r#"[{"id":"s1","tool":"pen","width":3,"points":[{"x":10.12,"y":20},{"x":30,"y":40.56,"p":0.5}]}]"#);
        assert_eq!(meta, format!(r#"{{"pageId":"p1","strokes":{compact},"images":[{{"id":"i","src":"data:x"}}],"deletedStrokeIds":["a","b"],"deletedStrokeStamps":{{"a":{{"clock":4,"actor":"z"}},"b":{{"clock":0,"actor":""}}}},"hasCustomBackground":true,"backgroundSource":"pdf","templateKind":"default","pdfPageIndex":3,"pageWidth":794,"pageHeight":1123,"sidePanel":{{"n":1}}}}"#));
    }
    #[test] fn stamps_merge_keep_maximum_and_sort() {
        let stamps = P::document(br#"[{"id":"b","stamp":{"clock":2,"actor":"x"}},{"id":"b","clock":5,"actor":"a"},"c"]"#).unwrap();
        let legacy = P::document(br#"["a",0,"b",null]"#).unwrap();
        let mut out = Vec::new();
        normalize_deletion_stamps(Some(&stamps), Some(&legacy)).write_plain(&mut out);
        assert_eq!(String::from_utf8(out).unwrap(), r#"{"a":{"clock":0,"actor":""},"b":{"clock":5,"actor":"a"},"c":{"clock":0,"actor":""}}"#);
    }
    #[test] fn export_plan_matches_original_profiles() {
        let plan = |s: &str| String::from_utf8(export_plan(s.as_bytes()).unwrap()).unwrap();
        assert!(plan(r#"{"kind":"raster","pageCount":3,"canvasScale":2}"#).contains(r#""exportProfile":"std:1.5:q0.75""#));
        assert!(plan(r#"{"kind":"raster","pageCount":25,"optimizeForUpload":true,"canvasScale":2}"#).contains(r#""exportProfile":"drv-opt:1.1:q0.55""#));
        assert!(plan(r#"{"kind":"raster","pageCount":25,"highQuality":true,"optimizeForUpload":true,"canvasScale":2}"#).contains(r#""exportProfile":"hq:2:q0.75""#));
        assert!(plan(r#"{"kind":"pdflib","canvasScale":2,"highQuality":true}"#).contains(r#""exportProfile":"pdflib:3.5:hq:png""#));
        let copy = plan(r#"{"kind":"pdflib","canvasScale":2,"originalPageCount":3,"rawBackend":true,"pages":[{"s":"pdf","i":2},{"s":"template"},{"s":"pdf","i":9},null,{"s":"pdf","i":1}]}"#);
        assert!(copy.contains(r#""stateToSourceIdx":[0,null,null,null,1],"sourceIndices":[1,0]"#), "{copy}");
        assert!(copy.contains("pdflib:2.5:std:raw"));
    }
    #[test] fn geometry_and_placement() {
        let mut g = [0.0; 5];
        page_geometry(&mut g, 794.0, 1123.0, 794.0, 1123.0);
        assert_eq!((g[0], g[1], g[2], g[3], g[4]), (794.0, 1123.0, 210.079, 297.127, 0.0));
        page_geometry(&mut g, f64::NAN, 10.0, 794.0, 1123.0);
        assert_eq!((g[0], g[1]), (794.0, 64.0));
        let mut p = [0.0; 4];
        overlay_placement(&mut p, 10.0, 20.0, 100.0, 200.0, 595.0, 842.0, 794.0, 1123.0);
        assert_eq!(p[2], 100.0 * (595.0 / 794.0));
        assert_eq!(p[1], 842.0 - ((20.0 + 200.0) * (842.0 / 1123.0)));
    }
    #[test] fn truncated_frames_are_errors() {
        assert!(page_export(b"\x05\0\0\0{").is_err());
        assert!(page_hash(b"").is_err());
        assert!(export_plan(b"[]").is_err());
    }
}
