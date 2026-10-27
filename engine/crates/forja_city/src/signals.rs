//! Valencian traffic signals modelled from photographs (Corts Valencianes,
//! Ausiàs March, Av. del Cid, the bike-lane crossings; see
//! `data/furniture_refs/traffic_lights.toml`): 110 mm steel columns painted
//! grey-green or anthracite, tall bent mast arms over the avenues,
//! anthracite polycarbonate heads of 200 mm LED lenses with deep tubular
//! visors and no backplate, side repeater heads, two-aspect pedestrian
//! heads (standing red man / walking green man) facing the crosswalk with
//! an optional countdown and acoustic unit, 100 mm bicycle heads on the
//! bike lanes and yellow push-button boxes.
//!
//! Origin at the base of the pole, vehicle heads face +Z (towards the
//! approaching traffic), a mast arm reaches along +X over the road.
//! [`crate::furniture::traffic_light`] keeps its signature and builds the
//! default [`SignalSpec`]; the state ([`Aspect`]) is a parameter so the
//! traffic AI can swap prebuilt variants (a handful per spec).

use forja_math::{Mat4, Quat, Vec2, Vec3};

use crate::furniture::Aspect;
use crate::kit::{Mat, Rgb, hex};
use crate::modules::P;
use crate::nightlight::{CityLight, LightKind};
use crate::street_waste::{label_at, loft, rounded_ring};
use crate::streettex::Label;

fn v(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(x, y, z)
}

/// Pole paints seen in Valencia.
pub const POLE_GREY_GREEN: Rgb = hex(0x8f9a95);
pub const POLE_ANTHRACITE: Rgb = hex(0x3b3f42);
/// Head housing: anthracite polycarbonate.
const HOUSING: Rgb = hex(0x2a2d2f);

/// How the signal is mounted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mount {
    /// Straight 110 mm column, 3.6 m.
    Column,
    /// Bent mast ("báculo"): 6 m column curving into a horizontal arm that
    /// reaches `reach` metres over the road (+X), main head hanging from
    /// its tip and a repeater on the column.
    Mast { reach: f32 },
}

/// Full description of a signal post.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SignalSpec {
    pub mount: Mount,
    pub pole: Rgb,
    /// Second vehicle head beside the first on a T bracket (left-turn or
    /// double head of the avenues).
    pub twin: bool,
    /// Small low repeater facing the stop line at 2.3 m.
    pub low_repeater: bool,
    /// Pedestrian head facing `ped_yaw`.
    pub ped: bool,
    /// Countdown display above the pedestrian head.
    pub countdown: bool,
    /// Acoustic unit for blind pedestrians on top of the pedestrian head.
    pub acoustic: bool,
    /// Yellow push-button box at 1.1 m on the crosswalk side.
    pub button: bool,
    /// 100 mm bicycle head on the column (bike-lane crossings).
    pub bike: bool,
}

impl SignalSpec {
    /// Street corner column: grey-green, pedestrian head, acoustic unit.
    pub const CORNER: SignalSpec = SignalSpec { mount: Mount::Column, pole: POLE_GREY_GREEN, twin: false, low_repeater: false, ped: true, countdown: false, acoustic: true, button: false, bike: false };

    /// The spec [`crate::furniture::traffic_light`] builds for `arm`.
    pub fn for_arm(arm: f32) -> SignalSpec {
        if arm > 0.0 {
            SignalSpec { mount: Mount::Mast { reach: arm }, pole: POLE_ANTHRACITE, twin: false, low_repeater: true, ped: true, countdown: true, acoustic: false, button: true, bike: false }
        } else {
            SignalSpec::CORNER
        }
    }
}

/// Kinds of head.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Head {
    /// Three 200 mm aspects; `small` = 100 mm repeater.
    Vehicle { small: bool },
    Pedestrian,
    Bicycle,
}

/// Head placements of a spec: kind, centre of the head front, yaw (0 = +Z).
fn heads(s: &SignalSpec, ped_yaw: f32) -> Vec<(Head, Vec3, f32)> {
    let mut out = Vec::new();
    match s.mount {
        Mount::Column => {
            out.push((Head::Vehicle { small: false }, v(0.0, 2.95, 0.19), 0.0));
            if s.twin {
                out.push((Head::Vehicle { small: false }, v(0.42, 2.95, 0.19), 0.0));
            }
        }
        Mount::Mast { reach } => {
            out.push((Head::Vehicle { small: false }, v(reach - 0.05, 5.25, 0.17), 0.0));
            if s.twin {
                out.push((Head::Vehicle { small: false }, v(reach - 0.5, 5.25, 0.17), 0.0));
            }
            out.push((Head::Vehicle { small: false }, v(0.0, 2.95, 0.19), 0.0));
        }
    }
    if s.low_repeater {
        out.push((Head::Vehicle { small: true }, v(0.0, 2.2, 0.12), 0.0));
    }
    if s.bike {
        out.push((Head::Bicycle, v(-0.2, 2.25, 0.08), 0.0));
    }
    if s.ped {
        let d = Quat::from_rotation_y(ped_yaw) * Vec3::Z;
        out.push((Head::Pedestrian, v(0.0, 2.45, 0.0) + d * 0.19, ped_yaw));
    }
    out
}

/// Lens centres (relative to the head front centre, local to the head)
/// and their aspect, top to bottom.
fn lens_layout(h: Head) -> (f32, f32, Vec<f32>) {
    // (lens radius, cell pitch, offsets)
    match h {
        Head::Vehicle { small: false } => (0.1, 0.27, vec![0.27, 0.0, -0.27]),
        Head::Vehicle { small: true } | Head::Bicycle => (0.05, 0.14, vec![0.14, 0.0, -0.14]),
        Head::Pedestrian => (0.1, 0.27, vec![0.135, -0.135]),
    }
}

/// Anthracite housing with rounded vertical edges, `w × h × d`, front at
/// z = 0 facing +Z, centred on the origin.
fn housing(p: &mut P, w: f32, h: f32, d: f32) {
    let m = Mat::PowderCoat(HOUSING);
    let rings = vec![rounded_ring(w * 0.5, d * 0.5, 0.03, -h * 0.5, -d * 0.5, 2), rounded_ring(w * 0.5, d * 0.5, 0.03, h * 0.5, -d * 0.5, 2)];
    loft(p.get(m), &rings, true, true, 1.0);
}

/// Tubular visor (open underneath) of radius `r` and depth `len` around a
/// lens at `c`, double surfaced.
fn visor(p: &mut P, c: Vec3, r: f32, len: f32) {
    let m = Mat::PowderCoat(HOUSING);
    let seg = 10;
    let (a0, a1) = (-0.35f32, std::f32::consts::PI + 0.35);
    let prim = p.get(m);
    for (rr, out) in [(r + 0.008, true), (r, false)] {
        let base = prim.positions.len() as u32;
        for k in 0..=seg {
            let a = a0 + (a1 - a0) * k as f32 / seg as f32;
            let d = v(a.cos(), a.sin(), 0.0);
            let n = if out { d } else { -d };
            // Visors are cut at an angle: longer on top.
            let l = len * (0.55 + 0.45 * a.sin().max(0.0));
            prim.vertex(c + d * rr, n, Vec2::new(k as f32 * 0.1, 0.0));
            prim.vertex(c + d * rr + Vec3::Z * l, n, Vec2::new(k as f32 * 0.1, 1.0));
        }
        for k in 0..seg as u32 {
            let i = base + k * 2;
            if out {
                p_tri(prim, [i, i + 2, i + 3, i, i + 3, i + 1]);
            } else {
                p_tri(prim, [i, i + 3, i + 2, i, i + 1, i + 3]);
            }
        }
    }
}

fn p_tri(prim: &mut crate::geom::Prim, idx: [u32; 6]) {
    prim.indices.extend(idx);
}

const RED: Rgb = hex(0xff2a10);
const AMBER: Rgb = hex(0xffa000);
const GREEN: Rgb = hex(0x20ff70);

/// One head at the origin (front centre), facing +Z.
fn head(h: Head, state: Aspect, walk: bool, spec: &SignalSpec) -> P {
    let mut p = P::new();
    let (r, pitch, offs) = lens_layout(h);
    let w = r * 2.0 + 0.09;
    let hh = pitch * offs.len() as f32 + 0.04;
    let d = if r > 0.07 { 0.2 } else { 0.12 };
    p.append(&{
        let mut q = P::new();
        housing(&mut q, w, hh, d);
        q
    }, &Mat4::IDENTITY);
    let black = Mat::Plastic(hex(0x0c0c0c));
    for (i, y) in offs.iter().enumerate() {
        let c = v(0.0, *y, 0.0);
        // Black lens surround then the LED lens.
        p.get(black).cylinder(c, c + v(0.0, 0.0, 0.006), r + 0.012, 16, true);
        let lens = match h {
            Head::Vehicle { .. } => {
                let (a, col) = [(Aspect::Red, RED), (Aspect::Amber, AMBER), (Aspect::Green, GREEN)][i];
                Mat::Lens(col, a == state)
            }
            Head::Bicycle => {
                let a = [Aspect::Red, Aspect::Amber, Aspect::Green][i];
                Mat::BikeLens(i as u8, a == state)
            }
            Head::Pedestrian => {
                let is_walk = i == 1;
                if is_walk == walk { Mat::PedLens(is_walk) } else { Mat::Lens(if is_walk { GREEN } else { RED }, false) }
            }
        };
        if matches!(h, Head::Vehicle { .. }) {
            p.get(lens).cone(c + v(0.0, 0.0, 0.006), c + v(0.0, 0.0, 0.02), r, r * 0.8, 16, true);
        } else {
            let (a, b) = (Vec2::new(0.0, 1.0), Vec2::new(1.0, 0.0));
            let z = 0.008;
            p.get(lens).quad_uv([c + v(-r, -r, z), c + v(r, -r, z), c + v(r, r, z), c + v(-r, r, z)], [a, Vec2::ONE, b, Vec2::ZERO]);
        }
        if h == Head::Pedestrian {
            // Flat hood over each pedestrian aspect.
            p.get(Mat::PowderCoat(HOUSING)).aabox(c + v(-r - 0.02, r + 0.015, 0.0), c + v(r + 0.02, r + 0.025, 0.16), 1.0);
        } else {
            visor(&mut p, c, r + 0.012, if r > 0.07 { 0.2 } else { 0.1 });
        }
    }
    if h == Head::Pedestrian {
        let top = hh * 0.5;
        if spec.countdown {
            let (cw, ch) = (w, 0.16);
            p.get(Mat::PowderCoat(HOUSING)).aabox(v(-cw * 0.5, top, -d), v(cw * 0.5, top + ch, 0.0), 1.0);
            let m = if walk { Mat::Countdown(12) } else { Mat::Plastic(hex(0x080808)) };
            let (a, b) = (Vec2::new(0.0, 1.0), Vec2::new(1.0, 0.0));
            p.get(m).quad_uv([v(-0.11, top + 0.025, 0.004), v(0.11, top + 0.025, 0.004), v(0.11, top + 0.135, 0.004), v(-0.11, top + 0.135, 0.004)], [a, Vec2::ONE, b, Vec2::ZERO]);
        }
        if spec.acoustic {
            let y = top + if spec.countdown { 0.16 } else { 0.0 };
            p.get(Mat::PowderCoat(hex(0x55595c))).aabox(v(-0.08, y, -0.14), v(0.08, y + 0.1, -0.02), 1.0);
            for k in 0..4 {
                let x = -0.05 + k as f32 * 0.033;
                p.get(black).aabox(v(x, y + 0.025, -0.019), v(x + 0.018, y + 0.075, -0.017), 1.0);
            }
        }
    }
    p
}

fn pole(p: &mut P, s: &SignalSpec) {
    let m = Mat::PowderCoat(s.pole);
    let top = match s.mount {
        Mount::Column => 3.6,
        Mount::Mast { .. } => 5.2,
    };
    // Base with a foot ring and the column.
    p.get(m).lathe(Vec3::ZERO, &[(0.0, 0.0), (0.1, 0.0), (0.1, 0.05), (0.075, 0.09), (0.065, 0.35)], 12);
    let r1 = if matches!(s.mount, Mount::Mast { .. }) { 0.075 } else { 0.055 };
    p.get(m).cone(v(0.0, 0.3, 0.0), v(0.0, top, 0.0), r1 + 0.01, r1, 12, false);
    match s.mount {
        Mount::Column => {
            p.get(m).lathe(v(0.0, top, 0.0), &[(0.06, 0.0), (0.06, 0.03), (0.03, 0.06), (0.0, 0.07)], 12);
        }
        Mount::Mast { reach } => {
            // Quarter bend of radius 0.9 m then a straight arm, tapering.
            let rb = 0.9;
            let mut prev = v(0.0, top, 0.0);
            let n = 8;
            for i in 1..=n {
                let a = i as f32 / n as f32 * std::f32::consts::FRAC_PI_2;
                let q = v(rb - rb * a.cos(), top + rb * a.sin(), 0.0);
                p.get(m).cylinder(prev, q, r1 - 0.005, 10, false);
                prev = q;
            }
            let end = v(reach + 0.1, top + rb, 0.0);
            p.get(m).cone(prev, end, r1 - 0.005, 0.045, 10, true);
            // Hanger from the arm tip down to the head.
            p.get(m).cylinder(v(reach - 0.05, top + rb, 0.0), v(reach - 0.05, 5.25 + 0.45, 0.0), 0.025, 8, false);
        }
    }
}

/// Bracket from the pole (or hanger) to the back of a head.
fn bracket(p: &mut P, s: &SignalSpec, at: Vec3, yaw: f32, depth: f32) {
    let m = Mat::PowderCoat(s.pole);
    let back = at - Quat::from_rotation_y(yaw) * Vec3::Z * depth;
    let axis = if (at.x - 0.0).abs() > 0.3 { v(at.x, 0.0, 0.0) } else { Vec3::ZERO };
    for dy in [-0.22f32, 0.22] {
        let a = axis + Vec3::Y * (at.y + dy);
        let b = back + Vec3::Y * dy;
        if (b - a).length() > 0.02 {
            p.get(m).beam(a, b, Vec3::Y, 0.05, 0.03, 1.0);
        }
    }
}

/// Signal post of `spec` showing `state` to vehicles and walk / don't walk
/// to pedestrians, the pedestrian head facing `ped_yaw` (radians around Y).
pub fn signal_post(spec: &SignalSpec, state: Aspect, ped_walk: bool, ped_yaw: f32) -> P {
    let mut p = P::new();
    pole(&mut p, spec);
    for (h, at, yaw) in heads(spec, ped_yaw) {
        let (_, pitch, offs) = lens_layout(h);
        let hh = pitch * offs.len() as f32 + 0.04;
        let depth = if matches!(h, Head::Vehicle { small: true } | Head::Bicycle) { 0.12 } else { 0.2 };
        let q = head(h, state, ped_walk, spec);
        let m = Mat4::from_translation(at) * Mat4::from_quat(Quat::from_rotation_y(yaw));
        p.append(&q, &m);
        let hanging = matches!(spec.mount, Mount::Mast { reach } if (at.x - (reach - 0.05)).abs() < 0.01 || (at.x - (reach - 0.5)).abs() < 0.01) && at.y > 4.0;
        if hanging {
            let mm = Mat::PowderCoat(spec.pole);
            p.get(mm).beam(v(at.x, at.y + hh * 0.5 + 0.0, at.z - depth * 0.5), v(at.x, at.y + 0.45, 0.0), Vec3::X, 0.04, 0.04, 1.0);
        } else {
            let _ = hh;
            bracket(&mut p, spec, at, yaw, depth);
        }
    }
    if spec.twin {
        // T bracket between the twin heads.
        let y = match spec.mount {
            Mount::Column => 2.95,
            Mount::Mast { .. } => 0.0,
        };
        if y > 0.0 {
            p.get(Mat::PowderCoat(spec.pole)).beam(v(0.0, y, 0.0), v(0.42, y, 0.0), Vec3::Y, 0.05, 0.05, 1.0);
        }
    }
    if spec.button {
        let rot = Mat4::from_quat(Quat::from_rotation_y(ped_yaw));
        let mut b = P::new();
        b.get(Mat::PowderCoat(hex(0xf2c200))).aabox(v(-0.07, 1.0, 0.055), v(0.07, 1.28, 0.13), 1.0);
        label_at(&mut b, Label::Button, v(0.0, 1.14, 0.13), Vec3::Z, 0.13, 0.26);
        b.get(Mat::PowderCoat(hex(0x202020))).cylinder(v(0.0, 1.1, 0.13), v(0.0, 1.1, 0.15), 0.026, 10, true);
        p.append(&b, &rot);
    }
    p
}

/// Night glow of the lit aspects of [`signal_post`] (same arguments), in
/// its local frame.
pub fn signal_post_lights(spec: &SignalSpec, state: Aspect, ped_walk: bool, ped_yaw: f32) -> Vec<CityLight> {
    let mut out = Vec::new();
    let col = |a: Aspect| match a {
        Aspect::Red => Vec3::new(1.0, 0.04, 0.01),
        Aspect::Amber => Vec3::new(1.0, 0.35, 0.0),
        Aspect::Green => Vec3::new(0.05, 1.0, 0.35),
    };
    let idx = match state {
        Aspect::Red => 0,
        Aspect::Amber => 1,
        Aspect::Green => 2,
    };
    for (h, at, yaw) in heads(spec, ped_yaw) {
        let (_, _, offs) = lens_layout(h);
        let dir = Quat::from_rotation_y(yaw) * Vec3::Z;
        match h {
            Head::Vehicle { small } => {
                let pos = at + v(0.0, offs[idx], 0.0) + dir * 0.2;
                let (i, r) = if small { (2.0, 4.0) } else { (5.0, 6.0) };
                out.push(CityLight::spot(LightKind::Signal, pos, dir, 60.0, col(state), i, r).with_size(if small { 0.1 } else { 0.2 }));
            }
            Head::Bicycle => {
                let pos = at + v(0.0, offs[idx], 0.0) + dir * 0.1;
                out.push(CityLight::spot(LightKind::Signal, pos, dir, 60.0, col(state), 1.5, 3.0).with_size(0.1));
            }
            Head::Pedestrian => {
                let y = if ped_walk { offs[1] } else { offs[0] };
                let c = if ped_walk { Vec3::new(0.1, 1.0, 0.45) } else { Vec3::new(1.0, 0.06, 0.02) };
                out.push(CityLight::spot(LightKind::Signal, at + v(0.0, y, 0.0) + dir * 0.25, dir, 60.0, c, 2.5, 4.0).with_size(0.2));
            }
        }
    }
    out
}

/// Signal head clamped to a lamp post of radius `post_r` at `height`
/// (secondary crossings): bracket and a three-aspect head facing +Z.
pub fn head_on_lamp_post(state: Aspect, post_r: f32, height: f32) -> P {
    let mut p = P::new();
    let spec = SignalSpec::CORNER;
    let at = v(0.0, height, post_r + 0.2);
    p.append(&head(Head::Vehicle { small: false }, state, false, &spec), &Mat4::from_translation(at));
    for dy in [-0.22f32, 0.22] {
        p.get(Mat::PowderCoat(HOUSING)).beam(v(0.0, height + dy, 0.0), v(0.0, height + dy, post_r + 0.0), Vec3::Y, 0.05, 0.03, 1.0);
        p.get(Mat::Galvanized).cylinder(v(0.0, height + dy - 0.02, 0.0), v(0.0, height + dy + 0.02, 0.0), post_r + 0.01, 12, false);
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::prim_count;

    #[test]
    fn signal_variants_within_budget() {
        let specs = [
            SignalSpec::CORNER,
            SignalSpec::for_arm(5.0),
            SignalSpec { twin: true, bike: true, countdown: true, button: true, ..SignalSpec::CORNER },
            SignalSpec { mount: Mount::Mast { reach: 6.5 }, twin: true, ..SignalSpec::for_arm(6.5) },
        ];
        for s in specs {
            for a in [Aspect::Red, Aspect::Amber, Aspect::Green] {
                let p = signal_post(&s, a, a == Aspect::Red, 1.57);
                let t = prim_count(&p);
                assert!(t > 200 && t <= 5000, "{s:?}: {t} tris");
                let l = signal_post_lights(&s, a, a == Aspect::Red, 1.57);
                assert_eq!(l.len(), heads(&s, 1.57).len());
            }
        }
    }

    #[test]
    fn lit_lens_matches_state() {
        let p = signal_post(&SignalSpec::CORNER, Aspect::Green, true, 0.0);
        assert!(p.prims.contains_key(&Mat::Lens(GREEN, true)));
        assert!(!p.prims.contains_key(&Mat::Lens(RED, true)));
        assert!(p.prims.contains_key(&Mat::PedLens(true)));
    }
}
