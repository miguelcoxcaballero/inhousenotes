//! Physics benchmark on the real default city:
//! `cargo run --release -p forja_world --example physbench -- [all|city|micro] [--cars N] [--peds N] [--debris N] [--ai N] [--steps N]`.
//!
//! * `city`: builds the default procedural city (`SpanishCity::new`), feeds
//!   its colliders to a `PhysicsWorld` and reports collider counts, the
//!   collision mesh, BVH build time and memory; then simulates N driven cars,
//!   the bike, pedestrians (character controllers), debris crates and
//!   kinematic AI traffic around the player spawn and prints the step time
//!   per stage (mean, p95, max) plus the controllers' own cost.
//! * `micro`: stability checks on small synthetic worlds: stacking and
//!   jitter, tunnelling at 50 m/s, boxes sinking into kerbs and stairs, the
//!   character on 30° slopes and stairs, the bike parked and ridden and a car
//!   on a sloped alley.
//!
//! Numbers are recorded in `docs/physics_research.md`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use forja_math::{Iso, Quat, Ray, Vec3, vec3};
use forja_physics::*;
use forja_world::{SpanishCity, SpawnKind, WorldSource};

const DT: f32 = 1.0 / 60.0;

fn rss_mb() -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("VmRSS:")).and_then(|l| l.split_whitespace().nth(1)?.parse::<f64>().ok()))
        .map_or(0.0, |kb| kb / 1024.0)
}

fn arg(name: &str, default: usize) -> usize {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(default)
}

#[derive(Default)]
struct Series(Vec<f32>);

impl Series {
    fn push(&mut self, v: f32) {
        self.0.push(v);
    }
    fn stats(&self) -> (f32, f32, f32) {
        if self.0.is_empty() {
            return (0.0, 0.0, 0.0);
        }
        let mut s = self.0.clone();
        s.sort_by(f32::total_cmp);
        let mean = s.iter().sum::<f32>() / s.len() as f32;
        (mean, s[(s.len() * 95 / 100).min(s.len() - 1)], *s.last().unwrap())
    }
    fn row(&self, name: &str) {
        let (m, p, x) = self.stats();
        let mut v = self.0.clone();
        v.sort_by(f32::total_cmp);
        let med = v.get(v.len() / 2).copied().unwrap_or(0.0);
        println!("| {name:<26} | {m:>7.3} | {med:>7.3} | {p:>7.3} | {x:>7.3} |");
    }
}

fn main() {
    let mode = std::env::args().nth(1).filter(|a| !a.starts_with("--")).unwrap_or_else(|| "all".into());
    if mode == "all" || mode == "micro" {
        micro();
    }
    if mode == "all" || mode == "city" {
        city();
    }
}

// ---------------------------------------------------------------------------
// City scenario.

fn city() {
    let root = forja_assets::manifest::find_engine_root().unwrap_or_else(|| PathBuf::from("."));
    let dl = root.join("assets/downloaded");
    let rss0 = rss_mb();
    let t = Instant::now();
    let cache = std::env::args().skip_while(|a| a != "--cache").nth(1).map(PathBuf::from);
    let desc = match cache.as_ref().and_then(|c| load_cache(c)) {
        Some(d) => {
            println!("\n## City (collider cache)\n");
            d
        }
        None => {
            let mut desc = SpanishCity::new(&dl, root.join("data/city/styles.toml")).build().expect("city");
            println!("\n## City\n");
            println!("city built in {:.1} s, {} placements, RSS {:.0} MB (+{:.0})", t.elapsed().as_secs_f32(), desc.placements.len(), rss_mb(), rss_mb() - rss0);
            // The benchmark needs no render data.
            desc.scenes.clear();
            desc.placements.clear();
            desc.traffic = None;
            if let Some(c) = &cache {
                save_cache(c, &desc);
            }
            desc
        }
    };
    let mut kinds = std::collections::BTreeMap::<&str, usize>::new();
    let mut tris = 0usize;
    let mut verts = 0usize;
    let mut mesh: Option<Arc<TriMesh>> = None;
    for (c, _) in &desc.colliders {
        let k = match &c.shape {
            Shape::Sphere { .. } => "sphere",
            Shape::Cuboid { .. } => "cuboid",
            Shape::Capsule { .. } => "capsule",
            Shape::Convex(_) => "convex",
            Shape::TriMesh(m) => {
                tris += m.indices.len();
                verts += m.vertices.len();
                if mesh.as_ref().is_none_or(|b| b.indices.len() < m.indices.len()) {
                    mesh = Some(Arc::clone(m));
                }
                "trimesh"
            }
            Shape::HeightField(_) => "heightfield",
        };
        *kinds.entry(k).or_default() += 1;
    }
    println!("static colliders: {} {:?}", desc.colliders.len(), kinds);
    if let Some(m) = &mesh {
        let t = Instant::now();
        let rebuilt = TriMesh::new(m.vertices.clone(), m.indices.clone());
        let build = t.elapsed().as_secs_f32() * 1000.0;
        let mb = |b: usize| b as f64 / 1048576.0;
        println!(
            "collision mesh: {} tris, {} verts; BVH build {:.0} ms; memory: verts+indices {:.1} MB, BVH {:.1} MB",
            tris,
            verts,
            build,
            mb(m.vertices.len() * 12 + m.indices.len() * 12),
            mb(rebuilt.bvh.memory_bytes())
        );
        let b = m.bvh.bounds();
        println!("mesh bounds {:.0} x {:.0} x {:.0} m", b.max.x - b.min.x, b.max.y - b.min.y, b.max.z - b.min.z);
    }
    let rss1 = rss_mb();
    let t = Instant::now();
    let mut world = PhysicsWorld::default();
    world.config.active_radius = 160.0; // as the sandbox (NEW_API)
    desc.add_to_physics(&mut world);
    println!("add_to_physics: {:.0} ms, RSS {:.0} MB (+{:.0})", t.elapsed().as_secs_f32() * 1000.0, rss_mb(), rss_mb() - rss1);

    // Dynamic content around the player spawn.
    // `--game`: what the sandbox runs (one player, the 10 crates, AI cars
    // with colliders near the player); default: a stress load.
    let game = std::env::args().any(|a| a == "--game");
    let n_cars = arg("--cars", 8);
    let n_peds = arg("--peds", if game { 1 } else { 24 });
    let n_debris = arg("--debris", if game { 10 } else { 60 });
    let n_ai = arg("--ai", if game { 12 } else { 24 });
    let steps = arg("--steps", 900);
    let player = desc.spawn(SpawnKind::Player).unwrap_or(Iso::from_pos(Vec3::Y));
    let mut car_spawns: Vec<Iso> = desc.spawns_of(SpawnKind::Vehicle).map(|s| s.iso).collect();
    car_spawns.sort_by(|a, b| (a.pos - player.pos).length_squared().total_cmp(&(b.pos - player.pos).length_squared()));
    let mut ped_spawns: Vec<Iso> = desc.spawns_of(SpawnKind::Player).map(|s| s.iso).collect();
    ped_spawns.sort_by(|a, b| (a.pos - player.pos).length_squared().total_cmp(&(b.pos - player.pos).length_squared()));
    println!("spawns: {} vehicle, {} player", car_spawns.len(), ped_spawns.len());
    let mut vehicles: Vec<Vehicle> = Vec::new();
    for i in 0..n_cars {
        let iso = car_spawns.get(i).copied().unwrap_or(Iso::from_pos(player.pos + vec3(4.0 * i as f32, 0.0, 6.0)));
        let cfg = if i % 4 == 3 { VehicleConfig::default() } else { VehicleConfig::sedan() };
        vehicles.push(Vehicle::spawn(&mut world, cfg, Iso::new(iso.pos + Vec3::Y * 0.8, iso.rot)));
    }
    let bike_at = player.pos + player.rot * vec3(-3.0, 0.5, 2.0);
    vehicles.push(Vehicle::spawn(&mut world, VehicleConfig::motorcycle(), Iso::new(bike_at, player.rot)));
    let mut peds: Vec<(CharacterController, Vec3)> = Vec::new();
    for i in 0..n_peds {
        let base = ped_spawns.get(i % ped_spawns.len().max(1)).map_or(player.pos, |s| s.pos);
        let p = base + vec3((i / ped_spawns.len().max(1)) as f32 * 1.2, 0.5, 0.0);
        let a = i as f32 * 2.4;
        peds.push((CharacterController::new(&mut world, CharacterConfig::default(), p), vec3(a.cos(), 0.0, a.sin()) * 1.4));
    }
    let mut debris = Vec::new();
    for i in 0..n_debris {
        let ring = (i / 10) as f32;
        let a = i as f32 * 0.63;
        let p = player.pos + vec3(a.cos() * (6.0 + ring), 1.0 + (i % 3) as f32 * 0.9, a.sin() * (6.0 + ring));
        let (b, _) = world.add_dynamic(
            RigidBodyDesc::dynamic().position(p),
            ColliderDesc::new(Shape::cuboid(Vec3::splat(0.4))).mass(25.0).friction(0.7).groups(CollisionGroups::new(layers::DYNAMIC, u32::MAX)),
        );
        debris.push(b);
    }
    let mut ai = Vec::new();
    for i in 0..n_ai {
        let iso = car_spawns.get(n_cars + i).copied().unwrap_or(Iso::from_pos(player.pos + vec3(-10.0, 0.0, 3.0 * i as f32)));
        let b = world.add_body(RigidBodyDesc::kinematic().position(iso.pos + Vec3::Y * 0.75).rotation(iso.rot));
        world.add_collider(ColliderDesc::new(Shape::cuboid(vec3(0.9, 0.7, 2.2))).groups(CollisionGroups::new(layers::VEHICLE, u32::MAX)), Some(b));
        ai.push((b, iso));
    }
    println!("dynamic: {} cars + 1 bike, {} pedestrians, {} crates, {} kinematic AI cars; {} steps at 60 Hz", n_cars, n_peds, n_debris, n_ai, steps);

    let (mut s_total, mut s_step, mut s_broad, mut s_narrow, mut s_solve, mut s_wb, mut s_veh, mut s_chr, mut s_ai): (Series, Series, Series, Series, Series, Series, Series, Series, Series) =
        Default::default();
    let mut pairs = Series::default();
    let mut points = Series::default();
    let mut awake = Series::default();
    let (mut lo_y, mut fell) = (f32::MAX, 0usize);
    for step in 0..steps {
        let time = step as f32 * DT;
        let t0 = Instant::now();
        // AI traffic: kinematic boxes teleported along their lanes, as the
        // traffic plugin does every frame.
        for (i, (b, iso)) in ai.iter().enumerate() {
            let fwd = iso.rot * Vec3::Z;
            let v = 8.0 + (i % 3) as f32 * 2.0;
            let d = (time * v) % 40.0;
            let at = Iso::new(iso.pos + fwd * d + Vec3::Y * 0.75, iso.rot);
            world.set_body_iso(*b, at);
            if let Some(body) = world.body_mut(*b) {
                body.set_linvel(fwd * v);
            }
        }
        let t1 = Instant::now();
        for (i, v) in vehicles.iter_mut().enumerate() {
            let ph = (time * 0.4 + i as f32).sin();
            v.controls = VehicleControls { throttle: if time > 1.0 { 0.7 } else { 0.0 }, steer: ph * 0.6, brake: 0.0, handbrake: false };
            v.update(&mut world, DT);
        }
        let t2 = Instant::now();
        for (c, dir) in &mut peds {
            let turn = Quat::from_rotation_y(0.4 * DT);
            *dir = turn * *dir;
            c.update(&mut world, *dir, false, DT);
        }
        let t3 = Instant::now();
        world.set_focus(peds.first().map(|p| p.0.position)); // NEW_API
        world.step(DT);
        let t4 = Instant::now();
        if let Some(k) = std::env::args().skip_while(|a| a != "--trace").nth(1).and_then(|v| v.parse::<usize>().ok())
            && step % 15 == 0
            && let Some(v) = vehicles.get(k)
        {
            let b = world.body(v.body).unwrap();
            let tilt = (b.rotation() * Vec3::Y).y.clamp(-1.0, 1.0).acos().to_degrees();
            let g: Vec<String> = v.wheels.iter().map(|w| format!("{}{:.2}", if w.grounded { "g" } else { "-" }, w.compression)).collect();
            println!("trace t {:.2} pos {:.1} {:.1} {:.1} v {:.1} tilt {:.0} wheels {}", time, b.position().x, b.position().y, b.position().z, v.speed(&world), tilt, g.join(" "));
        }
        let ms = |a: Instant, b: Instant| (b - a).as_secs_f32() * 1000.0;
        if step >= 60 {
            s_ai.push(ms(t0, t1));
            s_veh.push(ms(t1, t2));
            s_chr.push(ms(t2, t3));
            s_step.push(ms(t3, t4));
            s_total.push(ms(t0, t4));
            let st = world.stats;
            s_broad.push(st.broadphase_ms);
            s_narrow.push(st.narrowphase_ms);
            s_solve.push(st.solver_ms);
            s_wb.push(st.writeback_ms);
            pairs.push(st.pairs as f32);
            points.push(st.contact_points as f32);
            awake.push(st.awake_bodies as f32);
        }
    }
    for b in &debris {
        let y = world.body(*b).unwrap().position().y;
        lo_y = lo_y.min(y);
        if y < player.pos.y - 20.0 {
            fell += 1;
        }
    }
    println!("\n| stage (ms)                 |    mean |  median |     p95 |     max |");
    println!("|---|---|---|---|---|");
    s_total.row("total physics frame");
    s_step.row("PhysicsWorld::step");
    s_broad.row("  broadphase + pairs");
    s_narrow.row("  narrowphase");
    s_solve.row("  solver");
    s_wb.row("  write-back + islands");
    s_veh.row("vehicles (rays + tyres)");
    s_chr.row("characters");
    s_ai.row("AI kinematic sync");
    println!("pairs mean {:.0}, contact points mean {:.0} (max {:.0}), awake bodies mean {:.0}", pairs.stats().0, points.stats().0, points.stats().2, awake.stats().0);
    println!("crates fallen through the world: {fell} (lowest y {:.1}, spawn y {:.1})", lo_y, player.pos.y);
    let rolled: Vec<String> = vehicles
        .iter()
        .map(|v| {
            let b = world.body(v.body).unwrap();
            format!("{:.0}", (b.rotation() * Vec3::Y).y.clamp(-1.0, 1.0).acos().to_degrees())
        })
        .collect();
    println!("vehicle tilt after run (deg): {}", rolled.join(" "));
    println!("RSS end {:.0} MB", rss_mb());
}

// ---------------------------------------------------------------------------
// Collider cache (the city takes ~95 s and ~3.5 GB to build).

fn put(b: &mut Vec<u8>, v: &[f32]) {
    for x in v {
        b.extend_from_slice(&x.to_le_bytes());
    }
}

fn save_cache(path: &std::path::Path, d: &forja_world::WorldDescription) {
    let mut b = Vec::new();
    let mut n = 0u32;
    let mut body = Vec::new();
    for (c, iso) in &d.colliders {
        let iso = iso.mul(&c.local);
        let (kind, data): (u8, Vec<f32>) = match &c.shape {
            Shape::Cuboid { half } => (0, half.to_array().to_vec()),
            Shape::Capsule { half_height, radius } => (1, vec![*half_height, *radius]),
            Shape::Sphere { radius } => (2, vec![*radius]),
            Shape::TriMesh(m) => {
                let mut v = vec![m.vertices.len() as f32, m.indices.len() as f32];
                v.extend(m.vertices.iter().flat_map(|p| p.to_array()));
                v.extend(m.indices.iter().flat_map(|t| t.map(|i| f32::from_bits(i))));
                (3, v)
            }
            _ => continue,
        };
        n += 1;
        body.push(kind);
        put(&mut body, &[c.friction, c.groups.memberships as f32]);
        put(&mut body, &iso.pos.to_array());
        put(&mut body, &iso.rot.to_array());
        body.extend_from_slice(&(data.len() as u32).to_le_bytes());
        put(&mut body, &data);
    }
    b.extend_from_slice(&n.to_le_bytes());
    b.extend(body);
    b.extend_from_slice(&(d.spawns.len() as u32).to_le_bytes());
    for s in &d.spawns {
        b.push(match s.kind {
            SpawnKind::Player => 0,
            SpawnKind::Vehicle => 1,
            SpawnKind::Bike => 2,
            SpawnKind::Weapon => 3,
            SpawnKind::Prop => 4,
        });
        put(&mut b, &s.iso.pos.to_array());
        put(&mut b, &s.iso.rot.to_array());
    }
    let _ = std::fs::write(path, b);
}

fn load_cache(path: &std::path::Path) -> Option<forja_world::WorldDescription> {
    let b = std::fs::read(path).ok()?;
    let mut i = 0usize;
    let u32_ = |i: &mut usize| {
        let v = u32::from_le_bytes(b[*i..*i + 4].try_into().unwrap());
        *i += 4;
        v
    };
    let f = |i: &mut usize| f32::from_bits(u32_(i));
    let mut d = forja_world::WorldDescription::new("cache");
    let n = u32_(&mut i);
    for _ in 0..n {
        let kind = b[i];
        i += 1;
        let friction = f(&mut i);
        let _mem = f(&mut i);
        let pos = vec3(f(&mut i), f(&mut i), f(&mut i));
        let rot = Quat::from_xyzw(f(&mut i), f(&mut i), f(&mut i), f(&mut i));
        let len = u32_(&mut i) as usize;
        let data: Vec<f32> = (0..len).map(|_| f(&mut i)).collect();
        let shape = match kind {
            0 => Shape::cuboid(vec3(data[0], data[1], data[2])),
            1 => Shape::capsule(data[0], data[1]),
            2 => Shape::sphere(data[0]),
            _ => {
                let (nv, nt) = (data[0] as usize, data[1] as usize);
                let v = (0..nv).map(|k| vec3(data[2 + k * 3], data[3 + k * 3], data[4 + k * 3])).collect();
                let o = 2 + nv * 3;
                let t = (0..nt).map(|k| [data[o + k * 3].to_bits(), data[o + k * 3 + 1].to_bits(), data[o + k * 3 + 2].to_bits()]).collect();
                Shape::TriMesh(Arc::new(TriMesh::new(v, t)))
            }
        };
        d.colliders.push((ColliderDesc::new(shape).friction(friction).groups(CollisionGroups::new(layers::WORLD, u32::MAX)), Iso::new(pos, rot)));
    }
    let ns = u32_(&mut i);
    for _ in 0..ns {
        let k = b[i];
        i += 1;
        let pos = vec3(f(&mut i), f(&mut i), f(&mut i));
        let rot = Quat::from_xyzw(f(&mut i), f(&mut i), f(&mut i), f(&mut i));
        let kind = [SpawnKind::Player, SpawnKind::Vehicle, SpawnKind::Bike, SpawnKind::Weapon, SpawnKind::Prop][k as usize];
        d.spawns.push(forja_world::SpawnPoint { kind, iso: Iso::new(pos, rot) });
    }
    Some(d)
}

// ---------------------------------------------------------------------------
// Micro stability scenes.

fn ground() -> PhysicsWorld {
    let mut w = PhysicsWorld::default();
    w.add_collider(ColliderDesc::new(Shape::cuboid(vec3(200.0, 0.5, 200.0))).translation(vec3(0.0, -0.5, 0.0)).friction(0.8), None);
    w
}

fn run(w: &mut PhysicsWorld, s: f32) {
    for _ in 0..(s / DT) as usize {
        w.step(DT);
    }
}

fn quad(v: &mut Vec<Vec3>, t: &mut Vec<[u32; 3]>, a: Vec3, b: Vec3, c: Vec3, d: Vec3) {
    // a b c d counter-clockwise seen from the front.
    let i = v.len() as u32;
    v.extend([a, b, c, d]);
    t.extend([[i, i + 1, i + 2], [i, i + 2, i + 3]]);
}

/// A street like the city's: road at 0, a 0.15 m kerb, a pavement, then a
/// flight of 10 stairs (0.17 rise, 0.30 run), all as one triangle mesh.
fn street_mesh() -> TriMesh {
    let (mut v, mut t) = (Vec::new(), Vec::new());
    let (z0, z1) = (-20.0, 20.0);
    let top = |v: &mut Vec<Vec3>, t: &mut Vec<[u32; 3]>, x0: f32, x1: f32, y: f32| quad(v, t, vec3(x0, y, z0), vec3(x0, y, z1), vec3(x1, y, z1), vec3(x1, y, z0));
    let riser = |v: &mut Vec<Vec3>, t: &mut Vec<[u32; 3]>, x: f32, y0: f32, y1: f32| quad(v, t, vec3(x, y0, z0), vec3(x, y1, z0), vec3(x, y1, z1), vec3(x, y0, z1));
    top(&mut v, &mut t, -20.0, 0.0, 0.0);
    riser(&mut v, &mut t, 0.0, 0.0, 0.15);
    top(&mut v, &mut t, 0.0, 3.0, 0.15);
    let mut y = 0.15;
    let mut x = 3.0;
    for _ in 0..10 {
        riser(&mut v, &mut t, x, y, y + 0.17);
        y += 0.17;
        top(&mut v, &mut t, x, x + 0.3, y);
        x += 0.3;
    }
    top(&mut v, &mut t, x, x + 10.0, y);
    TriMesh::new(v, t)
}

/// A 30° ramp (trimesh) rising along +X from x = 0, 20 m long.
fn slope_mesh(deg: f32) -> TriMesh {
    let (mut v, mut t) = (Vec::new(), Vec::new());
    let h = 20.0 * deg.to_radians().tan();
    quad(&mut v, &mut t, vec3(-20.0, 0.0, -10.0), vec3(-20.0, 0.0, 10.0), vec3(0.0, 0.0, 10.0), vec3(0.0, 0.0, -10.0));
    quad(&mut v, &mut t, vec3(0.0, 0.0, -10.0), vec3(0.0, 0.0, 10.0), vec3(20.0, h, 10.0), vec3(20.0, h, -10.0));
    quad(&mut v, &mut t, vec3(20.0, h, -10.0), vec3(20.0, h, 10.0), vec3(40.0, h, 10.0), vec3(40.0, h, -10.0));
    TriMesh::new(v, t)
}

fn set_rider(v: &mut Vehicle, rider: bool) {
    v.rider = rider;
}

fn surface_y(w: &PhysicsWorld, x: f32, z: f32, skip: BodyHandle) -> f32 {
    w.raycast(&Ray::new(vec3(x, 20.0, z), Vec3::NEG_Y), 40.0, QueryFilter::exclude(skip)).map_or(0.0, |h| h.point.y)
}

fn micro() {
    println!("\n## Stability\n");
    // 1. Stack of 10 boxes: drift and jitter.
    {
        let mut w = ground();
        let bs: Vec<BodyHandle> = (0..10)
            .map(|i| w.add_dynamic(RigidBodyDesc::dynamic().position(vec3(0.0, 0.5 + i as f32, 0.0)).can_sleep(false), ColliderDesc::new(Shape::cuboid(Vec3::splat(0.5)))).0)
            .collect();
        run(&mut w, 4.0);
        let mut jitter = 0.0f32;
        for _ in 0..60 {
            w.step(DT);
            for b in &bs {
                jitter = jitter.max(w.body(*b).unwrap().linvel.length());
            }
        }
        let top = w.body(bs[9]).unwrap().position();
        println!("stack 10 (no sleep): top drift {:.3} m horizontal, {:.3} m vertical; max speed at rest {:.4} m/s", vec3(top.x, 0.0, top.z).length(), top.y - 9.5, jitter);
    }
    // 1b. Pyramid of 55 boxes (10 base rows) and timing.
    {
        let mut w = ground();
        let mut bs = Vec::new();
        for row in 0..10 {
            for i in 0..(10 - row) {
                let x = (i as f32 - (9 - row) as f32 * 0.5) * 1.02;
                bs.push(w.add_dynamic(RigidBodyDesc::dynamic().position(vec3(x, 0.5 + row as f32, 0.0)).can_sleep(false), ColliderDesc::new(Shape::cuboid(Vec3::splat(0.5)))).0);
            }
        }
        let mut ms = Series::default();
        for _ in 0..300 {
            w.step(DT);
            ms.push(w.stats.step_ms);
        }
        let top = w.body(*bs.last().unwrap()).unwrap().position();
        println!("pyramid 55: top at y {:.3} (start 9.5), x {:.3}; step mean {:.3} ms", top.y, top.x, ms.stats().0);
    }
    // 2. Tunnelling at 50 m/s.
    {
        let mut results = Vec::new();
        for (name, shape) in [("sphere r0.1", Shape::sphere(0.1)), ("box 0.2", Shape::cuboid(Vec3::splat(0.1))), ("capsule", Shape::capsule(0.2, 0.1))] {
            for wall in ["box 5cm", "trimesh"] {
                let mut w = PhysicsWorld::default();
                w.config.gravity = Vec3::ZERO;
                match wall {
                    "box 5cm" => {
                        w.add_collider(ColliderDesc::new(Shape::cuboid(vec3(0.025, 5.0, 5.0))).translation(vec3(10.0, 0.0, 0.0)), None);
                    }
                    _ => {
                        let (mut v, mut t) = (Vec::new(), Vec::new());
                        quad(&mut v, &mut t, vec3(10.0, -5.0, -5.0), vec3(10.0, 5.0, -5.0), vec3(10.0, 5.0, 5.0), vec3(10.0, -5.0, 5.0));
                        w.add_collider(ColliderDesc::new(Shape::TriMesh(Arc::new(TriMesh::new(v, t)))), None);
                    }
                }
                let (b, _) = w.add_dynamic(
                    RigidBodyDesc::dynamic().position(vec3(0.0, 0.0, 0.0)).linvel(vec3(50.0, 0.0, 0.0)).angvel(vec3(0.0, 0.0, 20.0)),
                    ColliderDesc::new(shape.clone()),
                );
                run(&mut w, 1.0);
                let x = w.body(b).unwrap().position().x;
                results.push(format!("{name} vs {wall}: {}", if x < 10.0 { "stopped" } else { "TUNNELLED" }));
            }
        }
        println!("tunnelling at 50 m/s (+20 rad/s spin): {}", results.join("; "));
        // A car at 50 m/s into a building wall (trimesh).
        let mut w = PhysicsWorld::default();
        w.add_collider(ColliderDesc::new(Shape::cuboid(vec3(200.0, 0.5, 200.0))).translation(vec3(0.0, -0.5, 0.0)), None);
        let (mut v, mut t) = (Vec::new(), Vec::new());
        quad(&mut v, &mut t, vec3(-10.0, 0.0, 60.0), vec3(-10.0, 10.0, 60.0), vec3(10.0, 10.0, 60.0), vec3(10.0, 0.0, 60.0));
        w.add_collider(ColliderDesc::new(Shape::TriMesh(Arc::new(TriMesh::new(v, t)))), None);
        let car = Vehicle::spawn(&mut w, VehicleConfig::sedan(), Iso::from_pos(vec3(0.0, 0.8, 0.0)));
        w.body_mut(car.body).unwrap().set_linvel(vec3(0.0, 0.0, 50.0));
        let mut car = car;
        for _ in 0..120 {
            car.update(&mut w, DT);
            w.step(DT);
        }
        let z = w.body(car.body).unwrap().position().z;
        println!("sedan at 50 m/s into a trimesh wall: {}", if z < 60.0 { format!("stopped (z {z:.2})") } else { "TUNNELLED".into() });
    }
    // 3. Boxes and spheres dropped on kerbs and stairs.
    {
        let mut w = PhysicsWorld::default();
        w.add_collider(ColliderDesc::new(Shape::TriMesh(Arc::new(street_mesh()))).friction(0.8), None);
        let mut bs = Vec::new();
        for i in 0..60 {
            let x = -2.0 + (i % 20) as f32 * 0.4;
            let z = -6.0 + (i / 20) as f32 * 4.0;
            let shape = if i % 2 == 0 { Shape::cuboid(Vec3::splat(0.15)) } else { Shape::sphere(0.15) };
            bs.push((w.add_dynamic(RigidBodyDesc::dynamic().position(vec3(x, 3.5, z)).rotation(Quat::from_rotation_y(i as f32)), ColliderDesc::new(shape).friction(0.8)).0, i % 2 == 0));
        }
        run(&mut w, 4.0);
        let (mut worst, mut sum, mut fell) = (0.0f32, 0.0f32, 0);
        for (b, is_box) in &bs {
            let body = w.body(*b).unwrap();
            let p = body.position();
            let bottom = if *is_box {
                (0..8)
                    .map(|k| {
                        let c = vec3(if k & 1 == 0 { -0.15 } else { 0.15 }, if k & 2 == 0 { -0.15 } else { 0.15 }, if k & 4 == 0 { -0.15 } else { 0.15 });
                        body.iso().transform_point(c).y
                    })
                    .fold(f32::MAX, f32::min)
            } else {
                p.y - 0.15
            };
            let s = surface_y(&w, p.x, p.z, *b);
            if p.y < -1.0 {
                fell += 1;
                continue;
            }
            let pen = (s - bottom).max(0.0);
            worst = worst.max(pen);
            sum += pen;
        }
        println!("60 small boxes/spheres on kerb+stairs after 4 s: max sink {:.1} mm, mean {:.1} mm, fell through {}", worst * 1000.0, sum / bs.len() as f32 * 1000.0, fell);
    }
    // 4. Character on 30° slope and on stairs.
    for (name, mesh) in [("30° slope", slope_mesh(30.0)), ("40° slope", slope_mesh(40.0)), ("stairs 17/30 cm", street_mesh())] {
        let mut w = PhysicsWorld::default();
        w.add_collider(ColliderDesc::new(Shape::TriMesh(Arc::new(mesh))), None);
        let start = vec3(-3.0, 1.2, 0.0);
        let mut c = CharacterController::new(&mut w, CharacterConfig::default(), start);
        for _ in 0..30 {
            c.update(&mut w, Vec3::ZERO, false, DT);
        }
        let (mut grounded, mut n, mut max_dy, mut prev_vy) = (0, 0, 0.0f32, 0.0f32);
        let mut last_y = c.feet().y;
        let mut max_step = 0.0f32;
        let mut max_vis = 0.0f32;
        let mut last_vis = c.smoothed_feet().y;
        let x0 = c.position.x;
        let t = Instant::now();
        for _ in 0..240 {
            c.update(&mut w, vec3(4.0, 0.0, 0.0), false, DT);
            let y = c.feet().y;
            let vy = (y - last_y) / DT;
            max_dy = max_dy.max((vy - prev_vy).abs());
            max_step = max_step.max((y - last_y).abs());
            let vis = c.smoothed_feet().y;
            max_vis = max_vis.max((vis - last_vis).abs());
            last_vis = vis;
            prev_vy = vy;
            last_y = y;
            grounded += c.state.grounded as usize;
            n += 1;
        }
        let up_ms = t.elapsed().as_secs_f32() * 1000.0 / n as f32;
        let up_dist = c.position.x - x0;
        let top_y = c.feet().y;
        // And back down.
        let (mut gdown, mut air_frames) = (0, 0);
        for _ in 0..240 {
            c.update(&mut w, vec3(-4.0, 0.0, 0.0), false, DT);
            gdown += c.state.grounded as usize;
            air_frames += (!c.state.grounded) as usize;
        }
        println!(
            "character {name}: up {:.1} m in 4 s at 4 m/s (feet y {:.2}), grounded {:.0}% up / {:.0}% down ({} airborne frames), max per-frame y jump {:.3} m (smoothed {:.3}), vy jerk {:.1} m/s; {:.3} ms/update",
            up_dist,
            top_y,
            grounded as f32 * 100.0 / n as f32,
            gdown as f32 * 100.0 / 240.0,
            air_frames,
            max_step,
            max_vis,
            max_dy,
            up_ms
        );
    }
    // 5. Bike parked and ridden.
    {
        let mut w = ground();
        let mut bike = Vehicle::spawn(&mut w, VehicleConfig::motorcycle(), Iso::from_pos(vec3(0.0, 0.8, 0.0)));
        // Parked: nobody on it (the old model had no notion of a rider and
        // ignores this).
        set_rider(&mut bike, false);
        let roll = |w: &PhysicsWorld, v: &Vehicle| {
            let b = w.body(v.body).unwrap();
            let fwd = b.rotation() * Vec3::Z;
            let up = b.rotation() * Vec3::Y;
            let flat_up = (up - fwd * up.dot(fwd)).normalize_or(Vec3::Y);
            flat_up.y.clamp(-1.0, 1.0).acos().to_degrees()
        };
        let mut max_roll = 0.0f32;
        for _ in 0..600 {
            bike.update(&mut w, DT);
            w.step(DT);
            max_roll = max_roll.max(roll(&w, &bike));
        }
        println!("bike parked 10 s: roll {:.1}° (max {:.1}°), sleeping {}", roll(&w, &bike), max_roll, w.body(bike.body).unwrap().is_sleeping());
        // Pushes from the side while parked: a nudge and a shove.
        for j in [25.0, 120.0] {
            let at = w.body(bike.body).unwrap().position() + Vec3::Y * 0.4;
            w.body_mut(bike.body).unwrap().apply_impulse_at_point(vec3(-j, 0.0, 0.0), at);
            for _ in 0..300 {
                bike.update(&mut w, DT);
                w.step(DT);
            }
            println!("bike parked, pushed sideways ({j} N s): roll after 5 s {:.1}°", roll(&w, &bike));
        }
        let mut w = ground();
        let mut bike = Vehicle::spawn(&mut w, VehicleConfig::motorcycle(), Iso::from_pos(vec3(0.0, 0.8, 0.0)));
        set_rider(&mut bike, true);
        let mut max_roll = 0.0f32;
        let mut speed_max = 0.0f32;
        let mut fell = false;
        for i in 0..1200 {
            let t = i as f32 * DT;
            bike.controls = VehicleControls { throttle: 0.6, steer: if t > 4.0 { (t * 0.7).sin() } else { 0.0 }, brake: 0.0, handbrake: false };
            bike.update(&mut w, DT);
            w.step(DT);
            let r = roll(&w, &bike);
            max_roll = max_roll.max(r);
            fell |= r > 70.0;
            speed_max = speed_max.max(bike.speed(&w));
        }
        // Stop, get off and let go.
        let mut settle_roll = 0.0;
        for i in 0..600 {
            if i == 240 {
                set_rider(&mut bike, false);
            }
            bike.controls = VehicleControls { throttle: 0.0, steer: 0.0, brake: if i < 240 { 1.0 } else { 0.0 }, handbrake: false };
            bike.update(&mut w, DT);
            w.step(DT);
            settle_roll = roll(&w, &bike);
        }
        println!("bike ridden 20 s (slalom): max roll {:.1}°, top speed {:.1} m/s, fell {}; stopped and released 10 s: roll {:.1}°", max_roll, speed_max, fell, settle_roll);
    }
    // 6. Car on a sloped alley (12°, trimesh).
    {
        let mut w = PhysicsWorld::default();
        w.add_collider(ColliderDesc::new(Shape::TriMesh(Arc::new(slope_mesh(12.0)))).friction(0.9), None);
        let rot = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let mut car = Vehicle::spawn(&mut w, VehicleConfig::sedan(), Iso::new(vec3(8.0, 0.8 + 8.0 * 12f32.to_radians().tan(), 0.0), rot));
        let mut ms = Series::default();
        for _ in 0..240 {
            car.controls = VehicleControls { handbrake: true, ..Default::default() };
            let t = Instant::now();
            car.update(&mut w, DT);
            ms.push(t.elapsed().as_secs_f32() * 1000.0);
            w.step(DT);
        }
        let p0 = w.body(car.body).unwrap().position();
        for _ in 0..240 {
            car.controls = VehicleControls { handbrake: true, ..Default::default() };
            car.update(&mut w, DT);
            w.step(DT);
        }
        let p1 = w.body(car.body).unwrap().position();
        let mut vmax = 0.0f32;
        for _ in 0..180 {
            car.controls = VehicleControls { throttle: 1.0, ..Default::default() };
            car.update(&mut w, DT);
            w.step(DT);
            vmax = vmax.max(car.speed(&w));
        }
        println!(
            "sedan on 12° alley: handbrake creep {:.2} m in 4 s; full throttle uphill 3 s: {:.1} m/s; vehicle update {:.3} ms",
            (p1 - p0).length(),
            vmax,
            ms.stats().0
        );
    }
}
