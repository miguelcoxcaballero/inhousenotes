//! The island at run time: the buildings' colliders follow the camera
//! (a 3x3 block of 256 m tiles around it; the ground is one height field
//! that is always there).

use std::collections::HashMap;
use std::sync::Arc;

use forja_core::{App, Stage};
use forja_math::Vec3;
use forja_physics::{ColliderDesc, ColliderHandle, CollisionGroups, PhysicsWorld, Shape, TriMesh, layers};
use forja_world::{Island, SpawnKind};

use super::RenderScene;
use super::world::GameWorld;

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

/// Adds the island's run-time systems when the loaded world is an island.
pub fn install(app: &mut App) {
    let Some(island) = app.world.res::<GameWorld>().desc.island.clone() else { return };
    let spawn = app.world.res::<GameWorld>().desc.spawn(SpawnKind::Player).map_or(Vec3::ZERO, |s| s.pos);
    let mut col = IslandColliders { island, loaded: HashMap::new() };
    col.update(app.world.res_mut::<PhysicsWorld>(), spawn, usize::MAX);
    app.insert_resource(col);
    app.add_system(Stage::PreUpdate, "island_colliders", |w| {
        let p = w.res::<RenderScene>().camera.position;
        w.scope::<IslandColliders, _>(|w, col| col.update(w.res_mut::<PhysicsWorld>(), p, 1));
    });
}
