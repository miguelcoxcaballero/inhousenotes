//! Loads the selected map and its static colliders.

use std::sync::Arc;

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
}

fn load(map: MapKind, paths: &Paths, hour: Option<f32>) -> Result<WorldDescription, forja_world::WorldError> {
    let dl = &paths.downloaded;
    match map {
        MapKind::Spain => SpanishCity { hour, ..SpanishCity::new(dl, paths.data.join("city/styles.toml")) }.build(),
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
        app.insert_resource(GameWorld { desc, kind, scene_base, static_base, night_content: self.hour.is_some() });
    }
}
