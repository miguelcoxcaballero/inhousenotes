//! Any glTF/GLB scene as a level: rendered as is, collision from its triangles.

use std::path::PathBuf;
use std::sync::Arc;

use forja_math::{Iso, Mat4, Vec2, Vec3};
use forja_physics::{ColliderDesc, CollisionGroups, Shape, TriMesh, layers};

use crate::{SpawnKind, SpawnPoint, WorldDescription, WorldError, WorldSource};

pub struct GltfWorld {
    pub path: PathBuf,
    pub scale: f32,
}

impl GltfWorld {
    pub fn new(path: impl Into<PathBuf>, scale: f32) -> Self {
        Self { path: path.into(), scale }
    }
}

impl WorldSource for GltfWorld {
    fn name(&self) -> String {
        format!("glTF {}", self.path.display())
    }

    fn build(&self) -> Result<WorldDescription, WorldError> {
        if !self.path.exists() {
            return Err(WorldError::Missing(self.path.display().to_string()));
        }
        let scene = forja_assets::load_model(&self.path).map_err(|e| WorldError::Parse(e.to_string()))?;
        let (verts, tris) = scene.world_triangles();
        let verts: Vec<Vec3> = verts.into_iter().map(|v| v * self.scale).collect();
        let bounds = forja_math::Aabb::from_points(verts.iter().copied());
        let mut w = WorldDescription::new(&scene.name.clone());
        w.add_scene(Arc::new(scene), Mat4::from_scale(Vec3::splat(self.scale)));
        if !tris.is_empty() {
            w.colliders.push((
                ColliderDesc::new(Shape::TriMesh(Arc::new(TriMesh::new(verts, tris)))).groups(CollisionGroups::new(layers::WORLD, u32::MAX)),
                Iso::IDENTITY,
            ));
        }
        w.spawns.push(SpawnPoint { kind: SpawnKind::Player, iso: Iso::from_pos(Vec3::new(bounds.center().x, bounds.max.y + 2.0, bounds.center().z)) });
        w.extent = Vec2::new(bounds.half_extents().x, bounds.half_extents().z);
        Ok(w)
    }
}
