//! The hand-modelled Valencia landmarks on the island.
//!
//! `data/maps/isla/landmarks.toml` (written by `tools/isla/isla_landmarks.py`
//! from a fit of each real footprint to the island's) says where every
//! model of `data/landmarks/valencia` stands, how it is turned and which
//! island buildings it replaces. Here:
//!
//! * the ground under each landmark is levelled to its street level (the
//!   models have a flat base), blending back into the terrain around it;
//! * the shells of the replaced buildings are left out;
//! * each model is drawn with its own distance LODs near the camera and as
//!   a far stand-in of a few thousand triangles beyond
//!   [`LANDMARK_DETAIL`] HLOD distances; the game takes the full meshes off
//!   the GPU while they are far and reads them back from the cache
//!   ([`WorldDescription::stream_groups`]);
//! * its collision mesh is kept for the game to add around the player.
//!
//! Building 63 landmarks takes minutes, so the built models are kept in
//! the asset cache (see `landmark_models.rs`).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use forja_math::{Iso, Mat4, Quat, Vec2, Vec3};
use forja_physics::TriMesh;

use super::data;
use super::ground::{CELL, Ground, SEA};
use crate::{WorldDescription, WorldError};

#[path = "landmark_models.rs"]
mod models;
use models::{load_models, read_meshes};

/// Landmarks keep their full models within this many HLOD distances.
pub const LANDMARK_DETAIL: f32 = 2.4;
/// The ground is level this far around a footprint, then eases back into
/// the terrain over [`BLEND`].
const LEVEL: f32 = CELL * 1.5;
const BLEND: f32 = 48.0;
/// The models stand this far above the ground (their floors would
/// otherwise fight with it).
const LIFT: f32 = 0.04;

/// One row of `landmarks.toml`.
#[derive(Clone, Debug)]
pub struct Placement {
    pub slug: String,
    pub name: String,
    /// World `(x, z)` of the landmark's origin.
    pub pos: Vec2,
    /// Degrees counter-clockwise seen from above.
    pub rotation: f32,
    /// Largest footprint vertex error of the fit (m).
    pub residual: f32,
    /// Island buildings the model replaces (`way * 2`, `relation * 2 + 1`).
    pub replaces: Vec<u64>,
    /// The footprint's outer rings in world `(x, z)`.
    pub rings: Vec<Vec<Vec2>>,
}

impl Placement {
    fn rotate(&self, p: Vec2) -> Vec2 {
        // Counter-clockwise seen from above, with z pointing south.
        let (s, c) = self.rotation.to_radians().sin_cos();
        Vec2::new(c * p.x + s * p.y, -s * p.x + c * p.y)
    }

    /// Landmark-local `(x, z)` to world.
    pub fn to_world(&self, p: Vec2) -> Vec2 {
        self.pos + self.rotate(p)
    }

    pub fn bounds(&self) -> (Vec2, Vec2) {
        self.rings.iter().flatten().fold((Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)), |(lo, hi), p| (lo.min(*p), hi.max(*p)))
    }

    /// Distance from `p` to the footprint (0 inside).
    fn distance(&self, p: Vec2) -> f32 {
        let mut best = f32::MAX;
        for r in &self.rings {
            if data::point_in_ring(r, p) {
                return 0.0;
            }
            for i in 0..r.len() {
                let (a, b) = (r[i], r[(i + 1) % r.len()]);
                let ab = b - a;
                let t = ((p - a).dot(ab) / ab.length_squared().max(1e-9)).clamp(0.0, 1.0);
                best = best.min(p.distance(a + ab * t));
            }
        }
        best
    }
}

fn osm_id(s: &str) -> Option<u64> {
    let (kind, id) = s.split_once('/')?;
    let id: u64 = id.parse().ok()?;
    match kind {
        "way" => Some(id * 2),
        "relation" => Some(id * 2 + 1),
        _ => None,
    }
}

/// The landmarks of an island: the table and the models' folder.
#[derive(Clone, Debug, Default)]
pub struct Set {
    /// `data/landmarks/valencia`.
    pub dir: PathBuf,
    pub placements: Vec<Placement>,
}

impl Set {
    /// Reads `table` and the footprints of its landmarks under `dir`. Rows
    /// that cannot be used (no model, broken numbers) are skipped with a
    /// warning: the file is data.
    pub fn load(table: &Path, dir: &Path) -> Result<Set, WorldError> {
        let text = std::fs::read_to_string(table).map_err(|e| WorldError::Io(format!("{}: {e}", table.display())))?;
        let doc: toml::Table = text.parse().map_err(|e| WorldError::Parse(format!("{}: {e}", table.display())))?;
        let rows = doc.get("landmark").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        let num = |v: &toml::Value| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)).map(|f| f as f32).filter(|f| f.is_finite());
        let mut placements = Vec::new();
        for row in &rows {
            let Some(slug) = row.get("slug").and_then(|v| v.as_str()) else { continue };
            // A slug is a folder name, never a path.
            if slug.is_empty() || !slug.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
                log::warn!("landmarks: bad slug {slug:?} skipped");
                continue;
            }
            let pos = row.get("pos").and_then(|v| v.as_array()).filter(|a| a.len() == 2).and_then(|a| Some(Vec2::new(num(&a[0])?, num(&a[1])?)));
            let Some(pos) = pos else {
                log::warn!("landmarks: {slug} has no position; skipped");
                continue;
            };
            let folder = dir.join(slug);
            if !folder.join("landmark.toml").exists() {
                log::warn!("landmarks: {slug} has no model in {}; skipped", dir.display());
                continue;
            }
            let mut p = Placement {
                slug: slug.to_string(),
                name: row.get("name").and_then(|v| v.as_str()).unwrap_or(slug).to_string(),
                pos,
                rotation: row.get("rotation").and_then(num).unwrap_or(0.0),
                residual: row.get("residual").and_then(num).unwrap_or(0.0),
                replaces: row.get("replaces").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|v| v.as_str().and_then(osm_id)).collect()).unwrap_or_default(),
                rings: Vec::new(),
            };
            match forja_city::landmark::Footprint::load(&folder.join("footprint.json")) {
                Ok(fp) => p.rings = fp.outlines.iter().map(|o| o.shape.outer.iter().map(|q| p.to_world(*q)).collect::<Vec<Vec2>>()).filter(|r| r.len() >= 3).collect(),
                Err(e) => log::warn!("landmarks: {slug}: {e}"),
            }
            placements.push(p);
        }
        Ok(Set { dir: dir.to_path_buf(), placements })
    }

    /// Levels the ground under every landmark to its street level and
    /// eases it back into the terrain around; returns each landmark's
    /// level.
    pub fn level_ground(&self, g: &mut Ground) -> Vec<f32> {
        let mut levels = Vec::with_capacity(self.placements.len());
        for p in &self.placements {
            // Street level: the mean ground along the outline (the doors).
            let (mut sum, mut n) = (0.0, 0usize);
            for r in &p.rings {
                for i in 0..r.len() {
                    let (a, b) = (r[i], r[(i + 1) % r.len()]);
                    let k = (a.distance(b) / 4.0).ceil().max(1.0) as usize;
                    for t in 0..k {
                        let q = a.lerp(b, t as f32 / k as f32);
                        sum += g.height(q.x, q.y);
                        n += 1;
                    }
                }
            }
            let level = if n > 0 { sum / n as f32 } else { g.height(p.pos.x, p.pos.y) };
            levels.push(level);
            if p.rings.is_empty() {
                continue;
            }
            let (lo, hi) = p.bounds();
            let reach = LEVEL + BLEND;
            let c0 = ((lo.x - reach - g.x0) / CELL).floor().max(0.0) as usize;
            let r0 = ((lo.y - reach - g.z0) / CELL).floor().max(0.0) as usize;
            let c1 = (((hi.x + reach - g.x0) / CELL).ceil().max(0.0) as usize).min(g.cols - 1);
            let r1 = (((hi.y + reach - g.z0) / CELL).ceil().max(0.0) as usize).min(g.rows - 1);
            for r in r0..=r1 {
                for c in c0..=c1 {
                    let i = r * g.cols + c;
                    // Water and the sea bed stay as they are.
                    if g.h[i] <= SEA + 0.3 {
                        continue;
                    }
                    let d = p.distance(Vec2::new(g.x0 + c as f32 * CELL, g.z0 + r as f32 * CELL));
                    if d >= reach {
                        continue;
                    }
                    let t = ((d - LEVEL) / BLEND).clamp(0.0, 1.0);
                    let w = 1.0 - t * t * (3.0 - 2.0 * t);
                    g.h[i] += (level - g.h[i]) * w;
                }
            }
        }
        levels
    }

    /// OSM ids and footprints whose shells give way to the models.
    pub fn replaced(&self) -> (Vec<u64>, Vec<Vec<Vec2>>) {
        (self.placements.iter().flat_map(|p| p.replaces.iter().copied()).collect(), self.placements.iter().flat_map(|p| p.rings.iter().cloned()).collect())
    }

    /// Adds the models to `w` (full model and far stand-in per landmark,
    /// as far-distance groups after `*group`) and returns what the game
    /// needs of them. `levels` from [`Set::level_ground`].
    pub fn place(&self, levels: &[f32], w: &mut WorldDescription, group: &mut u32) -> Vec<Placed> {
        let models = load_models(&self.dir, &self.placements.iter().map(|p| p.slug.clone()).collect::<Vec<_>>());
        let mut out = Vec::new();
        let (mut tris, mut far_tris, mut col_tris) = (0, 0, 0);
        for ((p, m), &level) in self.placements.iter().zip(models).zip(levels) {
            let Some(m) = m else { continue };
            let pos = Vec3::new(p.pos.x, level + LIFT, p.pos.y);
            let rot = Quat::from_rotation_y(p.rotation.to_radians());
            let iso = Iso::new(pos, rot);
            let mat = Mat4::from_rotation_translation(rot, pos);
            *group += 1;
            tris += m.scene.triangle_count();
            far_tris += m.far.triangle_count();
            col_tris += m.col_tris.len();
            let near = w.placements.len();
            w.lod_tags.push((near, *group, false));
            let scene = w.add_scene(Arc::new(m.scene), mat);
            if !m.far.meshes.is_empty() {
                w.lod_tags.push((w.placements.len(), *group, true));
                w.add_scene(Arc::new(m.far), mat);
            }
            w.lod_scales.push((*group, LANDMARK_DETAIL));
            // The full model can leave the GPU while it is far: its meshes
            // are read back from its cache entry.
            if let Some(key) = m.cache_key {
                w.stream_groups.push(crate::StreamGroup { group: *group, scenes: vec![scene], scale: LANDMARK_DETAIL, source: Arc::new(move || Some(vec![read_meshes(&key)?])) });
            }
            let corners = (0..8).map(|i| mat.transform_point3(Vec3::new(if i & 1 == 0 { m.bounds.0.x } else { m.bounds.1.x }, if i & 2 == 0 { m.bounds.0.y } else { m.bounds.1.y }, if i & 4 == 0 { m.bounds.0.z } else { m.bounds.1.z })));
            let (lo, hi) = corners.fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), q| (lo.min(q), hi.max(q)));
            out.push(Placed { slug: p.slug.clone(), name: p.name.clone(), iso, bounds: (lo, hi), group: *group, placement: near, collider: (!m.col_tris.is_empty()).then(|| Arc::new((m.col_verts, m.col_tris))) });
        }
        w.stats.push(("landmarks".into(), out.len()));
        w.stats.push(("landmark triangles".into(), tris));
        w.stats.push(("landmark far triangles".into(), far_tris));
        w.stats.push(("landmark collision triangles".into(), col_tris));
        out
    }
}

/// A landmark standing in the world.
#[derive(Clone)]
pub struct Placed {
    pub slug: String,
    pub name: String,
    /// Local-to-world transform of the model (its base on the ground).
    pub iso: Iso,
    /// World bounds.
    pub bounds: (Vec3, Vec3),
    /// Its far-distance group.
    pub group: u32,
    /// Index of the full model's placement in the world's placements.
    pub placement: usize,
    /// Collision triangles in the landmark's frame (place the mesh with
    /// [`Placed::iso`]); the game builds the mesh when the player is near.
    pub collider: Option<Arc<(Vec<Vec3>, Vec<[u32; 3]>)>>,
}

impl Placed {
    /// The collision mesh (builds its tree: milliseconds to a tenth of a
    /// second for the largest; call it off the frame).
    pub fn collision_mesh(&self) -> Option<TriMesh> {
        self.collider.as_ref().map(|c| TriMesh::new(c.0.clone(), c.1.clone()))
    }

    /// Horizontal distance from `p` to the landmark's bounds.
    pub fn distance(&self, p: Vec3) -> f32 {
        let q = p.clamp(self.bounds.0, self.bounds.1);
        Vec2::new(p.x - q.x, p.z - q.z).length()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::island::data::{IslandData, PolyKind};

    fn data_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")
    }

    fn set() -> Set {
        Set::load(&data_dir().join("maps/isla/landmarks.toml"), &data_dir().join("landmarks/valencia")).expect("landmarks.toml")
    }

    #[test]
    fn every_model_is_placed_on_its_island_footprint() {
        let s = set();
        let models = forja_city::landmark::find_all(&data_dir().join("landmarks/valencia"));
        assert!(models.len() >= 60, "{} models", models.len());
        assert_eq!(s.placements.len(), models.len(), "run tools/isla/isla_landmarks.py after adding a landmark");
        let d = IslandData::load(&data_dir().join("maps/isla")).expect("island data");
        for p in &s.placements {
            // The fit is a translation: Valencia keeps its real shape.
            assert!(p.residual < 0.05 && p.rotation.abs() < 0.02, "{}: residual {} m, rotation {}", p.slug, p.residual, p.rotation);
            assert!(!p.rings.is_empty() && !p.replaces.is_empty(), "{}", p.slug);
            // Each replaced building is on the island, under the model.
            let (lo, hi) = p.bounds();
            for id in &p.replaces {
                let b = d.polys.iter().find(|q| q.kind == PolyKind::Building && q.osm == *id).unwrap_or_else(|| panic!("{}: building {id} is not on the island", p.slug));
                let c = data::ring_centroid(&b.rings[0]);
                assert!(c.x > lo.x - 30.0 && c.x < hi.x + 30.0 && c.y > lo.y - 30.0 && c.y < hi.y + 30.0, "{}: {id} at {c}", p.slug);
            }
        }
        // The Llotja's main outline is the island's building, vertex by vertex.
        let llotja = s.placements.iter().find(|p| p.slug == "llotja_de_la_seda").expect("llotja");
        let b = d.polys.iter().find(|q| q.osm == 23377876 * 2).expect("way 23377876");
        for v in &llotja.rings[0] {
            let near = b.rings[0].iter().map(|q| q.distance(*v)).fold(f32::MAX, f32::min);
            assert!(near < 0.1, "vertex {v} is {near} m off");
        }
    }

    #[test]
    fn ground_is_level_under_landmarks_and_untouched_far_away() {
        let s = set();
        let d = IslandData::load(&data_dir().join("maps/isla")).expect("island data");
        let mut g = Ground::build(&d);
        let before = g.h.clone();
        let levels = s.level_ground(&mut g);
        let mut worst = 0.0f32;
        for (p, level) in s.placements.iter().zip(&levels) {
            for r in &p.rings {
                for v in r {
                    worst = worst.max((g.height(v.x, v.y) - level).abs());
                }
            }
        }
        // Neighbouring landmarks (the Catedral and the Basílica) share ground.
        assert!(worst < 0.5, "ground {worst} m off a landmark's level");
        let moved = before.iter().zip(&g.h).filter(|(a, b)| a != b).count();
        assert!(moved > 500 && moved < before.len() / 100, "{moved} of {} vertices moved", before.len());
        let step = before.iter().zip(&g.h).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
        assert!(step < 6.0, "the ground moved {step} m");
        println!("levelled {moved} vertices, largest change {step:.2} m, worst outline error {worst:.3} m");
    }

    /// `cargo test --release -p forja_world model_sizes -- --ignored --nocapture`
    #[test]
    #[ignore = "builds every landmark (minutes); prints their sizes"]
    fn model_sizes() {
        let s = set();
        let mut seen = std::collections::HashSet::new();
        let (mut tex_bytes, mut total) = (0usize, [0usize; 5]);
        for p in &s.placements {
            let t = std::time::Instant::now();
            let Some(m) = models::build_model(&s.dir.join(&p.slug)) else { continue };
            let prims: Vec<&forja_assets::MeshData> = m.scene.meshes.iter().flatten().collect();
            let verts: usize = prims.iter().map(|q| q.positions.len()).sum();
            let low: usize = prims.iter().map(|q| q.lods.last().map_or(q.indices.len(), |l| l.len()) / 3).sum();
            for t in m.scene.textures.iter().chain(&m.far.textures) {
                if seen.insert(t.rgba8.as_ptr() as usize) {
                    tex_bytes += t.rgba8.len();
                }
            }
            let row = [m.scene.triangle_count(), verts, low, m.far.triangle_count(), m.col_tris.len()];
            for k in 0..5 {
                total[k] += row[k];
            }
            println!("{:34} {:7} tris {:7} verts, lowest LOD {:7}, far {:5} ({} mats), collider {:6} tris {:6} verts, {} materials, {:.1}s", p.slug, row[0], row[1], row[2], row[3], m.far.materials.len(), row[4], m.col_verts.len(), m.scene.materials.len(), t.elapsed().as_secs_f32());
        }
        println!("total: {} tris, {} verts, lowest LOD {}, far {}, collider {}; {} unique textures, {:.0} MB", total[0], total[1], total[2], total[3], total[4], seen.len(), tex_bytes as f64 / 1048576.0);
    }

    #[test]
    #[ignore = "builds every landmark (minutes); prints their textures"]
    fn texture_sizes() {
        let s = set();
        let mut by_ptr = std::collections::HashSet::new();
        let mut buffers = 0;
        let mut by_hash: std::collections::HashMap<u64, (String, u32, u32, usize)> = std::collections::HashMap::new();
        for p in &s.placements {
            // A library per landmark: the shared one keeps every texture.
            let lib = forja_city::landmark::Library::new(forja_city::landmark::shared_library(&s.dir).kit);
            let Ok(lm) = forja_city::landmark::load_with(&s.dir.join(&p.slug), &lib, Default::default()) else { continue };
            by_ptr.clear();
            for (i, t) in lm.scene.textures.iter().enumerate() {
                buffers += 1;
                if !by_ptr.insert(t.rgba8.as_ptr() as usize) {
                    continue;
                }
                let role = lm.scene.materials.iter().find_map(|m| if m.base_color_tex == Some(i) { Some("color") } else if m.normal_tex == Some(i) { Some("normal") } else if m.metallic_roughness_tex == Some(i) { Some("orm") } else if m.emissive_tex == Some(i) { Some("emissive") } else { None }).unwrap_or("?");
                let e = by_hash.entry(forja_assets::cache::fast_hash(&t.rgba8)).or_insert((format!("{} {role}", t.name), t.width, t.height, 0));
                e.3 += 1;
            }
        }
        let mut kinds: std::collections::BTreeMap<String, (usize, usize, usize)> = Default::default();
        for (name, w, h, n) in by_hash.values() {
            let e = kinds.entry(format!("{name} {w}x{h}")).or_default();
            e.0 += 1;
            e.1 += n;
            e.2 += (*w as usize) * (*h as usize) * 4;
        }
        for (k, (unique, copies, bytes)) in &kinds {
            println!("{k:60} {unique:4} unique {copies:4} buffers {:7.1} MB", *bytes as f64 / 1048576.0);
        }
        println!("{} buffers, {} unique by content, {:.0} MB unique", buffers, by_hash.len(), kinds.values().map(|k| k.2).sum::<usize>() as f64 / 1048576.0);
    }
}
