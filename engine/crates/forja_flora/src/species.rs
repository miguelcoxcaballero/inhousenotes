//! Data-driven species definitions (`data/flora/species.toml`).
//!
//! A species is a set of parameters for one of three growth models:
//! Weber-Penn style recursive levels (`model = "parametric"`), space
//! colonization of a crown envelope (`model = "colonization"`, with
//! parametric twigs on top) or the palm builder (`kind = "palm"`). Every
//! field has a default so a new species only lists what differs.

use std::path::Path;

use serde::Deserialize;

/// The species file shipped with the engine (used when no path is given).
pub const BUILTIN: &str = include_str!("../../../data/flora/species.toml");

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Broadleaf,
    Conifer,
    Palm,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Model {
    #[default]
    Parametric,
    Colonization,
}

/// Weber-Penn crown shapes: first-level branch length against height.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    Conical,
    #[default]
    Spherical,
    Hemispherical,
    Cylindrical,
    TaperedCylindrical,
    Flame,
    InverseConical,
    TendFlame,
    /// Long branches only near the top (stone pine).
    Umbrella,
}

impl Shape {
    /// Length ratio for a branch at `x` (0 = crown base, 1 = top).
    pub fn ratio(self, x: f32) -> f32 {
        let x = x.clamp(0.0, 1.0);
        let pi = std::f32::consts::PI;
        match self {
            Shape::Conical => 1.0 - 0.8 * x,
            Shape::Spherical => 0.25 + 0.75 * (pi * (0.08 + 0.84 * x)).sin(),
            Shape::Hemispherical => 0.2 + 0.8 * (1.0 - x * x).max(0.0).sqrt(),
            Shape::Cylindrical => 1.0,
            Shape::TaperedCylindrical => 1.0 - 0.5 * x,
            Shape::Flame => {
                if x < 0.3 { 0.35 + x / 0.3 * 0.65 } else { 1.0 - (x - 0.3) / 0.7 * 0.85 }
            }
            Shape::InverseConical => 0.2 + 0.8 * x,
            Shape::TendFlame => 0.5 + 0.5 * Shape::Flame.ratio(x),
            Shape::Umbrella => 0.25 + 0.75 * x.powf(0.6),
        }
    }
}

/// Crown envelope used for pruning and for colonization attractors.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Envelope {
    #[default]
    Ellipsoid,
    Sphere,
    Cone,
    Column,
    /// Flat-topped dome (stone pine, mature ficus).
    Umbrella,
    /// Wide, flattened dome spreading sideways (ficus, old olives).
    Spreading,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LeafShape {
    #[default]
    Ovate,
    /// 3-5 lobed, maple-like (plane tree).
    Palmate,
    Elliptic,
    Lanceolate,
    /// Asymmetric, serrate, acuminate tip (hackberry).
    Serrate,
    /// Long needles in bundles (pines).
    Needle,
    /// Short needles all round the twig (spruce/fir).
    Shortneedle,
    /// Fern-like doubly compound leaves (jacaranda).
    Bipinnate,
    /// Flattened scale-leaf sprays (cypress).
    Scale,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BarkStyle {
    #[default]
    Furrowed,
    /// Exfoliating jigsaw patches, cream / olive / grey (Platanus).
    Plane,
    /// Grey, deeply fissured and twisted (Olea).
    Olive,
    /// Thick orange-brown plates with dark fissures (Pinus pinea).
    Pine,
    /// Smooth fine grey-green (Citrus).
    Smooth,
    /// Smooth elephant-grey with fine wrinkles (Ficus, Celtis).
    Elephant,
    /// Fine scaly grey-brown (Jacaranda).
    Scaly,
    /// Fibrous vertical strips (Cupressus).
    Fibrous,
    /// Leaf-base diamonds (Phoenix).
    Datepalm,
    /// Grey rings and vertical cracks (Washingtonia).
    Fanpalm,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TrunkDef {
    /// Base radius above the flare at maturity (m).
    pub radius: f32,
    pub radius_var: f32,
    pub lean: f32,
    pub lean_var: f32,
    /// Total bend along the trunk (degrees) and random wiggle per segment.
    pub curve: f32,
    pub curve_var: f32,
    pub segments: u32,
    /// Extra radius at the ground (multiplier) and its decay height (m).
    pub flare: f32,
    pub flare_height: f32,
    /// Root-flare lobes and their depth.
    pub lobes: u32,
    pub lobe_depth: f32,
    /// Buttress planks (ficus): amplitude (× radius) and height (m).
    pub buttress: f32,
    pub buttress_height: f32,
    /// Spiral twist of the lobes and fluting (degrees per metre).
    pub twist: f32,
    /// Axis wander (× radius) and radial fluting noise.
    pub gnarl: f32,
    pub fluting: f32,
    /// Vertical ribs (fused aerial roots of strangler figs) and their depth.
    pub ribs: u32,
    pub rib_depth: f32,
    /// Codominant split: number of leaders, where (fraction of trunk), angle.
    pub splits: u32,
    pub split_at: f32,
    pub split_angle: f32,
    /// Fraction of the tree height clear of first-level branches.
    pub bare: f32,
    /// Fraction of the tree height the leader reaches (decurrent crowns < 1).
    pub length: f32,
    /// Separate stems from the ground (olive stools).
    pub stems: u32,
    pub stem_spread: f32,
}

impl Default for TrunkDef {
    fn default() -> Self {
        Self {
            radius: 0.25,
            radius_var: 0.1,
            lean: 0.0,
            lean_var: 3.0,
            curve: 0.0,
            curve_var: 2.0,
            segments: 24,
            flare: 0.35,
            flare_height: 0.5,
            lobes: 5,
            lobe_depth: 0.25,
            buttress: 0.0,
            buttress_height: 1.0,
            twist: 0.0,
            gnarl: 0.05,
            fluting: 0.03,
            ribs: 0,
            rib_depth: 0.0,
            splits: 0,
            split_at: 0.5,
            split_angle: 25.0,
            bare: 0.3,
            length: 1.0,
            stems: 1,
            stem_spread: 20.0,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LevelDef {
    /// Children per parent (scaled by the parent's relative length).
    pub count: f32,
    pub count_var: f32,
    /// Where along the parent children grow (fractions).
    pub start: f32,
    pub end: f32,
    /// Length relative to the parent (level 1: relative to tree height).
    pub length: f32,
    pub length_var: f32,
    /// Crown shape (level 1 only).
    pub shape: Shape,
    /// Angle from the parent axis at the start and end of the parent (deg).
    pub down: f32,
    pub down_end: f32,
    pub down_var: f32,
    /// Azimuth step between successive children (deg, phyllotaxis).
    pub rotate: f32,
    pub rotate_var: f32,
    /// Children per node (whorls).
    pub whorl: u32,
    pub curve: f32,
    pub curve_var: f32,
    /// Bend toward +Y (positive) or droop (negative) per unit length.
    pub gravity: f32,
    pub segments: u32,
}

impl Default for LevelDef {
    fn default() -> Self {
        Self {
            count: 8.0,
            count_var: 2.0,
            start: 0.1,
            end: 1.0,
            length: 0.4,
            length_var: 0.1,
            shape: Shape::Spherical,
            down: 50.0,
            down_end: 40.0,
            down_var: 10.0,
            rotate: 137.5,
            rotate_var: 25.0,
            whorl: 1,
            curve: 0.0,
            curve_var: 8.0,
            gravity: 0.0,
            segments: 6,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CrownDef {
    pub envelope: Envelope,
    /// Crown diameter / tree height.
    pub width: f32,
    /// Crown base as a fraction of height (envelope bottom).
    pub base: f32,
    /// Cut branches that leave the envelope (topiary, columnar forms).
    pub prune: bool,
}

impl Default for CrownDef {
    fn default() -> Self {
        Self { envelope: Envelope::Ellipsoid, width: 0.7, base: 0.3, prune: false }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ColonizationDef {
    pub attractors: u32,
    /// Influence and kill radius (m) and growth step (m).
    pub influence: f32,
    pub kill: f32,
    pub step: f32,
    pub iterations: u32,
    /// Upward tendency added to every growth direction.
    pub tropism: f32,
    /// Where on the trunk growth starts (fraction of trunk length).
    pub start: f32,
}

impl Default for ColonizationDef {
    fn default() -> Self {
        Self { attractors: 1600, influence: 3.0, kill: 0.9, step: 0.45, iterations: 90, tropism: 0.1, start: 0.6 }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LeafDef {
    pub shape: LeafShape,
    /// Blade length (m) and width / length.
    pub length: f32,
    pub width: f32,
    pub length_var: f32,
    /// sRGB hex colours.
    pub color: String,
    pub color_alt: String,
    pub underside: String,
    pub autumn: String,
    pub twig: String,
    pub color_var: f32,
    /// Fraction of leaves showing the (lighter) underside.
    pub underside_share: f32,
    pub roughness: f32,
    /// Leaf transmission strength for the foliage shading (0..1).
    pub translucency: f32,
    /// Card edge length (m) and cards per metre of leafy stem.
    pub card_size: f32,
    pub card_var: f32,
    pub density: f32,
    pub min_cards: u32,
    pub max_cards: u32,
    /// Leaves per card spray (texture).
    pub per_card: u32,
    /// Leafy levels counted from the last (1 = only the finest).
    pub on_levels: u32,
    /// Fraction of each leafy stem (from its base) left bare.
    pub start: f32,
    /// How much cards point outward from the crown (0..1).
    pub outward: f32,
    /// How much cards hang down (0..1).
    pub droop: f32,
    /// Folded (V) cards for parallax at close range.
    pub fold: f32,
    /// Bent-normal blend toward the crown normal (0..1).
    pub bend: f32,
}

impl Default for LeafDef {
    fn default() -> Self {
        Self {
            shape: LeafShape::Ovate,
            length: 0.08,
            width: 0.5,
            length_var: 0.2,
            color: "#4f6f2a".into(),
            color_alt: "#5d7d30".into(),
            underside: "#7f9a5a".into(),
            autumn: "#b8892c".into(),
            twig: "#5a4a3a".into(),
            color_var: 0.12,
            underside_share: 0.2,
            roughness: 0.55,
            translucency: 0.6,
            card_size: 0.5,
            card_var: 0.25,
            density: 6.0,
            min_cards: 400,
            max_cards: 4500,
            per_card: 14,
            on_levels: 1,
            start: 0.2,
            outward: 0.5,
            droop: 0.2,
            fold: 25.0,
            bend: 0.7,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BarkDef {
    pub style: BarkStyle,
    /// World size of one texture tile (m): around and along the stem.
    pub tile: [f32; 2],
    /// Optional sRGB palette overrides (style-specific meaning).
    pub colors: Vec<String>,
    pub roughness: f32,
    /// Branch bark (thin stems) blends toward this colour.
    pub young: String,
}

impl Default for BarkDef {
    fn default() -> Self {
        Self { style: BarkStyle::Furrowed, tile: [0.8, 1.2], colors: Vec::new(), roughness: 0.85, young: "#6b5a48".into() }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RootDef {
    /// Surface roots radiating from the base.
    pub count: u32,
    pub length: f32,
    /// Root base radius / trunk radius.
    pub radius: f32,
    /// Aerial roots hanging from big limbs (per metre of limb).
    pub aerial: f32,
    /// Share of aerial roots that reach the ground and thicken into pillars.
    pub pillars: f32,
}

impl Default for RootDef {
    fn default() -> Self {
        Self { count: 0, length: 1.5, radius: 0.35, aerial: 0.0, pillars: 0.15 }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FruitKind {
    #[default]
    Orange,
    /// Plane-tree seed balls on long stalks (stay on in winter).
    Seedball,
    /// Date clusters under the crown.
    Dates,
    /// Pine cones.
    Cone,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FruitDef {
    pub kind: FruitKind,
    /// Fruits per leaf card (expected value).
    pub per_card: f32,
    pub radius: f32,
    pub color: String,
}

impl Default for FruitDef {
    fn default() -> Self {
        Self { kind: FruitKind::Orange, per_card: 0.05, radius: 0.04, color: "#e8820c".into() }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BloomDef {
    pub color: String,
    pub color_alt: String,
    /// Fraction of leaf cards that become flower cards in bloom.
    pub coverage: f32,
    /// Flowers come before the leaves (jacaranda): bloom cards replace most leaves.
    pub before_leaves: bool,
    /// Flower size (m).
    pub size: f32,
}

impl Default for BloomDef {
    fn default() -> Self {
        Self { color: "#8a6fd0".into(), color_alt: "#a48de0".into(), coverage: 0.5, before_leaves: false, size: 0.035 }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PalmKind {
    #[default]
    Pinnate,
    Fan,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PalmDef {
    pub kind: PalmKind,
    pub fronds: u32,
    /// Frond length (m, petiole + blade) and variation.
    pub frond_length: f32,
    pub frond_var: f32,
    /// Leaflet reach each side of the rachis (pinnate) / fan radius (m).
    pub blade_width: f32,
    /// Petiole share of the frond length.
    pub petiole: f32,
    /// How far the old fronds hang (0..1).
    pub droop: f32,
    /// Dead frond skirt length (m, 0 = trimmed).
    pub skirt: f32,
    /// Spacing of trunk rings / leaf scars (m).
    pub ring: f32,
    /// Height of the leaf-base boot under the crown (m).
    pub boot: f32,
    /// Leaflet V angle (deg) for pinnate fronds.
    pub fold: f32,
    pub color: String,
    pub color_alt: String,
    pub dead: String,
    pub translucency: f32,
    pub roughness: f32,
}

impl Default for PalmDef {
    fn default() -> Self {
        Self {
            kind: PalmKind::Pinnate,
            fronds: 70,
            frond_length: 4.5,
            frond_var: 0.15,
            blade_width: 0.6,
            petiole: 0.15,
            droop: 0.6,
            skirt: 0.0,
            ring: 0.12,
            boot: 1.0,
            fold: 35.0,
            color: "#6d7f4f".into(),
            color_alt: "#7f8f5c".into(),
            dead: "#9c8158".into(),
            translucency: 0.45,
            roughness: 0.5,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PollardDef {
    /// Trunk height to the scaffold limbs (m).
    pub height: f32,
    pub limbs: u32,
    pub limb_length: f32,
    pub limb_angle: f32,
    /// Knuckle swelling at the cut (× limb radius).
    pub knuckle: f32,
    /// Sprouts per knuckle and their length (m) at one year of regrowth.
    pub sprouts: u32,
    pub sprout_length: f32,
}

impl Default for PollardDef {
    fn default() -> Self {
        Self { height: 3.2, limbs: 4, limb_length: 2.6, limb_angle: 38.0, knuckle: 1.6, sprouts: 9, sprout_length: 2.6 }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Species {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub latin: String,
    #[serde(default)]
    pub kind: Kind,
    #[serde(default)]
    pub model: Model,
    #[serde(default)]
    pub deciduous: bool,
    /// Mature height range (m).
    pub height: [f32; 2],
    /// `natural`, `pollard`, `lollipop`, `lifted` or `trimmed`.
    #[serde(default)]
    pub pruning: Option<String>,
    #[serde(default)]
    pub trunk: TrunkDef,
    #[serde(default)]
    pub levels: Vec<LevelDef>,
    #[serde(default)]
    pub crown: CrownDef,
    #[serde(default)]
    pub colonization: Option<ColonizationDef>,
    #[serde(default)]
    pub leaves: LeafDef,
    #[serde(default)]
    pub bark: BarkDef,
    #[serde(default)]
    pub roots: RootDef,
    #[serde(default)]
    pub fruit: Option<FruitDef>,
    #[serde(default)]
    pub bloom: Option<BloomDef>,
    #[serde(default)]
    pub palm: Option<PalmDef>,
    #[serde(default)]
    pub pollard: Option<PollardDef>,
    /// Wikimedia Commons reference photos (file names) for comparisons.
    #[serde(default)]
    pub photos: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct File {
    species: Vec<Species>,
}

/// All species, in file order.
#[derive(Clone, Debug)]
pub struct SpeciesLibrary {
    pub species: Vec<Species>,
}

impl SpeciesLibrary {
    pub fn parse(text: &str) -> Result<Self, String> {
        let f: File = toml::from_str(text).map_err(|e| format!("species.toml: {e}"))?;
        let lib = SpeciesLibrary { species: f.species };
        lib.validate()?;
        Ok(lib)
    }

    /// The engine's built-in species.
    pub fn builtin() -> Self {
        Self::parse(BUILTIN).expect("built-in species.toml is valid")
    }

    /// Loads a species file, falling back to the built-in set on error.
    pub fn load(path: Option<&Path>) -> Self {
        match path.map(std::fs::read_to_string) {
            Some(Ok(text)) => match Self::parse(&text) {
                Ok(l) => l,
                Err(e) => {
                    log::warn!("{e}; using the built-in species");
                    Self::builtin()
                }
            },
            Some(Err(e)) => {
                log::warn!("species file: {e}; using the built-in species");
                Self::builtin()
            }
            None => Self::builtin(),
        }
    }

    pub fn get(&self, id: &str) -> Option<&Species> {
        self.species.iter().find(|s| s.id == id || s.name.eq_ignore_ascii_case(id))
    }

    pub fn ids(&self) -> Vec<&str> {
        self.species.iter().map(|s| s.id.as_str()).collect()
    }

    fn validate(&self) -> Result<(), String> {
        let mut seen = std::collections::HashSet::new();
        for s in &self.species {
            if !seen.insert(s.id.as_str()) {
                return Err(format!("duplicate species id {}", s.id));
            }
            if s.height[0] <= 0.0 || s.height[1] < s.height[0] {
                return Err(format!("{}: bad height range", s.id));
            }
            if s.kind == Kind::Palm && s.palm.is_none() {
                return Err(format!("{}: palms need a [species.palm] table", s.id));
            }
            if s.kind != Kind::Palm && s.levels.is_empty() {
                return Err(format!("{}: needs at least one [[species.levels]]", s.id));
            }
            if let Some(p) = &s.pruning
                && crate::Pruning::parse(p).is_none()
            {
                return Err(format!("{}: unknown pruning {p}", s.id));
            }
            for c in [&s.leaves.color, &s.leaves.color_alt, &s.leaves.underside, &s.leaves.autumn, &s.leaves.twig, &s.bark.young] {
                parse_hex(c).ok_or_else(|| format!("{}: bad colour {c}", s.id))?;
            }
        }
        Ok(())
    }
}

/// `#rrggbb` → linear RGB.
pub fn parse_hex(s: &str) -> Option<[f32; 3]> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    let c = |s: u32| srgb_to_linear(((v >> s) & 0xff) as f32 / 255.0);
    Some([c(16), c(8), c(0)])
}

pub fn hex(s: &str) -> [f32; 3] {
    parse_hex(s).unwrap_or([0.5, 0.5, 0.5])
}

pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

pub fn linear_to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.0031308 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_species_parse() {
        let lib = SpeciesLibrary::builtin();
        for id in ["platanus", "naranjo", "olivo", "phoenix", "washingtonia", "pino_pinonero", "ficus", "jacaranda", "almez", "cipres", "broadleaf", "conifer"] {
            assert!(lib.get(id).is_some(), "missing {id}");
        }
    }

    #[test]
    fn shapes_are_bounded() {
        for s in [Shape::Conical, Shape::Spherical, Shape::Hemispherical, Shape::Cylindrical, Shape::TaperedCylindrical, Shape::Flame, Shape::InverseConical, Shape::TendFlame, Shape::Umbrella] {
            for i in 0..=20 {
                let r = s.ratio(i as f32 / 20.0);
                assert!((0.0..=1.01).contains(&r), "{s:?} {r}");
            }
        }
    }
}
