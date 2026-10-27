//! Street furniture: Spanish traffic lights and road signs, lamps,
//! bollards, litter bins, recycling containers, tree grates and street
//! name plates. Origin at the base, +Z is the facing direction.

use forja_math::{Vec2, Vec3};

use crate::kit::{Mat, Rgb, hex};
use crate::nightlight::{CityLight, LightKind};
use crate::modules::{self, P};
use crate::texgen::{SignKind, SignShape};

fn v(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(x, y, z)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Aspect {
    Red,
    Amber,
    Green,
}

const POLE: Mat = Mat::Plastic([0x3b, 0x40, 0x42]);
const HOUSING: Mat = Mat::Plastic([0x16, 0x17, 0x18]);

/// Traffic light: vehicle head facing +Z at the top, pedestrian head
/// facing `ped_yaw` (radians around Y), push-button box at hand height.
pub fn traffic_light(state: Aspect, ped_walk: bool, ped_yaw: f32, arm: f32) -> P {
    let mut p = P::new();
    let h = 3.4;
    p.get(POLE).cylinder(v(0.0, 0.0, 0.0), v(0.0, h, 0.0), 0.06, 12, true);
    p.get(POLE).cylinder(v(0.0, 0.0, 0.0), v(0.0, 0.25, 0.0), 0.1, 12, true);
    // Optional cantilever arm over the road (+X).
    let head_at = if arm > 0.0 {
        p.get(POLE).cylinder(v(0.0, h - 0.15, 0.0), v(arm, h - 0.15, 0.0), 0.05, 10, true);
        p.get(POLE).beam(v(0.0, h - 0.9, 0.0), v(arm * 0.4, h - 0.18, 0.0), Vec3::Z, 0.04, 0.04, 1.0);
        v(arm - 0.2, h - 0.75, 0.12)
    } else {
        v(0.0, h - 0.7, 0.16)
    };
    vehicle_head(&mut p, head_at, state);
    if arm > 0.0 {
        // Repeater head on the pole too.
        vehicle_head(&mut p, v(0.0, 2.3, 0.16), state);
    }
    // Pedestrian head.
    let mut ped = P::new();
    {
        let b = ped.get(HOUSING);
        b.aabox(v(-0.16, 0.0, 0.0), v(0.16, 0.62, 0.2), 1.0);
    }
    let uv = [Vec2::new(0.0, 1.0), Vec2::new(1.0, 1.0), Vec2::new(1.0, 0.0), Vec2::ZERO];
    for (k, walk) in [(1u8, false), (0u8, true)] {
        let y = 0.04 + k as f32 * 0.29;
        let lit = walk == ped_walk;
        let m = if lit { Mat::PedLens(walk) } else { Mat::Iron };
        ped.get(m).quad_uv([v(-0.12, y, 0.205), v(0.12, y, 0.205), v(0.12, y + 0.25, 0.205), v(-0.12, y + 0.25, 0.205)], uv);
        ped.get(HOUSING).aabox(v(-0.14, y + 0.25, 0.2), v(0.14, y + 0.27, 0.32), 1.0);
    }
    let m = forja_math::Mat4::from_translation(v(0.0, 2.15, 0.0)) * forja_math::Mat4::from_rotation_y(ped_yaw) * forja_math::Mat4::from_translation(v(0.0, 0.0, 0.07));
    p.append(&ped, &m);
    // Push button ("PULSE"), yellow box.
    let b = forja_math::Mat4::from_rotation_y(ped_yaw);
    let mut btn = P::new();
    btn.get(Mat::Plastic(hex(0xf2c200))).aabox(v(-0.07, 1.0, 0.06), v(0.07, 1.25, 0.14), 1.0);
    btn.get(Mat::Plastic(hex(0x202020))).cylinder(v(0.0, 1.1, 0.14), v(0.0, 1.1, 0.155), 0.03, 10, true);
    p.append(&btn, &b);
    p
}

fn vehicle_head(p: &mut P, at: Vec3, state: Aspect) {
    p.get(HOUSING).aabox(at + v(-0.17, -0.5, -0.12), at + v(0.17, 0.5, 0.1), 1.0);
    // Black backplate with a thin white border on the front only.
    p.get(HOUSING).aabox(at + v(-0.29, -0.62, -0.135), at + v(0.29, 0.62, -0.12), 1.0);
    let border = p.get(Mat::Paint(hex(0xeeeeee)));
    for (x0, x1, y0, y1) in [(-0.29, 0.29, 0.585, 0.62), (-0.29, 0.29, -0.62, -0.585), (-0.29, -0.255, -0.62, 0.62), (0.255, 0.29, -0.62, 0.62)] {
        border.aabox(at + v(x0, y0, -0.12), at + v(x1, y1, -0.116), 1.0);
    }
    let colors = [(Aspect::Red, hex(0xff2a10), 0.32), (Aspect::Amber, hex(0xffa000), 0.0), (Aspect::Green, hex(0x20ff70), -0.32)];
    for (a, c, y) in colors {
        let lens = Mat::Lens(c, a == state);
        let ctr = at + v(0.0, y, 0.1);
        p.get(lens).cylinder(ctr, ctr + v(0.0, 0.0, 0.012), 0.1, 16, true);
        // Visor.
        let vis = p.get(HOUSING);
        vis.aabox(ctr + v(-0.12, 0.11, 0.0), ctr + v(0.12, 0.125, 0.16), 1.0);
        vis.aabox(ctr + v(-0.125, -0.05, 0.0), ctr + v(-0.11, 0.125, 0.14), 1.0);
        vis.aabox(ctr + v(0.11, -0.05, 0.0), ctr + v(0.125, 0.125, 0.14), 1.0);
    }
}

/// Night glow of the lit lenses of [`traffic_light`] (same arguments), in
/// its local frame: small colored lights in front of each lit head.
pub fn signal_lights(state: Aspect, ped_walk: bool, ped_yaw: f32, arm: f32) -> Vec<CityLight> {
    let h = 3.4;
    let (c, y) = match state {
        Aspect::Red => (Vec3::new(1.0, 0.04, 0.01), 0.32),
        Aspect::Amber => (Vec3::new(1.0, 0.35, 0.0), 0.0),
        Aspect::Green => (Vec3::new(0.05, 1.0, 0.35), -0.32),
    };
    let mut heads = vec![if arm > 0.0 { v(arm - 0.2, h - 0.75, 0.12) } else { v(0.0, h - 0.7, 0.16) }];
    if arm > 0.0 {
        heads.push(v(0.0, 2.3, 0.16));
    }
    let mut out: Vec<CityLight> = heads.into_iter().map(|at| CityLight::spot(LightKind::Signal, at + v(0.0, y, 0.2), Vec3::Z, 60.0, c, 5.0, 6.0).with_size(0.2)).collect();
    let pc = if ped_walk { Vec3::new(0.1, 1.0, 0.45) } else { Vec3::new(1.0, 0.06, 0.02) };
    let py = 2.15 + if ped_walk { 0.04 + 0.125 } else { 0.33 + 0.125 };
    let dir = forja_math::Quat::from_rotation_y(ped_yaw) * Vec3::Z;
    out.push(CityLight::spot(LightKind::Signal, v(0.0, py, 0.0) + dir * 0.45, dir, 60.0, pc, 2.5, 4.0).with_size(0.2));
    out
}

/// Road sign on a galvanized post, plate facing +Z.
pub fn sign(kind: SignKind, height: f32) -> P {
    let mut p = P::new();
    let size = 0.6;
    p.get(Mat::Galvanized).cylinder(v(0.0, 0.0, -0.05), v(0.0, height + size * 0.5, -0.05), 0.03, 10, true);
    let ctr = v(0.0, height, 0.0);
    let outline: Vec<Vec2> = match kind.shape() {
        SignShape::Circle => (0..32).map(|i| Vec2::from_angle(std::f32::consts::TAU * i as f32 / 32.0) * size * 0.5).collect(),
        SignShape::Octagon => (0..8).map(|i| Vec2::from_angle(std::f32::consts::PI / 8.0 + std::f32::consts::FRAC_PI_4 * i as f32) * size * 0.5).collect(),
        SignShape::TriangleDown => vec![Vec2::new(-0.5, 0.31) * size, Vec2::new(0.0, -0.525) * size, Vec2::new(0.5, 0.31) * size],
        SignShape::Square => vec![Vec2::new(-0.5, -0.5) * size, Vec2::new(0.5, -0.5) * size, Vec2::new(0.5, 0.5) * size, Vec2::new(-0.5, 0.5) * size],
    };
    let outline: Vec<Vec2> = outline.iter().map(|q| *q + ctr.truncate()).collect();
    // Back and rim in galvanized steel, face with the printed texture.
    p.get(Mat::Galvanized).extrude_xy(&outline, -0.03, 0.0, 1.0, |q| q);
    let face = Mat::SignFace(kind);
    let c2 = ctr.truncate();
    p.get(face).extrude_xy(&outline, 0.0, 0.004, 1.0, move |q| {
        let l = (q - c2) / size;
        Vec2::new(l.x + 0.5, 0.5 - l.y)
    });
    // Clamps.
    for y in [height - 0.15, height + 0.15] {
        p.get(Mat::Galvanized).aabox(v(-0.05, y - 0.02, -0.08), v(0.05, y + 0.02, -0.03), 1.0);
    }
    p
}

/// Cast-iron street lamp: Madrid "fernandina" (one lantern) or the
/// Valencian candelabra (four lanterns on scrolled arms).
pub fn street_lamp(candelabra: bool) -> P {
    street_lamp_lit(candelabra, hex(0xffd9a0))
}

/// Height of the bulbs of [`street_lamp`] above its base.
pub fn street_lamp_bulb(candelabra: bool) -> f32 {
    if candelabra { 4.55 } else { 3.62 }
}

/// [`street_lamp`] whose lanterns glow with `glow` at night.
pub fn street_lamp_lit(candelabra: bool, glow: Rgb) -> P {
    let mut p = P::new();
    let iron = Mat::Iron;
    let h = if candelabra { 4.2 } else { 3.3 };
    let col = [(0.24, 0.0), (0.24, 0.3), (0.2, 0.36), (0.15, 0.42), (0.15, 0.6), (0.11, 0.7), (0.1, 0.9), (0.085, 1.0), (0.075, h - 0.2), (0.11, h - 0.15), (0.11, h - 0.05), (0.07, h)];
    p.get(iron).lathe(Vec3::ZERO, &col, 12);
    if candelabra {
        for k in 0..4 {
            let a = std::f32::consts::FRAC_PI_2 * k as f32 + std::f32::consts::FRAC_PI_4;
            let d = v(a.cos(), 0.0, a.sin());
            let tip = d * 0.75 + v(0.0, h + 0.15, 0.0);
            p.get(iron).beam(v(0.0, h - 0.3, 0.0), tip, Vec3::Y.cross(d), 0.035, 0.035, 1.0);
            p.get(iron).beam(v(0.0, h + 0.1, 0.0), tip, Vec3::Y.cross(d), 0.025, 0.025, 1.0);
            modules::lantern_lit(&mut p, tip + v(0.0, 0.02, 0.0), 0.95, false, glow);
        }
        p.get(iron).lathe(v(0.0, h, 0.0), &[(0.07, 0.0), (0.05, 0.4), (0.09, 0.5), (0.0, 0.7)], 10);
    } else {
        modules::lantern_lit(&mut p, v(0.0, h, 0.0), 1.35, false, glow);
    }
    p
}

/// Tall modern road light for avenues (báculo) reaching over the road (+Z).
pub fn road_light() -> P {
    road_light_lit(hex(0xfff1dc))
}

/// Head of [`road_light`] relative to its base (the arm reaches +Z).
pub const ROAD_LIGHT_HEAD: Vec3 = Vec3::new(0.0, 9.47, 1.8);

/// [`road_light`] with its LED panel in `glow`.
pub fn road_light_lit(glow: Rgb) -> P {
    let mut p = P::new();
    let g = Mat::Galvanized;
    p.get(g).cone(v(0.0, 0.0, 0.0), v(0.0, 9.0, 0.0), 0.11, 0.06, 12, true);
    let mut prev = v(0.0, 9.0, 0.0);
    for i in 1..=8 {
        let t = i as f32 / 8.0;
        let q = v(0.0, 9.0 + (t * std::f32::consts::FRAC_PI_2).sin() * 0.6, t * 1.8);
        p.get(g).beam(prev, q, Vec3::X, 0.07, 0.07, 1.0);
        prev = q;
    }
    p.get(Mat::Plastic(hex(0x5a5e62))).aabox(prev + v(-0.18, -0.12, -0.1), prev + v(0.18, 0.02, 0.6), 1.0);
    p.get(Mat::Lamp(glow)).aabox(prev + v(-0.14, -0.13, 0.0), prev + v(0.14, -0.12, 0.5), 1.0);
    p
}

/// Madrid black bollard or Valencian green/white striped one.
pub fn bollard(valencia: bool) -> P {
    let mut p = P::new();
    if valencia {
        let (g, w) = (Mat::Plastic(hex(0x1f7a45)), Mat::Plastic(hex(0xf2f2ee)));
        for k in 0..6 {
            let m = if k % 2 == 0 { g } else { w };
            p.get(m).cylinder(v(0.0, k as f32 * 0.15, 0.0), v(0.0, (k + 1) as f32 * 0.15, 0.0), 0.075, 12, k == 0);
        }
        p.get(g).lathe(v(0.0, 0.9, 0.0), &[(0.075, 0.0), (0.06, 0.05), (0.0, 0.08)], 12);
    } else {
        let prof = [(0.09, 0.0), (0.09, 0.05), (0.07, 0.08), (0.065, 0.7), (0.08, 0.74), (0.065, 0.78), (0.07, 0.82), (0.085, 0.88), (0.07, 0.96), (0.03, 1.0), (0.0, 1.01)];
        p.get(Mat::Iron).lathe(Vec3::ZERO, &prof, 12);
    }
    p
}

/// Litter bin on a post.
pub fn litter_bin() -> P {
    let mut p = P::new();
    p.get(Mat::Galvanized).cylinder(v(0.0, 0.0, -0.25), v(0.0, 1.0, -0.25), 0.03, 8, true);
    let body = Mat::Plastic(hex(0x3d4a40));
    p.get(body).lathe(v(0.0, 0.0, 0.0), &[(0.0, 0.35), (0.18, 0.35), (0.2, 0.95), (0.21, 0.97), (0.21, 1.0), (0.16, 1.0)], 14);
    p.get(Mat::Iron).cylinder(v(0.0, 0.92, -0.06), v(0.0, 0.92, -0.25), 0.03, 6, false);
    p
}

/// Kerbside recycling container: 0 yellow (packaging), 1 blue (paper),
/// 2 green igloo (glass), 3 grey (general), 4 brown (organic).
pub fn container(kind: u8) -> P {
    let mut p = P::new();
    let colors = [hex(0xf2c200), hex(0x1565c0), hex(0x2e7d32), hex(0x5a6461), hex(0x6d4c2f)];
    let m = Mat::Plastic(colors[kind as usize % 5]);
    if kind % 5 == 2 {
        let prof = [(0.0, 0.0), (0.75, 0.0), (0.78, 0.6), (0.7, 1.1), (0.5, 1.42), (0.22, 1.55), (0.0, 1.57)];
        p.get(m).lathe(Vec3::ZERO, &prof, 16);
        p.get(Mat::Iron).cylinder(v(0.0, 1.55, 0.0), v(0.0, 1.75, 0.0), 0.05, 8, true);
        return p;
    }
    let (w, h, d) = (1.7, 1.25, 1.05);
    p.get(m).aabox(v(-w * 0.5, 0.18, -d * 0.5), v(w * 0.5, h, d * 0.5), 1.0);
    // Sloped lid.
    let lid = p.get(m);
    lid.quad(v(-w * 0.5 - 0.02, h, d * 0.5 + 0.03), v(w * 0.5 + 0.02, h, d * 0.5 + 0.03), v(w * 0.5 + 0.02, h + 0.18, -d * 0.5 - 0.02), v(-w * 0.5 - 0.02, h + 0.18, -d * 0.5 - 0.02), 1.0);
    let dark = Mat::Plastic(hex(0x1a1a1a));
    for (x, z) in [(-0.7, -0.4), (0.7, -0.4), (-0.7, 0.4), (0.7, 0.4)] {
        p.get(dark).cylinder(v(x - 0.04, 0.09, z), v(x + 0.04, 0.09, z), 0.09, 10, true);
    }
    p.get(dark).aabox(v(-0.5, 0.8, d * 0.5), v(0.5, 0.86, d * 0.5 + 0.04), 1.0);
    p
}

/// Square cast-iron tree grate (alcorque) with soil.
pub fn tree_grate() -> P {
    let mut p = P::new();
    p.get(Mat::Soil).aabox(v(-0.6, -0.02, -0.6), v(0.6, 0.0, 0.6), 1.0);
    let g = p.get(Mat::Iron);
    for k in 0..7 {
        let t = -0.6 + 1.2 * k as f32 / 6.0;
        g.aabox(v(t - 0.02, 0.0, -0.6), v(t + 0.02, 0.02, 0.6), 1.0);
    }
    g.aabox(v(-0.62, 0.0, -0.62), v(0.62, 0.025, -0.58), 1.0);
    g.aabox(v(-0.62, 0.0, 0.58), v(0.62, 0.025, 0.62), 1.0);
    p
}

/// Street name plate for a wall (faces +Z).
pub fn street_plate(name: u8) -> P {
    let mut p = P::new();
    let (w, h) = (0.62, 0.23);
    p.get(Mat::Moulding([235, 232, 222])).aabox(v(-w * 0.5 - 0.02, -0.02, 0.0), v(w * 0.5 + 0.02, h + 0.02, 0.02), 1.0);
    p.get(Mat::Plate(name)).quad_uv([v(-w * 0.5, 0.0, 0.025), v(w * 0.5, 0.0, 0.025), v(w * 0.5, h, 0.025), v(-w * 0.5, h, 0.025)], [Vec2::new(0.0, 1.0), Vec2::new(1.0, 1.0), Vec2::new(1.0, 0.0), Vec2::ZERO]);
    p
}

// ------------------------------------------------------------ bars and shops

/// Bar terrace set: round aluminium table with chairs around it and,
/// for odd variants, a square parasol. Origin at the table foot.
pub fn terrace_set(variant: u8) -> P {
    let mut p = P::new();
    let alu = Mat::Galvanized;
    p.get(alu).lathe(Vec3::ZERO, &[(0.0, 0.0), (0.24, 0.0), (0.22, 0.03), (0.04, 0.05), (0.03, 0.7), (0.06, 0.72), (0.0, 0.72)], 12);
    let top = Mat::Plastic([hex(0xe8e4da), hex(0x2b2b2b), hex(0xb0b4b6), hex(0xe8e4da)][variant as usize % 4]);
    p.get(top).cylinder(v(0.0, 0.72, 0.0), v(0.0, 0.75, 0.0), 0.34, 18, true);
    let seat = [hex(0x2f4a35), hex(0x1d1d1d), hex(0x7a4a2a), hex(0xd8d4c8)][(variant as usize / 2) % 4];
    let chairs = 2 + (variant as usize % 3);
    for k in 0..chairs {
        let a = std::f32::consts::TAU * k as f32 / chairs as f32 + 0.4;
        let d = v(a.cos(), 0.0, a.sin());
        let m = forja_math::Mat4::from_translation(d * 0.62) * forja_math::Mat4::from_rotation_y(-a - std::f32::consts::FRAC_PI_2 + 0.15 * (k as f32 - 1.0));
        p.append(&chair(seat), &m);
    }
    if variant % 2 == 1 {
        parasol(&mut p, [hex(0xf1ead8), hex(0xb3261e), hex(0x2e5d8a)][variant as usize % 3]);
    }
    p
}

/// Stacking terrace chair: aluminium tubes, coloured slatted seat and back.
/// Faces +Z (the sitter looks along +Z).
fn chair(seat: [u8; 3]) -> P {
    let mut p = P::new();
    let alu = Mat::Galvanized;
    let (hw, d) = (0.21, 0.2);
    {
        let g = p.get(alu);
        for (x, z) in [(-hw, -d), (hw, -d), (-hw, d), (hw, d)] {
            g.beam(v(x, 0.0, z), v(x, 0.45, z), Vec3::X, 0.022, 0.022, 1.0);
        }
        for x in [-hw, hw] {
            g.beam(v(x, 0.45, -d), v(x, 0.88, -d - 0.06), Vec3::X, 0.022, 0.022, 1.0);
            g.beam(v(x, 0.66, -d), v(x, 0.66, d), Vec3::Y, 0.02, 0.02, 1.0);
        }
    }
    let s = p.get(Mat::Plastic(seat));
    for k in 0..4 {
        let z = -d + 0.03 + k as f32 * 0.11;
        s.aabox(v(-hw, 0.44, z), v(hw, 0.47, z + 0.08), 1.0);
    }
    for k in 0..3 {
        let y = 0.58 + k as f32 * 0.1;
        let z = -d - 0.06 * (y - 0.45) / 0.43;
        s.aabox(v(-hw, y, z - 0.03), v(hw, y + 0.07, z), 1.0);
    }
    p
}

fn parasol(p: &mut P, color: [u8; 3]) {
    p.get(Mat::Galvanized).cylinder(v(0.0, 0.0, 0.0), v(0.0, 2.45, 0.0), 0.025, 8, true);
    let half = 0.95;
    let (h0, h1) = (2.05, 2.4);
    let c = v(0.0, h1, 0.0);
    let corners = [v(-half, h0, -half), v(half, h0, -half), v(half, h0, half), v(-half, h0, half)];
    let m = Mat::Plastic(color);
    let prim = p.get(m);
    for i in 0..4 {
        let (a, b) = (corners[i], corners[(i + 1) % 4]);
        // Both sides of the canopy, and a short valance.
        prim.tri(a, c, b, [Vec2::ZERO, Vec2::new(0.5, 1.0), Vec2::X]);
        prim.tri(a, b, c, [Vec2::ZERO, Vec2::X, Vec2::new(0.5, 1.0)]);
        let dn = Vec3::Y * -0.16;
        prim.quad(b, a, a + dn, b + dn, 1.0);
        prim.quad(a, b, b + dn, a + dn, 1.0);
    }
}

/// Wine barrel used as a standing table outside Madrid bars.
pub fn barrel() -> P {
    let mut p = P::new();
    let prof = [(0.0, 0.0), (0.27, 0.0), (0.31, 0.25), (0.33, 0.5), (0.31, 0.75), (0.27, 1.0), (0.0, 1.0)];
    p.get(Mat::Wood(hex(0x7a5232))).lathe(Vec3::ZERO, &prof, 16);
    for y in [0.08, 0.3, 0.7, 0.92] {
        let r = 0.27 + 0.06 * (1.0 - ((y - 0.5) / 0.5f32).powi(2)) + 0.006;
        p.get(Mat::Iron).lathe(Vec3::ZERO, &[(r, y - 0.025), (r, y + 0.025)], 16);
    }
    p
}

/// Green LED pharmacy cross on a wall arm, perpendicular to the facade
/// (+Z out of the wall), double-sided.
pub fn pharmacy_cross() -> P {
    let mut p = P::new();
    let g = Mat::Galvanized;
    p.get(g).aabox(v(-0.06, -0.15, 0.0), v(0.06, 0.15, 0.03), 1.0);
    p.get(g).beam(v(0.0, 0.0, 0.0), v(0.0, 0.0, 0.2), Vec3::Y, 0.05, 0.05, 1.0);
    let (c, s, t) = (v(0.0, 0.0, 0.52), 0.32, 0.11);
    p.get(Mat::Plastic(hex(0x202020))).aabox(c + v(-0.03, -s - 0.02, -s - 0.02), c + v(0.03, s + 0.02, s + 0.02), 1.0);
    let lit = p.get(Mat::Lamp(hex(0x20d860)));
    for side in [-1.0f32, 1.0] {
        let x = side * 0.035;
        lit.aabox(c + v(x - 0.005, -t, -s), c + v(x + 0.005, t, s), 1.0);
        lit.aabox(c + v(x - 0.005, -s, -t), c + v(x + 0.005, s, t), 1.0);
    }
    p
}

/// Projecting blade sign (banderola) carrying the centre of a shop-sign
/// atlas row, double-sided, perpendicular to the facade.
pub fn blade_sign(row: u8) -> P {
    blade_sign_lit(row, false)
}

/// [`blade_sign`] with its faces lit (night).
pub fn blade_sign_lit(row: u8, lit: bool) -> P {
    let mut p = P::new();
    let g = Mat::Iron;
    p.get(g).beam(v(0.0, 0.36, 0.0), v(0.0, 0.36, 1.05), Vec3::Y, 0.03, 0.03, 1.0);
    p.get(g).beam(v(0.0, -0.05, 0.0), v(0.0, 0.34, 0.35), Vec3::X, 0.025, 0.025, 1.0);
    let (z0, z1, y0, y1) = (0.15, 1.0, 0.0, 0.3);
    p.get(Mat::Plastic(hex(0x1a1a1a))).aabox(v(-0.035, y0 - 0.02, z0 - 0.02), v(0.035, y1 + 0.02, z1 + 0.02), 1.0);
    let (r0, r1) = (row as f32 / 16.0, (row as f32 + 1.0) / 16.0);
    let (ua, ub) = (0.34, 0.66);
    let face = p.get(if lit { Mat::ShopSignLit } else { Mat::ShopSign });
    face.quad_uv([v(0.04, y0, z1), v(0.04, y0, z0), v(0.04, y1, z0), v(0.04, y1, z1)], [Vec2::new(ua, r1), Vec2::new(ub, r1), Vec2::new(ub, r0), Vec2::new(ua, r0)]);
    face.quad_uv([v(-0.04, y0, z0), v(-0.04, y0, z1), v(-0.04, y1, z1), v(-0.04, y1, z0)], [Vec2::new(ua, r1), Vec2::new(ub, r1), Vec2::new(ub, r0), Vec2::new(ua, r0)]);
    p
}

/// Stone fountain with two basins.
pub fn fountain(radius: f32) -> P {
    let mut p = P::new();
    let stone = Mat::Moulding(hex(0xd8cdb8));
    let r = radius;
    p.get(stone).lathe(Vec3::ZERO, &[(r + 0.3, 0.0), (r + 0.3, 0.45), (r + 0.1, 0.55), (r, 0.55), (r, 0.2)], 48);
    p.get(Mat::Water).polygon(&(0..48).map(|i| Vec2::from_angle(std::f32::consts::TAU * i as f32 / 48.0) * r).collect::<Vec<_>>(), 0.4, true, 4.0);
    p.get(stone).lathe(Vec3::ZERO, &[(0.0, 0.4), (0.6, 0.4), (0.45, 0.6), (0.35, 1.6), (0.4, 1.7), (1.6, 1.8), (1.7, 2.0), (1.5, 2.05), (0.3, 2.1), (0.25, 2.8), (0.45, 2.9), (0.2, 3.2), (0.0, 3.3)], 32);
    p.get(Mat::Water).polygon(&(0..32).map(|i| Vec2::from_angle(std::f32::consts::TAU * i as f32 / 32.0) * 1.5).collect::<Vec<_>>(), 1.98, true, 4.0);
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::prim_count;

    #[test]
    fn furniture_builds() {
        for s in [Aspect::Red, Aspect::Amber, Aspect::Green] {
            let p = traffic_light(s, s == Aspect::Red, 1.57, if s == Aspect::Green { 4.0 } else { 0.0 });
            assert!(prim_count(&p) > 100);
        }
        for k in SignKind::ALL {
            let p = sign(k, 2.3);
            assert!(p.prims.contains_key(&Mat::SignFace(k)));
        }
        for m in [street_lamp(true), street_lamp(false), road_light(), bollard(true), bollard(false), litter_bin(), container(0), container(2), tree_grate(), street_plate(1), fountain(5.0), terrace_set(0), terrace_set(3), barrel(), pharmacy_cross(), blade_sign(4)] {
            assert!(prim_count(&m) > 10);
        }
    }

    #[test]
    fn sign_face_uvs_cover_the_texture() {
        let p = sign(SignKind::Stop, 2.3);
        let f = &p.prims[&Mat::SignFace(SignKind::Stop)];
        let (mut lo, mut hi) = (Vec2::splat(9.0), Vec2::splat(-9.0));
        for uv in &f.uvs {
            lo = lo.min(*uv);
            hi = hi.max(*uv);
        }
        assert!(lo.x < 0.05 && hi.x > 0.95 && lo.y < 0.05 && hi.y > 0.95, "{lo} {hi}");
    }
}

// ------------------------------------------------------------ plazas and alleys

/// Dense low planting with red and white flowers in a disc of radius `r`
/// around `c`, standing on `y`.
pub fn flower_bed(p: &mut P, c: Vec2, r: f32, y: f32, seed: u64) {
    let mut rng = crate::geom::Rng::new(seed);
    let n = (r * r * 5.0) as i32;
    {
        let leaves = p.get(Mat::Leaves);
        for _ in 0..n {
            let q = c + Vec2::from_angle(rng.range(0.0, std::f32::consts::TAU)) * r * rng.f().sqrt();
            let s = rng.range(0.14, 0.24);
            leaves.lathe(v(q.x, y, q.y), &[(0.0, 0.0), (s, s * 0.4), (s * 0.8, s * 1.1), (0.0, s * 1.4)], 5);
        }
    }
    let reds = p.get(Mat::Flowers(hex(0xd31f2a)));
    for _ in 0..n * 3 {
        let q = c + Vec2::from_angle(rng.range(0.0, std::f32::consts::TAU)) * r * rng.f().sqrt();
        let h = y + rng.range(0.2, 0.34);
        reds.aabox(v(q.x - 0.035, h, q.y - 0.035), v(q.x + 0.035, h + 0.05, q.y + 0.035), 1.0);
    }
}

/// Formal garden parterre of half size `s` with a statue on a granite
/// plinth (Madrid squares). Origin at the centre on the paving.
pub fn parterre(s: f32) -> P {
    let mut p = P::new();
    p.get(Mat::Granite).aabox(v(-s - 0.3, 0.0, -s - 0.3), v(s + 0.3, 0.3, s + 0.3), 1.5);
    p.get(Mat::Grass).polygon(&[Vec2::new(-s, -s), Vec2::new(s, -s), Vec2::new(s, s), Vec2::new(-s, s)], 0.31, true, 3.0);
    for q in 0..4u64 {
        let c = Vec2::new(if q & 1 == 0 { -1.0 } else { 1.0 }, if q & 2 == 0 { -1.0 } else { 1.0 }) * s * 0.5;
        flower_bed(&mut p, c, (s * 0.25).min(2.2), 0.31, 11 + q);
    }
    p.get(Mat::Granite).aabox(v(-1.0, 0.3, -1.0), v(1.0, 2.45, 1.0), 1.5);
    p.get(Mat::Galvanized).lathe(v(0.0, 2.45, 0.0), &[(0.0, 0.0), (0.55, 0.0), (0.35, 0.4), (0.3, 1.4), (0.42, 1.7), (0.25, 2.1), (0.2, 2.4), (0.0, 2.5)], 12);
    p
}

/// A cable strung across an alley between two facades, `span` metres
/// along X, origin at its middle: a hanging lantern, or washing on a line.
pub fn catenary(span: f32, laundry: bool, variant: u8) -> P {
    let mut p = P::new();
    let sag = (span * 0.06).clamp(0.08, 0.3);
    let y = |x: f32| -sag * (1.0 - (2.0 * x / span).powi(2));
    let n = 8;
    for i in 0..n {
        let (x0, x1) = (-span * 0.5 + span * i as f32 / n as f32, -span * 0.5 + span * (i + 1) as f32 / n as f32);
        p.get(Mat::Iron).beam(v(x0, y(x0), 0.0), v(x1, y(x1), 0.0), Vec3::Z, 0.012, 0.012, 1.0);
    }
    for x in [-span * 0.5, span * 0.5] {
        p.get(Mat::Iron).aabox(v(x - 0.04, -0.06, -0.04), v(x + 0.04, 0.06, 0.04), 1.0);
    }
    if !laundry {
        p.get(Mat::Iron).beam(v(0.0, y(0.0), 0.0), v(0.0, y(0.0) - 0.35, 0.0), Vec3::X, 0.015, 0.015, 1.0);
        modules::lantern(&mut p, v(0.0, y(0.0) - 0.9, 0.0), 0.9, true);
        return p;
    }
    // Shirts, sheets and towels in Mediterranean colours.
    let colours = [hex(0xf4f1ea), hex(0x3b6fb6), hex(0xd8553a), hex(0xf2c94c), hex(0x7bb661), hex(0xe9a7b8), hex(0xffffff)];
    let mut rng = crate::geom::Rng::new(variant as u64 * 7 + 3);
    let mut x = -span * 0.5 + 0.4;
    while x < span * 0.5 - 0.6 {
        let w = rng.range(0.35, 0.9);
        let h = rng.range(0.4, 0.8);
        let c = colours[rng.below(colours.len())];
        let (a, b) = (x, (x + w).min(span * 0.5 - 0.3));
        let (ya, yb) = (y(a), y(b));
        let m = Mat::Plastic(c);
        let cloth = p.get(m);
        cloth.quad(v(a, ya - h, 0.0), v(b, yb - h, 0.0), v(b, yb, 0.0), v(a, ya, 0.0), 1.0);
        cloth.quad(v(b, yb - h, 0.0), v(a, ya - h, 0.0), v(a, ya, 0.0), v(b, yb, 0.0), 1.0);
        x = b + rng.range(0.1, 0.35);
    }
    p
}
