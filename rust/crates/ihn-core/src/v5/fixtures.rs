//! Test-only access to W1.2's 5.18.0 fixtures (`tests/v6/fixtures/v5`): the `/Keywords` of a fixture PDF (pdf-lib
//! writes the Info dictionary into a compressed object stream, or as a plain object), the import records, and the
//! storage dumps as `json::Value` trees (`{"$":"u8"}` → `Bytes`, `{"$":"blob"}` → `Blob`, `{"$":"undef"}` → `Undef`).

use super::inflate::inflate_raw;
use super::json::{self, Value};
use crate::codec::b64_decode;
use std::path::PathBuf;

pub fn dir() -> PathBuf { PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../tests/v6/fixtures/v5") }

pub fn read(name: &str) -> Vec<u8> { std::fs::read(dir().join(name)).unwrap_or_else(|e| panic!("fixture {name}: {e}")) }

pub fn json(name: &str) -> Value { json::parse(&read(name)).unwrap_or_else(|| panic!("fixture {name}: not JSON")) }

/// The Drive fixtures that carry an import record.
pub fn drive_names() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir()).unwrap().filter_map(|e| e.ok()?.file_name().into_string().ok()?.strip_suffix(".expect.json").map(str::to_string)).collect();
    names.sort();
    names
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> { hay.get(from..)?.windows(needle.len()).position(|w| w == needle).map(|i| i + from) }

/// A PDF string object at the start of `s` (after whitespace): literal with escapes, or hex.
fn pdf_string(s: &[u8]) -> Vec<u8> {
    let s = &s[s.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(0)..];
    let mut out = Vec::new();
    if s[0] == b'<' {
        let hex: Vec<u8> = s[1..].iter().take_while(|&&b| b != b'>').copied().filter(u8::is_ascii_hexdigit).collect();
        let v = |b: u8| (b as char).to_digit(16).unwrap() as u8;
        for pair in hex.chunks(2) {
            out.push(v(pair[0]) << 4 | pair.get(1).map_or(0, |&b| v(b)));
        }
        return out;
    }
    let (mut i, mut depth) = (1, 1);
    while i < s.len() {
        let b = s[i];
        i += 1;
        match b {
            b'\\' => {
                let e = s[i];
                i += 1;
                match e {
                    b'n' => out.push(b'\n'),
                    b'r' => out.push(b'\r'),
                    b't' => out.push(b'\t'),
                    b'b' => out.push(8),
                    b'f' => out.push(12),
                    b'\r' | b'\n' => {}
                    b'0'..=b'7' => {
                        let mut n = (e - b'0') as u32;
                        for _ in 0..2 {
                            match s.get(i) {
                                Some(d @ b'0'..=b'7') => {
                                    n = n * 8 + (d - b'0') as u32;
                                    i += 1;
                                }
                                _ => break,
                            }
                        }
                        out.push(n as u8);
                    }
                    other => out.push(other),
                }
            }
            b'(' => {
                depth += 1;
                out.push(b);
            }
            b')' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
                out.push(b);
            }
            _ => out.push(b),
        }
    }
    out
}

/// What pdf.js `info.Keywords` returns: UTF-16BE with a BOM, else PDFDocEncoding (Latin-1 for these fixtures).
fn pdf_text(bytes: &[u8]) -> String {
    match bytes {
        [0xfe, 0xff, rest @ ..] => String::from_utf16_lossy(&rest.chunks(2).map(|c| u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)])).collect::<Vec<_>>()),
        _ => bytes.iter().map(|&b| b as char).collect(),
    }
}

/// `/Keywords` of a PDF, the last one in file order (an incremental update appends a newer Info). `""` without one.
pub fn keywords(pdf: &[u8]) -> String {
    let mut found: Option<Vec<u8>> = None;
    let mut pos = 0;
    while let Some(at) = find(pdf, b"/Keywords", pos) {
        found = Some(pdf_string(&pdf[at + 9..]));
        pos = at + 9;
    }
    // Object streams: inflate each `/Type /ObjStm` stream and look inside.
    pos = 0;
    while let Some(at) = find(pdf, b"stream", pos) {
        let mut start = at + 6;
        pos = start;
        if pdf.get(at.wrapping_sub(3)..at) == Some(b"end") {
            continue;
        }
        start += if pdf.get(start) == Some(&b'\r') { 2 } else { 1 };
        let Some(end) = find(pdf, b"endstream", start) else { break };
        // Only object streams can hold the Info dictionary (content and image streams are skipped).
        let dict = &pdf[at.saturating_sub(300)..at];
        if find(dict, b"/ObjStm", 0).is_none() {
            pos = end + 9;
            continue;
        }
        if let Ok((data, _)) = inflate_raw(pdf.get(start + 2..end).unwrap_or(&[]), 256 << 20) {
            if let Some(k) = find(&data, b"/Keywords", 0) {
                found = Some(pdf_string(&data[k + 9..]));
            }
        }
        pos = end + 9;
    }
    found.map(|b| pdf_text(&b)).unwrap_or_default()
}

/// A storage dump with its typed values decoded (`Blob::obj` = order of appearance).
pub struct Dump {
    pub root: Value,
}

fn revive(v: Value, blobs: &mut Vec<Vec<u8>>) -> Value {
    match v {
        Value::Obj(fields) => {
            let tag = fields.iter().find(|(k, _)| &**k == "$").and_then(|(_, t)| t.as_str().map(str::to_string));
            let bytes = || fields.iter().find(|(k, _)| &**k == "b").and_then(|(_, b)| b64_decode(b.as_str()?.as_bytes())).unwrap_or_default();
            match tag.as_deref() {
                Some("undef") => Value::Undef,
                Some("u8") => Value::Bytes(bytes()),
                Some("blob") => {
                    let b = bytes();
                    let size = b.len() as f64;
                    blobs.push(b);
                    Value::Blob { obj: blobs.len() as u32 - 1, size }
                }
                _ => Value::Obj(fields.into_iter().map(|(k, v)| (k, revive(v, blobs))).collect()),
            }
        }
        Value::Arr(list) => Value::Arr(list.into_iter().map(|v| revive(v, blobs)).collect()),
        other => other,
    }
}

impl Dump {
    pub fn load(scenario: &str) -> Dump {
        let mut blobs = Vec::new();
        let root = revive(json(&format!("storage/{scenario}.dump.json")), &mut blobs);
        Dump { root }
    }
    /// The record values of one store.
    pub fn store(&self, db: &str, store: &str) -> Vec<Value> {
        let dbs = self.root.get("indexedDB").arr();
        let Some(d) = dbs.iter().find(|d| d.get("name").as_str() == Some(db)) else { return Vec::new() };
        let Some(s) = d.get("stores").arr().iter().find(|s| s.get("name").as_str() == Some(store)) else { return Vec::new() };
        s.get("records").arr().iter().map(|r| r.get("value").clone()).collect()
    }
    pub fn local(&self, key: &str) -> Option<&str> { self.root.get("localStorage").get(key).as_str() }
}
