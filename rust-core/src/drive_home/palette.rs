//! Folder colour palette maths. Every expression keeps the operand order of the
//! original JavaScript so the IEEE-754 results (and therefore the rounded hex
//! strings) are bit-identical. Only the final CSS strings leave this module.

use serde::{Deserialize, Serialize};

/// JS `String.prototype.trim` whitespace set (WhiteSpace + LineTerminator).
fn is_js_space(c: char) -> bool {
    matches!(c, '\u{9}'..='\u{d}' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}'
        | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
}

/// `resolveDriveFolderColor`: lower-case `#rrggbb`, or the fallback for anything else
/// (Drive reports #8f8f8f for folders nobody coloured).
pub fn resolve_color(raw: &str, fallback: &str) -> String {
    let trimmed = raw.trim_matches(is_js_space);
    let bytes = trimmed.as_bytes();
    if bytes.len() == 7 && bytes[0] == b'#'
        && bytes[1..].iter().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f' | b'A'..=b'F')) {
        let lower = trimmed.to_ascii_lowercase();
        if lower != "#8f8f8f" { return lower; }
    }
    fallback.to_string()
}

/// `parseInt(hex.slice(1), 16)` for a strict `#rrggbb` (either case).
fn parse_hex(hex: &str) -> Result<u32, String> {
    let bytes = hex.as_bytes();
    if bytes.len() != 7 || bytes[0] != b'#' || !bytes[1..].iter().all(u8::is_ascii_hexdigit) {
        return Err(format!("Invalid folder colour {hex:?}"));
    }
    u32::from_str_radix(&hex[1..], 16).map_err(|_| format!("Invalid folder colour {hex:?}"))
}

/// `Math.round`: halves go towards +infinity.
fn js_round(x: f64) -> f64 {
    let floor = x.floor();
    if x - floor >= 0.5 { floor + 1.0 } else { floor }
}

fn relative_luminance(n: u32) -> f64 {
    fn linear(value: u32) -> f64 {
        let c = value as f64 / 255.0;
        if c <= 0.03928 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    }
    0.2126 * linear(n >> 16) + 0.7152 * linear((n >> 8) & 255) + 0.0722 * linear(n & 255)
}

type Hsl = (f64, f64, f64);

fn hex_to_hsl(hex: &str) -> Result<Hsl, String> {
    let n = parse_hex(hex)?;
    let r = (n >> 16) as f64 / 255.0;
    let g = ((n >> 8) & 255) as f64 / 255.0;
    let b = (n & 255) as f64 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d == 0.0 { return Ok((0.0, 0.0, l * 100.0)); }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r { ((g - b) / d) % 6.0 }
        else if max == g { (b - r) / d + 2.0 }
        else { (r - g) / d + 4.0 };
    Ok(((h * 60.0 + 360.0) % 360.0, s * 100.0, l * 100.0))
}

fn hsl_to_hex(h: f64, s: f64, l: f64) -> String {
    let sat = s.clamp(0.0, 100.0) / 100.0;
    let light = l.clamp(0.0, 100.0) / 100.0;
    let k = |n: f64| (n + h / 30.0) % 12.0;
    let a = sat * light.min(1.0 - light);
    let channel = |n: f64| {
        let kn = k(n);
        let v = js_round(255.0 * (light - a * (-1.0f64).max((kn - 3.0).min(9.0 - kn).min(1.0))));
        v.clamp(0.0, 255.0) as u32
    };
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(7);
    out.push('#');
    for value in [channel(0.0), channel(8.0), channel(4.0)] {
        out.push(DIGITS[(value >> 4) as usize] as char);
        out.push(DIGITS[(value & 15) as usize] as char);
    }
    out
}

/// Yellows turn green when their hue leans that way: pull them towards amber by `pull`.
fn warm_hue(h: f64, pull: f64) -> f64 {
    if h <= 42.0 || h >= 72.0 { h } else { h - (h - 42.0) * pull }
}

fn is_warm(h: f64, s: f64) -> bool { s > 8.0 && h >= 28.0 && h < 72.0 }

/// The colour a folder is drawn in: Drive colour by day (yellows warmer), quieter at night.
fn display_color(base: &str, night: bool) -> Result<String, String> {
    let (h0, s0, l0) = hex_to_hsl(base)?;
    let warm = is_warm(h0, s0);
    if !night {
        return Ok(if warm { hsl_to_hex(warm_hue(h0, 0.45), s0, l0) } else { base.to_string() });
    }
    if warm {
        return Ok(hsl_to_hex(warm_hue(h0, 0.85), s0.min(70.0) * 0.78, 53.0f64.max(58.0f64.min(l0 * 0.8))));
    }
    Ok(hsl_to_hex(h0, s0 * 0.68, 28.0f64.max(l0.min(62.0) * 0.8)))
}

fn shade_color(color: &str, amount: f64) -> Result<String, String> {
    let (h, s, l) = hex_to_hsl(color)?;
    let warm = is_warm(h, s);
    Ok(hsl_to_hex(if warm { h - amount * 0.25 } else { h }, s, l - amount))
}

/// The CSS custom-property values of `applyDriveFolderPalette` that need computing.
/// `dark` selects the ink pair; its fixed strings are listed in the JS adapter.
#[derive(Debug, PartialEq)]
pub struct Palette { pub color: String, pub cover: String, pub inside: String, pub tab: String, pub dark: bool }

pub fn palette(base: &str, night: bool, exact: bool) -> Result<Palette, String> {
    let color = if exact { parse_hex(base)?; base.to_string() } else { display_color(base, night)? };
    let (h, s, l) = hex_to_hsl(&color)?;
    let luminance = relative_luminance(parse_hex(&color)?);
    // Whichever ink has the higher contrast on this folder colour.
    let dark = (luminance + 0.05) / 0.0614 >= 1.05 / (luminance + 0.05);
    Ok(Palette {
        cover: hsl_to_hex(h, s, l + 2.0),
        inside: hsl_to_hex(h, s * 0.85, 94.0f64.min(l + if night { 9.0 } else { 14.0 })),
        tab: shade_color(&color, if night { 6.0 } else { 12.0 })?,
        color, dark,
    })
}

#[derive(Serialize, Debug, PartialEq)]
pub struct ViewTint { pub bg: String, pub inside: String, pub tab: String }

/// `getDriveFolderViewTint`: the softer colour an opened folder sits on.
pub fn view_tint(color: &str, night: bool) -> Result<ViewTint, String> {
    let (h0, s0, _) = hex_to_hsl(color)?;
    if night {
        // Dark yellows read as olive, so the night tint leans amber.
        let h = warm_hue(h0, 0.9);
        let s = (s0 * 0.3).min(18.0);
        return Ok(ViewTint { bg: hsl_to_hex(h, s, 14.0), inside: hsl_to_hex(h, s, 18.0), tab: display_color(color, true)? });
    }
    let h = warm_hue(h0, 0.6);
    let s = s0.min(85.0) * 0.62;
    Ok(ViewTint { bg: hsl_to_hex(h, s, 89.0), inside: hsl_to_hex(h, s, 94.0), tab: hsl_to_hex(h, s * 1.2, 78.0) })
}

const MAX_ITEMS: usize = 100_000;

#[derive(Deserialize)]
pub struct ResolveInput { fallback: String, items: Vec<String> }
pub fn resolve_call(input: ResolveInput) -> Result<Vec<String>, String> {
    if input.items.len() > MAX_ITEMS { return Err("Too many folder colours".into()); }
    Ok(input.items.iter().map(|raw| resolve_color(raw, &input.fallback)).collect())
}

#[derive(Deserialize)]
pub struct PaletteInput { night: bool, #[serde(default)] exact: bool, items: Vec<String> }
/// Flat wire format (cheap to parse): `colors` holds color, cover, inside, tab for each item in order,
/// `dark` holds 1 when the item takes the dark ink.
#[derive(Serialize)]
pub struct PaletteBatch { colors: Vec<String>, dark: Vec<u8> }
pub fn palette_call(input: PaletteInput) -> Result<PaletteBatch, String> {
    if input.items.len() > MAX_ITEMS { return Err("Too many folder colours".into()); }
    let mut batch = PaletteBatch { colors: Vec::with_capacity(input.items.len() * 4), dark: Vec::with_capacity(input.items.len()) };
    for base in &input.items {
        let p = palette(base, input.night, input.exact)?;
        batch.colors.extend([p.color, p.cover, p.inside, p.tab]);
        batch.dark.push(p.dark as u8);
    }
    Ok(batch)
}

#[derive(Deserialize)]
pub struct TintInput { night: bool, color: String }
pub fn view_tint_call(input: TintInput) -> Result<ViewTint, String> { view_tint(&input.color, input.night) }

#[cfg(test)]
mod tests {
    use super::*;

    #[test] fn resolves_colours_like_drive() {
        let fb = "#e3c58e";
        assert_eq!(resolve_color("  #AABBCC\n", fb), "#aabbcc");
        assert_eq!(resolve_color("#8F8F8F", fb), fb);
        assert_eq!(resolve_color("", fb), fb);
        assert_eq!(resolve_color("#abc", fb), fb);
        assert_eq!(resolve_color("#abcdeg", fb), fb);
        assert_eq!(resolve_color("\u{feff}#123456\u{2028}", fb), "#123456");
        assert_eq!(resolve_color("\u{85}#123456", fb), fb); // NEL is not JS whitespace
    }
    #[test] fn hex_round_trip_is_stable_for_greys_and_primaries() {
        for hex in ["#000000", "#ffffff", "#808080", "#ff0000", "#00ff00", "#0000ff", "#e3c58e"] {
            let (h, s, l) = hex_to_hsl(hex).unwrap();
            assert_eq!(hsl_to_hex(h, s, l), hex);
        }
    }
    #[test] fn js_round_matches_math_round() {
        assert_eq!(js_round(2.5), 3.0);
        assert_eq!(js_round(-2.5), -2.0);
        assert_eq!(js_round(0.49999999999999994), 0.0);
        assert_eq!(js_round(254.5), 255.0);
    }
    #[test] fn default_manila_palette_by_day() {
        let p = palette("#e3c58e", false, false).unwrap();
        assert_eq!(p.color, "#e3c58e");
        assert!(p.dark);
    }
    #[test] fn dark_colour_gets_white_ink() {
        let p = palette("#10203a", false, false).unwrap();
        assert!(!p.dark);
    }
    #[test] fn exact_keeps_colour_and_night_darkens_others() {
        assert_eq!(palette("#336699", true, true).unwrap().color, "#336699");
        assert_ne!(palette("#336699", true, false).unwrap().color, "#336699");
    }
    #[test] fn bad_input_is_an_error_not_a_panic() {
        assert!(palette("red", false, false).is_err());
        assert!(palette("#12345", false, false).is_err());
        assert!(palette("#ééééé", false, false).is_err());
        assert!(view_tint("", true).is_err());
    }
    #[test] fn view_tint_day_and_night() {
        let day = view_tint("#e3c58e", false).unwrap();
        let night = view_tint("#e3c58e", true).unwrap();
        assert!(day.bg > night.bg);
        assert_eq!(night.tab, display_color("#e3c58e", true).unwrap());
    }
    #[test] fn batch_call_round_trips_json() {
        let out = crate::bridge::dispatch("drive_home.palette", br##"{"night":false,"items":["#e3c58e","#336699"]}"##).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["colors"].as_array().unwrap().len(), 8);
        assert_eq!(v["colors"][0], "#e3c58e");
        assert_eq!(v["dark"].as_array().unwrap().len(), 2);
    }
}
