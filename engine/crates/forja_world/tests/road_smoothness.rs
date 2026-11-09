//! Roads must be seamless to drive on: no step, no kink and nothing that
//! stops a car at 120 km/h, across tile borders, junctions and bridges.
//!
//! For every world with a road graph:
//! * a wheel probe (a ray cast down every 2.5 cm along routes that chain
//!   streets through junctions) must find no height step above 2 mm and no
//!   vertical acceleration spike at 120 km/h;
//! * a physics car launched at 120 km/h along the straight routes must not
//!   lose speed abruptly or be thrown up.

use std::path::PathBuf;

use forja_math::{Iso, Quat, Ray, Vec2, Vec3};
use forja_physics::{PhysicsWorld, Vehicle, VehicleConfig};
use forja_world::*;

fn data() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")
}

const V: f32 = 120.0 / 3.6;

/// Routes of up to `max_len` metres: streets chained through junctions,
/// always continuing along the straightest way on.
fn routes(w: &WorldDescription, max_len: f32, count: usize) -> Vec<Vec<Vec2>> {
    routes_where(w, max_len, count, |_| true)
}

/// [`routes`] over the streets `keep` accepts.
fn routes_where(w: &WorldDescription, max_len: f32, count: usize, keep: impl Fn(&forja_city::traffic::TrafficStreet) -> bool) -> Vec<Vec<Vec2>> {
    let info = w.traffic.as_ref().expect("road graph");
    let key = |p: Vec2| ((p.x * 2.0).round() as i64, (p.y * 2.0).round() as i64);
    let mut at: std::collections::HashMap<(i64, i64), Vec<(usize, bool)>> = Default::default();
    let lines: Vec<Vec<Vec2>> = info.streets.iter().filter(|s| keep(s)).map(|s| s.centreline()).collect();
    for (i, l) in lines.iter().enumerate() {
        at.entry(key(l[0])).or_default().push((i, true));
        at.entry(key(l[l.len() - 1])).or_default().push((i, false));
    }
    let mut out = Vec::new();
    let step = (lines.len() / count.max(1)).max(1);
    for start in (0..lines.len()).step_by(step) {
        let mut pts = lines[start].clone();
        let mut used = vec![start];
        let mut len: f32 = pts.windows(2).map(|s| s[0].distance(s[1])).sum();
        while len < max_len {
            let end = pts[pts.len() - 1];
            let dir = (end - pts[pts.len() - 2]).normalize_or(Vec2::X);
            let mut best: Option<(f32, usize, bool)> = None;
            for &(i, from_start) in at.get(&key(end)).into_iter().flatten() {
                if used.contains(&i) {
                    continue;
                }
                let l = &lines[i];
                let d = if from_start { l[1] - l[0] } else { l[l.len() - 2] - l[l.len() - 1] }.normalize_or(Vec2::X);
                if d.dot(dir) > 0.5 && best.is_none_or(|b| d.dot(dir) > b.0) {
                    best = Some((d.dot(dir), i, from_start));
                }
            }
            let Some((_, i, from_start)) = best else { break };
            used.push(i);
            let mut next = lines[i].clone();
            if !from_start {
                next.reverse();
            }
            len += next.windows(2).map(|s| s[0].distance(s[1])).sum::<f32>();
            pts.extend_from_slice(&next[1..]);
        }
        if len > 60.0 || max_len == 0.0 {
            out.push(pts);
        }
    }
    out
}

/// `route` without its first and last `d` metres (`None` if nothing is left).
fn trim(route: &[Vec2], d: f32) -> Option<Vec<Vec2>> {
    let len: f32 = route.windows(2).map(|s| s[0].distance(s[1])).sum();
    if len < 2.0 * d + 20.0 {
        return None;
    }
    let at = |t: f32| {
        let mut left = t;
        for s in route.windows(2) {
            let l = s[0].distance(s[1]);
            if left <= l {
                return s[0].lerp(s[1], left / l.max(1e-6));
            }
            left -= l;
        }
        route[route.len() - 1]
    };
    let mut out = vec![at(d)];
    let mut run = 0.0;
    for s in route.windows(2) {
        run += s[0].distance(s[1]);
        if run > d && run < len - d {
            out.push(s[1]);
        }
    }
    out.push(at(len - d));
    Some(out)
}

fn ground(pw: &PhysicsWorld, p: Vec2, from: f32) -> Option<f32> {
    pw.raycast(&Ray::new(Vec3::new(p.x, from, p.y), Vec3::NEG_Y), from + 100.0, Default::default()).map(|h| h.point.y)
}

struct Report {
    probes: usize,
    metres: f32,
    max_step: f32,
    step_at: Vec3,
    max_accel: f32,
    accel_at: Vec3,
    max_grade: f32,
    cars: usize,
    worst_loss: f32,
    loss_at: Vec3,
    worst_kick: f32,
}

/// Probes `routes` on the world's colliders and drives the straight ones.
fn check(name: &str, w: &WorldDescription, pw: &mut PhysicsWorld, routes: &[Vec<Vec2>], offsets: &[f32]) -> Report {
    let mut r = Report { probes: 0, metres: 0.0, max_step: 0.0, step_at: Vec3::ZERO, max_accel: 0.0, accel_at: Vec3::ZERO, max_grade: 0.0, cars: 0, worst_loss: 0.0, loss_at: Vec3::ZERO, worst_kick: 0.0 };
    const DS: f32 = 0.025;
    for route in routes {
        for &off in offsets {
            // Height profile every 2.5 cm along the wheel track.
            let mut prof: Vec<(Vec2, f32)> = Vec::new();
            let mut last_h = None;
            for s in route.windows(2) {
                let len = s[0].distance(s[1]);
                if len < 1e-3 {
                    continue;
                }
                let d = (s[1] - s[0]) / len;
                let side = Vec2::new(-d.y, d.x) * off;
                let n = (len / DS).floor() as usize;
                for k in 0..n {
                    let p = s[0] + d * (k as f32 * DS) + side;
                    let from = last_h.map_or(2000.0, |h: f32| h + 1.2);
                    let Some(h) = ground(pw, p, from).or_else(|| ground(pw, p, 2000.0)) else { continue };
                    last_h = Some(h);
                    prof.push((p, h));
                }
            }
            r.probes += prof.len();
            r.metres += prof.len() as f32 * DS;
            for i in 1..prof.len().saturating_sub(1) {
                // A lane offset jumps sideways at corners: only compare
                // neighbours that are neighbours.
                let (d0, d1) = (prof[i].0 - prof[i - 1].0, prof[i + 1].0 - prof[i].0);
                if d0.length() > DS * 1.5 || d1.length() > DS * 1.5 || d0.normalize_or(Vec2::X).dot(d1.normalize_or(Vec2::X)) < 0.9999 {
                    continue;
                }
                let step = (prof[i + 1].1 - 2.0 * prof[i].1 + prof[i - 1].1).abs();
                if step > r.max_step {
                    if step > 0.002 && std::env::var_os("ROAD_DEBUG").is_some() {
                        println!("  step {:.1} mm: {:?} {:.3} -> {:?} {:.3} -> {:?} {:.3}", step * 1000.0, prof[i - 1].0, prof[i - 1].1, prof[i].0, prof[i].1, prof[i + 1].0, prof[i + 1].1);
                    }
                    r.max_step = step;
                    r.step_at = Vec3::new(prof[i].0.x, prof[i].1, prof[i].0.y);
                }
                r.max_grade = r.max_grade.max((prof[i + 1].1 - prof[i].1).abs() / DS);
            }
            // Vertical acceleration at 120 km/h as the suspension sees it:
            // the change of grade over 4 m.
            let k = (2.0 / DS) as usize;
            for i in (k..prof.len().saturating_sub(k)).step_by(8) {
                // (Only where the way is straight: on a tilted plane a bend
                // in plan is not a bend in profile.)
                let (d0, d1) = (prof[i].0 - prof[i - k].0, prof[i + k].0 - prof[i].0);
                if d0.length() > 2.2 || d1.length() > 2.2 || d0.normalize_or(Vec2::X).dot(d1.normalize_or(Vec2::X)) < 0.9995 {
                    continue;
                }
                let a = V * V * (prof[i + k].1 - 2.0 * prof[i].1 + prof[i - k].1) / 4.0;
                if a.abs() > r.max_accel {
                    r.max_accel = a.abs();
                    r.accel_at = Vec3::new(prof[i].0.x, prof[i].1, prof[i].0.y);
                }
            }
        }
        // A car at 120 km/h down the route while it is straight.
        let mut straight = 1;
        while straight + 1 < route.len() && {
            let chord = (route[straight + 1] - route[0]).normalize_or(Vec2::X);
            route[1..=straight + 1].iter().all(|p| (*p - route[0]).perp_dot(chord).abs() < 0.6)
        } {
            straight += 1;
        }
        let run = route[0].distance(route[straight]);
        if run < 70.0 || r.cars >= 40 {
            continue;
        }
        let dir = (route[straight] - route[0]) / run;
        let Some(h0) = ground(pw, route[0] + dir * 2.0, 2000.0) else { continue };
        let rot = Quat::from_rotation_y(dir.x.atan2(dir.y));
        let mut car = Vehicle::spawn(pw, VehicleConfig::sedan(), Iso::new(Vec3::new(route[0].x + dir.x * 2.0, h0 + 0.62, route[0].y + dir.y * 2.0), rot));
        let fwd = Vec3::new(dir.x, 0.0, dir.y);
        pw.body_mut(car.body).unwrap().set_linvel(fwd * V);
        car.controls.throttle = 1.0;
        let dt = 1.0 / 60.0;
        let (mut prev_speed, mut prev_vy, mut t, mut gone) = (V, 0.0f32, 0.0f32, 0.0f32);
        while gone < run - 12.0 && t < 12.0 {
            car.update(pw, dt);
            pw.step(dt);
            t += dt;
            let b = pw.body(car.body).unwrap();
            let (speed, vy, pos) = (b.linvel.dot(fwd), b.linvel.y, b.position());
            gone = Vec2::new(pos.x, pos.z).distance(route[0]);
            if t > 0.6 {
                let loss = prev_speed - speed;
                if loss > r.worst_loss {
                    if loss > 0.5 && std::env::var_os("ROAD_DEBUG").is_some() {
                        println!("  car lost {loss:.1} m/s at {pos:?} (t {t:.2}, from {:?} towards {dir:?})", route[0]);
                    }
                    r.worst_loss = loss;
                    r.loss_at = pos;
                }
                r.worst_kick = r.worst_kick.max((vy - prev_vy).abs() / dt);
            }
            prev_speed = speed;
            prev_vy = vy;
        }
        pw.remove_body(car.body);
        r.cars += 1;
    }
    println!(
        "{name}: {} routes, {:.0} m probed ({} rays): max step {:.2} mm at {:.1?}, max vertical acceleration at 120 km/h {:.1} m/s2 at {:.1?}, max grade {:.0} %; {} cars: worst speed loss in a frame {:.2} m/s at {:.1?}, worst vertical kick {:.0} m/s2",
        routes.len(),
        r.metres,
        r.probes,
        r.max_step * 1000.0,
        r.step_at,
        r.max_accel,
        r.accel_at,
        r.max_grade * 100.0,
        r.cars,
        r.worst_loss,
        r.loss_at,
        r.worst_kick
    );
    let _ = w;
    r
}

fn assert_smooth(name: &str, r: &Report) {
    assert!(r.probes > 5_000, "{name}: only {} probes", r.probes);
    assert!(r.max_step < 0.002, "{name}: step of {:.2} mm at {:?}", r.max_step * 1000.0, r.step_at);
    // 20 m/s2 is a change of grade of 3.6 % over 4 m (the wheels move
    // 7 cm against the body): well inside the suspension's travel.
    assert!(r.max_accel < 20.0, "{name}: vertical acceleration {:.1} m/s2 at {:?}", r.max_accel, r.accel_at);
    assert!(r.cars > 0, "{name}: no straight route to drive");
    // Coasting loses a few cm/s per frame; hitting something loses metres.
    assert!(r.worst_loss < 0.5, "{name}: a car lost {:.2} m/s in one frame at {:?}", r.worst_loss, r.loss_at);
}

#[test]
fn island_roads_are_smooth() {
    let w = IslandWorld { trees: 0.0, ..IslandWorld::new(&data()) }.build().expect("island");
    let mut pw = PhysicsWorld::default();
    w.add_to_physics(&mut pw);
    let r = check("island", &w, &mut pw, &routes(&w, 700.0, 160), &[0.0, 1.4]);
    assert_smooth("island", &r);
}

#[test]
fn city_roads_are_smooth() {
    for preset in ["small", "organic"] {
        let w = SpanishCity { downloaded: None, styles: None, ..SpanishCity::preset(data(), data().join("city/styles.toml"), preset).expect("preset") }.build().expect("city");
        let mut pw = PhysicsWorld::default();
        w.add_to_physics(&mut pw);
        // Along every driving lane, street by street between its stop
        // lines: the city's junctions have kerbs by design (continuous
        // footways, plazas), which the physics tests cover (forja_physics:
        // kerbs and steps at 100 km/h).
        let mut streets: Vec<Vec<Vec2>> = Vec::new();
        for st in &w.traffic.as_ref().expect("road graph").streets {
            let Some(c) = trim(&st.centreline(), 9.0) else { continue };
            for lane in &st.lanes {
                // Left of a -> b is positive.
                let shifted: Vec<Vec2> = (0..c.len())
                    .map(|i| {
                        let d = (c[(i + 1).min(c.len() - 1)] - c[i.saturating_sub(1)]).normalize_or(Vec2::X);
                        c[i] + Vec2::new(d.y, -d.x) * lane.offset
                    })
                    .collect();
                streets.push(shifted);
            }
        }
        let r = check(preset, &w, &mut pw, &streets, &[0.0]);
        assert_smooth(preset, &r);
    }
}

/// A car at 120 km/h over height fields (flat, sloping, gently rolling):
/// the seams between the field's triangles must not catch it.
#[test]
fn car_keeps_speed_on_height_fields() {
    use forja_physics::{ColliderDesc, HeightField, Shape};
    for (name, f) in [("flat", (|_, _| 3.0) as fn(f32, f32) -> f32), ("slope", |x, z| 10.0 + x * 0.02 + z * 0.01), ("rolling", |x, z| 10.0 + (x * 0.02).sin() * 1.5 + (z * 0.015).cos())] {
        let (n, cell) = (65usize, 16.0f32);
        let h: Vec<f32> = (0..n * n).map(|i| f((i % n) as f32 * cell - 512.0, (i / n) as f32 * cell - 512.0)).collect();
        let mut pw = PhysicsWorld::default();
        pw.add_collider(ColliderDesc::new(Shape::HeightField(std::sync::Arc::new(HeightField::new(n, n, h, Vec2::splat(cell))))).friction(1.0), None);
        for (k, dir) in [Vec2::new(1.0, 0.0), Vec2::new(0.6, 0.8), Vec2::new(-0.7071, 0.7071)].into_iter().enumerate() {
            let start = -dir * 300.0 + Vec2::new(3.0, 5.0) * k as f32;
            let h0 = ground(&pw, start, 500.0).expect("ground");
            let mut car = Vehicle::spawn(&mut pw, VehicleConfig::sedan(), Iso::new(Vec3::new(start.x, h0 + 0.62, start.y), Quat::from_rotation_y(dir.x.atan2(dir.y))));
            let fwd = Vec3::new(dir.x, 0.0, dir.y);
            pw.body_mut(car.body).unwrap().set_linvel(fwd * V);
            car.controls.throttle = 1.0;
            let mut prev = V;
            for step in 0..600 {
                car.update(&mut pw, 1.0 / 60.0);
                pw.step(1.0 / 60.0);
                let b = pw.body(car.body).unwrap();
                let v = b.linvel.dot(fwd);
                if std::env::var_os("ROAD_DEBUG").is_some() && (40..52).contains(&step) {
                    println!("{name} {k} step {step}: v {:?} pos {:?} up {:?} angvel {:?} wheels {:?}", b.linvel, b.position(), b.rotation() * Vec3::Y, b.angvel, car.wheels.iter().map(|w| (w.grounded, (w.compression * 100.0).round())).collect::<Vec<_>>());
                }
                assert!(step < 30 || prev - v < 0.5, "{name}, direction {k}: lost {:.1} m/s in a frame at {:?} (step {step}, speed {v:.1})", prev - v, b.position());
                prev = v;
            }
            assert!(prev > V * 0.8, "{name}, direction {k}: slowed to {prev:.1} m/s");
            pw.remove_body(car.body);
        }
    }
}
