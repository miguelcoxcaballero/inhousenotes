//! Valencian waste furniture modelled from photographs (see
//! `data/furniture_refs/`): the kerbside containers of the municipal
//! service in their three generations, the Ecovidrio-style glass igloo,
//! the underground ("soterrats") mouths of the old town and the litter
//! bins. Origin at the base centre, +Z is the front (the side people use,
//! facing the pavement), +X runs along the kerb.
//!
//! Real dimensions (mm) are noted at each model. Bodies are lofts of
//! rounded rectangles in moulded HDPE ([`Mat::Hdpe`]) with labels from the
//! sticker atlas ([`Label`]).

use forja_math::{Mat4, Vec2, Vec3};

use crate::kit::{Mat, hex};
use crate::modules::P;
use crate::streettex::{BODY_RGB, ENVASES, FRACTION_RGB, Label, ORGANICO, PAPEL, RESTO, VIDRIO};

fn v(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(x, y, z)
}

/// Rounded rectangle ring at height `y`: half sizes `hx`, `hz`, corner
/// radius `r`, `seg` segments per corner, counter-clockwise seen from
/// above, starting at the front-right corner arc. Centred at `(0, cz)`.
pub fn rounded_ring(hx: f32, hz: f32, r: f32, y: f32, cz: f32, seg: u32) -> Vec<Vec3> {
    let r = r.min(hx).min(hz).max(0.0);
    let mut out = Vec::with_capacity(4 * (seg as usize + 1));
    // Corner centres, CCW seen from +Y means (x, -z) CCW: go +x+z, +x-z,
    // -x-z, -x+z.
    let corners = [(hx - r, hz - r, 0.0f32), (hx - r, -(hz - r), -std::f32::consts::FRAC_PI_2), (-(hx - r), -(hz - r), -std::f32::consts::PI), (-(hx - r), hz - r, std::f32::consts::FRAC_PI_2)];
    for (cx, czz, a0) in corners {
        for k in 0..=seg {
            let a = a0 + std::f32::consts::FRAC_PI_2 - k as f32 / seg as f32 * std::f32::consts::FRAC_PI_2;
            out.push(v(cx + r * a.cos(), y, cz + czz + r * a.sin()));
        }
    }
    out
}

/// Smooth loft through rings of equal length (outward normals), with
/// world-scale UVs: u along the perimeter, v down the height. `cap_top`
/// and `cap_bottom` close the ends with fans.
pub fn loft(p: &mut crate::geom::Prim, rings: &[Vec<Vec3>], cap_bottom: bool, cap_top: bool, scale: f32) {
    let m = rings.len();
    if m < 2 {
        return;
    }
    let n = rings[0].len();
    let centre = |r: &Vec<Vec3>| r.iter().copied().sum::<Vec3>() / r.len() as f32;
    // Perimeter distance of the first ring for u.
    let mut us = vec![0.0f32; n + 1];
    for i in 0..n {
        us[i + 1] = us[i] + (rings[0][(i + 1) % n] - rings[0][i]).length();
    }
    let mut vs = vec![0.0f32; m];
    for k in 1..m {
        vs[k] = vs[k - 1] + (centre(&rings[k]) - centre(&rings[k - 1])).length().max((rings[k][0] - rings[k - 1][0]).length());
    }
    let base = p.positions.len() as u32;
    for k in 0..m {
        let c = centre(&rings[k]);
        for i in 0..=n {
            let ii = i % n;
            let q = rings[k][ii];
            let tu = rings[k][(ii + 1) % n] - rings[k][(ii + n - 1) % n];
            let tv = rings[(k + 1).min(m - 1)][ii] - rings[k.saturating_sub(1)][ii];
            let mut nn = tv.cross(tu).normalize_or(Vec3::Y);
            if nn.dot(q - c + Vec3::Y * 1e-4 * (k as f32 - m as f32 * 0.5)) < 0.0 && (q - c).length() > 1e-4 {
                nn = -nn;
            }
            p.vertex(q, nn, Vec2::new(us[i], -(vs[k] + q.y * 0.0)) / scale);
        }
    }
    let row = n as u32 + 1;
    for k in 0..m as u32 - 1 {
        for i in 0..n as u32 {
            let a = base + k * row + i;
            let b = a + row;
            // Rings run CCW seen from above: (a, a+1, b+1) faces out.
            p.indices.extend([a, a + 1, b + 1, a, b + 1, b]);
        }
    }
    for (ring, up) in [(&rings[0], false), (&rings[m - 1], true)] {
        if (up && !cap_top) || (!up && !cap_bottom) {
            continue;
        }
        let c = centre(ring);
        let nn = if up { Vec3::Y } else { Vec3::NEG_Y };
        let ci = p.vertex(c, nn, Vec2::new(c.x, c.z) / scale);
        let first = p.positions.len() as u32;
        for q in ring {
            p.vertex(*q, nn, Vec2::new(q.x, q.z) / scale);
        }
        for i in 0..n as u32 {
            let j = (i + 1) % n as u32;
            if up {
                p.indices.extend([ci, first + i, first + j]);
            } else {
                p.indices.extend([ci, first + j, first + i]);
            }
        }
    }
}

/// Atlas quad: `corners` bottom-left, bottom-right, top-right, top-left
/// (front view), mapped to `label`.
pub fn label(p: &mut P, l: Label, corners: [Vec3; 4]) {
    let (a, b) = l.uv();
    let uv = if l == Label::Chevron {
        // Stored lying down: u runs up the strip.
        [Vec2::new(a.x, a.y), Vec2::new(a.x, b.y), Vec2::new(b.x, b.y), Vec2::new(b.x, a.y)]
    } else {
        [Vec2::new(a.x, b.y), Vec2::new(b.x, b.y), Vec2::new(b.x, a.y), Vec2::new(a.x, a.y)]
    };
    p.get(Mat::StreetLabel).quad_uv(corners, uv);
}

/// Label on a plane facing `n` centred at `c`, `w` wide and `h` tall,
/// upright (its up is the plane's projection of +Y).
pub fn label_at(p: &mut P, l: Label, c: Vec3, n: Vec3, w: f32, h: f32) {
    let n = n.normalize();
    let right = Vec3::Y.cross(n).normalize_or(Vec3::X);
    let up = n.cross(right);
    let c = c + n * 0.004;
    let (hr, hu) = (right * w * 0.5, up * h * 0.5);
    label(p, l, [c - hr - hu, c + hr - hu, c + hr + hu, c - hr + hu]);
}

fn fraction_mat(f: u8, wear: u8) -> Mat {
    let c = FRACTION_RGB[f as usize % 5];
    // Sun fading: older lids drift lighter and greyer.
    let k = (wear % 4) as f32 * 0.06;
    let fade = |x: u32| -> u8 {
        let x = x as f32;
        (x + (200.0 - x) * k) as u8
    };
    Mat::Hdpe([fade((c >> 16) & 255), fade((c >> 8) & 255), fade(c & 255)])
}

fn body_mat(wear: u8) -> Mat {
    let g = [hex(BODY_RGB), hex(0x5c6062), hex(0x54585a), hex(0x64686a)][wear as usize % 4];
    Mat::Hdpe(g)
}

/// Swivel castor: fork, wheel of radius `r` with a rubber tyre.
fn castor(p: &mut P, at: Vec3, r: f32) {
    let g = Mat::Galvanized;
    p.get(g).aabox(at + v(-0.05, 2.0 * r + 0.02, -0.05), at + v(0.05, 2.0 * r + 0.045, 0.05), 1.0);
    p.get(g).aabox(at + v(-0.035, r, -0.012), at + v(0.035, 2.0 * r + 0.02, 0.012), 1.0);
    p.get(Mat::Rubber).cylinder(at + v(-0.025, r, 0.0), at + v(0.025, r, 0.0), r, 10, true);
    p.get(Mat::Plastic(hex(0x3a3a3a))).cylinder(at + v(-0.03, r, 0.0), at + v(0.03, r, 0.0), r * 0.45, 8, true);
}

// ------------------------------------------------------------- containers

/// Container generations seen on Valencian streets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ContainerModel {
    /// The 3.2 m³ kerbside container of the 2000s–2010s fleet, still the
    /// most common (Iecla, Blanqueria, l'Estació photos): grey body on four
    /// castors, coloured two-part domed lid, galvanized lifting frames on
    /// both ends with red/white chevrons, foot pedal.
    Kerbside,
    /// The side-loading container of the current contract (2023–): taller
    /// grey body without wheels, coloured upper shell with the mouth of
    /// the fraction, city emblem and "thank you" band, side lifting bracket.
    SideLoader,
    /// 1100 l rear-loading bin of the narrow old-town streets (Sabaters,
    /// Altar de Sant Vicent): grey body, coloured flat lid, four castors.
    Bin1100,
}

/// Kerbside container of `fraction` (0 yellow packaging, 1 blue paper,
/// 2 green glass igloo, 3 grey general, 4 brown organic): the current
/// Valencian default, a [`ContainerModel::SideLoader`] for paper and
/// organic and a [`ContainerModel::Kerbside`] for the rest, and the igloo
/// for glass.
pub fn container(kind: u8) -> P {
    match kind % 5 {
        VIDRIO => glass_igloo(0),
        PAPEL | ORGANICO => container_model(ContainerModel::SideLoader, kind % 5, 0),
        f => container_model(ContainerModel::Kerbside, f, 0),
    }
}

/// Container of a given model and fraction; `wear` (0..) picks body tone,
/// lid fading, stickers and graffiti deterministically. Glass always
/// returns the igloo.
pub fn container_model(model: ContainerModel, fraction: u8, wear: u8) -> P {
    let f = fraction % 5;
    if f == VIDRIO {
        return glass_igloo(wear);
    }
    match model {
        ContainerModel::Kerbside => kerbside(f, wear),
        ContainerModel::SideLoader => side_loader(f, wear),
        ContainerModel::Bin1100 => bin_1100(f, wear),
    }
}

/// Footprint (length along the kerb, depth) of a container model, for
/// placement.
pub fn container_size(model: ContainerModel, fraction: u8) -> Vec2 {
    if fraction % 5 == VIDRIO {
        return Vec2::new(1.55, 1.55);
    }
    match model {
        ContainerModel::Kerbside => Vec2::new(1.95, 1.3),
        ContainerModel::SideLoader => Vec2::new(1.8, 1.45),
        ContainerModel::Bin1100 => Vec2::new(1.4, 1.1),
    }
}

/// 3.2 m³ kerbside container, 1850 long × 1250 deep, body top at 1300,
/// lid crown at 1560 (Iecla / Blanqueria photos).
fn kerbside(f: u8, wear: u8) -> P {
    let mut p = P::new();
    let body = body_mat(wear);
    let lid = fraction_mat(f, wear);
    let (hx, hz) = (0.925, 0.625);
    let y0 = 0.22;
    let ytop = 1.3;
    // Body: narrower at the bottom (nesting taper), lips at the top.
    let rings = vec![
        rounded_ring(hx - 0.1, hz - 0.12, 0.1, y0, 0.0, 3),
        rounded_ring(hx - 0.06, hz - 0.07, 0.12, y0 + 0.35, 0.0, 3),
        rounded_ring(hx - 0.02, hz - 0.02, 0.12, ytop - 0.1, 0.0, 3),
        rounded_ring(hx, hz, 0.12, ytop - 0.08, 0.0, 3),
        rounded_ring(hx, hz, 0.12, ytop, 0.0, 3),
    ];
    crate::street_waste::loft(p.get(body), &rings, true, false, 1.0);
    // Moulded reinforcing ribs across the front and back.
    for z in [hz - 0.03, -hz + 0.03] {
        let s = z.signum();
        for x in [-0.45f32, 0.0, 0.45] {
            p.get(body).obox(v(x, 0.72, z + s * 0.002), [Vec3::X, Vec3::Y, Vec3::Z], v(0.025, 0.42, 0.018), 1.0);
        }
    }
    // Two-part domed lid: a flat rear part hinged on the back, the front
    // part domed, both overhanging the body by 3 cm.
    // Barrel dome: curved front to back, nearly straight along the length.
    let lid_rings = vec![
        rounded_ring(hx + 0.03, hz + 0.03, 0.13, ytop, 0.0, 3),
        rounded_ring(hx + 0.03, hz + 0.03, 0.13, ytop + 0.07, 0.0, 3),
        rounded_ring(hx + 0.01, hz - 0.04, 0.14, ytop + 0.17, -0.01, 3),
        rounded_ring(hx - 0.01, hz - 0.17, 0.14, ytop + 0.26, -0.02, 3),
        rounded_ring(hx - 0.03, hz - 0.33, 0.12, ytop + 0.31, -0.03, 3),
        rounded_ring(hx - 0.05, hz - 0.5, 0.06, ytop + 0.33, -0.04, 3),
    ];
    crate::street_waste::loft(p.get(lid), &lid_rings, false, true, 1.0);
    // Seam between the two lid halves and the handle on the front.
    p.get(Mat::Plastic(hex(0x2a2a2a))).aabox(v(-hx + 0.04, ytop + 0.28, -0.22), v(hx - 0.04, ytop + 0.315, -0.2), 1.0);
    p.get(lid).aabox(v(-0.3, ytop + 0.02, hz + 0.03), v(0.3, ytop + 0.07, hz + 0.08), 1.0);
    // Galvanized lifting frames on both ends: two posts and a bar, with
    // the trunnion the truck's arms grab.
    let g = Mat::Galvanized;
    for s in [-1.0f32, 1.0] {
        let x = s * (hx + 0.02);
        for z in [-0.42f32, 0.42] {
            p.get(g).beam(v(x - s * 0.06, 0.25, z), v(x + s * 0.005, ytop - 0.05, z), Vec3::Z, 0.05, 0.03, 1.0);
        }
        p.get(g).beam(v(x, 0.95, -0.45), v(x, 0.95, 0.45), Vec3::Y, 0.07, 0.04, 1.0);
        p.get(g).cylinder(v(x, 0.95, 0.0), v(x + s * 0.09, 0.95, 0.0), 0.04, 8, true);
        // Red/white chevrons on the four vertical corners, front and back
        // faces (Iecla, "Uves recicladas" photos).
        for z in [-1.0f32, 1.0] {
            label_at(&mut p, Label::Chevron, v(s * (hx - 0.13), 0.78, z * (hz - 0.035)), v(0.0, 0.05, z), 0.13, 0.5);
        }
    }
    // Lower tube along the front (pedal bar) and foot pedal.
    p.get(g).cylinder(v(-hx + 0.15, 0.32, hz - 0.08), v(hx - 0.15, 0.32, hz - 0.08), 0.022, 8, true);
    p.get(g).beam(v(-0.12, 0.3, hz - 0.08), v(-0.12, 0.26, hz + 0.18), Vec3::X, 0.05, 0.02, 1.0);
    p.get(g).aabox(v(-0.2, 0.245, hz + 0.12), v(-0.04, 0.27, hz + 0.24), 1.0);
    // Lid push rods from the pedal up the left front corner.
    p.get(g).cylinder(v(-hx + 0.04, 0.32, hz - 0.02), v(-hx + 0.04, ytop + 0.02, hz - 0.02), 0.015, 6, true);
    // Castors under the base.
    for (x, z) in [(-0.62f32, -0.35f32), (0.62, -0.35), (-0.62, 0.35), (0.62, 0.35)] {
        castor(&mut p, v(x, 0.0, z), 0.09);
    }
    // Stickers on the front: fraction label left, emblem centre-right,
    // older units carry the white information sticker.
    let front_z = hz - 0.015;
    let tilt = Vec3::new(0.0, 0.05, 1.0);
    label_at(&mut p, Label::Fraction(f), v(-0.45, 0.88, front_z), tilt, 0.3, 0.4);
    label_at(&mut p, Label::Info, v(0.25, 0.85, front_z), tilt, 0.36, 0.36);
    label_at(&mut p, Label::Band(f), v(0.0, ytop - 0.04, hz + 0.004), Vec3::Z, 0.75, 0.07);
    wear_decals(&mut p, wear, front_z, hz, 0.75);
    p
}

/// Graffiti tags and fly-posted stickers on worn units.
fn wear_decals(p: &mut P, wear: u8, front_z: f32, _hz: f32, y: f32) {
    let tilt = Vec3::new(0.0, 0.05, 1.0);
    match wear % 4 {
        1 => label_at(p, Label::Sticker(wear / 4), v(0.62, y, front_z + 0.002), tilt, 0.08, 0.12),
        2 => label_at(p, Label::Tag(wear / 4), v(0.1, y - 0.25, front_z + 0.002), tilt, 0.8, 0.45),
        3 => {
            label_at(p, Label::Tag(wear / 4 + 1), v(-0.2, y - 0.3, front_z + 0.002), tilt, 0.6, 0.34);
            label_at(p, Label::Sticker(wear / 4 + 2), v(0.65, y + 0.1, front_z + 0.003), tilt, 0.08, 0.12);
        }
        _ => {}
    }
}

/// Side-loading container of the current fleet, 1800 long × 1450 deep ×
/// 1700 high (3.2 m³ class): grey body sitting on moulded feet with a
/// central notch at the base, coloured upper shell 0.45 m tall carrying
/// the mouth (slot with rubber flaps for paper, round holes for packaging,
/// pedal lid for organic and general waste).
fn side_loader(f: u8, wear: u8) -> P {
    let mut p = P::new();
    let body = body_mat(wear);
    let top = fraction_mat(f, wear);
    let (hx, hz) = (0.9, 0.72);
    let ysplit = 1.18;
    let rings = vec![
        rounded_ring(hx - 0.08, hz - 0.1, 0.12, 0.0, 0.0, 3),
        rounded_ring(hx - 0.06, hz - 0.07, 0.14, 0.12, 0.0, 3),
        rounded_ring(hx - 0.02, hz - 0.02, 0.16, ysplit - 0.06, 0.0, 3),
        rounded_ring(hx, hz, 0.16, ysplit, 0.0, 3),
    ];
    loft(p.get(body), &rings, true, false, 1.0);
    // Base notch (forklift / drainage recess) front and back: a dark inset.
    for s in [-1.0f32, 1.0] {
        p.get(Mat::Plastic(hex(0x2b2d2e))).aabox(v(-0.12, 0.0, s * (hz - 0.1) - 0.01), v(0.12, 0.16, s * (hz - 0.1) + 0.01), 1.0);
    }
    // Upper coloured shell, slightly proud, with a rounded crown.
    let shell = vec![
        rounded_ring(hx + 0.02, hz + 0.02, 0.17, ysplit, 0.0, 3),
        rounded_ring(hx + 0.02, hz + 0.02, 0.17, ysplit + 0.42, 0.0, 3),
        rounded_ring(hx - 0.04, hz - 0.06, 0.2, ysplit + 0.55, 0.0, 3),
        rounded_ring(hx - 0.2, hz - 0.25, 0.2, ysplit + 0.61, 0.0, 3),
    ];
    loft(p.get(top), &shell, false, true, 1.0);
    // Band of grey HDPE between body and shell.
    p.get(body).aabox(v(-hx - 0.025, ysplit - 0.02, -hz - 0.025), v(hx + 0.025, ysplit + 0.03, hz + 0.025), 1.0);
    let fz = hz + 0.02;
    let dark = Mat::Plastic(hex(0x1e2022));
    match f {
        PAPEL => {
            // Letterbox slot with grey rubber flaps (photo "Contenidor de paper").
            p.get(dark).aabox(v(-0.42, ysplit + 0.1, fz - 0.03), v(0.42, ysplit + 0.3, fz + 0.002), 1.0);
            for k in 0..12 {
                let x = -0.4 + k as f32 * 0.068;
                p.get(Mat::Plastic(hex(0x5e6264))).aabox(v(x, ysplit + 0.11, fz - 0.005), v(x + 0.062, ysplit + 0.29, fz + 0.006), 1.0);
            }
        }
        ENVASES => {
            for x in [-0.25f32, 0.25] {
                p.get(dark).cylinder(v(x, ysplit + 0.22, fz - 0.03), v(x, ysplit + 0.22, fz + 0.003), 0.11, 14, true);
                p.get(top).cylinder(v(x, ysplit + 0.22, fz - 0.0), v(x, ysplit + 0.22, fz + 0.02), 0.125, 14, false);
            }
        }
        _ => {
            // Front-hinged lid panel and the pedal below.
            p.get(top).aabox(v(-0.5, ysplit + 0.08, fz), v(0.5, ysplit + 0.33, fz + 0.025), 1.0);
            p.get(dark).aabox(v(-0.15, ysplit + 0.26, fz + 0.02), v(0.15, ysplit + 0.3, fz + 0.06), 1.0);
            let g = Mat::Galvanized;
            p.get(g).beam(v(0.0, 0.2, hz - 0.08), v(0.0, 0.16, hz + 0.16), Vec3::X, 0.08, 0.02, 1.0);
            p.get(g).aabox(v(-0.1, 0.14, hz + 0.1), v(0.1, 0.165, hz + 0.24), 1.0);
        }
    }
    // Fraction labels on the shell sides, emblem and thanks band on the
    // body front (as on the 2023 fleet).
    for s in [-1.0f32, 1.0] {
        label_at(&mut p, Label::Fraction(f), v(s * (hx + 0.02), ysplit + 0.2, 0.0), Vec3::X * s, 0.24, 0.32);
    }
    label_at(&mut p, Label::Thanks, v(0.0, ysplit - 0.14, hz - 0.02), Vec3::new(0.0, 0.03, 1.0), 0.6, 0.075);
    label_at(&mut p, Label::Emblem, v(0.0, 0.72, hz - 0.035), Vec3::new(0.0, 0.03, 1.0), 0.26, 0.26);
    label_at(&mut p, Label::Round, v(-0.55, 0.85, hz - 0.03), Vec3::new(0.0, 0.03, 1.0), 0.13, 0.13);
    // Side-lift bracket on the road side (-Z): galvanized plate and bar.
    let g = Mat::Galvanized;
    p.get(g).aabox(v(-0.55, 0.85, -hz - 0.04), v(0.55, 0.95, -hz + 0.01), 1.0);
    for x in [-0.5f32, 0.5] {
        p.get(g).aabox(v(x - 0.04, 0.5, -hz - 0.035), v(x + 0.04, 1.2, -hz + 0.01), 1.0);
    }
    // Reflective chevrons on the road-side corners.
    for s in [-1.0f32, 1.0] {
        label_at(&mut p, Label::Chevron, v(s * (hx - 0.12), 0.7, -hz + 0.02), Vec3::NEG_Z, 0.12, 0.4);
    }
    wear_decals(&mut p, wear, hz - 0.02, hz, 0.6);
    p
}

/// 1100 l rear-loading bin (EN 840): 1370 wide × 1075 deep × 1330 high
/// to the lid, 200 mm castors, comb rim on the front, flat ribbed lid.
fn bin_1100(f: u8, wear: u8) -> P {
    let mut p = P::new();
    let body = body_mat(wear);
    let lid = fraction_mat(f, wear);
    let (hx, hz) = (0.685, 0.5);
    let y0 = 0.24;
    let ytop = 1.27;
    let rings = vec![
        rounded_ring(hx - 0.09, hz - 0.1, 0.06, y0, 0.0, 2),
        rounded_ring(hx - 0.02, hz - 0.02, 0.07, ytop - 0.12, 0.0, 2),
        rounded_ring(hx, hz, 0.07, ytop - 0.1, 0.0, 2),
        rounded_ring(hx, hz, 0.07, ytop, 0.0, 2),
    ];
    loft(p.get(body), &rings, true, false, 1.0);
    // Comb rim (lifting lip) on the front, side trunnions.
    p.get(body).aabox(v(-hx + 0.05, ytop - 0.2, hz - 0.01), v(hx - 0.05, ytop - 0.1, hz + 0.06), 1.0);
    for s in [-1.0f32, 1.0] {
        p.get(Mat::Galvanized).cylinder(v(s * hx, ytop - 0.35, 0.0), v(s * (hx + 0.1), ytop - 0.35, 0.0), 0.035, 8, true);
        p.get(body).aabox(v(s * hx - 0.04, ytop - 0.45, -0.1), v(s * hx + 0.04 * s.signum(), ytop - 0.25, 0.1), 1.0);
    }
    // Flat lid with two stiffening ridges and a front handle lip.
    p.get(lid).aabox(v(-hx - 0.02, ytop, -hz - 0.02), v(hx + 0.02, ytop + 0.05, hz + 0.04), 1.0);
    for x in [-0.3f32, 0.3] {
        p.get(lid).aabox(v(x - 0.1, ytop + 0.05, -hz + 0.05), v(x + 0.1, ytop + 0.07, hz - 0.05), 1.0);
    }
    // Base frame and four castors (200 mm).
    p.get(body).aabox(v(-hx + 0.1, y0 - 0.02, -hz + 0.12), v(hx - 0.1, y0 + 0.02, hz - 0.12), 1.0);
    for (x, z) in [(-0.5f32, -0.35f32), (0.5, -0.35), (-0.5, 0.35), (0.5, 0.35)] {
        castor(&mut p, v(x, 0.0, z), 0.1);
    }
    let tilt = Vec3::new(0.0, 0.07, 1.0);
    label_at(&mut p, Label::Emblem, v(-0.42, 0.95, hz - 0.03), tilt, 0.17, 0.17);
    label_at(&mut p, Label::Fraction(f), v(0.15, 0.7, hz - 0.04), tilt, 0.27, 0.36);
    label_at(&mut p, Label::Band(f), v(0.0, ytop - 0.15, hz + 0.065), Vec3::Z, 0.6, 0.06);
    wear_decals(&mut p, wear, hz - 0.035, hz, 0.7);
    p
}

/// Glass igloo (Ecovidrio type, 2.5 m³): 1500 diameter, crown at 1580,
/// lifting mushroom to 1780, two round mouths with rubber rings.
/// `wear` adds campaign stickers and tags.
pub fn glass_igloo(wear: u8) -> P {
    let mut p = P::new();
    let m = fraction_mat(VIDRIO, wear);
    let prof = [(0.7, 0.0), (0.74, 0.03), (0.74, 0.1), (0.72, 0.12), (0.75, 0.5), (0.74, 0.85), (0.69, 1.12), (0.58, 1.34), (0.42, 1.5), (0.22, 1.57), (0.0, 1.59)];
    p.get(m).lathe(Vec3::ZERO, &prof, 24);
    p.get(m).polygon(&(0..24).map(|i| Vec2::from_angle(-std::f32::consts::TAU * i as f32 / 24.0) * 0.7).collect::<Vec<_>>(), 0.0, false, 1.0);
    // Lifting mushroom and its ring.
    let g = Mat::Galvanized;
    p.get(g).cylinder(v(0.0, 1.57, 0.0), v(0.0, 1.73, 0.0), 0.035, 8, false);
    p.get(g).lathe(v(0.0, 1.72, 0.0), &[(0.0, 0.0), (0.1, 0.0), (0.12, 0.03), (0.08, 0.06), (0.0, 0.07)], 10);
    p.get(Mat::Plastic(hex(0x2a4a2c))).cylinder(v(0.0, 1.55, 0.0), v(0.0, 1.6, 0.0), 0.14, 12, true);
    // Mouths: front and back, high on the dome, with black rubber.
    for s in [1.0f32, -1.0] {
        let c = v(0.0, 1.22, s * 0.64);
        let n = v(0.0, 0.55, s * 0.84).normalize();
        let rot = Mat4::from_quat(forja_math::Quat::from_rotation_arc(Vec3::Y, n));
        let mut ring = P::new();
        ring.get(Mat::Rubber).lathe(Vec3::ZERO, &[(0.075, -0.05), (0.075, 0.0), (0.11, 0.02), (0.12, 0.0)], 14);
        ring.get(Mat::Plastic(hex(0x050505))).polygon(&(0..14).map(|i| Vec2::from_angle(-std::f32::consts::TAU * i as f32 / 14.0) * 0.076).collect::<Vec<_>>(), -0.04, true, 1.0);
        p.append(&ring, &(Mat4::from_translation(c) * rot));
    }
    // Front stickers: fraction label, campaign sticker on some.
    label_at(&mut p, Label::Fraction(VIDRIO), v(0.0, 0.62, 0.752), Vec3::Z, 0.34, 0.46);
    label_at(&mut p, Label::Emblem, v(0.0, 1.03, 0.73), v(0.0, 0.25, 1.0), 0.2, 0.2);
    if wear % 3 == 1 {
        label_at(&mut p, Label::Tag(wear), v(0.5, 0.5, 0.53), v(0.7, 0.0, 0.7), 0.6, 0.34);
    }
    p
}

/// Underground container mouth ("contenidor soterrat", Ciutat Vella and
/// new plazas): a stainless column 600 wide × 520 deep × 1150 high with a
/// rotating drum, the fraction band on the head, standing on the 1.6 m
/// square galvanized platform lid flush with the paving.
pub fn underground_bin(fraction: u8) -> P {
    let mut p = P::new();
    let f = fraction % 5;
    let s = Mat::Stainless;
    // Platform lid (chequer plate) and its frame, flush with the pavement.
    p.get(Mat::Galvanized).aabox(v(-0.8, -0.01, -0.8), v(0.8, 0.008, 0.8), 1.0);
    p.get(Mat::Iron).aabox(v(-0.83, -0.01, -0.83), v(0.83, 0.004, -0.8), 1.0);
    p.get(Mat::Iron).aabox(v(-0.83, -0.01, 0.8), v(0.83, 0.004, 0.83), 1.0);
    p.get(Mat::Iron).aabox(v(-0.83, -0.01, -0.8), v(-0.8, 0.004, 0.8), 1.0);
    p.get(Mat::Iron).aabox(v(0.8, -0.01, -0.8), v(0.83, 0.004, 0.8), 1.0);
    // Column: slightly tapered box with a sloped, rounded head.
    let col = Mat::Stainless;
    let rings = vec![
        rounded_ring(0.3, 0.26, 0.04, 0.0, 0.0, 2),
        rounded_ring(0.26, 0.22, 0.04, 0.95, 0.0, 2),
    ];
    loft(p.get(col), &rings, false, false, 1.0);
    // Tapered column (Burjassot / Alfafar photos): 600 × 520 at the foot,
    // 520 × 440 under the head. Sloped head: front 1.05 m, back 1.15 m.
    let hd = [v(-0.26, 0.95, 0.22), v(0.26, 0.95, 0.22), v(0.26, 1.05, 0.22), v(-0.26, 1.05, 0.22)];
    p.get(col).quad(hd[0], hd[1], hd[2], hd[3], 1.0);
    p.get(col).quad(v(0.26, 0.95, -0.22), v(-0.26, 0.95, -0.22), v(-0.26, 1.15, -0.22), v(0.26, 1.15, -0.22), 1.0);
    for sx in [-0.26f32, 0.26] {
        let o = if sx > 0.0 { [v(sx, 0.95, 0.22), v(sx, 0.95, -0.22), v(sx, 1.15, -0.22), v(sx, 1.05, 0.22)] } else { [v(sx, 0.95, -0.22), v(sx, 0.95, 0.22), v(sx, 1.05, 0.22), v(sx, 1.15, -0.22)] };
        p.get(col).quad_uv(o, [Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y]);
    }
    p.get(col).quad(v(-0.27, 1.05, 0.23), v(0.27, 1.05, 0.23), v(0.27, 1.155, -0.23), v(-0.27, 1.155, -0.23), 1.0);
    // Stainless drum door with its handle on the front.
    p.get(s).aabox(v(-0.21, 0.48, 0.226), v(0.21, 0.9, 0.262), 1.0);
    p.get(Mat::Plastic(hex(0x151515))).aabox(v(-0.18, 0.84, 0.262), v(0.18, 0.88, 0.264), 1.0);
    p.get(s).cylinder(v(-0.12, 0.56, 0.275), v(0.12, 0.56, 0.275), 0.015, 6, true);
    // Pedal for organic and general waste.
    if f == ORGANICO || f == RESTO {
        p.get(s).aabox(v(-0.12, 0.02, 0.26), v(0.12, 0.05, 0.4), 1.0);
    }
    // Fraction band on the sloped head and on the door.
    let n = v(0.0, 0.25, 0.05).normalize() * 0.0 + v(0.0, 0.0, 1.0);
    label_at(&mut p, Label::Band(f), v(0.0, 1.0, 0.224), n, 0.5, 0.07);
    label_at(&mut p, Label::Fraction(f), v(0.0, 0.3, 0.25), n, 0.2, 0.27);
    p
}

// ------------------------------------------------------------ litter bins

/// Litter-bin (papelera) models of Valencia.
#[derive(Clone, Copy, Debug, PartialEq, Hash, Eq)]
pub enum BinModel {
    /// The municipal double bin (carrer dels Soguers): anthracite 110 mm
    /// post with a ball finial, two ribbed 50 l drums on scrolled brackets.
    PostDouble,
    /// One municipal drum on its own post.
    PostSingle,
    /// One drum on a wall bracket (narrow old-town streets, Mare Vella).
    Wall,
    /// One drum clamped to a lamp post or signal pole (radius in cm).
    Clamp(u8),
    /// Pivoting drum between two square legs (Russafa / Malilla corners).
    Frame,
    /// Lime-green 50 l park bin with ashtray on a galvanized post
    /// (Albalat dels Tarongers garden).
    Park,
    /// Solar compacting bin of Ciutat Vella (120 l): grey for general,
    /// yellow for packaging.
    Compactor(u8),
}

const BIN_RGB: u32 = 0x3a3d40;

/// Municipal 50 l drum: 360 mm across the rim, 330 at the base, 520 high,
/// two pressed ribs, a ring of vent slots near the base, reflective label
/// and a black bag folded over the rim. Origin at the drum's base centre.
fn drum(p: &mut P, wear: u8) {
    let m = Mat::PowderCoat([0x3a + (wear % 3) * 4, 0x3d + (wear % 3) * 4, 0x40 + (wear % 3) * 3]);
    let (r0, r1, h) = (0.165f32, 0.18f32, 0.52f32);
    let r = |y: f32| r0 + (r1 - r0) * y / h;
    let mut prof = vec![(0.0, 0.0), (r0 - 0.01, 0.0), (r0, 0.01)];
    for y in [0.16f32, 0.36] {
        prof.extend([(r(y - 0.025), y - 0.025), (r(y) + 0.008, y), (r(y + 0.025), y + 0.025)]);
    }
    prof.extend([(r1 - 0.002, h - 0.03), (r1 + 0.008, h - 0.015), (r1 + 0.008, h), (r1 - 0.006, h)]);
    // Inside, back down to the bottom (inward normals).
    prof.extend([(r1 - 0.008, h - 0.02), (r0 - 0.008, 0.03), (0.0, 0.03)]);
    p.get(m).lathe(Vec3::ZERO, &prof, 12);
    // Vent slots near the base and the reflective label.
    let dark = Mat::Plastic(hex(0x111213));
    for k in 0..10 {
        let a = k as f32 / 10.0 * std::f32::consts::TAU + 0.3;
        let d = v(a.sin(), 0.0, a.cos());
        let c = d * (r(0.07) + 0.002) + Vec3::Y * 0.07;
        let t = Vec3::Y.cross(d);
        p.get(dark).quad(c - t * 0.025 - Vec3::Y * 0.02, c + t * 0.025 - Vec3::Y * 0.02, c + t * 0.025 + Vec3::Y * 0.02, c - t * 0.025 + Vec3::Y * 0.02, 1.0);
    }
    p.get(Mat::Plastic(hex(0xc8202a))).obox(v(0.0, 0.26, r(0.26) + 0.002), [Vec3::X, Vec3::Y, Vec3::Z], v(0.03, 0.012, 0.003), 1.0);
    // Bag over the rim (crumpled: alternate radii).
    let bag = Mat::Plastic(hex(0x141414));
    let n = 14;
    let mut out = Vec::new();
    for k in 0..=n {
        let a = k as f32 / n as f32 * std::f32::consts::TAU;
        let wob = if k % 2 == 0 { 0.006 } else { -0.001 };
        out.push((a, wob));
    }
    let pr = p.get(bag);
    let base = pr.positions.len() as u32;
    for (a, wob) in &out {
        let d = v(a.sin(), 0.0, a.cos());
        for (rr, y) in [(r1 + 0.012 + wob, h - 0.07 - wob * 2.0), (r1 + 0.014, h + 0.01), (r1 - 0.02 - wob, h + 0.02)] {
            pr.vertex(d * rr + Vec3::Y * y, (d + Vec3::Y * 0.3).normalize(), Vec2::new(*a, y));
        }
    }
    for k in 0..n as u32 {
        let a = base + k * 3;
        let b = a + 3;
        for j in 0..2 {
            pr.indices.extend([a + j, b + j, b + j + 1, a + j, b + j + 1, a + j + 1]);
        }
    }
}

/// Bin hanging ring and scrolled brace from a post at `post` (axis +Y)
/// to a drum centred at `c` (base).
fn drum_bracket(p: &mut P, post: Vec3, c: Vec3, post_r: f32) {
    let m = Mat::PowderCoat(hex(BIN_RGB));
    let rim = c + Vec3::Y * 0.47;
    let dir = (c - post).with_y(0.0).normalize_or(Vec3::X);
    p.get(m).lathe(rim, &[(0.185, 0.0), (0.193, 0.0), (0.193, 0.03), (0.185, 0.03)], 14);
    let a = post + dir * post_r + Vec3::Y * (rim.y - post.y + 0.015);
    let b = rim + Vec3::Y * 0.015 - dir * 0.185;
    p.get(m).beam(a, b, Vec3::Y, 0.03, 0.03, 1.0);
    // Scroll brace below: from the post down-out to the drum side.
    let mut prev = post + dir * post_r + Vec3::Y * (rim.y - post.y - 0.42);
    for k in 1..=5 {
        let t = k as f32 / 5.0;
        let q = post + dir * (post_r + t * 0.16) + Vec3::Y * (rim.y - post.y - 0.42 + (t * std::f32::consts::FRAC_PI_2).sin() * 0.4);
        p.get(m).beam(prev, q, Vec3::Y.cross(dir), 0.018, 0.018, 1.0);
        prev = q;
    }
}

fn post(p: &mut P, h: f32, ball: bool) {
    let m = Mat::PowderCoat(hex(BIN_RGB));
    p.get(m).aabox(v(-0.12, 0.0, -0.12), v(0.12, 0.012, 0.12), 1.0);
    p.get(m).lathe(Vec3::ZERO, &[(0.0, 0.012), (0.07, 0.012), (0.06, 0.06), (0.045, 0.1)], 12);
    p.get(m).cylinder(v(0.0, 0.08, 0.0), v(0.0, h, 0.0), 0.045, 12, false);
    if ball {
        p.get(m).lathe(v(0.0, h, 0.0), &[(0.0, 0.0), (0.05, 0.0), (0.04, 0.02), (0.05, 0.06), (0.05, 0.08), (0.03, 0.11), (0.0, 0.12)], 12);
    } else {
        p.get(m).cylinder(v(0.0, h, 0.0), v(0.0, h + 0.01, 0.0), 0.05, 12, true);
    }
}

/// Litter bin of `model`; `wear` varies the paint and adds stickers.
/// Origin at the foot of the post (or the wall), drums open on the +Z side
/// of a post (double: along ±X).
pub fn papelera(model: BinModel, wear: u8) -> P {
    let mut p = P::new();
    let drum_at = |p: &mut P, c: Vec3| {
        let mut d = P::new();
        drum(&mut d, wear);
        p.append(&d, &Mat4::from_translation(c));
    };
    match model {
        BinModel::PostDouble => {
            post(&mut p, 1.18, true);
            for s in [-1.0f32, 1.0] {
                let c = v(s * 0.23, 0.45, 0.0);
                drum_at(&mut p, c);
                drum_bracket(&mut p, Vec3::ZERO, c, 0.045);
            }
        }
        BinModel::PostSingle => {
            post(&mut p, 1.1, true);
            let c = v(0.0, 0.42, 0.23);
            drum_at(&mut p, c);
            drum_bracket(&mut p, Vec3::ZERO, c, 0.045);
        }
        BinModel::Wall => {
            let m = Mat::PowderCoat(hex(BIN_RGB));
            p.get(m).aabox(v(-0.06, 0.6, 0.0), v(0.06, 1.05, 0.012), 1.0);
            let c = v(0.0, 0.55, 0.2);
            drum_at(&mut p, c);
            drum_bracket(&mut p, v(0.0, 0.0, -0.0), c, 0.012);
            // Drain pipe-style support down to the pavement as in the photo.
            p.get(m).cylinder(v(0.0, 0.0, 0.04), v(0.0, 0.6, 0.04), 0.02, 8, true);
        }
        BinModel::Clamp(rcm) => {
            let r = rcm as f32 / 100.0;
            let c = v(0.0, 0.75, r + 0.2);
            drum_at(&mut p, c);
            drum_bracket(&mut p, Vec3::ZERO, c, r);
            for y in [0.83f32, 1.21] {
                p.get(Mat::Galvanized).cylinder(v(0.0, y, 0.0), v(0.0, y + 0.04, 0.0), r + 0.008, 12, false);
            }
        }
        BinModel::Frame => {
            let m = Mat::PowderCoat(hex(0x2e3133));
            for x in [-0.26f32, 0.26] {
                p.get(m).aabox(v(x - 0.02, 0.0, -0.02), v(x + 0.02, 0.92, 0.02), 1.0);
                p.get(m).aabox(v(x - 0.05, 0.0, -0.05), v(x + 0.05, 0.01, 0.05), 1.0);
                p.get(Mat::Galvanized).cylinder(v(x, 0.8, 0.0), v(x * 0.75, 0.8, 0.0), 0.012, 6, true);
            }
            p.get(m).aabox(v(-0.26, 0.3, -0.015), v(0.26, 0.33, 0.015), 1.0);
            drum_at(&mut p, v(0.0, 0.33, 0.0));
            // Copper-coloured band under the rim and a pictogram plate.
            p.get(Mat::PowderCoat(hex(0xa86a3a))).lathe(v(0.0, 0.33, 0.0), &[(0.181, 0.42), (0.183, 0.42), (0.183, 0.45), (0.181, 0.45)], 14);
            label_at(&mut p, Label::BinPlate, v(0.0, 0.58, 0.178), Vec3::Z, 0.13, 0.1);
        }
        BinModel::Park => {
            let g = Mat::Galvanized;
            p.get(g).cylinder(v(0.0, 0.0, -0.05), v(0.0, 1.25, -0.05), 0.03, 10, true);
            let m = Mat::Hdpe(hex(0xa9c23a));
            let rings = vec![rounded_ring(0.17, 0.13, 0.06, 0.38, 0.13, 3), rounded_ring(0.19, 0.15, 0.07, 0.95, 0.13, 3), rounded_ring(0.19, 0.15, 0.07, 1.0, 0.13, 3)];
            loft(p.get(m), &rings, true, false, 1.0);
            let lid = vec![rounded_ring(0.2, 0.16, 0.07, 1.0, 0.13, 3), rounded_ring(0.2, 0.16, 0.07, 1.04, 0.13, 3), rounded_ring(0.12, 0.08, 0.05, 1.09, 0.13, 3)];
            loft(p.get(m), &lid, false, true, 1.0);
            p.get(Mat::Plastic(hex(0x151515))).aabox(v(-0.14, 0.86, 0.27), v(0.14, 0.95, 0.284), 1.0);
            p.get(g).aabox(v(-0.06, 1.09, 0.08), v(0.06, 1.1, 0.18), 1.0);
            label_at(&mut p, Label::BinPlate, v(0.0, 0.65, 0.282), Vec3::Z, 0.16, 0.12);
        }
        BinModel::Compactor(f) => {
            // Ciutat Vella 2024 pilot (valencia.es / Valencia Plaza photos):
            // 640 wide × 680 deep × 1270 high, anthracite frame, coloured
            // front and side panels (grey general, yellow packaging), a
            // black hopper head with a steel pull handle, a steel kick
            // plate and a flat top carrying the solar panel.
            let col = if f % 5 == ENVASES { hex(0xe2b21c) } else { hex(0x8c9094) };
            let frame = Mat::PowderCoat(hex(0x2a2c2f));
            let panel = Mat::PowderCoat(col);
            let (hx, hz, h) = (0.32f32, 0.34f32, 1.27f32);
            let rings = vec![rounded_ring(hx, hz, 0.03, 0.0, 0.0, 2), rounded_ring(hx, hz, 0.03, h - 0.03, 0.0, 2)];
            loft(p.get(frame), &rings, false, false, 1.0);
            p.get(frame).aabox(v(-hx - 0.01, h - 0.04, -hz - 0.01), v(hx + 0.01, h, hz + 0.01), 1.0);
            p.get(Mat::Plastic(hex(0x1b2a4a))).aabox(v(-hx + 0.04, h, -hz + 0.05), v(hx - 0.04, h + 0.008, hz - 0.08), 1.0);
            // Front panel (inset in the frame) and side panels.
            p.get(panel).aabox(v(-hx + 0.04, 0.12, hz - 0.005), v(hx - 0.04, 0.84, hz + 0.008), 1.0);
            for s in [-1.0f32, 1.0] {
                p.get(panel).aabox(v(s * hx - 0.008, 0.12, -hz + 0.06), v(s * hx + 0.008, 0.84, hz - 0.06), 1.0);
            }
            // Hopper head: black, with the slot and the steel handle.
            let dark = Mat::Plastic(hex(0x151617));
            p.get(dark).aabox(v(-hx + 0.03, 0.88, hz - 0.004), v(hx - 0.03, h - 0.06, hz + 0.012), 1.0);
            p.get(Mat::Plastic(hex(0x050505))).aabox(v(-hx + 0.08, 1.0, hz + 0.012), v(hx - 0.08, 1.14, hz + 0.014), 1.0);
            p.get(Mat::Stainless).aabox(v(-0.18, 1.15, hz + 0.012), v(0.18, 1.18, hz + 0.06), 1.0);
            // Kick plate and the butt slot on the side.
            p.get(Mat::Stainless).aabox(v(-hx + 0.04, 0.02, hz - 0.004), v(hx - 0.04, 0.1, hz + 0.012), 1.0);
            p.get(Mat::Stainless).aabox(v(hx - 0.004, 0.95, -0.05), v(hx + 0.012, 1.02, 0.05), 1.0);
            label_at(&mut p, Label::Fraction(f % 5), v(0.0, 0.58, hz + 0.01), Vec3::Z, 0.24, 0.32);
            label_at(&mut p, Label::Emblem, v(0.0, 0.26, hz + 0.01), Vec3::Z, 0.12, 0.12);
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::prim_count;

    fn tris(p: &P) -> usize {
        p.tri_count()
    }

    #[test]
    fn containers_build_within_budget() {
        for model in [ContainerModel::Kerbside, ContainerModel::SideLoader, ContainerModel::Bin1100] {
            for f in 0..5u8 {
                for wear in 0..4u8 {
                    let p = container_model(model, f, wear);
                    assert!(prim_count(&p) > 50);
                    assert!(tris(&p) <= 4000, "{model:?} {f} {wear}: {} tris", tris(&p));
                }
            }
        }
        for m in [BinModel::PostDouble, BinModel::PostSingle, BinModel::Wall, BinModel::Clamp(8), BinModel::Frame, BinModel::Park, BinModel::Compactor(0), BinModel::Compactor(3)] {
            let t = tris(&papelera(m, 1));
            assert!(t > 100 && t <= 1500, "{m:?}: {t} tris");
        }
        for f in 0..5u8 {
            assert!(tris(&underground_bin(f)) <= 4000);
            assert!(tris(&container(f)) > 100);
        }
    }

    #[test]
    fn loft_normals_point_out() {
        let mut p = crate::geom::Prim::default();
        let rings = vec![rounded_ring(1.0, 0.5, 0.2, 0.0, 0.0, 3), rounded_ring(1.0, 0.5, 0.2, 1.0, 0.0, 3)];
        loft(&mut p, &rings, false, false, 1.0);
        for (q, n) in p.positions.iter().zip(&p.normals) {
            assert!(Vec3::new(q.x, 0.0, q.z).dot(*n) > 0.0, "{q} {n}");
        }
        // Winding agrees with the normals.
        for t in p.indices.chunks_exact(3) {
            let (a, b, c) = (p.positions[t[0] as usize], p.positions[t[1] as usize], p.positions[t[2] as usize]);
            let fn_ = (b - a).cross(c - a);
            assert!(fn_.dot(p.normals[t[0] as usize]) >= -1e-6);
        }
    }
}
