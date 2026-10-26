//! The player: character controller, animation state machine, entering and
//! driving vehicles.

use std::sync::Arc;

use forja_anim::{AnimController, Clip, ControllerDef, Skeleton};
use forja_core::{App, Plugin, Stage, Time};
use forja_gameplay::Health;
use forja_math::{Iso, Mat4, Quat, Vec3, Vec4};
use forja_physics::{CharacterConfig, CharacterController, CollisionGroups, PhysicsWorld, Vehicles, layers};
use forja_world::SpawnKind;

use super::camera::CameraRig;
use super::props::box_scene;
use super::vehicles::{Driving, VehicleVisuals};
use super::world::GameWorld;
use super::{Hud, Intent, Paths, RenderItem, RenderScene};

pub struct PlayerPlugin;

pub struct Avatar {
    pub model: usize,
    pub skeleton: Skeleton,
    pub anim: Option<AnimController>,
    pub scale: f32,
    /// Joint of the right hand/arm for weapon attachment.
    pub hand: Option<usize>,
    /// Realistic skinned human (vs. rigid node animation).
    pub human: Option<super::humans::HumanAvatar>,
}

pub struct Player {
    pub controller: CharacterController,
    pub facing: f32,
    pub health: Health,
    pub avatar: Avatar,
    pub hand_world: Mat4,
    pub respawn: Iso,
}

impl Player {
    pub fn position(&self) -> Vec3 {
        self.controller.position
    }
}

fn load_avatar(app: &mut App) -> Avatar {
    let paths = app.world.res::<Paths>().clone();
    if super::humans::HumanLibrary::available(&paths) {
        let human = app.world.scope::<super::humans::HumanLibrary, _>(|w, lib| {
            lib.spawn(&paths, w.res_mut::<RenderScene>(), &forja_assets::makehuman::HumanSpec::casual_male())
        });
        if let Some(h) = human {
            let hand = h.skeleton.find("wrist.R");
            return Avatar { model: h.model, skeleton: h.skeleton.clone(), anim: None, scale: 1.0, hand, human: Some(h) };
        }
    }
    let path = paths.asset("kenney_blocky_characters/character-a.glb");
    let rs = app.world.res_mut::<RenderScene>();
    if let Some(model) = rs.load(&path) {
        let scene = Arc::clone(&rs.models[model]);
        let skeleton = Skeleton::from_scene_any(&scene);
        let clips = Clip::bind_all(&scene, &skeleton, None);
        let anim = std::fs::read_to_string(paths.data.join("anim/blocky_humanoid.toml"))
            .map_err(|e| e.to_string())
            .and_then(|t| ControllerDef::from_toml(&t))
            .and_then(|def| AnimController::new(&def, &skeleton, clips))
            .map_err(|e| log::warn!("player animation: {e}"))
            .ok();
        let height = scene.bounds().max.y.max(0.1);
        let hand = skeleton.find("arm-right");
        return Avatar { model, skeleton, anim, scale: 1.8 / height, hand, human: None };
    }
    let model = rs.add_model(Arc::new(box_scene(Vec3::new(0.3, 0.9, 0.2), Vec4::new(0.2, 0.5, 0.9, 1.0))));
    let scene = Arc::clone(&rs.models[model]);
    Avatar { model, skeleton: Skeleton::from_nodes(&scene), anim: None, scale: 1.0, hand: None, human: None }
}

impl Plugin for PlayerPlugin {
    fn name(&self) -> &'static str {
        "player"
    }

    fn dependencies(&self) -> &[&'static str] {
        &["world", "vehicles"]
    }

    fn build(&self, app: &mut App) {
        let spawn = app.world.res::<GameWorld>().desc.spawn(SpawnKind::Player).unwrap_or(Iso::from_pos(Vec3::Y * 2.0));
        let avatar = load_avatar(app);
        let physics = app.world.res_mut::<PhysicsWorld>();
        let controller = CharacterController::new(physics, CharacterConfig::default(), spawn.pos + Vec3::Y * 0.5);
        let facing = (spawn.rot * Vec3::Z).x.atan2((spawn.rot * Vec3::Z).z);
        app.insert_resource(Player { controller, facing, health: Health::default(), avatar, hand_world: Mat4::IDENTITY, respawn: spawn });
        app.add_system(Stage::FixedUpdate, "player_move", player_move);
        app.add_system(Stage::Update, "player_animate", player_animate);
        app.add_system_at(Stage::PostUpdate, "player_render", 20, player_render);
    }
}

fn nearest_vehicle(w: &forja_core::World, pos: Vec3) -> Option<(usize, f32)> {
    let physics = w.res::<PhysicsWorld>();
    w.res::<Vehicles>()
        .0
        .iter()
        .enumerate()
        .filter_map(|(i, v)| physics.body(v.body).map(|b| (i, (b.position() - pos).length())))
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

fn player_move(w: &mut forja_core::World) {
    let dt = w.res::<Time>().fixed_dt;
    let cam_yaw = w.res::<CameraRig>().yaw;
    let intent = w.res::<Intent>().clone();
    let enter = Intent::take(&mut w.res_mut::<Intent>().enter);
    let jump = Intent::take(&mut w.res_mut::<Intent>().jump);
    let driving = w.res::<Driving>().0;
    let pos = w.res::<Player>().position();
    let dead = w.res::<Player>().health.is_dead();

    // Hint for the HUD.
    let near = nearest_vehicle(w, pos);
    w.res_mut::<Hud>().interact_hint = match (driving, near) {
        (Some(_), _) => Some("F: get out".into()),
        (None, Some((i, d))) if d < 3.5 => Some(format!("F: drive {}", w.res::<VehicleVisuals>().0[i].name)),
        _ => None,
    };

    if let Some(i) = driving {
        // Drive.
        let vehicles = &mut w.res_mut::<Vehicles>().0;
        let v = &mut vehicles[i];
        v.controls.throttle = intent.movement.y;
        v.controls.steer = intent.movement.x;
        v.controls.handbrake = intent.handbrake;
        v.controls.brake = 0.0;
        let body = v.body;
        if enter {
            v.controls = Default::default();
            // Get out on the driver's side, falling back to the roof.
            let physics = w.res::<PhysicsWorld>();
            let b = physics.body(body).expect("vehicle body");
            let side = b.rotation() * Vec3::X;
            let half = w.res::<Vehicles>().0[i].config.chassis_half;
            let mut exit = b.position() + side * (half.x + 0.9) + Vec3::Y * 0.4;
            if physics.raycast(&forja_math::Ray::new(b.position(), side), half.x + 1.2, forja_physics::QueryFilter::exclude(body)).is_some() {
                exit = b.position() + Vec3::Y * (half.y + 1.4);
            }
            let ch = w.res::<Player>().controller.body;
            let collider = w.res::<PhysicsWorld>().body(ch).and_then(|b| b.colliders().first().copied());
            if let Some(c) = collider {
                w.res_mut::<PhysicsWorld>().set_collider_groups(c, CollisionGroups::new(layers::CHARACTER, u32::MAX));
            }
            w.scope::<Player, _>(|w, player| player.controller.teleport(w.res_mut::<PhysicsWorld>(), exit));
            w.res_mut::<Driving>().0 = None;
        } else {
            // Ride along: keep the capsule with the car (no collisions).
            let p = w.res::<PhysicsWorld>().body(body).map(|b| b.iso()).expect("vehicle body");
            let seat = w.res::<VehicleVisuals>().0[i].seat;
            let player = w.res_mut::<Player>();
            player.controller.position = p.transform_point(seat);
            let fwd = p.rot * Vec3::Z;
            player.facing = fwd.x.atan2(fwd.z);
            let ch = player.controller.body;
            let pos = player.controller.position;
            w.res_mut::<PhysicsWorld>().set_body_iso(ch, Iso::from_pos(pos));
        }
        return;
    }
    if enter && !dead
        && let Some((i, d)) = near
        && d < 3.5
    {
        let ch = w.res::<Player>().controller.body;
        let collider = w.res::<PhysicsWorld>().body(ch).and_then(|b| b.colliders().first().copied());
        if let Some(c) = collider {
            w.res_mut::<PhysicsWorld>().set_collider_groups(c, CollisionGroups::new(0, 0));
        }
        w.res_mut::<Driving>().0 = Some(i);
        let name = w.res::<VehicleVisuals>().0[i].name.clone();
        w.res_mut::<Hud>().message(format!("Driving {name}"));
        return;
    }
    // Walk relative to the camera.
    let rot = Quat::from_rotation_y(cam_yaw);
    let fwd = rot * Vec3::NEG_Z;
    let right = rot * Vec3::X;
    let mut wish = fwd * intent.movement.y + right * intent.movement.x;
    if wish.length_squared() > 1.0 {
        wish = wish.normalize();
    }
    let speed = if dead { 0.0 } else if intent.sprint && !intent.aim { 6.5 } else if intent.aim { 2.2 } else { 3.6 };
    let target_vel = wish * speed;
    w.scope::<Player, _>(|w, player| {
    let physics = w.res_mut::<PhysicsWorld>();
    // Smooth acceleration so the blend space reads naturally.
    let current = Vec3::new(player.controller.velocity.x, 0.0, player.controller.velocity.z);
    let accel = if player.controller.state.grounded { 14.0 } else { 3.0 };
    let vel = current + (target_vel - current) * (accel * dt).min(1.0);
    player.controller.update(physics, vel, jump && !dead, dt);
    if intent.aim {
        player.facing = cam_yaw + std::f32::consts::PI;
    } else if vel.length_squared() > 0.2 {
        let target = vel.x.atan2(vel.z);
        let mut d = target - player.facing;
        while d > std::f32::consts::PI {
            d -= std::f32::consts::TAU;
        }
        while d < -std::f32::consts::PI {
            d += std::f32::consts::TAU;
        }
        player.facing += d * (10.0 * dt).min(1.0);
    }
    // Fell out of the world.
    if player.controller.position.y < -50.0 {
        let p = player.respawn.pos + Vec3::Y;
        player.controller.teleport(physics, p);
    }
    });
}

fn player_animate(w: &mut forja_core::World) {
    let dt = w.res::<Time>().delta;
    let aim = w.res::<Intent>().aim;
    let armed = w.res::<super::combat::Combat>().equipped.is_some();
    let driving = w.res::<Driving>().0.is_some();
    let player = w.res_mut::<Player>();
    let v = player.controller.velocity;
    let dead = player.health.is_dead();
    let anim = match &mut player.avatar.human {
        Some(h) => h.anim.as_mut(),
        None => player.avatar.anim.as_mut(),
    };
    if let Some(anim) = anim {
        anim.set("speed", Vec3::new(v.x, 0.0, v.z).length());
        anim.set("in_vehicle", driving as u32 as f32);
        anim.set("aiming", (aim || armed) as u32 as f32 * if driving { 0.0 } else { 1.0 });
        anim.set("dead", dead as u32 as f32);
        anim.update(dt);
    }
}

fn player_render(w: &mut forja_core::World) {
    let driving = w.res::<Driving>().0;
    let aim_dir = w.res::<RenderScene>().camera.forward();
    let aiming = w.res::<Intent>().aim && driving.is_none() && w.res::<super::combat::Combat>().equipped.is_some();
    let bike = driving.is_some_and(|i| w.res::<VehicleVisuals>().0[i].bike);
    let vehicle_rot = driving.and_then(|i| w.res::<PhysicsWorld>().body(w.res::<Vehicles>().0[i].body).map(|b| b.rotation()));
    let player = w.res_mut::<Player>();
    let feet = player.controller.feet();
    let rot = vehicle_rot.unwrap_or_else(|| Quat::from_rotation_y(player.facing));
    let transform = Mat4::from_rotation_translation(rot, feet) * Mat4::from_scale(Vec3::splat(player.avatar.scale));
    let mut item = RenderItem::new(player.avatar.model, transform);
    if let Some(h) = &player.avatar.human
        && let Some(anim) = &h.anim
    {
        let mut m = Vec::new();
        h.skeleton.model_matrices(anim.pose(), &mut m);
        if aiming {
            super::humans::aim_arm(h, &mut m, rot.inverse() * aim_dir, 1.0);
        }
        if let Some(hand) = player.avatar.hand {
            player.hand_world = transform * m[hand];
        }
        for (k, mm) in m.iter_mut().enumerate() {
            *mm *= h.skeleton.inverse_bind[k];
        }
        item.joints = Some(Arc::new(m));
    } else if let Some(anim) = &player.avatar.anim {
        let mut m = Vec::new();
        player.avatar.skeleton.model_matrices(anim.pose(), &mut m);
        if let Some(h) = player.avatar.hand {
            player.hand_world = transform * m[h];
        }
        item.nodes = Some(Arc::new(m));
    }
    let hidden = driving.is_some() && !bike;
    if !hidden {
        w.res_mut::<RenderScene>().dynamic.push(item);
    }
}
