//! Photoreal content shared by the procedural worlds: scanned PBR material
//! sets and street props, loaded from `assets/downloaded` when present.
//! Every world falls back to flat colors when an entry is missing.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use forja_assets::{PbrSet, SceneData};

#[derive(Default)]
pub struct ContentLibrary {
    pub root: PathBuf,
    materials: HashMap<String, PbrSet>,
    props: HashMap<String, Arc<SceneData>>,
}

/// Street props: `(asset folder, file, keep nodes not containing)`.
const PROPS: [(&str, &str, &str); 4] = [
    ("polyhaven_island_tree", "island_tree_01_1k.gltf", ""),
    ("polyhaven_fire_hydrant", "fire_hydrant_1k.gltf", "aged"),
    ("polyhaven_trash_can", "metal_trash_can_1k.gltf", "rust"),
    ("polyhaven_road_barrier", "concrete_road_barrier_1k.gltf", ""),
];

impl ContentLibrary {
    pub fn load(root: &Path) -> Self {
        let mut lib = ContentLibrary { root: root.to_path_buf(), ..Default::default() };
        for id in [
            "pbr_asphalt",
            "pbr_sidewalk",
            "pbr_sand",
            "pbr_grass",
            "pbr_rock",
            "pbr_roof",
            "pbr_facade_brick",
            "pbr_facade_stone",
            "pbr_facade_classic",
            "pbr_facade_office",
            "pbr_facade_glass",
        ] {
            let dir = root.join(id);
            if !dir.exists() {
                continue;
            }
            match forja_assets::load_pbr_set(&dir) {
                Ok(set) => {
                    lib.materials.insert(id.to_string(), set);
                }
                Err(e) => log::warn!("{id}: {e}"),
            }
        }
        for (dir, file, skip) in PROPS {
            let path = root.join(dir).join(file);
            if !path.exists() {
                continue;
            }
            match forja_assets::load_model(&path) {
                Ok(mut scene) => {
                    if !skip.is_empty() {
                        keep_variant(&mut scene, skip);
                    }
                    forja_assets::lod::generate_lods(&mut scene, 8, 2000);
                    lib.props.insert(dir.to_string(), Arc::new(scene));
                }
                Err(e) => log::warn!("{dir}: {e}"),
            }
        }
        log::info!("content library: {} materials, {} props", lib.materials.len(), lib.props.len());
        lib
    }

    pub fn material(&self, id: &str) -> Option<&PbrSet> {
        self.materials.get(id)
    }

    pub fn prop(&self, id: &str) -> Option<Arc<SceneData>> {
        self.props.get(id).cloned()
    }

    pub fn is_empty(&self) -> bool {
        self.materials.is_empty() && self.props.is_empty()
    }
}

/// Drops the alternative variant nodes (e.g. `*_aged`) that Poly Haven ships
/// side by side, and recenters the remaining ones on the origin.
fn keep_variant(scene: &mut SceneData, skip: &str) {
    for n in &mut scene.nodes {
        if n.name.contains(skip) {
            n.mesh = None;
        }
    }
    let b = scene.bounds();
    if b.is_empty() {
        return;
    }
    let shift = forja_math::Vec3::new(-b.center().x, 0.0, -b.center().z);
    for r in scene.roots.clone() {
        scene.nodes[r].local.t += shift;
    }
}
