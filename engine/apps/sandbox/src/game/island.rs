//! The island at run time: the buildings' colliders follow the camera
//! (a 3x3 block of 256 m tiles around it; the ground is one height field
//! that is always there), and the modelled landmarks of Valencia
//! (`data/landmarks/valencia`) replace the shells of their footprints as a
//! background thread builds them, nearest to the player first. The
//! detailed facades near the player are `island_detail`'s.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

use forja_assets::SceneData;
use forja_core::{App, Stage};
use forja_math::{Mat4, Vec2, Vec3};
use forja_physics::{ColliderDesc, ColliderHandle, CollisionGroups, PhysicsWorld, Shape, TriMesh, layers};
use forja_world::island::Site;
use forja_world::{Island, SpawnKind};

use super::island_detail::LayerCmd;
use super::world::GameWorld;
use super::{Paths, RenderItem, RenderScene};

/// Tiles this far (in tiles) from the camera's tile get colliders; they
/// are dropped one tile further out.
const NEAR: i32 = 1;
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

/// Landmarks arriving from the builder thread.
pub struct Landmarks {
    island: Arc<Island>,
    rx: Receiver<(usize, Arc<SceneData>, Vec2)>,
    pub placed: usize,
    pub total: usize,
}

/// Layer ids of the landmarks (the detail cells use their own range).
const LANDMARK_LAYER: u64 = 1 << 48;

fn start_landmarks(app: &mut App, island: &Arc<Island>, spawn: Vec3) {
    let paths = app.world.res::<Paths>().clone();
    let mut jobs: Vec<(usize, PathBuf, f32)> = landmark_sites(&paths)
        .into_iter()
        .enumerate()
        .filter(|(k, _)| island.sites.get(*k).is_some_and(Option::is_some))
        .map(|(k, (dir, site))| {
            let c = site.rings[0].iter().copied().sum::<Vec2>() / site.rings[0].len() as f32;
            (k, dir, c.distance(Vec2::new(spawn.x, spawn.z)))
        })
        .collect();
    // Opt-in for now (`FORJA_LANDMARKS=1`): the models need the city kit a
    // second time (the detail cells hold their own); without them every
    // landmark keeps its shell.
    if jobs.is_empty() || std::env::var_os("FORJA_LANDMARKS").is_none() {
        return;
    }
    jobs.sort_by(|a, b| a.2.total_cmp(&b.2));
    let total = jobs.len();
    let (tx, rx) = channel();
    let spawned = std::thread::Builder::new().name("landmarks".into()).stack_size(16 << 20).spawn(move || {
        crate::threads::lower_priority();
        let t = std::time::Instant::now();
        let kit = forja_city::landmark::shared_library(&paths.data).kit;
        let mut built = 0;
        for (k, dir, _) in jobs {
            // A landmark is data: one that fails to build keeps its shell.
            // (A material library of its own: the shared one keeps every
            // texture set it has ever built.)
            match std::panic::catch_unwind(|| forja_city::landmark::load_with(&dir, &forja_city::landmark::Library::new(kit), Default::default())) {
                Ok(Ok(l)) => {
                    built += 1;
                    if tx.send((k, Arc::new(l.scene), l.anchor)).is_err() {
                        return;
                    }
                }
                Ok(Err(e)) => log::warn!("landmark {}: {e}", dir.display()),
                Err(_) => log::warn!("landmark {}: its builder panicked", dir.display()),
            }
        }
        log::info!("landmarks: {built} of {total} built in {:.1?}", t.elapsed());
    });
    if spawned.is_err() {
        return;
    }
    app.insert_resource(Landmarks { island: island.clone(), rx, placed: 0, total });
    app.add_system(Stage::PreUpdate, "island_landmarks", |w| {
        w.scope::<Landmarks, _>(|w, st| {
            // One per frame: its upload is a few megabytes.
            let Ok((k, scene, anchor)) = st.rx.try_recv() else { return };
            let Some(site) = st.island.sites.get(k).copied().flatten() else { return };
            let rs = w.res_mut::<RenderScene>();
            let model = rs.add_model(scene);
            rs.strip_after_upload.push(model);
            // The model as a static layer; its shell goes once it is in.
            rs.layer_cmds.push(LayerCmd::Add { id: LANDMARK_LAYER + k as u64, items: vec![RenderItem::new(model, Mat4::from_translation(Vec3::new(anchor.x, site.ground, anchor.y)))], visible: true });
            rs.layer_cmds.push(LayerCmd::HideGroup { group: site.group, hidden: true });
            st.placed += 1;
        });
    });
}

/// Adds the island's run-time systems when the loaded world is an island.
pub fn install(app: &mut App) {
    let Some(island) = app.world.res::<GameWorld>().desc.island.clone() else { return };
    let spawn = app.world.res::<GameWorld>().desc.spawn(SpawnKind::Player).map_or(Vec3::ZERO, |s| s.pos);
    let mut col = IslandColliders { island, loaded: HashMap::new() };
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
