//! Procedural city from modular kit pieces (Kenney City Kits): a street grid
//! with crossroads, sidewalks, street lights, downtown towers and suburbs.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use forja_math::{Iso, Mat4, Quat, Vec2, Vec3};
use forja_physics::{ColliderDesc, CollisionGroups, Shape, TriMesh, layers};

use crate::{SpawnKind, SpawnPoint, WorldDescription, WorldError, WorldSource};

pub struct KitCity {
    /// `assets/downloaded`.
    pub root: PathBuf,
    /// City blocks per side.
    pub blocks: usize,
    /// Meters per kit tile.
    pub tile: f32,
    pub seed: u64,
}

impl KitCity {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into(), blocks: 5, tile: 12.0, seed: 7 }
    }
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn pick<'a, T>(&mut self, v: &'a [T]) -> &'a T {
        &v[(self.next() % v.len() as u64) as usize]
    }
}

/// Block edge length in tiles (one road tile + lots).
const BLOCK: usize = 4;

impl WorldSource for KitCity {
    fn name(&self) -> String {
        "Kit city".into()
    }

    fn build(&self) -> Result<WorldDescription, WorldError> {
        let mut w = WorldDescription::new("Kit city");
        let mut cache: HashMap<String, usize> = HashMap::new();
        let mut load = |w: &mut WorldDescription, pack: &str, name: &str| -> Result<usize, WorldError> {
            let key = format!("{pack}/{name}");
            if let Some(i) = cache.get(&key) {
                return Ok(*i);
            }
            let path = self.root.join(pack).join(format!("{name}.glb"));
            if !path.exists() {
                return Err(WorldError::Missing(path.display().to_string()));
            }
            let s = forja_assets::load_gltf(&path).map_err(|e| WorldError::Parse(e.to_string()))?;
            let i = w.register_scene(Arc::new(s));
            cache.insert(key, i);
            Ok(i)
        };
        let roads = "kenney_city_roads";
        let straight = load(&mut w, roads, "road-straight")?;
        let cross = load(&mut w, roads, "road-crossroad")?;
        let light = load(&mut w, roads, "light-square")?;
        let downtown: Vec<usize> = ["a", "b", "c", "d", "e"]
            .iter()
            .map(|s| load(&mut w, "kenney_city_commercial", &format!("building-skyscraper-{s}")))
            .collect::<Result<_, _>>()?;
        let commercial: Vec<usize> = "abcdefghijklmn"
            .chars()
            .map(|c| load(&mut w, "kenney_city_commercial", &format!("building-{c}")))
            .collect::<Result<_, _>>()?;
        let suburban: Vec<usize> = "abcdefghijklmnopqrstu"
            .chars()
            .map(|c| load(&mut w, "kenney_city_suburban", &format!("building-type-{c}")))
            .collect::<Result<_, _>>()?;
        let trees: Vec<usize> = ["tree-large", "tree-small"].iter().map(|t| load(&mut w, "kenney_city_suburban", t)).collect::<Result<_, _>>()?;

        let n = self.blocks * BLOCK + 1;
        let t = self.tile;
        let origin = -(n as f32 - 1.0) * 0.5 * t;
        let scale = Mat4::from_scale(Vec3::splat(t));
        let mut rng = Rng(self.seed.max(1));
        let (mut road_verts, mut road_tris) = (Vec::new(), Vec::new());
        let mut place_road = |w: &mut WorldDescription, scene: usize, m: Mat4| {
            w.place(scene, m);
            let (v, tri) = w.scenes[scene].world_triangles();
            let base = road_verts.len() as u32;
            road_verts.extend(v.into_iter().map(|p| m.transform_point3(p)));
            road_tris.extend(tri.into_iter().map(|t| [t[0] + base, t[1] + base, t[2] + base]));
        };
        let center = (n as f32 - 1.0) * 0.5;
        for i in 0..n {
            for j in 0..n {
                let pos = Vec3::new(origin + i as f32 * t, 0.0, origin + j as f32 * t);
                let (road_x, road_z) = (i % BLOCK == 0, j % BLOCK == 0);
                let at = |rot: f32| Mat4::from_rotation_translation(Quat::from_rotation_y(rot), pos) * scale;
                match (road_x, road_z) {
                    (true, true) => place_road(&mut w, cross, at(0.0)),
                    (true, false) => place_road(&mut w, straight, at(0.0)),
                    (false, true) => place_road(&mut w, straight, at(std::f32::consts::FRAC_PI_2)),
                    (false, false) => {
                        // Face the nearest street.
                        let (bi, bj) = (i % BLOCK, j % BLOCK);
                        let rot = if bi == 1 {
                            -std::f32::consts::FRAC_PI_2
                        } else if bi == BLOCK - 1 {
                            std::f32::consts::FRAC_PI_2
                        } else if bj == 1 {
                            std::f32::consts::PI
                        } else {
                            0.0
                        };
                        let d = ((i as f32 - center).powi(2) + (j as f32 - center).powi(2)).sqrt() / center;
                        let interior = bi == BLOCK / 2 && bj == BLOCK / 2;
                        let scene = if interior && rng.next() % 3 == 0 {
                            *rng.pick(&trees)
                        } else if d < 0.35 {
                            *rng.pick(&downtown)
                        } else if d < 0.7 {
                            *rng.pick(&commercial)
                        } else {
                            *rng.pick(&suburban)
                        };
                        let m = at(rot);
                        w.place(scene, m);
                        let b = w.scenes[scene].bounds();
                        if !b.is_empty() && !trees.contains(&scene) {
                            let c = m.transform_point3(b.center());
                            let h = forja_math::Mat3::from_mat4(m) * b.half_extents();
                            w.colliders.push((
                                ColliderDesc::new(Shape::cuboid(h.abs())).groups(CollisionGroups::new(layers::WORLD, u32::MAX)),
                                Iso::from_pos(c),
                            ));
                        }
                    }
                }
                // Street lights on the corners of each block.
                if road_x && road_z && i + 1 < n && j + 1 < n {
                    let p = pos + Vec3::new(t * 0.42, 0.0, t * 0.42);
                    w.place(light, Mat4::from_translation(p) * scale);
                }
                if road_x && !road_z && j % 2 == 0 {
                    w.spawns.push(SpawnPoint { kind: SpawnKind::Vehicle, iso: Iso::new(pos + Vec3::new(t * 0.2, 1.0, 0.0), Quat::IDENTITY) });
                }
            }
        }
        let half = n as f32 * 0.5 * t;
        w.colliders.push((
            ColliderDesc::new(Shape::cuboid(Vec3::new(half + 50.0, 1.0, half + 50.0))).translation(Vec3::new(0.0, -1.0, 0.0)).friction(1.0).groups(CollisionGroups::new(layers::WORLD, u32::MAX)),
            Iso::IDENTITY,
        ));
        w.colliders.push((
            ColliderDesc::new(Shape::TriMesh(Arc::new(TriMesh::new(road_verts, road_tris)))).friction(1.0).groups(CollisionGroups::new(layers::WORLD, u32::MAX)),
            Iso::IDENTITY,
        ));
        let mid = origin + (self.blocks / 2 * BLOCK) as f32 * t;
        w.spawns.insert(0, SpawnPoint { kind: SpawnKind::Player, iso: Iso::from_pos(Vec3::new(mid + t * 0.3, 1.2, mid + t * 1.5)) });
        w.extent = Vec2::splat(half);
        w.attribution.push("City kits by Kenney (CC0)".into());
        w.stats.push(("tiles".into(), n * n));
        Ok(w)
    }
}

