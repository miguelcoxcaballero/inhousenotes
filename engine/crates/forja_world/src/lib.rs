//! # forja_world
//!
//! Every map source implements [`WorldSource`] and produces the same
//! [`WorldDescription`]: render scenes with instance transforms, physics
//! colliders and spawn points. The renderer and the physics engine consume
//! the description and never know where the world came from, so any map
//! (real OpenStreetMap data, elevation terrain, a modular kit city, an
//! artist's glTF level or a procedural test track) plugs into the same game.

pub mod builder;
mod gltf_world;
pub mod library;
mod kit;
pub mod osm;
pub mod spain;
pub mod terrain;
mod track;

use std::sync::Arc;

use forja_assets::SceneData;
use forja_math::{Aabb, Iso, Mat4, Vec2};
use forja_physics::{ColliderDesc, PhysicsWorld};

pub use gltf_world::GltfWorld;
pub use kit::KitCity;
pub use library::ContentLibrary;
pub use osm::OsmWorld;
pub use spain::SpanishCity;
pub use terrain::TerrainWorld;
pub use track::TestTrack;

#[derive(Debug)]
pub enum WorldError {
    Io(String),
    Parse(String),
    Missing(String),
}

impl std::fmt::Display for WorldError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorldError::Io(e) => write!(f, "io: {e}"),
            WorldError::Parse(e) => write!(f, "parse: {e}"),
            WorldError::Missing(e) => write!(f, "missing: {e} (run `cargo run -p forja_fetch -- fetch --all`)"),
        }
    }
}

impl std::error::Error for WorldError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpawnKind {
    Player,
    Vehicle,
    Bike,
    Weapon,
    Prop,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpawnPoint {
    pub kind: SpawnKind,
    pub iso: Iso,
}

/// Neutral world format shared by every source.
#[derive(Clone, Default)]
pub struct WorldDescription {
    pub name: String,
    /// Unique render scenes; instanced through `instances`.
    pub scenes: Vec<Arc<SceneData>>,
    /// `(scene index, transform)`.
    pub placements: Vec<(usize, Mat4)>,
    /// Static colliders in world space.
    pub colliders: Vec<(ColliderDesc, Iso)>,
    pub spawns: Vec<SpawnPoint>,
    /// Street lights and other static lamps: `(position, color)`.
    pub lights: Vec<(forja_math::Vec3, forja_math::Vec3)>,
    /// Half size of the playable area (XZ).
    pub extent: Vec2,
    /// Credits required by the data licenses.
    pub attribution: Vec<String>,
    pub stats: Vec<(String, usize)>,
    /// Lanes, signs and traffic lights for traffic AI (procedural cities);
    /// signal placements index `placements`.
    pub traffic: Option<forja_city::traffic::TrafficInfo>,
}

impl WorldDescription {
    pub fn new(name: &str) -> Self {
        Self { name: name.into(), ..Default::default() }
    }

    /// Adds a scene used once at `transform`; returns its index.
    pub fn add_scene(&mut self, scene: Arc<SceneData>, transform: Mat4) -> usize {
        let i = self.scenes.len();
        self.scenes.push(scene);
        self.placements.push((i, transform));
        i
    }

    /// Registers a scene for instancing without placing it.
    pub fn register_scene(&mut self, scene: Arc<SceneData>) -> usize {
        self.scenes.push(scene);
        self.scenes.len() - 1
    }

    pub fn place(&mut self, scene: usize, transform: Mat4) {
        self.placements.push((scene, transform));
    }

    pub fn instances(&self) -> Vec<(usize, &Arc<SceneData>, Mat4)> {
        self.placements.iter().map(|(s, m)| (*s, &self.scenes[*s], *m)).collect()
    }

    pub fn spawn(&self, kind: SpawnKind) -> Option<Iso> {
        self.spawns.iter().find(|s| s.kind == kind).map(|s| s.iso)
    }

    pub fn spawns_of(&self, kind: SpawnKind) -> impl Iterator<Item = &SpawnPoint> {
        self.spawns.iter().filter(move |s| s.kind == kind)
    }

    /// Inserts every static collider into a physics world.
    pub fn add_to_physics(&self, physics: &mut PhysicsWorld) {
        for (desc, iso) in &self.colliders {
            let mut d = desc.clone();
            d.local = iso.mul(&d.local);
            physics.add_collider(d, None);
        }
    }

    pub fn triangle_count(&self) -> usize {
        self.placements.iter().map(|(s, _)| self.scenes[*s].triangle_count()).sum()
    }

    pub fn bounds(&self) -> Aabb {
        let mut b = Aabb::EMPTY;
        for (s, m) in &self.placements {
            let sb = self.scenes[*s].bounds();
            if sb.is_empty() {
                continue;
            }
            let corners = (0..8).map(|i| {
                let p = forja_math::Vec3::new(
                    if i & 1 == 0 { sb.min.x } else { sb.max.x },
                    if i & 2 == 0 { sb.min.y } else { sb.max.y },
                    if i & 4 == 0 { sb.min.z } else { sb.max.z },
                );
                m.transform_point3(p)
            });
            b = b.union(&Aabb::from_points(corners));
        }
        b
    }
}

pub trait WorldSource {
    fn name(&self) -> String;
    fn build(&self) -> Result<WorldDescription, WorldError>;
}
