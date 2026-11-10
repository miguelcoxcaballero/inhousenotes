//! The island's detail around the player: the shells of the cells near
//! the camera give way to procedural buildings and dressed streets
//! (`forja_world::island::detail`), built on a worker thread and streamed
//! in and out.
//!
//! * A shell tile (256 m) within [`TILE_IN`] of the camera becomes
//!   *active*: its one shell mesh is hidden and its sixteen cells (64 m)
//!   each show their own shell mesh instead (the same geometry, so nothing
//!   changes on screen).
//! * A cell of an active tile within [`DETAIL_IN`] is built by the worker
//!   (nearest first, one at a time). When its meshes are on the GPU the
//!   cell's shell is hidden and its detail shown in the same frame; beyond
//!   [`DETAIL_OUT`] the shell comes back and the detail's meshes are
//!   released (hysteresis, so a camera at the border never flips).
//! * A built cell keeps its instance list and its render models; its
//!   meshes are written to the session's folder of the disk cache
//!   (`cache/isla_detail/<process>/`) and read back when the camera
//!   returns, instead of being generated again. The folder is capped
//!   ([`CACHE_BYTES`]): the cells used longest ago are forgotten.
//! * Shared modules (windows, balconies, shop fronts, lamps, trees) are
//!   uploaded once and stay; a cell is one layer of static instances in
//!   the renderer (`Renderer::add_static_layer`), culled and batched with
//!   the world's.
//!
//! The main thread only moves prepared vertex data into the renderer's
//! pools, at most [`FILL_BYTES_PER_FRAME`] per frame, and builds the
//! culling grid of one cell's instances.
//!
//! Landmarks placed by hand: list their footprints in [`DetailSkip`] and
//! neither their shells nor their procedural buildings are drawn by the
//! cells.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use forja_assets::SceneData;
use forja_core::{App, Stage};
use forja_math::{Mat4, Quat, Vec2, Vec3, Vec4};
use forja_render::{DrawInstance, Light, MeshUpload, ModelHandle, Renderer};
use forja_world::island::detail::{self, CellStats, DetailIndex, DetailKit};
use forja_world::island::shells::BTILE;
use forja_world::{Island, SpawnKind};

use super::world::GameWorld;
use super::{Paths, RenderItem, RenderScene};

/// A cell is detailed within this distance of the camera (m, to the
/// cell's square) and returns to its shell beyond [`DETAIL_OUT`].
pub const DETAIL_IN: f32 = 150.0;
pub const DETAIL_OUT: f32 = 190.0;
/// A shell tile splits into its cells within this distance and becomes
/// one mesh again beyond [`TILE_OUT`].
pub const TILE_IN: f32 = DETAIL_IN + 70.0;
pub const TILE_OUT: f32 = DETAIL_OUT + 110.0;
/// Vertex and index data copied into the renderer per frame.
pub const FILL_BYTES_PER_FRAME: u64 = 10 << 20;
/// Size of the session's cell cache on disk.
pub const CACHE_BYTES: u64 = 256 << 20;
/// Built cells remembered while they are not shown.
const KEPT_CELLS: usize = 160;

/// A command for the renderer's static layers, applied by the frame
/// extraction in order (see [`sync`]).
pub enum LayerCmd {
    /// Replaces the (empty) meshes of `model` with prepared vertex data.
    Fill { model: usize, data: Arc<MeshUpload> },
    /// Frees the meshes of `model` (its slot stays for a later `Fill`).
    Release { model: usize },
    Add { id: u64, items: Vec<RenderItem>, visible: bool },
    Show { id: u64, visible: bool },
    Remove { id: u64 },
    /// Hides a far-distance group of the world's static set.
    HideGroup { group: u32, hidden: bool },
}

/// Renderer side of the layers: ids to renderer layers, what is waiting.
#[derive(Default)]
pub struct LayerSync {
    layers: HashMap<u64, u32>,
    queue: std::collections::VecDeque<LayerCmd>,
    /// Longest [`sync`] so far (ms) and the bytes it has filled.
    pub max_ms: f32,
    pub filled_bytes: u64,
    pub fills: usize,
}

/// Applies the scene's layer commands: called once per frame after the
/// frame's models are uploaded. Fills are budgeted; everything after a
/// fill that has to wait waits with it, so a layer is never shown before
/// its meshes are in.
pub fn sync(state: &mut LayerSync, renderer: &mut Renderer, handles: &[ModelHandle], rs: &mut RenderScene) {
    // The detail radius follows the graphics preset's facade distance
    // (Low 90 m, Medium 120 m, High 150 m, Ultra 190 m).
    rs.detail_radius = Some(if renderer.settings.hlod_distance <= 0.0 { DETAIL_IN } else { (renderer.settings.hlod_distance * 0.6).clamp(90.0, 190.0) });
    // CPU copies of models that are on the GPU now.
    let mut i = 0;
    let mut freed = 0;
    while i < rs.strip_after_upload.len() {
        let m = rs.strip_after_upload[i];
        if m < handles.len() {
            let (s, b) = super::world::strip(&rs.models[m]);
            freed += b + rs.models[m].textures.iter().map(|t| t.rgba8.len()).sum::<usize>();
            rs.models[m] = Arc::new(s);
            rs.strip_after_upload.swap_remove(i);
        } else {
            i += 1;
        }
    }
    // The kit's pixels (hundreds of MB) leave with its first scene.
    if freed > 64 << 20 {
        renderer.prune_texture_cache();
        // The pixels were allocated by the pool threads that decoded the
        // kit: each returns its own pages.
        rayon::broadcast(|_| crate::perf::trim_heap());
        if let Some(pool) = forja_assets::threads::frame_pool() {
            pool.broadcast(|_| crate::perf::trim_heap());
        }
        crate::perf::trim_heap();
        log::info!("island detail: kit pixels released ({:.0} MB), resident {:.0} MB", freed as f64 / 1048576.0, crate::perf::resident_mb());
    }
    state.queue.extend(rs.layer_cmds.drain(..));
    if state.queue.is_empty() {
        return;
    }
    let t = std::time::Instant::now();
    let mut budget = FILL_BYTES_PER_FRAME as i64;
    while let Some(cmd) = state.queue.front() {
        match cmd {
            LayerCmd::Fill { model, data } => {
                let Some(&h) = handles.get(*model) else { break };
                if budget <= 0 {
                    break;
                }
                budget -= data.bytes() as i64;
                renderer.release_model(h);
                state.filled_bytes += renderer.reload_prepared(h, data);
                state.fills += 1;
            }
            LayerCmd::Release { model } => {
                if let Some(&h) = handles.get(*model) {
                    renderer.release_model(h);
                }
            }
            LayerCmd::Add { id, items, visible } => {
                if items.iter().any(|it| it.model >= handles.len()) {
                    break;
                }
                let inst: Vec<DrawInstance> = items.iter().map(|it| DrawInstance { model: handles[it.model], transform: it.transform, nodes: None, joints: None, tint: it.tint, cast_shadows: true, motion_id: 0 }).collect();
                if let Some(old) = state.layers.insert(*id, renderer.add_static_layer(&inst, *visible)) {
                    renderer.remove_static_layer(old);
                }
            }
            LayerCmd::Show { id, visible } => {
                if let Some(&l) = state.layers.get(id) {
                    renderer.set_static_layer_visible(l, *visible);
                }
            }
            LayerCmd::Remove { id } => {
                if let Some(l) = state.layers.remove(id) {
                    renderer.remove_static_layer(l);
                }
            }
            LayerCmd::HideGroup { group, hidden } => renderer.set_static_group_hidden(*group, *hidden),
        }
        state.queue.pop_front();
    }
    state.max_ms = state.max_ms.max(t.elapsed().as_secs_f32() * 1000.0);
}

/// Footprints the cells leave out: hand-modelled landmarks. Island
/// polygon indices (`forja_world::island::shells::Building::poly`), OSM
/// ids (`way * 2`, `relation * 2 + 1`, as in the island's data) or rings
/// around their centres (world `(x, z)`). Insert or change it at any
/// time; cells built afterwards follow it.
#[derive(Clone, Default)]
pub struct DetailSkip {
    pub polys: HashSet<u32>,
    pub osm: HashSet<u64>,
    pub rings: Vec<Vec<Vec2>>,
    /// Bump it ([`DetailSkip::touch`]) after changing a list without
    /// changing its length; the cells are then built again.
    pub version: u32,
}

impl DetailSkip {
    pub fn touch(&mut self) {
        self.version += 1;
    }

    fn resolve(&self, island: &Island) -> HashSet<u32> {
        let mut out = self.polys.clone();
        if !self.osm.is_empty() || !self.rings.is_empty() {
            for b in &island.buildings {
                let p = &island.data.polys[b.poly as usize];
                if (p.osm != 0 && self.osm.contains(&p.osm)) || self.rings.iter().any(|r| forja_world::island::data::point_in_ring(r, b.centre)) {
                    out.insert(b.poly);
                }
            }
        }
        out
    }
}

/// A scene without its vertex data; materials, textures and bounds stay,
/// so it uploads as the scene's slots and the data follows as a `Fill`.
fn hollow(s: &SceneData) -> SceneData {
    let meshes = s.meshes.iter().map(|prims| prims.iter().map(|m| forja_assets::MeshData { material: m.material, aabb: m.aabb, lod_sync: m.lod_sync, shadow: m.shadow, ..Default::default() }).collect()).collect();
    SceneData { name: s.name.clone(), nodes: s.nodes.clone(), roots: s.roots.clone(), meshes, materials: s.materials.clone(), textures: s.textures.clone(), skins: Vec::new(), clips: Vec::new() }
}

/// A mesh of a cell: its slots and its data.
struct Unique {
    scene: Arc<SceneData>,
    data: Arc<MeshUpload>,
}

impl Unique {
    fn of(s: &SceneData) -> Unique {
        Unique { scene: Arc::new(hollow(s)), data: Arc::new(MeshUpload::prepare(s)) }
    }
}

enum Job {
    Shells { tile: usize, cells: Vec<(i32, i32)>, skip: Arc<HashSet<u32>> },
    Detail { key: (i32, i32), skip: Arc<HashSet<u32>>, file: Option<PathBuf> },
    Reload { key: (i32, i32), file: PathBuf },
}

struct Built {
    key: (i32, i32),
    /// Block, shadow proxy, remaining shells.
    unique: [Option<Unique>; 3],
    modules: Vec<(u32, Arc<SceneData>)>,
    placements: Vec<(u32, Mat4)>,
    lights: Vec<Light>,
    cars: Vec<detail::CarSpot>,
    stats: CellStats,
    file: Option<(PathBuf, u64)>,
}

enum Done {
    Shells { tile: usize, cells: Vec<((i32, i32), Unique)> },
    Detail(Box<Built>),
    Reloaded { key: (i32, i32), data: Option<[Option<Arc<MeshUpload>>; 3]>, ms: f32 },
}

/// The unique meshes of a cell as one cache payload (meshes only: the
/// materials and textures stay on the GPU).
fn write_cell(path: &std::path::Path, scenes: [Option<&SceneData>; 3]) -> Option<u64> {
    let mut w = forja_assets::cache::Writer::new();
    for s in scenes {
        w.bool(s.is_some());
        if let Some(s) = s {
            let materials = s.materials.iter().map(|m| forja_assets::MaterialData { base_color_tex: None, metallic_roughness_tex: None, normal_tex: None, occlusion_tex: None, emissive_tex: None, ..m.clone() }).collect();
            let meshes_only = SceneData { name: s.name.clone(), nodes: s.nodes.clone(), roots: s.roots.clone(), meshes: s.meshes.clone(), materials, ..Default::default() };
            if !w.scene(&meshes_only) {
                return None;
            }
        }
    }
    let bytes = w.finish();
    std::fs::write(path, &bytes).ok()?;
    Some(bytes.len() as u64)
}

fn read_cell(path: &std::path::Path) -> Option<[Option<Arc<MeshUpload>>; 3]> {
    let bytes = std::fs::read(path).ok()?;
    let mut r = forja_assets::cache::Reader::new(&bytes)?;
    let mut out: [Option<Arc<MeshUpload>>; 3] = Default::default();
    for o in &mut out {
        if r.bool()? {
            *o = Some(Arc::new(MeshUpload::prepare(&r.scene()?)));
        }
    }
    Some(out)
}

fn worker(kit: Arc<DetailKit>, island: Arc<Island>, index: Arc<DetailIndex>, species: Option<PathBuf>, dress: bool, jobs: Receiver<Job>, done: Sender<Done>) {
    crate::threads::lower_priority();
    let mut b = kit.builder(island.clone(), index.clone(), species.as_deref());
    b.dress = dress;
    for job in jobs {
        let out = match job {
            Job::Shells { tile, cells, skip } => Done::Shells { tile, cells: cells.into_iter().filter_map(|k| index.cell_shell(&island, k, &skip).map(|s| (k, Unique::of(&s)))).collect() },
            Job::Detail { key, skip, file } => {
                let d = b.build_cell(key, &skip);
                let file = file.and_then(|p| write_cell(&p, [d.block.as_ref(), d.proxy.as_ref(), d.rest.as_ref()]).map(|n| (p, n)));
                let lights = d.lights.iter().filter_map(super::env::from_city).collect();
                Done::Detail(Box::new(Built { key, unique: [d.block.as_ref().map(Unique::of), d.proxy.as_ref().map(Unique::of), d.rest.as_ref().map(Unique::of)], modules: d.modules, placements: d.placements, lights, cars: d.cars, stats: d.stats, file }))
            }
            Job::Reload { key, file } => {
                let t = std::time::Instant::now();
                let data = read_cell(&file);
                Done::Reloaded { key, data, ms: t.elapsed().as_secs_f32() * 1000.0 }
            }
        };
        if done.send(out).is_err() {
            break;
        }
        // What earlier jobs allocated here and the main thread has since
        // freed goes back to the system.
        crate::perf::trim_heap();
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CellState {
    /// Showing its shell.
    Shell,
    /// Its detail is being built or read back.
    Loading,
    Detailed,
}

/// What stays of a built cell.
struct Rec {
    /// Render models of the block, the proxy and the remaining shells.
    models: [Option<usize>; 3],
    items: Vec<RenderItem>,
    lights: Vec<Light>,
    file: Option<(PathBuf, u64)>,
    used: u64,
    cars: usize,
    stats: CellStats,
}

struct Cell {
    /// Render model of the cell's shell and whether it is filled.
    shell: Option<usize>,
    state: CellState,
    rec: Option<Rec>,
    /// Version of the skip list the cell was built with.
    skip: u32,
}

#[derive(Default)]
struct Tile {
    requested: bool,
    active: bool,
}

/// Totals for the log and the measurements.
#[derive(Clone, Copy, Default, Debug)]
pub struct DetailStats {
    pub tiles_active: usize,
    pub cells_detailed: usize,
    pub cells_loading: usize,
    pub built: usize,
    pub reloaded: usize,
    pub build_ms_max: f32,
    pub build_ms_sum: f32,
    pub reload_ms_max: f32,
    /// Longest main-thread update of this system (ms).
    pub apply_ms_max: f32,
    pub modules: usize,
    pub instances: usize,
    pub triangles: usize,
    pub lights: usize,
    pub cars: usize,
    pub buildings: usize,
    pub detailed_buildings: usize,
    pub shell_buildings: usize,
    pub over_budget: usize,
    pub cache_bytes: u64,
}

/// Rotation onto +Z and bounds of a car model (as the traffic's).
struct CarModel {
    model: usize,
    local: Mat4,
}

pub struct IslandDetail {
    island: Arc<Island>,
    index: Arc<DetailIndex>,
    jobs: Sender<Job>,
    done: Receiver<Done>,
    tiles: HashMap<usize, Tile>,
    cells: HashMap<(i32, i32), Cell>,
    /// Builder module id -> render model.
    modules: HashMap<u32, usize>,
    /// Modules by kind (scene name), for the log.
    module_kinds: std::collections::BTreeMap<String, usize>,
    cars: Vec<CarModel>,
    in_flight: usize,
    /// Tiles whose cell shells the worker is meshing.
    shells_in_flight: usize,
    dir: Option<PathBuf>,
    /// Held while the session lives (see [`session_dir`]).
    _lock: Option<std::fs::File>,
    frame: u64,
    skip: Arc<HashSet<u32>>,
    skip_version: u32,
    skip_key: (u32, usize, usize, usize),
    /// Block until every wanted cell is in (captures, measurements).
    pub wait: bool,
    pub radius: (f32, f32),
    /// The radius was set by hand (`FORJA_ISLA_DETAIL_M`): the graphics
    /// preset does not change it.
    radius_fixed: bool,
    pub stats: DetailStats,
    logged: (usize, usize),
}

fn layer_shell(key: (i32, i32)) -> u64 {
    1 << 62 | (key.0 as u32 as u64) << 31 | key.1 as u32 as u64 & 0x7fff_ffff
}

fn layer_detail(key: (i32, i32)) -> u64 {
    2 << 62 | (key.0 as u32 as u64) << 31 | key.1 as u32 as u64 & 0x7fff_ffff
}

fn dist_to_square(p: Vec2, lo: Vec2, hi: Vec2) -> f32 {
    (lo - p).max(p - hi).max(Vec2::ZERO).length()
}

const CAR_TINTS: [[f32; 3]; 8] = [[1.0, 1.0, 1.0], [0.78, 0.8, 0.82], [0.5, 0.52, 0.55], [0.22, 0.23, 0.25], [0.85, 0.25, 0.2], [0.3, 0.42, 0.72], [0.9, 0.86, 0.74], [0.35, 0.5, 0.4]];

impl IslandDetail {
    fn cell_file(&self, key: (i32, i32)) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join(format!("cell_{}_{}.bin", key.0, key.1)))
    }

    /// Whether nothing is being built and nothing near is missing.
    pub fn settled(&self) -> bool {
        self.in_flight + self.shells_in_flight == 0 && self.stats.cells_loading == 0
    }

    fn apply_shells(&mut self, rs: &mut RenderScene, tile: usize, cells: Vec<((i32, i32), Unique)>) {
        let group = self.island.tiles[tile].group;
        for (key, u) in cells {
            let cell = self.cells.entry(key).or_insert(Cell { shell: None, state: CellState::Shell, rec: None, skip: self.skip_version });
            let model = *cell.shell.get_or_insert_with(|| rs.add_model(u.scene.clone()));
            rs.layer_cmds.push(LayerCmd::Fill { model, data: u.data });
            rs.layer_cmds.push(LayerCmd::Add { id: layer_shell(key), items: vec![RenderItem::new(model, Mat4::IDENTITY)], visible: cell.state != CellState::Detailed });
        }
        // The tile's one shell mesh goes as its cells' shells come in.
        rs.layer_cmds.push(LayerCmd::HideGroup { group, hidden: true });
        self.tiles.entry(tile).or_default().active = true;
    }

    fn apply_detail(&mut self, rs: &mut RenderScene, b: Built) {
        for (id, scene) in b.modules {
            let kind = scene.name.split(|c: char| c == '_' || c == '(' || c.is_ascii_digit()).next().unwrap_or("").to_string();
            *self.module_kinds.entry(kind).or_default() += 1;
            let m = rs.add_model(scene);
            rs.strip_after_upload.push(m);
            self.modules.insert(id, m);
        }
        self.stats.modules = self.modules.len();
        let Some(cell) = self.cells.get_mut(&b.key) else { return };
        let mut models = [None; 3];
        let mut items: Vec<RenderItem> = Vec::with_capacity(b.placements.len() + b.cars.len() + 3);
        for (k, u) in b.unique.into_iter().enumerate() {
            let Some(u) = u else { continue };
            let model = rs.add_model(u.scene);
            rs.layer_cmds.push(LayerCmd::Fill { model, data: u.data });
            models[k] = Some(model);
            items.push(RenderItem::new(model, Mat4::IDENTITY));
        }
        for (id, m) in &b.placements {
            if let Some(&model) = self.modules.get(id) {
                items.push(RenderItem::new(model, *m));
            }
        }
        if !self.cars.is_empty() {
            for c in &b.cars {
                let car = &self.cars[c.seed as usize % self.cars.len()];
                let t = CAR_TINTS[(c.seed >> 8) as usize % CAR_TINTS.len()];
                let mut it = RenderItem::new(car.model, Mat4::from_rotation_translation(Quat::from_rotation_y(c.yaw), c.pos) * car.local);
                it.tint = Vec4::new(t[0], t[1], t[2], 1.0);
                items.push(it);
            }
        }
        if let Some((_, n)) = &b.file {
            self.stats.cache_bytes += n;
        }
        self.stats.built += 1;
        self.stats.build_ms_max = self.stats.build_ms_max.max(b.stats.ms);
        self.stats.build_ms_sum += b.stats.ms;
        cell.rec = Some(Rec { models, items, lights: b.lights, file: b.file, used: self.frame, cars: b.cars.len(), stats: b.stats });
        Self::show(rs, b.key, cell);
    }

    /// Shows the detail of a cell whose meshes are queued to fill: the
    /// layer, then the swap with the shell.
    fn show(rs: &mut RenderScene, key: (i32, i32), cell: &mut Cell) {
        let Some(rec) = &cell.rec else { return };
        rs.layer_cmds.push(LayerCmd::Add { id: layer_detail(key), items: rec.items.clone(), visible: true });
        rs.layer_cmds.push(LayerCmd::Show { id: layer_shell(key), visible: false });
        cell.state = CellState::Detailed;
    }

    /// Back to the shell; the detail's meshes leave the GPU.
    fn hide(rs: &mut RenderScene, key: (i32, i32), cell: &mut Cell) {
        rs.layer_cmds.push(LayerCmd::Show { id: layer_shell(key), visible: true });
        rs.layer_cmds.push(LayerCmd::Remove { id: layer_detail(key) });
        if let Some(rec) = &cell.rec {
            for m in rec.models.iter().flatten() {
                rs.layer_cmds.push(LayerCmd::Release { model: *m });
            }
        }
        cell.state = CellState::Shell;
    }

    fn receive(&mut self, rs: &mut RenderScene, d: Done) {
        match d {
            Done::Shells { tile, cells } => {
                self.shells_in_flight = self.shells_in_flight.saturating_sub(1);
                self.apply_shells(rs, tile, cells)
            }
            Done::Detail(b) => {
                self.in_flight -= 1;
                self.apply_detail(rs, *b);
            }
            Done::Reloaded { key, data, ms } => {
                self.in_flight -= 1;
                self.stats.reload_ms_max = self.stats.reload_ms_max.max(ms);
                let Some(cell) = self.cells.get_mut(&key) else { return };
                match (data, &mut cell.rec) {
                    (Some(data), Some(rec)) => {
                        for (m, d) in rec.models.iter().zip(data) {
                            if let (Some(m), Some(d)) = (m, d) {
                                rs.layer_cmds.push(LayerCmd::Fill { model: *m, data: d });
                            }
                        }
                        rec.used = self.frame;
                        self.stats.reloaded += 1;
                        Self::show(rs, key, cell);
                    }
                    _ => {
                        // The file is gone or unreadable: build it again.
                        cell.rec = None;
                        cell.state = CellState::Shell;
                    }
                }
            }
        }
    }

    /// One frame for a camera at `eye`.
    pub fn update(&mut self, rs: &mut RenderScene, eye: Vec3, skip: Option<&DetailSkip>) {
        let t = std::time::Instant::now();
        self.frame += 1;
        if !self.radius_fixed
            && let Some(r) = rs.detail_radius
        {
            self.radius = (r, r + 40.0);
        }
        if let Some(s) = skip {
            let key = (s.version, s.polys.len(), s.osm.len(), s.rings.len());
            if key != self.skip_key {
                self.skip_key = key;
                self.skip = Arc::new(s.resolve(&self.island));
                // Cells built before the change are built again, and the
                // split tiles mesh their cells' shells again.
                self.skip_version += 1;
                for t in self.tiles.values_mut() {
                    t.requested = false;
                }
            }
        }
        let p = Vec2::new(eye.x, eye.z);
        // Results: every shell, one detail per frame.
        let mut details = 0;
        while details < 1 {
            let Ok(d) = self.done.try_recv() else { break };
            if !matches!(d, Done::Shells { .. }) {
                details += 1;
            }
            self.receive(rs, d);
        }
        self.step(rs, p);
        self.stats.apply_ms_max = self.stats.apply_ms_max.max(t.elapsed().as_secs_f32() * 1000.0);
        // Captures: wait for the worker instead of the next frames.
        while self.wait && self.in_flight + self.shells_in_flight > 0 {
            let Ok(d) = self.done.recv() else { break };
            let t = std::time::Instant::now();
            self.receive(rs, d);
            self.step(rs, p);
            self.stats.apply_ms_max = self.stats.apply_ms_max.max(t.elapsed().as_secs_f32() * 1000.0);
        }
        let now = (self.stats.cells_detailed, self.stats.built + self.stats.reloaded);
        if now != self.logged && (self.frame % 30 == 0 || self.wait) && self.in_flight == 0 {
            self.logged = now;
            let s = &self.stats;
            log::info!(
                "island detail: {} cells detailed ({} tiles split), {} buildings ({} detailed, {} shells, {} over budget), {} instances of {} modules ({}), {:.2} M triangles at full detail, {} lights, {} cars; built {} (max {:.0} ms, mean {:.0} ms), reloaded {} (max {:.0} ms), main thread max {:.2} ms, cache {:.0} MB, resident {:.0} MB",
                s.cells_detailed,
                s.tiles_active,
                s.buildings,
                s.detailed_buildings,
                s.shell_buildings,
                s.over_budget,
                s.instances,
                s.modules,
                self.module_kinds.iter().map(|(k, n)| format!("{n} {k}")).collect::<Vec<_>>().join(", "),
                s.triangles as f64 / 1e6,
                s.lights,
                s.cars,
                s.built,
                s.build_ms_max,
                s.build_ms_sum / s.built.max(1) as f32,
                s.reloaded,
                s.reload_ms_max,
                s.apply_ms_max,
                s.cache_bytes as f64 / 1048576.0,
                crate::perf::resident_mb()
            );
        }
    }

    /// Decides what each tile and cell near `p` should be and asks the
    /// worker for what is missing.
    fn step(&mut self, rs: &mut RenderScene, p: Vec2) {
        let (r_in, r_out) = self.radius;
        let island = self.island.clone();
        let reach = ((r_out + 110.0) / BTILE).ceil() as i32 + 1;
        // Tiles.
        for ti in island.tiles_near(Vec3::new(p.x, 0.0, p.y), reach) {
            let key = island.tiles[ti].key;
            let lo = Vec2::new(key.0 as f32, key.1 as f32) * BTILE;
            let d = dist_to_square(p, lo, lo + Vec2::splat(BTILE));
            let tile = self.tiles.entry(ti).or_default();
            if d < r_in + 70.0 && !tile.requested {
                tile.requested = true;
                self.shells_in_flight += 1;
                let _ = self.jobs.send(Job::Shells { tile: ti, cells: self.index.cells_of_tile(key), skip: self.skip.clone() });
            }
        }
        // Tiles far behind: one mesh again.
        let far: Vec<usize> = self
            .tiles
            .iter()
            .filter(|(ti, t)| {
                let key = island.tiles[**ti].key;
                let lo = Vec2::new(key.0 as f32, key.1 as f32) * BTILE;
                t.active && dist_to_square(p, lo, lo + Vec2::splat(BTILE)) > r_out + 110.0
            })
            .map(|(ti, _)| *ti)
            .collect();
        for ti in far {
            let key = island.tiles[ti].key;
            let cells = self.index.cells_of_tile(key);
            if cells.iter().any(|k| self.cells.get(k).is_some_and(|c| c.state != CellState::Shell)) {
                continue;
            }
            rs.layer_cmds.push(LayerCmd::HideGroup { group: island.tiles[ti].group, hidden: false });
            for k in cells {
                if let Some(c) = self.cells.get(&k) {
                    rs.layer_cmds.push(LayerCmd::Remove { id: layer_shell(k) });
                    if let Some(m) = c.shell {
                        rs.layer_cmds.push(LayerCmd::Release { model: m });
                    }
                }
            }
            self.tiles.insert(ti, Tile::default());
        }
        // Cells: the nearest missing one is built next.
        let mut want: Option<((i32, i32), f32)> = None;
        let (mut detailed, mut loading) = (0, 0);
        let mut totals = DetailStats::default();
        let frame = self.frame;
        for (key, cell) in self.cells.iter_mut() {
            let ti = island.tile_index.get(&detail::tile_of_cell(*key));
            if !ti.is_some_and(|ti| self.tiles.get(ti).is_some_and(|t| t.active)) {
                continue;
            }
            let (lo, hi) = detail::cell_bounds(*key);
            let d = dist_to_square(p, lo, hi);
            match cell.state {
                CellState::Shell if d < r_in => {
                    loading += 1;
                    if want.is_none_or(|(_, best)| d < best) {
                        want = Some((*key, d));
                    }
                }
                CellState::Detailed if d > r_out || cell.skip != self.skip_version => {
                    Self::hide(rs, *key, cell);
                    if cell.skip != self.skip_version {
                        cell.rec = None;
                        cell.skip = self.skip_version;
                    }
                }
                CellState::Detailed => {
                    detailed += 1;
                    if let Some(rec) = &mut cell.rec {
                        rec.used = frame;
                        totals.instances += rec.items.len();
                        totals.triangles += rec.stats.block_triangles + rec.stats.module_triangles;
                        totals.lights += rec.lights.len();
                        totals.cars += rec.cars;
                        totals.buildings += rec.stats.buildings;
                        totals.detailed_buildings += rec.stats.detailed;
                        totals.shell_buildings += rec.stats.shells;
                        totals.over_budget += rec.stats.over_budget;
                    }
                }
                CellState::Loading => loading += 1,
                _ => {}
            }
        }
        self.stats.cells_detailed = detailed;
        self.stats.cells_loading = loading;
        self.stats.tiles_active = self.tiles.values().filter(|t| t.active).count();
        self.stats.instances = totals.instances;
        self.stats.triangles = totals.triangles;
        self.stats.lights = totals.lights;
        self.stats.cars = totals.cars;
        self.stats.buildings = totals.buildings;
        self.stats.detailed_buildings = totals.detailed_buildings;
        self.stats.shell_buildings = totals.shell_buildings;
        self.stats.over_budget = totals.over_budget;
        if let Some((key, _)) = want
            && self.in_flight == 0
        {
            let file = self.cell_file(key);
            let cell = self.cells.get_mut(&key).expect("cell");
            let job = match &cell.rec {
                Some(Rec { file: Some((f, _)), .. }) => Job::Reload { key, file: f.clone() },
                _ => {
                    cell.rec = None;
                    cell.skip = self.skip_version;
                    Job::Detail { key, skip: self.skip.clone(), file }
                }
            };
            cell.state = CellState::Loading;
            self.in_flight += 1;
            let _ = self.jobs.send(job);
        }
        self.forget();
    }

    /// Keeps the remembered cells and their files within the caps: the
    /// ones used longest ago go first.
    fn forget(&mut self) {
        let kept = self.cells.values().filter(|c| c.rec.is_some() && c.state == CellState::Shell).count();
        if kept <= KEPT_CELLS && self.stats.cache_bytes <= CACHE_BYTES {
            return;
        }
        let mut old: Vec<((i32, i32), u64)> = self.cells.iter().filter(|(_, c)| c.state == CellState::Shell).filter_map(|(k, c)| c.rec.as_ref().map(|r| (*k, r.used))).collect();
        old.sort_by_key(|o| o.1);
        let mut kept = kept;
        for (k, _) in old {
            if kept <= KEPT_CELLS * 3 / 4 && self.stats.cache_bytes <= CACHE_BYTES * 3 / 4 {
                break;
            }
            if let Some(rec) = self.cells.get_mut(&k).and_then(|c| c.rec.take()) {
                if let Some((f, n)) = rec.file {
                    let _ = std::fs::remove_file(f);
                    self.stats.cache_bytes = self.stats.cache_bytes.saturating_sub(n);
                }
                kept -= 1;
            }
        }
    }

    /// Night lights of the detailed cells.
    pub fn lights(&self) -> impl Iterator<Item = &Light> {
        self.cells.values().filter(|c| c.state == CellState::Detailed).filter_map(|c| c.rec.as_ref()).flat_map(|r| r.lights.iter())
    }
}

impl Drop for IslandDetail {
    fn drop(&mut self) {
        if let Some(d) = self.dir.take() {
            let _ = std::fs::remove_dir_all(d);
        }
    }
}

static SESSION: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// The session's folder of the cell cache and the lock that marks it as
/// in use. Folders whose lock nobody holds belong to sessions that are
/// gone (a crash, a killed process) and are removed.
fn session_dir() -> Option<(PathBuf, std::fs::File)> {
    let root = forja_assets::cache::dir()?.join("isla_detail");
    for e in std::fs::read_dir(&root).into_iter().flatten().flatten() {
        let path = e.path();
        let stale = match std::fs::File::open(path.join("lock")) {
            Ok(f) => f.try_lock().is_ok(),
            Err(_) => e.metadata().and_then(|m| m.modified()).is_ok_and(|t| t.elapsed().is_ok_and(|a| a.as_secs() > 600)),
        };
        if stale {
            let _ = std::fs::remove_dir_all(path);
        }
    }
    let dir = root.join(format!("{}-{}", std::process::id(), SESSION.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    std::fs::create_dir_all(&dir).ok()?;
    let lock = std::fs::File::create(dir.join("lock")).ok()?;
    lock.try_lock().ok()?;
    Some((dir, lock))
}

fn car_models(rs: &mut RenderScene, paths: &Paths) -> Vec<CarModel> {
    ["sedan", "hatchback-sports", "suv", "sedan-sports", "van", "suv-luxury", "taxi"]
        .iter()
        .filter_map(|f| {
            let c = super::traffic::car_model(rs, paths, &format!("kenney_car_kit/{f}.glb"))?;
            let len = if *f == "van" { 4.9 } else { 4.3 };
            Some(CarModel { model: c.model, local: c.local(len) })
        })
        .collect()
}

/// Adds the island's detail streaming when the loaded world is an island
/// (`hour`: the clock hour of the night content, as the world's).
/// `FORJA_ISLA_DETAIL=0` turns it off, `FORJA_ISLA_DETAIL=buildings`
/// leaves the streets undressed, `FORJA_ISLA_DETAIL_WAIT=1` makes every
/// frame wait for the cells it wants (captures), `FORJA_ISLA_DETAIL_M`
/// sets the radius (m).
pub fn install(app: &mut App, hour: Option<f32>) {
    let Some(island) = app.world.res::<GameWorld>().desc.island.clone() else { return };
    let mode = std::env::var("FORJA_ISLA_DETAIL").unwrap_or_default();
    if mode == "0" || mode == "off" {
        return;
    }
    let t0 = std::time::Instant::now();
    let paths = app.world.res::<Paths>().clone();
    let styles = paths.data.join("city/styles.toml");
    let mut kit = DetailKit::load(Some(&paths.downloaded).filter(|p| p.exists()).map(|p| p.as_path()), Some(styles.as_path()).filter(|p| p.exists()), hour);
    // The kit's textures go up with the world, under the loading screen,
    // as one small scene; the kit then keeps placeholders for them (it
    // lives as long as the island, and its pixels are a gigabyte).
    {
        let warm = Arc::new(kit.warm_scene());
        let pairs = kit.hollow(&warm);
        let rs = app.world.res_mut::<RenderScene>();
        let m = rs.add_model(warm);
        rs.strip_after_upload.push(m);
        rs.texture_aliases = Some((m, pairs));
    }
    let kit = Arc::new(kit);
    let index = Arc::new(DetailIndex::new(&island));
    let species = Some(paths.data.join("flora/species.toml")).filter(|p| p.exists());
    let (jobs, jobs_rx) = channel();
    let (done_tx, done) = channel();
    let dress = mode != "buildings";
    {
        let (kit, island, index) = (kit.clone(), island.clone(), index.clone());
        if std::thread::Builder::new().name("island detail".into()).spawn(move || worker(kit, island, index, species, dress, jobs_rx, done_tx)).is_err() {
            return;
        }
    }
    let rs = app.world.res_mut::<RenderScene>();
    let cars = if dress { car_models(rs, &paths) } else { Vec::new() };
    let radius = std::env::var("FORJA_ISLA_DETAIL_M").ok().and_then(|v| v.parse::<f32>().ok()).filter(|r| *r >= 40.0 && *r <= 600.0).map_or((DETAIL_IN, DETAIL_OUT), |r| (r, r + 40.0));
    let (dir, lock) = session_dir().map_or((None, None), |(d, l)| (Some(d), Some(l)));
    let mut detail = IslandDetail {
        island,
        index,
        jobs,
        done,
        tiles: HashMap::new(),
        cells: HashMap::new(),
        modules: HashMap::new(),
        module_kinds: Default::default(),
        cars,
        in_flight: 0,
        shells_in_flight: 0,
        dir,
        _lock: lock,
        frame: 0,
        skip: Arc::new(HashSet::new()),
        skip_version: 0,
        skip_key: (0, 0, 0, 0),
        wait: std::env::var_os("FORJA_ISLA_DETAIL_WAIT").is_some(),
        radius,
        radius_fixed: std::env::var_os("FORJA_ISLA_DETAIL_M").is_some(),
        stats: DetailStats::default(),
        logged: (0, 0),
    };
    // The cells around the start are asked for now: they build while the
    // rest of the game loads.
    let spawn = app.world.res::<GameWorld>().desc.spawn(SpawnKind::Player).map_or(Vec3::ZERO, |s| s.pos);
    if !detail.wait {
        let skip = app.world.resources.get::<DetailSkip>().cloned();
        detail.update(app.world.res_mut::<RenderScene>(), spawn, skip.as_ref());
    }
    log::info!("island detail: kit and index in {:.2}s, {} cells with buildings, radius {:.0}/{:.0} m", t0.elapsed().as_secs_f32(), detail.index.cells.len(), radius.0, radius.1);
    app.insert_resource(detail);
    app.add_system(Stage::PreUpdate, "island_detail", |w| {
        // The camera of the last frame; a fixed camera (captures,
        // cinematics) is known before its first frame, and before any
        // frame the player's start stands in.
        let cam = w.res::<RenderScene>().camera.position;
        let fixed = w.resources.get::<super::camera::CameraRig>().and_then(|r| r.fixed).map(|f| f.0);
        let eye = fixed.unwrap_or(if cam == Vec3::ZERO { w.res::<GameWorld>().desc.spawn(SpawnKind::Player).map_or(cam, |s| s.pos) } else { cam });
        let skip = w.resources.get::<DetailSkip>().cloned();
        w.scope::<IslandDetail, _>(|w, d| d.update(w.res_mut::<RenderScene>(), eye, skip.as_ref()));
    });
    // Night: the lights of the detailed cells, dimmed in through dusk as
    // the map's.
    app.add_system_at(Stage::PostUpdate, "island_detail_lights", 51, |w| {
        let Some(t) = w.res::<super::env::TimeOfDay>().dusk_to_night() else { return };
        let level = (t / 0.35).clamp(0.0, 1.0);
        let level = level * level * (3.0 - 2.0 * level);
        let lights: Vec<Light> = w.res::<IslandDetail>().lights().map(|l| Light { intensity: l.intensity * level, ..*l }).collect();
        w.res_mut::<RenderScene>().local_lights.extend(lights);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_ids_are_distinct() {
        let a = layer_shell((-3, 7));
        let b = layer_detail((-3, 7));
        assert_ne!(a, b);
        assert_ne!(layer_shell((-3, 7)), layer_shell((7, -3)));
        assert_ne!(layer_detail((0, 1)), layer_detail((1, 0)));
    }

    #[test]
    fn squares_measure_distance() {
        let (lo, hi) = (Vec2::ZERO, Vec2::splat(64.0));
        assert_eq!(dist_to_square(Vec2::new(10.0, 10.0), lo, hi), 0.0);
        assert!((dist_to_square(Vec2::new(-30.0, 104.0), lo, hi) - 50.0).abs() < 1e-4);
    }
}
