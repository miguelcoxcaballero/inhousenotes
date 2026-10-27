//! Cars and bikes: spawning from models + handling data, driving, wheel
//! visuals, headlights and brake lights.

use std::sync::Arc;

use forja_core::{App, Plugin, Stage};
use forja_gameplay::HandlingRegistry;
use forja_math::{Iso, Mat4, Quat, Vec3, Vec4};
use forja_physics::{PhysicsWorld, Vehicle, VehicleConfig, Vehicles};
use forja_render::PointLight;
use forja_world::SpawnKind;

use super::props::box_scene;
use super::world::GameWorld;
use super::{Paths, RenderItem, RenderScene};

pub struct VehiclesPlugin;

/// Render side of a vehicle; index matches `Vehicles.0`.
pub struct VehicleVisual {
    pub name: String,
    pub model: usize,
    /// Model-to-body transform (offset, orientation and scale).
    pub local: Mat4,
    /// `(nodes, rest center)` per physics wheel.
    pub wheels: Vec<(Vec<usize>, Vec3)>,
    pub rest: Arc<Vec<Mat4>>,
    pub tint: Vec4,
    pub bike: bool,
    /// Seat position in the body frame.
    pub seat: Vec3,
}

#[derive(Default)]
pub struct VehicleVisuals(pub Vec<VehicleVisual>);

/// Index of the vehicle the player drives.
#[derive(Default)]
pub struct Driving(pub Option<usize>);

/// `(model file under assets/downloaded, handling, scale, paint color)`.
const CARS: [(&str, &str, f32, [f32; 3]); 8] = [
    ("khronos_car_concept/CarConcept.glb", "sports", 1.0, [0.55, 0.02, 0.03]),
    ("khronos_car_concept/CarConcept.glb", "sports", 1.0, [0.02, 0.05, 0.25]),
    ("khronos_car_concept/CarConcept.glb", "sports", 1.0, [0.8, 0.8, 0.82]),
    ("khronos_car_concept/CarConcept.glb", "sports", 1.0, [0.02, 0.02, 0.02]),
    ("kenney_car_kit/suv.glb", "suv", 1.8, [1.0, 1.0, 1.0]),
    ("kenney_car_kit/police.glb", "sports", 1.8, [1.0, 1.0, 1.0]),
    ("kenney_car_kit/taxi.glb", "sedan", 1.8, [1.0, 1.0, 1.0]),
    ("kenney_car_kit/truck.glb", "truck", 1.8, [1.0, 1.0, 1.0]),
];

/// Copy of a car scene with its paint materials recolored.
fn repaint(scene: &forja_assets::SceneData, color: [f32; 3]) -> forja_assets::SceneData {
    let mut s = scene.clone();
    for m in &mut s.materials {
        if m.name.to_ascii_lowercase().starts_with("paint") {
            m.base_color = Vec4::new(color[0], color[1], color[2], 1.0);
            m.base_color_tex = None;
        }
    }
    s
}

/// Photoreal cars downloaded from Sketchfab (`sf_car_*/scene.gltf`), if any.
fn downloaded_cars(paths: &Paths) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(&paths.downloaded)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    (name.starts_with("sf_car_") && e.path().join("scene.gltf").exists()).then(|| format!("{name}/scene.gltf"))
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

fn spawn_car(app: &mut App, file: &str, handling: &str, scale: f32, color: [f32; 3], iso: Iso, reg: &HandlingRegistry) -> bool {
    let paths = app.world.res::<Paths>().clone();
    let path = paths.asset(file);
    let rs = app.world.res_mut::<RenderScene>();
    let Some(base_model) = rs.load(&path) else { return false };
    let base = Arc::clone(&rs.models[base_model]);
    let paintable = base.materials.iter().any(|m| m.name.to_ascii_lowercase().starts_with("paint"));
    let (model, tint) = if paintable {
        (rs.add_model(Arc::new(repaint(&base, color))), Vec4::ONE)
    } else {
        (base_model, Vec4::new(color[0], color[1], color[2], 1.0))
    };
    let handling_cfg = reg.get(handling).cloned().unwrap_or_else(VehicleConfig::sedan);
    // Downloaded models: fix orientation and units automatically.
    let (pre, scale) = if file.starts_with("sf_") { (forja_gameplay::auto_orient(&base), forja_gameplay::auto_scale(&base, 4.5)) } else { (Quat::IDENTITY, scale) };
    let bound = forja_gameplay::bind_vehicle_model_oriented(handling_cfg, &base, scale, pre);
    let wheels = bound.wheel_nodes.iter().cloned().zip(bound.wheel_centers.iter().copied()).collect();
    let seat = Vec3::new(0.35, 0.1, -0.1);
    let physics = app.world.res_mut::<PhysicsWorld>();
    let vehicle = Vehicle::spawn(physics, bound.config, iso);
    app.world.res_mut::<Vehicles>().0.push(vehicle);
    app.world.res_mut::<VehicleVisuals>().0.push(VehicleVisual {
        name: file.rsplit('/').next().unwrap_or(file).trim_end_matches(".glb").to_string(),
        model,
        local: bound.model_to_body,
        wheels,
        rest: Arc::new(base.world_matrices()),
        tint,
        bike: false,
        seat,
    });
    true
}

fn spawn_bike(app: &mut App, iso: Iso, reg: &HandlingRegistry) {
    let paths = app.world.res::<Paths>().clone();
    let config = reg.get("sport_bike").cloned().unwrap_or_else(VehicleConfig::motorcycle);
    let rs = app.world.res_mut::<RenderScene>();
    // The OBJ bike points down -X and is about 3.9 units long.
    let (model, local) = match rs.load(&paths.asset("oga_fancy_motorcycle/bike.obj")) {
        Some(m) => {
            let b = rs.model(m).bounds();
            let s = 2.05 / (b.max.x - b.min.x);
            let rot = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
            let center = Vec3::new((b.min.x + b.max.x) * 0.5, b.min.y, (b.min.z + b.max.z) * 0.5);
            let feet = -(config.chassis_half.y + 0.31 + 0.12);
            (m, Mat4::from_translation(Vec3::new(0.0, feet, 0.0)) * Mat4::from_quat(rot) * Mat4::from_scale(Vec3::splat(s)) * Mat4::from_translation(-center))
        }
        None => (rs.add_model(Arc::new(box_scene(config.chassis_half, Vec4::new(0.1, 0.1, 0.1, 1.0)))), Mat4::IDENTITY),
    };
    let rest = Arc::new(rs.model(model).world_matrices());
    let physics = app.world.res_mut::<PhysicsWorld>();
    // Parked: on the ground at its ride height, already leaning onto its
    // kickstand (dropped from a height, a parked bike bounces and falls).
    let iso = match physics.raycast(&forja_math::Ray::new(iso.pos + Vec3::Y * 1.5, Vec3::NEG_Y), 4.0, Default::default()) {
        Some(hit) => {
            let w = config.wheels.last().expect("bike wheels");
            let ride = -(w.position.y - w.suspension_length * 0.8 - w.radius);
            let lean = config.bike.as_ref().map_or(0.0, |b| b.kickstand_lean_deg.to_radians());
            Iso::new(Vec3::new(iso.pos.x, hit.point.y + ride * lean.cos(), iso.pos.z), iso.rot * Quat::from_rotation_z(-lean))
        }
        None => iso,
    };
    let vehicle = Vehicle::spawn(physics, config, iso);
    app.world.res_mut::<Vehicles>().0.push(vehicle);
    app.world.res_mut::<VehicleVisuals>().0.push(VehicleVisual {
        name: "motorcycle".into(),
        model,
        local,
        wheels: Vec::new(),
        rest,
        tint: Vec4::ONE,
        bike: true,
        seat: Vec3::new(0.0, 0.15, -0.2),
    });
}

impl Plugin for VehiclesPlugin {
    fn name(&self) -> &'static str {
        "vehicles"
    }

    fn dependencies(&self) -> &[&'static str] {
        &["world", "forja_physics"]
    }

    fn build(&self, app: &mut App) {
        app.insert_resource(VehicleVisuals::default()).insert_resource(Driving::default());
        let reg = HandlingRegistry::load(&app.world.res::<Paths>().data.join("handling")).unwrap_or_else(|e| {
            log::warn!("handling: {e}");
            HandlingRegistry::default()
        });
        let desc = &app.world.res::<GameWorld>().desc;
        let player = desc.spawn(SpawnKind::Player).unwrap_or(Iso::from_pos(Vec3::Y));
        let mut car_spawns: Vec<Iso> = desc.spawns_of(SpawnKind::Vehicle).map(|s| s.iso).collect();
        car_spawns.sort_by(|a, b| (a.pos - player.pos).length_squared().total_cmp(&(b.pos - player.pos).length_squared()));
        car_spawns.truncate(CARS.len());
        let mut bike_spawns: Vec<Iso> = desc.spawns_of(SpawnKind::Bike).map(|s| s.iso).collect();
        if bike_spawns.is_empty() {
            // Beside the player, on open level ground (not in a raised
            // flower bed or a facade, where it fell over).
            let fwd = player.rot * Vec3::Z;
            let at = super::props::open_ground(app.world.res::<PhysicsWorld>(), player.pos, 3.5, 3, 0.9, fwd).map_or(player.pos + player.rot * Vec3::new(-3.0, 0.2, 2.0), |g| g + fwd * 0.9 + Vec3::Y * 0.2);
            bike_spawns.push(Iso::new(at, player.rot));
        }
        if car_spawns.is_empty() {
            car_spawns.push(Iso::new(player.pos + Vec3::new(4.0, 0.5, 0.0), player.rot));
        }
        let mut spawned = 0;
        // Prefer scanned/high-detail downloads, then the built-in list.
        let paths = app.world.res::<Paths>().clone();
        let mut lineup: Vec<(String, &str, f32, [f32; 3])> = downloaded_cars(&paths).into_iter().map(|f| (f, "sedan", 1.0, [1.0, 1.0, 1.0])).collect();
        lineup.extend(CARS.iter().map(|(f, h, s, c)| (f.to_string(), *h, *s, *c)));
        for (i, iso) in car_spawns.iter().enumerate() {
            let (file, handling, scale, color) = lineup[i % lineup.len()].clone();
            let file = file.as_str();
            let lifted = Iso::new(iso.pos + Vec3::Y * 0.8, iso.rot);
            if spawn_car(app, file, handling, scale, color, lifted, &reg) {
                spawned += 1;
            }
        }
        if spawned == 0 {
            // No Kenney cars downloaded: drive the procedural sedan box.
            let physics = app.world.res_mut::<PhysicsWorld>();
            let v = Vehicle::spawn(physics, VehicleConfig::sedan(), Iso::new(car_spawns[0].pos + Vec3::Y, car_spawns[0].rot));
            let half = v.config.chassis_half;
            app.world.res_mut::<Vehicles>().0.push(v);
            let model = app.world.res_mut::<RenderScene>().add_model(Arc::new(box_scene(half, Vec4::new(0.7, 0.1, 0.1, 1.0))));
            app.world.res_mut::<VehicleVisuals>().0.push(VehicleVisual {
                name: "box sedan".into(),
                model,
                local: Mat4::IDENTITY,
                wheels: Vec::new(),
                rest: Arc::new(vec![Mat4::IDENTITY]),
                tint: Vec4::ONE,
                bike: false,
                seat: Vec3::new(0.35, 0.1, -0.1),
            });
        }
        for iso in bike_spawns {
            spawn_bike(app, Iso::new(iso.pos + Vec3::Y * 0.5, iso.rot), &reg);
        }

        app.add_system_at(Stage::PostUpdate, "vehicles_render", 10, render_vehicles);
    }
}

fn render_vehicles(w: &mut forja_core::World) {
    let tod = w.res::<super::env::TimeOfDay>();
    let night = if tod.photo_sky { 0.0 } else { tod.night() };
    let driving = w.res::<Driving>().0;
    let alpha = w.res::<forja_core::Time>().alpha;
    let mut items = Vec::new();
    let mut lights = Vec::new();
    {
        let physics = w.res::<PhysicsWorld>();
        let vehicles = &w.res::<Vehicles>().0;
        for (i, vis) in w.res::<VehicleVisuals>().0.iter().enumerate() {
            let Some(v) = vehicles.get(i) else { continue };
            let Some(body) = physics.body(v.body) else { continue };
            // Interpolated between the last two fixed steps (no judder at
            // frame rates other than 60 Hz).
            let iso = body.interpolated_iso(alpha);
            let body_m = iso.to_mat4();
            let transform = body_m * vis.local;
            let mut item = RenderItem::new(vis.model, transform).with_id(1 << 40 | i as u64);
            item.tint = vis.tint;
            if !vis.wheels.is_empty() {
                // Spin and steer each wheel group about its own center, in
                // model space (the model may be flipped relative to the body).
                let inv = transform.inverse();
                let (_, orient, _) = vis.local.to_scale_rotation_translation();
                let mut nodes = (*vis.rest).clone();
                for (wheel, (group, rest_center)) in vis.wheels.iter().enumerate() {
                    let Some(ws) = v.wheels.get(wheel) else { continue };
                    let center = inv.transform_point3(v.wheel_iso_at(iso, wheel).pos);
                    let body_rot = Quat::from_rotation_y(ws.steer_angle) * Quat::from_rotation_x(ws.spin);
                    let model_rot = orient.inverse() * body_rot * orient;
                    let m = Mat4::from_translation(center) * Mat4::from_quat(model_rot) * Mat4::from_translation(-*rest_center);
                    for &node in group {
                        nodes[node] = m * vis.rest[node];
                    }
                }
                item.nodes = Some(Arc::new(nodes));
            }
            items.push(item);
            // Headlights at night or when driven; brake lights when braking.
            let fwd = iso.rot * Vec3::Z;
            let up = iso.rot * Vec3::Y;
            let half = v.config.chassis_half;
            if night > 0.2 || driving == Some(i) && night > 0.05 {
                for side in [-0.6f32, 0.6] {
                    let p = iso.pos + fwd * (half.z + 0.6) + up * 0.1 + iso.rot * Vec3::X * side * half.x;
                    lights.push(PointLight { position: p + fwd * 3.0, radius: 18.0, color: Vec3::new(1.0, 0.95, 0.85), intensity: 90.0 });
                }
            }
            if v.controls.brake > 0.1 || (v.controls.throttle < -0.1 && v.speed(physics) > 1.0) {
                let p = iso.pos - fwd * (half.z + 0.3) + up * 0.15;
                lights.push(PointLight { position: p, radius: 5.0, color: Vec3::new(1.0, 0.05, 0.02), intensity: 25.0 });
            }
        }
    }
    let rs = w.res_mut::<RenderScene>();
    rs.dynamic.extend(items);
    rs.lights.extend(lights);
    let gauge = driving.and_then(|i| {
        let v = w.res::<Vehicles>().0.get(i)?;
        Some((v.speed(w.res::<PhysicsWorld>()).abs() * 3.6, v.gear, v.rpm))
    });
    let hud = w.res_mut::<super::Hud>();
    hud.speed_kmh = gauge.map(|g| g.0);
    if let Some((_, gear, rpm)) = gauge {
        hud.gear = gear;
        hud.rpm = rpm;
    }
}
