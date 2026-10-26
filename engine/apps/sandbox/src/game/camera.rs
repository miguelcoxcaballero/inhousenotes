//! Third-person camera: over-the-shoulder on foot, chase cam in vehicles,
//! collision against the world so it never clips into walls.

use forja_core::{App, Plugin, Stage, Time};
use forja_math::{EulerRot, Quat, Ray, Vec3, smoothing};
use forja_physics::{CollisionGroups, PhysicsWorld, QueryFilter, Vehicles, layers};
use forja_render::Camera;

use super::player::Player;
use super::vehicles::{Driving, VehicleVisuals};
use super::{Intent, RenderScene};

pub struct CameraPlugin;

pub struct CameraRig {
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub fov: f32,
    /// Seconds since the mouse last moved (chase cam recenters after a while).
    pub idle: f32,
    /// Overrides everything (screenshots, cinematics).
    pub fixed: Option<(Vec3, Vec3)>,
    smoothed: Option<Vec3>,
}

impl Plugin for CameraPlugin {
    fn name(&self) -> &'static str {
        "camera"
    }

    fn dependencies(&self) -> &[&'static str] {
        &["player"]
    }

    fn build(&self, app: &mut App) {
        let facing = app.world.res::<Player>().facing;
        app.insert_resource(CameraRig { yaw: facing + std::f32::consts::PI, pitch: -0.18, distance: 4.2, fov: 62.0, idle: 0.0, fixed: None, smoothed: None });
        app.add_system(Stage::PostUpdate, "camera", camera_system);
    }
}

fn camera_system(w: &mut forja_core::World) {
    let dt = w.res::<Time>().delta.max(1e-4);
    let intent = w.res::<Intent>().clone();
    let driving = w.res::<Driving>().0;
    let (target, exclude, chase_heading, speed, bike) = {
        let physics = w.res::<PhysicsWorld>();
        let player = w.res::<Player>();
        match driving.and_then(|i| w.res::<Vehicles>().0.get(i).map(|v| (i, v))) {
            Some((i, v)) => {
                let b = physics.body(v.body).expect("vehicle body");
                let fwd = b.rotation() * Vec3::Z;
                let bike = w.res::<VehicleVisuals>().0[i].bike;
                (b.position() + Vec3::Y * if bike { 1.0 } else { 1.4 }, v.body, Some(fwd.x.atan2(fwd.z) + std::f32::consts::PI), b.linvel.length(), bike)
            }
            None => (player.position() + Vec3::Y * 0.55, player.controller.body, None, 0.0, false),
        }
    };
    let rig = w.res_mut::<CameraRig>();
    if intent.look.length_squared() > 0.0 {
        rig.idle = 0.0;
    } else {
        rig.idle += dt;
    }
    rig.yaw -= intent.look.x;
    rig.pitch = (rig.pitch - intent.look.y).clamp(-1.25, 0.7);
    if let Some(h) = chase_heading
        && rig.idle > 1.2
    {
        let mut d = h - rig.yaw;
        while d > std::f32::consts::PI {
            d -= std::f32::consts::TAU;
        }
        while d < -std::f32::consts::PI {
            d += std::f32::consts::TAU;
        }
        rig.yaw += d * smoothing(2.5, dt);
        rig.pitch += (-0.22 - rig.pitch) * smoothing(1.5, dt);
    }
    let aiming = intent.aim && driving.is_none();
    let want_dist = if driving.is_some() { if bike { 4.6 } else { 7.0 } } else if aiming { 1.9 } else { 4.2 };
    rig.distance += (want_dist - rig.distance) * smoothing(8.0, dt);
    let want_fov = if aiming { 45.0 } else { 62.0 + (speed * 0.35).min(14.0) };
    rig.fov += (want_fov - rig.fov) * smoothing(6.0, dt);
    let rot = Quat::from_euler(EulerRot::YXZ, rig.yaw, rig.pitch, 0.0);
    let shoulder = if driving.is_some() { 0.0 } else if aiming { 0.55 } else { 0.35 };
    let pivot = match rig.smoothed {
        Some(s) if driving.is_some() => s + (target - s) * smoothing(18.0, dt),
        _ => target,
    };
    rig.smoothed = Some(pivot);
    let desired = pivot + rot * Vec3::new(shoulder, 0.25, rig.distance);
    let (fixed, fov) = (rig.fixed, rig.fov);
    // Pull in against walls.
    let dir = desired - pivot;
    let len = dir.length();
    let filter = QueryFilter::exclude(exclude).with_groups(CollisionGroups::new(u32::MAX, layers::WORLD));
    let pos = match w.res::<PhysicsWorld>().raycast(&Ray::new(pivot, dir), len, filter) {
        Some(h) => pivot + dir / len * (h.t - 0.25).max(0.3),
        None => desired,
    };
    let camera = match fixed {
        Some((eye, look)) => Camera::look_at(eye, look),
        None => Camera { position: pos, rotation: rot, fov_y: fov.to_radians(), near: 0.1, aspect: 16.0 / 9.0 },
    };
    w.res_mut::<RenderScene>().camera = camera;
}
