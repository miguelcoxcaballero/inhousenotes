//! The sandbox game, assembled from independent plugins. Each plugin owns
//! its resources and systems and talks to the others only through the
//! shared resources declared here, so any of them can be replaced.

pub mod camera;
pub mod combat;
pub mod crowd;
pub mod env;
pub mod humans;
pub mod people;
pub mod player;
pub mod props;
pub mod traffic;
pub mod vehicles;
pub mod walkable;
pub mod world;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use forja_assets::SceneData;
use forja_core::{App, Stage};
use forja_math::{Mat4, Vec2, Vec4};
use forja_render::{Camera, Environment, LineVertex, PointLight};

/// Where engine data and downloads live.
#[derive(Clone)]
pub struct Paths {
    pub data: PathBuf,
    pub downloaded: PathBuf,
}

impl Paths {
    pub fn asset(&self, rel: &str) -> PathBuf {
        self.downloaded.join(rel)
    }
}

/// Player intent sampled once per frame from input (or a script) and
/// consumed by the fixed-step systems. One-shot actions are latched until
/// a system takes them.
#[derive(Default, Clone)]
pub struct Intent {
    pub movement: Vec2,
    pub look: Vec2,
    pub sprint: bool,
    pub aim: bool,
    pub fire: bool,
    pub handbrake: bool,
    pub jump: bool,
    pub enter: bool,
    pub reload: bool,
    pub select: Option<forja_gameplay::WeaponCategory>,
    pub holster: bool,
    pub use_item: Option<String>,
    pub drop_item: Option<String>,
    pub equip_item: Option<String>,
}

impl Intent {
    pub fn take(flag: &mut bool) -> bool {
        std::mem::take(flag)
    }
}

#[derive(Clone)]
pub struct RenderItem {
    pub model: usize,
    pub transform: Mat4,
    pub nodes: Option<Arc<Vec<Mat4>>>,
    pub joints: Option<Arc<Vec<Mat4>>>,
    pub tint: Vec4,
}

impl RenderItem {
    pub fn new(model: usize, transform: Mat4) -> Self {
        Self { model, transform, nodes: None, joints: None, tint: Vec4::ONE }
    }
}

/// Everything the renderer needs, filled by the plugins every frame.
#[derive(Default)]
pub struct RenderScene {
    pub models: Vec<Arc<SceneData>>,
    paths: HashMap<PathBuf, Option<usize>>,
    pub statics: Vec<RenderItem>,
    /// Indices of `statics` whose model or transform changes at run time
    /// (traffic signal heads switching aspect). The renderer's static
    /// culling grid is built once from the others; these are submitted
    /// every frame like dynamic items.
    pub volatile_statics: Vec<usize>,
    pub dynamic: Vec<RenderItem>,
    pub lights: Vec<PointLight>,
    /// Physical point and spot lights of this frame (the city at night).
    pub local_lights: Vec<forja_render::Light>,
    pub lines: Vec<LineVertex>,
    pub camera: Camera,
    pub env: Environment,
    /// Equirectangular HDR photo used for image-based lighting.
    pub hdri: Option<PathBuf>,
}

impl RenderScene {
    pub fn add_model(&mut self, scene: Arc<SceneData>) -> usize {
        self.models.push(scene);
        self.models.len() - 1
    }

    /// Loads a model once; `None` (and a warning) when the file is missing.
    pub fn load(&mut self, path: &Path) -> Option<usize> {
        if let Some(i) = self.paths.get(path) {
            return *i;
        }
        let r = match forja_assets::load_model(path) {
            Ok(mut s) => {
                // Scanned assets are dense: build LOD chains. Parts from 1 k
                // triangles get them too: cars (~1.7 k per part, hundreds
                // of instances in the shadow maps) were drawn at full detail
                // at any distance.
                forja_assets::lod::generate_lods(&mut s, 8, 1_000);
                Some(self.add_model(Arc::new(s)))
            }
            Err(e) => {
                log::warn!("{e}");
                None
            }
        };
        self.paths.insert(path.to_path_buf(), r);
        r
    }

    pub fn model(&self, i: usize) -> &SceneData {
        &self.models[i]
    }
}

/// HUD state produced by gameplay, drawn by the UI.
#[derive(Default, Clone)]
pub struct Hud {
    pub map: String,
    pub messages: Vec<(String, f32)>,
    pub speed_kmh: Option<f32>,
    pub gear: i32,
    pub rpm: f32,
    pub health: f32,
    pub armor: f32,
    pub weapon: Option<String>,
    pub ammo: Option<(u32, u32)>,
    pub reloading: Option<f32>,
    pub aiming: bool,
    pub hit_marker: f32,
    pub interact_hint: Option<String>,
    pub physics_ms: f32,
    pub bodies: usize,
    pub attribution: Vec<String>,
}

impl Hud {
    pub fn message(&mut self, text: impl Into<String>) {
        self.messages.push((text.into(), 4.0));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapKind {
    Spain,
    /// The classic grids-and-boulevard map.
    SpainClassic,
    /// Valencia's Barrio del Carmen, procedural.
    Carmen,
    /// Madrid de los Austrias, procedural.
    Austrias,
    /// The real Barrio del Carmen street layout (OpenStreetMap).
    CarmenOsm,
    /// The real Austrias street layout (OpenStreetMap).
    AustriasOsm,
    Track,
    Kit,
    Osm,
    Terrain,
}

impl MapKind {
    pub const ALL: [MapKind; 10] = [MapKind::Spain, MapKind::Carmen, MapKind::Austrias, MapKind::CarmenOsm, MapKind::AustriasOsm, MapKind::SpainClassic, MapKind::Osm, MapKind::Terrain, MapKind::Track, MapKind::Kit];

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "spain" | "espana" | "madrid" | "valencia" => Self::Spain,
            "classic" | "spain-classic" => Self::SpainClassic,
            "carmen" => Self::Carmen,
            "austrias" => Self::Austrias,
            "carmen-osm" | "osm-carmen" => Self::CarmenOsm,
            "austrias-osm" | "osm-austrias" => Self::AustriasOsm,
            "track" => Self::Track,
            "kit" | "city" => Self::Kit,
            "osm" | "miami" => Self::Osm,
            "terrain" | "hills" => Self::Terrain,
            _ => return None,
        })
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Spain => "Madrid y Valencia (procedural)",
            Self::SpainClassic => "Madrid y Valencia, ensanches (procedural)",
            Self::Carmen => "Barrio del Carmen (procedural)",
            Self::Austrias => "Madrid de los Austrias (procedural)",
            Self::CarmenOsm => "Barrio del Carmen (OpenStreetMap)",
            Self::AustriasOsm => "Madrid de los Austrias (OpenStreetMap)",
            Self::Track => "Test track",
            Self::Kit => "Kenney city",
            Self::Osm => "Miami Beach (OpenStreetMap)",
            Self::Terrain => "Hollywood Hills terrain",
        }
    }

    pub fn next(&self) -> Self {
        let i = Self::ALL.iter().position(|m| m == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
}

/// Builds the game for one map.
pub fn build(map: MapKind, paths: Paths) -> App {
    build_at(map, paths, None)
}

/// [`build`] at a clock hour (`None`: the photo sky of the afternoon). The
/// procedural city bakes its night content (lit windows, lamps) for it.
pub fn build_at(map: MapKind, paths: Paths, hour: Option<f32>) -> App {
    let mut app = App::new();
    app.insert_resource(paths.clone())
        .insert_resource(Intent::default())
        .insert_resource(RenderScene { hdri: Some(paths.asset("hdri_wide_street/wide_street_01_2k.hdr")).filter(|p| p.exists()), ..Default::default() })
        .insert_resource(Hud::default())
        .insert_resource(humans::HumanLibrary::default())
        .insert_resource(people::PeopleLibrary::default())
        .add_plugin(forja_physics::PhysicsPlugin::default())
        .add_plugin(world::WorldPlugin { map, hour: hour.and_then(env::content_hour) });
    forja_assets::progress::stage("Cargando gente, coches y tráfico", 0.66);
    app.add_plugin(env::EnvPlugin)
        .add_plugin(props::PropsPlugin)
        .add_plugin(vehicles::VehiclesPlugin)
        .add_plugin(player::PlayerPlugin)
        .add_plugin(combat::CombatPlugin)
        .add_plugin(traffic::TrafficPlugin)
        .add_plugin(crowd::CrowdPlugin)
        .add_plugin(camera::CameraPlugin);
    crate::perf::trim_heap();
    app.add_system(Stage::PreUpdate, "clear_frame", |w| {
        let rs = w.res_mut::<RenderScene>();
        rs.dynamic.clear();
        rs.lights.clear();
        rs.local_lights.clear();
        rs.lines.clear();
        let dt = w.res::<forja_core::Time>().delta;
        let hud = w.res_mut::<Hud>();
        for m in &mut hud.messages {
            m.1 -= dt;
        }
        hud.messages.retain(|m| m.1 > 0.0);
        hud.hit_marker = (hud.hit_marker - dt * 4.0).max(0.0);
    });
    if let Some(h) = hour {
        let t = app.world.res_mut::<env::TimeOfDay>();
        t.hours = h;
        t.photo_sky = false;
    }
    app
}
