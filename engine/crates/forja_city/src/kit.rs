//! Material palette of the city kit and the assembler that turns
//! per-material primitives into render scenes.
//!
//! Scanned PBR sets come from `assets/downloaded` (Poly Haven, CC0) when
//! present; procedural ones (tiles, signs, shop signs, graffiti) are
//! generated here. Missing scans fall back to flat colors so the kit always
//! builds, including in tests.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use forja_assets::{AlphaMode, MaterialData, NodeData, PbrSet, SceneData, TextureData};
use forja_math::{Vec3, Vec4};

use rayon::prelude::*;

use crate::geom::{Parts, Prim};
use crate::macrotex::{self, Recipe};
use crate::texgen::{self, GenSet, SignKind, TilePattern};
use crate::weathering::{self, Decal, DecalTextures};

pub type Rgb = [u8; 3];

pub const fn hex(c: u32) -> Rgb {
    [(c >> 16) as u8, (c >> 8) as u8, c as u8]
}

/// Material key. Tinted variants share their textures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mat {
    Stucco(Rgb),
    Plaster(Rgb),
    Brick(Rgb),
    Ashlar(Rgb),
    Moulding(Rgb),
    Granite,
    RoofClay,
    RoofSlate,
    RoofTerrace,
    Zinc,
    Iron,
    Glass,
    Interior,
    ShopInterior,
    PaintedWood(Rgb),
    Wood(Rgb),
    /// 0 = clean, 1..=3 graffiti variants.
    MetalShutter(u8),
    Tiles(TilePattern),
    Awning(u8),
    ShopSign,
    Terracotta,
    Leaves,
    Flowers(Rgb),
    Asphalt,
    SidewalkHex,
    SidewalkSlab,
    Curb,
    Cobble,
    Paint(Rgb),
    Galvanized,
    Plastic(Rgb),
    SignFace(SignKind),
    Lamp(Rgb),
    /// Traffic light lens: color, lit.
    Lens(Rgb, bool),
    PedLens(bool),
    Plate(u8),
    Water,
    Grass,
    Soil,
    /// Weathering decal and dirt level (index into [`weathering::LEVELS`]).
    Decal(Decal, u8),
}

impl Mat {
    /// Meters per texture repeat for planar UVs. Scanned sets repeat once
    /// per macro texture (several scan repeats, see [`scan_layout`]).
    pub fn scale(self) -> f32 {
        match self {
            Mat::Stucco(_) => macro_scale("arch_stucco"),
            Mat::Plaster(_) => macro_scale("arch_plaster_ochre"),
            Mat::Brick(_) => macro_scale("arch_brick_red"),
            Mat::Ashlar(_) => macro_scale("arch_ashlar"),
            Mat::Moulding(_) => macro_scale("arch_moulding"),
            Mat::Granite => macro_scale("arch_granite"),
            Mat::RoofClay => macro_scale("arch_roof_clay"),
            Mat::RoofSlate => macro_scale("arch_roof_slate"),
            Mat::RoofTerrace => macro_scale("pbr_roof"),
            Mat::Tiles(_) => 0.6,
            Mat::PaintedWood(_) | Mat::Wood(_) => 1.2,
            Mat::MetalShutter(_) => 1.0,
            Mat::Asphalt => macro_scale("pbr_asphalt"),
            Mat::SidewalkHex => macro_scale("street_sidewalk_hex"),
            Mat::SidewalkSlab => macro_scale("street_sidewalk_tiles"),
            Mat::Curb => macro_scale("street_curb"),
            Mat::Cobble => macro_scale("street_cobble"),
            Mat::Grass => 3.0,
            Mat::Awning(_) => 1.0,
            _ => 1.0,
        }
    }
}

/// Meters per repeat of one scan and how many repeats per side its macro
/// texture holds (1 = used as is).
pub fn scan_layout(id: &str) -> (f32, u32) {
    match id {
        "arch_stucco" | "arch_plaster_ochre" => (2.5, 4),
        "arch_brick_red" => (1.4, 3),
        "arch_ashlar" => (1.6, 2),
        "arch_moulding" => (1.2, 3),
        "arch_granite" => (1.5, 2),
        "arch_roof_clay" => (2.0, 2),
        "arch_roof_slate" => (1.6, 2),
        "pbr_roof" => (3.0, 2),
        "pbr_asphalt" => (5.0, 3),
        "street_sidewalk_hex" => (1.6, 3),
        "street_sidewalk_tiles" => (2.0, 3),
        "street_curb" => (1.0, 2),
        "street_cobble" => (2.0, 2),
        _ => (1.0, 1),
    }
}

fn macro_scale(id: &str) -> f32 {
    let (m, t) = scan_layout(id);
    m * t as f32
}

/// Weathering and calibration of each scanned set (see [`macrotex`]).
/// Roughness targets: weathered stucco and lime plaster 0.85–0.95, brick
/// and sandstone 0.75–0.85, sawn granite 0.7 with foot-polished slabs
/// near 0.6, asphalt 0.75–0.9 with glossy tar seams and oil.
pub fn recipe(id: &str) -> Option<Recipe> {
    let (base, tiles) = scan_layout(id);
    if tiles <= 1 {
        return None;
    }
    let seed = id.bytes().fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    let r = Recipe { tiles, meters: base * tiles as f32, seed, ..Default::default() };
    Some(match id {
        "arch_stucco" | "arch_plaster_ochre" => Recipe {
            remix: true,
            rotate: true,
            mottle: 0.07,
            mottle_m: 3.0,
            hue: 0.025,
            streaks: 0.05,
            rough_var: 0.05,
            patches: 6,
            patch_value: 0.07,
            patch_rough: -0.06,
            patch_m: [0.5, 2.2],
            patch_flat: 0.55,
            stains: 3,
            stain_dark: 0.12,
            stain_m: [0.6, 2.0],
            cracks: 4,
            crack_w: 0.0025,
            crack_dark: 0.35,
            normal: 0.9,
            ..r
        },
        "arch_brick_red" => Recipe { saturation: 0.9, mottle: 0.1, mottle_m: 2.0, hue: 0.03, rough_mul: 1.55, rough_var: 0.06, stains: 4, stain_dark: 0.25, patches: 2, patch_value: 0.08, patch_m: [0.4, 1.2], ..r },
        "arch_ashlar" => Recipe { saturation: 0.55, mottle: 0.08, mottle_m: 1.5, rough_mul: 1.45, rough_var: 0.05, stains: 3, stain_dark: 0.15, stain_m: [0.3, 1.0], ..r },
        "arch_moulding" => Recipe { remix: true, rotate: true, saturation: 0.6, mean: Some(0.5), mottle: 0.06, mottle_m: 1.5, rough_var: 0.04, ..r },
        "arch_granite" => Recipe { remix: true, rotate: true, mottle: 0.06, mottle_m: 1.2, rough_mul: 0.82, rough_var: 0.06, stains: 2, stain_dark: 0.12, ..r },
        "arch_roof_clay" => Recipe { mottle: 0.12, mottle_m: 1.5, hue: 0.04, stains: 4, stain_dark: 0.22, rough_var: 0.04, ..r },
        "arch_roof_slate" | "pbr_roof" => Recipe { mottle: 0.08, mottle_m: 1.5, stains: 3, stain_dark: 0.12, rough_var: 0.05, ..r },
        "pbr_asphalt" => Recipe {
            remix: true,
            rotate: true,
            mottle: 0.1,
            mottle_m: 4.0,
            rough_var: 0.06,
            patches: 4,
            patch_value: 0.12,
            patch_rough: -0.05,
            patch_m: [1.2, 3.5],
            patch_flat: 0.3,
            stains: 6,
            stain_dark: 0.2,
            stain_m: [0.4, 1.5],
            oil: 8,
            cracks: 10,
            crack_w: 0.012,
            crack_dark: 0.6,
            crack_rough: 0.35,
            ..r
        },
        "street_sidewalk_hex" => Recipe { saturation: 0.4, mean: Some(0.24), mottle: 0.08, mottle_m: 1.2, stains: 5, stain_dark: 0.18, gum: 150, oil: 1, rough_var: 0.05, ..r },
        "street_sidewalk_tiles" => Recipe { saturation: 0.6, mottle: 0.08, mottle_m: 1.5, stains: 5, stain_dark: 0.18, gum: 200, rough_add: -0.1, rough_var: 0.06, ..r },
        "street_curb" => Recipe { mottle: 0.08, mottle_m: 0.8, stains: 2, stain_dark: 0.15, rough_var: 0.05, ..r },
        "street_cobble" => Recipe { mottle: 0.1, mottle_m: 1.2, stains: 4, stain_dark: 0.18, gum: 30, rough_var: 0.05, ..r },
        _ => r,
    })
}

/// Names on the street plates (`Mat::Plate(i)`).
pub const STREETS: [&str; 8] = ["DE ALCALÁ", "DE COLÓN", "DE RUZAFA", "DEL CARMEN", "DE LA PAZ", "DE CIRILO AMORÓS", "DE LAVAPIÉS", "DE SORNÍ"];

pub struct Kit {
    pbr: HashMap<&'static str, PbrSet>,
    tiles: HashMap<TilePattern, GenSet>,
    signs: HashMap<SignKind, TextureData>,
    ped: [TextureData; 2],
    shop_signs: TextureData,
    graffiti: Vec<TextureData>,
    awnings: Vec<TextureData>,
    interiors: TextureData,
    shop_interiors: TextureData,
    plates: Vec<TextureData>,
    decals: DecalTextures,
}

const SETS: [&str; 19] = [
    "arch_stucco",
    "arch_plaster_ochre",
    "arch_brick_red",
    "arch_ashlar",
    "arch_moulding",
    "arch_granite",
    "arch_roof_clay",
    "arch_roof_slate",
    "arch_wood_shutter",
    "arch_metal_shutter",
    "street_sidewalk_hex",
    "street_sidewalk_tiles",
    "street_curb",
    "street_cobble",
    "pbr_asphalt",
    "pbr_roof",
    "pbr_grass",
    "pbr_rock",
    "pbr_sand",
];

impl Kit {
    /// Loads scanned sets from `downloaded` (missing ones fall back to flat
    /// colors) and generates the procedural textures.
    pub fn load(downloaded: Option<&Path>) -> Self {
        // Decode and weather the scans in parallel (macro textures take a
        // fraction of a second each in release builds).
        let found: Vec<(&'static str, std::path::PathBuf)> = match downloaded {
            Some(root) => SETS.iter().map(|id| (*id, root.join(id))).filter(|(_, d)| d.exists()).collect(),
            None => Vec::new(),
        };
        let mut pbr: HashMap<&'static str, PbrSet> = found
            .into_par_iter()
            .filter_map(|(id, dir)| match forja_assets::load_pbr_set(&dir) {
                Ok(s) => Some((id, match recipe(id) {
                    Some(r) => macrotex::build(&s, &r),
                    None => s,
                })),
                Err(e) => {
                    log::warn!("{id}: {e}");
                    None
                }
            })
            .collect();
        // Grain without hue: paint and stain colors come from the tint.
        if let Some(w) = pbr.get("arch_wood_shutter").cloned() {
            let mut gray = w.clone();
            gray.color = desaturate(&w.color, 0.82, 0.45);
            gray.name = "arch_wood_gray".into();
            pbr.insert("arch_wood_gray", gray);
        }
        let shutter_base = pbr.get("arch_metal_shutter").map(|s: &PbrSet| s.color.clone()).unwrap_or_else(|| texgen::Canvas::new(256, 256, texgen::rgb(0x8a9096)).to_texture("shutter", true));
        let tiles = [TilePattern::CheckerGreen, TilePattern::CheckerBlue, TilePattern::DiamondBlue, TilePattern::Frieze, TilePattern::White]
            .into_iter()
            .enumerate()
            .map(|(i, p)| (p, texgen::tiles(p, i as u64 + 3)))
            .collect();
        let kit = Kit {
            tiles,
            signs: SignKind::ALL.iter().map(|k| (*k, texgen::sign_face(*k))).collect(),
            ped: [texgen::pedestrian_lens(false), texgen::pedestrian_lens(true)],
            shop_signs: texgen::shop_sign_atlas(),
            graffiti: (0..3).map(|i| texgen::graffiti_over(&shutter_base, 11 + i)).collect(),
            awnings: vec![texgen::awning(0x2e6b3a, 0xf1ead8), texgen::awning(0xb3261e, 0xf3eee2), texgen::awning(0xd9b779, 0xd9b779)],
            interiors: texgen::window_interiors(),
            shop_interiors: texgen::shop_interiors(),
            plates: STREETS.iter().map(|s| texgen::street_plate(s)).collect(),
            decals: DecalTextures::generate(),
            pbr,
        };
        log::info!("city kit: {} scanned materials", kit.pbr.len());
        kit
    }

    pub fn has_scans(&self) -> bool {
        !self.pbr.is_empty()
    }

    fn set(&self, id: &str) -> Option<&PbrSet> {
        self.pbr.get(id)
    }
}

/// Luminance-only copy of a color texture, mean brought to `mean`, with
/// grain contrast scaled by `contrast`.
fn desaturate(t: &TextureData, mean: f32, contrast: f32) -> TextureData {
    let lum: Vec<f32> = t.rgba8.chunks_exact(4).map(|c| (0.2126 * c[0] as f32 + 0.7152 * c[1] as f32 + 0.0722 * c[2] as f32) / 255.0).collect();
    let avg = lum.iter().sum::<f32>() / lum.len().max(1) as f32;
    let mut out = Vec::with_capacity(t.rgba8.len());
    for l in lum {
        let v = ((mean + (l - avg) * contrast).clamp(0.0, 1.0) * 255.0) as u8;
        out.extend([v, v, v, 255]);
    }
    TextureData { name: format!("{}_gray", t.name), width: t.width, height: t.height, rgba8: Arc::from(out), srgb: true }
}

fn tint(c: Rgb) -> Vec4 {
    let f = |v: u8| {
        let s = v as f32 / 255.0;
        if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    };
    Vec4::new(f(c[0]), f(c[1]), f(c[2]), 1.0)
}

/// Builds render scenes from kit primitives, adding each material and
/// texture once per scene.
pub struct Assembler<'a> {
    pub kit: &'a Kit,
    pub scene: SceneData,
    mats: HashMap<Mat, usize>,
}

impl<'a> Assembler<'a> {
    pub fn new(kit: &'a Kit, name: &str) -> Self {
        Self { kit, scene: SceneData { name: name.into(), ..Default::default() }, mats: HashMap::new() }
    }

    fn tex(&mut self, t: &TextureData) -> usize {
        if let Some(i) = self.scene.textures.iter().position(|e| Arc::ptr_eq(&e.rgba8, &t.rgba8) && e.srgb == t.srgb) {
            return i;
        }
        self.scene.textures.push(t.clone());
        self.scene.textures.len() - 1
    }

    fn scanned(&mut self, id: &str, c: Rgb, fallback: Rgb, rough: f32) -> MaterialData {
        match self.kit.set(id).cloned() {
            Some(set) => {
                let base = self.tex(&set.color);
                let normal = set.normal.as_ref().map(|t| self.tex(t));
                let orm = set.orm.as_ref().map(|t| self.tex(t));
                MaterialData {
                    name: id.into(),
                    base_color: tint(c),
                    base_color_tex: Some(base),
                    metallic: if orm.is_some() { 1.0 } else { 0.0 },
                    roughness: 1.0,
                    metallic_roughness_tex: orm,
                    normal_tex: normal,
                    occlusion_tex: orm,
                    ..Default::default()
                }
            }
            None => {
                // Flat fallback: tint times the set's typical color.
                let t = tint(c) * tint(fallback);
                MaterialData { name: id.into(), base_color: Vec4::new(t.x, t.y, t.z, 1.0), roughness: rough, ..Default::default() }
            }
        }
    }

    fn generated(&mut self, s: &GenSet, name: &str) -> MaterialData {
        let base = self.tex(&s.color);
        let normal = s.normal.as_ref().map(|t| self.tex(t));
        let orm = s.orm.as_ref().map(|t| self.tex(t));
        MaterialData { name: name.into(), base_color_tex: Some(base), metallic: 0.0, roughness: 1.0, metallic_roughness_tex: orm, normal_tex: normal, ..Default::default() }
    }

    fn textured(&mut self, t: &TextureData, name: &str, rough: f32, emissive: f32) -> MaterialData {
        let base = self.tex(t);
        MaterialData {
            name: name.into(),
            base_color_tex: Some(base),
            roughness: rough,
            emissive: Vec3::splat(emissive),
            emissive_tex: (emissive > 0.0).then_some(base),
            ..Default::default()
        }
    }

    pub fn material(&mut self, m: Mat) -> usize {
        if let Some(&i) = self.mats.get(&m) {
            return i;
        }
        let white: Rgb = [255; 3];
        let flat = |name: &str, c: Rgb, rough: f32, metal: f32| MaterialData { name: name.into(), base_color: tint(c), roughness: rough, metallic: metal, ..Default::default() };
        let kit = self.kit;
        let data = match m {
            Mat::Stucco(c) => self.scanned("arch_stucco", c, hex(0xe9e4da), 0.9),
            Mat::Plaster(c) => self.scanned("arch_plaster_ochre", c, hex(0xe2dcd2), 0.9),
            Mat::Brick(c) => self.scanned("arch_brick_red", c, hex(0x8c4a36), 0.85),
            Mat::Ashlar(c) => self.scanned("arch_ashlar", c, hex(0xd8ccb0), 0.85),
            Mat::Moulding(c) => self.scanned("arch_moulding", c, hex(0xe8e4dc), 0.8),
            Mat::Granite => self.scanned("arch_granite", white, hex(0x8f8c88), 0.8),
            Mat::RoofClay => self.scanned("arch_roof_clay", white, hex(0xb5603a), 0.85),
            Mat::RoofSlate => self.scanned("arch_roof_slate", hex(0xb8bcc4), hex(0x4a4e55), 0.6),
            Mat::RoofTerrace => self.scanned("pbr_roof", hex(0xd8d0c4), hex(0x9a948a), 0.9),
            // Weathered zinc: dull grey patina, half metallic.
            Mat::Zinc => flat("zinc", hex(0x8d9399), 0.62, 0.6),
            // Wrought iron is painted: a dielectric, satin black paint
            // (albedo ~0.02, never pure black).
            Mat::Iron => flat("iron", hex(0x2a2a28), 0.55, 0.0),
            Mat::Glass => MaterialData { name: "glass".into(), base_color: Vec4::new(0.30, 0.34, 0.33, 1.0), roughness: 0.03, transmission: 0.94, ..Default::default() },
            Mat::Interior => {
                let t = kit.interiors.clone();
                self.textured(&t, "interior", 0.9, 0.035)
            }
            Mat::ShopInterior => {
                let t = kit.shop_interiors.clone();
                let d = self.textured(&t, "shop_interior", 0.7, 0.12);
                d
            }
            Mat::PaintedWood(c) => {
                let mut d = self.scanned("arch_wood_gray", c, hex(0xd0d0d0), 0.5);
                d.normal_scale = 0.4;
                d
            }
            Mat::Wood(c) => self.scanned("arch_wood_gray", c, hex(0xd0d0d0), 0.6),
            Mat::MetalShutter(v) => {
                if v == 0 {
                    self.scanned("arch_metal_shutter", white, hex(0x9aa0a6), 0.5)
                } else {
                    let mut d = self.scanned("arch_metal_shutter", white, hex(0x9aa0a6), 0.5);
                    let g = kit.graffiti[(v as usize - 1) % kit.graffiti.len()].clone();
                    d.base_color_tex = Some(self.tex(&g));
                    d.base_color = Vec4::ONE;
                    d
                }
            }
            Mat::Tiles(p) => {
                let s = kit.tiles[&p].clone();
                self.generated(&s, "tiles")
            }
            Mat::Awning(v) => {
                let t = kit.awnings[v as usize % kit.awnings.len()].clone();
                let mut d = self.textured(&t, "awning", 0.9, 0.0);
                d.double_sided = true;
                d
            }
            Mat::ShopSign => {
                let t = kit.shop_signs.clone();
                self.textured(&t, "shop_sign", 0.5, 0.6)
            }
            Mat::Terracotta => flat("terracotta", hex(0xa9542f), 0.85, 0.0),
            Mat::Leaves => MaterialData { double_sided: true, ..flat("leaves", hex(0x2f5a22), 0.75, 0.0) },
            Mat::Flowers(c) => flat("flowers", c, 0.6, 0.0),
            Mat::Asphalt => self.scanned("pbr_asphalt", hex(0xb0b0b0), hex(0x2a2a2c), 0.85),
            Mat::SidewalkHex => self.scanned("street_sidewalk_hex", white, hex(0xb9b2a4), 0.85),
            Mat::SidewalkSlab => self.scanned("street_sidewalk_tiles", hex(0xd0d0d0), hex(0x9c9a96), 0.85),
            Mat::Curb => self.scanned("street_curb", hex(0xe0e0e0), hex(0x8e8e8e), 0.75),
            Mat::Cobble => self.scanned("street_cobble", white, hex(0x8a8a88), 0.8),
            Mat::Paint(c) => flat("paint", c, 0.55, 0.0),
            // Hot-dip galvanizing weathers to a dull spangled grey.
            Mat::Galvanized => flat("galvanized", hex(0xa8acb0), 0.52, 1.0),
            Mat::Plastic(c) => flat("plastic", c, 0.5, 0.0),
            Mat::SignFace(k) => {
                let t = kit.signs[&k].clone();
                self.textured(&t, "sign", 0.35, 0.08)
            }
            Mat::Lamp(c) => MaterialData { name: "lamp".into(), base_color: tint(c), emissive: tint(c).truncate() * 12.0, roughness: 0.3, ..Default::default() },
            Mat::Lens(c, on) => {
                if on {
                    MaterialData { name: "lens_on".into(), base_color: tint(c), emissive: tint(c).truncate() * 18.0, roughness: 0.2, ..Default::default() }
                } else {
                    MaterialData { name: "lens_off".into(), base_color: tint(c) * 0.08, roughness: 0.15, ..Default::default() }
                }
            }
            Mat::PedLens(walk) => {
                let t = kit.ped[walk as usize].clone();
                self.textured(&t, "ped_lens", 0.2, 6.0)
            }
            Mat::Plate(i) => {
                let t = kit.plates[i as usize % kit.plates.len()].clone();
                let mut d = self.textured(&t, "plate", 0.2, 0.0);
                d.alpha = AlphaMode::Opaque;
                d
            }
            Mat::Water => MaterialData { name: "water".into(), base_color: Vec4::new(0.03, 0.10, 0.11, 1.0), roughness: 0.02, ..Default::default() },
            Mat::Grass => self.scanned("pbr_grass", hex(0xd8e0c8), hex(0x4a6a32), 0.95),
            Mat::Soil => self.scanned("pbr_sand", hex(0x8a7058), hex(0x5a4636), 0.95),
            Mat::Decal(d, level) => {
                let t = kit.decals.get(d).clone();
                let base = self.tex(&t);
                let a = weathering::LEVELS[level as usize % weathering::LEVELS.len()];
                MaterialData { name: format!("decal_{d:?}"), base_color: Vec4::new(1.0, 1.0, 1.0, a), base_color_tex: Some(base), roughness: 0.95, alpha: AlphaMode::Blend, ..Default::default() }
            }
        };
        self.scene.materials.push(data);
        let i = self.scene.materials.len() - 1;
        self.mats.insert(m, i);
        i
    }

    /// Adds `parts` as one node. Meshes above `lod_min` triangles get
    /// simplified levels.
    pub fn add_parts(&mut self, parts: Parts<Mat>, lod_min: Option<usize>) {
        let mut prims = Vec::new();
        let Parts { prims: mut map, order } = parts;
        for k in order {
            let p: Prim = map.remove(&k).unwrap_or_default();
            if p.indices.is_empty() {
                continue;
            }
            let mi = self.material(k);
            let mut mesh = p.into_mesh(mi);
            if let Some(min) = lod_min {
                forja_assets::lod::generate_mesh_lods(&mut mesh, 5, min);
            }
            prims.push(mesh);
        }
        if prims.is_empty() {
            return;
        }
        let mesh = self.scene.meshes.len();
        self.scene.meshes.push(prims);
        let node = self.scene.nodes.len();
        self.scene.nodes.push(NodeData { name: format!("part{node}"), mesh: Some(mesh), ..Default::default() });
        self.scene.roots.push(node);
    }

    pub fn finish(self) -> SceneData {
        self.scene
    }
}

/// One-call helper.
pub fn assemble(kit: &Kit, name: &str, parts: Parts<Mat>, lod_min: Option<usize>) -> SceneData {
    let mut a = Assembler::new(kit, name);
    a.add_parts(parts, lod_min);
    a.finish()
}
