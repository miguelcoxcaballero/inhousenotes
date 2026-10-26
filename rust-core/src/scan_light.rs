//! Ported logic for this area lives here. Pure functions + a `call` dispatcher entry.
//! Unit U1 (stencil removal) lives in `scan_light/stencil.rs`.
mod stencil;

pub fn call(_method: &str, _input: &[u8]) -> Option<Result<Vec<u8>, String>> { None }
