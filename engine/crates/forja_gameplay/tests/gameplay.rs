use std::path::PathBuf;

use forja_gameplay::*;
use forja_math::{Iso, Vec3, vec3};
use forja_physics::{ColliderDesc, PhysicsWorld, RigidBodyDesc, Shape, Vehicle};

fn data() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")
}

fn registries() -> (ItemRegistry, WeaponRegistry) {
    (ItemRegistry::load(&data().join("items")).unwrap(), WeaponRegistry::load(&data().join("weapons")).unwrap())
}

#[test]
fn data_files_are_consistent() {
    let (items, weapons) = registries();
    assert!(items.len() >= 10 && weapons.len() >= 5);
    for it in items.iter() {
        if let Some(w) = &it.weapon {
            let def = weapons.get(w).unwrap_or_else(|| panic!("item {} -> unknown weapon {w}", it.id));
            if let Some(a) = &def.ammo {
                assert!(items.get(a).is_some(), "weapon {} uses unknown ammo {a}", def.id);
            }
        }
    }
    let handling = HandlingRegistry::load(&data().join("handling")).unwrap();
    for name in ["sedan", "sports", "suv", "truck", "sport_bike"] {
        assert!(handling.get(name).is_some(), "missing handling {name}");
    }
    assert_eq!(handling.get("sport_bike").unwrap().wheels.len(), 2);
    assert!(handling.get("sport_bike").unwrap().bike.is_some());
}

#[test]
fn inventory_stacks_and_respects_weight() {
    let (items, _) = registries();
    let mut inv = Inventory::new(6, 10.0);
    assert_eq!(inv.add(&items, "ammo_9mm", 300), 0);
    assert_eq!(inv.count("ammo_9mm"), 300);
    assert_eq!(inv.slots.iter().flatten().count(), 2, "250 + 50");
    // 4 kg armor + 2.4 kg ammo + rifle 4.1 > 10 kg: rifle rejected.
    assert_eq!(inv.add(&items, "body_armor", 1), 0);
    assert_eq!(inv.add(&items, "rifle", 1), 1);
    assert!(inv.weight(&items) <= 10.0);
    assert_eq!(inv.remove("ammo_9mm", 260), 260);
    assert_eq!(inv.count("ammo_9mm"), 40);
    assert_eq!(inv.add(&items, "unknown", 3), 3);
    let slot = inv.slots.iter().position(|s| s.as_ref().is_some_and(|s| s.item == "ammo_9mm")).unwrap();
    let new = inv.split(slot, 15).unwrap();
    assert_eq!(inv.slots[new].as_ref().unwrap().count, 15);
    inv.compact(&items);
    assert_eq!(inv.slots.iter().flatten().filter(|s| s.item == "ammo_9mm").count(), 1);
    // Full slots: leftover returned.
    let mut small = Inventory::new(1, 100.0);
    assert_eq!(small.add(&items, "medkit", 7), 2);
}

#[test]
fn weapon_wheel_cycles_categories() {
    let (items, weapons) = registries();
    let mut inv = Inventory::new(10, 50.0);
    for w in ["pistol", "blaster_smg", "carbine", "rifle"] {
        inv.add(&items, w, 1);
    }
    let mut wheel = WeaponWheel::default();
    wheel.refresh(&inv, &items, &weapons);
    assert_eq!(wheel.select_category(WeaponCategory::Pistol), Some("service_pistol"));
    assert_eq!(wheel.select_category(WeaponCategory::Rifle), Some("carbine"));
    assert_eq!(wheel.select_category(WeaponCategory::Shotgun), None);
    assert_eq!(wheel.active_weapon(), Some("carbine"));
    inv.remove("carbine", 1);
    wheel.refresh(&inv, &items, &weapons);
    assert_eq!(wheel.active_weapon(), None, "dropped weapon must be unequipped");
}

fn range_world() -> (PhysicsWorld, forja_physics::BodyHandle) {
    let mut w = PhysicsWorld::default();
    w.config.gravity = Vec3::ZERO;
    let (b, _) = w.add_dynamic(RigidBodyDesc::dynamic().position(vec3(0.0, 0.0, 20.0)), ColliderDesc::new(Shape::cuboid(Vec3::splat(1.0))).mass(10.0));
    w.step(1.0 / 60.0);
    (w, b)
}

#[test]
fn pistol_fires_at_its_rate_and_reloads() {
    let (_, weapons) = registries();
    let (mut world, target) = range_world();
    let mut gun = WeaponState::new(weapons.get("service_pistol").unwrap().clone());
    // Semi-auto: holding the trigger fires once.
    let mut shots = 0;
    for _ in 0..30 {
        gun.tick(1.0 / 60.0, 100);
        shots += gun.trigger(&mut world, true, true, Vec3::ZERO, Vec3::Z, None).1;
    }
    assert_eq!(shots, 1);
    // Tapping fires up to the magazine.
    let mut hits = 0;
    for f in 0..600 {
        gun.tick(1.0 / 60.0, 100);
        let (h, _) = gun.trigger(&mut world, f % 2 == 0, true, Vec3::ZERO, Vec3::Z, None);
        hits += h.iter().filter(|h| h.body == target).count();
    }
    assert_eq!(gun.in_mag, 0);
    assert!(hits >= 9, "aimed pistol at 19 m should mostly hit: {hits}");
    assert!(world.body(target).unwrap().linvel.z > 0.0, "hits push the target");
    assert!(gun.start_reload(100));
    let mut taken = 0;
    for _ in 0..120 {
        taken += gun.tick(1.0 / 60.0, 100);
    }
    assert_eq!((gun.in_mag, taken), (12, 12));
    assert!(!gun.start_reload(0) || gun.in_mag < 12);
}

#[test]
fn smg_auto_rate_shotgun_pellets_and_falloff() {
    let (_, weapons) = registries();
    let (mut world, _) = range_world();
    let mut smg = WeaponState::new(weapons.get("compact_smg").unwrap().clone());
    let mut shots = 0;
    for _ in 0..60 {
        smg.tick(1.0 / 60.0, 0);
        shots += smg.trigger(&mut world, true, false, Vec3::ZERO, Vec3::Z, None).1;
    }
    // 900 rpm = 15 rounds per second exactly.
    assert!((15..=16).contains(&shots), "smg shots per second {shots}");
    // Faster than the tick rate still fires at the right rate.
    let mut fast = WeaponState::new(WeaponDef { rpm: 6000.0, magazine: 1000, ..weapons.get("compact_smg").unwrap().clone() });
    let mut n = 0;
    for _ in 0..60 {
        fast.tick(1.0 / 60.0, 0);
        n += fast.trigger(&mut world, true, false, Vec3::ZERO, Vec3::Z, None).1;
    }
    assert!((99..=101).contains(&n), "6000 rpm over 1 s fired {n}");
    // Burst fires exactly three per pull.
    let mut burst = WeaponState::new(weapons.get("carbine").unwrap().clone());
    let mut b = 0;
    for f in 0..30 {
        burst.tick(1.0 / 60.0, 0);
        b += burst.trigger(&mut world, f < 2, true, Vec3::ZERO, Vec3::Z, None).1;
    }
    assert_eq!(b, 3);
    let mut sg = WeaponState::new(weapons.get("pump_shotgun").unwrap().clone());
    let (hits, _) = sg.trigger(&mut world, true, true, Vec3::ZERO, Vec3::Z, None);
    assert!(hits.len() >= 4, "most of 8 pellets hit a 2 m box at 19 m: {}", hits.len());
    for h in &hits {
        assert!(h.damage < sg.def.damage, "shotgun falls off beyond 8 m");
    }
}

#[test]
fn car_concept_wheels_are_grouped() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/downloaded/khronos_car_concept/CarConcept.glb");
    if !path.exists() {
        return;
    }
    let scene = forja_assets::load_gltf(&path).unwrap();
    let bound = bind_vehicle_model(forja_physics::VehicleConfig::sedan(), &scene, 1.0);
    assert_eq!(bound.config.wheels.len(), 4, "tyre, rim and brake parts form 4 wheels");
    assert!(bound.wheel_nodes.iter().all(|w| w.len() >= 3));
    assert!(bound.config.wheels.iter().filter(|w| w.steer).all(|w| w.position.z > 0.5));
    assert!(bound.config.wheels.iter().all(|w| w.radius > 0.25 && w.radius < 0.5), "{:?}", bound.config.wheels.iter().map(|w| w.radius).collect::<Vec<_>>());
    // The unnamed tyre meshes under `WheelFrontL` etc. turn with their
    // wheel (they were left in the body, frozen at the rest pose).
    let world = scene.world_matrices();
    for (w, nodes) in bound.wheel_nodes.iter().enumerate() {
        assert!(nodes.iter().any(|&n| scene.nodes[n].name.is_empty()), "wheel {w} has its tyre: {:?}", nodes.iter().map(|&n| &scene.nodes[n].name).collect::<Vec<_>>());
        assert!(bound.wheel_calipers[w].iter().all(|&n| scene.nodes[n].name.contains("BrakePad")));
    }
    // The model is posed with the front wheels steered 30 degrees; the
    // binding undoes it (rears are straight).
    for (w, q) in bound.wheel_rest.iter().enumerate() {
        let (axis, angle) = q.to_axis_angle();
        let front = bound.config.wheels[w].steer;
        if front {
            assert!((angle.to_degrees() - 30.0).abs() < 3.0 && axis.y.abs() > 0.99, "front wheel {w}: {axis} {}", angle.to_degrees());
        } else {
            assert!(angle.to_degrees() < 3.0, "rear wheel {w}: {}", angle.to_degrees());
        }
        // Straightened, the tyre's axle lies along X.
        let pts: Vec<_> = bound.wheel_nodes[w]
            .iter()
            .flat_map(|&n| {
                let m = world[n];
                scene.meshes[scene.nodes[n].mesh.unwrap()].iter().flat_map(move |p| p.positions.iter().map(move |v| m.transform_point3(*v)))
            })
            .map(|p| *q * p)
            .collect();
        assert_eq!(forja_gameplay::straighten_yaw(&pts), forja_math::Quat::IDENTITY, "wheel {w} straight");
    }
}

#[test]
fn kenney_suv_spare_wheel_is_body() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/downloaded/kenney_car_kit/suv.glb");
    if !path.exists() {
        return;
    }
    let scene = forja_assets::load_gltf(&path).unwrap();
    let bound = bind_vehicle_model(forja_physics::VehicleConfig::sedan(), &scene, 1.8);
    assert_eq!(bound.config.wheels.len(), 4, "the spare on the tailgate is not a road wheel");
    assert!(bound.wheel_rest.iter().all(|q| *q == forja_math::Quat::IDENTITY), "Kenney wheels are modelled straight");
}

#[test]
fn health_and_armor() {
    let mut h = Health { armor: 50.0, ..Default::default() };
    assert!((h.damage(40.0) - 12.0).abs() < 1e-4);
    assert!((h.armor - 22.0).abs() < 1e-4);
    h.damage(1000.0);
    assert!(h.is_dead());
    h.heal(50.0);
    assert!(h.is_dead(), "dead stays dead");
}

#[test]
fn kenney_sedan_binds_and_drives() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/downloaded/kenney_car_kit/sedan.glb");
    if !path.exists() {
        return;
    }
    let scene = forja_assets::load_gltf(&path).unwrap();
    let handling = HandlingRegistry::load(&data().join("handling")).unwrap();
    let bound = bind_vehicle_model(handling.get("sedan").unwrap().clone(), &scene, 1.8);
    let cfg = bound.config;
    assert_eq!(cfg.wheels.len(), 4);
    assert_eq!(cfg.wheels.iter().filter(|w| w.steer).count(), 2);
    assert!(cfg.wheels.iter().filter(|w| w.steer).all(|w| w.position.z > 0.0), "front wheels must be at +Z");
    let mut world = PhysicsWorld::default();
    world.add_collider(ColliderDesc::new(Shape::cuboid(vec3(200.0, 0.5, 200.0))).translation(vec3(0.0, -0.5, 0.0)), None);
    let mut car = Vehicle::spawn(&mut world, cfg, Iso::from_pos(vec3(0.0, 1.5, 0.0)));
    car.controls.throttle = 1.0;
    for _ in 0..240 {
        car.update(&mut world, 1.0 / 60.0);
        world.step(1.0 / 60.0);
    }
    assert!(car.speed(&world) > 10.0, "bound sedan speed {}", car.speed(&world));
    assert!((world.body(car.body).unwrap().rotation() * Vec3::Y).y > 0.95);
}
