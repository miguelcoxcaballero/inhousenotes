//! 5.x readers (ARCHITECTURE §9, §10): the `/Keywords` container, page entries, structure and field registers, the
//! timeline archive, IndexedDB records, packed ink, and the deterministic conversion to `Batch`es.
//! Owned by W2.2. Nothing here writes 5.x data.

pub mod checkpoint;
pub mod idb;
pub mod import;
pub mod inflate;
pub mod json;
pub mod keywords;
pub mod pages;
pub mod sha256;
pub mod structfields;
pub mod timeline;

use crate::jsfmt;

// ------------------------------------------------------------------------------------- security-core limits

/// `security-core-v5.js`: largest PDF read from any source.
pub const MAX_PDF_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_PDF_PAGES: u32 = 2000;
/// pdf.js `maxImageSize`.
pub const MAX_PDF_IMAGE_PIXELS: u32 = 40_000_000;
/// `STROKES_*` / `IH_VERSIONS`: encoded characters and inflated bytes.
pub const MAX_SECTION_ENCODED: usize = 48 * 1024 * 1024;
pub const MAX_SECTION_INFLATED: usize = 96 * 1024 * 1024;
/// `IH_STRUCT` / `IH_FIELDS` encoded characters.
pub const MAX_COLLAB_SECTION: usize = 8 * 1024 * 1024;
/// `IH_SYNC` encoded characters (larger = ignored).
pub const MAX_SYNC_SECTION: usize = 16384;
/// Inflated 5.x payloads above this are parsed in the export-worker instance (ARCHITECTURE §9.1).
pub const WORKER_PARSE_THRESHOLD: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum V5Error {
    PdfTooLarge,
    PdfType,
    PageCount,
    PayloadTooLarge,
    PayloadExpands,
    StrokesInvalid,
    StructureTooLarge,
    FieldsTooLarge,
    /// `atob` rejected the text.
    Base64,
    /// `decodeURIComponent(escape(…))` rejected the bytes (not UTF-8).
    Utf8,
    /// `JSON.parse` rejected the text.
    Json,
    /// The zlib stream is damaged.
    Inflate,
    Checkpoint(&'static str),
}

impl V5Error {
    /// 5.18.0's `error.message`. For the three failures that 5.18.0 leaves to the engine (`atob`, `decodeURIComponent`,
    /// `JSON.parse`, `DecompressionStream`) the text is Chromium's, without the position-dependent tail of
    /// `JSON.parse` messages.
    pub fn message(&self) -> &'static str {
        match self {
            V5Error::PdfTooLarge => "This PDF is larger than the 64 MB safety limit.",
            V5Error::PdfType => "PDF source has an unexpected content type",
            V5Error::PageCount => "PDF page count exceeds the safe limit",
            V5Error::PayloadTooLarge => "PDF metadata payload is too large",
            V5Error::PayloadExpands => "PDF metadata expands beyond the safe limit",
            V5Error::StrokesInvalid => "Embedded stroke metadata is invalid",
            V5Error::StructureTooLarge => "Collaboration structure metadata is too large",
            V5Error::FieldsTooLarge => "Collaboration field metadata is too large",
            V5Error::Base64 => "Failed to execute 'atob' on 'Window': The string to be decoded is not correctly encoded.",
            V5Error::Utf8 => "URI malformed",
            V5Error::Json => "Unexpected token in JSON",
            V5Error::Inflate => "The compressed data was not valid.",
            V5Error::Checkpoint(m) => m,
        }
    }
}

impl std::fmt::Display for V5Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(self.message()) }
}

/// `importPDFData` + `assertSafePdfBlob`: the announced size and the blob type (`""` = none).
pub fn check_pdf_blob(size: u64, mime: &str) -> Result<(), V5Error> {
    if size > MAX_PDF_BYTES {
        return Err(V5Error::PdfTooLarge);
    }
    let base = mime.split(';').next().unwrap_or("").trim_end();
    if !mime.is_empty() && !base.eq_ignore_ascii_case("application/pdf") && !base.eq_ignore_ascii_case("application/octet-stream") {
        return Err(V5Error::PdfType);
    }
    Ok(())
}

/// `loadPdfDocument`: 1..=2000 pages.
pub fn check_page_count(pages: u32) -> Result<(), V5Error> {
    if (1..=MAX_PDF_PAGES).contains(&pages) {
        Ok(())
    } else {
        Err(V5Error::PageCount)
    }
}

// ------------------------------------------------------------------------------------- simpleHash, legacy ids

fn radix36(mut n: u64) -> String {
    let mut out = Vec::new();
    loop {
        out.push(b"0123456789abcdefghijklmnopqrstuvwxyz"[(n % 36) as usize]);
        n /= 36;
        if n == 0 {
            break;
        }
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// `(n).toString(36)` for a non-negative integer.
pub fn to_base36(n: u64) -> String { radix36(n) }

fn java_hash(units: impl Iterator<Item = u16>) -> String {
    let h = units.fold(0i32, |h, c| (h << 5).wrapping_sub(h).wrapping_add(c as i32));
    let mut s = radix36(h.unsigned_abs() as u64);
    s.truncate(8);
    s
}

/// 5.x `simpleHash(str)`: the Java string hash over UTF-16 code units, `Math.abs(h).toString(36).slice(0, 8)`.
pub fn simple_hash(s: &str) -> String { java_hash(s.encode_utf16()) }

/// `pg_legacy_<simpleHash(documentId:pageNumber)>` (`page` is 1-based). `None` without a document id (5.x then
/// uses a random id; the importer derives one from its own inputs instead).
pub fn legacy_page_id(document_id: &str, page: u32) -> Option<String> {
    let doc = document_id.trim();
    (!doc.is_empty()).then(|| format!("pg_legacy_{}", simple_hash(&format!("{doc}:{page}"))))
}

/// `st_legacy_…` / `im_legacy_…` with the import's `fastStableIndex` (empty fingerprint).
pub fn legacy_item_id(document_id: &str, page_id: &str, image: bool, index: usize) -> Option<String> {
    let (doc, page) = (document_id.trim(), page_id.trim());
    if doc.is_empty() || page.is_empty() {
        return None;
    }
    let identity = format!("{doc}:{page}:{}:{index}:", if image { "image" } else { "stroke" });
    // `[...identity].reverse().join('')` reverses code points, not code units.
    let reversed: String = identity.chars().rev().collect();
    Some(format!("{}_legacy_{}{}", if image { "im" } else { "st" }, simple_hash(&identity), simple_hash(&reversed)))
}

/// `local:<simpleHash(lowercase(trim(exportName || 'untitled')) + ':' + (firstPageId || 'untitled'))>`: the
/// `timeline-history` key of a document without a Drive id.
pub fn local_timeline_key(export_name: &str, first_page_id: &str) -> String {
    let name = export_name.trim().to_lowercase();
    format!("local:{}", simple_hash(&format!("{}:{}", if name.is_empty() { "untitled" } else { &name }, if first_page_id.is_empty() { "untitled" } else { first_page_id })))
}

/// Test command hook: `__IHN_TEST_API__.cmd('core.v5.<name>', args)` (JSON text in, JSON text out).
pub fn test_cmd(name: &str, args: &str) -> Option<String> {
    let mut out = String::new();
    match name {
        "simpleHash" => json::push_str(&mut out, &simple_hash(args)),
        "sections" => {
            let names: Vec<json::Value> = keywords::section_names(args).into_iter().map(|n| json::Value::Str(n.into())).collect();
            out = json::stringify(&json::Value::Arr(names));
        }
        "num" => out = jsfmt::num(json::parse(args.as_bytes())?.num()),
        _ => return None,
    }
    Some(out)
}

#[cfg(test)]
pub(crate) mod fixtures;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_hash_matches_5_18_0() {
        // Values computed with 5.18.0's `simpleHash` in Chromium.
        assert_eq!(simple_hash(""), "0");
        assert_eq!(simple_hash("a"), "2p");
        assert_eq!(simple_hash("hello world"), "to5x38");
        assert_eq!(simple_hash("scene-template:1"), "dddvde");
        assert_eq!(simple_hash("😀é"), "wpzhi");
        assert_eq!(legacy_page_id(" scene-template ", 1).as_deref(), Some("pg_legacy_dddvde"));
        assert_eq!(legacy_page_id("  ", 1), None);
        assert_eq!(legacy_item_id("doc", "", false, 0), None);
        let id = legacy_item_id("doc", "pg_1", false, 3).unwrap();
        assert_eq!(id, format!("st_legacy_{}{}", simple_hash("doc:pg_1:stroke:3:"), simple_hash(":3:ekorts:1_gp:cod")));
        assert!(legacy_item_id("doc", "pg_1", true, 0).unwrap().starts_with("im_legacy_"));
    }

    #[test]
    fn limits_and_messages() {
        assert_eq!(check_pdf_blob(MAX_PDF_BYTES, ""), Ok(()));
        assert_eq!(check_pdf_blob(MAX_PDF_BYTES + 1, "application/pdf").unwrap_err().message(), "This PDF is larger than the 64 MB safety limit.");
        assert_eq!(check_pdf_blob(10, "Application/PDF; x=1"), Ok(()));
        assert_eq!(check_pdf_blob(10, "application/octet-stream"), Ok(()));
        assert_eq!(check_pdf_blob(10, "text/html").unwrap_err().message(), "PDF source has an unexpected content type");
        assert_eq!(check_page_count(0).unwrap_err().message(), "PDF page count exceeds the safe limit");
        assert_eq!((check_page_count(1), check_page_count(2000), check_page_count(2001)), (Ok(()), Ok(()), Err(V5Error::PageCount)));
    }
}
