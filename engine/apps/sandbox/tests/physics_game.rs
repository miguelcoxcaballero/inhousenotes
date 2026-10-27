//! The game's physics integration on the procedural test track: parked
//! bikes rest on their kickstands, the player stands, cars drive, and the
//! fixed steps stay cheap.

use std::path::PathBuf;

use forja_math::Vec3;
use forja_physics::{PhysicsWorld, Vehicles};
use sandbox::game::{self, Intent, MapKind, Paths};

#[test]
fn track_physics_parks_bikes_and_drives_cars() {
    let root = forja_assets::manifest::find_engine_root().unwrap_or_else(|| PathBuf::from("."));
    let paths = Paths { data: root.join("data"), downloaded: root.join("assets/downloaded") };
    let mut app = game::build(MapKind::Track, paths);
    for _ in 0..360 {
        app.update_with_dt(1.0 / 60.0);
    }
    {
        let physics = app.world.res::<PhysicsWorld>();
        let vehicles = &app.world.res::<Vehicles>().0;
        assert!(!vehicles.is_empty());
        for v in vehicles {
            let b = physics.body(v.body).expect("vehicle body");
            if v.is_bike() {
                assert!(!v.rider, "nobody rides the parked bike");
                let lean = v.lean(physics).to_degrees();
                assert!((-25.0..-3.0).contains(&lean), "parked bike lean {lean}° (should rest on its stand)");
            } else {
                assert!((b.rotation() * Vec3::Y).y > 0.95, "parked car tilted");
            }
        }
        assert!(physics.stats.step_ms < 5.0, "step {} ms", physics.stats.step_ms);
    }
    let player = app.world.res::<game::player::Player>();
    assert!(player.controller.state.grounded, "player not standing");
    // Drive the first car.
    let car = app.world.res::<Vehicles>().0.iter().position(|v| !v.is_bike()).expect("a car");
    app.world.res_mut::<game::vehicles::Driving>().0 = Some(car);
    // As entering does: the capsule rides along without colliding.
    let ch = app.world.res::<game::player::Player>().controller.body;
    let collider = app.world.res::<PhysicsWorld>().body(ch).and_then(|b| b.colliders().first().copied()).expect("capsule");
    app.world.res_mut::<PhysicsWorld>().set_collider_groups(collider, forja_physics::CollisionGroups::new(0, 0));
    for _ in 0..240 {
        app.world.res_mut::<Intent>().movement = forja_math::Vec2::new(0.0, 1.0);
        app.update_with_dt(1.0 / 60.0);
    }
    let speed = {
        let physics = app.world.res::<PhysicsWorld>();
        app.world.res::<Vehicles>().0[car].speed(physics)
    };
    assert!(speed > 8.0, "car speed {speed}");
}
