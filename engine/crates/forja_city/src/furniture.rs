//! Street furniture: Spanish traffic lights and road signs, lamps,
//! bollards, litter bins, recycling containers, tree grates and street
//! name plates. Origin at the base, +Z is the facing direction.

use forja_math::{Vec2, Vec3};

use crate::kit::{Mat, Rgb, hex};
use crate::nightlight::CityLight;
use crate::{signals, street_waste};
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

/// Traffic light: vehicle head facing +Z at the top, pedestrian head
/// facing `ped_yaw` (radians around Y). `arm > 0` builds the avenue bent
/// mast reaching `arm` metres along +X with a repeater, a low repeater,
/// countdown and push button; otherwise the grey-green corner column with
/// an acoustic unit. See [`signals`] for every variant.
pub fn traffic_light(state: Aspect, ped_walk: bool, ped_yaw: f32, arm: f32) -> P {
    signals::signal_post(&signals::SignalSpec::for_arm(arm), state, ped_walk, ped_yaw)
}

/// Night glow of the lit lenses of [`traffic_light`] (same arguments), in
/// its local frame: small colored lights in front of each lit head.
pub fn signal_lights(state: Aspect, ped_walk: bool, ped_yaw: f32, arm: f32) -> Vec<CityLight> {
    signals::signal_post_lights(&signals::SignalSpec::for_arm(arm), state, ped_walk, ped_yaw)
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

/// Litter bin on a post: the Valencian municipal drum (see
/// [`street_waste::papelera`] for the other models).
pub fn litter_bin() -> P {
    street_waste::papelera(street_waste::BinModel::PostSingle, 0)
}

/// Kerbside recycling container: 0 yellow (packaging), 1 blue (paper),
/// 2 green igloo (glass), 3 grey (general), 4 brown (organic). See
/// [`street_waste::container_model`] for the other models.
pub fn container(kind: u8) -> P {
    street_waste::container(kind)
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
