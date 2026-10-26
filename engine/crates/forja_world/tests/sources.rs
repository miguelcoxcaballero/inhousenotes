//! Builds every world source and drops a car and a character into it.

use std::path::PathBuf;

use forja_math::{Iso, Vec3};
use forja_physics::{CharacterConfig, CharacterController, PhysicsWorld, Vehicle, VehicleConfig};
use forja_world::*;

fn downloaded() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/downloaded")
}

/// Car and character must rest on the ground near their spawns.
fn smoke(w: &WorldDescription) {
    let mut pw = PhysicsWorld::default();
    w.add_to_physics(&mut pw);
    let spawn = w.spawn(SpawnKind::Vehicle).unwrap_or(Iso::from_pos(Vec3::Y));
    let mut car = Vehicle::spawn(&mut pw, VehicleConfig::sedan(), Iso::new(spawn.pos + Vec3::Y, spawn.rot));
    let p = w.spawn(SpawnKind::Player).unwrap();
    let mut ch = CharacterController::new(&mut pw, CharacterConfig::default(), p.pos + Vec3::Y);
    for _ in 0..180 {
        car.update(&mut pw, 1.0 / 60.0);
        ch.update(&mut pw, Vec3::ZERO, false, 1.0 / 60.0);
        pw.step(1.0 / 60.0);
    }
    let cb = pw.body(car.body).unwrap();
    assert!(cb.linvel.length() < 0.5, "{}: car still moving {}", w.name, cb.linvel);
    assert!(cb.position().y - spawn.pos.y < 2.0 && cb.position().y > spawn.pos.y - 3.0, "{}: car fell to {}", w.name, cb.position());
    assert!(ch.state.grounded, "{}: character not grounded at {}", w.name, ch.position);
}

#[test]
fn test_track() {
    let w = TestTrack::default().build().unwrap();
    assert!(w.triangle_count() > 100);
    smoke(&w);
}

#[test]
fn osm_miami_beach() {
    let p = downloaded().join("map_miami_beach_osm/map.osm");
    if !p.exists() {
        return;
    }
    let t = std::time::Instant::now();
    let w = OsmWorld::new(&p).build().unwrap();
    eprintln!("osm built in {:?}: {:?}, {} tris", t.elapsed(), w.stats, w.triangle_count());
    assert!(w.stats.iter().any(|(k, v)| k == "buildings" && *v > 500));
    smoke(&w);
}

#[test]
fn hollywood_terrain() {
    let dir = downloaded().join("map_hollywood_hills_terrain");
    if !dir.exists() {
        return;
    }
    let w = TerrainWorld::from_dir(&dir, 12, 2, 34.12).build().unwrap();
    let b = w.bounds();
    eprintln!("terrain bounds {:?} relief {:.0} m", b, b.max.y - b.min.y);
    assert!(b.max.y - b.min.y > 200.0, "Hollywood Hills should have relief");
    smoke(&w);
}

#[test]
fn kit_city() {
    if !downloaded().join("kenney_city_roads").exists() {
        return;
    }
    let w = KitCity::new(downloaded()).build().unwrap();
    eprintln!("kit city: {} placements, {} scenes, {} tris", w.placements.len(), w.scenes.len(), w.triangle_count());
    assert!(w.placements.len() > 400);
    smoke(&w);
}

#[test]
fn gltf_level() {
    let p = downloaded().join("kenney_city_commercial/building-skyscraper-a.glb");
    if !p.exists() {
        return;
    }
    let w = GltfWorld::new(&p, 20.0).build().unwrap();
    assert_eq!(w.colliders.len(), 1);
}
