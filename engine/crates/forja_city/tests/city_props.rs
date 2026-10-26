//! Properties of realized cities (meshes, furniture, spawns) on a grid
//! city, an organic old town and a narrow-alley old town.

use std::sync::OnceLock;

use forja_city::dress::ItemKind;
use forja_city::kit::Kit;
use forja_city::layout::{CityConfig, CityLayout, build_city};
use forja_city::plan::{End, Located};
use forja_city::profile::{Class, ProfileLibrary};
use forja_city::style::StyleLibrary;
use forja_city::texgen::SignKind;
use forja_math::Vec2;

struct City {
    name: &'static str,
    lay: CityLayout,
    public: Located,
}

fn cities() -> &'static [City] {
    static C: OnceLock<Vec<City>> = OnceLock::new();
    C.get_or_init(|| {
        let (kit, styles) = (Kit::load(None), StyleLibrary::builtin());
        ["small", "carmen", "austrias"]
            .into_iter()
            .map(|name| {
                let lay = build_city(&kit, &styles, &CityConfig::preset(name).unwrap());
                let public = Located::new(lay.fabric.public.clone());
                City { name, lay, public }
            })
            .collect()
    })
}

fn xz(p: forja_math::Vec3) -> Vec2 {
    Vec2::new(p.x, p.z)
}

/// The street whose space `p` is in: the one whose centreline is nearest
/// relative to its half width (None outside every street).
fn street_at(c: &City, p: Vec2) -> Option<(u32, f32)> {
    let g = &c.lay.fabric.graph;
    g.near(p, 30.0)
        .into_iter()
        .map(|(s, t, o)| (s, o.abs() / (g.street(s).width_at(t) * 0.5).max(0.1)))
        .filter(|(_, r)| *r < 1.0)
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

#[test]
fn all_geometry_is_finite() {
    for c in cities() {
        let o = &c.lay.out;
        assert!(o.placements.iter().all(|(_, m)| m.to_cols_array().iter().all(|v| v.is_finite())), "{}: placement", c.name);
        assert!(o.col_verts.iter().all(|v| v.is_finite()), "{}: collider", c.name);
        assert!(o.lights.iter().all(|(p, col)| p.is_finite() && col.is_finite()), "{}: light", c.name);
        for s in &o.scenes {
            let b = s.bounds();
            assert!(b.min.is_finite() && b.max.is_finite(), "{}: scene bounds", c.name);
        }
        assert!(o.buildings > 40, "{}: {} buildings", c.name, o.buildings);
    }
}

#[test]
fn furniture_is_on_walkable_paving() {
    for c in cities() {
        let g = &c.lay.fabric.ground;
        let mut bad = Vec::new();
        for it in &c.lay.items {
            let p = xz(it.pos);
            match it.kind {
                // Plates hang on corner walls; catenaries span between facades.
                ItemKind::Plate(_) | ItemKind::Catenary { .. } => continue,
                _ => {}
            }
            if !c.public.contains(p) {
                bad.push(format!("{:?} at {p:?} off public space", it.kind));
                continue;
            }
            let roadway_ok = matches!(it.kind, ItemKind::Container(_) | ItemKind::RoadLight | ItemKind::Palm);
            if !roadway_ok && g.in_carriage(p) {
                bad.push(format!("{:?} at {p:?} on the roadway", it.kind));
            }
        }
        println!("{}: {} items, {} misplaced {:?}", c.name, c.lay.items.len(), bad.len(), &bad[..bad.len().min(4)]);
        assert!(bad.is_empty(), "{}: {} of {} items misplaced, e.g. {:?}", c.name, bad.len(), c.lay.items.len(), &bad[..bad.len().min(8)]);
    }
}

#[test]
fn alleys_have_no_traffic_dressing() {
    let lib = ProfileLibrary::builtin();
    for c in cities() {
        let gr = &c.lay.fabric.graph;
        let mut bad = Vec::new();
        for it in &c.lay.items {
            let traffic = match it.kind {
                ItemKind::TrafficLight { .. } | ItemKind::RoadLight | ItemKind::Container(_) | ItemKind::Tree => true,
                ItemKind::Lamp { .. } => true,
                ItemKind::Sign(k) => !matches!(k, SignKind::NoEntry),
                _ => false,
            };
            if !traffic {
                continue;
            }
            let p = xz(it.pos);
            if let Some((s, _)) = street_at(c, p) {
                let st = gr.street(s);
                let prof = lib.get(st.profile);
                // Inside a street without cars (and not at its mouth onto one).
                let at_mouth = [End::A, End::B].iter().any(|&e| gr.nodes[st.node(e) as usize].p.distance(p) < st.width_at(st.end_s(e)) * 0.5 + 8.0);
                if !prof.vehicles && st.carriageway <= 0.0 && prof.class == Class::Alley && !at_mouth {
                    bad.push(format!("{:?} in {} at {p:?}", it.kind, prof.id));
                }
            }
        }
        assert!(bad.is_empty(), "{}: traffic dressing in alleys: {:?}", c.name, &bad[..bad.len().min(8)]);
    }
    // The old towns close their alleys to cars with bollards at the mouths.
    for c in cities().iter().filter(|c| c.name != "small") {
        let n = c.lay.items.iter().filter(|i| matches!(i.kind, ItemKind::Bollard { .. })).count();
        println!("{}: {n} bollards", c.name);
        assert!(n > 10, "{}: {n} bollards", c.name);
    }
}

#[test]
fn junction_control_is_consistent() {
    let lib = ProfileLibrary::builtin();
    for c in cities() {
        let g = &c.lay.fabric.graph;
        let car_arms = |n: u32| g.star(n).iter().filter(|a| lib.get(g.street(a.0).profile).vehicles && g.street(a.0).carriageway > 0.0).count();
        let controlled: Vec<Vec2> = (0..g.nodes.len() as u32).filter(|&n| car_arms(n) >= 3).map(|n| g.nodes[n as usize].p).collect();
        for it in &c.lay.items {
            let control = matches!(it.kind, ItemKind::TrafficLight { .. } | ItemKind::Sign(SignKind::Stop) | ItemKind::Sign(SignKind::Yield));
            if control {
                let p = xz(it.pos);
                let d = controlled.iter().map(|q| q.distance(p)).fold(f32::INFINITY, f32::min);
                assert!(d < 40.0, "{}: {:?} at {p:?} is {d:.0} m from any junction of three car streets", c.name, it.kind);
            }
        }
        // Most such junctions get some control (a few have no paved spot).
        let with = controlled
            .iter()
            .filter(|q| c.lay.items.iter().any(|it| matches!(it.kind, ItemKind::TrafficLight { .. } | ItemKind::Sign(_)) && xz(it.pos).distance(**q) < 40.0))
            .count();
        println!("{}: {with} of {} junctions controlled", c.name, controlled.len());
        assert!(with * 10 >= controlled.len() * 7, "{}: {with} of {} junctions controlled", c.name, controlled.len());
    }
}

#[test]
fn spawns_on_vehicle_streets_only() {
    for c in cities() {
        let g = &c.lay.fabric.ground;
        assert!(c.lay.spawns.iter().any(|s| !s.vehicle), "{}: no player spawn", c.name);
        for s in c.lay.spawns.iter().filter(|s| s.vehicle) {
            assert!(g.in_carriage(xz(s.pos)), "{}: car spawn at {:?} off the roadway", c.name, s.pos);
        }
        let player = c.lay.spawns.iter().find(|s| !s.vehicle).unwrap();
        assert!(c.public.contains(xz(player.pos)), "{}: player spawns inside a building", c.name);
    }
}

#[test]
fn street_plates_at_junctions() {
    for c in cities() {
        let g = &c.lay.fabric.graph;
        let plates: Vec<Vec2> = c.lay.items.iter().filter(|i| matches!(i.kind, ItemKind::Plate(_))).map(|i| xz(i.pos)).collect();
        let nodes: Vec<Vec2> = (0..g.nodes.len() as u32).filter(|&n| g.degree(n) >= 3).map(|n| g.nodes[n as usize].p).collect();
        let named = nodes.iter().filter(|q| plates.iter().any(|p| p.distance(**q) < 25.0)).count();
        println!("{}: plates at {named} of {} junctions", c.name, nodes.len());
        assert!(named * 10 >= nodes.len() * 8, "{}: plates at {named} of {} junctions", c.name, nodes.len());
    }
}

#[test]
fn realize_is_deterministic() {
    let (kit, styles) = (Kit::load(None), StyleLibrary::builtin());
    let a = &cities()[0].lay;
    let b = build_city(&kit, &styles, &CityConfig::small());
    assert_eq!(a.fabric.hash64(), b.fabric.hash64());
    assert_eq!(a.out.placements.len(), b.out.placements.len());
    assert_eq!(a.items.len(), b.items.len());
    for (x, y) in a.out.placements.iter().zip(&b.out.placements) {
        assert_eq!(x.0, y.0);
        assert_eq!(x.1, y.1);
    }
}

#[test]
fn alley_fronts_are_tight() {
    // Facades on narrow streets learn it through their fronts: no awnings
    // or miradores, shallow projections, lanterns on the walls.
    for c in cities().iter().filter(|c| c.name != "small") {
        let (mut narrow, mut lit) = (0, 0);
        for l in &c.lay.fabric.lots {
            for (k, f) in l.fronts.iter().enumerate() {
                if !l.edges[k].is_street() || f.clear >= 4.0 || f.open {
                    continue;
                }
                narrow += 1;
                lit += f.lamps.is_some() as usize;
                assert!(!f.awnings && !f.miradores && f.reach <= 0.12 * f.clear + 1e-4, "{}: loose alley front {f:?}", c.name);
                assert!(f.max_height <= 4.5 * f.clear + 1e-3 && f.portal <= 1.4, "{}: {f:?}", c.name);
            }
        }
        println!("{}: {narrow} street edges on < 4 m, {lit} lit from the walls", c.name);
        assert!(narrow > 50 && lit * 2 > narrow, "{}: {narrow} narrow edges, {lit} lit", c.name);
    }
}
