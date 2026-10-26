//! # forja_flora
//!
//! Procedural trees for photoreal Spanish cities.
//!
//! ```text
//! species.toml ──► Species ──► skeleton (Weber-Penn levels | space colonization | palm)
//!                                 │ pipe-model radii, roots, aerial roots, pruning
//!                                 ▼
//!              bark tubes + leaf cards / fronds + fruit ──► LOD 0 / 1 / 2 (billboard)
//!                                 │
//!              procedural bark + foliage atlas ──► impostor views (8 directions)
//!                                 ▼
//!                             TreeAsset (SceneData, wind weights, capsule)
//! ```
//!
//! [`generate`] is deterministic in `(species, seed, params)`. A
//! [`TreeAsset`] is one model with two primitives (bark, foliage); each
//! carries its coarser levels in `MeshData::lods` over a shared vertex
//! buffer, so the renderer's per-mesh LOD selection switches them for free:
//! LOD 0 (20-80k triangles), LOD 1 (3-8k) and LOD 2 (far trunk plus an
//! 8-quad impostor billboard whose views live in the foliage atlas).

pub mod city;
pub mod foliage;
pub mod gltf;
pub mod impostor;
pub mod mesh;
pub mod palm;
pub mod rng;
pub mod skeleton;
pub mod species;
pub mod texture;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use forja_assets::{AlphaMode, FoliageShading, MaterialData, NodeData, SceneData, TextureData};
use forja_math::{Vec2, Vec3, Vec4};

use crate::mesh::{MeshBuilder, TrunkShape, TubeOpts};
use crate::rng::{Rng, hash_str};
use crate::skeleton::{Growth, Skeleton, StemKind};
use crate::species::{Kind, Model, Species};
use crate::texture::{FoliageAtlas, FoliageStyle, TextureSet};

pub use crate::species::SpeciesLibrary;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Season {
    #[default]
    Summer,
    /// Blossom (jacaranda, orange blossom).
    Spring,
    Autumn,
    /// Deciduous species are leafless.
    Winter,
}

impl Season {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.to_ascii_lowercase().as_str() {
            "summer" | "verano" => Season::Summer,
            "spring" | "primavera" | "bloom" | "blossom" => Season::Spring,
            "autumn" | "fall" | "otono" | "otoño" => Season::Autumn,
            "winter" | "invierno" => Season::Winter,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Pruning {
    #[default]
    Natural,
    /// Street plane trees: scaffold limbs with knuckles and sprouts.
    Pollard,
    /// Clipped globe (orange trees in plazas).
    Lollipop,
    /// Crown raised above the street clearance.
    Lifted,
    /// Palms with the dead fronds removed.
    Trimmed,
}

impl Pruning {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.to_ascii_lowercase().as_str() {
            "natural" => Pruning::Natural,
            "pollard" | "pollarded" => Pruning::Pollard,
            "lollipop" | "globe" | "topiary" => Pruning::Lollipop,
            "lifted" | "street" => Pruning::Lifted,
            "trimmed" | "clean" => Pruning::Trimmed,
            _ => return None,
        })
    }
}

/// Generation parameters.
#[derive(Clone, Debug)]
pub struct Params {
    /// 1 = mature; 0.2 sapling … 1.5 veteran.
    pub age: f32,
    /// Extra scale on the species height.
    pub size: f32,
    pub season: Season,
    /// 1 = healthy … 0 = dying (thin crown, yellowing, dead limbs).
    pub health: f32,
    /// `None`: the species default.
    pub pruning: Option<Pruning>,
    /// Bake the impostor views on the CPU (otherwise the billboard is blank
    /// until [`impostor::bake`] or [`impostor::share`]).
    pub bake_impostor: bool,
}

impl Default for Params {
    fn default() -> Self {
        Params { age: 1.0, size: 1.0, season: Season::Summer, health: 1.0, pruning: None, bake_impostor: true }
    }
}

/// Vertical collision capsule (trunk).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Capsule {
    pub a: Vec3,
    pub b: Vec3,
    pub radius: f32,
}

/// Square impostor tile in tree space.
#[derive(Clone, Copy, Debug)]
pub struct Billboard {
    pub center: Vec3,
    pub half: f32,
}

#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub stems: usize,
    pub cards: usize,
    pub fruits: usize,
    pub fronds: usize,
    pub ms: f32,
}

#[derive(Clone, Debug)]
pub struct TreeAsset {
    pub species: String,
    pub seed: u64,
    pub params: Params,
    /// One node, one mesh with `[bark, foliage]`; LODs in `MeshData::lods`.
    pub scene: SceneData,
    /// Per-vertex wind sway weight (0 = rigid … 1 = free tip), parallel to
    /// the primitives of `scene.meshes[0]`.
    pub wind: Vec<Vec<f32>>,
    pub capsule: Capsule,
    pub height: f32,
    pub crown_center: Vec3,
    pub crown_radii: Vec3,
    /// Triangles of LOD 0, 1, 2 (both primitives).
    pub lod_triangles: [usize; 3],
    pub billboard: Billboard,
    pub stats: Stats,
}

pub const BARK: usize = 0;
pub const FOLIAGE: usize = 1;
/// Texture indices in [`TreeAsset::scene`].
pub const FOLIAGE_ALBEDO: usize = 3;
pub const ALPHA_CUTOFF: u8 = 107;

impl TreeAsset {
    /// The scene with only one LOD level (indices replaced), for exports,
    /// impostor capture and tests.
    pub fn lod_scene(&self, lod: usize) -> SceneData {
        let mut s = self.scene.clone();
        for prims in &mut s.meshes {
            for p in prims.iter_mut() {
                if lod > 0 {
                    p.indices = p.lods.get(lod - 1).cloned().unwrap_or_default();
                }
                p.lods.clear();
            }
            prims.retain(|p| !p.indices.is_empty());
        }
        s
    }

    pub fn triangles(&self, prim: usize, lod: usize) -> usize {
        let p = &self.scene.meshes[0][prim];
        if lod == 0 { p.indices.len() / 3 } else { p.lods.get(lod - 1).map(|l| l.len() / 3).unwrap_or(0) }
    }
}

type TexKey = (String, Season, u8, bool);

struct TexCache {
    bark: HashMap<String, Arc<TextureSet>>,
    foliage: HashMap<TexKey, Arc<FoliageAtlas>>,
}

fn cache() -> &'static Mutex<TexCache> {
    static C: OnceLock<Mutex<TexCache>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(TexCache { bark: HashMap::new(), foliage: HashMap::new() }))
}

fn bark_textures(sp: &Species) -> Arc<TextureSet> {
    if let Some(t) = cache().lock().unwrap().bark.get(&sp.id) {
        return t.clone();
    }
    let t = Arc::new(texture::bark_set(sp, hash_str(&sp.id) as u32));
    cache().lock().unwrap().bark.insert(sp.id.clone(), t.clone());
    t
}

fn foliage_textures(st: &FoliageStyle) -> Arc<FoliageAtlas> {
    let key = (st.sp.id.clone(), st.season, (st.health.clamp(0.0, 1.0) * 4.0).round() as u8, st.pruning == Pruning::Trimmed);
    if let Some(t) = cache().lock().unwrap().foliage.get(&key) {
        return t.clone();
    }
    let t = Arc::new(texture::foliage_atlas(st, hash_str(&st.sp.id) as u32 ^ 0x1eaf));
    cache().lock().unwrap().foliage.insert(key, t.clone());
    t
}

/// Default pruning: the species' own, with palms trimmed in cities.
pub fn resolve_pruning(sp: &Species, p: &Params) -> Pruning {
    p.pruning.or_else(|| sp.pruning.as_deref().and_then(Pruning::parse)).unwrap_or(Pruning::Natural)
}

/// Generates one tree.
pub fn generate(sp: &Species, seed: u64, params: &Params) -> TreeAsset {
    let t0 = std::time::Instant::now();
    let mut rng = Rng::new(seed ^ hash_str(&sp.id));
    let pruning = resolve_pruning(sp, params);
    let age = params.age.clamp(0.15, 1.6);
    let height = rng.range(sp.height[0], sp.height[1]) * params.size * age.powf(0.65);
    let trunk_radius = sp.trunk.radius * (1.0 + sp.trunk.radius_var * rng.signed()) * age.powf(1.1) * (height / sp.height[1]).powf(0.6).max(0.3);
    let clearance = match pruning {
        Pruning::Lifted | Pruning::Pollard => (height * 0.3).clamp(1.8, 3.6),
        _ => 0.0,
    };
    let g = Growth { height, trunk_radius, age, health: params.health.clamp(0.0, 1.0), pruning, clearance };
    let st = FoliageStyle { sp, season: params.season, health: g.health, pruning };
    let bark = bark_textures(sp);
    let atlas = foliage_textures(&st);
    let mut stats = Stats::default();

    let (skel, mut fol_levels, crown_center, crown_radii) = if sp.kind == Kind::Palm {
        let pd = sp.palm.as_ref().expect("validated");
        let tree = palm::build(sp, pd, &g, &mut rng.fork(1));
        let mut lv: [MeshBuilder; 2] = Default::default();
        for (lod, mb) in lv.iter_mut().enumerate() {
            palm::mesh_fronds(mb, pd, &tree.fronds, &atlas, tree.crown_center, lod);
        }
        stats.fronds = tree.fronds.len();
        (tree.skeleton, lv, tree.crown_center, tree.crown_radii)
    } else {
        let sk = match sp.model {
            Model::Colonization => skeleton::colonization(sp, &g, &mut rng.fork(1)),
            Model::Parametric => skeleton::parametric(sp, &g, &mut rng.fork(1)),
        };
        let fol = foliage::place(&sk, &st, &atlas, &mut rng.fork(2));
        stats.cards = fol.cards.len();
        stats.fruits = fol.fruits.len();
        let lv = leaf_levels(sp, &fol, &atlas);
        (sk, lv, fol.crown_center, fol.crown_radii)
    };
    stats.stems = skel.stems.len();
    let trunk_shape = {
        let mut ts = TrunkShape::new(&sp.trunk, &mut rng.fork(3), height / sp.height[1]);
        if let Some(pd) = &sp.palm {
            ts.ring = pd.ring;
            ts.flare = 0.0;
            ts.lobes = 0;
            ts.planks.clear();
            ts.ring_depth = if pd.kind == species::PalmKind::Pinnate { 0.07 } else { 0.03 };
        }
        ts
    };
    let crown_bottom = if sp.kind == Kind::Palm { f32::MAX } else { (crown_center.y - crown_radii.y * 0.8).max(0.8) };
    let bark_levels = bark_levels(sp, &skel, &trunk_shape, fol_levels[1].triangles(), crown_bottom);

    // Billboard (LOD 2 of the foliage) from the LOD 0 bounds.
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    for p in bark_levels[0].pos.iter().chain(fol_levels[0].pos.iter()) {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    let reach = bark_levels[0].pos.iter().chain(fol_levels[0].pos.iter()).map(|p| Vec2::new(p.x, p.z).length()).fold(0.0f32, f32::max);
    let half = (reach.max(hi.y * 0.5) * 1.03).max(0.5);
    let bb = Billboard { center: Vec3::new(0.0, half, 0.0), half };
    let billboard = impostor::billboard_mesh(&bb, crown_center);

    let fol0 = std::mem::take(&mut fol_levels[0]);
    let fol1 = std::mem::take(&mut fol_levels[1]);
    let (bark_mesh, bark_wind) = mesh::merge_lods(&bark_levels, BARK);
    let (fol_mesh, fol_wind) = mesh::merge_lods(&[fol0, fol1, billboard], FOLIAGE);
    let lod_triangles = [0usize, 1, 2].map(|l| tri(&bark_mesh, l) + tri(&fol_mesh, l));

    let translucency = sp.palm.as_ref().map(|p| p.translucency).unwrap_or(sp.leaves.translucency);
    let roughness = 1.0;
    let mut scene = SceneData { name: format!("tree_{}_{}", sp.id, seed), ..Default::default() };
    scene.textures = vec![bark.albedo.clone(), bark.normal.clone(), bark.orm.clone(), atlas.tex.albedo.clone(), atlas.tex.normal.clone(), atlas.tex.orm.clone()];
    scene.materials = vec![
        MaterialData {
            name: format!("{}_bark", sp.id),
            base_color: Vec4::ONE,
            base_color_tex: Some(0),
            normal_tex: Some(1),
            metallic_roughness_tex: Some(2),
            occlusion_tex: Some(2),
            metallic: 0.0,
            roughness,
            ..Default::default()
        },
        MaterialData {
            name: format!("{}_foliage", sp.id),
            base_color: Vec4::ONE,
            base_color_tex: Some(3),
            normal_tex: Some(4),
            normal_scale: 0.8,
            metallic_roughness_tex: Some(5),
            metallic: 0.0,
            roughness,
            alpha: AlphaMode::Mask(ALPHA_CUTOFF),
            double_sided: true,
            foliage: Some(FoliageShading { translucency, backface_cull_v: texture::IMPOSTOR_V + 0.001 }),
            ..Default::default()
        },
    ];
    scene.meshes = vec![vec![bark_mesh, fol_mesh]];
    scene.nodes = vec![NodeData { name: scene.name.clone(), mesh: Some(0), ..Default::default() }];
    scene.roots = vec![0];

    let trunk = &skel.stems[0];
    let r1 = skeleton::interp(&trunk.radius, 1.0_f32.min((trunk.radius.len() - 1) as f32 * 0.25));
    let top = trunk.pts.iter().map(|p| p.y).fold(0.0f32, f32::max).min(height * 0.6).max(1.2);
    let capsule = Capsule { a: Vec3::new(trunk.pts[0].x, r1.max(0.15), trunk.pts[0].z), b: Vec3::new(trunk.pts[0].x, top, trunk.pts[0].z), radius: r1.max(0.08) };
    stats.ms = t0.elapsed().as_secs_f32() * 1000.0;
    let mut asset = TreeAsset {
        species: sp.id.clone(),
        seed,
        params: params.clone(),
        scene,
        wind: vec![bark_wind, fol_wind],
        capsule,
        height: hi.y.max(height * 0.5),
        crown_center,
        crown_radii,
        lod_triangles,
        billboard: bb,
        stats,
    };
    if params.bake_impostor {
        impostor::bake_cpu(&mut asset);
    }
    asset.stats.ms = t0.elapsed().as_secs_f32() * 1000.0;
    asset
}

fn tri(m: &forja_assets::MeshData, lod: usize) -> usize {
    if lod == 0 { m.indices.len() / 3 } else { m.lods.get(lod - 1).map(|l| l.len() / 3).unwrap_or(0) }
}

/// Foliage LOD 0 (folded cards + fruit) and LOD 1 (merged flat cards).
fn leaf_levels(sp: &Species, fol: &foliage::Foliage, atlas: &FoliageAtlas) -> [MeshBuilder; 2] {
    let mut l0 = MeshBuilder::default();
    let bend = sp.leaves.bend;
    for c in &fol.cards {
        mesh::card(&mut l0, c, bend, true);
    }
    let fruit_rect = atlas.of(texture::SlotKind::Fruit).first().copied().unwrap_or([0.75, 0.25, 0.875, 0.375]);
    let fruit_rect = [fruit_rect[0] + 0.01, fruit_rect[1] + 0.01, fruit_rect[2] - 0.01, fruit_rect[3] - 0.01];
    for f in &fol.fruits {
        let (rings, segs) = if f.radii.x < 0.03 { (4, 6) } else { (5, 8) };
        mesh::sphere(&mut l0, f.center, f.radii, rings, segs, fruit_rect, f.wind);
    }
    // LOD 1: every k-th card, scaled up to keep the crown's coverage.
    let mut l1 = MeshBuilder::default();
    let target = 1450usize;
    let n = fol.cards.len();
    let k = n.div_ceil(target).max(1);
    // Few cards: keep them all, folded, so the mid level keeps its volume.
    let fold_mid = n * 2 < 3200;
    let grow = (k as f32).sqrt() * 0.92;
    for (i, c) in fol.cards.iter().enumerate() {
        if i % k == 0 {
            let mut cc = *c;
            cc.length *= grow;
            cc.width *= grow;
            cc.origin -= cc.axis * cc.length * 0.1;
            mesh::card(&mut l1, &cc, bend, fold_mid);
        }
    }
    for (i, f) in fol.fruits.iter().enumerate() {
        if i % 3 == 0 {
            mesh::sphere(&mut l1, f.center, f.radii * 1.2, 2, 4, fruit_rect, f.wind);
        }
    }
    [l0, l1]
}

/// Bark LOD 0 / 1 / 2. LOD 1 stays at or below the foliage LOD 1 so the
/// trunk never drops out while the crown is still mid-detail.
fn bark_levels(sp: &Species, sk: &Skeleton, shape: &TrunkShape, leaf_mid: usize, crown_bottom: f32) -> [MeshBuilder; 3] {
    let winds = foliage::stem_wind(sk);
    let tile = sp.bark.tile;
    let palm = sp.kind == Kind::Palm;
    let mut out: [MeshBuilder; 3] = Default::default();
    // LOD 0: everything, coarsened step by step if over the near budget.
    let mut f = 1.0f32;
    for attempt in 0..8 {
        let mut mb = MeshBuilder::default();
        for (i, s) in sk.stems.iter().enumerate() {
            let trunkish = matches!(s.kind, StemKind::Trunk | StemKind::Root);
            let r0 = s.radius.first().copied().unwrap_or(0.0);
            let thin = r0 < 0.02;
            if attempt >= 5 && r0 < 0.008 + 0.004 * (attempt - 5) as f32 && !trunkish {
                // The leaf cards draw these twigs.
                continue;
            }
            let o = if palm {
                TubeOpts { sides_per_m: 22.0, min_sides: 12, max_sides: 28, step: 1, max_seg: sp.palm.as_ref().map(|p| p.ring * 0.5).unwrap_or(0.1), tile }
            } else if trunkish {
                TubeOpts { sides_per_m: 30.0 * f, min_sides: 8, max_sides: if attempt >= 4 { 20 } else { 32 }, step: 1, max_seg: 0.22 / f, tile }
            } else {
                let min_sides = if attempt >= 3 && thin { 3 } else if s.level >= 3 { 4 } else { 5 };
                let step = 1 + attempt / 2 + if thin && attempt >= 2 { 1 } else { 0 };
                TubeOpts { sides_per_m: 36.0 * f, min_sides, max_sides: 16, step, max_seg: 0.45 / f, tile }
            };
            let sh = if s.kind == StemKind::Trunk && s.parent.is_none() { Some(shape) } else { None };
            let w = &winds[i];
            mesh::tube(&mut mb, s, &o, sh, &|t, _| w(t));
        }
        if mb.triangles() <= 52_000 || attempt == 7 {
            out[0] = mb;
            break;
        }
        f *= 0.72;
    }
    // LOD 1: thinner stems dropped until it lands just above the foliage
    // LOD 1, so the bark switches to its far level first: the crown at
    // LOD 1 hides the missing limbs, while limbs never poke through the
    // billboard.
    let lo = leaf_mid.max(800);
    let hi = ((lo as f32 * 1.3) as usize).min(7800usize.saturating_sub(leaf_mid)).max(lo.min(2000));
    let build1 = |cut: f32, sides: f32| {
        let mut mb = MeshBuilder::default();
        for (i, s) in sk.stems.iter().enumerate() {
            if s.radius.first().copied().unwrap_or(0.0) < cut || (s.kind == StemKind::Aerial && s.radius[0] < 0.04) {
                continue;
            }
            let o = TubeOpts { sides_per_m: sides, min_sides: 3, max_sides: if palm { 12 } else { 10 }, step: if palm { 4 } else { 2 }, max_seg: if palm { 0.6 } else { 1.2 }, tile };
            let sh = if s.kind == StemKind::Trunk && s.parent.is_none() { Some(shape) } else { None };
            let w = &winds[i];
            mesh::tube(&mut mb, s, &o, sh, &|t, _| w(t));
        }
        mb
    };
    let (mut cut, mut sides) = (0.012f32, 9.0f32);
    for _ in 0..16 {
        let mb = build1(cut, sides);
        let n = mb.triangles();
        if n > hi && cut < 0.6 {
            cut *= 1.35;
            continue;
        }
        if n < lo && sides < 30.0 && cut <= 0.012 {
            sides *= 1.4;
            continue;
        }
        out[1] = mb;
        break;
    }
    if out[1].pos.is_empty() {
        out[1] = build1(cut, sides);
    }
    // LOD 2: the trunk below the crown (the billboard shows the rest).
    let crown_base = crown_bottom;
    let mut mb = MeshBuilder::default();
    for (i, s) in sk.stems.iter().enumerate() {
        if s.kind != StemKind::Trunk {
            continue;
        }
        let mut st = s.clone();
        if let Some(k) = st.pts.iter().position(|p| p.y > crown_base) {
            st.pts.truncate((k + 1).max(2));
            st.radius.truncate(st.pts.len());
        }
        if s.parent.is_some() && st.pts[0].y > crown_base {
            continue;
        }
        let o = TubeOpts { sides_per_m: 5.0, min_sides: 4, max_sides: 6, step: if palm { 6 } else { 3 }, max_seg: 4.0, tile };
        let w = &winds[i];
        mesh::tube(&mut mb, &st, &o, None, &|t, _| w(t));
    }
    out[2] = mb;
    out
}

/// Ready-made shared textures (for tools that want them without a tree).
pub fn species_textures(sp: &Species, season: Season) -> (TextureSet, Vec<TextureData>) {
    let b = bark_textures(sp);
    let st = FoliageStyle { sp, season, health: 1.0, pruning: Pruning::Natural };
    let f = foliage_textures(&st);
    ((*b).clone(), vec![f.tex.albedo.clone(), f.tex.normal.clone(), f.tex.orm.clone()])
}

#[cfg(test)]
mod tests;
