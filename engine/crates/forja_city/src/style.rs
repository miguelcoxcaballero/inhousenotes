//! Data-driven architectural styles (`data/city/styles.toml`).

use serde::Deserialize;

use crate::kit::{Mat, Rgb, hex};
use crate::texgen::TilePattern;

/// Built-in copy of `data/city/styles.toml`; a file on disk overrides it.
pub const BUILTIN: &str = include_str!("../../../data/city/styles.toml");

fn d_ground() -> f32 {
    4.4
}
fn d_floor() -> f32 {
    3.3
}
fn d_bay() -> [f32; 2] {
    [3.0, 3.5]
}
fn d_ww() -> f32 {
    1.15
}
fn d_wh() -> f32 {
    2.35
}
fn d_shops() -> f32 {
    0.8
}

#[derive(Clone, Debug, Deserialize)]
pub struct Style {
    pub id: String,
    #[serde(default)]
    pub city: String,
    /// Upper floors (above the ground floor), inclusive range.
    pub floors: [u32; 2],
    #[serde(default = "d_ground")]
    pub ground_height: f32,
    #[serde(default = "d_floor")]
    pub floor_height: f32,
    /// Bay (window axis) spacing range.
    #[serde(default = "d_bay")]
    pub bay: [f32; 2],
    #[serde(default = "d_ww")]
    pub window_width: f32,
    #[serde(default = "d_wh")]
    pub window_height: f32,
    pub walls: Vec<String>,
    #[serde(default)]
    pub ground: String,
    #[serde(default)]
    pub surround: Vec<String>,
    /// plain | lintel | keystone | pediment | eared
    #[serde(default)]
    pub surround_kind: String,
    /// individual | noble | corrido | none
    #[serde(default)]
    pub balcony: String,
    /// bars | belly
    #[serde(default)]
    pub railing: String,
    /// `alicantina #rrggbb`, `louvre #rrggbb`, `roller #rrggbb`
    #[serde(default)]
    pub shutters: Vec<String>,
    /// Window frame paint colors.
    #[serde(default)]
    pub frames: Vec<String>,
    /// clay | mansard | balustrade | parapet
    #[serde(default)]
    pub roof: String,
    /// eave | classic | modillions
    #[serde(default)]
    pub cornice: String,
    #[serde(default)]
    pub tile_bands: Option<String>,
    #[serde(default)]
    pub tile_panels: Option<String>,
    #[serde(default)]
    pub frieze: Option<String>,
    #[serde(default)]
    pub miradores: f32,
    /// dome | spire (corner towers)
    #[serde(default)]
    pub tower: String,
    #[serde(default)]
    pub pinnacles: bool,
    #[serde(default)]
    pub crest: bool,
    #[serde(default)]
    pub rustication: bool,
    #[serde(default)]
    pub pilasters: bool,
    #[serde(default)]
    pub quoins: bool,
    #[serde(default = "d_shops")]
    pub shops: f32,
    /// Chance that a closed shop shutter is covered in graffiti.
    #[serde(default)]
    pub graffiti: f32,
    #[serde(default)]
    pub awnings: f32,
    #[serde(default)]
    pub flower_pots: f32,
    #[serde(default)]
    pub wall_lamps: bool,
    #[serde(default)]
    pub grilles: bool,
    #[serde(default)]
    pub dormers: bool,
    /// Chance that a flat (one floor, one half of the front) has replaced
    /// its joinery with white aluminium and roller blinds.
    #[serde(default)]
    pub reformed: f32,
    /// Density of lived-in facade clutter: AC condensers, satellite
    /// dishes, laundry, cables (0 = pristine, 1 = Lavapiés).
    #[serde(default)]
    pub clutter: f32,
    /// Alternative ground-floor treatments (one picked per building, `wall`
    /// = same as the upper wall with a stone plinth); empty uses `ground`.
    #[serde(default)]
    pub grounds: Vec<String>,
    /// Chance of an extra storey set back from the street behind a terrace
    /// (ático retranqueado), the usual later addition on old fronts.
    #[serde(default)]
    pub atico: f32,
}

#[derive(Deserialize)]
struct StyleFile {
    style: Vec<Style>,
}

#[derive(Clone, Debug)]
pub struct StyleLibrary {
    pub styles: Vec<Style>,
}

impl StyleLibrary {
    pub fn parse(toml_text: &str) -> Result<Self, String> {
        let f: StyleFile = toml::from_str(toml_text).map_err(|e| e.to_string())?;
        Ok(Self { styles: f.style })
    }

    pub fn builtin() -> Self {
        Self::parse(BUILTIN).expect("built-in styles parse")
    }

    /// `data/city/styles.toml` from disk, else the built-in copy.
    pub fn load(path: Option<&std::path::Path>) -> Self {
        path.and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| Self::parse(&t).map_err(|e| log::warn!("styles: {e}")).ok())
            .unwrap_or_else(Self::builtin)
    }

    pub fn get(&self, id: &str) -> Option<&Style> {
        self.styles.iter().find(|s| s.id == id)
    }

    pub fn of_city<'a>(&'a self, city: &'a str) -> impl Iterator<Item = &'a Style> + 'a {
        self.styles.iter().filter(move |s| s.city == city)
    }
}

pub fn parse_rgb(s: &str) -> Option<Rgb> {
    let h = s.trim().trim_start_matches('#');
    u32::from_str_radix(h, 16).ok().filter(|_| h.len() == 6).map(hex)
}

pub fn parse_pattern(s: &str) -> TilePattern {
    match s.trim() {
        "checker_green" => TilePattern::CheckerGreen,
        "checker_blue" => TilePattern::CheckerBlue,
        "diamond_blue" => TilePattern::DiamondBlue,
        "frieze" => TilePattern::Frieze,
        _ => TilePattern::White,
    }
}

/// `"stucco #e8d8b8"` → material.
pub fn parse_material(s: &str) -> Mat {
    let mut it = s.split_whitespace();
    let kind = it.next().unwrap_or("stucco");
    let arg = it.next().unwrap_or("");
    let c = parse_rgb(arg).unwrap_or([255; 3]);
    match kind {
        "plaster" => Mat::Plaster(c),
        "brick" => Mat::Brick(c),
        "ashlar" => Mat::Ashlar(c),
        "granite" => Mat::Granite,
        "moulding" => Mat::Moulding(c),
        "tiles" => Mat::Tiles(parse_pattern(arg)),
        _ => Mat::Stucco(c),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShutterKind {
    /// Valencian rolled wooden slat blind hanging outside.
    Alicantina,
    /// Wooden louvred leaves (contraventanas de librillo).
    Louvre,
    /// Roller blind inside the reveal.
    Roller,
}

pub fn parse_shutter(s: &str) -> (ShutterKind, Rgb) {
    let mut it = s.split_whitespace();
    let kind = match it.next().unwrap_or("") {
        "alicantina" => ShutterKind::Alicantina,
        "roller" => ShutterKind::Roller,
        _ => ShutterKind::Louvre,
    };
    (kind, it.next().and_then(parse_rgb).unwrap_or(hex(0x5a3a22)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_styles_parse_and_cover_both_cities() {
        let lib = StyleLibrary::builtin();
        assert!(lib.of_city("madrid").count() >= 3);
        assert!(lib.of_city("valencia").count() >= 4);
        for s in &lib.styles {
            assert!(!s.walls.is_empty(), "{}", s.id);
            assert!(s.floors[0] <= s.floors[1]);
            for w in &s.walls {
                let _ = parse_material(w);
            }
        }
        assert_eq!(parse_material("tiles checker_green"), Mat::Tiles(TilePattern::CheckerGreen));
        assert_eq!(parse_material("brick #ffffff"), Mat::Brick([255, 255, 255]));
    }
}
