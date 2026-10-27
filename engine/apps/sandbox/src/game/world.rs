//! Loads the selected map and its static colliders.

use std::sync::Arc;

use forja_assets::{MeshData, SceneData, TextureData};
use forja_core::{App, Plugin};
use forja_physics::PhysicsWorld;
use forja_world::{KitCity, OsmWorld, SpanishCity, TerrainWorld, TestTrack, WorldDescription, WorldSource};

use super::{Hud, MapKind, Paths, RenderItem, RenderScene};

pub struct WorldPlugin {
    pub map: MapKind,
    /// Clock hour of the night content (`None`: daytime city).
    pub hour: Option<f32>,
}

/// The loaded world description (spawns, lights, extent).
pub struct GameWorld {
    pub desc: WorldDescription,
    pub kind: MapKind,
    /// Render model of `desc.scenes[0]` and render item of
    /// `desc.placements[0]` (to switch traffic light states).
    pub scene_base: usize,
    pub static_base: usize,
    /// Built with the night content (lit windows and lamps).
    pub night_content: bool,
    /// The CPU copies of the world's meshes and textures were dropped
    /// after the GPU upload (see [`strip`]).
    pub released: bool,
}

/// A copy of `s` without its vertex, index and texture data, for scenes
/// the renderer has uploaded: the city's scenes are only drawn, never read
/// back (colliders, traffic and spawns live elsewhere), and keeping them
/// doubled the game's memory. Bounds, nodes and materials stay, so the
/// scene still describes itself. Also returns the mesh bytes dropped.
pub fn strip(s: &SceneData) -> (SceneData, usize) {
    let mut freed = 0;
    let meshes = s
        .meshes
        .iter()
        .map(|prims| {
            prims
                .iter()
                .map(|m| {
                    freed += m.positions.len() * 48 + m.indices.len() * 4 + m.lods.iter().map(|l| l.len() * 4).sum::<usize>();
                    MeshData { material: m.material, aabb: m.aabb, lod_sync: m.lod_sync, shadow: m.shadow, ..Default::default() }
                })
                .collect()
        })
        .collect();
    let textures = s
        .textures
        .iter()
        .map(|t| {
            // Shared buffers: freed once the last scene using them is
            // stripped (not counted here).
            TextureData { name: t.name.clone(), width: t.width, height: t.height, rgba8: Arc::from(Vec::new()), srgb: t.srgb }
        })
        .collect();
    (SceneData { name: s.name.clone(), nodes: s.nodes.clone(), roots: s.roots.clone(), meshes, materials: s.materials.clone(), textures, skins: s.skins.clone(), clips: s.clips.clone() }, freed)
}

fn load(map: MapKind, paths: &Paths, hour: Option<f32>) -> Result<WorldDescription, forja_world::WorldError> {
    let dl = &paths.downloaded;
    let styles = paths.data.join("city/styles.toml");
    let preset = |name: &str| SpanishCity { hour, ..SpanishCity::preset(dl, &styles, name).expect("built-in plan") }.build();
    // Real tiles: map.json under osm/, or forja_fetch's map.osm.
    let osm = |tile: &str, id: &str| {
        let json = dl.join("osm").join(format!("{tile}.json"));
        let path = if json.exists() { json } else { dl.join(id).join("map.osm") };
        SpanishCity { hour, ..SpanishCity::from_osm(Some(dl), Some(&styles), &path)? }.build()
    };
    match map {
        MapKind::Spain => SpanishCity { hour, ..SpanishCity::new(dl, &styles) }.build(),
        MapKind::SpainClassic => preset("classic"),
        MapKind::Carmen => preset("carmen"),
        MapKind::Austrias => preset("austrias"),
        MapKind::CarmenOsm => osm("carmen", "map_valencia_carmen_osm"),
        MapKind::AustriasOsm => osm("austrias", "map_madrid_austrias_osm"),
        MapKind::Track => TestTrack::default().build(),
        MapKind::Kit => KitCity::new(dl).build(),
        MapKind::Osm => OsmWorld::new(dl.join("map_miami_beach_osm/map.osm")).with_content(dl).build(),
        MapKind::Terrain => TerrainWorld::from_dir(&dl.join("map_hollywood_hills_terrain"), 12, 2, 34.12).build(),
    }
}

impl Plugin for WorldPlugin {
    fn name(&self) -> &'static str {
        "world"
    }

    fn dependencies(&self) -> &[&'static str] {
        &["forja_physics"]
    }

    fn build(&self, app: &mut App) {
        let paths = app.world.res::<Paths>().clone();
        let (desc, kind) = match load(self.map, &paths, self.hour) {
            Ok(d) => (d, self.map),
            Err(e) => {
                log::warn!("{}: {e}; falling back to the test track", self.map.label());
                app.world.res_mut::<Hud>().message(format!("{} unavailable ({e}). Run forja_fetch.", self.map.label()));
                (TestTrack::default().build().expect("procedural"), MapKind::Track)
            }
        };
        desc.add_to_physics(app.world.res_mut::<PhysicsWorld>());
        let (scene_base, static_base) = {
            let rs = app.world.res_mut::<RenderScene>();
            let base = rs.models.len();
            let static_base = rs.statics.len();
            for s in &desc.scenes {
                rs.add_model(Arc::clone(s));
            }
            for (s, m) in &desc.placements {
                rs.statics.push(RenderItem::new(base + s, *m));
            }
            (base, static_base)
        };
        let hud = app.world.res_mut::<Hud>();
        hud.map = kind.label().to_string();
        hud.attribution = desc.attribution.clone();
        log::info!("{}: {} placements, {} triangles", kind.label(), desc.placements.len(), desc.triangle_count());
        app.insert_resource(GameWorld { desc, kind, scene_base, static_base, night_content: self.hour.is_some(), released: false });
    }
}
