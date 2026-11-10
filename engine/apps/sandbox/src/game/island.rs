//! The island at run time: the buildings' colliders follow the camera
//! (a 3x3 block of 256 m tiles around it; the ground is one height field
//! that is always there), and the modelled landmarks of Valencia
//! (`data/landmarks/valencia`) replace the shells of their sites, where
//! `data/maps/isla/landmarks.toml` places them, as a background thread
//! reads their models from the asset cache (or builds them the first
//! time), nearest first; their collision meshes follow the camera. The
//! detailed facades near the player are `island_detail`'s.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

use forja_assets::SceneData;
use forja_core::{App, Stage};
use forja_math::Vec3;
use forja_physics::{ColliderDesc, ColliderHandle, CollisionGroups, PhysicsWorld, Shape, TriMesh, layers};
use forja_world::{Island, SpawnKind};

use super::island_detail::LayerCmd;
use super::world::GameWorld;
use super::{Paths, RenderItem, RenderScene};

/// Tiles this far (in tiles) from the camera's tile get colliders; they
/// are dropped one tile further out.
const NEAR: i32 = 1;
/// A landmark's collision mesh is in the physics world within this
/// distance (m) of the camera, and taken out [`LANDMARK_KEEP`] away.
const LANDMARK_NEAR: f32 = 220.0;
const LANDMARK_KEEP: f32 = 320.0;
/// Streams the island's building colliders.
pub struct IslandColliders {
    pub island: Arc<Island>,
    loaded: HashMap<usize, ColliderHandle>,
    /// Collision meshes of the landmarks whose models have arrived (by
    /// index in `island.landmarks`), and those in the physics world.
    landmark_meshes: HashMap<usize, Arc<TriMesh>>,
    landmarks: HashMap<usize, ColliderHandle>,
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
        // Landmarks collide as their models while the camera is near.
        for (&i, mesh) in &self.landmark_meshes {
            let lm = &self.island.landmarks[i];
            let d = lm.distance(p);
            if d < LANDMARK_NEAR && !self.landmarks.contains_key(&i) {
                let desc = ColliderDesc::new(Shape::TriMesh(mesh.clone())).local(lm.iso).friction(0.8).groups(CollisionGroups::new(layers::WORLD, u32::MAX));
                self.landmarks.insert(i, physics.add_collider(desc, None));
            } else if d > LANDMARK_KEEP
                && let Some(h) = self.landmarks.remove(&i)
            {
                physics.remove_collider(h);
            }
        }
    }

    pub fn loaded(&self) -> usize {
        self.loaded.len()
    }
}

/// The modelled landmarks of Valencia and where they stand on the island
/// (`data/maps/isla/landmarks.toml`, see `forja_world::island::landmarks`).
/// `FORJA_NO_LANDMARKS=1`: plain shells.
pub fn landmark_set(paths: &Paths) -> Option<forja_world::island::landmarks::Set> {
    if std::env::var_os("FORJA_NO_LANDMARKS").is_some() {
        return None;
    }
    match forja_world::island::landmarks::Set::load(&paths.data.join("maps/isla/landmarks.toml"), &paths.data.join("landmarks/valencia")) {
        Ok(set) => Some(set),
        Err(e) => {
            log::warn!("landmarks: {e}");
            None
        }
    }
}

/// A landmark from the loader thread: its index in `Island::landmarks`,
/// its scene and its collision mesh.
type Arrived = (usize, Arc<SceneData>, Option<Arc<TriMesh>>);

/// Landmarks arriving from the loader thread.
pub struct Landmarks {
    island: Arc<Island>,
    rx: Receiver<Arrived>,
    pub placed: usize,
    pub total: usize,
}

/// Layer ids of the landmarks (the detail cells use their own range).
const LANDMARK_LAYER: u64 = 1 << 48;

fn start_landmarks(app: &mut App, island: &Arc<Island>, spawn: Vec3) {
    if island.landmarks.is_empty() {
        return;
    }
    let paths = app.world.res::<Paths>().clone();
    // Nearest to the camera first (the player's spawn unless the run
    // starts with a camera of its own).
    let eye = Some(app.world.res::<RenderScene>().camera.position).filter(|p| p.length_squared() > 1.0).unwrap_or(spawn);
    let mut jobs: Vec<(usize, String, f32)> = island.landmarks.iter().enumerate().map(|(k, l)| (k, l.slug.clone(), l.distance(eye))).collect();
    jobs.sort_by(|a, b| a.2.total_cmp(&b.2));
    let total = jobs.len();
    let (tx, rx) = channel();
    let spawned = std::thread::Builder::new().name("landmarks".into()).stack_size(16 << 20).spawn(move || {
        crate::threads::lower_priority();
        let t = std::time::Instant::now();
        // From the asset cache; built the first time (minutes for all of
        // them: their shells stand in meanwhile). Dropped at the end: it
        // holds the textures the landmarks share.
        let models = forja_world::island::landmarks::Models::new(&paths.data.join("landmarks/valencia"));
        let (mut loaded, mut built) = (0, 0);
        for (k, slug, _) in jobs {
            // A landmark is data: one that fails to build keeps its shell.
            let Some((m, new)) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| models.load(&slug))).unwrap_or_else(|_| {
                log::warn!("landmark {slug}: its builder panicked");
                None
            }) else {
                continue;
            };
            loaded += 1;
            built += usize::from(new);
            let collider = (!m.col_tris.is_empty()).then(|| Arc::new(TriMesh::new(m.col_verts, m.col_tris)));
            if tx.send((k, Arc::new(m.scene), collider)).is_err() {
                return;
            }
        }
        log::info!("landmarks: {loaded} of {total} ready in {:.1?} ({built} built, the rest from the cache)", t.elapsed());
    });
    if spawned.is_err() {
        return;
    }
    app.insert_resource(Landmarks { island: island.clone(), rx, placed: 0, total });
    app.add_system(Stage::PreUpdate, "island_landmarks", |w| {
        // One per frame: its upload is a few megabytes.
        let arrived = w.scope::<Landmarks, _>(|w, st| {
            let (k, scene, collider) = st.rx.try_recv().ok()?;
            let lm = st.island.landmarks.get(k)?;
            let rs = w.res_mut::<RenderScene>();
            let model = rs.add_model(scene);
            rs.strip_after_upload.push(model);
            // The model as a static layer, where the table places it; its
            // site's shell goes once it is in.
            rs.layer_cmds.push(LayerCmd::Add { id: LANDMARK_LAYER + k as u64, items: vec![RenderItem::new(model, lm.transform())], visible: true });
            if let Some(site) = st.island.sites.get(lm.site).copied().flatten() {
                rs.layer_cmds.push(LayerCmd::HideGroup { group: site.group, hidden: true });
            }
            st.placed += 1;
            collider.map(|c| (k, c))
        });
        if let Some((k, c)) = arrived
            && let Some(col) = w.resources.get_mut::<IslandColliders>()
        {
            col.landmark_meshes.insert(k, c);
        }
    });
}

/// Adds the island's run-time systems when the loaded world is an island.
pub fn install(app: &mut App) {
    let Some(island) = app.world.res::<GameWorld>().desc.island.clone() else { return };
    let spawn = app.world.res::<GameWorld>().desc.spawn(SpawnKind::Player).map_or(Vec3::ZERO, |s| s.pos);
    let mut col = IslandColliders { island, loaded: HashMap::new(), landmark_meshes: HashMap::new(), landmarks: HashMap::new() };
    col.update(app.world.res_mut::<PhysicsWorld>(), spawn, usize::MAX);
    // The detail cells leave the landmarks' footprints to their models.
    app.insert_resource(super::island_detail::DetailSkip { polys: col.island.buildings.iter().filter(|b| b.site.is_some()).map(|b| b.poly).collect(), ..Default::default() });
    start_landmarks(app, &col.island, spawn);
    app.insert_resource(col);
    app.add_system(Stage::PreUpdate, "island_colliders", |w| {
        let p = w.res::<RenderScene>().camera.position;
        w.scope::<IslandColliders, _>(|w, col| col.update(w.res_mut::<PhysicsWorld>(), p, 1));
    });
}
