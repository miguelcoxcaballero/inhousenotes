//! Mesh builders: generalized cylinders along stems, leaf cards, spheres.

use forja_assets::MeshData;
use forja_math::{Vec2, Vec3};

use crate::rng::fbm3;
use crate::skeleton::{Stem, StemKind, perp};
use crate::species::TrunkDef;

/// Growing triangle list with a wind weight per vertex.
#[derive(Clone, Debug, Default)]
pub struct MeshBuilder {
    pub pos: Vec<Vec3>,
    pub nrm: Vec<Vec3>,
    pub uv: Vec<Vec2>,
    pub wind: Vec<f32>,
    pub idx: Vec<u32>,
}

impl MeshBuilder {
    pub fn vertex(&mut self, p: Vec3, n: Vec3, uv: Vec2, wind: f32) -> u32 {
        self.pos.push(p);
        self.nrm.push(n.normalize_or(Vec3::Y));
        self.uv.push(uv);
        self.wind.push(wind.clamp(0.0, 1.0));
        (self.pos.len() - 1) as u32
    }

    pub fn tri(&mut self, a: u32, b: u32, c: u32) {
        self.idx.extend_from_slice(&[a, b, c]);
    }

    pub fn triangles(&self) -> usize {
        self.idx.len() / 3
    }

    /// Appends another builder's geometry; returns the index range it adds.
    pub fn append(&mut self, o: &MeshBuilder) -> std::ops::Range<usize> {
        let base = self.pos.len() as u32;
        let start = self.idx.len();
        self.pos.extend_from_slice(&o.pos);
        self.nrm.extend_from_slice(&o.nrm);
        self.uv.extend_from_slice(&o.uv);
        self.wind.extend_from_slice(&o.wind);
        self.idx.extend(o.idx.iter().map(|i| i + base));
        start..self.idx.len()
    }
}

/// One primitive with LOD index lists over a shared vertex buffer:
/// LOD 0 = `levels[0]`, `MeshData::lods` = the coarser ones.
pub fn merge_lods(levels: &[MeshBuilder], material: usize) -> (MeshData, Vec<f32>) {
    let mut all = MeshBuilder::default();
    let mut lists = Vec::new();
    for l in levels {
        let r = all.append(l);
        lists.push(all.idx[r].to_vec());
    }
    let mut m = MeshData {
        positions: all.pos,
        normals: all.nrm,
        uvs: all.uv,
        indices: lists[0].clone(),
        material,
        ..Default::default()
    };
    // Tangents over every level (finalize only sees LOD 0's indices).
    let all_idx: Vec<u32> = lists.iter().flatten().copied().collect();
    m.tangents = forja_assets::generate_tangents(&m.positions, &m.normals, &m.uvs, &all_idx);
    m.lods = lists[1..].to_vec();
    m.lod_sync = true;
    m.finalize();
    m.aabb = forja_math::Aabb::from_points(m.positions.iter().copied());
    (m, all.wind)
}

/// How a stem is turned into a tube.
#[derive(Clone, Copy, Debug)]
pub struct TubeOpts {
    /// Ring vertices per metre of circumference, and the clamp.
    pub sides_per_m: f32,
    pub min_sides: u32,
    pub max_sides: u32,
    /// Keep every n-th skeleton point (plus the ends).
    pub step: usize,
    /// Subdivide segments longer than this (m) with Catmull-Rom.
    pub max_seg: f32,
    /// Bark texture tile (m around, m along).
    pub tile: [f32; 2],
}

/// Radial modulation for the trunk: flare, lobes, buttresses, fluting, twist.
#[derive(Clone, Debug)]
pub struct TrunkShape {
    pub flare: f32,
    pub flare_h: f32,
    pub lobes: u32,
    pub lobe_depth: f32,
    pub twist: f32,
    pub fluting: f32,
    pub planks: Vec<(f32, f32, f32)>,
    pub plank_h: f32,
    /// Palm trunk rings: spacing (m) and relative depth.
    pub ring: f32,
    pub ring_depth: f32,
    pub ribs: u32,
    pub rib_depth: f32,
    pub seed: u32,
}

impl TrunkShape {
    pub fn new(t: &TrunkDef, rng: &mut crate::rng::Rng, height_scale: f32) -> Self {
        let mut planks = Vec::new();
        if t.buttress > 0.0 {
            let n = t.lobes.max(3);
            for k in 0..n {
                let a = (k as f32 + rng.signed() * 0.3) / n as f32 * std::f32::consts::TAU;
                planks.push((a, t.buttress * rng.range(0.5, 1.2), rng.range(10.0, 26.0)));
            }
        }
        TrunkShape {
            flare: t.flare,
            flare_h: t.flare_height * height_scale.sqrt(),
            lobes: t.lobes,
            lobe_depth: t.lobe_depth,
            twist: t.twist.to_radians(),
            fluting: t.fluting,
            planks,
            plank_h: t.buttress_height * height_scale.sqrt(),
            ring: 0.0,
            ring_depth: 0.0,
            ribs: t.ribs,
            rib_depth: t.rib_depth,
            seed: rng.next_u64() as u32,
        }
    }

    /// Radius multiplier at angle `th` and height `h` above ground.
    pub fn factor(&self, th: f32, h: f32) -> f32 {
        let hh = h.max(0.0);
        let ph = th + self.twist * hh;
        let fl = (-hh / self.flare_h.max(0.05)).exp();
        let mut m = 1.0 + self.flare * fl;
        if self.lobes > 1 {
            let c = 0.5 + 0.5 * (self.lobes as f32 * ph).cos();
            let lob = (-hh / (self.flare_h * 2.5).max(0.1)).exp() * 0.75 + 0.25;
            m *= 1.0 + self.lobe_depth * lob * (c * c - 0.4);
        }
        for (a, amp, sharp) in &self.planks {
            let d = (th - a).cos().max(0.0);
            let fall = (-hh / self.plank_h.max(0.1)).exp();
            m += amp * fall * d.powf(*sharp);
        }
        if self.ribs > 0 {
            // Fused aerial roots: rounded vertical ridges that wander a little.
            let wob = fbm3(Vec3::new(0.0, hh * 0.35, 0.0), 2, self.seed ^ 77) * 0.6;
            let r = ((th + wob) * self.ribs as f32 * 0.5).sin().abs();
            m *= 1.0 + self.rib_depth * (r.sqrt() - 0.6);
        }
        if self.ring > 0.0 && hh > 0.05 {
            // Leaf scars: a sharp ledge every `ring` metres.
            let f = (hh / self.ring + 0.13 * (ph * 2.0).sin()).fract();
            m *= 1.0 + self.ring_depth * (f.powf(3.0) - 0.25);
        }
        if self.fluting > 0.0 {
            let p = Vec3::new(ph.cos() * 1.6, hh * 0.9, ph.sin() * 1.6);
            m *= 1.0 + self.fluting * fbm3(p, 3, self.seed);
        }
        m.max(0.3)
    }
}

/// Catmull-Rom resampling so long segments bend smoothly.
fn refine(pts: &[Vec3], rad: &[f32], step: usize, max_seg: f32) -> (Vec<Vec3>, Vec<f32>) {
    let n = pts.len();
    let mut keep: Vec<usize> = (0..n).step_by(step.max(1)).collect();
    if *keep.last().unwrap() != n - 1 {
        keep.push(n - 1);
    }
    let p: Vec<Vec3> = keep.iter().map(|&i| pts[i]).collect();
    let r: Vec<f32> = keep.iter().map(|&i| rad[i.min(rad.len() - 1)]).collect();
    let mut op = Vec::new();
    let mut or = Vec::new();
    for i in 0..p.len() - 1 {
        let p0 = if i == 0 { p[0] * 2.0 - p[1] } else { p[i - 1] };
        let (p1, p2) = (p[i], p[i + 1]);
        let p3 = if i + 2 < p.len() { p[i + 2] } else { p2 * 2.0 - p1 };
        let k = (((p2 - p1).length() / max_seg).ceil() as usize).clamp(1, 8);
        for j in 0..k {
            let t = j as f32 / k as f32;
            let t2 = t * t;
            let t3 = t2 * t;
            let q = 0.5 * ((2.0 * p1) + (-p0 + p2) * t + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2 + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3);
            op.push(q);
            or.push(r[i] + (r[i + 1] - r[i]) * t);
        }
    }
    op.push(*p.last().unwrap());
    or.push(*r.last().unwrap());
    (op, or)
}

/// Meshes one stem as a closed generalized cylinder: rings along the
/// curve with parallel-transported frames, a base cap and a tip cone (or a
/// rounded knuckle). Bark UVs: `u` wraps an integer number of tiles around
/// the base circumference, `v` is arc length, so the texture runs
/// continuously along the stem.
pub fn tube(mb: &mut MeshBuilder, stem: &Stem, o: &TubeOpts, shape: Option<&TrunkShape>, wind: &dyn Fn(f32, Vec3) -> f32) {
    if stem.pts.len() < 2 {
        return;
    }
    let (pts, mut rad) = refine(&stem.pts, &stem.radius, o.step, o.max_seg);
    let n = pts.len();
    // Pollard knuckle: swell toward the tip.
    if stem.knob > 0.0 {
        let len: f32 = pts.windows(2).map(|w| (w[1] - w[0]).length()).sum();
        let mut acc = 0.0;
        let knob_len = (rad[0] * 3.0).min(len * 0.4);
        for i in 0..n {
            if i > 0 {
                acc += (pts[i] - pts[i - 1]).length();
            }
            let t = ((acc - (len - knob_len)) / knob_len).clamp(0.0, 1.0);
            let bump = (t * std::f32::consts::PI * 0.85).sin().max(0.0);
            rad[i] = rad[i].max(rad[0] * 0.8) * (1.0 + (stem.knob - 1.0).max(0.0) * bump);
        }
    }
    let rmax = rad.iter().cloned().fold(0.0, f32::max);
    let mut sides = ((std::f32::consts::TAU * rmax * o.sides_per_m).round() as u32).clamp(o.min_sides, o.max_sides);
    if shape.is_some_and(|s| !s.planks.is_empty()) {
        sides = (sides * 2).min(64);
    }
    let sides = sides as usize;
    // Flared bases are wider than the pipe radius: tile for the average.
    let flare = shape.map(|s| 1.0 + s.flare * 0.4 + s.planks.iter().map(|p| p.1).sum::<f32>() * 0.08).unwrap_or(1.0);
    let circ = std::f32::consts::TAU * rad[0].max(0.005) * flare;
    let urep = (circ / o.tile[0]).round().max(1.0);
    // Tangents and parallel-transport frames.
    let tan: Vec<Vec3> = (0..n)
        .map(|i| {
            let a = pts[i.saturating_sub(1)];
            let b = pts[(i + 1).min(n - 1)];
            (b - a).normalize_or(Vec3::Y)
        })
        .collect();
    let mut nor = Vec::with_capacity(n);
    let mut nv = perp(tan[0]);
    if stem.kind == StemKind::Trunk {
        nv = (Vec3::X - tan[0] * tan[0].x).normalize_or(perp(tan[0]));
    }
    for i in 0..n {
        if i > 0 {
            let q = forja_math::Quat::from_rotation_arc(tan[i - 1], tan[i]);
            nv = (q * nv).normalize_or(perp(tan[i]));
            nv = (nv - tan[i] * nv.dot(tan[i])).normalize_or(perp(tan[i]));
        }
        nor.push(nv);
    }
    let mut arc = vec![0.0f32; n];
    for i in 1..n {
        arc[i] = arc[i - 1] + (pts[i] - pts[i - 1]).length();
    }
    let total = arc[n - 1].max(1e-3);
    // Ring positions.
    let ring = sides + 1;
    let mut grid = vec![Vec3::ZERO; n * ring];
    for i in 0..n {
        let (t, nn) = (tan[i], nor[i]);
        let b = t.cross(nn);
        for j in 0..ring {
            let th = (j % sides) as f32 / sides as f32 * std::f32::consts::TAU;
            let mut off = nn * th.cos() + b * th.sin();
            if stem.aspect != 1.0 {
                off += Vec3::Y * off.y * (stem.aspect - 1.0);
            }
            let f = shape.map(|s| s.factor(th, pts[i].y)).unwrap_or(1.0);
            grid[i * ring + j] = pts[i] + off * rad[i] * f;
        }
    }
    let base = mb.pos.len() as u32;
    for i in 0..n {
        for j in 0..ring {
            let jp = if j + 1 >= ring { 1 } else { j + 1 };
            let jm = if j == 0 { sides - 1 } else { j - 1 };
            let around = grid[i * ring + jp] - grid[i * ring + jm];
            let along = grid[(i + 1).min(n - 1) * ring + j] - grid[i.saturating_sub(1) * ring + j];
            let mut nrm = around.cross(along);
            let out = grid[i * ring + j] - pts[i];
            if nrm.dot(out) < 0.0 {
                nrm = -nrm;
            }
            if nrm.length_squared() < 1e-12 {
                nrm = out;
            }
            let u = j as f32 / sides as f32 * urep;
            let v = arc[i] / o.tile[1];
            mb.vertex(grid[i * ring + j], nrm, Vec2::new(u, v), wind(arc[i] / total, pts[i]));
        }
    }
    for i in 0..n - 1 {
        for j in 0..sides {
            let a = base + (i * ring + j) as u32;
            let b = a + 1;
            let c = a + ring as u32;
            let d = c + 1;
            mb.tri(a, b, c);
            mb.tri(b, d, c);
        }
    }
    // Base cap (closed mesh; hidden inside the parent or under ground).
    let c0 = mb.vertex(pts[0], -tan[0], Vec2::new(0.5 * urep, 0.0), wind(0.0, pts[0]));
    for j in 0..sides {
        let a = base + j as u32;
        mb.tri(c0, a + 1, a);
    }
    // Tip: a cone, or a rounded dome over a knuckle.
    let last = base + ((n - 1) * ring) as u32;
    let tip_len = if stem.knob > 0.0 { rad[n - 1] * 0.7 } else { rad[n - 1] * 1.5 };
    let tip = pts[n - 1] + tan[n - 1] * tip_len;
    let ct = mb.vertex(tip, tan[n - 1], Vec2::new(0.5 * urep, (arc[n - 1] + tip_len) / o.tile[1]), wind(1.0, tip));
    for j in 0..sides {
        let a = last + j as u32;
        mb.tri(a, a + 1, ct);
    }
}

/// A leaf card: a quad (optionally folded along its axis into a V) whose
/// normals are bent toward the crown normal for soft two-sided lighting.
#[derive(Clone, Copy, Debug)]
pub struct Card {
    pub origin: Vec3,
    pub axis: Vec3,
    pub normal: Vec3,
    pub length: f32,
    pub width: f32,
    pub rect: Rect,
    pub mirror: bool,
    pub wind: f32,
    /// Crown normal at the card (for bent normals).
    pub crown_n: Vec3,
    pub fold: f32,
}

/// UV rectangle in the atlas: `(u0, v0, u1, v1)`, v0 = top of the image.
pub type Rect = [f32; 4];

pub fn card(mb: &mut MeshBuilder, c: &Card, bend: f32, folded: bool) {
    let rect = c.rect;
    let a = c.axis.normalize_or(Vec3::Y);
    let n0 = (c.normal - a * a.dot(c.normal)).normalize_or(perp(a));
    let s = a.cross(n0);
    let nb = (n0 * (1.0 - bend) + c.crown_n * bend).normalize_or(n0);
    let (u0, u1) = if c.mirror { (rect[2], rect[0]) } else { (rect[0], rect[2]) };
    let um = (u0 + u1) * 0.5;
    let (vt, vb) = (rect[1], rect[3]);
    let hw = c.width * 0.5;
    let tipw = c.wind;
    let basew = (c.wind - 0.12).max(0.0);
    if folded && c.fold.abs() > 0.5 {
        let f = c.fold.to_radians();
        let el = crate::skeleton::rotate(-s, a, -f);
        let er = crate::skeleton::rotate(s, a, f);
        let o = c.origin;
        let tipv = a * c.length;
        let l0 = mb.vertex(o + el * hw, nb, Vec2::new(u0, vb), basew);
        let c0 = mb.vertex(o, nb, Vec2::new(um, vb), basew);
        let r0 = mb.vertex(o + er * hw, nb, Vec2::new(u1, vb), basew);
        let l1 = mb.vertex(o + el * hw + tipv, nb, Vec2::new(u0, vt), tipw);
        let c1 = mb.vertex(o + tipv, nb, Vec2::new(um, vt), tipw);
        let r1 = mb.vertex(o + er * hw + tipv, nb, Vec2::new(u1, vt), tipw);
        mb.tri(l0, c0, l1);
        mb.tri(c0, c1, l1);
        mb.tri(c0, r0, c1);
        mb.tri(r0, r1, c1);
    } else {
        let o = c.origin;
        let tipv = a * c.length;
        let v0 = mb.vertex(o - s * hw, nb, Vec2::new(u0, vb), basew);
        let v1 = mb.vertex(o + s * hw, nb, Vec2::new(u1, vb), basew);
        let v2 = mb.vertex(o + s * hw + tipv, nb, Vec2::new(u1, vt), tipw);
        let v3 = mb.vertex(o - s * hw + tipv, nb, Vec2::new(u0, vt), tipw);
        mb.tri(v0, v1, v2);
        mb.tri(v0, v2, v3);
    }
}

/// UV sphere mapped into `rect` (fruit).
pub fn sphere(mb: &mut MeshBuilder, center: Vec3, radius: Vec3, rings: usize, segs: usize, rect: Rect, wind: f32) {
    let base = mb.pos.len() as u32;
    for i in 0..=rings {
        let phi = i as f32 / rings as f32 * std::f32::consts::PI;
        for j in 0..=segs {
            let th = j as f32 / segs as f32 * std::f32::consts::TAU;
            let d = Vec3::new(phi.sin() * th.cos(), phi.cos(), phi.sin() * th.sin());
            let uv = Vec2::new(rect[0] + (rect[2] - rect[0]) * (j as f32 / segs as f32), rect[1] + (rect[3] - rect[1]) * (i as f32 / rings as f32));
            mb.vertex(center + d * radius, d / radius.max(Vec3::splat(1e-4)), uv, wind);
        }
    }
    let w = (segs + 1) as u32;
    for i in 0..rings as u32 {
        for j in 0..segs as u32 {
            let a = base + i * w + j;
            let b = a + 1;
            let c = a + w;
            let d = c + 1;
            // Outward: (b - a) x (c - a) points away from the centre.
            mb.tri(a, b, c);
            mb.tri(b, d, c);
        }
    }
}

#[cfg(test)]
pub(crate) fn welded_edges_closed(mb: &MeshBuilder, range: std::ops::Range<usize>) -> bool {
    use std::collections::HashMap;
    let q = |p: Vec3| ((p.x * 1e4).round() as i64, (p.y * 1e4).round() as i64, (p.z * 1e4).round() as i64);
    let mut edges: HashMap<((i64, i64, i64), (i64, i64, i64)), i32> = HashMap::new();
    for t in mb.idx[range].chunks_exact(3) {
        let v: Vec<_> = t.iter().map(|i| q(mb.pos[*i as usize])).collect();
        if v[0] == v[1] || v[1] == v[2] || v[0] == v[2] {
            continue;
        }
        for k in 0..3 {
            let (a, b) = (v[k], v[(k + 1) % 3]);
            let key = if a < b { (a, b) } else { (b, a) };
            *edges.entry(key).or_default() += 1;
        }
    }
    edges.values().all(|c| *c == 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tube_is_closed_with_outward_normals() {
        let pts: Vec<Vec3> = (0..8).map(|i| Vec3::new((i as f32 * 0.4).sin() * 0.3, i as f32 * 0.5, 0.0)).collect();
        let mut s = Stem::new(StemKind::Branch, 1, pts, None, 0.0);
        s.radius = (0..8).map(|i| 0.2 - i as f32 * 0.02).collect();
        let mut mb = MeshBuilder::default();
        let o = TubeOpts { sides_per_m: 40.0, min_sides: 5, max_sides: 16, step: 1, max_seg: 0.3, tile: [1.0, 1.0] };
        tube(&mut mb, &s, &o, None, &|_, _| 0.0);
        assert!(welded_edges_closed(&mb, 0..mb.idx.len()));
        // Signed volume of a closed outward mesh is positive.
        let vol: f32 = mb.idx.chunks_exact(3).map(|t| mb.pos[t[0] as usize].dot(mb.pos[t[1] as usize].cross(mb.pos[t[2] as usize])) / 6.0).sum();
        assert!(vol > 0.0, "volume {vol}");
    }
}
