//! Elevation terrain from Terrarium-encoded PNG tiles
//! (`height = R * 256 + G + B / 256 - 32768` meters).

use std::path::PathBuf;
use std::sync::Arc;

use forja_assets::{MaterialData, MeshData, NodeData, SceneData};
use forja_math::{Iso, Mat4, Vec2, Vec3, Vec4};
use forja_physics::{ColliderDesc, CollisionGroups, HeightField, Shape, layers};

use crate::{SpawnKind, SpawnPoint, WorldDescription, WorldError, WorldSource};

pub struct TerrainWorld {
    /// `tiles[row][col]` PNG paths (north row first).
    pub tiles: Vec<Vec<PathBuf>>,
    /// Ground size of one tile in meters.
    pub tile_meters: f32,
    /// Output grid resolution (vertices per side) for render and collision.
    pub resolution: usize,
}

impl TerrainWorld {
    /// Loads `z_dx_dy.png` tiles written by forja_fetch's `terrarium` source.
    pub fn from_dir(dir: &std::path::Path, z: u32, tiles: u32, lat_deg: f64) -> Self {
        let grid = (0..tiles).map(|dy| (0..tiles).map(|dx| dir.join(format!("{z}_{dx}_{dy}.png"))).collect()).collect();
        // Web Mercator tile width at this latitude.
        let tile_meters = (40_075_016.686 * lat_deg.to_radians().cos() / f64::from(1u32 << z)) as f32;
        Self { tiles: grid, tile_meters, resolution: 257 }
    }

    pub fn decode(rgb: &[u8]) -> f32 {
        rgb[0] as f32 * 256.0 + rgb[1] as f32 + rgb[2] as f32 / 256.0 - 32768.0
    }

    /// Height samples stitched from all tiles, row-major.
    pub fn heights(&self) -> Result<(usize, Vec<f32>), WorldError> {
        let rows = self.tiles.len();
        let mut tile_px = 0;
        let mut imgs = Vec::new();
        for row in &self.tiles {
            let mut r = Vec::new();
            for p in row {
                let img = image::open(p).map_err(|e| WorldError::Io(format!("{}: {e}", p.display())))?.to_rgb8();
                tile_px = img.width() as usize;
                r.push(img);
            }
            imgs.push(r);
        }
        let size = tile_px * rows;
        let mut h = vec![0.0; size * size];
        for (ty, row) in imgs.iter().enumerate() {
            for (tx, img) in row.iter().enumerate() {
                for (x, y, px) in img.enumerate_pixels() {
                    h[(ty * tile_px + y as usize) * size + tx * tile_px + x as usize] = Self::decode(&px.0);
                }
            }
        }
        Ok((size, h))
    }
}

/// Bilinear resample of a square grid.
fn resample(src: &[f32], n: usize, m: usize) -> Vec<f32> {
    let mut out = vec![0.0; m * m];
    for j in 0..m {
        for i in 0..m {
            let fx = i as f32 / (m - 1) as f32 * (n - 1) as f32;
            let fy = j as f32 / (m - 1) as f32 * (n - 1) as f32;
            let (x0, y0) = (fx as usize, fy as usize);
            let (x1, y1) = ((x0 + 1).min(n - 1), (y0 + 1).min(n - 1));
            let (u, v) = (fx - x0 as f32, fy - y0 as f32);
            let a = src[y0 * n + x0] * (1.0 - u) + src[y0 * n + x1] * u;
            let b = src[y1 * n + x0] * (1.0 - u) + src[y1 * n + x1] * u;
            out[j * m + i] = a * (1.0 - v) + b * v;
        }
    }
    out
}

pub fn heightfield_world(name: &str, heights: Vec<f32>, n: usize, size_m: f32) -> WorldDescription {
    let min = heights.iter().copied().fold(f32::MAX, f32::min);
    let heights: Vec<f32> = heights.iter().map(|h| h - min).collect();
    let cell = size_m / (n - 1) as f32;
    let hf = HeightField::new(n, n, heights.clone(), Vec2::splat(cell));
    // Render mesh: one vertex per sample, slope- and height-based colors in
    // two materials (grass, rock) split by slope per triangle.
    let mut grass = MeshData { material: 0, ..Default::default() };
    let mut rock = MeshData { material: 1, ..Default::default() };
    let normal_at = |r: usize, c: usize| {
        let h = |r: usize, c: usize| heights[r.min(n - 1) * n + c.min(n - 1)];
        let dx = h(r, c + 1) - h(r, c.saturating_sub(1));
        let dz = h(r + 1, c) - h(r.saturating_sub(1), c);
        Vec3::new(-dx, 2.0 * cell, -dz).normalize()
    };
    for m in [&mut grass, &mut rock] {
        for r in 0..n {
            for c in 0..n {
                m.positions.push(hf.point(r, c));
                m.normals.push(normal_at(r, c));
                m.uvs.push(Vec2::new(c as f32, r as f32) * cell / 8.0);
            }
        }
    }
    for r in 0..n - 1 {
        for c in 0..n - 1 {
            let i = (r * n + c) as u32;
            let (a, b, cc, d) = (i, i + n as u32, i + n as u32 + 1, i + 1);
            let steep = normal_at(r, c).y < 0.8;
            let m = if steep { &mut rock } else { &mut grass };
            // Same split as HeightField::triangle so render and collision match.
            m.indices.extend([a, b, cc, a, cc, d]);
        }
    }
    let mut prims = Vec::new();
    for mut m in [grass, rock] {
        if !m.indices.is_empty() {
            m.finalize();
            prims.push(m);
        }
    }
    let mat = |name: &str, c: [f32; 3]| MaterialData { name: name.into(), base_color: Vec4::new(c[0], c[1], c[2], 1.0), roughness: 0.95, ..Default::default() };
    let scene = SceneData {
        name: name.into(),
        nodes: vec![NodeData { name: name.into(), mesh: Some(0), ..Default::default() }],
        roots: vec![0],
        meshes: vec![prims],
        materials: vec![mat("terrain_grass", [0.33, 0.40, 0.20]), mat("terrain_rock", [0.45, 0.41, 0.36])],
        ..Default::default()
    };
    let mut w = WorldDescription::new(name);
    let spawn_h = hf.height_at(0.0, 0.0).unwrap_or(0.0);
    w.add_scene(Arc::new(scene), Mat4::IDENTITY);
    w.colliders.push((
        ColliderDesc::new(Shape::HeightField(Arc::new(hf))).friction(1.0).groups(CollisionGroups::new(layers::WORLD, u32::MAX)),
        Iso::IDENTITY,
    ));
    w.spawns.push(SpawnPoint { kind: SpawnKind::Player, iso: Iso::from_pos(Vec3::new(0.0, spawn_h + 2.0, 0.0)) });
    w.spawns.push(SpawnPoint { kind: SpawnKind::Vehicle, iso: Iso::from_pos(Vec3::new(6.0, spawn_h + 3.0, 0.0)) });
    w.extent = Vec2::splat(size_m * 0.5);
    w
}

impl WorldSource for TerrainWorld {
    fn name(&self) -> String {
        "Elevation terrain".into()
    }

    fn build(&self) -> Result<WorldDescription, WorldError> {
        let (n, h) = self.heights()?;
        let m = self.resolution.min(n);
        let hs = resample(&h, n, m);
        let mut w = heightfield_world("terrain", hs, m, self.tile_meters * self.tiles.len() as f32);
        w.attribution.push("Terrain Tiles: Mapzen/Tilezen joerd (SRTM, USGS NED, ETOPO1)".into());
        Ok(w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terrarium_decoding() {
        assert_eq!(TerrainWorld::decode(&[128, 0, 0]), 0.0);
        assert_eq!(TerrainWorld::decode(&[128, 100, 128]), 100.5);
    }

    #[test]
    fn render_matches_collision() {
        let n = 33;
        let h: Vec<f32> = (0..n * n).map(|i| ((i % n) as f32 * 0.3).sin() * 5.0 + (i / n) as f32 * 0.5).collect();
        let w = heightfield_world("t", h, n, 320.0);
        let (_, scene, _) = &w.instances()[0];
        let mut pw = forja_physics::PhysicsWorld::default();
        w.add_to_physics(&mut pw);
        // Every render vertex sits on the collision surface.
        for m in &scene.meshes[0] {
            for p in m.positions.iter().step_by(37) {
                let hit = pw.raycast(&forja_math::Ray::new(*p + Vec3::Y * 50.0, Vec3::NEG_Y), 100.0, Default::default()).unwrap();
                assert!((hit.point.y - p.y).abs() < 1e-2, "{p} vs {}", hit.point.y);
            }
        }
    }
}
