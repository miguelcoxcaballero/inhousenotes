//! Streaming of the city's tile detail.
//!
//! Each city tile has detail (its block scene: walls with openings,
//! mouldings, roofs; its window, shop and balcony modules) and an HLOD
//! stand-in. The renderer draws the stand-in beyond
//! `RenderSettings::hlod_distance`; this module also takes the tile's
//! unique block meshes off the GPU once the tile is well beyond that
//! distance, and brings them back on a worker thread before the camera
//! gets close again:
//!
//! * at the first upload every block scene is written to
//!   `cache/stream/<process>/` (a writer thread; the CPU copy is then
//!   dropped as before);
//! * beyond 1.4x the HLOD distance a tile's blocks are released
//!   ([`Renderer::release_model`]) and the tile is held at its HLOD;
//! * within 1.2x a worker reads and decodes them and converts the
//!   vertices to the renderer's layout; the main thread only copies them
//!   into the renderer's shared mesh pools (no GPU resource is created and
//!   nothing waits), model by model and at most [`RELOAD_BYTES_PER_FRAME`]
//!   per frame (a single larger model goes alone in its frame), and lets
//!   the tile show its detail again (the switch itself happens at 1.0x /
//!   0.9x, so a tile is resident before it is needed and never flips at
//!   the border).
//!
//! Shared modules (windows, shops) are used by every tile and stay
//! resident; they are cheap to keep and their instances are culled with
//! the tile.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use forja_assets::SceneData;
use forja_math::Vec3;
use forja_render::{MeshUpload, ModelHandle, Renderer};

/// Re-upload budget per frame: a typical tile (~5 MB) at a time; a larger
/// tile is spread over several frames, so a tile becoming resident costs
/// a few small memcpys, not a hitch.
pub const RELOAD_BYTES_PER_FRAME: u64 = 8 << 20;

enum State {
    Resident,
    Released,
    Loading(Receiver<Option<Vec<MeshUpload>>>),
    /// Read back; its models are copied in from `next` on, within each
    /// frame's budget. The tile shows its detail once all are in.
    Uploading { scenes: Vec<MeshUpload>, next: usize, bytes: u64 },
}

struct Tile {
    group: u32,
    /// Render models of the tile's block scenes.
    models: Vec<usize>,
    state: State,
}

/// Streams the detail of city tiles in and out.
pub struct Streamer {
    dir: PathBuf,
    tiles: Vec<Tile>,
    writer: Option<Sender<(PathBuf, Arc<SceneData>)>>,
    writer_done: Option<std::thread::JoinHandle<()>>,
    /// Bytes currently released.
    pub released_bytes: u64,
    pub released_tiles: usize,
    pub loads: usize,
}

static SESSION: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

impl Streamer {
    /// `tiles`: `(group, render models of its blocks)`. `None` when
    /// streaming is off (`FORJA_NO_STREAM=1`) or there is no cache folder.
    pub fn new(tiles: Vec<(u32, Vec<usize>)>) -> Option<Self> {
        if tiles.is_empty() || std::env::var_os("FORJA_NO_STREAM").is_some() {
            return None;
        }
        let n = SESSION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = forja_assets::cache::dir()?.join("stream");
        // Folders of sessions that did not exit cleanly.
        for e in std::fs::read_dir(&root).into_iter().flatten().flatten() {
            if e.metadata().and_then(|m| m.modified()).is_ok_and(|t| t.elapsed().is_ok_and(|a| a.as_secs() > 6 * 3600)) {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
        let dir = root.join(format!("{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).ok()?;
        let (tx, rx) = channel::<(PathBuf, Arc<SceneData>)>();
        let writer_done = std::thread::Builder::new()
            .name("stream writer".into())
            .spawn(move || {
                for (path, scene) in rx {
                    // Meshes only: the textures stay on the GPU (and are
                    // shared with every other tile).
                    let materials = scene.materials.iter().map(|m| forja_assets::MaterialData { base_color_tex: None, metallic_roughness_tex: None, normal_tex: None, occlusion_tex: None, emissive_tex: None, ..m.clone() }).collect();
                    let meshes_only = SceneData { name: scene.name.clone(), nodes: scene.nodes.clone(), roots: scene.roots.clone(), meshes: scene.meshes.clone(), materials, ..Default::default() };
                    drop(scene);
                    let mut w = forja_assets::cache::Writer::new();
                    if w.scene(&meshes_only) {
                        let _ = std::fs::write(&path, w.finish());
                    }
                }
            })
            .ok()?;
        Some(Self { dir, tiles: tiles.into_iter().map(|(group, models)| Tile { group, models, state: State::Resident }).collect(), writer: Some(tx), writer_done: Some(writer_done), released_bytes: 0, released_tiles: 0, loads: 0 })
    }

    fn path(&self, model: usize) -> PathBuf {
        self.dir.join(format!("{model}.bin"))
    }

    /// Whether `model` is streamed (its scene must be kept on disk).
    pub fn streams(&self, model: usize) -> bool {
        self.tiles.iter().any(|t| t.models.contains(&model))
    }

    /// Keeps the full scene of a streamed model before its CPU copy is
    /// dropped.
    pub fn keep(&self, model: usize, scene: Arc<SceneData>) {
        if let Some(tx) = &self.writer {
            let _ = tx.send((self.path(model), scene));
        }
    }

    /// Once the first upload is done the writer gets no more scenes;
    /// `true` once it has written them all (tiles may then be released).
    /// Never waits for it.
    fn writer_finished(&mut self) -> bool {
        self.writer = None;
        match &self.writer_done {
            Some(h) if !h.is_finished() => false,
            Some(_) => {
                if let Some(h) = self.writer_done.take() {
                    let _ = h.join();
                }
                true
            }
            None => true,
        }
    }

    /// One frame of streaming decisions for a camera at `eye`.
    pub fn update(&mut self, renderer: &mut Renderer, handles: &[ModelHandle], eye: Vec3) {
        if !self.writer_finished() {
            return;
        }
        let d0 = renderer.settings.hlod_distance;
        let mut budget = RELOAD_BYTES_PER_FRAME;
        for i in 0..self.tiles.len() {
            let group = self.tiles[i].group;
            let d = renderer.static_group_distance(group, eye).unwrap_or(0.0);
            let near = d0 <= 0.0 || d < d0 * 1.2;
            match &self.tiles[i].state {
                State::Resident if !near && d > d0 * 1.4 => {
                    renderer.set_static_group_ready(group, false);
                    let mut freed = 0;
                    for &m in &self.tiles[i].models {
                        freed += renderer.release_model(handles[m]);
                    }
                    self.released_bytes += freed;
                    self.released_tiles += 1;
                    self.tiles[i].state = State::Released;
                }
                State::Released if near => {
                    let (tx, rx) = channel();
                    let paths: Vec<PathBuf> = self.tiles[i].models.iter().map(|&m| self.path(m)).collect();
                    let _ = std::thread::Builder::new().name("stream loader".into()).spawn(move || {
                        crate::threads::lower_priority();
                        let scenes: Option<Vec<MeshUpload>> = paths.iter().map(|p| std::fs::read(p).ok().and_then(|b| forja_assets::cache::Reader::new(&b)).and_then(|mut r| r.scene()).map(|s| MeshUpload::prepare(&s))).collect();
                        let _ = tx.send(scenes);
                    });
                    self.tiles[i].state = State::Loading(rx);
                }
                State::Loading(rx) if budget > 0 => {
                    if let Ok(result) = rx.try_recv() {
                        let Some(scenes) = result else {
                            log::warn!("stream: tile {group} could not be read back; it stays at its HLOD");
                            self.tiles[i].state = State::Released;
                            continue;
                        };
                        self.tiles[i].state = State::Uploading { scenes, next: 0, bytes: 0 };
                        self.upload_tile(i, renderer, handles, &mut budget);
                    }
                }
                State::Uploading { .. } if budget > 0 => self.upload_tile(i, renderer, handles, &mut budget),
                _ => {}
            }
        }
    }

    /// Copies the models of tile `i` (in [`State::Uploading`]) into the
    /// renderer while `budget` lasts; the tile is resident once all are in.
    fn upload_tile(&mut self, i: usize, renderer: &mut Renderer, handles: &[ModelHandle], budget: &mut u64) {
        let tile = &mut self.tiles[i];
        let State::Uploading { scenes, next, bytes } = &mut tile.state else { return };
        while *next < scenes.len().min(tile.models.len()) && *budget > 0 {
            let s = &scenes[*next];
            *budget = budget.saturating_sub(s.bytes());
            *bytes += renderer.reload_prepared(handles[tile.models[*next]], s);
            *next += 1;
        }
        if *next < scenes.len().min(tile.models.len()) {
            return;
        }
        let bytes = *bytes;
        self.released_bytes = self.released_bytes.saturating_sub(bytes);
        self.released_tiles -= 1;
        self.loads += 1;
        renderer.set_static_group_ready(tile.group, true);
        tile.state = State::Resident;
    }
}

impl Drop for Streamer {
    /// Deletes the session folder. A writer still at work (a world swapped
    /// right after its upload) is waited for on a thread of its own, not on
    /// the main thread; the folder of a process that exits first is
    /// removed by a later session.
    fn drop(&mut self) {
        self.writer = None;
        let dir = std::mem::take(&mut self.dir);
        match self.writer_done.take() {
            Some(h) if !h.is_finished() => {
                let _ = std::thread::Builder::new().name("stream cleanup".into()).spawn(move || {
                    crate::threads::lower_priority();
                    let _ = h.join();
                    let _ = std::fs::remove_dir_all(dir);
                });
            }
            h => {
                if let Some(h) = h {
                    let _ = h.join();
                }
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }
}
