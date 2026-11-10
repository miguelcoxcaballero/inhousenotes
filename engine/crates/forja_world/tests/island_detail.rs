//! The island's detail lots on the real map: every footprint near the
//! places the game is looked at becomes a valid lot with a street front, a
//! shell, or nothing else.
//!
//! `ISLA_DUMP=x,z` prints the lots of the cell at that point.

use std::path::PathBuf;
use std::sync::Arc;

use forja_city::lot::EdgeKind;
use forja_math::Vec2;
use forja_world::island::detail::{self, DetailIndex, Zone};
use forja_world::{IslandWorld, WorldSource};

fn island() -> Arc<forja_world::Island> {
    let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let w = IslandWorld { trees: 0.0, ..IslandWorld::new(&data) }.build().expect("island");
    w.island.expect("island runtime")
}

#[test]
fn footprints_become_lots_with_street_fronts() {
    let island = island();
    let index = DetailIndex::new(&island);
    // Old town, Eixample, Russafa, Cabanyal, a village, an industrial estate.
    let places = [(-243.0, -628.0, Zone::OldTown), (835.0, 280.0, Zone::Eixample), (565.0, 955.0, Zone::Russafa), (4060.0, -117.0, Zone::Cabanyal), (-9727.0, -4367.0, Zone::Village), (-2907.0, 970.0, Zone::Periphery)];
    let dump = std::env::var("ISLA_DUMP").ok().and_then(|v| {
        let c: Vec<f32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
        (c.len() == 2).then(|| Vec2::new(c[0], c[1]))
    });
    for (x, z, zone) in places {
        let c = detail::cell_of(Vec2::new(x, z));
        let (mut lots, mut shells, mut street, mut party, mut court) = (0, 0, 0.0, 0.0, 0.0);
        let (mut sliced, mut plinth) = (0, 0.0f32);
        let mut worst = (0u32, 0u32, 0usize);
        for dz in -2..=2 {
            for dx in -2..=2 {
                let (plans, rest) = detail::plan_cell(&island, &index, (c.0 + dx, c.1 + dz));
                shells += rest.len();
                for p in &plans {
                    p.lot.check().expect("a planned lot keeps the lot contract");
                    assert!(p.piece > 0 || p.lot.edges.iter().any(|e| e.is_street()), "a whole lot has a street front");
                    lots += 1;
                    sliced += (p.piece > 0) as usize;
                    for i in 0..p.lot.ring.len() {
                        if p.lot.edges[i].is_street() {
                            let (a, b) = p.lot.edge(i);
                            let d = (b - a).normalize_or_zero();
                            let out = Vec2::new(d.y, -d.x) * 0.6;
                            for q in [a + out, (a + b) * 0.5 + out, b + out] {
                                let h = p.lot.base - island.ground.height(q.x, q.y);
                                if h > plinth {
                                    plinth = h;
                                    worst = (island.buildings[p.building as usize].poly, p.piece, p.lot.ring.len());
                                }
                            }
                        }
                    }
                    for i in 0..p.lot.ring.len() {
                        let (a, b) = p.lot.edge(i);
                        match p.lot.edges[i] {
                            EdgeKind::Street | EdgeKind::Corner => street += a.distance(b),
                            EdgeKind::Party => party += a.distance(b),
                            EdgeKind::Courtyard => court += a.distance(b),
                        }
                    }
                }
            }
        }
        println!("{zone:?} at {x}, {z}: {sliced} of {lots} lots are slices of footprints on a slope, the worst plinth is {plinth:.2} m (footprint {}, slice {}, {} corners)", worst.0, worst.1, worst.2);
        // No lot stands more than a plinth over the ground at its fronts.
        assert!(plinth < 3.2, "{zone:?}: a lot stands {plinth:.2} m over its lowest front");
        println!("{zone:?} at {x}, {z}: {lots} lots, {shells} shells; walls: {street:.0} m street, {party:.0} m party or blind, {court:.0} m courtyard");
        assert_eq!(detail::zone_at(Vec2::new(x, z)), zone);
        assert!(lots >= 8, "{zone:?}: {lots} lots");
        assert!(lots * 3 >= shells, "{zone:?}: {lots} lots against {shells} shells");
        assert!(street > 100.0);
    }
    if let Some(p) = dump {
        let key = detail::cell_of(p);
        let (plans, rest) = detail::plan_cell(&island, &index, key);
        println!("cell {key:?}: {} lots, {} shells", plans.len(), rest.len());
        for pl in &plans {
            let b = &island.buildings[pl.building as usize];
            println!("lot poly {} centre {:.1},{:.1} storeys {} height {:.1} floor {:.2} base {:.2} zone {:?} frontage {:.1}", b.poly, b.centre.x, b.centre.y, b.storeys, b.height, b.floor, pl.lot.base, pl.zone, pl.frontage);
            for i in 0..pl.lot.ring.len() {
                let (a, c) = pl.lot.edge(i);
                let f = pl.lot.front(i);
                println!("   {:?} {:.1},{:.1} -> {:.1},{:.1} len {:.1} clear {:.1} walk {:.1} shops {:.2}", pl.lot.edges[i], a.x, a.y, c.x, c.y, a.distance(c), f.clear, f.walk, f.shops);
            }
        }
        for bi in rest {
            let b = &island.buildings[bi as usize];
            println!("shell poly {} centre {:.1},{:.1} storeys {} height {:.1}", b.poly, b.centre.x, b.centre.y, b.storeys, b.height);
        }
    }
}
