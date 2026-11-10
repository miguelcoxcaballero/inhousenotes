//! The whole Isla Levante as one world: 500 km² of terrain, sea, lagoons,
//! land cover, every road and all 43 000 buildings.
//!
//! * [`data`]: the packed map (`data/maps/isla`).
//! * [`ground`]: one height grid for the island (16 m cells) with a land
//!   cover per triangle; drawn as 512 m tiles with distance levels, collided
//!   as one height field.
//! * [`roads`]: every road as a ribbon on the ground, bridges as decks.
//! * [`shells`]: every building as a cheap textured shell, in 256 m tiles.
//!   The game replaces a tile's shells with detailed facades near the
//!   player and streams the buildings' colliders ([`Island`] keeps what it
//!   needs for that).
//! * [`flora`]: instanced trees by land cover.
//! * [`MapData`]: the island as a map (outline, water, roads by class,
//!   building blocks, places) for minimaps and navigation.
//!
//! The world frame is the frame of the Valencia landmarks: x east, z south,
//! origin at the Plaça de l'Ajuntament, the plain at height 0.

pub mod data;
pub mod flora;
pub mod ground;
pub mod landmarks;
pub mod roads;
pub mod shells;
pub mod tex;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use forja_assets::{MaterialData, MeshData, NodeData, SceneData, TextureData};
use forja_math::{Iso, Mat4, Quat, Vec2, Vec3, Vec4};
use forja_physics::{ColliderDesc, CollisionGroups, HeightField, Shape, TriMesh, layers};
use rayon::prelude::*;

use crate::{SpawnKind, SpawnPoint, WorldDescription, WorldError, WorldSource};
use data::{IslandData, PointKind, PolyKind, RoadClass};
use ground::{CELL, Cover, Ground, SEA, TILE};
use shells::{BTILE, Building};

pub use data::Line;

/// The island world source.
pub struct IslandWorld {
    /// `data/maps/isla`.
    pub dir: PathBuf,
    /// `data/flora/species.toml` (the built-in species without it).
    pub species: Option<PathBuf>,
    /// Clock hour the content is built for (`None`: day).
    pub hour: Option<f32>,
    /// Tree density (1 = full; 0 = none).
    pub trees: f32,
    /// Footprints the caller replaces with its own models (landmarks):
    /// buildings whose centroid lies in one of these rings, or whose OSM id
    /// is listed, get no shell.
    pub replaced: Vec<Vec<Vec2>>,
    pub replaced_osm: Vec<u64>,
    /// The hand-modelled landmarks standing on the island (see
    /// [`landmarks`]): the folder of their models, placed by
    /// `landmarks.toml` in [`IslandWorld::dir`]. `None`: plain shells.
    pub landmarks: Option<PathBuf>,
}

impl IslandWorld {
    pub fn new(data: &Path) -> Self {
        Self { dir: data.join("maps/isla"), species: Some(data.join("flora/species.toml")).filter(|p| p.exists()), hour: None, trees: 1.0, replaced: Vec::new(), replaced_osm: Vec::new(), landmarks: Some(data.join("landmarks/valencia")).filter(|p| p.is_dir() && std::env::var_os("FORJA_NO_LANDMARKS").is_none()) }
    }
}

/// One 256 m tile of buildings.
#[derive(Clone, Debug)]
pub struct BuildingTile {
    pub key: (i32, i32),
    /// Its buildings in [`Island::buildings`].
    pub buildings: std::ops::Range<usize>,
    /// Index of its shell placement in the world's placements.
    pub placement: usize,
    pub bounds: (Vec3, Vec3),
}

/// What the game keeps of the island at run time.
pub struct Island {
    pub data: IslandData,
    pub ground: Ground,
    /// Sorted by tile.
    pub buildings: Vec<Building>,
    pub tiles: Vec<BuildingTile>,
    pub tile_index: HashMap<(i32, i32), usize>,
    pub map: Arc<MapData>,
    /// Night level the shells were built for (0 by day).
    pub night: f32,
    /// Far-distance groups the world uses (`1..=groups`); the game numbers
    /// its own after them.
    pub groups: u32,
    /// The landmarks standing on the island (models, bounds, colliders).
    pub landmarks: Vec<landmarks::Placed>,
}

impl Island {
    /// Collision triangles of a building tile.
    pub fn tile_collider(&self, tile: usize) -> (Vec<Vec3>, Vec<[u32; 3]>) {
        shells::tile_collider(&self.buildings[self.tiles[tile].buildings.clone()], &self.data.polys)
    }

    /// Building tiles whose key is within `radius` tiles of the tile of `p`.
    pub fn tiles_near(&self, p: Vec3, radius: i32) -> Vec<usize> {
        let (cx, cz) = ((p.x / BTILE).floor() as i32, (p.z / BTILE).floor() as i32);
        let mut out = Vec::new();
        for dz in -radius..=radius {
            for dx in -radius..=radius {
                if let Some(&i) = self.tile_index.get(&(cx + dx, cz + dz)) {
                    out.push(i);
                }
            }
        }
        out
    }
}

/// A filled area of the map.
#[derive(Clone, Debug)]
pub struct MapArea {
    pub kind: PolyKind,
    pub name: String,
    /// Outer ring first, then holes; `(x, z)` world metres.
    pub rings: Vec<Vec<Vec2>>,
    pub min: Vec2,
    pub max: Vec2,
}

#[derive(Clone, Debug)]
pub struct MapRoad {
    pub class: RoadClass,
    /// Paved width (m).
    pub width: f32,
    pub name: String,
    pub bridge: bool,
    pub tunnel: bool,
    pub oneway: bool,
    pub points: Vec<Vec2>,
    pub min: Vec2,
    pub max: Vec2,
}

#[derive(Clone, Debug)]
pub struct MapBuilding {
    /// Outer ring `(x, z)`.
    pub ring: Vec<Vec2>,
    pub height: f32,
    pub min: Vec2,
    pub max: Vec2,
}

#[derive(Clone, Debug)]
pub struct MapPlace {
    pub kind: PointKind,
    pub name: String,
    pub category: String,
    pub pos: Vec2,
}

/// The island as a map, in world metres (`x` east, `z` south as the
/// second coordinate of every `Vec2`): what a minimap or a route planner
/// needs, with a grid to query what is near a point.
#[derive(Clone, Debug, Default)]
pub struct MapData {
    /// Bounding box of the land.
    pub min: Vec2,
    pub max: Vec2,
    /// Coastline rings of the island.
    pub outline: Vec<Vec<Vec2>>,
    /// Sea level and the height of the plain (the map is drawn flat).
    pub sea_level: f32,
    /// Lakes, lagoons and rivers.
    pub water: Vec<MapArea>,
    /// Land cover areas (parks, woods, fields, beaches, the airfield...).
    pub areas: Vec<MapArea>,
    /// Every road, path and railway, sorted by class (motorways first).
    pub roads: Vec<MapRoad>,
    pub buildings: Vec<MapBuilding>,
    /// Landmarks, stations and other named places.
    pub places: Vec<MapPlace>,
    /// Where the player starts.
    pub start: Vec2,
    /// Query grid: `cell`-metre cells from `min`, `cols` per row; road and
    /// building indices per cell.
    pub cell: f32,
    pub cols: usize,
    pub rows: usize,
    road_cells: Vec<Vec<u32>>,
    building_cells: Vec<Vec<u32>>,
}

impl MapData {
    fn cell_range(&self, lo: Vec2, hi: Vec2) -> (usize, usize, usize, usize) {
        let f = |v: f32, n: usize| ((v / self.cell).floor().max(0.0) as usize).min(n.saturating_sub(1));
        (f(lo.x - self.min.x, self.cols), f(lo.y - self.min.y, self.rows), f(hi.x - self.min.x, self.cols), f(hi.y - self.min.y, self.rows))
    }

    fn gather(&self, cells: &[Vec<u32>], lo: Vec2, hi: Vec2) -> Vec<u32> {
        if self.cols == 0 || self.rows == 0 {
            return Vec::new();
        }
        let (c0, r0, c1, r1) = self.cell_range(lo, hi);
        let mut out: Vec<u32> = Vec::new();
        for r in r0..=r1 {
            for c in c0..=c1 {
                out.extend_from_slice(&cells[r * self.cols + c]);
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Indices into [`MapData::roads`] of the roads touching a box
    /// (ascending: main roads first).
    pub fn roads_in(&self, lo: Vec2, hi: Vec2) -> Vec<u32> {
        let mut v = self.gather(&self.road_cells, lo, hi);
        v.retain(|&i| {
            let r = &self.roads[i as usize];
            r.max.x >= lo.x && r.min.x <= hi.x && r.max.y >= lo.y && r.min.y <= hi.y
        });
        v
    }

    /// Indices into [`MapData::buildings`] of the buildings touching a box.
    pub fn buildings_in(&self, lo: Vec2, hi: Vec2) -> Vec<u32> {
        let mut v = self.gather(&self.building_cells, lo, hi);
        v.retain(|&i| {
            let b = &self.buildings[i as usize];
            b.max.x >= lo.x && b.min.x <= hi.x && b.max.y >= lo.y && b.min.y <= hi.y
        });
        v
    }

    fn build(d: &IslandData, removed: &[bool], start: Vec2) -> MapData {
        let bbox = |pts: &[Vec2]| pts.iter().fold((Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
        let mut m = MapData { sea_level: SEA, start, cell: 128.0, ..Default::default() };
        let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
        for (i, p) in d.polys.iter().enumerate() {
            let (a, b) = bbox(&p.rings[0]);
            match p.kind {
                PolyKind::Coast => {
                    lo = lo.min(a);
                    hi = hi.max(b);
                    m.outline.extend(p.rings.iter().cloned());
                }
                PolyKind::Building => {
                    if !removed[i] {
                        m.buildings.push(MapBuilding { ring: p.rings[0].clone(), height: p.height, min: a, max: b });
                    }
                }
                k => {
                    let area = MapArea { kind: k, name: d.name(p.name).to_string(), rings: p.rings.clone(), min: a, max: b };
                    if k.is_water() { m.water.push(area) } else { m.areas.push(area) }
                }
            }
        }
        if lo.x > hi.x {
            (lo, hi) = (Vec2::new(d.height.extent[0], d.height.extent[1]), Vec2::new(d.height.extent[2], d.height.extent[3]));
        }
        (m.min, m.max) = (lo, hi);
        for l in &d.lines {
            let (a, b) = bbox(&l.pts);
            m.roads.push(MapRoad { class: l.class, width: l.width, name: d.name(l.name).to_string(), bridge: l.bridge(), tunnel: l.tunnel(), oneway: l.oneway(), points: l.pts.clone(), min: a, max: b });
        }
        m.roads.sort_by_key(|r| r.class);
        for p in &d.points {
            m.places.push(MapPlace { kind: p.kind, name: d.name(p.name).to_string(), category: d.name(p.category).to_string(), pos: p.pos });
        }
        m.cols = (((hi.x - lo.x) / m.cell).ceil() as usize).max(1);
        m.rows = (((hi.y - lo.y) / m.cell).ceil() as usize).max(1);
        let mut road_cells = vec![Vec::new(); m.cols * m.rows];
        let mut building_cells = vec![Vec::new(); m.cols * m.rows];
        for (i, r) in m.roads.iter().enumerate() {
            for w in r.points.windows(2) {
                let (c0, r0, c1, r1) = m.cell_range(w[0].min(w[1]), w[0].max(w[1]));
                for rr in r0..=r1 {
                    for c in c0..=c1 {
                        let cell: &mut Vec<u32> = &mut road_cells[rr * m.cols + c];
                        if cell.last() != Some(&(i as u32)) {
                            cell.push(i as u32);
                        }
                    }
                }
            }
        }
        for (i, b) in m.buildings.iter().enumerate() {
            let (c0, r0, c1, r1) = m.cell_range(b.min, b.max);
            for rr in r0..=r1 {
                for c in c0..=c1 {
                    building_cells[rr * m.cols + c].push(i as u32);
                }
            }
        }
        m.road_cells = road_cells;
        m.building_cells = building_cells;
        m
    }
}

fn scene_of(name: &str, parts: Vec<(TextureSet, MeshData)>) -> SceneData {
    let mut s = SceneData { name: name.into(), nodes: vec![NodeData { name: name.into(), mesh: Some(0), ..Default::default() }], roots: vec![0], ..Default::default() };
    let mut prims = Vec::with_capacity(parts.len());
    for (set, mut mesh) in parts {
        let base = s.textures.len();
        s.textures.push(set.0.clone());
        let mut mat = set.2.clone();
        mat.base_color_tex = Some(base);
        if let Some(g) = &set.1 {
            s.textures.push(g.clone());
            mat.emissive_tex = Some(base + 1);
        }
        mesh.material = s.materials.len();
        s.materials.push(mat);
        prims.push(mesh);
    }
    s.meshes.push(prims);
    s
}

/// A texture, its optional glow map and the material using them (shared
/// by every tile: the pixel buffers are reference counted).
type TextureSet = Arc<(TextureData, Option<TextureData>, MaterialData)>;

/// The island's shared materials.
pub struct Materials {
    covers: Vec<TextureSet>,
    cover_colors: [[u8; 3]; ground::COVERS],
    surfaces: HashMap<tex::Surface, TextureSet>,
    shell: TextureSet,
}

impl Materials {
    /// `night`: 0 by day, 1 in full night (lit windows on the shells).
    pub fn new(night: f32) -> Self {
        let covers: Vec<TextureSet> = (0..ground::COVERS as u8)
            .map(|k| {
                let (t, m) = tex::cover_material(Cover::from_u8(k));
                Arc::new((t, None, m))
            })
            .collect();
        let mut cover_colors = [[0u8; 3]; ground::COVERS];
        for (k, c) in covers.iter().enumerate() {
            cover_colors[k] = tex::mean_color(&c.0);
        }
        let surfaces = tex::SURFACES
            .iter()
            .map(|s| {
                let (t, m) = tex::surface_material(*s);
                (*s, Arc::new((t, None, m)))
            })
            .collect();
        Materials { covers, cover_colors, surfaces, shell: Arc::new(tex::shell_material(night)) }
    }

    /// The shell scene of a set of buildings (`None` without any).
    pub fn shell_scene(&self, buildings: &[Building], polys: &[data::Poly]) -> Option<SceneData> {
        let mesh = shells::tile_mesh(buildings, polys);
        (!mesh.indices.is_empty()).then(|| scene_of("isla_shells", vec![(self.shell.clone(), mesh)]))
    }
}

fn sea_scene(mats: &Materials, lo: Vec2, hi: Vec2) -> SceneData {
    // One sheet under the whole island and far beyond it.
    let far = 60_000.0;
    let (a, b) = (lo - Vec2::splat(far), hi + Vec2::splat(far));
    let mut m = MeshData::default();
    // A grid, so no triangle is absurdly large.
    let n = 24usize;
    for j in 0..=n {
        for i in 0..=n {
            let p = Vec2::new(a.x + (b.x - a.x) * i as f32 / n as f32, a.y + (b.y - a.y) * j as f32 / n as f32);
            m.positions.push(Vec3::new(p.x, SEA, p.y));
            m.normals.push(Vec3::Y);
            m.uvs.push(p / 24.0);
            m.tangents.push(Vec4::new(1.0, 0.0, 0.0, 1.0));
        }
    }
    for j in 0..n {
        for i in 0..n {
            let v = |i: usize, j: usize| (j * (n + 1) + i) as u32;
            m.indices.extend([v(i, j), v(i, j + 1), v(i + 1, j + 1), v(i, j), v(i + 1, j + 1), v(i + 1, j)]);
        }
    }
    m.aabb = forja_math::Aabb::from_points(m.positions.iter().copied());
    scene_of("isla_sea", vec![(mats.covers[Cover::Water as usize].clone(), m)])
}

impl WorldSource for IslandWorld {
    fn name(&self) -> String {
        "Isla Levante".into()
    }

    fn build(&self) -> Result<WorldDescription, WorldError> {
        let t0 = std::time::Instant::now();
        let lap = |what: &str| log::info!("island: {what} at {:.2}s", t0.elapsed().as_secs_f32());
        forja_assets::progress::stage("Leyendo el mapa de la isla", 0.04);
        let d = IslandData::load(&self.dir)?;
        lap("map read");
        forja_assets::progress::stage("Levantando el relieve", 0.10);
        let mut g = Ground::build(&d);
        // The landmarks: level ground under each, no shells in their place.
        let lm = match &self.landmarks {
            Some(dir) if self.dir.join("landmarks.toml").exists() => landmarks::Set::load(&self.dir.join("landmarks.toml"), dir).unwrap_or_else(|e| {
                log::warn!("landmarks: {e}");
                landmarks::Set::default()
            }),
            _ => landmarks::Set::default(),
        };
        let lm_levels = lm.level_ground(&mut g);
        let g = g;
        lap("ground");
        let night = self.hour.map_or(0.0, |h| forja_city::NightState::at(h).dark);
        let mats = Materials::new(night);
        let mut w = WorldDescription::new("Isla Levante");
        // Terrain tiles: the near meshes (a material per land cover) and
        // the far stand-in (one mesh, the tile's cover map as its texture).
        forja_assets::progress::stage("Modelando el terreno", 0.18);
        let (tx, tz) = g.tiles();
        let keys: Vec<(usize, usize)> = (0..tz).flat_map(|z| (0..tx).map(move |x| (x, z))).collect();
        let far_mat = MaterialData { name: "isla_terrain_far".into(), base_color: Vec4::ONE, base_color_tex: Some(0), roughness: 0.95, specular: 0.6, ..Default::default() };
        let side = (ground::FAR_TEXELS * ground::ATLAS_TILES) as u32;
        let (ax, az) = (tx.div_ceil(ground::ATLAS_TILES), tz.div_ceil(ground::ATLAS_TILES));
        let atlases: Vec<TextureSet> = (0..ax * az)
            .into_par_iter()
            .map(|i| {
                let map = TextureData { name: "isla_cover_map".into(), width: side, height: side, rgba8: Arc::from(g.cover_atlas(i % ax, i / ax, &mats.cover_colors)), srgb: true };
                Arc::new((map, None, far_mat.clone()))
            })
            .collect();
        let terrain: Vec<(SceneData, SceneData)> = keys
            .par_iter()
            .filter_map(|&(x, z)| {
                let far = g.far_mesh(x, z)?;
                let parts = g.tile_meshes(x, z);
                let near = scene_of("isla_terrain", parts.into_iter().map(|(c, m)| (mats.covers[c as usize].clone(), m)).collect());
                let atlas = atlases[z / ground::ATLAS_TILES * ax + x / ground::ATLAS_TILES].clone();
                Some((near, scene_of("isla_terrain_far", vec![(atlas, far)])))
            })
            .collect();
        let terrain_tiles = terrain.len();
        let mut tris = 0;
        let mut group = 0u32;
        for (near, far) in terrain {
            tris += near.triangle_count();
            group += 1;
            w.lod_tags.push((w.placements.len(), group, false));
            w.add_scene(Arc::new(near), Mat4::IDENTITY);
            w.lod_tags.push((w.placements.len(), group, true));
            w.add_scene(Arc::new(far), Mat4::IDENTITY);
            w.lod_scales.push((group, TERRAIN_DETAIL));
        }
        w.stats.push(("terrain tiles".into(), terrain_tiles));
        w.stats.push(("terrain triangles".into(), tris));
        let (lo, hi) = (Vec2::new(g.x0, g.z0), Vec2::new(g.x0 + (g.cols - 1) as f32 * CELL, g.z0 + (g.rows - 1) as f32 * CELL));
        w.add_scene(Arc::new(sea_scene(&mats, lo, hi)), Mat4::IDENTITY);
        lap("terrain");
        // Roads: the main roads always, streets and paths near the camera.
        forja_assets::progress::stage("Trazando las carreteras", 0.30);
        let roads = roads::Roads::build(&d.lines, &d.polys, &g);
        w.stats.push(("road tiles".into(), roads.tiles.len()));
        w.stats.push(("road triangles".into(), roads.triangles));
        let mut road_tiles: Vec<((i32, i32), roads::RoadTile)> = roads.tiles.into_iter().collect();
        road_tiles.sort_by_key(|(k, _)| *k);
        for (_, t) in road_tiles {
            let scene = |name: &str, parts: Vec<(tex::Surface, MeshData)>| Arc::new(scene_of(name, parts.into_iter().map(|(s, m)| (mats.surfaces[&s].clone(), m)).collect()));
            if !t.major.is_empty() {
                w.add_scene(scene("isla_roads", t.major), Mat4::IDENTITY);
            }
            if !t.minor.is_empty() {
                group += 1;
                w.lod_tags.push((w.placements.len(), group, false));
                w.lod_scales.push((group, STREET_DETAIL));
                w.add_scene(scene("isla_streets", t.minor), Mat4::IDENTITY);
            }
        }
        lap("roads");
        // Buildings.
        forja_assets::progress::stage("Levantando los edificios", 0.42);
        let (lm_osm, lm_rings) = lm.replaced();
        let osm: std::collections::HashSet<u64> = self.replaced_osm.iter().copied().chain(lm_osm).collect();
        let removed: Vec<bool> = d
            .polys
            .par_iter()
            .map(|p| {
                if p.kind != PolyKind::Building {
                    return false;
                }
                if p.osm != 0 && osm.contains(&p.osm) {
                    return true;
                }
                let c = data::ring_centroid(&p.rings[0]);
                self.replaced.iter().chain(&lm_rings).any(|r| data::point_in_ring(r, c))
            })
            .collect();
        let mut buildings: Vec<Building> = d.polys.par_iter().enumerate().filter(|(i, p)| p.kind == PolyKind::Building && !removed[*i]).map(|(i, p)| Building::new(i, p, &g)).collect();
        buildings.sort_by_key(|b| (b.tile(), b.poly));
        let mut tiles: Vec<BuildingTile> = Vec::new();
        let mut i = 0;
        while i < buildings.len() {
            let key = buildings[i].tile();
            let start = i;
            while i < buildings.len() && buildings[i].tile() == key {
                i += 1;
            }
            tiles.push(BuildingTile { key, buildings: start..i, placement: 0, bounds: (Vec3::ZERO, Vec3::ZERO) });
        }
        let shell_scenes: Vec<Option<SceneData>> = tiles.par_iter().map(|t| mats.shell_scene(&buildings[t.buildings.clone()], &d.polys)).collect();
        let mut shell_tris = 0;
        let mut kept = Vec::with_capacity(tiles.len());
        for (mut t, s) in tiles.into_iter().zip(shell_scenes) {
            let Some(s) = s else { continue };
            shell_tris += s.triangle_count();
            let b = s.bounds();
            t.bounds = (b.min, b.max);
            t.placement = w.placements.len();
            w.add_scene(Arc::new(s), Mat4::IDENTITY);
            kept.push(t);
        }
        let tiles = kept;
        w.stats.push(("buildings".into(), buildings.len()));
        w.stats.push(("building tiles".into(), tiles.len()));
        w.stats.push(("shell triangles".into(), shell_tris));
        lap("buildings");
        forja_assets::progress::stage("Colocando los monumentos", 0.47);
        let placed = lm.place(&lm_levels, &mut w, &mut group);
        lap("landmarks");
        // Trees.
        if self.trees > 0.0 {
            forja_assets::progress::stage("Plantando los árboles", 0.52);
            let trees = flora::scatter(&d, &g, self.trees);
            lap("trees scattered");
            let lib = forja_flora::SpeciesLibrary::load(self.species.as_deref());
            let props: Vec<(forja_city::layout::PropKind, Mat4)> = trees.iter().map(|t| (if t.palm { forja_city::layout::PropKind::Palm } else { forja_city::layout::PropKind::Tree }, Mat4::from_translation(t.pos))).collect();
            let wanted = trees.iter().enumerate().map(|(i, t)| (i, t.species, t.pruning)).collect();
            let forest = forja_flora::city::plant_species(&props, wanted, &lib, &forja_flora::city::PlantOptions { variants: 2, season: forja_flora::Season::Summer, seed: 0x15_1a });
            let ids: Vec<usize> = forest.models.into_iter().map(|t| w.register_scene(Arc::new(t.scene))).collect();
            for (m, t) in &forest.instances {
                // Sizes vary a little from tree to tree.
                let p = t.w_axis.truncate();
                let k = ((p.x * 12.9898 + p.z * 78.233).sin() * 43_758.547).fract().abs();
                w.place(ids[*m], *t * Mat4::from_scale(Vec3::splat(0.8 + 0.45 * k)));
            }
            w.stats.push(("trees".into(), forest.instances.len()));
            w.stats.push(("tree models".into(), ids.len()));
            lap("trees");
        }
        // Collision: the ground as one height field, the bridges, and a
        // floor far below the sea bed.
        let groups = CollisionGroups::new(layers::WORLD, u32::MAX);
        let hf = HeightField::new(g.rows, g.cols, g.h.clone(), Vec2::splat(CELL));
        w.colliders.push((ColliderDesc::new(Shape::HeightField(Arc::new(hf))).friction(1.0).groups(groups), Iso::from_pos(Vec3::new((lo.x + hi.x) * 0.5, 0.0, (lo.y + hi.y) * 0.5))));
        if !roads.col_tris.is_empty() {
            w.colliders.push((ColliderDesc::new(Shape::TriMesh(Arc::new(TriMesh::new(roads.col_verts, roads.col_tris)))).friction(1.0).groups(groups), Iso::IDENTITY));
        }
        w.colliders.push((ColliderDesc::new(Shape::cuboid(Vec3::new(40_000.0, 1.0, 40_000.0))).translation(Vec3::new(0.0, SEA - 8.0, 0.0)).friction(1.0).groups(groups), Iso::IDENTITY));
        // Spawns: the player on the street nearest to the Plaça de
        // l'Ajuntament, cars parked along the streets around it.
        let start = spawns(&mut w, &d, &g);
        w.traffic = Some(traffic(&d, &g));
        lap("traffic");
        w.extent = (hi - lo) * 0.5;
        w.attribution.push("Isla Levante: map data © OpenStreetMap contributors, ODbL 1.0; barrios: Ajuntament de València, CC BY 4.0".into());
        let map = Arc::new(MapData::build(&d, &removed, start));
        let tile_index = tiles.iter().enumerate().map(|(i, t)| (t.key, i)).collect();
        w.island = Some(Arc::new(Island { data: d, ground: g, buildings, tiles, tile_index, map, night, groups: group, landmarks: placed }));
        log::info!("island: built in {:.2}s ({})", t0.elapsed().as_secs_f32(), w.stats.iter().map(|(k, v)| format!("{v} {k}")).collect::<Vec<_>>().join(", "));
        Ok(w)
    }
}

/// Player, car, bike, weapon and prop spawns; returns the player's spot.
fn spawns(w: &mut WorldDescription, d: &IslandData, g: &Ground) -> Vec2 {
    let near = |p: Vec2| p.length_squared();
    // The player: on a pedestrian street or footway near the origin.
    let mut player = Vec2::ZERO;
    let mut best = f32::MAX;
    for l in d.lines.iter().filter(|l| roads::visible(l) && !l.bridge() && matches!(l.class, RoadClass::Pedestrian | RoadClass::Footway | RoadClass::LivingStreet)) {
        for s in l.pts.windows(2) {
            let m = (s[0] + s[1]) * 0.5;
            if near(m) < best {
                best = near(m);
                player = m;
            }
        }
    }
    let at = |p: Vec2, up: f32| Vec3::new(p.x, g.height(p.x, p.y) + up, p.y);
    w.spawns.push(SpawnPoint { kind: SpawnKind::Player, iso: Iso::from_pos(at(player, 1.1)) });
    // Cars: on the right side of the nearest streets.
    let mut spots: Vec<(f32, Vec2, Vec2)> = Vec::new();
    for l in d.lines.iter().filter(|l| roads::visible(l) && !l.bridge() && matches!(l.class, RoadClass::Residential | RoadClass::Tertiary | RoadClass::Secondary | RoadClass::Primary | RoadClass::Unclassified)) {
        for s in l.pts.windows(2) {
            let len = s[0].distance(s[1]);
            if len < 14.0 {
                continue;
            }
            let dir = (s[1] - s[0]) / len;
            let m = (s[0] + s[1]) * 0.5 + Vec2::new(-dir.y, dir.x) * (l.width * 0.5 - 1.3).max(0.0);
            let dd = m.distance(player);
            if dd > 9.0 && dd < 400.0 {
                spots.push((dd, m, dir));
            }
        }
    }
    spots.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut taken: Vec<Vec2> = Vec::new();
    for (_, m, dir) in spots {
        if taken.iter().any(|t| t.distance(m) < 12.0) {
            continue;
        }
        taken.push(m);
        w.spawns.push(SpawnPoint { kind: SpawnKind::Vehicle, iso: Iso::new(at(m, 0.8), Quat::from_rotation_y(dir.x.atan2(dir.y))) });
        if taken.len() >= 14 {
            break;
        }
    }
    for i in 0..4 {
        w.spawns.push(SpawnPoint { kind: SpawnKind::Weapon, iso: Iso::from_pos(at(player + Vec2::new(-2.0 + i as f32 * 1.3, 2.5), 0.6)) });
    }
    for i in 0..8 {
        w.spawns.push(SpawnPoint { kind: SpawnKind::Prop, iso: Iso::from_pos(at(player + Vec2::new(3.0 + (i % 4) as f32 * 0.9, -3.0), 0.45 + (i / 4) as f32 * 0.85)) });
    }
    player
}

/// The drivable road graph of the island for the traffic AI: streets
/// between junctions with their lanes, and the ground under them.
fn traffic(d: &IslandData, g: &Ground) -> forja_city::traffic::TrafficInfo {
    use forja_city::traffic::{HeightGrid, TrafficEnd, TrafficInfo, TrafficLane, TrafficStreet};
    let key = |p: Vec2| ((p.x * 4.0).round() as i32, (p.y * 4.0).round() as i32);
    let drivable = |l: &&Line| l.class.drivable() && l.class != RoadClass::Service && l.class != RoadClass::Raceway && l.class != RoadClass::Busway;
    // Junctions: line ends and points shared by several lines.
    let mut uses: HashMap<(i32, i32), u32> = HashMap::new();
    for l in d.lines.iter().filter(drivable) {
        let n = l.pts.len();
        for (i, p) in l.pts.iter().enumerate() {
            *uses.entry(key(*p)).or_default() += if i == 0 || i + 1 == n { 2 } else { 1 };
        }
    }
    let mut info = TrafficInfo::default();
    let mut node_of: HashMap<(i32, i32), usize> = HashMap::new();
    let mut node = |info: &mut TrafficInfo, p: Vec2| -> usize {
        *node_of.entry(key(p)).or_insert_with(|| {
            info.nodes.push(p);
            info.nodes.len() - 1
        })
    };
    for l in d.lines.iter().filter(drivable) {
        let mut start = 0;
        for i in 1..l.pts.len() {
            if i + 1 < l.pts.len() && uses[&key(l.pts[i])] < 2 {
                continue;
            }
            let pts = &l.pts[start..=i];
            start = i;
            let len: f32 = pts.windows(2).map(|s| s[0].distance(s[1])).sum();
            if len < 1.0 {
                continue;
            }
            let (a, b) = (pts[0], pts[pts.len() - 1]);
            let (na, nb) = (node(&mut info, a), node(&mut info, b));
            if na == nb {
                continue;
            }
            let w = l.width;
            let lanes = if l.oneway() {
                let n = ((w / 3.3).floor() as usize).clamp(1, 3);
                let lw = w / n as f32;
                (0..n).map(|k| TrafficLane { offset: w * 0.5 - lw * (k as f32 + 0.5), forward: true, width: lw.min(3.5) }).collect()
            } else {
                // Right-hand traffic: the forward lane is right of a -> b.
                let per = if w >= 13.0 { 2 } else { 1 };
                let lw = w * 0.5 / per as f32;
                let mut v = Vec::new();
                for k in 0..per {
                    let o = lw * (k as f32 + 0.5);
                    v.push(TrafficLane { offset: -o, forward: true, width: lw.min(3.5) });
                    v.push(TrafficLane { offset: o, forward: false, width: lw.min(3.5) });
                }
                v
            };
            let speed = match l.class {
                RoadClass::Motorway => 100.0,
                RoadClass::Trunk => 80.0,
                RoadClass::MotorwayLink | RoadClass::TrunkLink => 50.0,
                RoadClass::Primary | RoadClass::Secondary => 50.0,
                RoadClass::LivingStreet => 20.0,
                _ => 30.0,
            };
            let highway = matches!(l.class, RoadClass::Motorway | RoadClass::Trunk | RoadClass::MotorwayLink | RoadClass::TrunkLink);
            let sidewalk = if highway { 0.0 } else { 1.6 };
            let stop = (len * 0.3).min(4.0);
            info.streets.push(TrafficStreet {
                a,
                b,
                half_width: w * 0.5 + sidewalk,
                road_half: w * 0.5,
                sidewalk,
                median_half: 0.0,
                lanes,
                parking: Vec::new(),
                speed_kmh: speed,
                crosswalk: None,
                stop_line: stop,
                ends: Some([TrafficEnd { crosswalk: None, stop_line: stop }; 2]),
                nodes: Some([na, nb]),
                points: if pts.len() > 2 { pts.to_vec() } else { Vec::new() },
            });
        }
    }
    // Roadway heights: the ground, raised to the deck along bridges.
    let lo = Vec2::new(g.x0, g.z0);
    let mut grid = HeightGrid { origin: lo, cell: CELL, nx: g.cols, nz: g.rows, h: g.h.iter().map(|h| h + 0.1).collect() };
    for l in d.lines.iter().filter(|l| l.bridge() && l.class.drivable()) {
        let (a, b) = (l.pts[0], l.pts[l.pts.len() - 1]);
        let deck = g.height(a.x, a.y).max(g.height(b.x, b.y)) + 0.3;
        for s in l.pts.windows(2) {
            let k = (s[0].distance(s[1]) / (CELL * 0.5)).ceil().max(1.0) as usize;
            for t in 0..=k {
                let p = s[0].lerp(s[1], t as f32 / k as f32);
                let (c, r) = (((p.x - lo.x) / CELL).floor() as isize, ((p.y - lo.y) / CELL).floor() as isize);
                for (dr, dc) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
                    let (rr, cc) = (r + dr, c + dc);
                    if rr >= 0 && cc >= 0 && (rr as usize) < g.rows && (cc as usize) < g.cols {
                        let e = &mut grid.h[rr as usize * g.cols + cc as usize];
                        *e = e.max(deck);
                    }
                }
            }
        }
    }
    info.ground = Some(grid);
    info
}

/// The world origin (the Plaça de l'Ajuntament) in the island's own metres
/// (`x` east, `y` north): `world.x = island.x - ORIGIN.0`, `world.z =
/// -(island.y - ORIGIN.1)`.
pub const ORIGIN: (f32, f32) = (3478.569, 2582.763);
/// Size of a terrain tile (m), for callers that lay things out by tile.
pub const TERRAIN_TILE: f32 = TILE;
/// The terrain keeps its near meshes (textured land cover) within this
/// many HLOD distances, streets and paths within [`STREET_DETAIL`].
pub const TERRAIN_DETAIL: f32 = 2.0;
pub const STREET_DETAIL: f32 = 5.0;
