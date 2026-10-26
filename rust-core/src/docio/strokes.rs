//! Stroke arrays for PDF export: duplicate removal, point compaction, overlay
//! bounds and the sync fingerprints that make up the page hash.

use super::jv::*;

/// A point coordinate as `JSON.parse` would hand it to the original code.
pub enum Sc { Undef, Num(f64), Other(Box<V>) }

impl Sc {
    fn number(&self) -> f64 {
        match self { Sc::Undef => f64::NAN, Sc::Num(n) => *n, Sc::Other(v) => to_number(Some(v)) }
    }
    /// `Number.isFinite(value)`.
    fn finite(&self) -> Option<f64> { match self { Sc::Num(n) if n.is_finite() => Some(*n), _ => None } }
    /// `value != null`.
    fn present(&self) -> bool {
        match self { Sc::Undef => false, Sc::Num(_) => true, Sc::Other(v) => !matches!(**v, V::Null) }
    }
}

pub struct Pt { pub null: bool, pub x: Sc, pub y: Sc, pub p: Sc, pub start: usize, pub end: usize }

pub enum Field { V(V), Pts(Vec<Pt>) }

pub struct Stroke { pub fields: Vec<(Bytes, Field)> }

pub enum Item { Falsy, Other, Obj(Stroke) }

impl Stroke {
    pub fn get(&self, key: &[u8]) -> Option<&Field> { self.fields.iter().find(|(k, _)| k == key).map(|(_, f)| f) }
    pub fn value(&self, key: &[u8]) -> Option<&V> { match self.get(key) { Some(Field::V(v)) => Some(v), _ => None } }
    pub fn points(&self) -> &[Pt] { match self.get(b"points") { Some(Field::Pts(p)) => p, _ => &[] } }
}

fn scalar(p: &mut P) -> Result<Sc, String> {
    p.ws();
    match p.peek() {
        Some(b'-' | b'0'..=b'9') => Ok(Sc::Num(p.number()?)),
        _ => Ok(Sc::Other(Box::new(p.value()?))),
    }
}

fn parse_points(p: &mut P, key: &mut Bytes) -> Result<Vec<Pt>, String> {
    p.i += 1; // '['
    let mut points = Vec::new();
    p.ws();
    if p.peek() == Some(b']') { p.i += 1; return Ok(points); }
    loop {
        p.ws();
        let start = p.i;
        let mut pt = Pt { null: false, x: Sc::Undef, y: Sc::Undef, p: Sc::Undef, start, end: start };
        match p.peek() {
            Some(b'{') => {
                p.i += 1;
                p.ws();
                if p.peek() == Some(b'}') { p.i += 1; } else {
                    loop {
                        p.ws();
                        key.clear();
                        p.string(Some(key))?;
                        p.expect(b':')?;
                        match key.as_slice() {
                            b"x" => pt.x = scalar(p)?,
                            b"y" => pt.y = scalar(p)?,
                            b"p" => pt.p = scalar(p)?,
                            _ => p.skip_value()?,
                        }
                        p.ws();
                        match p.peek() {
                            Some(b',') => p.i += 1,
                            Some(b'}') => { p.i += 1; break; }
                            _ => return p.err("expected ',' or '}'"),
                        }
                    }
                }
            }
            Some(b'n') => { p.skip_value()?; pt.null = true; }
            _ => p.skip_value()?,
        }
        pt.end = p.i;
        points.push(pt);
        p.ws();
        match p.peek() {
            Some(b',') => p.i += 1,
            Some(b']') => { p.i += 1; break; }
            _ => return p.err("expected ',' or ']'"),
        }
    }
    Ok(points)
}

fn parse_stroke(p: &mut P) -> Result<Stroke, String> {
    p.i += 1; // '{'
    let mut fields: Vec<(Bytes, Field)> = Vec::new();
    let mut key = Vec::new();
    let mut scratch = Vec::new();
    p.ws();
    if p.peek() == Some(b'}') { p.i += 1; return Ok(Stroke { fields }); }
    loop {
        p.ws();
        key.clear();
        p.string(Some(&mut key))?;
        p.expect(b':')?;
        p.ws();
        let field = if key == b"points" && p.peek() == Some(b'[') { Field::Pts(parse_points(p, &mut scratch)?) } else { Field::V(p.value()?) };
        match fields.iter().position(|(k, _)| *k == key) {
            Some(n) => fields[n].1 = field,
            None => fields.push((key.clone(), field)),
        }
        p.ws();
        match p.peek() {
            Some(b',') => p.i += 1,
            Some(b'}') => { p.i += 1; break; }
            _ => return p.err("expected ',' or '}'"),
        }
    }
    js_property_order(&mut fields);
    Ok(Stroke { fields })
}

/// Parse a JSON array of strokes. Points inside strokes keep only what the export reads.
pub fn parse_strokes(json: &[u8]) -> Result<Vec<Item>, String> {
    let mut p = P::new(json);
    p.ws();
    if p.peek() != Some(b'[') { return p.err("strokes must be an array"); }
    p.i += 1;
    let mut items = Vec::new();
    p.ws();
    if p.peek() == Some(b']') { p.i += 1; p.end()?; return Ok(items); }
    loop {
        p.ws();
        match p.peek() {
            Some(b'{') => items.push(Item::Obj(parse_stroke(&mut p)?)),
            Some(_) => {
                let v = p.value()?;
                items.push(if v.truthy() { Item::Other } else { Item::Falsy });
            }
            None => return p.err("unexpected end"),
        }
        p.ws();
        match p.peek() {
            Some(b',') => p.i += 1,
            Some(b']') => { p.i += 1; break; }
            _ => return p.err("expected ',' or ']'"),
        }
    }
    p.end()?;
    Ok(items)
}

pub fn write_field(f: &Field, input: &[u8], out: &mut Bytes) -> Result<(), String> {
    match f {
        Field::V(v) => write(v, out),
        Field::Pts(points) => write_points_canonical(points, input, out)?,
    }
    Ok(())
}

/// Original (uncompacted) points exactly as `JSON.stringify` would print the parsed array.
fn write_points_canonical(points: &[Pt], input: &[u8], out: &mut Bytes) -> Result<(), String> {
    out.push(b'[');
    for (n, pt) in points.iter().enumerate() {
        if n > 0 { out.push(b','); }
        let v = P::document(&input[pt.start..pt.end])?;
        write(&v, out);
    }
    out.push(b']');
    Ok(())
}

#[derive(Clone, Copy, PartialEq)]
pub enum Mode { PdfLib, Raster }

/// Indices of the strokes that survive the original duplicate filter.
pub fn dedupe(items: &[Item], input: &[u8]) -> Result<Vec<usize>, String> {
    let mut keep = Vec::with_capacity(items.len());
    if items.len() <= 1 {
        keep.extend(0..items.len());
        return Ok(keep);
    }
    let mut seen = std::collections::HashSet::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let Item::Obj(stroke) = item else { return Err("Cannot read properties of a non-object stroke".into()) };
        let mut fp: Bytes = Vec::new();
        match stroke.value(b"id").filter(|v| v.truthy()) {
            Some(id) => { fp.extend_from_slice(b"id:"); fp.extend_from_slice(&to_js_string(id)); }
            None => {
                fp.push(b'[');
                for (n, key) in [&b"tool"[..], b"color", b"width"].iter().enumerate() {
                    if n > 0 { fp.push(b','); }
                    match stroke.value(key) { Some(v) => write(v, &mut fp), None => fp.extend_from_slice(b"null") }
                }
                fp.push(b',');
                match stroke.get(b"points") {
                    Some(Field::Pts(points)) => write_points_canonical(points, input, &mut fp)?,
                    _ => fp.extend_from_slice(b"[]"),
                }
                fp.push(b']');
            }
        }
        if seen.insert(fp) { keep.push(index); }
    }
    Ok(keep)
}

fn write_compact_points(points: &[Pt], out: &mut Bytes) -> Result<(), String> {
    out.push(b'[');
    for (n, pt) in points.iter().enumerate() {
        if pt.null { return Err("Cannot read properties of null (reading 'x')".into()); }
        if n > 0 { out.push(b','); }
        out.extend_from_slice(b"{\"x\":");
        write_r2(pt.x.number(), out);
        out.extend_from_slice(b",\"y\":");
        write_r2(pt.y.number(), out);
        if pt.p.present() {
            out.extend_from_slice(b",\"p\":");
            write_r2(pt.p.number(), out);
        }
        out.push(b'}');
    }
    out.push(b']');
    Ok(())
}

/// `JSON.stringify(Math.round(v * 100) / 100)`.
fn write_r2(v: f64, out: &mut Bytes) {
    let r = js_round(v * 100.0);
    if r.is_finite() { fmt_r2(r, out) } else { out.extend_from_slice(b"null") }
}

/// `{...stroke, points: <compacted>}` as JSON, following the original per-path rules.
pub fn write_compact_stroke(stroke: &Stroke, mode: Mode, input: &[u8], out: &mut Bytes) -> Result<(), String> {
    out.push(b'{');
    let mut wrote_points = false;
    for (n, (key, field)) in stroke.fields.iter().enumerate() {
        if n > 0 { out.push(b','); }
        write_str(key, out);
        out.push(b':');
        if key == b"points" {
            wrote_points = true;
            match field {
                Field::Pts(points) if !points.is_empty() => write_compact_points(points, out)?,
                Field::Pts(_) => out.extend_from_slice(b"[]"),
                Field::V(v) => if mode == Mode::PdfLib { out.extend_from_slice(b"[]") } else { write(v, out) },
            }
        } else { write_field(field, input, out)?; }
    }
    if !wrote_points && mode == Mode::PdfLib {
        if !stroke.fields.is_empty() { out.push(b','); }
        out.extend_from_slice(b"\"points\":[]");
    }
    out.push(b'}');
    Ok(())
}

// ------------------------------------------------------------------ bounds

pub struct Rect { pub x: f64, pub y: f64, pub width: f64, pub height: f64 }

#[allow(clippy::too_many_arguments)]
pub fn overlay_bounds(items: &[Item], keep: Option<&[usize]>, images: Option<&V>, page_width: f64, page_height: f64, scale: f64) -> Result<Option<Rect>, String> {
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    let mut include = |x1: f64, y1: f64, x2: f64, y2: f64| {
        if !(x1.is_finite() && y1.is_finite() && x2.is_finite() && y2.is_finite()) { return; }
        min_x = min_x.min(x1).min(x2);
        min_y = min_y.min(y1).min(y2);
        max_x = max_x.max(x1).max(x2);
        max_y = max_y.max(y1).max(y2);
    };
    let mut visit = |item: &Item| -> Result<(), String> {
        let stroke = match item { Item::Obj(stroke) => stroke, Item::Falsy => return Ok(()), Item::Other => return Ok(()) };
        let tool = stroke.value(b"tool");
        if tool.is_some_and(|t| t.is_str("eraser-stroke")) { return Ok(()); }
        let points = stroke.points();
        let highlighter = tool.is_some_and(|t| t.is_str("highlighter"));
        let min_points = if highlighter { 1 } else { 2 };
        if points.len() < min_points { return Ok(()); }
        let (mut sx0, mut sy0, mut sx1, mut sy1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        for pt in points {
            if pt.null { continue; }
            let (x, y) = (pt.x.number(), pt.y.number());
            if !x.is_finite() || !y.is_finite() { continue; }
            sx0 = sx0.min(x); sy0 = sy0.min(y); sx1 = sx1.max(x); sy1 = sy1.max(y);
        }
        if !sx0.is_finite() { return Ok(()); }
        let raw_width = to_number(stroke.value(b"width"));
        let width = if highlighter { 4f64.max(if raw_width != 0.0 && !raw_width.is_nan() { raw_width } else { 12.0 }) }
            else { 1f64.max(if raw_width != 0.0 && !raw_width.is_nan() { raw_width } else { 1.0 }) };
        // Two logical pixels cover round caps and antialiasing.
        let padding = width * 0.52 + 2.0;
        include(sx0 - padding, sy0 - padding, sx1 + padding, sy1 + padding);
        Ok(())
    };
    match keep {
        Some(keep) => for &i in keep { visit(&items[i])?; },
        None => for item in items { visit(item)?; },
    }
    if let Some(V::Arr(list)) = images {
        for image in list {
            let num = |key: &[u8]| to_number(image.get(key));
            let (x, y, width, height) = (num(b"x"), num(b"y"), num(b"width"), num(b"height"));
            if !(x.is_finite() && y.is_finite() && width.is_finite() && height.is_finite()) || width <= 0.0 || height <= 0.0 { continue; }
            let rotation = num(b"rotation");
            let angle = if rotation != 0.0 && !rotation.is_nan() { rotation } else { 0.0 };
            let (cos, sin) = (angle.cos().abs(), angle.sin().abs());
            let half_width = (width * cos + height * sin) / 2.0;
            let half_height = (width * sin + height * cos) / 2.0;
            include(x - half_width - 2.0, y - half_height - 2.0, x + half_width + 2.0, y + half_height + 2.0);
        }
    }
    if !(min_x.is_finite() && min_y.is_finite() && max_x.is_finite() && max_y.is_finite()) { return Ok(None); }
    let positive = |v: f64| if v != 0.0 && !v.is_nan() { v } else { 1.0 };
    let safe_width = 1f64.max(positive(page_width));
    let safe_height = 1f64.max(positive(page_height));
    let safe_scale = 1f64.max(positive(scale));
    let x1 = (0f64.max(min_x) * safe_scale).floor() / safe_scale;
    let y1 = (0f64.max(min_y) * safe_scale).floor() / safe_scale;
    let x2 = safe_width.min((safe_width.min(max_x) * safe_scale).ceil() / safe_scale);
    let y2 = safe_height.min((safe_height.min(max_y) * safe_scale).ceil() / safe_scale);
    if x2 <= x1 || y2 <= y1 { return Ok(None); }
    Ok(Some(Rect { x: x1, y: y1, width: x2 - x1, height: y2 - y1 }))
}

// ------------------------------------------------------------------ sync fingerprints

/// `simpleHash` without allocating the joined string.
pub struct Hasher(i32);
impl Hasher {
    pub fn new() -> Self { Hasher(0) }
    #[inline] pub fn unit(&mut self, unit: u16) { self.0 = (self.0 << 5).wrapping_sub(self.0).wrapping_add(unit as i32); }
    pub fn feed(&mut self, bytes: &[u8]) {
        if bytes.is_ascii() { for &b in bytes { self.unit(b as u16); } }
        else { for u in utf16_units(bytes) { self.unit(u); } }
    }
    pub fn finish(&self, out: &mut Bytes) {
        let mut value = (self.0 as i64).unsigned_abs();
        let mut digits = [0u8; 16];
        let mut n = 0;
        if value == 0 { digits[0] = b'0'; n = 1; }
        while value > 0 { let d = (value % 36) as u8; digits[n] = if d < 10 { b'0' + d } else { b'a' + d - 10 }; value /= 36; n += 1; }
        let n = n.min(8);
        // Longest possible value (2^31) has 6 base-36 digits, so the slice(0, 8) never trims.
        for k in (0..n).rev() { out.push(digits[k]); }
    }
}

pub fn simple_hash(bytes: &[u8], out: &mut Bytes) { let mut h = Hasher::new(); h.feed(bytes); h.finish(out); }

fn rounded(v: Option<f64>, factor: f64) -> f64 { match v { Some(n) => js_round(n * factor) / factor, None => 0.0 } }

/// `strokeSyncFingerprint(stroke)`.
pub fn stroke_fingerprint(item: &Item, out: &mut Bytes) {
    let Item::Obj(stroke) = item else {
        if matches!(item, Item::Other) { out.extend_from_slice(b"|||0|0|0"); }
        return;
    };
    let text = |key: &[u8], out: &mut Bytes| if let Some(v) = stroke.value(key).filter(|v| v.truthy()) { out.extend_from_slice(&to_js_string(v)); };
    text(b"id", out); out.push(b'|');
    text(b"tool", out); out.push(b'|');
    text(b"color", out); out.push(b'|');
    fmt_num(rounded(finite_number(stroke.value(b"width")), 100.0), out);
    out.push(b'|');
    let points = stroke.points();
    fmt_num(points.len() as f64, out);
    out.push(b'|');
    let mut hasher = Hasher::new();
    let mut piece: Bytes = Vec::with_capacity(48);
    for (n, pt) in points.iter().enumerate() {
        if n > 0 { hasher.unit(b';' as u16); }
        piece.clear();
        if pt.null { piece.extend_from_slice(b"0,0,0"); } else {
            for (k, sc) in [&pt.x, &pt.y, &pt.p].into_iter().enumerate() {
                if k > 0 { piece.push(b','); }
                match sc.finite() { Some(v) => { let r = js_round(v * 100.0); if r.is_finite() { fmt_r2(r, &mut piece) } else { fmt_num(r / 100.0, &mut piece) } } None => piece.push(b'0') }
            }
        }
        hasher.feed(&piece);
    }
    hasher.finish(out);
}

/// `imageSyncFingerprint(image)`.
pub fn image_fingerprint(image: &V, out: &mut Bytes) -> Result<(), String> {
    if !image.truthy() { return Ok(()); }
    let get = |key: &[u8]| image.get(key);
    if let Some(id) = get(b"id").filter(|v| v.truthy()) { out.extend_from_slice(&to_js_string(id)); }
    out.push(b'|');
    match get(b"src").filter(|v| v.truthy()) {
        Some(V::Str(src)) => simple_hash(src, out),
        Some(V::Arr(items)) if !items.is_empty() => return Err("Image source must be a string".into()),
        _ => out.push(b'0'),
    }
    for (key, factor) in [(&b"x"[..], 100.0), (b"y", 100.0), (b"width", 100.0), (b"height", 100.0), (b"rotation", 10000.0)] {
        out.push(b'|');
        fmt_num(rounded(finite_number(get(key)), factor), out);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(text: &str) -> String { let mut out = Vec::new(); simple_hash(text.as_bytes(), &mut out); String::from_utf8(out).unwrap() }
    #[test] fn simple_hash_matches_javascript_reference_values() {
        // Values produced by the original simpleHash in Node.
        assert_eq!(hash(""), "0");
        assert_eq!(hash("a"), "2p");
        assert_eq!(hash("hello"), "1n1e4y");
        assert_eq!(hash("12.5,30,0.5;12.6,30.1,0"), "7pzd7s");
        assert_eq!(hash("\u{e9}\u{1f600}x"), "10urcc");
    }

    fn compact(json: &str, mode: Mode) -> Result<String, String> {
        let items = parse_strokes(json.as_bytes())?;
        let keep = dedupe(&items, json.as_bytes())?;
        let mut out = vec![b'['];
        for (n, &i) in keep.iter().enumerate() {
            if n > 0 { out.push(b','); }
            let Item::Obj(s) = &items[i] else { return Err("non-object".into()) };
            write_compact_stroke(s, mode, json.as_bytes(), &mut out)?;
        }
        out.push(b']');
        Ok(String::from_utf8(out).unwrap())
    }

    #[test] fn dedupe_by_id_and_by_geometry() {
        let json = r##"[{"id":"a","points":[{"x":1,"y":2}]},{"id":"a","points":[{"x":9,"y":9}]},
            {"tool":"pen","color":"#000","width":2,"points":[{"x":1,"y":2,"t":5}]},
            {"tool":"pen","color":"#000","width":2,"points":[{"x":1,"y":2,"t":5}]},
            {"tool":"pen","color":"#000","width":2,"points":[{"x":1,"y":2,"t":6}]}]"##;
        let items = parse_strokes(json.as_bytes()).unwrap();
        assert_eq!(dedupe(&items, json.as_bytes()).unwrap(), vec![0, 2, 4]);
    }
    #[test] fn compaction_rounds_and_drops_missing_pressure() {
        let out = compact(r#"[{"id":"s","points":[{"x":1.005,"y":2.4999,"p":0.123456,"t":9},{"x":3,"y":4},{"x":5,"y":6,"p":null}],"color":"red"}]"#, Mode::PdfLib).unwrap();
        assert_eq!(out, r#"[{"id":"s","points":[{"x":1,"y":2.5,"p":0.12},{"x":3,"y":4},{"x":5,"y":6}],"color":"red"}]"#);
    }
    #[test] fn points_key_rules_differ_between_paths() {
        assert_eq!(compact(r#"[{"id":"s"}]"#, Mode::PdfLib).unwrap(), r#"[{"id":"s","points":[]}]"#);
        assert_eq!(compact(r#"[{"id":"s"}]"#, Mode::Raster).unwrap(), r#"[{"id":"s"}]"#);
        assert_eq!(compact(r#"[{"points":null,"id":"s"}]"#, Mode::PdfLib).unwrap(), r#"[{"points":[],"id":"s"}]"#);
        assert_eq!(compact(r#"[{"points":null,"id":"s"}]"#, Mode::Raster).unwrap(), r#"[{"points":null,"id":"s"}]"#);
        assert_eq!(compact(r#"[{}]"#, Mode::PdfLib).unwrap(), r#"[{"points":[]}]"#);
    }
    #[test] fn null_point_is_an_error_like_the_typeerror() {
        assert!(compact(r#"[{"id":"s","points":[null]}]"#, Mode::PdfLib).is_err());
    }
    #[test] fn bounds_pad_pen_and_include_images() {
        let json = r#"[{"tool":"pen","width":4,"points":[{"x":10,"y":20},{"x":30,"y":40}]},{"tool":"eraser-stroke","points":[{"x":900,"y":900},{"x":1,"y":1}]}]"#;
        let items = parse_strokes(json.as_bytes()).unwrap();
        let r = overlay_bounds(&items, None, None, 794.0, 1123.0, 2.5).unwrap().unwrap();
        // padding = 4 * 0.52 + 2 = 4.08
        assert_eq!((r.x, r.y), ((5.92f64 * 2.5).floor() / 2.5, (15.92f64 * 2.5).floor() / 2.5));
        assert!(overlay_bounds(&[], None, None, 794.0, 1123.0, 2.0).unwrap().is_none());
    }
    #[test] fn fingerprint_shape() {
        let items = parse_strokes(br##"[{"id":"a","tool":"pen","color":"#111","width":2.345,"points":[{"x":1,"y":2,"p":0.5}]}, null]"##).unwrap();
        let mut out = Vec::new();
        stroke_fingerprint(&items[0], &mut out);
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with("a|pen|#111|2.35|1|"), "{text}");
        let mut empty = Vec::new();
        stroke_fingerprint(&items[1], &mut empty);
        assert!(empty.is_empty());
    }
}
