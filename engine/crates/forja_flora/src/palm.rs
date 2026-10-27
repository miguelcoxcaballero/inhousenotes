//! Palms: a segmented trunk with a leaf-base boot, a crown of pinnate
//! (date palm) or costapalmate fan (Washingtonia) fronds in a golden-angle
//! spiral, older fronds lower and drooping, and an optional skirt of dead
//! fronds hanging against the trunk.

use forja_math::{Vec2, Vec3};

use crate::Pruning;
use crate::mesh::{MeshBuilder, Rect};
use crate::rng::Rng;
use crate::skeleton::{Growth, Skeleton, Stem, StemKind, grow, rotate};
use crate::species::{PalmDef, PalmKind, Species};
use crate::texture::{FoliageAtlas, SlotKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrondKind {
    Green,
    Inner,
    Dead,
}

#[derive(Clone, Copy, Debug)]
pub struct Frond {
    pub base: Vec3,
    pub dir: Vec3,
    pub length: f32,
    /// Total droop of the frond tip (radians).
    pub droop: f32,
    pub kind: FrondKind,
    /// Roll of the blade around the rachis (radians).
    pub roll: f32,
    pub variant: usize,
    /// Fan opening (radians) for fan palms.
    pub open: f32,
}

pub struct PalmTree {
    pub skeleton: Skeleton,
    pub fronds: Vec<Frond>,
    pub crown_center: Vec3,
    pub crown_radii: Vec3,
}

pub fn build(sp: &Species, pd: &PalmDef, g: &Growth, rng: &mut Rng) -> PalmTree {
    let crown_h = match pd.kind {
        PalmKind::Pinnate => pd.frond_length * 0.45,
        PalmKind::Fan => pd.frond_length * 0.55,
    };
    let trunk_len = (g.height - crown_h).max(0.8);
    let lean = (sp.trunk.lean + sp.trunk.lean_var * rng.signed()).to_radians();
    let az = rng.f32() * std::f32::consts::TAU;
    let horiz = Vec3::new(az.cos(), 0.0, az.sin());
    let dir = rotate(Vec3::Y, horiz.cross(Vec3::Y), -lean);
    let segs = (trunk_len / 0.25).ceil().max(8.0) as u32;
    // Palms curve back toward vertical (phototropism).
    let pts = grow(Vec3::new(0.0, -0.3, 0.0), dir, trunk_len + 0.3, segs, sp.trunk.curve + lean.to_degrees() * 0.6, sp.trunk.curve_var, 0.0, rng);
    let n = pts.len();
    let r0 = g.trunk_radius;
    let boot = pd.boot * (g.height / sp.height[1]).clamp(0.4, 1.2);
    let mut radius = Vec::with_capacity(n);
    let mut acc = 0.0;
    for i in 0..n {
        if i > 0 {
            acc += (pts[i] - pts[i - 1]).length();
        }
        let t = acc / (trunk_len + 0.3);
        let from_top = (trunk_len + 0.3 - acc).max(0.0);
        // Slight taper, a swollen base (Washingtonia) and the boot.
        let mut r = r0 * (1.0 - 0.22 * t);
        r *= 1.0 + sp.trunk.flare * (-(acc - 0.3).max(0.0) / sp.trunk.flare_height.max(0.1)).exp();
        if from_top < boot {
            r *= 1.0 + 0.35 * (1.0 - from_top / boot).powf(0.7);
        }
        radius.push(r);
    }
    let mut trunk = Stem::new(StemKind::Trunk, 0, pts, None, 0.0);
    trunk.radius = radius;
    trunk.explicit = true;
    let top = *trunk.pts.last().unwrap();
    let top_dir = (trunk.pts[n - 1] - trunk.pts[n - 2]).normalize_or(Vec3::Y);
    let top_r = *trunk.radius.last().unwrap();
    let mut fronds = Vec::new();
    let count = (pd.fronds as f32 * g.age.clamp(0.35, 1.2) * (0.6 + 0.4 * g.health)).round().max(6.0) as usize;
    let golden = 2.399_963;
    let mut phi = rng.f32() * std::f32::consts::TAU;
    let size = (g.height / sp.height[1]).clamp(0.5, 1.15).sqrt();
    for i in 0..count {
        let r = i as f32 / count as f32; // 0 = youngest (centre), 1 = oldest
        phi += golden;
        let elev = (78.0 - 115.0 * r.powf(0.85) + rng.signed() * 7.0).to_radians();
        let h = Vec3::new(phi.cos(), 0.0, phi.sin());
        let d = (h * elev.cos() + Vec3::Y * elev.sin()).normalize();
        let d = (d + top_dir - Vec3::Y).normalize_or(d);
        let len = pd.frond_length * size * (1.0 + pd.frond_var * rng.signed()) * (0.55 + 0.45 * (r * 4.0).min(1.0));
        let base = top - top_dir * r * boot * 0.55 + h * top_r * 0.5;
        let droop = pd.droop * (0.25 + 0.95 * r) * rng.range(0.8, 1.2);
        let kind = if r > 0.55 && rng.chance(0.35) { FrondKind::Inner } else { FrondKind::Green };
        fronds.push(Frond { base, dir: d, length: len, droop, kind, roll: rng.signed() * 0.25, variant: rng.below(2), open: rng.range(2.9, 3.6) });
    }
    let trimmed = g.pruning == Pruning::Trimmed;
    if !trimmed {
        // A few dead fronds just under the green crown.
        for _ in 0..(count / 10) {
            phi += golden;
            let h = Vec3::new(phi.cos(), 0.0, phi.sin());
            let d = (h * 0.45 - Vec3::Y * 0.9).normalize();
            fronds.push(Frond { base: top - top_dir * boot * 0.7 + h * top_r, dir: d, length: pd.frond_length * size * 0.85, droop: 0.2, kind: FrondKind::Dead, roll: rng.signed() * 0.5, variant: rng.below(2), open: rng.range(1.6, 2.4) });
        }
        // Skirt: dead fronds hanging against the trunk.
        if pd.skirt > 0.0 {
            let skirt = pd.skirt * size;
            let rows = (skirt / 0.35).ceil() as usize;
            for k in 0..rows {
                let y_off = boot * 0.6 + k as f32 * skirt / rows as f32;
                let per = 9 + rng.below(4);
                for _ in 0..per {
                    phi += golden;
                    let h = Vec3::new(phi.cos(), 0.0, phi.sin());
                    let d = (h * 0.16 - Vec3::Y).normalize();
                    let base = top - top_dir * y_off + h * (top_r + 0.12 + k as f32 * 0.01);
                    fronds.push(Frond { base, dir: d, length: pd.frond_length * size * rng.range(0.6, 0.85), droop: 0.05, kind: FrondKind::Dead, roll: rng.signed() * 0.4, variant: rng.below(2), open: rng.range(1.4, 2.2) });
                }
            }
        }
    }
    let crown_center = top + Vec3::Y * crown_h * 0.25;
    let reach = pd.frond_length * size * 0.85;
    let env = crate::skeleton::Env::new(crate::species::Envelope::Sphere, g.height, reach * 2.0 / g.height, 0.6);
    PalmTree {
        skeleton: Skeleton { stems: vec![trunk], height: g.height, trunk_radius: r0, envelope: env },
        fronds,
        crown_center,
        crown_radii: Vec3::new(reach, crown_h, reach),
    }
}

fn frond_rect(atlas: &FoliageAtlas, f: &Frond, fan: bool) -> Rect {
    let kind = match (fan, f.kind) {
        (false, FrondKind::Green) => SlotKind::Frond,
        (false, FrondKind::Inner) => SlotKind::FrondInner,
        (false, FrondKind::Dead) => SlotKind::FrondDead,
        (true, FrondKind::Green) => SlotKind::Fan,
        (true, FrondKind::Inner) => SlotKind::FanInner,
        (true, FrondKind::Dead) => SlotKind::FanDead,
    };
    let l = atlas.of(kind);
    l.get(f.variant % l.len().max(1)).copied().unwrap_or([0.0, 0.0, 0.25, 0.5])
}

/// Rachis frame at fraction `t`: (position, tangent, side, up).
fn rachis(f: &Frond, t: f32) -> (Vec3, Vec3, Vec3, Vec3) {
    // Integrate the droop: the direction rotates down quadratically.
    let steps = 24;
    let mut p = f.base;
    let mut d = f.dir;
    let horiz = Vec3::new(f.dir.x, 0.0, f.dir.z).normalize_or(Vec3::X);
    let axis = horiz.cross(Vec3::Y).normalize_or(Vec3::Z);
    let n = (t * steps as f32).ceil() as usize;
    for i in 0..n {
        let s = ((i + 1) as f32 / steps as f32).min(t);
        let s0 = i as f32 / steps as f32;
        let ds = (s - s0).max(0.0);
        d = rotate(f.dir, axis, -f.droop * s * s * 1.5);
        p += d * f.length * ds;
    }
    let side = d.cross(Vec3::Y).normalize_or(axis);
    let side = rotate(side, d, f.roll);
    let up = side.cross(d).normalize_or(Vec3::Y);
    (p, d, side, up)
}

/// Meshes the fronds: pinnate fronds as two V-folded ribbons of leaflets,
/// fan fronds as a petiole and a pleated radial blade.
pub fn mesh_fronds(mb: &mut MeshBuilder, pd: &PalmDef, fronds: &[Frond], atlas: &FoliageAtlas, center: Vec3, lod: usize) {
    let fan = pd.kind == PalmKind::Fan;
    let petiole_rect = atlas.of(SlotKind::Petiole).first().copied().unwrap_or([0.75, 0.25, 1.0, 0.5]);
    for f in fronds {
        let rect = frond_rect(atlas, f, fan);
        let crown_n = |p: Vec3| ((p - center) * Vec3::new(1.0, 0.6, 1.0) + Vec3::Y * 0.6).normalize_or(Vec3::Y);
        if !fan {
            let segs = [16, 10, 3][lod.min(2)];
            let across = if lod == 0 { 3 } else { 1 };
            let fold = pd.fold.to_radians() * if f.kind == FrondKind::Dead { 0.4 } else { 1.0 };
            let w = pd.blade_width * (f.length / pd.frond_length).sqrt();
            for side_sign in [-1.0f32, 1.0] {
                let base = mb.pos.len() as u32;
                for i in 0..=segs {
                    let t = i as f32 / segs as f32;
                    let (p, d, side, up) = rachis(f, t);
                    // Leaflets V upward and sag toward their tips.
                    for j in 0..=across {
                        let k = j as f32 / across as f32;
                        let e = (side * side_sign * fold.cos() + up * fold.sin()).normalize();
                        let sag = -Vec3::Y * k * k * w * 0.35 * (0.4 + t);
                        let q = p + e * w * k + sag + d * w * 0.35 * k;
                        let n = (up * 0.5 + crown_n(q) * 0.7).normalize();
                        let uv = Vec2::new(rect[0] + (rect[2] - rect[0]) * k, rect[1] + (rect[3] - rect[1]) * (1.0 - t));
                        mb.vertex(q, n, uv, 0.45 + 0.55 * t);
                    }
                }
                let row = (across + 1) as u32;
                for i in 0..segs as u32 {
                    for j in 0..across as u32 {
                        let a = base + i * row + j;
                        let b = a + 1;
                        let c = a + row;
                        let d = c + 1;
                        mb.tri(a, b, c);
                        mb.tri(b, d, c);
                    }
                }
            }
        } else {
            // Petiole: two crossed ribbons from the trunk to the hastula.
            let pl = pd.petiole.max(0.2);
            let psegs = [4, 2, 1][lod.min(2)];
            let pw = 0.05;
            for cross in 0..if lod == 0 { 2 } else { 1 } {
                let base = mb.pos.len() as u32;
                for i in 0..=psegs {
                    let t = pl * i as f32 / psegs as f32;
                    let (p, _d, side, up) = rachis(f, t);
                    let e = if cross == 0 { side } else { up };
                    let n = crown_n(p);
                    for (j, s) in [-1.0f32, 1.0].iter().enumerate() {
                        let uv = Vec2::new(if j == 0 { petiole_rect[0] } else { petiole_rect[2] }, petiole_rect[1] + (petiole_rect[3] - petiole_rect[1]) * (i as f32 / psegs as f32));
                        mb.vertex(p + e * pw * *s, n, uv, 0.4 + 0.3 * t);
                    }
                }
                for i in 0..psegs as u32 {
                    let a = base + i * 2;
                    mb.tri(a, a + 1, a + 2);
                    mb.tri(a + 1, a + 3, a + 2);
                }
            }
            // Blade: radial grid around the hastula.
            let (h, d, side, up) = rachis(f, pl);
            let radius = f.length * (1.0 - pl) * 0.62;
            let ang = [32, 10, 6][lod.min(2)];
            let rad = [4, 2, 1][lod.min(2)];
            // Blade plane: spans `side` and the frond direction tilted up.
            let fwd = (d * 0.75 + up * 0.25).normalize();
            let base = mb.pos.len() as u32;
            let open = f.open;
            let dead = f.kind == FrondKind::Dead;
            for i in 0..=rad {
                let rho = (i as f32 / rad as f32).max(0.04);
                for j in 0..=ang {
                    let a = -open * 0.5 + open * j as f32 / ang as f32;
                    let dir = side * a.sin() + fwd * a.cos();
                    let pleat = if lod == 0 { ((j % 2) as f32 - 0.5) * 0.08 * radius * rho } else { 0.0 };
                    let sag = -Vec3::Y * rho * rho * radius * if dead { 0.15 } else { 0.35 };
                    // Costapalmate: the two halves fold up along the costa
                    // (a shallow V, not a flat disc), and past the split the
                    // free segment tips hang down.
                    let side_frac = (a.abs() / (open * 0.5)).min(1.0);
                    let fold = up * rho * radius * 0.28 * side_frac;
                    let hang = -Vec3::Y * (rho - 0.5).max(0.0).powi(2) * radius * if dead { 0.4 } else { 1.5 };
                    let curl = up * rho * rho * radius * 0.06 * side_frac * side_frac;
                    let q = h + dir * rho * radius + up * pleat + sag + fold + hang + curl;
                    let n = (up * 0.45 + crown_n(q) * 0.75).normalize();
                    // Texture angle matches the drawing: blade centre at the slot centre.
                    // The drawing is a full disc of segments: a narrower
                    // blade samples a wider arc so segments keep their width.
                    let ta = a * (4.8 / open).max(1.0) - std::f32::consts::FRAC_PI_2;
                    let uv = Vec2::new(
                        (rect[0] + rect[2]) * 0.5 + ta.cos() * rho * 0.48 * (rect[2] - rect[0]),
                        (rect[1] + rect[3]) * 0.5 + ta.sin() * rho * 0.48 * (rect[3] - rect[1]),
                    );
                    mb.vertex(q, n, uv, (0.5 + 0.5 * rho).min(1.0));
                }
            }
            let row = (ang + 1) as u32;
            for i in 0..rad as u32 {
                for j in 0..ang as u32 {
                    let a = base + i * row + j;
                    let b = a + 1;
                    let c = a + row;
                    let d = c + 1;
                    mb.tri(a, b, c);
                    mb.tri(b, d, c);
                }
            }
        }
    }
}
