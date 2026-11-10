//! The island at run time: the buildings' colliders follow the camera
//! (a 3x3 block of 256 m tiles around it; the ground is one height field
//! that is always there), and so do the landmarks' collision meshes.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use forja_core::{App, Stage};
use forja_math::Vec3;
use forja_physics::{ColliderDesc, ColliderHandle, CollisionGroups, PhysicsWorld, Shape, TriMesh, layers};
use forja_world::{Island, SpawnKind};

use super::RenderScene;
use super::world::GameWorld;

/// Tiles this far (in tiles) from the camera's tile get colliders; they
/// are dropped one tile further out.
const NEAR: i32 = 1;
/// A landmark's collision mesh is built (on a worker) when the camera
/// comes within this distance (m) of it, and dropped [`LANDMARK_KEEP`] away.
const LANDMARK_NEAR: f32 = 220.0;
const LANDMARK_KEEP: f32 = 320.0;

/// Streams the island's building colliders.
pub struct IslandColliders {
    pub island: Arc<Island>,
    loaded: HashMap<usize, ColliderHandle>,
    /// Colliders of the landmarks in `island.landmarks` that are near, and
    /// the meshes being built for those coming near.
    landmarks: HashMap<usize, ColliderHandle>,
    building: HashMap<usize, Receiver<TriMesh>>,
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
        self.update_landmarks(physics, p, false);
    }

    fn add_landmark(&mut self, physics: &mut PhysicsWorld, i: usize, mesh: TriMesh) {
        let desc = ColliderDesc::new(Shape::TriMesh(Arc::new(mesh))).local(self.island.landmarks[i].iso).friction(0.8).groups(CollisionGroups::new(layers::WORLD, u32::MAX));
        self.landmarks.insert(i, physics.add_collider(desc, None));
    }

    /// Landmark colliders around `p`. `wait`: build the near ones now (at
    /// the start, under the loading screen) instead of on a worker.
    fn update_landmarks(&mut self, physics: &mut PhysicsWorld, p: Vec3, wait: bool) {
        let island = self.island.clone();
        for (i, lm) in island.landmarks.iter().enumerate() {
            if lm.collider.is_none() {
                continue;
            }
            let d = lm.distance(p);
            if d > LANDMARK_KEEP {
                self.building.remove(&i);
                if let Some(h) = self.landmarks.remove(&i) {
                    physics.remove_collider(h);
                }
            } else if self.landmarks.contains_key(&i) {
            } else if let Some(rx) = self.building.get(&i) {
                match rx.try_recv() {
                    Ok(mesh) => {
                        self.building.remove(&i);
                        self.add_landmark(physics, i, mesh);
                    }
                    Err(TryRecvError::Empty) => {}
                    Err(TryRecvError::Disconnected) => {
                        self.building.remove(&i);
                    }
                }
            } else if d < LANDMARK_NEAR {
                if wait {
                    if let Some(mesh) = lm.collision_mesh() {
                        self.add_landmark(physics, i, mesh);
                    }
                    continue;
                }
                let (tx, rx) = channel();
                let lm = lm.clone();
                let spawned = std::thread::Builder::new().name("landmark collider".into()).spawn(move || {
                    crate::threads::lower_priority();
                    if let Some(mesh) = lm.collision_mesh() {
                        let _ = tx.send(mesh);
                    }
                });
                if spawned.is_ok() {
                    self.building.insert(i, rx);
                }
            }
        }
    }

    pub fn loaded(&self) -> usize {
        self.loaded.len()
    }

    /// Landmarks with their collision mesh in the physics world.
    pub fn landmarks_loaded(&self) -> usize {
        self.landmarks.len()
    }
}

/// Adds the island's run-time systems when the loaded world is an island.
pub fn install(app: &mut App) {
    let Some(island) = app.world.res::<GameWorld>().desc.island.clone() else { return };
    let spawn = app.world.res::<GameWorld>().desc.spawn(SpawnKind::Player).map_or(Vec3::ZERO, |s| s.pos);
    let mut col = IslandColliders { island, loaded: HashMap::new(), landmarks: HashMap::new(), building: HashMap::new() };
    col.update(app.world.res_mut::<PhysicsWorld>(), spawn, usize::MAX);
    col.update_landmarks(app.world.res_mut::<PhysicsWorld>(), spawn, true);
    app.insert_resource(col);
    app.add_system(Stage::PreUpdate, "island_colliders", |w| {
        let p = w.res::<RenderScene>().camera.position;
        w.scope::<IslandColliders, _>(|w, col| col.update(w.res_mut::<PhysicsWorld>(), p, 1));
    });
}
