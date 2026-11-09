use std::path::PathBuf;

use forja_math::Vec2;

use super::data::{IslandData, PolyKind};
use super::ground::{Cover, Ground, SEA};

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")
}

fn island() -> (IslandData, Ground) {
    let d = IslandData::load(&data_dir().join("maps/isla")).expect("island data");
    let g = Ground::build(&d);
    (d, g)
}

#[test]
fn packed_map_is_complete() {
    let d = IslandData::load(&data_dir().join("maps/isla")).expect("island data");
    assert!(d.polys.iter().filter(|p| p.kind == PolyKind::Building).count() > 43_000);
    assert!(d.lines.len() > 45_000);
    assert_eq!(d.polys.iter().filter(|p| p.kind == PolyKind::Coast).count(), 1);
    assert_eq!(d.height.dm.len(), d.height.size * d.height.size);
    // The Mercat Central is where the landmark frame says it is.
    let m = d.polys.iter().find(|p| d.name(p.name) == "Mercat Central").expect("Mercat Central");
    let c = super::data::ring_centroid(&m.rings[0]);
    assert!(c.distance(Vec2::new(-142.0, -412.0)) < 60.0, "{c}");
}

#[test]
fn ground_is_real_in_valencia_and_wet_at_sea() {
    let (d, g) = island();
    assert!(d.dem.is_some(), "real terrain of Valencia");
    // The city centre is ~15 m above the sea (IGN MDT25: 13-17 m around
    // the Plaça de l'Ajuntament) and nearly level.
    for (x, z) in [(0.0, 0.0), (-140.0, -410.0), (300.0, 200.0), (-800.0, 600.0)] {
        let h = g.height(x, z);
        assert!((9.0..22.0).contains(&h), "height {h} at {x}, {z}");
        assert!((g.height(x + 30.0, z) - h).abs() < 0.6, "slope at {x}, {z}");
        assert!(g.coast_at(x, z) > 1000.0);
    }
    // The city falls gently to the port.
    assert!(g.height(3500.0, 900.0) < g.height(0.0, 0.0) - 5.0);
    // Far out at sea the bed is under the surface; the peaks are high.
    assert!(g.height(g.x0 + 40.0, g.z0 + 40.0) < SEA - 1.0);
    let top = g.h.iter().copied().fold(f32::MIN, f32::max);
    assert!((500.0..1500.0).contains(&top), "{top}");
    // The lagoon is water below its banks, above the sea.
    assert_eq!(g.cover_at(3064.0, 14461.0), Cover::Water);
    let lagoon = g.height(3064.0, 14461.0);
    assert!(lagoon > SEA && lagoon < super::ground::PLAIN - 1.0, "{lagoon}");
    // Every building stands on land above the sea.
    for p in d.polys.iter().filter(|p| p.kind == PolyKind::Building).step_by(37) {
        let c = super::data::ring_centroid(&p.rings[0]);
        assert!(g.height(c.x, c.y) > SEA + 0.3, "building at {c} under the sea");
    }
}

#[test]
fn every_tile_meshes_within_the_height_field() {
    let (_, g) = island();
    let (tx, tz) = g.tiles();
    let mut tiles = 0;
    for z in (0..tz).step_by(3) {
        for x in (0..tx).step_by(3) {
            let parts = g.tile_meshes(x, z);
            assert_eq!(parts.is_empty(), g.far_mesh(x, z).is_none());
            for (_, m) in &parts {
                tiles += 1;
                assert_eq!(m.indices.len() % 3, 0);
                assert!(m.indices.iter().all(|i| (*i as usize) < m.positions.len()));
                // Surface vertices sit on the collision surface (skirts hang below).
                for p in m.positions.iter().step_by(11) {
                    let h = g.height(p.x, p.z);
                    assert!(p.y <= h + 2e-3 && p.y >= h - 30.1, "{p} vs {h}");
                }
            }
        }
    }
    assert!(tiles > 200);
}
