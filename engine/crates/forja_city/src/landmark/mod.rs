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
pub mod scaffold;
pub mod shapes;
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
        let mut mesh: MeshData = p.into_mesh(mi);
        if opts.lods && mesh.triangle_count() > 3000 {
            forja_assets::lod::generate_mesh_lods(&mut mesh, 4, 1500);
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

/// A built landmark, ready to place in a world at its anchor.
pub struct Landmark {
    pub slug: String,
    pub name: String,
    pub dir: PathBuf,
    /// City-frame position of the landmark-local origin (x east, z south).
    pub anchor: Vec2,
    pub scene: SceneData,
    pub collider: Collider,
    pub model: Model,
    pub source: Source,
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
        warnings: model.warnings.clone(),
        slug,
        scene,
        collider,
        model,
        source,
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
