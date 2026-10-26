//! Ported logic for the Drive home area. Pure functions + a `call` dispatcher entry.
//! Submodules: `palette` (folder colour maths).
mod palette;

use crate::bridge;

pub fn call(method: &str, input: &[u8]) -> Option<Result<Vec<u8>, String>> {
    match method {
        "drive_home.resolve_color" => Some(bridge::json(input, palette::resolve_call)),
        "drive_home.palette" => Some(bridge::json(input, palette::palette_call)),
        "drive_home.view_tint" => Some(bridge::json(input, palette::view_tint_call)),
        _ => None,
    }
}
