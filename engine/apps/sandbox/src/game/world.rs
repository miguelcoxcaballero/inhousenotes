//! Loads the selected map and its static colliders.

use std::sync::Arc;

use forja_core::{App, Plugin};
use forja_physics::PhysicsWorld;
use forja_world::{KitCity, OsmWorld, SpanishCity, TerrainWorld, TestTrack, WorldDescription, WorldSource};

use super::{Hud, MapKind, Paths, RenderItem, RenderScene};

pub struct WorldPlugin {
    pub map: MapKind,
}

/// The loaded world description (spawns, lights, extent).
pub struct GameWorld {
    pub desc: WorldDescription,
    pub kind: MapKind,
}

fn load(map: MapKind, paths: &Paths) -> Result<WorldDescription, forja_world::WorldError> {
    let dl = &paths.downloaded;
    match map {
        MapKind::Spain => SpanishCity::new(dl, paths.data.join("city/styles.toml")).build(),
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
        let (desc, kind) = match load(self.map, &paths) {
            Ok(d) => (d, self.map),
            Err(e) => {
                log::warn!("{}: {e}; falling back to the test track", self.map.label());
                app.world.res_mut::<Hud>().message(format!("{} unavailable ({e}). Run forja_fetch.", self.map.label()));
                (TestTrack::default().build().expect("procedural"), MapKind::Track)
            }
        };
        desc.add_to_physics(app.world.res_mut::<PhysicsWorld>());
        {
            let rs = app.world.res_mut::<RenderScene>();
            let base = rs.models.len();
            for s in &desc.scenes {
                rs.add_model(Arc::clone(s));
            }
            for (s, m) in &desc.placements {
                rs.statics.push(RenderItem::new(base + s, *m));
            }
        }
        let hud = app.world.res_mut::<Hud>();
        hud.map = kind.label().to_string();
        hud.attribution = desc.attribution.clone();
        log::info!("{}: {} placements, {} triangles", kind.label(), desc.placements.len(), desc.triangle_count());
        app.insert_resource(GameWorld { desc, kind });
    }
}
