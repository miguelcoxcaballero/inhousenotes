//! Undo/redo, three-way merge and version timeline logic. Pure computation only;
//! state, DOM and persistence stay with the host. Submodules hold each ported unit.
mod fingerprints;

pub fn call(_method: &str, _input: &[u8]) -> Option<Result<Vec<u8>, String>> { None }
