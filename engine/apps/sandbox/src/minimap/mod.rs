//! Radar (minimap) and pause map, after GTA V's.
//!
//! - [`data`]: what a world gives the map ([`MapData`], [`MapSource`]).
//! - [`raster`]: the map painted once into cached texture tiles.
//! - [`route`]: the GPS (A* on the road graph).
//! - [`view`]: the ground-to-screen projection (rotation, tilt).
//! - here: the [`Minimap`] resource (waypoint, route, blips, zoom) and the
//!   HUD radar; [`page`] draws the pause menu's map.
//!
//! The radar sits bottom-left over the health and armour bars. It turns
//! with the camera, is flat on foot and tilted in a vehicle, zooms out with
//! speed, and shows the road route to the waypoint set on the pause map.
//! Tapping the radar key (Z) zooms it out for a few seconds; holding it
//! shows the big square radar.

pub mod data;
pub mod page;
pub mod raster;
pub mod route;
pub mod view;

use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::Instant;

use egui::{Color32, Pos2, Rect, Stroke, pos2, vec2};
use forja_core::{App, Plugin, World};
use forja_math::{Vec2, Vec3};

use crate::game::player::Player;
use crate::game::vehicles::{Driving, VehicleVisuals};
use crate::game::world::GameWorld;
use crate::game::{Hud, MapKind, Paths, RenderScene};
pub use data::{IslandFrame, MapData, MapSource, Poi, PoiKind};
use raster::{MapIndex, TILE, TileKey, Tiles};
use route::{RoadGraph, Route};
pub use view::RadarView;

pub const WAYPOINT_COLOR: Color32 = Color32::from_rgb(168, 96, 232);
pub const MISSION_COLOR: Color32 = Color32::from_rgb(240, 200, 72);
const HEALTH_COLOR: Color32 = Color32::from_rgb(92, 178, 96);
const ARMOR_COLOR: Color32 = Color32::from_rgb(78, 150, 214);

/// How far from the road route the player may stray before the GPS
/// recalculates.
const OFF_ROUTE: f32 = 38.0;
/// How long the radar key may be down and still count as a tap.
const TAP: f32 = 0.28;

/// The player's radar settings (Opciones > Interfaz).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RadarOptions {
    pub show: bool,
    /// Size factor (1: about a fifth of the screen height).
    pub size: f32,
    /// The radar key is down.
    pub key_held: bool,
    /// Nothing of the HUD's corner is drawn (pause menu, cutscene).
    pub hidden: bool,
}

impl Default for RadarOptions {
    fn default() -> Self {
        Self { show: true, size: 1.0, key_held: false, hidden: false }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BlipKind {
    Poi(PoiKind),
    /// A vehicle of the player's; `owned`: the one last driven.
    Vehicle { owned: bool },
    /// Flashes red and blue.
    Police,
    Waypoint,
    Mission,
    /// A plain dot.
    Dot(Color32),
}

/// A marker on the radar and the map.
#[derive(Clone, Debug, PartialEq)]
pub struct Blip {
    pub kind: BlipKind,
    pub pos: Vec2,
    /// Height, when it matters: a small arrow shows it is above or below.
    pub height: Option<f32>,
    /// Stays on the radar's border when out of range.
    pub edge: bool,
    pub name: String,
}

/// The state the HUD radar reads from the game each frame.
#[derive(Clone, Debug)]
pub struct Scene {
    pub pos: Vec3,
    /// Unit ground direction the player (or the driven vehicle) faces.
    pub heading: Vec2,
    /// Unit ground direction the camera looks along.
    pub camera: Vec2,
    pub driving: Option<usize>,
    /// Metres per second.
    pub speed: f32,
    pub health: f32,
    pub armor: f32,
    /// The player's vehicles: `(index, position, is a police car)`.
    pub vehicles: Vec<(usize, Vec3, bool)>,
}

impl Scene {
    pub fn ground(&self) -> Vec2 {
        Vec2::new(self.pos.x, self.pos.z)
    }

    /// Reads the player, the driven vehicle and the camera.
    pub fn gather(world: &World, last_camera: Vec2) -> Scene {
        let player = world.res::<Player>();
        let driving = world.resources.get::<Driving>().and_then(|d| d.0);
        let physics = world.res::<forja_physics::PhysicsWorld>();
        let mut pos = player.position();
        let mut heading = Vec2::new(player.facing.sin(), player.facing.cos());
        let mut speed = player.controller.velocity.length();
        let mut vehicles = Vec::new();
        if let Some(list) = world.resources.get::<forja_physics::Vehicles>() {
            let visuals = world.resources.get::<VehicleVisuals>();
            for (i, v) in list.0.iter().enumerate() {
                let Some(b) = physics.body(v.body) else { continue };
                let iso = b.iso();
                if driving == Some(i) {
                    let f = iso.rot * Vec3::Z;
                    pos = iso.pos;
                    heading = Vec2::new(f.x, f.z).try_normalize().unwrap_or(heading);
                    speed = b.linvel.length();
                }
                let police = visuals.and_then(|vv| vv.0.get(i)).is_some_and(|vv| vv.name.to_ascii_lowercase().contains("police"));
                vehicles.push((i, iso.pos, police));
            }
        }
        let f = world.res::<RenderScene>().camera.forward();
        let camera = Vec2::new(f.x, f.z).try_normalize().unwrap_or(last_camera);
        let hud = world.res::<Hud>();
        Scene { pos, heading, camera, driving, speed, health: hud.health, armor: hud.armor, vehicles }
    }
}

/// Radar state of the running world.
pub struct Minimap {
    pub map: Arc<MapData>,
    pub graph: Arc<RoadGraph>,
    tiles: Tiles,
    /// The player's own destination (set on the pause map).
    pub waypoint: Option<Vec2>,
    /// A mission's destination: yellow blip and route.
    pub mission: Option<Vec2>,
    /// Wanted level: police blips flash while it is above zero.
    pub wanted: u8,
    /// Police units after the player (shown while wanted).
    pub police: Vec<Vec3>,
    /// Markers added by other systems.
    pub blips: Vec<Blip>,
    /// The vehicle the player last drove.
    pub owned_vehicle: Option<usize>,
    route: Option<Route>,
    /// What `route` leads to: the point and whether it is the mission's.
    goal: Option<(Vec2, bool)>,
    pending: Option<Receiver<(u64, Option<Route>)>>,
    request: u64,
    /// Seconds until the GPS may recalculate again.
    retry: f32,
    /// Route distance left, for the pause map.
    pub remaining: Option<f32>,
    /// Smoothed metres across the radar (`None`: not drawn yet).
    span: Option<f32>,
    /// 0 on foot, 1 in a vehicle.
    vehicle: f32,
    /// 0 the radar, 1 the big square one.
    big: f32,
    key_time: f32,
    was_held: bool,
    /// Seconds left of the zoom-out after a tap.
    zoom_out: f32,
    last_camera: Vec2,
    /// Paint tiles and compute routes on the calling thread (captures).
    sync: bool,
    /// CPU time of the radar: smoothed, and of the last frame (ms).
    pub cpu_ms: f32,
    pub last_ms: f32,
    page: page::PageState,
}

impl Minimap {
    pub fn new(map: MapData, noded: bool) -> Minimap {
        let t = Instant::now();
        let graph = Arc::new(RoadGraph::build(&map, noded));
        let map = Arc::new(map);
        let index = Arc::new(MapIndex::new(Arc::clone(&map)));
        log::info!("minimap: {} ({} areas, {} roads, {} places; GPS {} junctions, {} stretches) in {:.2?}", map.name, map.areas.len(), map.roads.len(), map.pois.len(), graph.node_count(), graph.edge_count(), t.elapsed());
        Minimap {
            map,
            graph,
            tiles: Tiles::new(index),
            waypoint: None,
            mission: None,
            wanted: 0,
            police: Vec::new(),
            blips: Vec::new(),
            owned_vehicle: None,
            route: None,
            goal: None,
            pending: None,
            request: 0,
            retry: 0.0,
            remaining: None,
            span: None,
            vehicle: 0.0,
            big: 0.0,
            key_time: 0.0,
            was_held: false,
            zoom_out: 0.0,
            last_camera: view::NORTH,
            sync: false,
            cpu_ms: 0.0,
            last_ms: 0.0,
            page: page::PageState::default(),
        }
    }

    /// Tiles and routes are computed at once on the calling thread
    /// (headless captures, where no later frame would pick them up).
    pub fn set_sync(&mut self, sync: bool) {
        self.sync = sync;
        self.tiles.sync = sync;
    }

    /// Captures: the radar as it is after holding the key (`big`) or after
    /// a tap (`zoomed`), and the pause map's zoom in points per metre.
    pub fn preset(&mut self, big: bool, zoomed: bool, map_zoom: Option<f32>) {
        if big {
            self.key_time = 1.0;
            self.big = 1.0;
        }
        if zoomed {
            self.zoom_out = 6.0;
        }
        self.page.preset = map_zoom;
    }

    pub fn route(&self) -> Option<&Route> {
        self.route.as_ref()
    }

    /// The destination the GPS leads to and whether it is the mission's.
    pub fn goal(&self) -> Option<(Vec2, bool)> {
        self.waypoint.map(|w| (w, false)).or(self.mission.map(|m| (m, true)))
    }

    pub fn tiles_loaded(&self) -> (usize, usize) {
        (self.tiles.loaded(), self.tiles.waiting())
    }

    fn ask_route(&mut self, from: Vec2, to: Vec2) {
        self.request += 1;
        if self.sync {
            self.route = self.graph.route(from, to, 600.0);
            self.pending = None;
            return;
        }
        let (tx, rx) = channel();
        let (graph, id) = (Arc::clone(&self.graph), self.request);
        let spawned = std::thread::Builder::new().name("minimap-gps".into()).spawn(move || {
            let _ = tx.send((id, graph.route(from, to, 600.0)));
        });
        self.pending = spawned.is_ok().then_some(rx);
    }

    /// Keeps the route to the current destination: asks for it when the
    /// destination changes, follows the player along it, recalculates when
    /// the player leaves it and clears the waypoint on arrival.
    pub fn update_route(&mut self, player: Vec2, dt: f32) {
        self.retry = (self.retry - dt).max(0.0);
        let goal = self.goal();
        if goal != self.goal {
            self.goal = goal;
            self.route = None;
            self.pending = None;
            self.remaining = None;
            if let Some((to, _)) = goal {
                self.ask_route(player, to);
                self.retry = 1.0;
            }
        }
        if let Some(rx) = &self.pending {
            match rx.try_recv() {
                Ok((id, route)) => {
                    self.pending = None;
                    if id == self.request {
                        self.route = route;
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.pending = None,
                Err(_) => {}
            }
        }
        let Some((to, mission)) = self.goal else { return };
        match &mut self.route {
            Some(route) => {
                let (along, off) = route.track(player);
                let left = route.remaining(along);
                self.remaining = Some(left);
                if left < 14.0 && off < 25.0 && !mission {
                    // Arrived: the waypoint goes, as in GTA V.
                    self.waypoint = None;
                } else if off > OFF_ROUTE && self.pending.is_none() && self.retry <= 0.0 {
                    self.ask_route(player, to);
                    self.retry = 1.5;
                }
            }
            None => {
                self.remaining = None;
                if player.distance(to) < 14.0 && !mission {
                    self.waypoint = None;
                } else if self.pending.is_none() && self.retry <= 0.0 {
                    // No way found (no road nearby): try again as the
                    // player moves.
                    self.ask_route(player, to);
                    self.retry = 3.0;
                }
            }
        }
    }

    /// Markers of the world: the map's places, the player's vehicles,
    /// police while wanted, the waypoint and the mission.
    /// `near`: only the places within that distance of a point.
    fn collect_blips(&self, scene: &Scene, all_places: bool, near: Option<(Vec2, f32)>, out: &mut Vec<Blip>) {
        for p in &self.map.pois {
            if (all_places || p.kind.on_radar()) && near.is_none_or(|(c, d)| p.pos.distance_squared(c) < d * d) {
                out.push(Blip { kind: BlipKind::Poi(p.kind), pos: p.pos, height: p.height, edge: false, name: p.name.clone() });
            }
        }
        for (i, pos, police) in &scene.vehicles {
            if scene.driving == Some(*i) {
                continue;
            }
            let ground = Vec2::new(pos.x, pos.z);
            if *police && self.wanted > 0 {
                out.push(Blip { kind: BlipKind::Police, pos: ground, height: Some(pos.y), edge: false, name: "Policía".into() });
            } else {
                let owned = self.owned_vehicle == Some(*i);
                out.push(Blip { kind: BlipKind::Vehicle { owned }, pos: ground, height: Some(pos.y), edge: owned, name: if owned { "Tu vehículo".into() } else { "Vehículo".into() } });
            }
        }
        if self.wanted > 0 {
            for p in &self.police {
                out.push(Blip { kind: BlipKind::Police, pos: Vec2::new(p.x, p.z), height: Some(p.y), edge: false, name: "Policía".into() });
            }
        }
        out.extend(self.blips.iter().cloned());
        if let Some(m) = self.mission {
            out.push(Blip { kind: BlipKind::Mission, pos: m, height: None, edge: true, name: "Misión".into() });
        }
        if let Some(w) = self.waypoint {
            out.push(Blip { kind: BlipKind::Waypoint, pos: w, height: None, edge: true, name: "Destino".into() });
        }
    }
}

/// Builds the world's [`Minimap`]: the island file under the Isla Levante
/// map, the world's own streets and roofs elsewhere. A world that inserts
/// its own [`MapData`] resource before this plugin is used as is.
pub struct MinimapPlugin;

impl Plugin for MinimapPlugin {
    fn name(&self) -> &'static str {
        "minimap"
    }

    fn dependencies(&self) -> &[&'static str] {
        &["world"]
    }

    fn build(&self, app: &mut App) {
        let given = app.world.resources.remove::<MapData>();
        let (map, noded) = match given {
            Some(m) => (m, true),
            None => {
                let gw = app.world.res::<GameWorld>();
                let file = app.world.res::<Paths>().data.join("maps/isla/minimap.bin");
                // A world placed elsewhere on the island says so with an
                // `IslandFrame` resource.
                let frame = app.world.resources.get::<IslandFrame>().copied();
                let island = (gw.kind == MapKind::Isla || frame.is_some()).then(|| MapData::from_island_file(&file, frame.unwrap_or(IslandFrame::VALENCIA)));
                match island {
                    Some(Ok(m)) => (m, true),
                    other => {
                        if let Some(Err(e)) = other {
                            log::warn!("minimap: {e}; drawing the streets of the built world only");
                        }
                        (gw.desc.map_data(), false)
                    }
                }
            }
        };
        app.insert_resource(Minimap::new(map, noded));
    }
}

/// The radar's rectangle and view for this frame.
struct Layout {
    rect: Rect,
    view: RadarView,
    bars: Rect,
    radius: f32,
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

impl Minimap {
    /// Advances the zoom, tilt and big-radar animations and lays the radar
    /// out on `screen`.
    fn layout(&mut self, screen: Rect, scene: &Scene, opts: &RadarOptions, dt: f32) -> Layout {
        // Tap: zoom out for a while (again: back). Hold: the big radar.
        if opts.key_held {
            self.key_time += dt;
        } else {
            if self.was_held && self.key_time <= TAP {
                self.zoom_out = if self.zoom_out > 0.0 { 0.0 } else { 6.0 };
            }
            self.key_time = 0.0;
        }
        self.was_held = opts.key_held;
        self.zoom_out = (self.zoom_out - dt).max(0.0);
        let want_big = opts.key_held && self.key_time > TAP;
        let first = self.span.is_none();
        let rate = |k: f32| if first { 1.0 } else { 1.0 - (-k * dt).exp() };
        self.big += ((want_big as u8 as f32) - self.big) * rate(12.0);
        self.vehicle += ((scene.driving.is_some() as u8 as f32) - self.vehicle) * rate(5.0);
        let (big, veh) = (smooth(self.big.clamp(0.0, 1.0)), smooth(self.vehicle.clamp(0.0, 1.0)));
        // Metres across the radar: closer on foot, wider with speed.
        let fast = (scene.speed / 38.0).clamp(0.0, 1.0);
        let mut want = lerp(150.0, 220.0 + 330.0 * fast, veh);
        if self.zoom_out > 0.0 {
            want *= 2.3;
        }
        want = lerp(want, 950.0 + 500.0 * fast, big);
        let span = match self.span {
            Some(s) => s + (want - s) * rate(if want_big || self.big > 0.02 { 12.0 } else { 3.0 }),
            None => want,
        };
        self.span = Some(span);
        let size = opts.size.clamp(0.5, 2.0);
        let h = (screen.height() * 0.185).clamp(96.0, 320.0) * size;
        let w = h * 1.55;
        let side = (screen.height() * 0.5).min(screen.width() * 0.42) * size.clamp(0.8, 1.25);
        let (w, h) = (lerp(w, side, big), lerp(h, side, big));
        let margin = (screen.width() * 0.012).clamp(12.0, 28.0);
        let bar_h = (h * 0.05).clamp(5.0, 9.0);
        let bottom = screen.bottom() - margin - bar_h - 3.0;
        let rect = Rect::from_min_size(pos2(screen.left() + margin, bottom - h), vec2(w, h));
        let bars = Rect::from_min_size(pos2(rect.left(), rect.bottom() + 3.0), vec2(w, bar_h));
        let anchor_y = lerp(lerp(0.60, 0.74, veh), 0.5, big);
        let view = RadarView { rect, anchor: pos2(rect.center().x, rect.top() + h * anchor_y), centre: scene.ground(), forward: scene.camera, scale: w / span, tilt: lerp(0.62 * veh, 0.0, big) };
        Layout { rect, view, bars, radius: (h * 0.045).clamp(4.0, 12.0) }
    }
}

/// Textured quads of the map tiles under `view`, with `alpha`.
/// A tilted view bends them, so its tiles are subdivided.
fn tile_meshes(tiles: &mut Tiles, ctx: &egui::Context, view: &RadarView, alpha: u8, out: &mut Vec<egui::Mesh>) {
    let ppp = ctx.pixels_per_point();
    // A tilted view shrinks the ground ahead: a coarser level there.
    let mut level = raster::level_for(view.metres_per_point() / ppp * (1.0 + 0.45 * view.tilt));
    let (lo, hi) = view.ground_bounds();
    // Never more than a few dozen tiles: a huge view steps up a level.
    let range = |level: u8| {
        let s = TileKey { level, x: 0, y: 0 }.size();
        ((lo.x / s).floor() as i32, (lo.y / s).floor() as i32, (hi.x / s).floor() as i32, (hi.y / s).floor() as i32)
    };
    while level + 1 < raster::LEVELS {
        let (x0, y0, x1, y1) = range(level);
        if (x1 - x0 + 1) as i64 * (y1 - y0 + 1) as i64 <= 96 {
            break;
        }
        level += 1;
    }
    let (x0, y0, x1, y1) = range(level);
    if (x1 - x0 + 1) as i64 * (y1 - y0 + 1) as i64 > 400 {
        return;
    }
    let grid = if view.tilt > 0.01 { 4 } else { 1 };
    let color = Color32::from_rgba_unmultiplied(255, 255, 255, alpha);
    let clip = view.rect.expand(2.0);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let key = TileKey { level, x, y };
            let (origin, size) = (key.origin(), key.size());
            let corners = [origin, origin + Vec2::new(size, 0.0), origin + Vec2::splat(size), origin + Vec2::new(0.0, size)].map(|c| view.project(c));
            let bb = Rect::from_points(&corners);
            if !bb.intersects(clip) {
                continue;
            }
            let edge = 1.0 / TILE as f32;
            let (texture, uv) = match tiles.get(ctx, key) {
                Some(t) => (t, [edge, edge, 1.0 - edge, 1.0 - edge]),
                None => match tiles.fallback(key) {
                    Some(f) => f,
                    None => continue,
                },
            };
            let mut mesh = egui::Mesh::with_texture(texture);
            for j in 0..=grid {
                for i in 0..=grid {
                    let (fx, fy) = (i as f32 / grid as f32, j as f32 / grid as f32);
                    let p = view.project(origin + Vec2::new(fx, fy) * size);
                    mesh.vertices.push(egui::epaint::Vertex { pos: p, uv: pos2(lerp(uv[0], uv[2], fx), lerp(uv[1], uv[3], fy)), color });
                }
            }
            let n = grid as u32 + 1;
            for j in 0..grid as u32 {
                for i in 0..grid as u32 {
                    let a = j * n + i;
                    mesh.indices.extend_from_slice(&[a, a + 1, a + n + 1, a, a + n + 1, a + n]);
                }
            }
            out.push(mesh);
        }
    }
}

/// The route from the player on, as a screen polyline (at most `reach`
/// metres of it).
fn route_points(route: &Route, view: &RadarView, from: Option<Vec2>, reach: f32, out: &mut Vec<Pos2>) {
    let start = route.cursor();
    let base = route.cum.get(start).copied().unwrap_or(0.0);
    if let Some(p) = from {
        out.push(view.project(p));
    }
    for i in start + usize::from(from.is_some())..route.pts.len() {
        out.push(view.project(route.pts[i]));
        if route.cum[i] - base > reach {
            break;
        }
    }
}

/// The player's arrow: white, pointing along `dir` (screen direction).
pub fn draw_arrow(p: &egui::Painter, at: Pos2, dir: egui::Vec2, size: f32) {
    let side = vec2(-dir.y, dir.x);
    let pt = |f: f32, s: f32| at + dir * f * size + side * s * size;
    let (tip, right, notch, left) = (pt(1.0, 0.0), pt(-0.8, 0.72), pt(-0.38, 0.0), pt(-0.8, -0.72));
    let mut mesh = egui::Mesh::default();
    for v in [tip, right, notch, left] {
        mesh.colored_vertex(v, Color32::WHITE);
    }
    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    p.add(egui::Shape::closed_line(vec![tip, right, notch, left], Stroke::new((size * 0.34).max(2.0), Color32::from_black_alpha(210))));
    p.add(egui::Shape::mesh(mesh));
}

/// Fill colour and letter of a place's icon.
pub fn poi_style(kind: PoiKind) -> (Color32, &'static str) {
    match kind {
        PoiKind::Landmark => (Color32::from_rgb(236, 190, 70), ""),
        PoiKind::Hospital => (Color32::from_rgb(214, 54, 54), ""),
        PoiKind::Police => (Color32::from_rgb(52, 98, 210), "P"),
        PoiKind::Shop => (Color32::from_rgb(62, 150, 84), "S"),
        PoiKind::Fuel => (Color32::from_rgb(224, 124, 40), "G"),
        PoiKind::Station => (Color32::from_rgb(40, 132, 140), "T"),
        PoiKind::Airport => (Color32::from_rgb(70, 110, 190), "A"),
        PoiKind::Church => (Color32::from_rgb(132, 104, 82), "I"),
        PoiKind::Museum => (Color32::from_rgb(150, 82, 160), "M"),
        PoiKind::School => (Color32::from_rgb(96, 110, 126), "U"),
        PoiKind::TownHall => (Color32::from_rgb(120, 60, 60), "C"),
        PoiKind::Stadium => (Color32::from_rgb(72, 128, 60), "E"),
        PoiKind::Culture => (Color32::from_rgb(170, 70, 110), "O"),
    }
}

/// One marker; `r` is its radius in points, `time` drives the flashing.
pub fn draw_blip(p: &egui::Painter, at: Pos2, kind: BlipKind, r: f32, time: f64) {
    let outline = Stroke::new((r * 0.24).max(1.0), Color32::from_black_alpha(220));
    match kind {
        BlipKind::Poi(PoiKind::Landmark) => {
            // A five-pointed star.
            let pts: Vec<Pos2> = (0..10).map(|i| {
                let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
                at + vec2(a.cos(), a.sin()) * if i % 2 == 0 { r * 1.25 } else { r * 0.55 }
            }).collect();
            let mut mesh = egui::Mesh::default();
            mesh.colored_vertex(at, poi_style(PoiKind::Landmark).0);
            for q in &pts {
                mesh.colored_vertex(*q, poi_style(PoiKind::Landmark).0);
            }
            for i in 0..10u32 {
                mesh.indices.extend_from_slice(&[0, 1 + i, 1 + (i + 1) % 10]);
            }
            p.add(egui::Shape::closed_line(pts, outline));
            p.add(egui::Shape::mesh(mesh));
        }
        BlipKind::Poi(PoiKind::Hospital) => {
            p.circle(at, r, poi_style(PoiKind::Hospital).0, outline);
            let (a, b) = (r * 0.62, r * 0.2);
            p.rect_filled(Rect::from_center_size(at, vec2(a * 2.0, b * 2.0)), 0.0, Color32::WHITE);
            p.rect_filled(Rect::from_center_size(at, vec2(b * 2.0, a * 2.0)), 0.0, Color32::WHITE);
        }
        BlipKind::Poi(kind) => {
            let (fill, letter) = poi_style(kind);
            p.rect(Rect::from_center_size(at, vec2(r * 1.9, r * 1.9)), r * 0.45, fill, outline, egui::StrokeKind::Outside);
            p.text(at, egui::Align2::CENTER_CENTER, letter, egui::FontId::proportional(r * 1.55), Color32::WHITE);
        }
        BlipKind::Vehicle { owned } => {
            // A car seen from above.
            let fill = if owned { Color32::WHITE } else { Color32::from_gray(176) };
            let s = if owned { r } else { r * 0.72 };
            let body = Rect::from_center_size(at, vec2(s * 1.25, s * 2.1));
            p.rect(body, s * 0.42, fill, outline, egui::StrokeKind::Outside);
            p.rect_filled(Rect::from_center_size(at - vec2(0.0, s * 0.18), vec2(s * 0.85, s * 0.62)), s * 0.15, Color32::from_black_alpha(170));
        }
        BlipKind::Police => {
            let red = (time * 3.3).fract() < 0.5;
            let fill = if red { Color32::from_rgb(226, 44, 44) } else { Color32::from_rgb(52, 110, 240) };
            p.circle(at, r * 0.8, fill, Stroke::new((r * 0.24).max(1.0), Color32::WHITE));
        }
        BlipKind::Waypoint | BlipKind::Mission => {
            let fill = if kind == BlipKind::Waypoint { WAYPOINT_COLOR } else { MISSION_COLOR };
            p.circle(at, r * 1.05, fill, Stroke::new((r * 0.3).max(1.5), Color32::WHITE));
            p.circle_filled(at, r * 0.36, Color32::WHITE);
        }
        BlipKind::Dot(c) => {
            p.circle(at, r * 0.7, c, outline);
        }
    }
}

/// The small arrow that says a marker is above (`up`) or below.
fn draw_height(p: &egui::Painter, at: Pos2, r: f32, up: bool) {
    let c = at + vec2(r * 1.25, -r * 0.9);
    let s = r * 0.55;
    let d = if up { -1.0 } else { 1.0 };
    let pts = vec![c + vec2(0.0, d * s), c + vec2(s, -d * s * 0.6), c + vec2(-s, -d * s * 0.6)];
    p.add(egui::Shape::convex_polygon(pts, Color32::WHITE, Stroke::new(1.0, Color32::from_black_alpha(220))));
}

/// Draws the markers of `blips` on `view`: those marked `edge` slide to
/// the border of `inner` when out of range. Returns the marker under
/// `hover`, if any.
#[allow(clippy::too_many_arguments)]
fn draw_blips(p: &egui::Painter, view: &RadarView, inner: Rect, blips: &[Blip], player_y: Option<f32>, r: f32, time: f64, hover: Option<Pos2>) -> Option<usize> {
    let mut hit: Option<(f32, usize)> = None;
    for (i, b) in blips.iter().enumerate() {
        let s = view.project(b.pos);
        let (at, clamped) = if b.edge { view::clamp_to_rect(inner, view.anchor, s) } else { (s, false) };
        if !clamped && !inner.expand(r).contains(at) {
            continue;
        }
        let big = matches!(b.kind, BlipKind::Waypoint | BlipKind::Mission | BlipKind::Vehicle { owned: true } | BlipKind::Police);
        let size = if big { r } else { r * 0.86 };
        draw_blip(p, at, b.kind, size, time);
        if let (Some(py), Some(h)) = (player_y, b.height)
            && (h - py).abs() > 3.5
        {
            draw_height(p, at, size, h > py);
        }
        if let Some(m) = hover {
            let d = (m - at).length();
            if d < size * 1.6 && hit.is_none_or(|h| d < h.0) {
                hit = Some((d, i));
            }
        }
    }
    hit.map(|h| h.1)
}

/// Health (green, left half) and armour (blue, right half) under the
/// radar, as in GTA V.
fn draw_bars(p: &egui::Painter, bars: Rect, health: f32, armor: f32) {
    let gap = 3.0;
    let half = (bars.width() - gap) * 0.5;
    let bar = |x: f32, frac: f32, color: Color32| {
        let r = Rect::from_min_size(pos2(x, bars.top()), vec2(half, bars.height()));
        p.rect_filled(r, 1.5, Color32::from_black_alpha(170));
        let back = Color32::from_rgba_unmultiplied(color.r() / 3, color.g() / 3, color.b() / 3, 200);
        p.rect_filled(r.shrink(1.0), 1.0, back);
        let f = frac.clamp(0.0, 1.0);
        if f > 0.0 {
            let fill = Rect::from_min_size(r.min + vec2(1.0, 1.0), vec2((half - 2.0) * f, bars.height() - 2.0));
            p.rect_filled(fill, 1.0, color);
        }
    };
    // Low health turns red.
    let hc = if health < 25.0 { Color32::from_rgb(200, 60, 52) } else { HEALTH_COLOR };
    bar(bars.left(), health / 100.0, hc);
    bar(bars.left() + half + gap, armor / 100.0, ARMOR_COLOR);
}

/// Draws the HUD radar with the health and armour bars. Returns `false`
/// when the radar is switched off (the caller then draws plain bars);
/// in the pause menu and in cutscenes nothing is drawn at all.
pub fn draw_radar(ctx: &egui::Context, world: &mut World, opts: &RadarOptions) -> bool {
    if opts.hidden || world.res::<Hud>().cutscene {
        return true;
    }
    if !opts.show || world.resources.get::<Minimap>().is_none() {
        return false;
    }
    let t0 = Instant::now();
    let screen = ctx.content_rect();
    let (dt, time) = ctx.input(|i| (i.stable_dt.min(0.1), i.time));
    world.scope::<Minimap, _>(|world, mm| {
        let scene = Scene::gather(world, mm.last_camera);
        mm.last_camera = scene.camera;
        if scene.driving.is_some() {
            mm.owned_vehicle = scene.driving;
        }
        mm.update_route(scene.ground(), dt);
        mm.tiles.pump(ctx);
        let lay = mm.layout(screen, &scene, opts, dt);
        let (rect, view) = (lay.rect, lay.view);
        let painter = ctx.layer_painter(egui::LayerId::background());
        let p = painter.with_clip_rect(rect);
        // Ground: the land colour until the tiles arrive.
        let base = if mm.map.sea { raster::SEA } else { raster::VOID };
        painter.rect_filled(rect, lay.radius, Color32::from_rgba_unmultiplied(base[0], base[1], base[2], 150));
        let mut meshes = Vec::new();
        tile_meshes(&mut mm.tiles, ctx, &view, 222, &mut meshes);
        for mut m in meshes {
            view::round_corners(&mut m, rect, lay.radius);
            p.add(egui::Shape::mesh(m));
        }
        // GPS route, from the player on.
        if let Some(route) = &mm.route {
            let mut pts = Vec::new();
            let span = rect.width() / view.scale;
            route_points(route, &view, Some(scene.ground()), span * 2.5, &mut pts);
            if pts.len() >= 2 {
                let color = if mm.goal.is_some_and(|g| g.1) { MISSION_COLOR } else { WAYPOINT_COLOR };
                let width = (9.0 * view.scale).clamp(3.0, 7.0);
                p.add(egui::Shape::line(pts, Stroke::new(width, color)));
            }
        }
        let r = (rect.height() * 0.042).clamp(5.0, 11.0);
        let inner = rect.shrink(r * 1.15);
        let mut blips = Vec::new();
        let (lo, hi) = view.ground_bounds();
        mm.collect_blips(&scene, false, Some(((lo + hi) * 0.5, lo.distance(hi) * 0.5 + 10.0)), &mut blips);
        draw_blips(&p, &view, inner, &blips, Some(scene.pos.y), r, time, None);
        draw_arrow(&p, view.anchor, view.screen_dir(scene.heading), r * 1.25);
        // Frame, and the N that slides along it.
        painter.rect_stroke(rect, lay.radius, Stroke::new(2.0, Color32::from_black_alpha(200)), egui::StrokeKind::Outside);
        let n = view::border_point(rect.shrink(r * 0.2), view.anchor, view.screen_dir(view::NORTH));
        painter.circle(n, r * 1.05, Color32::from_black_alpha(215), Stroke::new(1.0, Color32::from_white_alpha(200)));
        painter.text(n, egui::Align2::CENTER_CENTER, "N", egui::FontId::proportional(r * 1.5), Color32::WHITE);
        draw_bars(&painter, lay.bars, scene.health, scene.armor);
        let ms = t0.elapsed().as_secs_f32() * 1000.0;
        mm.last_ms = ms;
        mm.cpu_ms = if mm.cpu_ms == 0.0 { ms } else { mm.cpu_ms * 0.9 + ms * 0.1 };
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped island file: it loads, the GPS crosses the city on its
    /// roads and a tile of the old town paints quickly.
    #[test]
    fn the_island_file_loads_routes_and_paints() {
        let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/maps/isla/minimap.bin");
        let map = MapData::from_island_file(&file, IslandFrame::VALENCIA).expect("island file");
        assert!(map.sea && map.areas.len() > 40_000 && map.roads.len() > 20_000 && map.pois.len() > 300);
        // The island is some 24 x 31 km.
        let size = map.max - map.min;
        assert!(size.x > 20_000.0 && size.x < 30_000.0 && size.y > 28_000.0 && size.y < 36_000.0, "{size:?}");
        assert!(map.pois.iter().any(|p| p.kind == PoiKind::Hospital) && map.pois.iter().any(|p| p.kind == PoiKind::Police));
        let graph = RoadGraph::build(&map, true);
        assert!(graph.node_count() > 10_000);
        let (from, to) = (Vec2::ZERO, Vec2::new(1500.0, -900.0));
        let t = Instant::now();
        let route = graph.route(from, to, 600.0).expect("a road route across the city");
        let took = t.elapsed();
        // Longer than the straight line, not absurdly so.
        let straight = from.distance(to);
        assert!(route.length() > straight * 0.9 && route.length() < straight * 2.5, "{} m for {straight} m", route.length());
        assert!(took.as_millis() < 500, "{took:?}");
        // And back, and to the far south of the island.
        assert!(graph.route(to, from, 600.0).is_some());
        assert!(graph.route(from, Vec2::new(-3700.0, 8990.0), 600.0).is_some_and(|r| r.length() > 9000.0));
        let index = MapIndex::new(Arc::new(map));
        let image = index.paint_tile(TileKey { level: 1, x: 0, y: 0 });
        // Streets and blocks: more than one colour.
        let first = image.pixels[0];
        assert!(image.pixels.iter().any(|p| *p != first));
    }
}
