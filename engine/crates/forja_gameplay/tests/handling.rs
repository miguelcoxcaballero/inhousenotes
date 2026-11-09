//! Handling harness and regression tests: every handling preset is driven
//! by a keyboard-like driver (binary throttle and brake, steering keys that
//! are either fully pressed or released) through the manoeuvres that make a
//! car feel controllable or not, on a flat track and over kerbs.
//!
//! `cargo test -p forja_gameplay --test handling -- --ignored --nocapture`
//! prints the full table (see docs/physics_research.md, "Handling").

use std::path::PathBuf;

use forja_gameplay::*;
use forja_math::{Iso, Quat, Vec3, vec3};
use forja_physics::{ColliderDesc, PhysicsWorld, Shape, TriMesh, Vehicle, VehicleConfig, VehicleControls, WheelConfig};

const DT: f32 = 1.0 / 60.0;
const KMH: f32 = 1.0 / 3.6;

fn data() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")
}

fn registry() -> HandlingRegistry {
    HandlingRegistry::load(&data().join("handling")).unwrap()
}

/// The game's default car: the "sports" handling bound to the Khronos
/// CarConcept model. The geometry below is what `bind_vehicle_model` gives
/// for that model (checked by `concept_geometry_matches_the_model` when the
/// asset is present), so the tests do not need the download.
fn concept(reg: &HandlingRegistry) -> VehicleConfig {
    let mut c = reg.get("sports").unwrap().clone();
    let t = c.wheels[0].clone();
    let sag = (c.mass * 9.81 / 4.0 / c.suspension_stiffness).min(0.9 * t.suspension_length);
    let y = CONCEPT.wheel_y + t.suspension_length - sag;
    let wheel = |x: f32, z: f32, front: bool, r: f32| WheelConfig { position: vec3(x, y, z), radius: r, steer: front, drive: !front, handbrake: !front, ..t.clone() };
    c.chassis_half = CONCEPT.half;
    c.wheels = vec![
        wheel(CONCEPT.front_x, CONCEPT.front_z, true, CONCEPT.front_r),
        wheel(-CONCEPT.front_x, CONCEPT.front_z, true, CONCEPT.front_r),
        wheel(CONCEPT.rear_x, CONCEPT.rear_z, false, CONCEPT.rear_r),
        wheel(-CONCEPT.rear_x, CONCEPT.rear_z, false, CONCEPT.rear_r),
    ];
    c.com = vec3(0.0, -c.chassis_half.y * 0.6, 0.0);
    c
}

struct ConceptGeometry {
    half: Vec3,
    wheel_y: f32,
    front_x: f32,
    front_z: f32,
    front_r: f32,
    rear_x: f32,
    rear_z: f32,
    rear_r: f32,
}

const CONCEPT: ConceptGeometry = ConceptGeometry { half: Vec3::new(1.169, 0.318, 2.179), wheel_y: -0.1905, front_x: 0.978, front_z: 1.246, front_r: 0.3835, rear_x: 0.9845, rear_z: -1.5525, rear_r: 0.3833 };

fn concept_asset() -> Option<PathBuf> {
    let rel = "khronos_car_concept/CarConcept.glb";
    let local = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/downloaded").join(rel);
    let env = std::env::var_os("FORJA_ASSETS").map(|p| PathBuf::from(p).join(rel));
    [Some(local), env].into_iter().flatten().find(|p| p.exists())
}

#[test]
fn concept_geometry_matches_the_model() {
    let Some(path) = concept_asset() else { return };
    let scene = forja_assets::load_gltf(&path).unwrap();
    let reg = registry();
    let bound = bind_vehicle_model_oriented(reg.get("sports").unwrap().clone(), &scene, 1.0, Quat::IDENTITY).config;
    let ours = concept(&reg);
    println!("bound: half {:?} com {:?}", bound.chassis_half, bound.com);
    for w in &bound.wheels {
        println!("  wheel {:?} r {} steer {} drive {}", w.position, w.radius, w.steer, w.drive);
    }
    assert!((bound.chassis_half - ours.chassis_half).abs().max_element() < 0.03, "{:?}", bound.chassis_half);
    assert_eq!(bound.wheels.len(), 4);
    for b in &bound.wheels {
        let o = ours.wheels.iter().min_by(|x, y| (x.position - b.position).length().total_cmp(&(y.position - b.position).length())).unwrap();
        assert!((o.position - b.position).length() < 0.03, "wheel {:?} vs {:?}", b.position, o.position);
        assert!((o.radius - b.radius).abs() < 0.01);
        assert_eq!((o.steer, o.drive), (b.steer, b.drive));
    }
}

// ---------------------------------------------------------------------------
// Tracks.

fn flat() -> PhysicsWorld {
    let mut w = PhysicsWorld::default();
    w.add_collider(ColliderDesc::new(Shape::cuboid(vec3(2000.0, 0.5, 2000.0))).translation(vec3(0.0, -0.5, 0.0)).friction(1.0), None);
    w
}

fn quad(v: &mut Vec<Vec3>, t: &mut Vec<[u32; 3]>, a: Vec3, b: Vec3, c: Vec3, d: Vec3) {
    let i = v.len() as u32;
    v.extend([a, b, c, d]);
    t.extend([[i, i + 1, i + 2], [i, i + 2, i + 3]]);
}

/// A road along +Z that steps up by `rise` at `z = at`, as a triangle mesh
/// in 4 m strips like the game's road colliders. `run` is the length of the
/// riser: 0 for a square step, `rise` for a 45 degree chamfer.
fn step_track(at: f32, rise: f32, run: f32) -> PhysicsWorld {
    let (mut v, mut t) = (Vec::new(), Vec::new());
    let (x0, x1) = (-150.0, 150.0);
    let mut strip = |z0: f32, z1: f32, y0: f32, y1: f32| quad(&mut v, &mut t, vec3(x0, y0, z0), vec3(x0, y1, z1), vec3(x1, y1, z1), vec3(x1, y0, z0));
    let mut z = -40.0;
    while z < at - 1e-3 {
        strip(z, (z + 4.0).min(at), 0.0, 0.0);
        z += 4.0;
    }
    strip(at, at + run, 0.0, rise);
    let mut z = at + run;
    while z < at + 200.0 {
        strip(z, z + 4.0, rise, rise);
        z += 4.0;
    }
    let mut w = PhysicsWorld::default();
    w.add_collider(ColliderDesc::new(Shape::TriMesh(std::sync::Arc::new(TriMesh::new(v, t)))).friction(1.0), None);
    w
}

// ---------------------------------------------------------------------------
// Rig and keyboard driver.

#[derive(Clone, Copy, Default)]
struct Keys {
    /// -1 left, +1 right: a key is down or it is not.
    steer: i8,
    gas: bool,
    brake: bool,
    handbrake: bool,
}

impl Keys {
    fn controls(self) -> VehicleControls {
        // The game maps W/S to throttle +1/-1 (S brakes while moving).
        VehicleControls { throttle: if self.brake { -1.0 } else if self.gas { 1.0 } else { 0.0 }, brake: 0.0, steer: self.steer as f32, handbrake: self.handbrake }
    }
}

#[derive(Clone, Copy, Debug)]
struct Trace {
    /// Largest body slip angle (deg) while moving.
    beta: f32,
    /// Largest yaw rate (deg/s).
    yaw: f32,
    /// Largest lateral acceleration (g).
    lat_g: f32,
    /// Lowest `up.y` (1 = level).
    up: f32,
    min_speed: f32,
}

impl Default for Trace {
    fn default() -> Self {
        Self { beta: 0.0, yaw: 0.0, lat_g: 0.0, up: 1.0, min_speed: f32::MAX }
    }
}

impl Trace {
    fn spun(&self) -> bool {
        self.beta > 45.0
    }
    fn rolled(&self) -> bool {
        self.up < 0.5
    }
}

struct Rig {
    w: PhysicsWorld,
    car: Vehicle,
    tr: Trace,
    prev_vel: Vec3,
}

impl Rig {
    fn new(cfg: &VehicleConfig, w: PhysicsWorld) -> Self {
        Self::at(cfg, w, Iso::from_pos(Vec3::ZERO))
    }

    /// Spawns resting on y = 0 at `iso` and lets the suspension settle.
    fn at(cfg: &VehicleConfig, mut w: PhysicsWorld, iso: Iso) -> Self {
        let wc = &cfg.wheels[0];
        let y = -(wc.position.y - wc.suspension_length - wc.radius) + 0.02;
        let car = Vehicle::spawn(&mut w, cfg.clone(), Iso::new(iso.pos + Vec3::Y * y, iso.rot));
        let mut r = Self { w, car, tr: Trace::default(), prev_vel: Vec3::ZERO };
        r.run(1.0, |_| VehicleControls::default());
        r.tr = Trace::default();
        r
    }

    fn body(&self) -> &forja_physics::RigidBody {
        self.w.body(self.car.body).unwrap()
    }
    fn pos(&self) -> Vec3 {
        self.body().position()
    }
    fn fwd(&self) -> Vec3 {
        self.body().rotation() * Vec3::Z
    }
    fn speed(&self) -> f32 {
        self.body().linvel.length()
    }
    /// Heading (rad) about +Y, 0 = +Z, positive towards +X (left).
    fn heading(&self) -> f32 {
        let f = self.fwd();
        f.x.atan2(f.z)
    }
    /// Body slip angle (deg): velocity direction against the heading.
    fn beta(&self) -> f32 {
        let b = self.body();
        let v = b.linvel;
        if vec3(v.x, 0.0, v.z).length() < 3.0 {
            return 0.0;
        }
        let (f, l) = (b.rotation() * Vec3::Z, b.rotation() * Vec3::X);
        v.dot(l).atan2(v.dot(f)).to_degrees()
    }
    fn yaw_rate(&self) -> f32 {
        self.body().angvel.y.to_degrees()
    }

    /// Teleports the car to `v` m/s along its heading with rolling wheels,
    /// then holds that speed with the throttle key until the gearbox and
    /// suspension have caught up.
    fn cruise(&mut self, v: f32) {
        let fwd = self.fwd();
        self.w.body_mut(self.car.body).unwrap().set_linvel(fwd * v);
        for (ws, wc) in self.car.wheels.iter_mut().zip(&self.car.config.wheels) {
            ws.spin_rate = v / wc.radius;
        }
        self.run(2.5, |r| Keys { gas: r.speed() < v, ..Default::default() }.controls());
        self.tr = Trace::default();
    }

    fn step(&mut self, c: VehicleControls) {
        self.car.controls = c;
        self.car.update(&mut self.w, DT);
        self.w.step(DT);
        let b = self.body();
        let vel = b.linvel;
        let left = b.rotation() * Vec3::X;
        let up = (b.rotation() * Vec3::Y).y;
        let lat = ((vel - self.prev_vel) / DT).dot(left).abs() / 9.81;
        self.prev_vel = vel;
        let (beta, yaw, speed) = (self.beta().abs(), self.yaw_rate().abs(), self.speed());
        let t = &mut self.tr;
        t.beta = t.beta.max(beta);
        t.yaw = t.yaw.max(yaw);
        t.lat_g = t.lat_g.max(lat);
        t.up = t.up.min(up);
        t.min_speed = t.min_speed.min(speed);
    }

    fn run(&mut self, seconds: f32, mut f: impl FnMut(&Rig) -> VehicleControls) {
        for _ in 0..(seconds / DT).round() as usize {
            let c = f(self);
            self.step(c);
        }
    }

    fn keys(&mut self, seconds: f32, k: Keys) {
        self.run(seconds, |_| k.controls());
    }

    /// Hands off: time until the car runs straight again (slip under 3
    /// degrees and yaw under 5 deg/s for half a second), or `None` in 6 s.
    fn recover(&mut self) -> Option<f32> {
        let mut calm = 0.0;
        for i in 0..(6.0 / DT) as usize {
            self.step(VehicleControls::default());
            if self.speed() < 3.0 || (self.beta().abs() < 3.0 && self.yaw_rate().abs() < 5.0) {
                calm += DT;
                if calm >= 0.5 {
                    return Some(i as f32 * DT - 0.5 + DT);
                }
            } else {
                calm = 0.0;
            }
        }
        None
    }
}

fn verdict(tr: &Trace, rec: Option<f32>) -> String {
    let state = if tr.rolled() {
        "ROLLED"
    } else if tr.spun() {
        "SPUN"
    } else if tr.beta > 20.0 {
        "slide"
    } else {
        "ok"
    };
    let rec = rec.map_or("never".to_string(), |t| format!("{t:.1}s"));
    format!("{state:6} beta {:5.1} yaw {:5.1} lat {:4.2}g rec {rec}", tr.beta, tr.yaw, tr.lat_g)
}

// ---------------------------------------------------------------------------
// Manoeuvres.

/// Steering key held at constant speed, then released.
fn hold_steer(cfg: &VehicleConfig, v: f32) -> (Trace, Option<f32>) {
    let mut r = Rig::new(cfg, flat());
    r.cruise(v);
    r.run(3.0, |r| Keys { steer: 1, gas: r.speed() < v, ..Default::default() }.controls());
    let rec = r.recover();
    (r.tr, rec)
}

/// Corner on the throttle, then lift with the steering key still down.
fn lift_off(cfg: &VehicleConfig, v: f32) -> (Trace, Option<f32>) {
    let mut r = Rig::new(cfg, flat());
    r.cruise(v);
    r.keys(1.5, Keys { steer: 1, gas: true, ..Default::default() });
    r.tr = Trace::default();
    r.keys(2.0, Keys { steer: 1, ..Default::default() });
    let rec = r.recover();
    (r.tr, rec)
}

/// Brake hard with the steering key down (trail braking, panic braking).
fn brake_turn(cfg: &VehicleConfig, v: f32) -> (Trace, Option<f32>) {
    let mut r = Rig::new(cfg, flat());
    r.cruise(v);
    r.keys(0.8, Keys { steer: 1, gas: true, ..Default::default() });
    r.keys(1.5, Keys { steer: 1, brake: true, ..Default::default() });
    let rec = r.recover();
    (r.tr, rec)
}

/// Left key, right key, release.
fn flick(cfg: &VehicleConfig, v: f32) -> (Trace, Option<f32>) {
    let mut r = Rig::new(cfg, flat());
    r.cruise(v);
    r.run(0.5, |r| Keys { steer: -1, gas: r.speed() < v, ..Default::default() }.controls());
    r.run(0.5, |r| Keys { steer: 1, gas: r.speed() < v, ..Default::default() }.controls());
    let rec = r.recover();
    (r.tr, rec)
}

/// Turn in, tap the handbrake, release everything.
fn handbrake_tap(cfg: &VehicleConfig, v: f32) -> (Trace, Option<f32>, f32) {
    let mut r = Rig::new(cfg, flat());
    r.cruise(v);
    let h0 = r.heading();
    r.keys(0.3, Keys { steer: 1, ..Default::default() });
    r.keys(0.6, Keys { steer: 1, handbrake: true, ..Default::default() });
    r.keys(0.3, Keys { steer: 1, ..Default::default() });
    let rec = r.recover();
    let turned = wrap(r.heading() - h0).to_degrees().abs();
    (r.tr, rec, turned)
}

fn wrap(a: f32) -> f32 {
    let t = std::f32::consts::TAU;
    (a + t * 0.5).rem_euclid(t) - t * 0.5
}

/// Short alternating steering taps on a straight at speed.
fn taps(cfg: &VehicleConfig, v: f32) -> (Trace, Option<f32>) {
    let mut r = Rig::new(cfg, flat());
    r.cruise(v);
    for i in 0..6 {
        let s = if i % 2 == 0 { 1 } else { -1 };
        r.run(0.15, |r| Keys { steer: s, gas: r.speed() < v, ..Default::default() }.controls());
        r.run(0.35, |r| Keys { gas: r.speed() < v, ..Default::default() }.controls());
    }
    let rec = r.recover();
    (r.tr, rec)
}

/// ISO 3888-1 style double lane change (3.5 m offset) by a keyboard driver:
/// it looks ahead and presses left or right, or nothing inside a dead band.
/// Returns the trace, the largest path error (m) and the final lane error.
fn lane_change(cfg: &VehicleConfig, v: f32) -> (Trace, f32, f32) {
    let mut r = Rig::new(cfg, flat());
    r.cruise(v);
    let z0 = r.pos().z + 15.0;
    let x0 = r.pos().x;
    let lane = |z: f32| {
        let s = z - z0;
        let ramp = |a: f32, b: f32| ((s - a) / (b - a)).clamp(0.0, 1.0);
        let smooth = |t: f32| t * t * (3.0 - 2.0 * t);
        x0 + 3.5 * (smooth(ramp(0.0, 30.0)) - smooth(ramp(55.0, 80.0)))
    };
    let mut worst: f32 = 0.0;
    let look = (v * 0.7).max(8.0);
    while r.pos().z < z0 + 140.0 && r.speed() > 3.0 && !r.tr.spun() {
        let p = r.pos();
        let f = r.fwd();
        // Where the car will be at the preview point if it keeps going.
        let ahead = p.x + f.x / f.z.max(0.2) * look;
        let e = lane(p.z + look) - ahead;
        let steer = if e.abs() < 0.35 { 0 } else if e > 0.0 { -1 } else { 1 };
        worst = worst.max((lane(p.z) - p.x).abs());
        r.step(Keys { steer, gas: r.speed() < v, ..Default::default() }.controls());
    }
    let end = (lane(r.pos().z) - r.pos().x).abs();
    (r.tr, worst, end)
}

/// Full throttle from rest: (0-100 km/h time, heading drift in degrees,
/// sideways drift in metres, largest slip angle).
fn launch(cfg: &VehicleConfig) -> (Option<f32>, f32, f32, f32) {
    let mut r = Rig::new(cfg, flat());
    let mut t100 = None;
    for i in 0..(8.0 / DT) as usize {
        r.step(Keys { gas: true, ..Default::default() }.controls());
        if t100.is_none() && r.speed() >= 100.0 * KMH {
            t100 = Some((i + 1) as f32 * DT);
        }
    }
    (t100, r.heading().to_degrees().abs(), r.pos().x.abs(), r.tr.beta)
}

/// Full braking in a straight line from 100 km/h: distance (m) and drift.
fn brake_100(cfg: &VehicleConfig) -> (f32, f32) {
    let mut r = Rig::new(cfg, flat());
    r.cruise(100.0 * KMH);
    let z = r.pos().z;
    let mut n = 0;
    while r.speed() > 0.5 && n < 1200 {
        r.step(Keys { brake: true, ..Default::default() }.controls());
        n += 1;
    }
    (r.pos().z - z, r.heading().to_degrees().abs())
}

/// Steady-state grip: steering wound on slowly (an analog stick) at
/// 60 km/h; the best lateral acceleration held for a second, in g.
fn skidpad(cfg: &VehicleConfig) -> f32 {
    let v = 60.0 * KMH;
    let mut r = Rig::new(cfg, flat());
    r.cruise(v);
    let mut window = std::collections::VecDeque::new();
    let mut best: f32 = 0.0;
    let n = (14.0 / DT) as usize;
    for i in 0..n {
        let steer = i as f32 / n as f32;
        let c = VehicleControls { throttle: if r.speed() < v { 1.0 } else { 0.0 }, steer, ..Default::default() };
        r.step(c);
        window.push_back(r.speed() * r.body().angvel.y.abs() / 9.81);
        if window.len() > 60 {
            window.pop_front();
            best = best.max(window.iter().sum::<f32>() / 60.0);
        }
    }
    best
}

/// Crosses a step at `v`, optionally at an angle: (speed kept as a fraction,
/// heading change in degrees, trace).
fn kerb(cfg: &VehicleConfig, v: f32, rise: f32, run: f32, angle_deg: f32) -> (f32, f32, Trace) {
    // Far enough for the cruise run-up at any angle.
    let at = v * 2.5 + 60.0;
    let rot = Quat::from_rotation_y(angle_deg.to_radians());
    let mut r = Rig::at(cfg, step_track(at, rise, run), Iso::new(Vec3::ZERO, rot));
    r.cruise(v);
    let h0 = r.heading();
    let mut guard = 0;
    while r.pos().z < at - 6.0 && guard < 2000 {
        r.step(Keys { gas: r.speed() < v, ..Default::default() }.controls());
        guard += 1;
    }
    let v0 = r.speed();
    r.tr = Trace::default();
    r.run(1.5, |r| Keys { gas: r.speed() < v, ..Default::default() }.controls());
    (r.tr.min_speed / v0, wrap(r.heading() - h0).to_degrees().abs(), r.tr)
}

fn presets(reg: &HandlingRegistry) -> Vec<(&'static str, VehicleConfig)> {
    let mut v = vec![("concept", concept(reg))];
    for name in ["sports", "sedan", "suv", "truck"] {
        v.push((name, reg.get(name).unwrap().clone()));
    }
    v
}

#[test]
#[ignore = "prints the handling table; run with --ignored --nocapture"]
fn handling_report() {
    let reg = registry();
    for (name, cfg) in presets(&reg) {
        println!("== {name}");
        let (t100, drift, side, beta) = launch(&cfg);
        let (dist, bdrift) = brake_100(&cfg);
        println!("  0-100 {} heading drift {drift:.1} deg side {side:.2} m beta {beta:.1} | 100-0 {dist:.1} m drift {bdrift:.1} deg | skidpad {:.2} g", t100.map_or("--".into(), |t| format!("{t:.1}s")), skidpad(&cfg));
        for kmh in [30.0, 60.0, 100.0, 140.0] {
            let v = kmh * KMH;
            println!("  {kmh:3.0} km/h");
            let (t, r) = hold_steer(&cfg, v);
            println!("    hold steer   {}", verdict(&t, r));
            let (t, r) = lift_off(&cfg, v);
            println!("    lift-off     {}", verdict(&t, r));
            let (t, r) = brake_turn(&cfg, v);
            println!("    brake+turn   {}", verdict(&t, r));
            let (t, r) = flick(&cfg, v);
            println!("    left-right   {}", verdict(&t, r));
            let (t, r, turned) = handbrake_tap(&cfg, v);
            println!("    handbrake    {} turned {turned:.0} deg", verdict(&t, r));
            let (t, r) = taps(&cfg, v);
            println!("    taps         {}", verdict(&t, r));
            let (t, worst, end) = lane_change(&cfg, v);
            println!("    lane change  {} path err {worst:.2} m end {end:.2} m", verdict(&t, Some(0.0)));
        }
        for (label, rise, run, angle) in [("2 cm step", 0.02, 0.0, 0.0), ("5 cm step", 0.05, 0.0, 0.0), ("10 cm chamfer", 0.10, 0.10, 0.0), ("10 cm chamfer 25 deg", 0.10, 0.10, 25.0), ("15 cm chamfer", 0.15, 0.15, 0.0)] {
            let (kept, turn, t) = kerb(&cfg, 100.0 * KMH, rise, run, angle);
            println!("  kerb {label:22} speed kept {:.0}% heading {turn:.1} deg up {:.2} beta {:.1}", kept * 100.0, t.up, t.beta);
        }
    }
    let bike = reg.get("sport_bike").unwrap().clone();
    println!("== sport_bike");
    for kmh in [60.0, 100.0, 140.0, 180.0] {
        let v = kmh * KMH;
        let (t, r) = taps(&bike, v);
        println!("  {kmh:3.0} taps        {}  up {:.2}", verdict(&t, r), t.up);
        let (t, r) = hold_steer(&bike, v);
        println!("  {kmh:3.0} hold steer  {}  up {:.2}", verdict(&t, r), t.up);
        let (t, r) = flick(&bike, v);
        println!("  {kmh:3.0} left-right  {}  up {:.2}", verdict(&t, r), t.up);
        let (t, r) = brake_turn(&bike, v);
        println!("  {kmh:3.0} brake+turn  {}  up {:.2}", verdict(&t, r), t.up);
    }
}

// ---------------------------------------------------------------------------
// Regression tests: what a keyboard player does must not spin the car.

fn each_car(mut f: impl FnMut(&str, &VehicleConfig)) {
    let reg = registry();
    for (name, cfg) in presets(&reg) {
        f(name, &cfg);
    }
}

#[test]
fn double_lane_change_at_70_does_not_spin() {
    each_car(|name, cfg| {
        let (t, worst, end) = lane_change(cfg, 70.0 * KMH);
        assert!(t.beta < 10.0 && !t.rolled(), "{name}: slid in the lane change: {t:?}");
        assert!(worst < 1.6 && end < 0.8, "{name}: left the lane: path error {worst:.2} m, {end:.2} m at the end");
    });
}

#[test]
fn lane_change_and_flick_stay_in_control_at_motorway_speed() {
    each_car(|name, cfg| {
        for kmh in [100.0, 140.0] {
            let (t, worst, _) = lane_change(cfg, kmh * KMH);
            assert!(t.beta < 12.0 && worst < 2.5, "{name} at {kmh}: lane change {t:?} path error {worst:.2}");
            let (t, rec) = flick(cfg, kmh * KMH);
            assert!(t.beta < 12.0 && rec.is_some_and(|r| r < 1.5), "{name} at {kmh}: left-right flick {t:?} recovered {rec:?}");
        }
    });
}

#[test]
fn full_throttle_launch_is_straight() {
    each_car(|name, cfg| {
        let (t100, heading, side, beta) = launch(cfg);
        assert!(heading < 2.0 && side < 0.5 && beta < 3.0, "{name}: launch drifted {heading:.1} deg, {side:.2} m, slip {beta:.1}");
        if name != "truck" {
            assert!(t100.is_some(), "{name}: never reached 100 km/h");
        }
    });
}

#[test]
fn straight_at_130_with_steering_taps_is_stable() {
    each_car(|name, cfg| {
        let (t, rec) = taps(cfg, 130.0 * KMH);
        assert!(t.beta < 4.0 && t.yaw < 30.0, "{name}: twitchy at 130 km/h: {t:?}");
        assert!(rec.is_some_and(|r| r < 0.6), "{name}: did not settle after the taps: {rec:?}");
    });
}

#[test]
fn held_steering_key_never_spins_the_car() {
    each_car(|name, cfg| {
        for kmh in [30.0, 60.0, 100.0, 140.0] {
            let (t, rec) = hold_steer(cfg, kmh * KMH);
            assert!(t.beta < 14.0 && !t.rolled(), "{name} at {kmh}: {t:?}");
            assert!(rec.is_some_and(|r| r < 1.5), "{name} at {kmh}: no recovery {rec:?}");
        }
    });
}

#[test]
fn trail_braking_into_a_corner_is_recoverable() {
    each_car(|name, cfg| {
        for kmh in [60.0, 100.0, 140.0] {
            let (t, rec) = brake_turn(cfg, kmh * KMH);
            assert!(t.beta < 18.0 && !t.rolled(), "{name} at {kmh}: {t:?}");
            assert!(rec.is_some_and(|r| r < 1.5), "{name} at {kmh}: no recovery {rec:?}");
        }
    });
}

#[test]
fn lift_off_oversteer_is_mild() {
    each_car(|name, cfg| {
        for kmh in [60.0, 100.0, 140.0] {
            let (t, rec) = lift_off(cfg, kmh * KMH);
            assert!(t.beta < 11.0, "{name} at {kmh}: lift-off slip {t:?}");
            assert!(rec.is_some_and(|r| r < 1.2), "{name} at {kmh}: no recovery {rec:?}");
        }
    });
}

#[test]
fn handbrake_turn_is_possible_but_recoverable() {
    each_car(|name, cfg| {
        // In town it turns the car...
        let (t, rec, turned) = handbrake_tap(cfg, 40.0 * KMH);
        assert!(turned > 40.0, "{name}: handbrake turned only {turned:.0} deg");
        assert!(!t.spun() && rec.is_some_and(|r| r < 1.5), "{name}: handbrake turn at 40: {t:?} {rec:?}");
        // ...and at speed it is a slide the car comes back from, hands off.
        for kmh in [70.0, 100.0] {
            let (t, rec, turned) = handbrake_tap(cfg, kmh * KMH);
            assert!(turned > 15.0, "{name} at {kmh}: no slide ({turned:.0} deg)");
            assert!(t.beta > 12.0, "{name} at {kmh}: the tail did not step out: {t:?}");
            assert!(t.beta < 50.0 && !t.rolled() && rec.is_some_and(|r| r < 3.0), "{name} at {kmh}: {t:?} recovered {rec:?}");
        }
    });
}

#[test]
fn steps_and_kerbs_do_not_stop_the_car() {
    each_car(|name, cfg| {
        for (label, rise, run, angle) in [("2 cm step", 0.02, 0.0, 0.0), ("5 cm step", 0.05, 0.0, 0.0), ("10 cm chamfered kerb", 0.10, 0.10, 0.0), ("10 cm chamfered kerb at 25 deg", 0.10, 0.10, 25.0)] {
            let (kept, turn, t) = kerb(cfg, 100.0 * KMH, rise, run, angle);
            assert!(kept > 0.95, "{name}: lost {:.0}% of its speed on a {label}", (1.0 - kept) * 100.0);
            assert!(turn < 3.0 && t.beta < 3.0 && t.up > 0.97, "{name}: unsettled by a {label}: heading {turn:.1} deg, {t:?}");
        }
    });
}

#[test]
fn road_mesh_seams_are_invisible_at_speed() {
    // A flat road made of triangle strips: nothing to hit.
    each_car(|name, cfg| {
        // (The truck cannot hold 180 km/h: it loses a little to drag.)
        let (kept, turn, t) = kerb(cfg, 180.0 * KMH, 0.0, 0.0, 12.0);
        assert!(kept > 0.9 && turn < 1.0 && t.up > 0.995 && t.beta < 1.0, "{name}: caught a mesh seam: kept {kept:.2}, heading {turn:.1}, {t:?}");
    });
}

#[test]
fn performance_figures_are_realistic() {
    // (0-100 km/h s, 100-0 m, skidpad g) ranges per class.
    let expect = |name: &str| match name {
        "concept" | "sports" => ((4.0, 6.5), (30.0, 37.0), (0.92, 1.05)),
        "sedan" => ((6.0, 9.0), (35.0, 42.0), (0.8, 0.95)),
        "suv" => ((6.5, 10.0), (38.0, 46.0), (0.74, 0.9)),
        _ => ((8.0, f32::MAX), (42.0, 56.0), (0.62, 0.8)),
    };
    each_car(|name, cfg| {
        let ((a0, a1), (b0, b1), (g0, g1)) = expect(name);
        let (t100, ..) = launch(cfg);
        let t100 = t100.unwrap_or(f32::MAX);
        assert!(t100 >= a0 && t100 <= a1, "{name}: 0-100 km/h in {t100:.1} s");
        let (dist, drift) = brake_100(cfg);
        assert!(dist >= b0 && dist <= b1, "{name}: 100-0 km/h in {dist:.1} m");
        assert!(drift < 2.0, "{name}: pulled {drift:.1} deg under braking");
        let g = skidpad(cfg);
        assert!(g >= g0 && g <= g1, "{name}: skidpad {g:.2} g");
    });
}

#[test]
fn analog_steering_is_proportional() {
    // Half stick: about half the cornering of the full stick, not a switch.
    each_car(|name, cfg| {
        let v = 80.0 * KMH;
        let run = |steer: f32| {
            let mut r = Rig::new(cfg, flat());
            r.cruise(v);
            r.run(3.0, |r| VehicleControls { throttle: if r.speed() < v { 1.0 } else { 0.0 }, steer, ..Default::default() });
            r.speed() * r.body().angvel.y.abs() / 9.81
        };
        let (half, full) = (run(0.5), run(1.0));
        assert!(half > 0.3 * full && half < 0.8 * full, "{name}: half stick {half:.2} g, full {full:.2} g");
        assert!(run(0.1) < 0.35 * full, "{name}: a tenth of the stick is not a gentle turn");
    });
}

#[test]
fn sport_bike_is_stable_at_speed() {
    let bike = registry().get("sport_bike").unwrap().clone();
    for kmh in [100.0, 140.0, 180.0] {
        let v = kmh * KMH;
        for (label, (t, rec)) in [("taps", taps(&bike, v)), ("flick", flick(&bike, v)), ("held lean", hold_steer(&bike, v)), ("brake in a turn", brake_turn(&bike, v))] {
            assert!(t.up > 0.6, "bike fell at {kmh} km/h ({label}): {t:?}");
            assert!(t.beta < 14.0, "bike slid at {kmh} km/h ({label}): {t:?}");
            assert!(rec.is_some_and(|r| r < 1.5), "bike did not settle at {kmh} km/h ({label}): {rec:?}");
        }
    }
}
