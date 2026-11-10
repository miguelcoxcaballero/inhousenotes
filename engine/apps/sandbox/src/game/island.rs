//! The island at run time.
//!
//! * The buildings' colliders follow the camera (a 3x3 block of 256 m
//!   tiles around it; the ground is one height field that is always there).
//! * Near the camera a tile's building shells give way to detailed
//!   facades from the procedural building generator (`forja_city`): real
//!   openings, balconies, shutters, shop fronts, cornices and roofs. A
//!   background thread builds a tile when the camera comes within ~190 m
//!   of it; its meshes are uploaded a few megabytes per frame and shown as
//!   the detail of the tile's far-distance group (the shell is its
//!   stand-in beyond ~150 m). The farthest tiles are dropped again.
//! * The modelled landmarks of Valencia (`data/landmarks/valencia`)
//!   replace the shells of their footprints as the same thread builds
//!   them, nearest to the player first.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use forja_assets::SceneData;
use forja_city::building::{self, ModuleLib};
use forja_city::geom::Parts;
use forja_city::{CityOut, EdgeKind, Lot, Mat, StyleLibrary};
use forja_core::{App, Stage};
use forja_math::{Mat4, Vec2, Vec3};
use forja_physics::{ColliderDesc, ColliderHandle, CollisionGroups, PhysicsWorld, Shape, TriMesh, layers};
use forja_world::island::Site;
use forja_world::{Island, SpawnKind};

use super::world::GameWorld;
use super::{Paths, RenderItem, RenderScene, StaticLayer};

/// Tiles this far (in tiles) from the camera's tile get colliders; they
/// are dropped one tile further out.
const NEAR: i32 = 1;
/// A tile's facades are built when the camera is this close to it (m) and
/// dropped beyond [`DETAIL_DROP`] when more than [`DETAIL_KEEP`] are held.
const DETAIL_BUILD: f32 = 190.0;
const DETAIL_DROP: f32 = 420.0;
const DETAIL_KEEP: usize = 6;
/// Buildings per detail scene (so each uploads in a frame or two).
const CHUNK: usize = 6;
/// Name of the streamed facade scenes.
pub const DETAIL_SCENE: &str = "isla_detail";

/// Streams the island's building colliders.
pub struct IslandColliders {
    pub island: Arc<Island>,
    loaded: HashMap<usize, ColliderHandle>,
}

impl IslandColliders {
    fn add(&mut self, physics: &mut PhysicsWorld, tile: usize) {
        if self.loaded.contains_key(&tile) {
            return;
        }
        let (verts, tris) = self.island.tile_collider(tile);
        if tris.is_empty() {
            return;
        }
        let desc = ColliderDesc::new(Shape::TriMesh(Arc::new(TriMesh::new(verts, tris)))).friction(0.8).groups(CollisionGroups::new(layers::WORLD, u32::MAX));
        self.loaded.insert(tile, physics.add_collider(desc, None));
    }

    /// Brings the colliders around `p` in (at most `budget` new tiles) and
    /// drops the far ones.
    pub fn update(&mut self, physics: &mut PhysicsWorld, p: Vec3, budget: usize) {
        let mut added = 0;
        for t in self.island.tiles_near(p, NEAR) {
            if added < budget && !self.loaded.contains_key(&t) {
                self.add(physics, t);
                added += 1;
            }
        }
        let keep: std::collections::HashSet<usize> = self.island.tiles_near(p, NEAR + 1).into_iter().collect();
        let far: Vec<usize> = self.loaded.keys().copied().filter(|t| !keep.contains(t)).collect();
        for t in far {
            if let Some(h) = self.loaded.remove(&t) {
                physics.remove_collider(h);
            }
        }
    }

    pub fn loaded(&self) -> usize {
        self.loaded.len()
    }
}

/// The modelled landmarks and the footprints they replace on the island:
/// `(folder, site)` for every folder under `data/landmarks/valencia` with
/// a `landmark.toml` and a footprint. The landmark frame is the world
/// frame (x east, z south from the Plaça de l'Ajuntament).
pub fn landmark_sites(paths: &Paths) -> Vec<(PathBuf, Site)> {
    if std::env::var_os("FORJA_NO_LANDMARKS").is_some() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for dir in forja_city::landmark::find_all(&paths.data.join("landmarks/valencia")) {
        let Ok(text) = std::fs::read_to_string(dir.join("footprint.json")) else { continue };
        let Ok(fp) = forja_city::landmark::footprint::Footprint::parse(&text, "footprint.json") else { continue };
        let mut site = Site::default();
        for o in fp.outlines.iter().chain(&fp.parts) {
            site.rings.push(o.shape.outer.iter().map(|p| *p + fp.anchor).collect());
            // "osm:way/123" or "osm:relation/123".
            if let Some((kind, id)) = o.id.strip_prefix("osm:").and_then(|r| r.split_once('/'))
                && let Ok(n) = id.parse::<u64>()
            {
                site.osm.push(n * 2 + u64::from(kind == "relation"));
            }
        }
        if !site.rings.is_empty() {
            out.push((dir, site));
        }
    }
    out
}

/// The facades of one building tile, from the builder thread: new scenes
/// by builder index (shared window and shop modules appear once, with the
/// first tile that uses them) and where they stand.
struct TileDetail {
    tile: usize,
    scenes: Vec<(usize, Arc<SceneData>)>,
    placements: Vec<(usize, Mat4)>,
    /// Builder indices of the scenes only this tile uses.
    own: Vec<usize>,
    buildings: usize,
    seconds: f32,
}

enum Built {
    Landmark(usize, Arc<SceneData>, Vec2),
    Tile(Box<TileDetail>),
}

#[derive(Clone, Copy, PartialEq)]
enum TileState {
    Requested,
    Ready,
}

/// Detail and landmarks arriving from the builder thread.
pub struct IslandDetail {
    island: Arc<Island>,
    requests: Sender<usize>,
    results: Receiver<Built>,
    tiles: HashMap<usize, TileState>,
    /// Render model of each builder scene.
    models: HashMap<usize, usize>,
    frame: u64,
    pub landmarks: usize,
    pub tiles_built: usize,
}

fn hash(mut x: u64) -> u64 {
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    x
}

/// The lot of a building: its outer ring (counter-clockwise), with the
/// edges that touch a neighbour as party walls.
fn lot_of(island: &Island, b: &forja_world::island::shells::Building) -> Lot {
    let poly = &island.data.polys[b.poly as usize];
    let mut ring: Vec<Vec2> = Vec::with_capacity(poly.rings[0].len());
    for p in &poly.rings[0] {
        if ring.last().is_none_or(|q: &Vec2| q.distance(*p) > 0.08) {
            ring.push(*p);
        }
    }
    if ring.len() > 3 && ring[0].distance(ring[ring.len() - 1]) <= 0.08 {
        ring.pop();
    }
    if forja_world::island::data::ring_area(&ring) < 0.0 {
        ring.reverse();
    }
    let n = ring.len();
    let first = poly.rings[0][0];
    let mut edges = Vec::with_capacity(n);
    for i in 0..n {
        let (a, c) = (ring[i], ring[(i + 1) % n]);
        let d = (c - a).normalize_or(Vec2::X);
        let probe = (a + c) * 0.5 + Vec2::new(d.y, -d.x) * 0.7;
        let touching = island.map.buildings_in(probe - Vec2::splat(0.1), probe + Vec2::splat(0.1)).into_iter().any(|j| {
            let o = &island.map.buildings[j as usize];
            o.ring[0] != first && forja_world::island::data::point_in_ring(&o.ring, probe)
        });
        edges.push(if touching { EdgeKind::Party } else { EdgeKind::Street });
    }
    Lot { levels: Some(b.storeys.clamp(1, 40)), ..Lot::new(ring, edges, b.floor) }
}

/// Builds the detailed facades of a tile.
fn build_tile(island: &Island, tile: usize, mlib: &mut ModuleLib, out: &mut CityOut, styles: &StyleLibrary) -> TileDetail {
    let t0 = std::time::Instant::now();
    let (s0, p0) = (out.scenes.len(), out.placements.len());
    let buildings = &island.buildings[island.tiles[tile].buildings.clone()];
    let valencia: Vec<&forja_city::Style> = styles.of_city("valencia").collect();
    let all: Vec<&forja_city::Style> = if valencia.is_empty() { styles.styles.iter().collect() } else { valencia };
    let mut own = Vec::new();
    let mut failed: Vec<forja_world::island::shells::Building> = Vec::new();
    let mut built = 0;
    for chunk in buildings.chunks(CHUNK) {
        let mut parts: Parts<Mat> = Parts::new();
        let first_wall = out.party_walls.len();
        let masses = out.shadow_masses.len();
        for b in chunk.iter().filter(|b| b.site.is_none()) {
            let lot = lot_of(island, b);
            let tall = b.storeys >= 11;
            let fits: Vec<&&forja_city::Style> = all.iter().filter(|s| s.modern == tall).collect();
            let pool: Vec<&forja_city::Style> = if fits.is_empty() { all.clone() } else { fits.into_iter().copied().collect() };
            let Some(style) = pool.get((hash(b.seed as u64) % pool.len().max(1) as u64) as usize) else { continue };
            if lot.check().is_err() || lot.ring.len() > 120 {
                failed.push(b.clone());
                continue;
            }
            // A footprint is data: one the generator cannot take keeps its shell.
            let mut own_parts: Parts<Mat> = Parts::new();
            let (walls, placed, lights) = (out.party_walls.len(), out.placements.len(), out.city_lights.len());
            let ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| building::build_into(mlib, out, &mut own_parts, &lot, style, hash(b.seed as u64 ^ 0x15_1a)))).is_ok();
            if ok {
                parts.append(&own_parts, &Mat4::IDENTITY);
                built += 1;
            } else {
                out.party_walls.truncate(walls);
                out.placements.truncate(placed);
                out.city_lights.truncate(lights);
                failed.push(b.clone());
            }
        }
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| building::medianeras(mlib, out, &mut parts, first_wall)));
        if parts.tri_count() > 0 {
            let mut scene = forja_city::kit::assemble_roles(mlib.kit, DETAIL_SCENE, parts, None, &forja_city::kit::block_shadow_role);
            scene.name = DETAIL_SCENE.into();
            let si = out.add_scene(scene);
            out.placements.push((si, Mat4::IDENTITY));
            own.push(si);
        }
        if let Some(mut p) = building::proxy_scene(mlib.kit, &out.shadow_masses[masses..]) {
            p.name = DETAIL_SCENE.into();
            let pi = out.add_scene(p);
            out.placements.push((pi, Mat4::IDENTITY));
            own.push(pi);
        }
    }
    if let Some(mut s) = island.materials.shell_scene(&failed, &island.data.polys) {
        s.name = DETAIL_SCENE.into();
        let si = out.add_scene(s);
        out.placements.push((si, Mat4::IDENTITY));
        own.push(si);
    }
    let scenes: Vec<(usize, Arc<SceneData>)> = (s0..out.scenes.len()).map(|i| (i, out.scenes[i].clone())).collect();
    let placements = out.placements[p0..].to_vec();
    // The builder keeps the shared modules (it reads their bounds) but
    // not the tile's own meshes, nor the per-tile records.
    for &i in &own {
        out.scenes[i] = Arc::new(SceneData::default());
    }
    for i in s0..out.scenes.len() {
        if !own.contains(&i) {
            // Bounds, nodes and materials stay; the meshes go with the game's copy.
            out.scenes[i] = Arc::new(super::world::strip(&out.scenes[i]).0);
        }
    }
    out.placements.clear();
    out.shadow_masses.clear();
    out.party_walls.clear();
    out.city_lights.clear();
    out.lights.clear();
    out.frontages.clear();
    TileDetail { tile, scenes, placements, own, buildings: built, seconds: t0.elapsed().as_secs_f32() }
}

fn start_builder(app: &mut App, island: &Arc<Island>, spawn: Vec3) {
    let paths = app.world.res::<Paths>().clone();
    let mut landmarks: Vec<(usize, PathBuf, f32)> = landmark_sites(&paths)
        .into_iter()
        .enumerate()
        .filter(|(k, _)| island.sites.get(*k).is_some_and(Option::is_some))
        .map(|(k, (dir, site))| {
            let c = site.rings[0].iter().copied().sum::<Vec2>() / site.rings[0].len() as f32;
            (k, dir, c.distance(Vec2::new(spawn.x, spawn.z)))
        })
        .collect();
    // Farthest first: the list is popped from its end.
    landmarks.sort_by(|a, b| b.2.total_cmp(&a.2));
    let detail = std::env::var_os("FORJA_NO_DETAIL").is_none();
    let (req_tx, req_rx) = channel::<usize>();
    let (res_tx, res_rx) = channel::<Built>();
    let isl = island.clone();
    let spawned = std::thread::Builder::new().name("island builder".into()).stack_size(16 << 20).spawn(move || {
        crate::threads::lower_priority();
        let t = std::time::Instant::now();
        // The kit (scanned materials, 10-20 s the first time) is shared
        // by the facades and the landmarks.
        let lib = forja_city::landmark::shared_library(&paths.data);
        let kit = lib.kit;
        let styles = StyleLibrary::load(Some(&paths.data.join("city/styles.toml")));
        log::info!("island builder: kit ready in {:.1?}", t.elapsed());
        let mut mlib = ModuleLib::new(kit);
        let mut out = CityOut::default();
        loop {
            // Facades near the camera first, landmarks in between.
            let mut tile = req_rx.try_recv().ok();
            if tile.is_none() && landmarks.is_empty() {
                tile = req_rx.recv().ok();
                if tile.is_none() {
                    return;
                }
            }
            if let Some(tile) = tile {
                let d = build_tile(&isl, tile, &mut mlib, &mut out, &styles);
                log::info!("island builder: tile {tile}: {} buildings, {} placements, {} scenes in {:.2}s", d.buildings, d.placements.len(), d.scenes.len(), d.seconds);
                if res_tx.send(Built::Tile(Box::new(d))).is_err() {
                    return;
                }
                continue;
            }
            let Some((k, dir, _)) = landmarks.pop() else { continue };
            // A landmark is data: one that fails to build keeps its shell.
            // (A material library of its own: the shared one keeps every
            // texture set it has ever built.)
            match std::panic::catch_unwind(|| forja_city::landmark::load_with(&dir, &forja_city::landmark::Library::new(kit), Default::default())) {
                Ok(Ok(l)) => {
                    if res_tx.send(Built::Landmark(k, Arc::new(l.scene), l.anchor)).is_err() {
                        return;
                    }
                }
                Ok(Err(e)) => log::warn!("landmark {}: {e}", dir.display()),
                Err(_) => log::warn!("landmark {}: its builder panicked", dir.display()),
            }
            if landmarks.is_empty() {
                log::info!("island builder: landmarks done at {:.1?}", t.elapsed());
            }
        }
    });
    if spawned.is_err() {
        return;
    }
    app.insert_resource(IslandDetail { island: island.clone(), requests: req_tx, results: res_rx, tiles: HashMap::new(), models: HashMap::new(), frame: 0, landmarks: 0, tiles_built: 0 });
    app.add_system(Stage::PreUpdate, "island_detail", move |w| {
        let eye = w.res::<RenderScene>().camera.position;
        w.scope::<IslandDetail, _>(|w, st| {
            let rs = w.res_mut::<RenderScene>();
            // One result per frame.
            match st.results.try_recv() {
                Ok(Built::Landmark(k, scene, anchor)) => {
                    if let Some(site) = st.island.sites.get(k).copied().flatten() {
                        let model = rs.add_model(scene);
                        rs.draw_only.insert(model);
                        rs.static_layers.push(StaticLayer { items: vec![RenderItem::new(model, Mat4::from_translation(Vec3::new(anchor.x, site.ground, anchor.y)))], replaces: Some(site.group), ..Default::default() });
                        st.landmarks += 1;
                    }
                }
                Ok(Built::Tile(d)) => {
                    for (i, scene) in &d.scenes {
                        let m = rs.add_model(scene.clone());
                        rs.draw_only.insert(m);
                        st.models.insert(*i, m);
                    }
                    let items: Vec<RenderItem> = d.placements.iter().filter_map(|(s, m)| st.models.get(s).map(|model| RenderItem::new(*model, *m))).collect();
                    let own_models = d.own.iter().filter_map(|s| st.models.remove(s)).collect();
                    let group = st.island.tiles[d.tile].group;
                    rs.static_layers.push(StaticLayer { items, replaces: None, gate: Some(group), key: Some(d.tile as u64), own_models });
                    st.tiles.insert(d.tile, TileState::Ready);
                    st.tiles_built += 1;
                }
                Err(_) => {}
            }
            st.frame += 1;
            if !detail || st.frame % 12 != 0 {
                return;
            }
            let dist = |t: usize| {
                let (lo, hi) = st.island.tiles[t].bounds;
                let d = (lo - eye).max(eye - hi).max(Vec3::ZERO);
                Vec2::new(d.x, d.z).length()
            };
            // Ask for the nearest tile within reach, one at a time.
            if !st.tiles.values().any(|s| *s == TileState::Requested) {
                let want = st.island.tiles_near(eye, 1).into_iter().filter(|t| st.island.tiles[*t].group != 0 && !st.tiles.contains_key(t) && dist(*t) < DETAIL_BUILD).min_by(|a, b| dist(*a).total_cmp(&dist(*b)));
                if let Some(t) = want
                    && st.requests.send(t).is_ok()
                {
                    st.tiles.insert(t, TileState::Requested);
                }
            }
            // Drop the farthest when too many are held.
            let mut ready: Vec<usize> = st.tiles.iter().filter(|(_, s)| **s == TileState::Ready).map(|(t, _)| *t).collect();
            if ready.len() > DETAIL_KEEP {
                ready.sort_by(|a, b| dist(*b).total_cmp(&dist(*a)));
                if dist(ready[0]) > DETAIL_DROP {
                    rs.drop_layers.push(ready[0] as u64);
                    st.tiles.remove(&ready[0]);
                }
            }
        });
    });
}

/// Adds the island's run-time systems when the loaded world is an island.
pub fn install(app: &mut App) {
    let Some(island) = app.world.res::<GameWorld>().desc.island.clone() else { return };
    let spawn = app.world.res::<GameWorld>().desc.spawn(SpawnKind::Player).map_or(Vec3::ZERO, |s| s.pos);
    let mut col = IslandColliders { island, loaded: HashMap::new() };
    col.update(app.world.res_mut::<PhysicsWorld>(), spawn, usize::MAX);
    start_builder(app, &col.island, spawn);
    app.insert_resource(col);
    app.add_system(Stage::PreUpdate, "island_colliders", |w| {
        let p = w.res::<RenderScene>().camera.position;
        w.scope::<IslandColliders, _>(|w, col| col.update(w.res_mut::<PhysicsWorld>(), p, 1));
    });
}
