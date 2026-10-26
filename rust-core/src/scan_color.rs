//! Ported logic for this area lives here. Pure functions + a `call` dispatcher entry.
mod quality;

pub fn call(_method: &str, _input: &[u8]) -> Option<Result<Vec<u8>, String>> { None }
