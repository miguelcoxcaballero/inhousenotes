//! Generic host bridge for ported logic. One exported entry point carries a
//! method name plus an opaque byte/JSON payload; every ported area registers
//! its methods in its own module (`call`), so areas never edit shared files.
//!
//! Wire format of the returned block: byte 0 is the status (0 = ok, 1 = error),
//! the rest is the payload (or a UTF-8 error message). The block is allocated
//! with the host `alloc` and must be released with `free(ptr, len)`.
//! Hot numeric kernels should still use direct typed-pointer exports.

use serde::{de::DeserializeOwned, Serialize};

pub type Handler = fn(&str, &[u8]) -> Option<Result<Vec<u8>, String>>;

/// Each area owns exactly one entry; the list itself is fixed.
const HANDLERS: &[Handler] = &[
    call,
    crate::scan_light::call, crate::scan_color::call, crate::scan_geometry::call, crate::scan_live::call,
    crate::editor_input::call, crate::editor_render::call, crate::editor_view::call, crate::editor_history::call,
    crate::collab::call, crate::offline::call, crate::docio::call, crate::storage::call,
    crate::drive_home::call, crate::calendar::call, crate::ts_engine::call,
];

pub fn dispatch(name: &str, input: &[u8]) -> Result<Vec<u8>, String> {
    for handler in HANDLERS {
        if let Some(result) = handler(name, input) { return result; }
    }
    Err(format!("Unknown Rust method {name}"))
}

/// Decode JSON input, run `f`, encode its JSON output.
#[allow(dead_code)]
pub fn json<I: DeserializeOwned, O: Serialize>(input: &[u8], f: impl FnOnce(I) -> Result<O, String>) -> Result<Vec<u8>, String> {
    let value: I = serde_json::from_slice(input).map_err(|error| format!("Invalid input: {error}"))?;
    serde_json::to_vec(&f(value)?).map_err(|error| format!("Cannot encode output: {error}"))
}

/// Built-in methods used to verify the bridge itself.
fn call(method: &str, input: &[u8]) -> Option<Result<Vec<u8>, String>> {
    match method {
        "bridge.echo" => Some(Ok(input.to_vec())),
        "bridge.version" => Some(Ok(b"1".to_vec())),
        _ => None,
    }
}

/// Pointer/length pairs come from the host adapter, which owns the buffers.
/// Returns null when the arguments are invalid or allocation fails.
#[no_mangle]
pub unsafe extern "C" fn bridge_call(name: *const u8, name_len: usize, input: *const u8, input_len: usize, out_len: *mut usize) -> *mut u8 {
    if name.is_null() || out_len.is_null() || name_len == 0 || name_len > 128
        || (input.is_null() && input_len > 0) || input_len > 96 * 1024 * 1024 { return std::ptr::null_mut(); }
    let Ok(name) = std::str::from_utf8(std::slice::from_raw_parts(name, name_len)) else { return std::ptr::null_mut(); };
    let input: &[u8] = if input_len == 0 { &[] } else { std::slice::from_raw_parts(input, input_len) };
    let (status, body) = match dispatch(name, input) { Ok(body) => (0u8, body), Err(message) => (1u8, message.into_bytes()) };
    let total = body.len() + 1;
    let block = crate::alloc(total);
    if block.is_null() { return block; }
    *block = status;
    std::ptr::copy_nonoverlapping(body.as_ptr(), block.add(1), body.len());
    *out_len = total;
    block
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn echo_and_unknown() {
        assert_eq!(dispatch("bridge.echo", b"abc").unwrap(), b"abc");
        assert!(dispatch("nope.nothing", b"").is_err());
    }
    #[test] fn json_round_trip() {
        let out = json(b"[1,2,3]", |v: Vec<u32>| Ok::<_, String>(v.iter().sum::<u32>())).unwrap();
        assert_eq!(out, b"6");
        assert!(json(b"nope", |v: Vec<u32>| Ok::<_, String>(v.len())).is_err());
    }
}
