//! # Landmarks
//!
//! Real buildings described by data: a `landmark.toml` spec per landmark
//! (`data/landmarks/<city>/<slug>/`), next to the real-world
//! `footprint.json` (OSM outlines and parts, Catastro parcels, context) and
//! `sources.toml` (photos). The spec language covers everything from
//! Gothic cathedrals to organic white shells — masses with openings of any
//! arch shape, mouldings, battlements, roofs, domes, towers, columns,
//! buttresses, arcades, lofts, parametric shells, arrays and components —
//! on any plan, so landmarks fit organic street layouts and narrow alleys
//! as they really are. See `data/landmarks/README.md`.
//!
//! ```no_run
//! let lm = forja_city::landmark::load(std::path::Path::new("data/landmarks/valencia/lonja")).unwrap();
//! println!("{} at {:?}: {} triangles", lm.slug, lm.anchor, lm.scene.triangle_count());
//! ```

pub mod build;
pub mod camera;
pub mod details;
pub mod domes;
pub mod earcut;
pub mod facade;
pub mod footprint;
pub mod generic;
pub mod masses;
pub mod materials;
pub mod mesh;
pub mod metrics;
pub mod openings;
pub mod palette;
pub mod parse;
pub mod poly;
pub mod profiles;
pub mod raster;
pub mod resolve;
pub mod roofs;
pub mod runs;
pub mod scaffold;
pub mod shapes;
pub mod skeleton;
pub mod spec;
pub mod supports;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use forja_assets::{MeshData, NodeData, SceneData};
use forja_math::{Mat4, Vec2, Vec3};

pub use build::{ElemStat, Model};
pub use footprint::{Footprint, Photo, Sources};
pub use materials::Library;
pub use parse::{Error, Result};
pub use spec::Spec;

use crate::kit::Kit;

/// Static collision mesh in the landmark-local frame.
#[derive(Clone, Debug, Default)]
pub struct Collider {
    pub verts: Vec<Vec3>,
    pub tris: Vec<[u32; 3]>,
}

impl Collider {
    fn from_tris(t: &[[Vec3; 3]]) -> Self {
        let mut c = Collider { verts: Vec::with_capacity(t.len() * 3), tris: Vec::with_capacity(t.len()) };
        for tri in t {
            let b = c.verts.len() as u32;
            c.verts.extend_from_slice(tri);
            c.tris.push([b, b + 1, b + 2]);
        }
        c
    }
}

/// The parsed files of one landmark folder.
pub struct Source {
    pub dir: PathBuf,
    pub spec_path: PathBuf,
    pub text: String,
    pub spec: Spec,
    pub footprint: Option<Footprint>,
    pub sources: Option<Sources>,
}

impl Source {
    pub fn slug(&self) -> String {
        self.spec.landmark.slug.clone().unwrap_or_else(|| self.dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
    }

    /// City-frame position of the landmark-local origin.
    pub fn anchor(&self) -> Vec2 {
        if let Some(a) = self.spec.landmark.anchor {
            return Vec2::new(a[0], a[1]);
        }
        self.footprint.as_ref().map(|f| f.anchor).unwrap_or(Vec2::ZERO)
    }

    /// Folder of the raw photos and plans.
    pub fn raw_dir(&self) -> PathBuf {
        if let Some(r) = &self.spec.landmark.raw_dir {
            return self.dir.join(r);
        }
        let slug = self.slug();
        // data/landmarks/<city>/<slug> -> assets/downloaded/landmarks/<city>/<slug>
        let city = self.dir.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "valencia".into());
        match engine_root(&self.dir) {
            Some(root) => root.join("assets/downloaded/landmarks").join(city).join(slug),
            None => self.dir.clone(),
        }
    }

    /// Resolves a photo path from a `[[view]]` (raw folder, then the spec folder).
    pub fn photo_path(&self, rel: &str) -> PathBuf {
        let a = self.raw_dir().join(rel);
        if a.exists() {
            return a;
        }
        self.dir.join(rel)
    }
}

/// The engine root (the folder holding `data/` and `assets/`) above `p`.
pub fn engine_root(p: &Path) -> Option<PathBuf> {
    let abs = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let mut cur: Option<&Path> = Some(abs.as_path());
    while let Some(d) = cur {
        if d.join("data").is_dir() && d.join("crates").is_dir() {
            return Some(d.to_path_buf());
        }
        cur = d.parent();
    }
    None
}

/// Reads `landmark.toml`, `footprint.json` and `sources.toml` of a folder.
pub fn read(dir: &Path) -> Result<Source> {
    let spec_path = dir.join("landmark.toml");
    let text = std::fs::read_to_string(&spec_path).map_err(|e| Error::msg(format!("{}: {e}", spec_path.display())))?;
    let origin = spec_path.display().to_string();
    let spec = parse::parse_spec(&text, &origin)?;
    let fp_name = spec.landmark.footprint.clone().unwrap_or_else(|| "footprint.json".into());
    let fp_path = dir.join(&fp_name);
    let footprint = if fp_path.exists() { Some(Footprint::load(&fp_path)?) } else { None };
    let src_path = dir.join("sources.toml");
    let sources = if src_path.exists() { Sources::load(&src_path).ok() } else { None };
    Ok(Source { dir: dir.to_path_buf(), spec_path, text, spec, footprint, sources })
}

/// Builds the geometry of a parsed landmark.
pub fn build_model(src: &Source) -> Result<Model> {
    build::build(&src.spec, src.footprint.as_ref(), &src.text, &src.spec_path.display().to_string())
}

/// Options for scene assembly.
#[derive(Clone, Copy, Debug)]
pub struct AssembleOptions {
    /// Generate simplified LODs for large meshes (world use).
    pub lods: bool,
}

impl Default for AssembleOptions {
    fn default() -> Self {
        AssembleOptions { lods: true }
    }
}

/// Render scene of a model: one mesh per material, textures shared.
pub fn assemble(model: &Model, lib: &Library, name: &str, opts: AssembleOptions) -> SceneData {
    let mut scene = SceneData { name: name.into(), ..Default::default() };
    let mut prims = Vec::new();
    for id in &model.geo.order {
        let Some(src) = model.geo.prims.get(id) else { continue };
        if src.indices.is_empty() {
            continue;
        }
        let entry = model.palette.get(*id);
        let built = lib.material(entry);
        let base = scene.textures.len();
        let mut remap = Vec::with_capacity(built.textures.len());
        for t in &built.textures {
            if let Some(i) = scene.textures.iter().position(|e| std::sync::Arc::ptr_eq(&e.rgba8, &t.rgba8) && e.srgb == t.srgb) {
                remap.push(i);
            } else {
                scene.textures.push(t.clone());
                remap.push(scene.textures.len() - 1);
            }
        }
        let _ = base;
        let mut md = built.data.clone();
        let fix = |i: Option<usize>| i.map(|k| remap[k]);
        md.base_color_tex = fix(md.base_color_tex);
        md.normal_tex = fix(md.normal_tex);
        md.metallic_roughness_tex = fix(md.metallic_roughness_tex);
        md.occlusion_tex = fix(md.occlusion_tex);
        md.emissive_tex = fix(md.emissive_tex);
        scene.materials.push(md);
        let mi = scene.materials.len() - 1;
        let uvt = materials::uv_transform(entry);
        let mut p = src.clone();
        for uv in &mut p.uvs {
            *uv = uvt(*uv);
        }
        if opts.lods {
            weld(&mut p);
        }
        let mut mesh: MeshData = p.into_mesh(mi);
        if opts.lods && mesh.triangle_count() > 3000 {
            lods(&mut mesh, 4);
        }
        prims.push(mesh);
    }
    if !prims.is_empty() {
        scene.meshes.push(prims);
        scene.nodes.push(NodeData { name: name.into(), mesh: Some(0), ..Default::default() });
        scene.roots.push(0);
    }
    scene
}

/// Untextured scene of a model (one flat colour per material, from its
/// tint or kind): the fast "clay" preview, no texture generation.
pub fn assemble_clay(model: &Model, name: &str) -> SceneData {
    let mut scene = SceneData { name: name.into(), ..Default::default() };
    let mut prims = Vec::new();
    for id in &model.geo.order {
        let Some(src) = model.geo.prims.get(id) else { continue };
        if src.indices.is_empty() {
            continue;
        }
        let e = model.palette.get(*id);
        let kind = e.spec.kind.as_str();
        let c = e.spec.color.or(e.spec.tint).map(|c| c.0).unwrap_or(match kind {
            "glass" | "curtain_wall" => [0.2, 0.25, 0.3],
            "interior" | "dark" => [0.03, 0.03, 0.03],
            "roof_tiles" | "clay_tiles" | "terracotta" => [0.62, 0.36, 0.25],
            "slate" | "iron" | "lead" => [0.2, 0.2, 0.22],
            "glazed_tiles" => e.spec.colors.first().map(|c| c.0).unwrap_or([0.15, 0.3, 0.6]),
            "wood" | "door" => [0.36, 0.24, 0.15],
            "granite" | "asphalt" => [0.45, 0.45, 0.45],
            _ => [0.82, 0.8, 0.76],
        });
        let lin = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
        scene.materials.push(forja_assets::MaterialData {
            name: e.name.clone(),
            base_color: forja_math::Vec4::new(lin(c[0]), lin(c[1]), lin(c[2]), 1.0),
            roughness: e.spec.roughness.unwrap_or(0.8),
            metallic: 0.0,
            ..Default::default()
        });
        let mi = scene.materials.len() - 1;
        prims.push(src.clone().into_mesh(mi));
    }
    if !prims.is_empty() {
        scene.meshes.push(prims);
        scene.nodes.push(NodeData { name: name.into(), mesh: Some(0), ..Default::default() });
        scene.roots.push(0);
    }
    scene
}

/// Merges vertices with the same position, normal and UV (primitives
/// emit unshared vertices), so simplification can collapse edges. Hard
/// edges and UV seams stay split.
fn weld(p: &mut crate::geom::Prim) {
    use std::collections::HashMap;
    let q = |v: f32, k: f32| (v * k).round() as i32;
    let mut map: HashMap<[i32; 8], u32> = HashMap::with_capacity(p.positions.len());
    let mut np = crate::geom::Prim::default();
    let mut remap = Vec::with_capacity(p.positions.len());
    for i in 0..p.positions.len() {
        let (a, n, uv) = (p.positions[i], p.normals[i], p.uvs[i]);
        let key = [q(a.x, 1e3), q(a.y, 1e3), q(a.z, 1e3), q(n.x, 50.0), q(n.y, 50.0), q(n.z, 50.0), q(uv.x, 1e3), q(uv.y, 1e3)];
        let id = *map.entry(key).or_insert_with(|| np.vertex(a, n, uv));
        remap.push(id);
    }
    np.indices = p.indices.iter().map(|i| remap[*i as usize]).collect();
    *p = np;
}

/// Distance LODs by edge collapse with locked borders: material borders
/// and hard edges never move, so levels do not crack or merge windows;
/// stops when the mesh resists simplification (no sloppy fallback).
fn lods(mesh: &mut MeshData, max_levels: usize) {
    mesh.lods.clear();
    let bytes: &[u8] = unsafe { std::slice::from_raw_parts(mesh.positions.as_ptr().cast::<u8>(), std::mem::size_of_val(&mesh.positions[..])) };
    let Ok(adapter) = meshopt::VertexDataAdapter::new(bytes, 12, 0) else { return };
    let mut prev = mesh.indices.clone();
    for level in 1..=max_levels {
        let target = (prev.len() / 2) / 3 * 3;
        if target < 900 {
            break;
        }
        let mut err = 0.0;
        let lod = meshopt::simplify(&prev, &adapter, target, 0.005 * level as f32, meshopt::SimplifyOptions::LockBorder, Some(&mut err));
        if lod.is_empty() || lod.len() as f32 > prev.len() as f32 * 0.9 {
            break;
        }
        let lod = meshopt::optimize_vertex_cache(&lod, mesh.positions.len());
        mesh.lods.push(lod.clone());
        prev = lod;
    }
}

/// A built landmark, ready to place in a world at its anchor: the render
/// scene, the collider and a little metadata (the spec and raw geometry
/// are dropped after assembly; use [`read`] and [`build_model`] for them).
pub struct Landmark {
    pub slug: String,
    pub name: String,
    pub dir: PathBuf,
    /// City-frame position of the landmark-local origin (x east, z south).
    pub anchor: Vec2,
    pub scene: SceneData,
    pub collider: Collider,
    /// Landmark-local bounds of the geometry.
    pub bounds: (Vec3, Vec3),
    pub warnings: Vec<String>,
}

impl Landmark {
    /// Local-to-city transform.
    pub fn transform(&self) -> Mat4 {
        Mat4::from_translation(Vec3::new(self.anchor.x, 0.0, self.anchor.y))
    }
}

/// Loads and builds a landmark with an explicit material library.
pub fn load_with(dir: &Path, lib: &Library, opts: AssembleOptions) -> Result<Landmark> {
    let source = read(dir)?;
    let model = build_model(&source)?;
    let slug = source.slug();
    let scene = assemble(&model, lib, &slug, opts);
    let collider = Collider::from_tris(&model.collider);
    Ok(Landmark {
        name: source.spec.landmark.name.clone(),
        anchor: source.anchor(),
        dir: dir.to_path_buf(),
        bounds: mesh::bounds(&model.geo).unwrap_or((Vec3::ZERO, Vec3::ZERO)),
        warnings: model.warnings,
        slug,
        scene,
        collider,
    })
}

static SHARED: OnceLock<Library<'static>> = OnceLock::new();

/// Process-wide material library over the kit's downloaded texture sets
/// (found from `near`, the engine root, or `FORJA_DOWNLOADED`).
pub fn shared_library(near: &Path) -> &'static Library<'static> {
    SHARED.get_or_init(|| {
        let dl = std::env::var_os("FORJA_DOWNLOADED").map(PathBuf::from).or_else(|| engine_root(near).map(|r| r.join("assets/downloaded"))).filter(|p| p.exists());
        let kit: &'static Kit = Box::leak(Box::new(Kit::load(dl.as_deref())));
        Library::new(kit)
    })
}

/// Loads and builds the landmark in `slug_dir` (`.../<slug>/landmark.toml`).
pub fn load(slug_dir: &Path) -> Result<Landmark> {
    load_with(slug_dir, shared_library(slug_dir), AssembleOptions::default())
}

/// Every folder under `dir` (recursively, two levels) with a `landmark.toml`.
pub fn find_all(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![(dir.to_path_buf(), 0)];
    while let Some((d, depth)) = stack.pop() {
        if d.join("landmark.toml").exists() {
            out.push(d.clone());
            continue;
        }
        if depth >= 3 {
            continue;
        }
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten() {
                let p = e.path();
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if p.is_dir() && !name.starts_with('.') {
                    stack.push((p, depth + 1));
                }
            }
        }
    }
    out.sort();
    out
}

/// Loads every landmark below `dir`; failures are returned with their folder.
pub fn load_all(dir: &Path) -> (Vec<Landmark>, Vec<(PathBuf, Error)>) {
    let mut ok = Vec::new();
    let mut bad = Vec::new();
    for d in find_all(dir) {
        match load(&d) {
            Ok(l) => ok.push(l),
            Err(e) => bad.push((d, e)),
        }
    }
    (ok, bad)
}

/// Finds a landmark folder from a slug or a path.
pub fn locate(arg: &str, engine: &Path) -> Option<PathBuf> {
    let p = PathBuf::from(arg);
    if p.join("landmark.toml").exists() || p.join("footprint.json").exists() {
        return Some(p);
    }
    let base = engine.join("data/landmarks");
    for cand in [base.join("valencia").join(arg), base.join(arg), base.join("_samples").join(arg)] {
        if cand.is_dir() {
            return Some(cand);
        }
    }
    None
}
