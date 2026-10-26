//! Collaboration core: canonical document hash, stable stringify and the structure / field-meta
//! normalizers, ported from collaboration-core-v5.js. Strings are UTF-16 internally so every
//! result is byte-identical with the JavaScript it replaced.
mod json;
mod canon;
mod snapshot;

use json::{parse, write_stable, Pool};

fn text_out(units: &[u16]) -> Result<Vec<u8>, String> { Ok(json::units_to_string(units)?.into_bytes()) }

/// Parse `{"meta":..,"ids":..}` style arguments.
fn with_root<T>(input: &[u8], f: impl FnOnce(&Pool, &json::J) -> Result<T, String>) -> Result<T, String> {
    let mut pool = Pool::default();
    let root = parse(&mut pool, input)?;
    f(&pool, &root)
}

pub fn call(method: &str, input: &[u8]) -> Option<Result<Vec<u8>, String>> {
    Some(match method {
        "collab.hash_utf8" => std::str::from_utf8(input).map_err(|_| "Input is not UTF-8".to_string())
            .map(|text| canon::hash_units(text.encode_utf16()).into_bytes()),
        "collab.hash_utf16" => if input.len() % 2 != 0 { Err("Odd UTF-16 byte length".into()) } else {
            Ok(canon::hash_units(input.chunks_exact(2).map(|p| u16::from_le_bytes([p[0], p[1]]))).into_bytes()) },
        "collab.stable" => with_root(input, |pool, v| { let mut out: Vec<u16> = Vec::new(); write_stable(&mut out, pool, v); text_out(&out) }),
        "collab.document_hash" => canon::document_hash(input).map(String::into_bytes),
        "collab.structure_hash" => with_root(input, |pool, root| {
            Ok(canon::structure_hash(pool, root.get(pool, "meta"), root.get(pool, "ids")).into_bytes()) }),
        "collab.normalize_structure" => with_root(input, |pool, root| {
            let ids = canon::normalize_page_ids(pool, root.get(pool, "ids"));
            let meta = canon::normalize_structure(pool, root.get(pool, "meta"), &ids);
            let mut out: Vec<u16> = Vec::new(); canon::write_structure_plain(&mut out, &meta); text_out(&out) }),
        "collab.field_hash" => with_root(input, |pool, root| Ok(canon::field_meta_hash(pool, Some(root)).into_bytes())),
        "collab.normalize_fields" => with_root(input, |pool, root| {
            let f = canon::normalize_fields(pool, Some(root));
            let mut out: Vec<u16> = Vec::new(); canon::write_fields_plain(&mut out, pool, &f); text_out(&out) }),
        "collab.snap_remember" => snapshot::remember(input),
        "collab.snap_delta" => snapshot::delta(input),
        "collab.snap_reset" => snapshot::reset(),
        _ => return None,
    })
}

#[cfg(test)]
mod tests;
