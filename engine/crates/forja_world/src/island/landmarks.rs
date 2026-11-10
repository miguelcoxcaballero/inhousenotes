//! The hand-modelled Valencia landmarks on the island.
//!
//! `data/maps/isla/landmarks.toml` (written by `tools/isla/isla_landmarks.py`
//! from a fit of each real footprint to the island's) says where every
//! model of `data/landmarks/valencia` stands, how it is turned and which
//! island buildings it replaces. Here:
//!
//! * the ground under each landmark is levelled to its street level (the
//!   models have a flat base), blending back into the terrain around it;
//! * each landmark is a [`Site`] of the island: its buildings get one
//!   shell of their own, which the game replaces with the model
//!   (`sandbox::game::island`) at [`Placed::iso`];
//! * [`Models`] gives the models prepared for an open world (smaller
//!   textures, distance LODs, collision mesh), from the asset cache after
//!   the first time.

use std::path::{Path, PathBuf};

use forja_math::{Iso, Mat4, Quat, Vec2, Vec3};

use super::data;
use super::Site;
use super::ground::{CELL, Ground, SEA};
use crate::WorldError;

#[path = "landmark_models.rs"]
mod models;
pub use models::{Model, Models};

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
        // Street levels first, all from the untouched ground: the mean
        // along each outline (the doors).
        let levels: Vec<f32> = self
            .placements
            .iter()
            .map(|p| {
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
                if n > 0 { sum / n as f32 } else { g.height(p.pos.x, p.pos.y) }
            })
            .collect();
        // Per ground vertex: the pull of the nearest landmarks (neighbours
        // such as the Catedral and the Basílica share the ground between).
        let reach = LEVEL + BLEND;
        let mut pull: std::collections::HashMap<usize, (f32, f32, f32)> = std::collections::HashMap::new();
        for (p, &level) in self.placements.iter().zip(&levels) {
            if p.rings.is_empty() {
                continue;
            }
            let (lo, hi) = p.bounds();
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
                    // Nearer landmarks count far more.
                    let k = w * w / (1.0 + d * d);
                    let e = pull.entry(i).or_insert((0.0, 0.0, 0.0));
                    *e = (e.0.max(w), e.1 + k * level, e.2 + k);
                }
            }
        }
        for (i, (w, sum, k)) in pull {
            if k > 0.0 {
                g.h[i] += (sum / k - g.h[i]) * w;
            }
        }
        levels
    }

    /// A [`Site`] per landmark: its footprints and the island buildings it
    /// replaces.
    pub fn sites(&self) -> Vec<Site> {
        self.placements.iter().map(|p| Site { rings: p.rings.clone(), osm: p.replaces.clone() }).collect()
    }

    /// Where each model stands. `levels` from [`Set::level_ground`];
    /// `first_site`: index of the first landmark's site among the island's.
    pub fn placed(&self, levels: &[f32], first_site: usize) -> Vec<Placed> {
        self.placements
            .iter()
            .zip(levels)
            .enumerate()
            .map(|(i, (p, level))| Placed { slug: p.slug.clone(), name: p.name.clone(), iso: Iso::new(Vec3::new(p.pos.x, level + LIFT, p.pos.y), Quat::from_rotation_y(p.rotation.to_radians())), site: first_site + i, bounds: p.bounds() })
            .collect()
    }
}

/// A landmark standing on the island: the game brings its model (see
/// [`Models`]) as the detail that replaces the shell of its site.
#[derive(Clone, Debug)]
pub struct Placed {
    pub slug: String,
    pub name: String,
    /// Local-to-world transform of the model (its base on the ground).
    pub iso: Iso,
    /// Its site in `Island::sites`.
    pub site: usize,
    /// World `(x, z)` bounds of its footprint.
    pub bounds: (Vec2, Vec2),
}

impl Placed {
    /// Local-to-world matrix of the model.
    pub fn transform(&self) -> Mat4 {
        Mat4::from_rotation_translation(self.iso.rot, self.iso.pos)
    }

    /// Horizontal distance from `p` to the footprint's bounds.
    pub fn distance(&self, p: Vec3) -> f32 {
        let q = Vec2::new(p.x, p.z);
        q.distance(q.clamp(self.bounds.0, self.bounds.1))
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
        // The Llotja's outline lies on the island's building.
        let llotja = s.placements.iter().find(|p| p.slug == "llotja_de_la_seda").expect("llotja");
        let b = d.polys.iter().find(|q| q.osm == 23377876 * 2).expect("way 23377876");
        let ring = &b.rings[0];
        for v in &llotja.rings[0] {
            let near = (0..ring.len())
                .map(|i| {
                    let (a, e) = (ring[i], ring[(i + 1) % ring.len()]);
                    let t = ((*v - a).dot(e - a) / (e - a).length_squared().max(1e-9)).clamp(0.0, 1.0);
                    v.distance(a + (e - a) * t)
                })
                .fold(f32::MAX, f32::min);
            assert!(near < 0.5, "vertex {v} is {near} m off");
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
        assert!(worst < 1.0, "ground {worst} m off a landmark's level");
        let moved = before.iter().zip(&g.h).filter(|(a, b)| a != b).count();
        assert!(moved > 500 && moved < before.len() / 100, "{moved} of {} vertices moved", before.len());
        let step = before.iter().zip(&g.h).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
        assert!(step < 6.0, "the ground moved {step} m");
        println!("levelled {moved} vertices, largest change {step:.2} m, worst outline error {worst:.3} m");
    }
}
