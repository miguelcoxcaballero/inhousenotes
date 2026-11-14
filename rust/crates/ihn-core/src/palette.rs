//! Folder colour maths (HSL, luminance, warm hue, tint) and the sheet label tone. Every expression keeps the operand
//! order of 5.18.0's JavaScript, so the rounded hex strings are identical. Implemented by W2.5 (BUILD-PLAN.md).

use crate::jsmath::{pow, round};

// v5: app-v5.js:DRIVE_DEFAULT_FOLDER_COLOR
pub const DEFAULT_FOLDER_COLOR: &str = "#e3c58e";
// v5: app-v5.js:DRIVE_FOLDER_COLORS
pub const FOLDER_COLORS: [&str; 24] = [
    "#f83a22", "#fa573c", "#d06b64", "#ac725e", "#ff7537", "#ffad46", "#fad165", "#fbe983", "#b3dc6c", "#7bd148", "#16a765", "#42d692",
    "#92e1c0", "#9fe1e7", "#9fc5e8", "#4986e7", "#b99aff", "#a479e2", "#cd74e6", "#f691b2", "#cca6ac", "#cabdbf", "#c2c2c2", "#8f8f8f",
];

/// JS `String.prototype.trim` whitespace.
fn is_js_space(c: char) -> bool {
    matches!(c, '\u{9}'..='\u{d}' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
}

/// Lower-case `#rrggbb`, or manila for anything else (Drive reports `#8f8f8f` for folders nobody coloured).
// v5: app-v5.js:resolveDriveFolderColor
pub fn resolve_color(raw: &str) -> String {
    // Only ASCII can match `#rrggbb`, so ASCII lower-casing is `toLowerCase()` here.
    let lower = raw.trim_matches(is_js_space).to_ascii_lowercase();
    let b = lower.as_bytes();
    if b.len() == 7 && b[0] == b'#' && b[1..].iter().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f')) && lower != "#8f8f8f" {
        return lower;
    }
    DEFAULT_FOLDER_COLOR.to_string()
}

fn parse_hex(hex: &str) -> u32 { u32::from_str_radix(hex.get(1..).unwrap_or(""), 16).unwrap_or(0) }

// v5: app-v5.js:driveRelativeLuminance
fn relative_luminance(n: u32) -> f64 {
    let linear = |value: u32| {
        let c = value as f64 / 255.0;
        if c <= 0.03928 { c / 12.92 } else { pow((c + 0.055) / 1.055, 2.4) }
    };
    0.2126 * linear(n >> 16) + 0.7152 * linear((n >> 8) & 255) + 0.0722 * linear(n & 255)
}

// v5: app-v5.js:driveHexToHsl
pub fn hex_to_hsl(hex: &str) -> (f64, f64, f64) {
    let n = parse_hex(hex);
    let (r, g, b) = ((n >> 16) as f64 / 255.0, ((n >> 8) & 255) as f64 / 255.0, (n & 255) as f64 / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    let d = max - min;
    if d == 0.0 {
        return (0.0, 0.0, l * 100.0);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r { ((g - b) / d) % 6.0 } else if max == g { (b - r) / d + 2.0 } else { (r - g) / d + 4.0 };
    ((h * 60.0 + 360.0) % 360.0, s * 100.0, l * 100.0)
}

// v5: app-v5.js:driveHslToHex
pub fn hsl_to_hex(h: f64, s: f64, l: f64) -> String {
    let (sat, light) = (s.clamp(0.0, 100.0) / 100.0, l.clamp(0.0, 100.0) / 100.0);
    let a = sat * light.min(1.0 - light);
    let channel = |n: f64| {
        let k = (n + h / 30.0) % 12.0;
        // `1 << 24 | c << 16 | …` in JS: each channel is truncated to an int32 by the shift.
        round(255.0 * (light - a * (-1.0f64).max((k - 3.0).min(9.0 - k).min(1.0)))) as i64 as u32
    };
    let n = (1u32 << 24) | (channel(0.0) << 16) | (channel(8.0) << 8) | channel(4.0);
    format!("#{}", &format!("{n:x}")[1..])
}

// v5: app-v5.js:warmDriveFolderHue
fn warm_hue(h: f64, pull: f64) -> f64 { if h <= 42.0 || h >= 72.0 { h } else { h - (h - 42.0) * pull } }
fn is_warm(h: f64, s: f64) -> bool { s > 8.0 && h >= 28.0 && h < 72.0 }

// v5: app-v5.js:displayDriveFolderColor
pub fn display_color(base: &str, night: bool) -> String {
    let (h0, s0, l0) = hex_to_hsl(base);
    let warm = is_warm(h0, s0);
    if !night {
        return if warm { hsl_to_hex(warm_hue(h0, 0.45), s0, l0) } else { base.to_string() };
    }
    if warm {
        return hsl_to_hex(warm_hue(h0, 0.85), s0.min(70.0) * 0.78, 53.0f64.max(58.0f64.min(l0 * 0.8)));
    }
    hsl_to_hex(h0, s0 * 0.68, 28.0f64.max(l0.min(62.0) * 0.8))
}

// v5: app-v5.js:shadeDriveFolderColor
fn shade_color(color: &str, amount: f64) -> String {
    let (h, s, l) = hex_to_hsl(color);
    hsl_to_hex(if is_warm(h, s) { h - amount * 0.25 } else { h }, s, l - amount)
}

/// The custom properties `applyDriveFolderPalette` sets.
#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    pub color: String,
    pub cover: String,
    pub inside: String,
    pub tab: String,
    pub dark_ink: bool,
}

impl Palette {
    pub fn ink(&self) -> &'static str { if self.dark_ink { "#1f1a14" } else { "#ffffff" } }
    pub fn hl(&self) -> &'static str { if self.dark_ink { "rgba(0, 0, 0, 0.07)" } else { "rgba(255, 255, 255, 0.2)" } }
    pub fn hl_strong(&self) -> &'static str { if self.dark_ink { "rgba(0, 0, 0, 0.12)" } else { "rgba(255, 255, 255, 0.3)" } }
    pub fn line(&self) -> &'static str { if self.dark_ink { "rgba(0, 0, 0, 0.25)" } else { "rgba(255, 255, 255, 0.55)" } }
    /// `(property, value)` in the order 5.18.0 sets them (the order of the inline `style` text).
    pub fn vars(&self) -> [(&'static str, &str); 8] {
        [
            ("--folder-color", &self.color), ("--folder-cover", &self.cover), ("--folder-inside", &self.inside), ("--folder-tab", &self.tab),
            ("--folder-ink", self.ink()), ("--folder-hl", self.hl()), ("--folder-hl-strong", self.hl_strong()), ("--folder-line", self.line()),
        ]
    }
}

/// `base` is a Drive colour, or with `exact` the colour to show as it is.
// v5: app-v5.js:applyDriveFolderPalette
pub fn palette(base: &str, night: bool, exact: bool) -> Palette {
    let color = if exact { base.to_string() } else { display_color(base, night) };
    let (h, s, l) = hex_to_hsl(&color);
    let luminance = relative_luminance(parse_hex(&color));
    Palette {
        cover: hsl_to_hex(h, s, l + 2.0),
        inside: hsl_to_hex(h, s * 0.85, 94.0f64.min(l + if night { 9.0 } else { 14.0 })),
        tab: shade_color(&color, if night { 6.0 } else { 12.0 }),
        dark_ink: (luminance + 0.05) / 0.0614 >= 1.05 / (luminance + 0.05),
        color,
    }
}

/// The softer colour an opened folder sits on: `(bg, inside, tab)`.
// v5: app-v5.js:getDriveFolderViewTint
pub fn view_tint(color: &str, night: bool) -> (String, String, String) {
    let (h0, s0, _) = hex_to_hsl(color);
    if night {
        let (h, s) = (warm_hue(h0, 0.9), (s0 * 0.3).min(18.0));
        return (hsl_to_hex(h, s, 14.0), hsl_to_hex(h, s, 18.0), display_color(color, true));
    }
    let (h, s) = (warm_hue(h0, 0.6), s0.min(85.0) * 0.62);
    (hsl_to_hex(h, s, 89.0), hsl_to_hex(h, s, 94.0), hsl_to_hex(h, s * 1.2, 78.0))
}

/// The palette of the opened folder view: the tint as an exact colour, with its own inside and tab.
// v5: app-v5.js:applyDriveFolderViewPalette
pub fn view_palette(color: &str, night: bool) -> Palette {
    let (bg, inside, tab) = view_tint(color, night);
    Palette { inside, tab, ..palette(&bg, night, true) }
}

/// Width / height of a sheet from its thumbnail, or `None` when it is close enough to A4.
// v5: app-v5.js:getDriveSheetRatio
pub fn sheet_ratio(width: f64, height: f64) -> Option<f64> {
    if !(width > 0.0) || !(height > 0.0) {
        return None;
    }
    let ratio = (width / height).max(0.35).min(2.2);
    ((ratio - 0.7071).abs() >= 0.012).then_some(ratio)
}

/// The label colours of a sheet, from the pixels 5.18.0 samples.
#[derive(Clone, Debug, PartialEq)]
pub struct Tone {
    pub rgb: [u32; 3],
    pub dark_ink: bool,
}

impl Tone {
    pub fn rgb_text(&self) -> String { format!("{}, {}, {}", self.rgb[0], self.rgb[1], self.rgb[2]) }
    pub fn ink(&self) -> &'static str { if self.dark_ink { "#000" } else { "#fff" } }
    pub fn ink_rgb(&self) -> &'static str { if self.dark_ink { "0, 0, 0" } else { "255, 255, 255" } }
    pub fn shadow_rgb(&self) -> &'static str { if self.dark_ink { "255, 255, 255" } else { "0, 0, 0" } }
    pub fn name(&self) -> &'static str { if self.dark_ink { "light" } else { "dark" } }
}

/// The source rectangle `(x, y, w, h)` 5.18.0 draws into its 24×16 sampling canvas.
// v5: app-v5.js:sampleDrivePreviewTone
pub fn tone_source(natural_w: f64, natural_h: f64) -> (f64, f64, f64, f64) {
    let ratio = sheet_ratio(natural_w, natural_h).map_or(0.7071, |r| crate::rows::parse_decimal(&crate::jsfmt::fixed(r, 4)).unwrap_or(r));
    let (sw, sh) = (natural_w.min(natural_h * ratio), natural_h.min(natural_w / ratio));
    let left = (natural_w - sw) / 2.0;
    (left + sw * 0.04, sh * 0.66, sw * 0.92, sh * 0.3)
}

/// The dominant colour of RGBA pixels (3 bits per channel bins, the first bin to reach the highest count wins).
// v5: app-v5.js:sampleDrivePreviewTone
pub fn tone(rgba: &[u8]) -> Option<Tone> {
    let mut bins = [[0u32; 4]; 512];
    let mut dominant: Option<usize> = None;
    for px in rgba.chunks_exact(4) {
        let key = ((px[0] as usize >> 5) << 6) | ((px[1] as usize >> 5) << 3) | (px[2] as usize >> 5);
        let bin = &mut bins[key];
        bin[0] += 1;
        bin[1] += px[0] as u32;
        bin[2] += px[1] as u32;
        bin[3] += px[2] as u32;
        if dominant.is_none_or(|d| bins[key][0] > bins[d][0]) {
            dominant = Some(key);
        }
    }
    let bin = bins[dominant?];
    let rgb = [1, 2, 3].map(|i| round(bin[i] as f64 / bin[0] as f64) as u32);
    let linear = rgb.map(|v| {
        let srgb = v as f64 / 255.0;
        if srgb <= 0.04045 { srgb / 12.92 } else { pow((srgb + 0.055) / 1.055, 2.4) }
    });
    let luminance = linear[0] * 0.2126 + linear[1] * 0.7152 + linear[2] * 0.0722;
    Some(Tone { rgb, dark_ink: (luminance + 0.05) / 0.05 >= 1.05 / (luminance + 0.05) })
}

/// Test command hook: `core.palette.vars` with `"#rrggbb|night|exact"` → the eight values joined by `|`;
/// `core.palette.tint` with `"#rrggbb|night"`.
pub fn test_cmd(name: &str, args: &str) -> Option<String> {
    let f: Vec<&str> = args.trim_matches('"').split('|').collect();
    let flag = |i: usize| f.get(i).is_some_and(|v| *v == "1");
    match name {
        "vars" => {
            let p = palette(&resolve_color(f.first()?), flag(1), flag(2));
            Some(format!("\"{}\"", p.vars().iter().map(|(_, v)| *v).collect::<Vec<_>>().join("|")))
        }
        "tint" => {
            let (a, b, c) = view_tint(&resolve_color(f.first()?), flag(1));
            Some(format!("\"{a}|{b}|{c}\""))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_colours_like_drive() {
        assert_eq!(resolve_color(" #4986E7 "), "#4986e7");
        assert_eq!(resolve_color("#8f8f8f"), DEFAULT_FOLDER_COLOR);
        assert_eq!(resolve_color("#8F8F8F"), DEFAULT_FOLDER_COLOR);
        assert_eq!(resolve_color(""), DEFAULT_FOLDER_COLOR);
        assert_eq!(resolve_color("#12345"), DEFAULT_FOLDER_COLOR);
        assert_eq!(resolve_color("#gggggg"), DEFAULT_FOLDER_COLOR);
    }

    #[test]
    fn hex_round_trip_is_stable_for_greys_and_the_picker_colours() {
        for hex in ["#000000", "#ffffff", "#808080", "#ff0000", "#00ff00", "#0000ff"].iter().chain(FOLDER_COLORS.iter()) {
            let (h, s, l) = hex_to_hsl(hex);
            assert_eq!(&hsl_to_hex(h, s, l), hex);
        }
    }

    // (colour, day, night, exact: color|cover|inside|tab|ink, tint day, tint night) from 5.18.0's own functions (tools: node over app-v5.js).
    const GOLDEN: &[(&str, &str, &str, &str, &str, &str)] = &[
        ("#e3c58e", "#e3c58e|#e5c996|#eee1ca|#d7a65d|#1f1a14", "#c6a261|#c9a668|#ccb489|#be924b|#1f1a14", "#e3c58e|#e5c996|#eee1ca|#d7a65d|#1f1a14", "#ede6d8|#f5f1ea|#e0ceae", "#2a261d|#363026|#c6a261"),
        ("#4986e7", "#4986e7|#528ce8|#90b2e8|#1c64d7|#1f1a14", "#3a6bb9|#3c6fc1|#5f85c2|#335ea2|#ffffff", "#4986e7|#528ce8|#90b2e8|#1c64d7|#1f1a14", "#d6e0f0|#e8eef7|#a7c0e7", "#1d222a|#262c36|#3a6bb9"),
        ("#16a765", "#16a765|#17b06a|#31d489|#0f7144|#1f1a14", "#24734f|#267b54|#378e66|#1d5c3f|#ffffff", "#16a765|#17b06a|#31d489|#0f7144|#1f1a14", "#d6f0e4|#e8f7f0|#a7e7ca", "#1d2a24|#26362f|#24734f"),
        ("#0b3d91", "#0b3d91|#0c419a|#1f5dc5|#072558|#ffffff", "#1e3d71|#204179|#30538d|#183059|#ffffff", "#0b3d91|#0c419a|#1f5dc5|#072558|#ffffff", "#d4dff2|#e8eef8|#a3beea", "#1d222a|#262c36|#1e3d71"),
        ("#fbe983", "#fbe183|#fbe38d|#f9efcc|#f9ca48|#1f1a14", "#ceae59|#d0b261|#d2bc83|#c79f41|#1f1a14", "#fbe983|#fbeb8d|#f9f2cc|#f9d648|#1f1a14", "#f2ebd4|#f8f4e8|#ead9a3", "#2a261d|#363126|#ceae59"),
        ("#f83a22", "#f83a22|#f8432c|#ef8072|#d61e07|#1f1a14", "#b93929|#c13c2b|#c85647|#a03123|#ffffff", "#f83a22|#f8432c|#ef8072|#d61e07|#1f1a14", "#f2d7d4|#f8e9e8|#eaaba3", "#2a1f1d|#362726|#b93929"),
        ("#c2c2c2", "#c2c2c2|#c7c7c7|#e6e6e6|#a3a3a3|#1f1a14", "#7e7e7e|#838383|#959595|#6f6f6f|#1f1a14", "#c2c2c2|#c7c7c7|#e6e6e6|#a3a3a3|#1f1a14", "#e3e3e3|#f0f0f0|#c7c7c7", "#242424|#2e2e2e|#7e7e7e"),
        ("#ffad46", "#ffad46|#ffb250|#f6cc96|#ff8609|#1f1a14", "#c98f46|#cb944e|#cba471|#ba7c36|#1f1a14", "#ffad46|#ffb250|#f6cc96|#ff8609|#1f1a14", "#f2e5d4|#f8f1e8|#eacba3", "#2a241d|#362f26|#c98f46"),
    ];


    #[test]
    fn palettes_and_tints_equal_the_js_for_day_night_and_exact() {
        let row = |p: &Palette| format!("{}|{}|{}|{}|{}", p.color, p.cover, p.inside, p.tab, p.ink());
        for (color, day, night, exact, tint_day, tint_night) in GOLDEN {
            assert_eq!(&row(&palette(color, false, false)), day, "{color} day");
            assert_eq!(&row(&palette(color, true, false)), night, "{color} night");
            assert_eq!(&row(&palette(color, false, true)), exact, "{color} exact");
            let (a, b, c) = view_tint(color, false);
            assert_eq!(&format!("{a}|{b}|{c}"), tint_day, "{color} tint day");
            let (a, b, c) = view_tint(color, true);
            assert_eq!(&format!("{a}|{b}|{c}"), tint_night, "{color} tint night");
        }
    }

    #[test]
    fn dark_colours_get_white_ink_and_exact_keeps_the_colour() {
        let p = palette("#16a765", true, false);
        assert!(!p.dark_ink);
        let q = palette("#0b3d91", false, true);
        assert_eq!(q.color, "#0b3d91");
        assert!(!q.dark_ink);
        assert_eq!(q.ink(), "#ffffff");
        assert_eq!(q.vars()[5], ("--folder-hl", "rgba(255, 255, 255, 0.2)"));
    }

    #[test]
    fn the_ink_follows_the_contrast_rule_for_every_luminance() {
        // Exhaustive over greys: one switch from white to dark ink, at the luminance where both contrasts are equal.
        let mut flips = 0;
        let mut last = false;
        for v in 0..=255u32 {
            let p = palette(&format!("#{v:02x}{v:02x}{v:02x}"), false, true);
            let l = relative_luminance(v << 16 | v << 8 | v);
            assert_eq!(p.dark_ink, (l + 0.05) / 0.0614 >= 1.05 / (l + 0.05));
            if v > 0 && p.dark_ink != last {
                flips += 1;
            }
            last = p.dark_ink;
        }
        assert_eq!(flips, 1);
        assert!(last);
    }

    #[test]
    fn the_view_palette_is_the_tint_with_its_own_inside_and_tab() {
        let p = view_palette("#4986e7", false);
        let (bg, inside, tab) = view_tint("#4986e7", false);
        assert_eq!((p.color, p.inside, p.tab), (bg, inside, tab));
        assert_eq!(view_tint("#4986e7", true).2, display_color("#4986e7", true));
    }

    #[test]
    fn sheet_ratio_keeps_a4_plain_and_caps_extremes() {
        assert_eq!(sheet_ratio(311.0, 440.0), None);
        assert_eq!(sheet_ratio(440.0, 311.0).map(|r| crate::jsfmt::fixed(r, 4)), Some("1.4148".into()));
        assert_eq!(sheet_ratio(1000.0, 100.0), Some(2.2));
        assert_eq!(sheet_ratio(10.0, 1000.0), Some(0.35));
        assert_eq!(sheet_ratio(0.0, 10.0), None);
    }

    #[test]
    fn tone_picks_the_dominant_bin() {
        let mut px = Vec::new();
        for i in 0..24 * 16 {
            px.extend_from_slice(if i % 5 == 0 { &[10, 10, 10, 255] } else { &[250, 248, 240, 255] });
        }
        let t = tone(&px).unwrap();
        assert_eq!(t.rgb, [250, 248, 240]);
        assert_eq!((t.ink(), t.name(), t.rgb_text().as_str()), ("#000", "light", "250, 248, 240"));
        let dark = tone(&[20, 30, 40, 255].repeat(8)).unwrap();
        assert_eq!((dark.ink(), dark.name(), dark.shadow_rgb()), ("#fff", "dark", "0, 0, 0"));
        assert!(tone(&[]).is_none());
        let (x, y, w, h) = tone_source(311.0, 440.0);
        assert!((x - 311.0 * 0.04).abs() < 0.2 && (y - 440.0 * 0.66).abs() < 0.5 && w > 280.0 && h > 130.0);
    }
}
