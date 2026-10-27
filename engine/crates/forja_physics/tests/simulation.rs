//! End-to-end simulation tests. Each one builds a small world and checks a
//! physical property a player would notice when it breaks.

use std::sync::Arc;

use forja_math::{Iso, Quat, Ray, Vec2, Vec3, vec3};
use forja_physics::*;

const DT: f32 = 1.0 / 60.0;

fn ground_world() -> PhysicsWorld {
    let mut w = PhysicsWorld::default();
    w.add_collider(ColliderDesc::new(Shape::cuboid(vec3(100.0, 0.5, 100.0))).translation(vec3(0.0, -0.5, 0.0)), None);
    w
}

fn run(w: &mut PhysicsWorld, seconds: f32) {
    for _ in 0..(seconds / DT) as usize {
        w.step(DT);
    }
}

#[test]
fn sphere_falls_and_rests() {
    let mut w = ground_world();
    let (b, _) = w.add_dynamic(RigidBodyDesc::dynamic().position(vec3(0.0, 10.0, 0.0)), ColliderDesc::new(Shape::sphere(0.5)));
    // Free fall: y = 10 - g t^2 / 2 after 0.5 s.
    run(&mut w, 0.5);
    let y = w.body(b).unwrap().position().y;
    assert!((y - (10.0 - 0.5 * 9.81 * 0.25)).abs() < 0.1, "free fall y={y}");
    run(&mut w, 3.0);
    let body = w.body(b).unwrap();
    assert!((body.position().y - 0.5).abs() < 0.02, "rest y={}", body.position().y);
    assert!(body.linvel.length() < 0.05);
}

#[test]
fn box_stack_is_stable() {
    let mut w = ground_world();
    let mut boxes = Vec::new();
    for i in 0..8 {
        let (b, _) = w.add_dynamic(
            RigidBodyDesc::dynamic().position(vec3(0.0, 0.5 + i as f32 * 1.0, 0.0)),
            ColliderDesc::new(Shape::cuboid(Vec3::splat(0.5))),
        );
        boxes.push(b);
    }
    run(&mut w, 4.0);
    for (i, b) in boxes.iter().enumerate() {
        let p = w.body(*b).unwrap().position();
        assert!((p.y - (0.5 + i as f32)).abs() < 0.08, "box {i} at {p}");
        assert!(p.x.abs() < 0.05 && p.z.abs() < 0.05, "box {i} drifted to {p}");
    }
}

#[test]
fn bodies_fall_asleep_and_wake() {
    let mut w = ground_world();
    let (b, _) = w.add_dynamic(RigidBodyDesc::dynamic().position(vec3(0.0, 0.5, 0.0)), ColliderDesc::new(Shape::cuboid(Vec3::splat(0.5))));
    run(&mut w, 2.0);
    assert!(w.body(b).unwrap().is_sleeping());
    // A falling ball wakes it on impact.
    w.add_dynamic(RigidBodyDesc::dynamic().position(vec3(0.0, 3.0, 0.0)), ColliderDesc::new(Shape::sphere(0.3)));
    let mut woke = false;
    for _ in 0..90 {
        w.step(DT);
        woke |= !w.body(b).unwrap().is_sleeping();
    }
    assert!(woke);
}

fn slope_test(angle_deg: f32, friction: f32) -> f32 {
    let mut w = PhysicsWorld::default();
    let rot = Quat::from_rotation_z(angle_deg.to_radians());
    w.add_collider(ColliderDesc::new(Shape::cuboid(vec3(30.0, 0.5, 5.0))).local(Iso::new(Vec3::ZERO, rot)).friction(friction), None);
    let up = rot * Vec3::Y;
    let (b, _) = w.add_dynamic(
        RigidBodyDesc::dynamic().position(up * 1.0).rotation(rot),
        ColliderDesc::new(Shape::cuboid(Vec3::splat(0.5))).friction(friction),
    );
    let start = w.body(b).unwrap().position();
    run(&mut w, 2.0);
    (w.body(b).unwrap().position() - start).length()
}

#[test]
fn friction_holds_and_slides() {
    // tan(20 deg) = 0.36 < 0.6: static friction holds.
    assert!(slope_test(20.0, 0.6) < 0.05);
    // tan(40 deg) = 0.84 > 0.6: it slides.
    assert!(slope_test(40.0, 0.6) > 1.0);
}

#[test]
fn restitution_bounces() {
    let mut w = PhysicsWorld::default();
    w.add_collider(ColliderDesc::new(Shape::cuboid(vec3(10.0, 0.5, 10.0))).translation(vec3(0.0, -0.5, 0.0)).restitution(0.8), None);
    let (b, _) = w.add_dynamic(
        RigidBodyDesc::dynamic().position(vec3(0.0, 5.0, 0.0)),
        ColliderDesc::new(Shape::sphere(0.25)).restitution(0.8),
    );
    let mut max_after_bounce: f32 = 0.0;
    let mut bounced = false;
    for _ in 0..240 {
        w.step(DT);
        let body = w.body(b).unwrap();
        if body.linvel.y > 1.0 {
            bounced = true;
        }
        if bounced {
            max_after_bounce = max_after_bounce.max(body.position().y);
        }
    }
    // e = 0.8 -> rebound height ~ 0.64 * drop.
    assert!(bounced);
    assert!(max_after_bounce > 2.0 && max_after_bounce < 4.5, "rebound {max_after_bounce}");
}

#[test]
fn fast_ball_does_not_tunnel_thin_wall() {
    let mut w = PhysicsWorld::default();
    w.config.gravity = Vec3::ZERO;
    w.add_collider(ColliderDesc::new(Shape::cuboid(vec3(0.05, 5.0, 5.0))).translation(vec3(5.0, 0.0, 0.0)), None);
    let (b, _) = w.add_dynamic(RigidBodyDesc::dynamic().linvel(vec3(60.0, 0.0, 0.0)), ColliderDesc::new(Shape::sphere(0.2)));
    run(&mut w, 1.0);
    assert!(w.body(b).unwrap().position().x < 5.0, "tunnelled to {}", w.body(b).unwrap().position().x);
}

#[test]
fn capsule_on_heightfield_and_trimesh() {
    let mut w = PhysicsWorld::default();
    let n = 65;
    let heights: Vec<f32> = (0..n * n).map(|i| ((i % n) as f32 * 0.15).sin() * 0.5 + ((i / n) as f32 * 0.1).cos() * 0.5).collect();
    let hf = Arc::new(HeightField::new(n, n, heights, Vec2::splat(1.0)));
    w.add_collider(ColliderDesc::new(Shape::HeightField(hf.clone())), None);
    let mesh = TriMesh::new(
        vec![vec3(40.0, 2.0, -5.0), vec3(50.0, 2.0, -5.0), vec3(50.0, 2.0, 5.0), vec3(40.0, 2.0, 5.0)],
        vec![[0, 2, 1], [0, 3, 2]],
    );
    w.add_collider(ColliderDesc::new(Shape::TriMesh(Arc::new(mesh))), None);
    let (a, _) = w.add_dynamic(RigidBodyDesc::dynamic().position(vec3(3.3, 4.0, -2.7)), ColliderDesc::new(Shape::capsule(0.5, 0.3)));
    let (b, _) = w.add_dynamic(RigidBodyDesc::dynamic().position(vec3(45.0, 4.0, 0.0)), ColliderDesc::new(Shape::cuboid(Vec3::splat(0.4))));
    run(&mut w, 3.0);
    let pa = w.body(a).unwrap().position();
    let ground = hf.height_at(pa.x, pa.z).unwrap();
    assert!(pa.y > ground && pa.y < ground + 1.2, "capsule {pa} ground {ground}");
    let pb = w.body(b).unwrap().position();
    assert!((pb.y - 2.4).abs() < 0.05, "box on mesh {pb}");
}

#[test]
fn raycast_hits_closest() {
    let mut w = ground_world();
    let (b, c) = w.add_dynamic(RigidBodyDesc::dynamic().position(vec3(0.0, 1.0, 5.0)), ColliderDesc::new(Shape::cuboid(Vec3::splat(0.5))));
    w.step(DT);
    let hit = w.raycast(&Ray::new(vec3(0.0, 1.0, 0.0), Vec3::Z), 100.0, QueryFilter::default()).unwrap();
    assert_eq!(hit.collider, c);
    assert!((hit.t - 4.5).abs() < 0.02);
    assert!(hit.normal.z < -0.99);
    let hit = w.raycast(&Ray::new(vec3(0.0, 1.0, 0.0), Vec3::Z), 100.0, QueryFilter::exclude(b));
    assert!(hit.is_none());
}

#[test]
fn sensor_reports_overlap_events() {
    let mut w = ground_world();
    let sensor = w.add_collider(ColliderDesc::new(Shape::cuboid(vec3(2.0, 0.5, 2.0))).translation(vec3(0.0, 2.0, 0.0)).sensor(true), None);
    w.add_dynamic(RigidBodyDesc::dynamic().position(vec3(0.0, 4.0, 0.0)), ColliderDesc::new(Shape::sphere(0.3)));
    let mut started = false;
    let mut stopped = false;
    for _ in 0..120 {
        w.step(DT);
        for e in &w.events {
            match e {
                CollisionEvent::Started(a, b) if *a == sensor || *b == sensor => started = true,
                CollisionEvent::Stopped(a, b) if *a == sensor || *b == sensor => stopped = true,
                _ => {}
            }
        }
    }
    assert!(started && stopped);
}

#[test]
fn many_bodies_stay_fast() {
    let mut w = ground_world();
    for i in 0..400 {
        let p = vec3((i % 20) as f32 * 1.2 - 12.0, 0.6 + (i / 20) as f32 * 1.05, 0.0);
        let shape = if i % 2 == 0 { Shape::cuboid(Vec3::splat(0.5)) } else { Shape::sphere(0.5) };
        w.add_dynamic(RigidBodyDesc::dynamic().position(p), ColliderDesc::new(shape));
    }
    let t = std::time::Instant::now();
    run(&mut w, 1.0);
    let per_step = t.elapsed().as_secs_f32() * 1000.0 / 60.0;
    eprintln!("400 bodies: {per_step:.2} ms/step, {:?}", w.stats);
    // Generous bound so debug-ish CI machines pass; release is far faster.
    assert!(per_step < 40.0);
}

// ---------------------------------------------------------------------------
// Vehicles.

fn flat_track() -> PhysicsWorld {
    let mut w = PhysicsWorld::default();
    w.add_collider(ColliderDesc::new(Shape::cuboid(vec3(500.0, 0.5, 500.0))).translation(vec3(0.0, -0.5, 0.0)).friction(1.0), None);
    w
}

#[test]
fn car_settles_accelerates_brakes_and_turns() {
    let mut w = flat_track();
    let mut car = Vehicle::spawn(&mut w, VehicleConfig::sedan(), Iso::from_pos(vec3(0.0, 1.2, 0.0)));
    for _ in 0..120 {
        car.update(&mut w, DT);
        w.step(DT);
    }
    let b = w.body(car.body).unwrap();
    assert!(b.position().y > 0.3 && b.position().y < 1.0, "ride height {}", b.position().y);
    assert!((b.rotation() * Vec3::Y).y > 0.99, "car tilted");
    assert!(car.wheels.iter().all(|w| w.grounded));

    // Full throttle for 6 s.
    car.controls.throttle = 1.0;
    for _ in 0..360 {
        car.update(&mut w, DT);
        w.step(DT);
    }
    let v = car.speed(&w);
    assert!(v > 18.0, "too slow: {v} m/s, gear {}", car.gear);
    assert!(v < 60.0, "unrealistic: {v}");
    assert!(car.gear >= 2, "never shifted");
    let up = w.body(car.body).unwrap().rotation() * Vec3::Y;
    assert!(up.y > 0.95);

    // Turn at speed: heading changes, car stays upright.
    let heading0 = w.body(car.body).unwrap().rotation() * Vec3::Z;
    car.controls.throttle = 0.3;
    car.controls.steer = 1.0;
    for _ in 0..120 {
        car.update(&mut w, DT);
        w.step(DT);
    }
    let heading1 = w.body(car.body).unwrap().rotation() * Vec3::Z;
    assert!(heading0.dot(heading1) < 0.9, "did not turn");
    // Steering right turns toward -X (right in the glTF frame).
    assert!(heading1.x < heading0.x);
    assert!((w.body(car.body).unwrap().rotation() * Vec3::Y).y > 0.8, "rolled over in a turn");

    // Brake to a stop.
    car.controls = VehicleControls { brake: 1.0, ..Default::default() };
    for _ in 0..420 {
        car.update(&mut w, DT);
        w.step(DT);
    }
    assert!(w.body(car.body).unwrap().linvel.length() < 0.5, "did not stop: {}", w.body(car.body).unwrap().linvel.length());
}

#[test]
fn car_reverses() {
    let mut w = flat_track();
    let mut car = Vehicle::spawn(&mut w, VehicleConfig::sedan(), Iso::from_pos(vec3(0.0, 1.0, 0.0)));
    car.controls.throttle = -1.0;
    for _ in 0..180 {
        car.update(&mut w, DT);
        w.step(DT);
    }
    assert!(car.speed(&w) < -3.0, "reverse speed {}", car.speed(&w));
    assert_eq!(car.gear, -1);
}

#[test]
fn motorcycle_stays_upright_and_leans_into_turns() {
    let mut w = flat_track();
    let mut bike = Vehicle::spawn(&mut w, VehicleConfig::motorcycle(), Iso::from_pos(vec3(0.0, 0.9, 0.0)));
    for _ in 0..60 {
        bike.update(&mut w, DT);
        w.step(DT);
    }
    assert!((w.body(bike.body).unwrap().rotation() * Vec3::Y).y > 0.98, "bike fell while standing");
    bike.controls.throttle = 1.0;
    for _ in 0..240 {
        bike.update(&mut w, DT);
        w.step(DT);
    }
    let v = bike.speed(&w);
    assert!(v > 15.0, "bike speed {v}");
    assert!((w.body(bike.body).unwrap().rotation() * Vec3::Y).y > 0.95);
    bike.controls.throttle = 0.4;
    bike.controls.steer = 0.6;
    let mut max_lean: f32 = 0.0;
    for _ in 0..120 {
        bike.update(&mut w, DT);
        w.step(DT);
        let rot = w.body(bike.body).unwrap().rotation();
        let right = -(rot * Vec3::X);
        max_lean = max_lean.max(-right.y);
    }
    assert!(max_lean > 0.15, "bike should lean right into a right turn ({max_lean})");
    let up = w.body(bike.body).unwrap().rotation() * Vec3::Y;
    assert!(up.y > 0.5, "bike crashed in the turn");
}

// ---------------------------------------------------------------------------
// Character controller.

#[test]
fn character_walks_climbs_steps_and_is_blocked_by_walls() {
    let mut w = ground_world();
    // 0.3 m step at z = 3..6, then a wall at z = 10.
    w.add_collider(ColliderDesc::new(Shape::cuboid(vec3(5.0, 0.15, 1.5))).translation(vec3(0.0, 0.15, 4.5)), None);
    w.add_collider(ColliderDesc::new(Shape::cuboid(vec3(5.0, 2.0, 0.25))).translation(vec3(0.0, 2.0, 10.0)), None);
    let cfg = CharacterConfig::default();
    let stand = cfg.half_height + cfg.radius;
    let mut ch = CharacterController::new(&mut w, cfg, vec3(0.0, stand + 0.05, 0.0));
    for _ in 0..30 {
        ch.update(&mut w, Vec3::ZERO, false, DT);
        w.step(DT);
    }
    assert!(ch.state.grounded);
    assert!((ch.feet().y).abs() < 0.05, "feet {}", ch.feet().y);
    let mut max_feet: f32 = 0.0;
    for _ in 0..300 {
        ch.update(&mut w, vec3(0.0, 0.0, 4.0), false, DT);
        w.step(DT);
        if ch.position.z > 3.5 && ch.position.z < 5.5 {
            max_feet = max_feet.max(ch.feet().y);
        }
    }
    assert!(max_feet > 0.25, "did not climb the step (feet {max_feet})");
    assert!(ch.position.z < 10.0 - 0.25 - 0.3, "walked through the wall: z={}", ch.position.z);
    assert!(ch.position.z > 9.0, "stopped early: z={}", ch.position.z);
    // Jump.
    ch.update(&mut w, Vec3::ZERO, true, DT);
    let mut peak = ch.feet().y;
    for _ in 0..60 {
        ch.update(&mut w, Vec3::ZERO, false, DT);
        peak = peak.max(ch.feet().y);
    }
    assert!(peak > 1.0, "jump peak {peak}");
}

#[test]
fn character_pushes_crates() {
    let mut w = ground_world();
    let (crate_b, _) = w.add_dynamic(
        RigidBodyDesc::dynamic().position(vec3(0.0, 0.3, 2.0)),
        ColliderDesc::new(Shape::cuboid(Vec3::splat(0.3))).mass(15.0),
    );
    let cfg = CharacterConfig::default();
    let stand = cfg.half_height + cfg.radius;
    let mut ch = CharacterController::new(&mut w, cfg, vec3(0.0, stand, 0.0));
    for _ in 0..120 {
        ch.update(&mut w, vec3(0.0, 0.0, 3.0), false, DT);
        w.step(DT);
    }
    assert!(w.body(crate_b).unwrap().position().z > 2.5, "crate not pushed: {}", w.body(crate_b).unwrap().position());
}

// ---------------------------------------------------------------------------
// Regression tests for the physics upgrade (see docs/physics_research.md).

fn quad(v: &mut Vec<Vec3>, t: &mut Vec<[u32; 3]>, a: Vec3, b: Vec3, c: Vec3, d: Vec3) {
    let i = v.len() as u32;
    v.extend([a, b, c, d]);
    t.extend([[i, i + 1, i + 2], [i, i + 2, i + 3]]);
}

/// Road, 15 cm kerb, pavement, then ten 17 cm stairs, as one mesh along +X.
fn stairs_mesh() -> TriMesh {
    let (mut v, mut t) = (Vec::new(), Vec::new());
    let (z0, z1) = (-10.0, 10.0);
    let mut x = -10.0;
    let mut y = 0.0;
    let top = |v: &mut Vec<Vec3>, t: &mut Vec<[u32; 3]>, x0: f32, x1: f32, y: f32| quad(v, t, vec3(x0, y, z0), vec3(x0, y, z1), vec3(x1, y, z1), vec3(x1, y, z0));
    top(&mut v, &mut t, x, 0.0, y);
    x = 0.0;
    for (rise, run) in std::iter::once((0.15, 3.0)).chain(std::iter::repeat_n((0.17, 0.3), 10)) {
        quad(&mut v, &mut t, vec3(x, y, z0), vec3(x, y + rise, z0), vec3(x, y + rise, z1), vec3(x, y, z1));
        y += rise;
        top(&mut v, &mut t, x, x + run, y);
        x += run;
    }
    top(&mut v, &mut t, x, x + 10.0, y);
    TriMesh::new(v, t)
}

#[test]
fn fast_bodies_do_not_tunnel_thin_meshes_or_spin_through_walls() {
    for spin in [0.0, 20.0] {
        for shape in [Shape::sphere(0.1), Shape::cuboid(Vec3::splat(0.1)), Shape::capsule(0.2, 0.1)] {
            let mut w = PhysicsWorld::default();
            w.config.gravity = Vec3::ZERO;
            let (mut v, mut t) = (Vec::new(), Vec::new());
            quad(&mut v, &mut t, vec3(10.0, -5.0, -5.0), vec3(10.0, 5.0, -5.0), vec3(10.0, 5.0, 5.0), vec3(10.0, -5.0, 5.0));
            w.add_collider(ColliderDesc::new(Shape::TriMesh(Arc::new(TriMesh::new(v, t)))), None);
            w.add_collider(ColliderDesc::new(Shape::cuboid(vec3(0.025, 5.0, 5.0))).translation(vec3(-10.0, 0.0, 0.0)), None);
            let (a, _) = w.add_dynamic(RigidBodyDesc::dynamic().linvel(vec3(50.0, 0.0, 0.0)).angvel(vec3(0.0, 0.0, spin)), ColliderDesc::new(shape.clone()));
            let (b, _) = w.add_dynamic(RigidBodyDesc::dynamic().linvel(vec3(-50.0, 0.0, 0.0)).angvel(vec3(0.0, 0.0, spin)), ColliderDesc::new(shape.clone()));
            run(&mut w, 1.0);
            assert!(w.body(a).unwrap().position().x < 10.0, "{shape:?} spin {spin} went through the mesh");
            assert!(w.body(b).unwrap().position().x > -10.0, "{shape:?} spin {spin} went through the 5 cm wall");
        }
    }
}

#[test]
fn simulation_is_deterministic() {
    let build = || {
        let mut w = ground_world();
        let bodies: Vec<BodyHandle> = (0..30)
            .map(|i| {
                let p = vec3((i % 5) as f32 * 0.9 - 2.0, 0.5 + (i / 5) as f32 * 1.05, (i % 3) as f32 * 0.1);
                w.add_dynamic(RigidBodyDesc::dynamic().position(p).rotation(Quat::from_rotation_y(i as f32 * 0.3)), ColliderDesc::new(Shape::cuboid(Vec3::splat(0.45)))).0
            })
            .collect();
        run(&mut w, 3.0);
        bodies.iter().map(|b| w.body(*b).unwrap().position()).collect::<Vec<_>>()
    };
    assert_eq!(build(), build());
}

#[test]
fn character_climbs_kerbs_and_stairs_smoothly_and_stays_grounded_going_down() {
    let mut w = PhysicsWorld::default();
    w.add_collider(ColliderDesc::new(Shape::TriMesh(Arc::new(stairs_mesh()))), None);
    let cfg = CharacterConfig::default();
    let mut ch = CharacterController::new(&mut w, cfg, vec3(-3.0, 1.2, 0.0));
    for _ in 0..30 {
        ch.update(&mut w, Vec3::ZERO, false, DT);
    }
    let mut last = ch.smoothed_feet().y;
    let mut max_jump = 0.0f32;
    for _ in 0..240 {
        ch.update(&mut w, vec3(4.0, 0.0, 0.0), false, DT);
        assert!(ch.state.grounded, "lost the ground going up at x {}", ch.position.x);
        let y = ch.smoothed_feet().y;
        max_jump = max_jump.max((y - last).abs());
        last = y;
    }
    assert!(ch.feet().y > 1.8, "did not reach the top: {}", ch.feet().y);
    assert!(max_jump < 0.1, "visible step pops: {max_jump}");
    for _ in 0..240 {
        ch.update(&mut w, vec3(-4.0, 0.0, 0.0), false, DT);
        assert!(ch.state.grounded, "hopped going down at x {}", ch.position.x);
    }
    assert!(ch.feet().y < 0.05);
}

#[test]
fn character_respects_slope_limit_and_rides_platforms() {
    // A 60° ramp is a wall; a 30° one is walkable.
    for (deg, climbs) in [(30.0f32, true), (60.0, false)] {
        let mut w = ground_world();
        let rot = Quat::from_rotation_z(deg.to_radians());
        w.add_collider(ColliderDesc::new(Shape::cuboid(vec3(10.0, 0.5, 5.0))).local(Iso::new(vec3(10.0, 0.0, 0.0), rot)), None);
        let mut ch = CharacterController::new(&mut w, CharacterConfig::default(), vec3(-2.0, 1.0, 0.0));
        for _ in 0..240 {
            ch.update(&mut w, vec3(4.0, 0.0, 0.0), false, DT);
            w.step(DT);
        }
        assert_eq!(ch.feet().y > 1.5, climbs, "{deg}° slope: feet {}", ch.feet().y);
    }
    // Standing on a moving kinematic platform carries the character.
    let mut w = ground_world();
    let plat = w.add_body(RigidBodyDesc::kinematic().position(vec3(0.0, 0.5, 0.0)).linvel(vec3(2.0, 0.0, 0.0)));
    w.add_collider(ColliderDesc::new(Shape::cuboid(vec3(3.0, 0.25, 3.0))), Some(plat));
    let mut ch = CharacterController::new(&mut w, CharacterConfig::default(), vec3(0.0, 1.8, 0.0));
    for _ in 0..120 {
        ch.update(&mut w, Vec3::ZERO, false, DT);
        w.step(DT);
    }
    let px = w.body(plat).unwrap().position().x;
    assert!((ch.position.x - px).abs() < 0.3 && px > 3.0, "left behind: char {} platform {}", ch.position.x, px);
}

#[test]
fn parked_bike_rests_on_its_kickstand_and_ridden_bike_never_falls() {
    let mut w = flat_track();
    let mut bike = Vehicle::spawn(&mut w, VehicleConfig::motorcycle(), Iso::from_pos(vec3(0.0, 0.8, 0.0)));
    bike.rider = false;
    for _ in 0..600 {
        bike.update(&mut w, DT);
        w.step(DT);
    }
    let lean = bike.lean(&w).to_degrees();
    assert!((-25.0..-5.0).contains(&lean), "parked lean {lean} (should rest on the left stand)");
    assert!(w.body(bike.body).unwrap().is_sleeping(), "parked bike should sleep");
    // Ride a slalom.
    bike.rider = true;
    let mut max_lean = 0.0f32;
    for i in 0..1200 {
        let t = i as f32 * DT;
        bike.controls = VehicleControls { throttle: 0.6, steer: if t > 3.0 { (t * 0.8).sin() } else { 0.0 }, brake: 0.0, handbrake: false };
        bike.update(&mut w, DT);
        w.step(DT);
        max_lean = max_lean.max(bike.lean(&w).abs());
    }
    assert!(max_lean.to_degrees() > 20.0, "never leaned: {}", max_lean.to_degrees());
    assert!(max_lean.to_degrees() < 70.0, "fell while ridden: {}", max_lean.to_degrees());
    assert!(bike.speed(&w) > 8.0);
}

#[test]
fn parked_car_holds_on_a_hill_and_sleeps() {
    let mut w = PhysicsWorld::default();
    let rot = Quat::from_rotation_x(-12f32.to_radians());
    w.add_collider(ColliderDesc::new(Shape::cuboid(vec3(50.0, 0.5, 100.0))).local(Iso::new(Vec3::ZERO, rot)).friction(0.9), None);
    let mut car = Vehicle::spawn(&mut w, VehicleConfig::sedan(), Iso::new(rot * vec3(0.0, 1.4, 0.0), rot));
    car.controls.handbrake = true;
    for _ in 0..240 {
        car.update(&mut w, DT);
        w.step(DT);
    }
    let p0 = w.body(car.body).unwrap().position();
    for _ in 0..240 {
        car.update(&mut w, DT);
        w.step(DT);
    }
    let creep = (w.body(car.body).unwrap().position() - p0).length();
    assert!(creep < 0.02, "creeps {creep} m in 4 s");
    assert!(w.body(car.body).unwrap().is_sleeping(), "parked car should sleep");
}

#[test]
fn far_bodies_freeze_and_render_poses_interpolate() {
    let mut w = ground_world();
    w.config.active_radius = 50.0;
    w.set_focus(Some(Vec3::ZERO));
    let (near, _) = w.add_dynamic(RigidBodyDesc::dynamic().position(vec3(0.0, 5.0, 0.0)), ColliderDesc::new(Shape::sphere(0.5)));
    let (far, _) = w.add_dynamic(RigidBodyDesc::dynamic().position(vec3(80.0, 5.0, 0.0)), ColliderDesc::new(Shape::sphere(0.5)));
    w.step(DT);
    w.step(DT);
    assert!(w.body(far).unwrap().is_sleeping());
    assert_eq!(w.body(far).unwrap().position().y, 5.0);
    let b = w.body(near).unwrap();
    assert!(b.position().y < 5.0);
    let mid = b.interpolated_iso(0.5).pos.y;
    assert!(mid < b.prev_iso().pos.y && mid > b.position().y);
}
