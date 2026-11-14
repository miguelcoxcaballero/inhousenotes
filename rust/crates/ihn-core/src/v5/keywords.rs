//! The `/Keywords` container of a 5.x PDF (spec/documents.md §3): section location and the two payload encodings.

use super::inflate::{inflate, InflateError};
use super::json::{self, Value};
use super::{V5Error, MAX_COLLAB_SECTION, MAX_SECTION_ENCODED, MAX_SECTION_INFLATED, MAX_SYNC_SECTION};
use crate::codec::b64_decode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayRef<'a> {
    /// A pdf-lib object number.
    Object(u32),
    /// The XObject name in the page's Resources.
    Name(&'a str),
}

/// `IH_SYNC`: `{v: 1, writer, contentHash, contentVersion}`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SyncEnvelope {
    pub writer: String,
    pub content_hash: String,
    pub content_version: f64,
}

/// The sections of a Keywords string, still encoded. Located exactly as 5.18.0 locates them.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Keywords<'a> {
    /// `STROKES_Z:` (true) or `STROKES_DATA:` (false) at offset 0, up to the first `;`.
    pub strokes: Option<(bool, &'a str)>,
    /// `;IH_NORM:1` present.
    pub normalized: bool,
    /// `IH_OVERLAY_REFS:` — 0-based page index → overlay XObject.
    pub overlay_refs: Vec<(i64, OverlayRef<'a>)>,
    pub structure: Option<&'a str>,
    pub fields: Option<&'a str>,
    pub sync: Option<&'a str>,
    pub versions: Option<&'a str>,
}

fn section<'a>(k: &'a str, marker: &str) -> Option<&'a str> {
    let rest = &k[k.find(marker)? + marker.len()..];
    Some(rest.split(';').next().unwrap_or(rest))
}

/// `parseInt(text, 10)`.
fn parse_int(text: &str) -> Option<i64> {
    let t = text.trim_start();
    let (neg, digits) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    let end = digits.bytes().position(|b| !b.is_ascii_digit()).unwrap_or(digits.len());
    let n: i64 = digits[..end].parse().ok()?;
    Some(if neg { -n } else { n })
}

/// True for a 5.x document (`STROKES_` at offset 0); a v6 container (`;IHN6:`) is checked first by the caller.
pub fn is_v5(keywords: &str) -> bool { keywords.starts_with("STROKES_") }

pub fn parse(k: &str) -> Keywords<'_> {
    let first = |rest: &'_ str| -> &'_ str { rest.split(';').next().unwrap_or(rest) };
    let strokes = k.strip_prefix("STROKES_Z:").map(|r| (true, r)).or_else(|| k.strip_prefix("STROKES_DATA:").map(|r| (false, r)));
    let mut overlay_refs: Vec<(i64, OverlayRef)> = Vec::new();
    for entry in section(k, "IH_OVERLAY_REFS:").unwrap_or("").split(',') {
        let Some((page, value)) = entry.split_once('=') else { continue };
        let value = value.trim();
        let Some(page) = parse_int(page).filter(|_| !value.is_empty()) else { continue };
        let r = match value.parse::<u32>() {
            Ok(n) if value.bytes().all(|b| b.is_ascii_digit()) => OverlayRef::Object(n),
            _ => OverlayRef::Name(value),
        };
        // A Map: a repeated page keeps its first position and takes the last value.
        match overlay_refs.iter_mut().find(|(p, _)| *p == page) {
            Some(slot) => slot.1 = r,
            None => overlay_refs.push((page, r)),
        }
    }
    Keywords {
        strokes: strokes.map(|(z, r)| (z, first(r))),
        normalized: k.contains(";IH_NORM:1"),
        overlay_refs,
        structure: section(k, ";IH_STRUCT:"),
        fields: section(k, ";IH_FIELDS:"),
        sync: section(k, ";IH_SYNC:"),
        versions: section(k, "IH_VERSIONS:"),
    }
}

/// The section names in order (`split(';')`, text before `:`), as the fixture records list them.
pub fn section_names(k: &str) -> Vec<&str> { k.split(';').map(|part| part.split(':').next().unwrap_or("")).filter(|n| !n.is_empty()).collect() }

/// `decodeURIComponent(escape(atob(encoded)))`: base64 → strict UTF-8.
pub fn b64_utf8(encoded: &str) -> Result<Vec<u8>, V5Error> {
    let bytes = b64_decode(encoded.as_bytes()).ok_or(V5Error::Base64)?;
    std::str::from_utf8(&bytes).map_err(|_| V5Error::Utf8)?;
    Ok(bytes)
}

/// A `B64J` section → value.
pub fn b64_json(encoded: &str) -> Result<Value, V5Error> { json::parse(&b64_utf8(encoded)?).ok_or(V5Error::Json) }

/// A `B64Z` section → inflated bytes (zlib-wrapped deflate), within 5.18.0's limits.
pub fn b64_zlib(encoded: &str) -> Result<Vec<u8>, V5Error> {
    if encoded.len() > MAX_SECTION_ENCODED {
        return Err(V5Error::PayloadTooLarge);
    }
    let bytes = b64_decode(encoded.as_bytes()).ok_or(V5Error::Base64)?;
    inflate(&bytes, MAX_SECTION_INFLATED).map_err(|e| match e {
        InflateError::TooLarge => V5Error::PayloadExpands,
        InflateError::Corrupt => V5Error::Inflate,
    })
}

/// The JSON text of the `STROKES_*` section (`parseEmbeddedMetadataPayload`): inflated, or base64-decoded UTF-8.
pub fn strokes_json(compressed: bool, payload: &str) -> Result<Vec<u8>, V5Error> {
    if payload.len() > MAX_SECTION_ENCODED {
        return Err(V5Error::PayloadTooLarge);
    }
    let out = if compressed { b64_zlib(payload)? } else { b64_utf8(payload)? };
    if out.len() > MAX_SECTION_INFLATED {
        return Err(V5Error::PayloadExpands);
    }
    Ok(out)
}

/// `IH_STRUCT` → raw value (normalise with `structfields::structure`). `Ok(None)` when the section is absent.
pub fn structure_value(k: &Keywords) -> Result<Option<Value>, V5Error> {
    match k.structure {
        Some(s) if s.len() > MAX_COLLAB_SECTION => Err(V5Error::StructureTooLarge),
        Some(s) => b64_json(s).map(Some),
        None => Ok(None),
    }
}

pub fn fields_value(k: &Keywords) -> Result<Option<Value>, V5Error> {
    match k.fields {
        Some(s) if s.len() > MAX_COLLAB_SECTION => Err(V5Error::FieldsTooLarge),
        Some(s) => b64_json(s).map(Some),
        None => Ok(None),
    }
}

/// `decodeDriveSyncEnvelope`: valid only with `v === 1` and a truthy `contentHash`.
pub fn sync_envelope(k: &Keywords) -> Option<SyncEnvelope> {
    let v = b64_json(k.sync.filter(|s| s.len() <= MAX_SYNC_SECTION)?).ok()?;
    (v.get("v").finite() == Some(1.0) && v.get("contentHash").truthy()).then(|| SyncEnvelope {
        writer: v.get("writer").text(),
        content_hash: v.get("contentHash").text(),
        content_version: v.get("contentVersion").num(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::b64_encode;

    #[test]
    fn locates_sections_like_5_18_0() {
        let st = b64_encode(br#"{"v":1,"clock":5,"entries":{}}"#, false);
        let sy = b64_encode(br#"{"v":1,"writer":"w","contentHash":"abc","contentVersion":7}"#, false);
        let k = format!("STROKES_Z:AAAA;IH_NORM:1;IH_OVERLAY_REFS:0=IHOverlay-1, 2=17,x=3,4=,5=a,5=b;IH_STRUCT:{st};IH_TEST:1;IH_SYNC:{sy};IH_VERSIONS:QQ==");
        let p = parse(&k);
        assert_eq!(p.strokes, Some((true, "AAAA")));
        assert!(p.normalized);
        assert_eq!(p.overlay_refs, [(0, OverlayRef::Name("IHOverlay-1")), (2, OverlayRef::Object(17)), (5, OverlayRef::Name("b"))]);
        assert_eq!((p.structure, p.fields, p.versions), (Some(st.as_str()), None, Some("QQ==")));
        assert_eq!(structure_value(&p).unwrap().unwrap().get("clock").num(), 5.0);
        assert_eq!(fields_value(&p), Ok(None));
        assert_eq!(sync_envelope(&p).unwrap(), SyncEnvelope { writer: "w".into(), content_hash: "abc".into(), content_version: 7.0 });
        assert_eq!(section_names(&k), ["STROKES_Z", "IH_NORM", "IH_OVERLAY_REFS", "IH_STRUCT", "IH_TEST", "IH_SYNC", "IH_VERSIONS"]);

        // STROKES only at offset 0; DATA form; third-party keywords.
        assert_eq!(parse("x;STROKES_Z:AAAA").strokes, None);
        assert_eq!(parse("STROKES_DATA:W10=").strokes, Some((false, "W10=")));
        assert!(is_v5("STROKES_DATA:") && !is_v5("my, keywords") && !is_v5(" STROKES_Z:"));
        assert_eq!(strokes_json(false, "W10=").as_deref(), Ok(&b"[]"[..]));
        assert_eq!(parse("STROKES_Z:A;IH_NORM:10").normalized, true); // `includes`, as 5.18.0
    }

    #[test]
    fn decoding_failures_carry_5_18_0_messages() {
        assert_eq!(b64_json("%%%"), Err(V5Error::Base64));
        assert_eq!(b64_json(&b64_encode(&[0xff, 0xfe], false)), Err(V5Error::Utf8));
        assert_eq!(b64_json(&b64_encode(b"{", false)), Err(V5Error::Json));
        assert_eq!(strokes_json(true, &b64_encode(b"not zlib!", false)), Err(V5Error::Inflate));
        let big = "A".repeat(MAX_COLLAB_SECTION + 1);
        let k = format!("STROKES_Z:A;IH_STRUCT:{big};IH_FIELDS:{big}");
        let p = parse(&k);
        assert_eq!(structure_value(&p).unwrap_err().message(), "Collaboration structure metadata is too large");
        assert_eq!(fields_value(&p).unwrap_err().message(), "Collaboration field metadata is too large");
        assert_eq!(V5Error::PayloadTooLarge.message(), "PDF metadata payload is too large");
        assert_eq!(V5Error::PayloadExpands.message(), "PDF metadata expands beyond the safe limit");
        // A sync envelope over 16384 characters or without a hash is ignored.
        assert_eq!(sync_envelope(&parse(&format!("STROKES_Z:A;IH_SYNC:{}", b64_encode(br#"{"v":1,"contentHash":""}"#, false)))), None);
    }
}
