//! City traffic from `forja_ai`: cars that drive the procedural Spanish
//! city by its own lights and signs, pedestrians that cross on the walk
//! signal, and the traffic lights switched to the very state the AI obeys.
//!
//! - The road network is built from the layout's exported lanes, signs and
//!   lights (`WorldDescription::traffic`).
//! - AI cars near the player are rendered with Kenney car models and carry
//!   a kinematic collider, so the player can hit them and they push props;
//!   farther ones ride their lanes unseen.
//! - The pedestrians of the simulation are given avatars by the crowd
//!   plugin (`crowd.rs`), within its radius.
//! - The player (on foot or driving) and the physics cars are fed to the AI
//!   as obstacles: drivers brake and honk; gunshots make people flee.

use std::sync::Arc;

use forja_ai::network::HeadBinding;
use forja_ai::{Aspect, External, ExternalKind, PedSignal, RoadNetwork, SimConfig, SimEvent, TrafficSim};
use forja_core::{App, Plugin, Stage, Time};
use forja_math::{Mat4, Quat, Vec2, Vec3};
use forja_physics::{BodyHandle, ColliderDesc, CollisionGroups, PhysicsWorld, RigidBodyDesc, Shape, VehicleConfig, Vehicles, layers};
use forja_render::PointLight;

use super::player::Player;
use super::vehicles::Driving;
use super::world::GameWorld;
use super::{Hud, Paths, RenderItem, RenderScene};

pub struct TrafficPlugin;

/// AI cars closer than this to the player are drawn (m).
const DRAW: f32 = 170.0;
/// ...and closer than this get a collider.
const SOLID: f32 = 45.0;

/// A car model: scene, rotation onto +Z, bounds in that frame.
struct CarModel {
    model: usize,
    rot: Quat,
    lo: Vec3,
    hi: Vec3,
}

impl CarModel {
    /// Model transform for a car `len` long, base on the ground at the origin.
    fn local(&self, len: f32) -> Mat4 {
        let k = len / (self.hi.z - self.lo.z).max(0.1);
        let c = Vec3::new((self.lo.x + self.hi.x) * 0.5, self.lo.y, (self.lo.z + self.hi.z) * 0.5);
        Mat4::from_scale(Vec3::splat(k)) * Mat4::from_translation(-c) * Mat4::from_quat(self.rot)
    }
}

pub struct TrafficAi {
    pub sim: TrafficSim,
    /// `(static render item, variant scenes [aspect][walk], binding)`.
    heads: Vec<(usize, [[usize; 2]; 3], HeadBinding)>,
    /// Small, medium and large car models.
    cars: [Vec<CarModel>; 3],
    bodies: Vec<(u64, BodyHandle)>,
    shots_seen: u32,
    honk_msg: f32,
    /// The simulation's pedestrians get avatars from the crowd plugin.
    pub drives_peds: bool,
    /// Roadway height on sloping maps (the AI drives in 2D).
    ground: Option<forja_world::HeightGrid>,
}

/// Base height and rotation of a car `len` long at `pos` heading `fwd`:
/// on the roadway, pitched with its grade.
fn car_pose(ground: &Option<forja_world::HeightGrid>, pos: Vec2, fwd: Vec2, len: f32) -> (f32, Quat) {
    let yaw = Quat::from_rotation_y(fwd.x.atan2(fwd.y));
    let Some(g) = ground else { return (0.0, yaw) };
    let half = fwd * (len * 0.5);
    let (front, back) = (g.at(pos + half), g.at(pos - half));
    // Rotating about X by -pitch lifts the nose (+Z) towards +Y.
    let pitch = ((front - back) / len.max(0.5)).atan();
    ((front + back) * 0.5, yaw * Quat::from_rotation_x(-pitch))
}

fn car_model(rs: &mut RenderScene, paths: &Paths, file: &str) -> Option<CarModel> {
    let m = rs.load(&paths.asset(file))?;
    let scene = Arc::clone(&rs.models[m]);
    // Orientation from the wheel layout, as for the drivable cars.
    let bound = forja_gameplay::bind_vehicle_model(VehicleConfig::sedan(), &scene, 1.0);
    let (_, rot, _) = bound.model_to_body.to_scale_rotation_translation();
    let b = scene.bounds();
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    for k in 0..8 {
        let p = Vec3::new(if k & 1 == 0 { b.min.x } else { b.max.x }, if k & 2 == 0 { b.min.y } else { b.max.y }, if k & 4 == 0 { b.min.z } else { b.max.z });
        let q = rot * p;
        lo = lo.min(q);
        hi = hi.max(q);
    }
    Some(CarModel { model: m, rot, lo, hi })
}

impl Plugin for TrafficPlugin {
    fn name(&self) -> &'static str {
        "traffic"
    }

    fn dependencies(&self) -> &[&'static str] {
        &["world", "vehicles", "player"]
    }

    fn build(&self, app: &mut App) {
        let Some(info) = app.world.res::<GameWorld>().desc.traffic.clone() else { return };
        let t = std::time::Instant::now();
        let roads = forja_ai::city::roads(&info);
        let net = RoadNetwork::build(&roads.spec);
        let bound = net.bind_heads(&roads.heads);
        log::info!(
            "traffic: {} lanes, {} junctions, {} signals ({} of {} heads bound), {} crosswalks in {:.2?}",
            net.lanes.len(),
            net.nodes.len(),
            net.signals.len(),
            bound.iter().filter(|b| b.approach.is_some()).count(),
            bound.len(),
            net.crosswalks.len(),
            t.elapsed()
        );
        let paths = app.world.res::<Paths>().clone();
        let crowd = super::people::PeopleLibrary::available(&paths);
        let cfg = SimConfig {
            seed: 0x7a ^ 0xf1c,
            max_vehicles: 70,
            vehicle_density: 14.0,
            max_peds: if crowd { 56 } else { 0 },
            ped_density: 70.0,
            ped_radius: 62.0,
            spawn_min: 75.0,
            spawn_max: 175.0,
            despawn: 200.0,
            near: 80.0,
            ..Default::default()
        };
        let mut sim = TrafficSim::new(net, cfg);
        let (scene_base, static_base) = {
            let gw = app.world.res::<GameWorld>();
            (gw.scene_base, gw.static_base)
        };
        let heads: Vec<(usize, [[usize; 2]; 3], HeadBinding)> = info.signals.iter().zip(bound).filter(|(_, b)| b.approach.is_some()).map(|(p, b)| (static_base + p.placement, p.variants.map(|v| v.map(|s| scene_base + s)), b)).collect();
        // Room for the drivable cars parked by the vehicles plugin.
        {
            let physics = app.world.res::<PhysicsWorld>();
            let spots: Vec<Vec2> = app.world.res::<Vehicles>().0.iter().filter_map(|v| physics.body(v.body)).map(|b| Vec2::new(b.position().x, b.position().z)).collect();
            for p in spots {
                sim.clear_parked_near(p, 6.0);
            }
        }
        let feet = app.world.res::<Player>().controller.feet();
        sim.focus = Vec2::new(feet.x, feet.z);
        let rs = app.world.res_mut::<RenderScene>();
        // The heads swap their model with the aspect: keep them out of the
        // static culling grid.
        rs.volatile_statics.extend(heads.iter().map(|h| h.0));
        rs.volatile_statics.sort_unstable();
        rs.volatile_statics.dedup();
        let load = |rs: &mut RenderScene, files: &[&str]| files.iter().filter_map(|f| car_model(rs, &paths, &format!("kenney_car_kit/{f}.glb"))).collect::<Vec<_>>();
        let cars = [load(rs, &["hatchback-sports", "sedan-sports", "taxi"]), load(rs, &["sedan", "taxi", "suv", "police", "suv-luxury", "sedan"]), load(rs, &["van", "delivery", "truck"])];
        log::info!("traffic: {} car models", cars.iter().map(Vec::len).sum::<usize>());
        let shots = 0;
        app.insert_resource(TrafficAi { sim, heads, cars, bodies: Vec::new(), shots_seen: shots, honk_msg: 0.0, drives_peds: crowd, ground: info.ground.clone() });
        app.add_system_at(Stage::Update, "traffic_update", -5, traffic_update);
        app.add_system_at(Stage::PostUpdate, "traffic_render", 12, traffic_render);
    }
}

fn traffic_update(w: &mut forja_core::World) {
    let dt = w.res::<Time>().delta.min(0.1);
    // What the AI must see: the player and the physics cars.
    let mut ext = Vec::new();
    let driving = w.res::<Driving>().0;
    let player = {
        let p = w.res::<Player>();
        (p.controller.feet(), p.controller.velocity)
    };
    {
        let physics = w.res::<PhysicsWorld>();
        for (i, v) in w.res::<Vehicles>().0.iter().enumerate() {
            let Some(b) = physics.body(v.body) else { continue };
            let fwd3 = b.rotation() * Vec3::Z;
            let fwd = Vec2::new(fwd3.x, fwd3.z).normalize_or(Vec2::Y);
            let half = Vec2::new(v.config.chassis_half.x, v.config.chassis_half.z);
            let kind = if driving == Some(i) { ExternalKind::PlayerCar } else { ExternalKind::Car };
            ext.push(External { pos: Vec2::new(b.position().x, b.position().z), vel: Vec2::new(b.linvel.x, b.linvel.z), fwd, half, kind });
        }
    }
    if driving.is_none() {
        ext.push(External { pos: Vec2::new(player.0.x, player.0.z), vel: Vec2::new(player.1.x, player.1.z), fwd: Vec2::Y, half: Vec2::splat(0.35), kind: ExternalKind::Player });
    }
    let focus = Vec2::new(player.0.x, player.0.z);
    let shots = w.resources.get::<super::combat::Combat>().map_or(0, |c| c.shots_fired);
    let mut honk_near = false;
    w.scope::<TrafficAi, _>(|w, ai| {
        ai.sim.externals = ext;
        ai.sim.focus = focus;
        if shots > ai.shots_seen {
            ai.sim.alarm(focus, 45.0);
        }
        ai.shots_seen = shots;
        ai.sim.advance(dt);
        for e in ai.sim.events.drain(..) {
            if let SimEvent::Honk { pos, .. } = e
                && pos.distance(focus) < 25.0
            {
                honk_near = true;
            }
        }
        ai.honk_msg -= dt;
        // Colliders for the cars around the player.
        let physics = w.res_mut::<PhysicsWorld>();
        let alpha = ai.sim.alpha();
        let mut keep = Vec::new();
        for v in &ai.sim.vehicles {
            if v.pos.distance(focus) > SOLID {
                continue;
            }
            let pos = v.prev_pos.lerp(v.pos, alpha);
            let fwd = v.prev_fwd.lerp(v.fwd, alpha).normalize_or(v.fwd);
            let (y, rot) = car_pose(&ai.ground, pos, fwd, v.len);
            let iso = forja_math::Iso::new(Vec3::new(pos.x, y + 0.75, pos.y), rot);
            let vel = Vec3::new(v.fwd.x, 0.0, v.fwd.y) * v.v;
            let h = match ai.bodies.iter().find(|b| b.0 == v.id) {
                Some(b) => b.1,
                None => {
                    let body = physics.add_body(RigidBodyDesc::kinematic().position(iso.pos).rotation(iso.rot));
                    let groups = CollisionGroups::new(layers::VEHICLE, u32::MAX);
                    physics.add_collider(ColliderDesc::new(Shape::cuboid(Vec3::new(v.width * 0.5, 0.7, v.len * 0.5))).groups(groups), Some(body));
                    body
                }
            };
            physics.set_body_iso(h, iso);
            if let Some(b) = physics.body_mut(h) {
                b.set_linvel(vel);
            }
            keep.push((v.id, h));
        }
        for (id, h) in &ai.bodies {
            if !keep.iter().any(|k| k.0 == *id) {
                physics.remove_body(*h);
            }
        }
        ai.bodies = keep;
    });
    if honk_near && w.res::<TrafficAi>().honk_msg <= 0.0 {
        w.res_mut::<TrafficAi>().honk_msg = 5.0;
        w.res_mut::<Hud>().message("¡Piii! Un conductor te pita");
    }
}

fn traffic_render(w: &mut forja_core::World) {
    let night = {
        let tod = w.res::<super::env::TimeOfDay>();
        if tod.photo_sky { 0.0 } else { tod.night() }
    };
    let cam = w.res::<RenderScene>().camera.position;
    let cam2 = Vec2::new(cam.x, cam.z);
    let mut items = Vec::new();
    let mut lights: Vec<(f32, PointLight)> = Vec::new();
    let ai = w.res::<TrafficAi>();
    let sim = &ai.sim;
    let alpha = sim.alpha();
    let pick = |len: f32, style: u32| {
        let class = if len < 4.1 { 0 } else if len > 5.0 { 2 } else { 1 };
        let set = if ai.cars[class].is_empty() { &ai.cars[1] } else { &ai.cars[class] };
        (!set.is_empty()).then(|| &set[style as usize % set.len()])
    };
    for v in &sim.vehicles {
        let pos = v.prev_pos.lerp(v.pos, alpha);
        let d = pos.distance(cam2);
        if d > DRAW {
            continue;
        }
        let Some(m) = pick(v.len, v.style) else { continue };
        let fwd = v.prev_fwd.lerp(v.fwd, alpha).normalize_or(v.fwd);
        let (y, rot) = car_pose(&ai.ground, pos, fwd, v.len);
        let body = Mat4::from_rotation_translation(rot, Vec3::new(pos.x, y, pos.y));
        items.push(RenderItem::new(m.model, body * m.local(v.len)).with_id(2 << 40 | v.id));
        let f3 = rot * Vec3::Z;
        let back = Vec3::new(pos.x, y + 0.8, pos.y) - f3 * (v.len * 0.5 + 0.2);
        if v.braking && d < 70.0 {
            lights.push((d, PointLight { position: back, radius: 4.0, color: Vec3::new(1.0, 0.05, 0.02), intensity: 18.0 }));
        }
        if night > 0.2 && d < 90.0 {
            let front = Vec3::new(pos.x, y + 0.8, pos.y) + f3 * (v.len * 0.5 + 3.0);
            lights.push((d, PointLight { position: front, radius: 16.0, color: Vec3::new(1.0, 0.95, 0.85), intensity: 70.0 }));
        }
    }
    for c in &sim.parked {
        let d = c.pos.distance(cam2);
        if d > 120.0 {
            continue;
        }
        let Some(m) = pick(c.len, c.style) else { continue };
        let (y, rot) = car_pose(&ai.ground, c.pos, c.fwd, c.len);
        let body = Mat4::from_rotation_translation(rot, Vec3::new(c.pos.x, y, c.pos.y));
        items.push(RenderItem::new(m.model, body * m.local(c.len)));
    }
    // Lights: what the drivers obey is what the heads show; the walking
    // man flashes at 2 Hz.
    let flash_on = (sim.t * 2.0).fract() < 0.5;
    let states: Vec<(usize, usize)> = ai
        .heads
        .iter()
        .map(|(item, variants, b)| {
            let (a, p) = sim.net.head_state(*b, sim.t);
            let ai_ = match a {
                Aspect::Red => 0,
                Aspect::Amber => 1,
                Aspect::Green => 2,
            };
            let walk = match p {
                PedSignal::Walk => true,
                PedSignal::Flash => flash_on,
                PedSignal::DontWalk => false,
            };
            (*item, variants[ai_][walk as usize])
        })
        .collect();
    if night > 0.3 {
        for (item, _, b) in &ai.heads {
            let Some(a) = b.approach else { continue };
            let ap = &sim.net.approaches[a as usize];
            let d = ap.stop_pos.distance(cam2);
            if d < 80.0 {
                let color = match sim.net.approach_aspect(a, sim.t).0 {
                    Aspect::Red => Vec3::new(1.0, 0.05, 0.02),
                    Aspect::Amber => Vec3::new(1.0, 0.45, 0.0),
                    Aspect::Green => Vec3::new(0.1, 1.0, 0.4),
                };
                let _ = item;
                let y = ai.ground.as_ref().map_or(0.0, |g| g.at(ap.stop_pos));
                lights.push((d, PointLight { position: Vec3::new(ap.stop_pos.x, y + 3.0, ap.stop_pos.y), radius: 6.0, color, intensity: 8.0 }));
            }
        }
    }
    lights.sort_by(|a, b| a.0.total_cmp(&b.0));
    let rs = w.res_mut::<RenderScene>();
    for (item, model) in states {
        if let Some(it) = rs.statics.get_mut(item) {
            it.model = model;
        }
    }
    rs.dynamic.extend(items);
    rs.lights.extend(lights.into_iter().take(24).map(|l| l.1));
}
