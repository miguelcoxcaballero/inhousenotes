//! Branch skeletons: polylines with radii.
//!
//! Three builders share one representation ([`Stem`]):
//!
//! * [`parametric`]: Weber-Penn style recursive levels (count, length
//!   ratio, crown shape, down/rotate angles, curvature and tropism per
//!   level), optional envelope pruning, lifted crowns and pollarding.
//! * [`colonization`]: Runions et al. space colonization of the crown
//!   envelope from the top of a parametric trunk, then parametric twigs.
//! * palms live in [`crate::palm`] and only use the trunk stem.
//!
//! Radii come from the pipe model ([`pipe_radii`]): a stem's cross-section
//! at any point carries the summed "pipes" of everything above it, so
//! limbs taper exactly where children leave.

use std::collections::HashMap;

use forja_math::{Quat, Vec3};

use crate::Pruning;
use crate::rng::{Rng, vec_noise};
use crate::species::{Envelope, LevelDef, Species};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StemKind {
    Trunk,
    Branch,
    /// Surface root (half buried, plank-like for buttressed species).
    Root,
    /// Hanging aerial root or prop-root pillar (ficus).
    Aerial,
}

#[derive(Clone, Debug)]
pub struct Stem {
    pub kind: StemKind,
    pub level: u8,
    pub pts: Vec<Vec3>,
    pub radius: Vec<f32>,
    pub parent: Option<usize>,
    /// Fractional point index on the parent where this stem starts.
    pub at: f32,
    /// Radius set by the builder, not by the pipe model.
    pub explicit: bool,
    pub leafy: bool,
    /// Pollard knuckle at the tip (× radius).
    pub knob: f32,
    pub dead: bool,
    /// Vertical stretch of the cross-section (1 = round, >1 = plank).
    pub aspect: f32,
}

impl Stem {
    pub fn new(kind: StemKind, level: u8, pts: Vec<Vec3>, parent: Option<usize>, at: f32) -> Self {
        Stem { kind, level, pts, radius: Vec::new(), parent, at, explicit: false, leafy: false, knob: 0.0, dead: false, aspect: 1.0 }
    }

    pub fn length(&self) -> f32 {
        self.pts.windows(2).map(|w| (w[1] - w[0]).length()).sum()
    }

    /// Position, direction and fractional point index at arc length `s`.
    pub fn sample(&self, s: f32) -> (Vec3, Vec3, f32) {
        sample(&self.pts, s)
    }
}

pub fn sample(pts: &[Vec3], s: f32) -> (Vec3, Vec3, f32) {
    let mut acc = 0.0;
    for i in 0..pts.len().saturating_sub(1) {
        let d = pts[i + 1] - pts[i];
        let l = d.length();
        if acc + l >= s || i + 2 == pts.len() {
            let t = if l > 1e-6 { ((s - acc) / l).clamp(0.0, 1.0) } else { 0.0 };
            return (pts[i] + d * t, d.normalize_or(Vec3::Y), i as f32 + t);
        }
        acc += l;
    }
    (pts.first().copied().unwrap_or(Vec3::ZERO), Vec3::Y, 0.0)
}

/// A unit vector perpendicular to `d` (stable).
pub fn perp(d: Vec3) -> Vec3 {
    let a = if d.y.abs() < 0.9 { Vec3::Y } else { Vec3::X };
    d.cross(a).normalize_or(Vec3::Z)
}

pub fn rotate(v: Vec3, axis: Vec3, angle: f32) -> Vec3 {
    Quat::from_axis_angle(axis.normalize_or(Vec3::X), angle) * v
}

#[derive(Clone, Debug)]
pub struct Skeleton {
    pub stems: Vec<Stem>,
    pub height: f32,
    pub trunk_radius: f32,
    pub envelope: Env,
}

/// Resolved generation inputs.
#[derive(Clone, Debug)]
pub struct Growth {
    pub height: f32,
    pub trunk_radius: f32,
    pub age: f32,
    pub health: f32,
    pub pruning: Pruning,
    /// Clear height under the crown for lifted street trees (m).
    pub clearance: f32,
}

/// Crown envelope in tree space.
#[derive(Clone, Copy, Debug)]
pub struct Env {
    pub kind: Envelope,
    pub center: Vec3,
    pub radii: Vec3,
    pub base: f32,
    pub top: f32,
}

impl Env {
    pub fn new(kind: Envelope, height: f32, width: f32, base: f32) -> Self {
        let (b, t) = (height * base, height);
        let rx = (width * height * 0.5).max(0.3);
        let (center, radii) = match kind {
            Envelope::Sphere => (Vec3::new(0.0, t - rx, 0.0), Vec3::splat(rx)),
            Envelope::Umbrella => {
                let ry = ((t - b) * 0.5).max(0.4);
                (Vec3::new(0.0, t - ry, 0.0), Vec3::new(rx, ry, rx))
            }
            _ => (Vec3::new(0.0, (b + t) * 0.5, 0.0), Vec3::new(rx, ((t - b) * 0.5).max(0.3), rx)),
        };
        Env { kind, center, radii, base: b, top: t }
    }

    /// Normalised distance: < 1 inside.
    pub fn dist(&self, p: Vec3) -> f32 {
        let q = p - self.center;
        let rxz = (q.x * q.x + q.z * q.z).sqrt() / self.radii.x;
        let y = q.y / self.radii.y;
        match self.kind {
            Envelope::Ellipsoid | Envelope::Sphere => (rxz * rxz + y * y).sqrt(),
            Envelope::Spreading => (rxz.powf(2.4) + y.abs().powf(2.0)).powf(1.0 / 2.2),
            Envelope::Umbrella => (rxz.powf(2.6) + y.abs().powf(2.2)).powf(1.0 / 2.4),
            Envelope::Cone => {
                let t = ((p.y - self.base) / (self.top - self.base)).clamp(-0.5, 1.5);
                if !(0.0..=1.0).contains(&t) {
                    return 1.0 + t.abs().min((t - 1.0).abs());
                }
                let r = (1.0 - t).powf(0.9) * self.radii.x + 0.02;
                (q.x * q.x + q.z * q.z).sqrt() / r
            }
            Envelope::Column => {
                let t = (p.y - self.base) / (self.top - self.base);
                if !(0.0..=1.0).contains(&t) {
                    return 1.0 + (t - t.clamp(0.0, 1.0)).abs() * 4.0;
                }
                // Flame: fullest a third of the way up, pointed top.
                let r = self.radii.x * ((std::f32::consts::PI * t.powf(0.72)).sin().max(0.0)).powf(0.55) + 0.02;
                (q.x * q.x + q.z * q.z).sqrt() / r
            }
        }
    }

    /// Outward normal-ish direction of the envelope at `p` (for bent normals).
    pub fn normal(&self, p: Vec3) -> Vec3 {
        let q = (p - self.center) / self.radii.max(Vec3::splat(0.1));
        let mut n = Vec3::new(q.x, q.y, q.z);
        if matches!(self.kind, Envelope::Column | Envelope::Cone) {
            n.y *= 0.35;
        }
        (n + Vec3::Y * 0.15).normalize_or(Vec3::Y)
    }

    pub fn sample_inside(&self, rng: &mut Rng) -> Vec3 {
        for _ in 0..64 {
            let p = self.center + Vec3::new(rng.signed() * self.radii.x * 1.05, rng.signed() * self.radii.y * 1.05, rng.signed() * self.radii.z * 1.05);
            let p = if matches!(self.kind, Envelope::Cone | Envelope::Column) {
                Vec3::new(rng.signed() * self.radii.x, rng.range(self.base, self.top), rng.signed() * self.radii.x)
            } else {
                p
            };
            if self.dist(p) <= 1.0 && p.y >= self.base {
                return p;
            }
        }
        self.center
    }
}

/// Grows one stem from `start` along `dir`.
#[allow(clippy::too_many_arguments)]
pub fn grow(start: Vec3, dir: Vec3, length: f32, segments: u32, curve: f32, curve_var: f32, gravity: f32, rng: &mut Rng) -> Vec<Vec3> {
    let n = segments.max(1) as usize;
    let seg = length / n as f32;
    let mut d = dir.normalize_or(Vec3::Y);
    let mut p = start;
    let mut pts = Vec::with_capacity(n + 1);
    pts.push(p);
    let wiggle_axis0 = perp(d);
    for i in 0..n {
        // Curve: rotate toward +Y in the stem's vertical plane.
        let mut axis = d.cross(Vec3::Y);
        if axis.length_squared() < 1e-6 {
            axis = wiggle_axis0;
        }
        let c = curve.to_radians() / n as f32;
        d = rotate(d, axis, -c);
        if curve_var > 0.0 {
            let a = rotate(perp(d), d, rng.f32() * std::f32::consts::TAU);
            d = rotate(d, a, (curve_var * rng.normal() * 0.6).to_radians());
        }
        if gravity != 0.0 {
            d = (d + Vec3::Y * gravity * (seg / length.max(0.1)).min(0.5) * 2.0).normalize_or(d);
        }
        // Slightly shorter first segments keep child bases tidy.
        let s = if i == 0 { seg * 0.85 } else { seg };
        p += d * s;
        pts.push(p);
    }
    pts
}

fn age_count(age: f32) -> f32 {
    age.clamp(0.1, 1.6).powf(0.6)
}

/// Weber-Penn style recursive branching.
pub fn parametric(sp: &Species, g: &Growth, rng: &mut Rng) -> Skeleton {
    let env = Env::new(sp.crown.envelope, g.height, sp.crown.width, sp.crown.base);
    let mut stems: Vec<Stem> = Vec::new();
    if g.pruning == Pruning::Pollard
        && let Some(pd) = &sp.pollard
    {
        pollard(sp, pd, g, rng, &mut stems);
    } else {
        let leaders = trunks(sp, g, rng, &mut stems);
        let mut b = Brancher { sp, g, env, max_len: level_lengths(sp, g.height) };
        for t in leaders {
            b.children(&mut stems, t, 0, rng);
        }
    }
    finish(sp, g, &mut stems, rng);
    Skeleton { stems, height: g.height, trunk_radius: g.trunk_radius, envelope: env }
}

/// Nominal length of each level's stems (for child counts).
fn level_lengths(sp: &Species, h: f32) -> Vec<f32> {
    let mut v = vec![h * sp.trunk.length];
    for (i, l) in sp.levels.iter().enumerate() {
        let parent = v[i];
        v.push(if i == 0 { h * l.length } else { parent * l.length });
    }
    v
}

/// Trunk stem(s) and codominant leaders; returns the stems that carry level 1.
fn trunks(sp: &Species, g: &Growth, rng: &mut Rng, stems: &mut Vec<Stem>) -> Vec<usize> {
    let t = &sp.trunk;
    let n_stems = t.stems.max(1);
    let len = g.height * t.length;
    let mut carriers = Vec::new();
    let lean_az = rng.f32() * std::f32::consts::TAU;
    for k in 0..n_stems {
        let az = lean_az + k as f32 / n_stems as f32 * std::f32::consts::TAU + rng.signed() * 0.4;
        let lean = if n_stems > 1 { t.stem_spread + rng.signed() * 6.0 } else { t.lean + rng.signed() * t.lean_var };
        let horiz = Vec3::new(az.cos(), 0.0, az.sin());
        let dir = rotate(Vec3::Y, horiz.cross(Vec3::Y), -lean.to_radians());
        let base = if n_stems > 1 { horiz * g.trunk_radius * 0.55 } else { Vec3::ZERO } + Vec3::new(0.0, -0.3, 0.0);
        let stem_len = if n_stems > 1 { len * rng.range(0.75, 1.0) } else { len };
        let split = t.splits > 0;
        let main_len = if split { stem_len * t.split_at } else { stem_len };
        let segs = ((t.segments as f32 * main_len / len.max(0.1)).ceil() as u32).max(4);
        let mut pts = grow(base, dir, main_len + 0.3, segs, t.curve, t.curve_var, 0.0, rng);
        gnarl(&mut pts, g.trunk_radius * t.gnarl, rng.next_u64() as u32);
        let ti = stems.len();
        stems.push(Stem::new(StemKind::Trunk, 0, pts, None, 0.0));
        if split {
            let rest = stem_len - main_len;
            let n = t.splits;
            let end = *stems[ti].pts.last().unwrap();
            let d0 = (stems[ti].pts[stems[ti].pts.len() - 1] - stems[ti].pts[stems[ti].pts.len() - 2]).normalize_or(Vec3::Y);
            let az0 = rng.f32() * std::f32::consts::TAU;
            let at = (stems[ti].pts.len() - 1) as f32;
            for j in 0..n {
                let a = az0 + j as f32 / n as f32 * std::f32::consts::TAU + rng.signed() * 0.3;
                let side = rotate(perp(d0), d0, a);
                let dir = rotate(d0, side.cross(d0), -(t.split_angle + rng.signed() * 8.0).to_radians());
                let segs = ((t.segments as f32 * rest / len.max(0.1)).ceil() as u32).max(4);
                let mut pts = grow(end, dir, rest * rng.range(0.8, 1.0), segs, t.curve * 0.5 + 10.0, t.curve_var + 2.0, 0.15, rng);
                gnarl(&mut pts, g.trunk_radius * t.gnarl * 0.7, rng.next_u64() as u32);
                let li = stems.len();
                stems.push(Stem::new(StemKind::Trunk, 0, pts, Some(ti), at));
                carriers.push(li);
            }
            carriers.push(ti);
        } else {
            carriers.push(ti);
        }
    }
    carriers
}

/// Horizontal axis wander, anchored at the base.
fn gnarl(pts: &mut [Vec3], amp: f32, seed: u32) {
    if amp <= 0.0 {
        return;
    }
    for p in pts.iter_mut() {
        let h = (p.y + 0.3).max(0.0);
        let w = (h / 1.2).min(1.0);
        let n = vec_noise(Vec3::new(0.0, p.y * 0.45, 0.0) + Vec3::splat(seed as f32 * 0.001), seed);
        p.x += n.x * amp * w * 2.0;
        p.z += n.z * amp * w * 2.0;
    }
}

struct Brancher<'a> {
    sp: &'a Species,
    g: &'a Growth,
    env: Env,
    max_len: Vec<f32>,
}

const MAX_STEMS: usize = 9000;

impl Brancher<'_> {
    /// Adds level `li + 1` children (definition `levels[li]`) to `parent`.
    fn children(&mut self, stems: &mut Vec<Stem>, parent: usize, li: usize, rng: &mut Rng) {
        let Some(ld) = self.sp.levels.get(li) else { return };
        let ld: &LevelDef = ld;
        let plen = stems[parent].length();
        if plen < 0.05 || stems.len() > MAX_STEMS {
            return;
        }
        let on_trunk = stems[parent].kind == StemKind::Trunk;
        let rel = (plen / self.max_len[li].max(0.05)).clamp(0.12, 1.4);
        let mut count = ld.count + ld.count_var * rng.signed();
        count *= if li == 0 { age_count(self.g.age) } else { rel };
        if on_trunk && stems[parent].parent.is_some() {
            // Codominant leaders share the crown.
            count *= 0.7;
        }
        let count = count.round().max(0.0) as usize;
        if count == 0 {
            return;
        }
        let whorl = ld.whorl.max(1) as usize;
        let nodes = count.div_ceil(whorl);
        let mut s0 = ld.start * plen;
        let s1 = ld.end.min(1.0) * plen;
        if on_trunk {
            // Crown base: bare trunk fraction and street clearance.
            let base_y = stems[parent].pts[0].y;
            let need = (self.sp.trunk.bare * self.g.height).max(self.g.clearance) - base_y.max(0.0);
            let mut acc = 0.0;
            let pts = &stems[parent].pts;
            for w in pts.windows(2) {
                if w[0].y >= need + base_y.min(0.0) {
                    break;
                }
                acc += (w[1] - w[0]).length();
            }
            if stems[parent].parent.is_none() {
                s0 = s0.max(acc);
            }
        }
        if s1 <= s0 {
            return;
        }
        let mut phi = rng.f32() * std::f32::consts::TAU;
        let crown_lo = self.env.base;
        let crown_hi = self.g.height;
        for k in 0..nodes {
            let s = s0 + (s1 - s0) * ((k as f32 + 0.5 + rng.signed() * 0.35) / nodes as f32).clamp(0.0, 1.0);
            let (pos, pdir, at) = stems[parent].sample(s);
            for w in 0..whorl {
                if stems.len() > MAX_STEMS {
                    return;
                }
                let t_rel = s / plen;
                if ld.rotate < 0.0 {
                    // Distichous: alternate sides in the horizontal plane.
                    phi += std::f32::consts::PI + (ld.rotate_var * rng.signed()).to_radians();
                } else {
                    phi += (ld.rotate + ld.rotate_var * rng.signed()).to_radians();
                }
                let az = phi + w as f32 * std::f32::consts::TAU / whorl as f32;
                let mut length = if li == 0 {
                    let x = ((pos.y - crown_lo) / (crown_hi - crown_lo).max(0.1)).clamp(0.0, 1.0);
                    self.g.height * ld.length * ld.shape.ratio(x)
                } else {
                    ld.length * (plen - 0.55 * s).max(plen * 0.15)
                };
                length *= 1.0 + ld.length_var * rng.signed();
                if length < 0.04 {
                    continue;
                }
                let down = (ld.down + (ld.down_end - ld.down) * t_rel + ld.down_var * rng.signed()).to_radians();
                let side = horizontal_side(pdir, az);
                let dir = (pdir * down.cos() + side * down.sin()).normalize_or(side);
                let segs = ld.segments.max(1);
                let mut pts = grow(pos, dir, length, segs, ld.curve, ld.curve_var, ld.gravity, rng);
                if !self.keep(&mut pts) {
                    continue;
                }
                let ci = stems.len();
                stems.push(Stem::new(StemKind::Branch, (li + 1) as u8, pts, Some(parent), at));
                self.children(stems, ci, li + 1, rng);
            }
        }
    }

    /// Applies pruning; false drops the stem.
    fn keep(&self, pts: &mut Vec<Vec3>) -> bool {
        let prune_env = self.sp.crown.prune || self.g.pruning == Pruning::Lollipop;
        if prune_env {
            if self.env.dist(pts[0]) > 1.08 {
                return false;
            }
            if let Some(i) = pts.iter().position(|p| self.env.dist(*p) > 1.0) {
                pts.truncate(i.max(1) + 1);
                if pts.len() >= 2 {
                    let a = pts[pts.len() - 2];
                    let b = pts[pts.len() - 1];
                    // Cut back to the envelope surface.
                    let mut t = 1.0;
                    for _ in 0..8 {
                        let m = a + (b - a) * t;
                        if self.env.dist(m) > 1.0 {
                            t *= 0.7;
                        }
                    }
                    let last = pts.len() - 1;
                    pts[last] = a + (b - a) * t;
                }
            }
        }
        if self.g.clearance > 0.0 && matches!(self.g.pruning, Pruning::Lifted | Pruning::Pollard) {
            // Street trees: nothing hangs into the clear height.
            if let Some(i) = pts.iter().position(|p| p.y < self.g.clearance - 0.2) {
                if i <= 1 {
                    return false;
                }
                pts.truncate(i);
            }
        }
        pts.len() >= 2 && (pts[pts.len() - 1] - pts[0]).length() > 0.03
    }
}

/// Direction perpendicular to `d` at azimuth `az`, measured so that for a
/// vertical parent `az` is a compass angle.
fn horizontal_side(d: Vec3, az: f32) -> Vec3 {
    let r = Vec3::new(az.cos(), 0.0, az.sin());
    let s = r - d * d.dot(r);
    if s.length_squared() > 1e-4 { s.normalize() } else { rotate(perp(d), d, az) }
}

/// Pollarded street plane: trunk, scaffold limbs ending in knuckles, a
/// crown of straight one-year sprouts.
fn pollard(sp: &Species, pd: &crate::species::PollardDef, g: &Growth, rng: &mut Rng, stems: &mut Vec<Stem>) {
    let scale = (g.height / sp.height[1]).clamp(0.4, 1.3);
    let th = pd.height * scale.sqrt();
    let lean_az = rng.f32() * std::f32::consts::TAU;
    let dir = rotate(Vec3::Y, Vec3::new(lean_az.cos(), 0.0, lean_az.sin()), (rng.signed() * 3.0).to_radians());
    let mut pts = grow(Vec3::new(0.0, -0.3, 0.0), dir, th + 0.3, 10, 0.0, 1.5, 0.0, rng);
    gnarl(&mut pts, g.trunk_radius * sp.trunk.gnarl, rng.next_u64() as u32);
    stems.push(Stem::new(StemKind::Trunk, 0, pts, None, 0.0));
    let top = *stems[0].pts.last().unwrap();
    let at = (stems[0].pts.len() - 1) as f32;
    let n = pd.limbs.max(1) + rng.below(2) as u32;
    let az0 = rng.f32() * std::f32::consts::TAU;
    let sprout_def = sp.levels.last().cloned().unwrap_or_default();
    for i in 0..n {
        let az = az0 + i as f32 / n as f32 * std::f32::consts::TAU + rng.signed() * 0.35;
        let side = Vec3::new(az.cos(), 0.0, az.sin());
        let ang = (pd.limb_angle + rng.signed() * 8.0).to_radians();
        let ldir = (Vec3::Y * ang.cos() + side * ang.sin()).normalize();
        let len = pd.limb_length * scale * rng.range(0.75, 1.15);
        let pts = grow(top - Vec3::Y * 0.25, ldir, len, 6, 8.0, 5.0, 0.1, rng);
        let li = stems.len();
        let mut limb = Stem::new(StemKind::Branch, 1, pts, Some(0), at - 0.6);
        limb.knob = pd.knuckle;
        stems.push(limb);
        // A secondary knuckle on some limbs.
        let mut heads = vec![(li, stems[li].length())];
        if rng.chance(0.6) {
            let l = stems[li].length();
            let s = l * rng.range(0.45, 0.65);
            let (p, d, at2) = stems[li].sample(s);
            let sd = horizontal_side(d, az + rng.signed() * 1.2);
            let pts = grow(p, (d + sd * 0.9).normalize(), l * rng.range(0.25, 0.4), 3, 10.0, 4.0, 0.2, rng);
            let k = stems.len();
            let mut st = Stem::new(StemKind::Branch, 1, pts, Some(li), at2);
            st.knob = pd.knuckle * 0.85;
            stems.push(st);
            heads.push((k, stems[k].length()));
        }
        for (h, hl) in heads {
            let (p, d, _) = stems[h].sample(hl);
            let at_tip = (stems[h].pts.len() - 1) as f32;
            let count = (pd.sprouts as f32 * rng.range(0.7, 1.3) * g.age.clamp(0.3, 1.2)).round() as usize;
            for _ in 0..count {
                if stems.len() > MAX_STEMS {
                    break;
                }
                let az = rng.f32() * std::f32::consts::TAU;
                let cone = rng.range(5.0, 38.0).to_radians();
                let up = (Vec3::Y * 0.75 + d * 0.25).normalize();
                let sd = horizontal_side(up, az);
                let sdir = (up * cone.cos() + sd * cone.sin()).normalize();
                let sl = pd.sprout_length * scale.sqrt() * rng.range(0.45, 1.2);
                let pts = grow(p + sdir * 0.05, sdir, sl, 5, -6.0, 3.0, 0.05, rng);
                let si = stems.len();
                stems.push(Stem::new(StemKind::Branch, 2, pts, Some(h), at_tip));
                // Short leafy twigs along the sprout.
                let twig = LevelDef { count: sprout_def.count * 0.6, start: 0.35, length: sprout_def.length.min(0.35), ..sprout_def.clone() };
                let mut b = Brancher { sp, g, env: Env::new(Envelope::Ellipsoid, g.height, 1.5, 0.0), max_len: vec![1.0, 1.0, sl, sl * 0.3] };
                let lvl = LevelDefOverride(twig);
                b.children_with(stems, si, &lvl.0, 3, rng);
            }
        }
    }
}

struct LevelDefOverride(LevelDef);

impl Brancher<'_> {
    /// Children of `parent` with an explicit level definition (no recursion).
    fn children_with(&mut self, stems: &mut Vec<Stem>, parent: usize, ld: &LevelDef, level: u8, rng: &mut Rng) {
        let plen = stems[parent].length();
        let count = ((ld.count + ld.count_var * rng.signed()) * (plen / 2.0).clamp(0.3, 1.5)).round().max(0.0) as usize;
        let mut phi = rng.f32() * std::f32::consts::TAU;
        for k in 0..count {
            let s = plen * (ld.start + (1.0 - ld.start) * (k as f32 + 0.5 + 0.3 * rng.signed()) / count as f32);
            let (pos, pdir, at) = stems[parent].sample(s);
            phi += (ld.rotate + ld.rotate_var * rng.signed()).to_radians();
            let down = (ld.down + ld.down_var * rng.signed()).to_radians();
            let side = horizontal_side(pdir, phi);
            let dir = (pdir * down.cos() + side * down.sin()).normalize_or(side);
            let len = ld.length * (plen - 0.5 * s).max(0.1) * (1.0 + ld.length_var * rng.signed());
            let pts = grow(pos, dir, len.max(0.05), ld.segments.max(1), ld.curve, ld.curve_var, ld.gravity, rng);
            stems.push(Stem::new(StemKind::Branch, level, pts, Some(parent), at));
        }
    }
}

/// Space colonization crown on a parametric trunk.
pub fn colonization(sp: &Species, g: &Growth, rng: &mut Rng) -> Skeleton {
    let cd = sp.colonization.clone().unwrap_or_default();
    let env = Env::new(sp.crown.envelope, g.height, sp.crown.width, sp.crown.base);
    let mut stems: Vec<Stem> = Vec::new();
    let leaders = trunks(sp, g, rng, &mut stems);
    let scale = (g.height / sp.height[1].max(1.0)).clamp(0.3, 1.5);
    let step = cd.step * scale.sqrt();
    let influence = cd.influence * scale;
    let kill = cd.kill * scale;
    // Attractors.
    let n_attr = (cd.attractors as f32 * age_count(g.age)).round() as usize;
    let mut attractors: Vec<Vec3> = (0..n_attr).map(|_| env.sample_inside(rng)).filter(|p| p.y > g.clearance + 0.5).collect();
    // Nodes: (pos, parent node, trunk attachment).
    struct Node {
        pos: Vec3,
        parent: Option<usize>,
        trunk: Option<(usize, f32)>,
    }
    let mut nodes: Vec<Node> = Vec::new();
    for &t in &leaders {
        let pts = &stems[t].pts;
        let len = stems[t].length();
        let k0 = (cd.start * pts.len() as f32) as usize;
        let mut acc = 0.0;
        for i in 0..pts.len() {
            if i > 0 {
                acc += (pts[i] - pts[i - 1]).length();
            }
            if i >= k0.min(pts.len() - 1) && pts[i].y > g.clearance && acc <= len {
                nodes.push(Node { pos: pts[i], parent: None, trunk: Some((t, i as f32)) });
            }
        }
    }
    let cell = influence.max(0.5);
    let key = |p: Vec3| ((p.x / cell).floor() as i32, (p.y / cell).floor() as i32, (p.z / cell).floor() as i32);
    let mut grid: HashMap<(i32, i32, i32), Vec<usize>> = HashMap::new();
    for (i, n) in nodes.iter().enumerate() {
        grid.entry(key(n.pos)).or_default().push(i);
    }
    let mut seed_noise = rng.next_u64() as u32;
    for _ in 0..cd.iterations {
        if attractors.is_empty() || nodes.len() > 4500 {
            break;
        }
        let mut pull: HashMap<usize, (Vec3, u32)> = HashMap::new();
        for a in &attractors {
            let (kx, ky, kz) = key(*a);
            let mut best = (influence * influence, usize::MAX);
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        if let Some(v) = grid.get(&(kx + dx, ky + dy, kz + dz)) {
                            for &ni in v {
                                let d2 = (nodes[ni].pos - *a).length_squared();
                                if d2 < best.0 {
                                    best = (d2, ni);
                                }
                            }
                        }
                    }
                }
            }
            if best.1 != usize::MAX {
                let e = pull.entry(best.1).or_insert((Vec3::ZERO, 0));
                e.0 += (*a - nodes[best.1].pos).normalize_or(Vec3::Y);
                e.1 += 1;
            }
        }
        if pull.is_empty() {
            break;
        }
        let mut grown: Vec<(usize, Vec3)> = pull.into_iter().map(|(k, v)| (k, v.0)).collect();
        grown.sort_by_key(|(k, _)| *k);
        let mut added = 0;
        for (ni, d) in grown {
            seed_noise = seed_noise.wrapping_add(1);
            let jitter = vec_noise(nodes[ni].pos * 0.7, seed_noise & 0xff) * 0.25;
            let dir = (d.normalize_or(Vec3::Y) + Vec3::Y * cd.tropism + jitter).normalize_or(Vec3::Y);
            let p = nodes[ni].pos + dir * step;
            // Avoid doubling back onto an existing node.
            if nodes[ni].parent.is_some_and(|pp| (nodes[pp].pos - p).length() < step * 0.5) {
                continue;
            }
            let id = nodes.len();
            nodes.push(Node { pos: p, parent: Some(ni), trunk: None });
            grid.entry(key(p)).or_default().push(id);
            added += 1;
        }
        if added == 0 {
            break;
        }
        attractors.retain(|a| {
            let (kx, ky, kz) = key(*a);
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        if let Some(v) = grid.get(&(kx + dx, ky + dy, kz + dz)) {
                            for &ni in v.iter().rev().take(64) {
                                if (nodes[ni].pos - *a).length_squared() < kill * kill {
                                    return false;
                                }
                            }
                        }
                    }
                }
            }
            true
        });
    }
    // Node tree to stems: follow the heaviest child as the continuation.
    let n = nodes.len();
    let mut kids: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, nd) in nodes.iter().enumerate() {
        if let Some(p) = nd.parent {
            kids[p].push(i);
        }
    }
    let mut weight = vec![1usize; n];
    for i in (0..n).rev() {
        if let Some(p) = nodes[i].parent {
            weight[p] += weight[i];
        }
    }
    // Start chains at children of trunk nodes.
    let mut queue: Vec<(usize, usize, f32, u8)> = Vec::new(); // (first node, parent stem, at, level)
    for i in 0..n {
        if let Some((t, at)) = nodes[i].trunk {
            for &c in &kids[i] {
                queue.push((c, t, at, 1));
            }
        }
    }
    let mut stem_of_node: HashMap<usize, (usize, usize)> = HashMap::new();
    let mut qi = 0;
    while qi < queue.len() {
        let (first, pstem, at, level) = queue[qi];
        qi += 1;
        if stems.len() > MAX_STEMS {
            break;
        }
        let start = nodes[first].parent.map(|p| nodes[p].pos).unwrap_or(nodes[first].pos);
        let mut pts = vec![start];
        let si = stems.len();
        let mut cur = first;
        loop {
            pts.push(nodes[cur].pos);
            stem_of_node.insert(cur, (si, pts.len() - 1));
            let mut ks = kids[cur].clone();
            if ks.is_empty() {
                break;
            }
            ks.sort_by_key(|k| std::cmp::Reverse(weight[*k]));
            for &o in &ks[1..] {
                queue.push((o, si, (pts.len() - 1) as f32, level.saturating_add(1).min(4)));
            }
            cur = ks[0];
        }
        if pts.len() < 3 && level > 1 {
            // A one-node side shoot: leave it to the twigs.
            continue;
        }
        smooth(&mut pts, 2);
        stems.push(Stem::new(StemKind::Branch, level, pts, Some(pstem), at));
    }
    // Parametric twigs on top.
    if let Some(twig) = sp.levels.last() {
        let mut b = Brancher { sp, g, env, max_len: vec![1.0; 6] };
        let count = stems.len();
        for si in 0..count {
            if stems[si].kind == StemKind::Branch && stems[si].length() > 0.3 {
                let lv = stems[si].level + 1;
                b.children_with(&mut stems, si, twig, lv.max(2), rng);
            }
        }
    }
    finish(sp, g, &mut stems, rng);
    Skeleton { stems, height: g.height, trunk_radius: g.trunk_radius, envelope: env }
}

fn smooth(pts: &mut [Vec3], iters: usize) {
    for _ in 0..iters {
        let prev = pts.to_vec();
        for i in 1..pts.len().saturating_sub(1) {
            pts[i] = prev[i] * 0.5 + (prev[i - 1] + prev[i + 1]) * 0.25;
        }
    }
}

/// Leaf flags, dead branches, roots, aerial roots and radii.
fn finish(sp: &Species, g: &Growth, stems: &mut Vec<Stem>, rng: &mut Rng) {
    let n = stems.len();
    let mut has_kids = vec![false; n];
    for s in stems.iter() {
        if let Some(p) = s.parent {
            has_kids[p] = true;
        }
    }
    let max_level = stems.iter().map(|s| s.level).max().unwrap_or(0);
    let leafy_from = max_level.saturating_sub(sp.leaves.on_levels.max(1) as u8 - 1);
    for (i, s) in stems.iter_mut().enumerate() {
        if s.kind == StemKind::Branch && (s.level >= leafy_from || !has_kids[i]) {
            s.leafy = true;
        }
    }
    // Dead limbs on unhealthy trees.
    let dead_p = (1.0 - g.health).clamp(0.0, 1.0) * 0.35;
    if dead_p > 0.0 {
        for i in 0..n {
            let dead_parent = stems[i].parent.is_some_and(|p| stems[p].dead);
            if dead_parent || (stems[i].level == 2 && rng.chance(dead_p)) {
                stems[i].dead = true;
                stems[i].leafy = false;
            }
        }
    }
    roots(sp, g, stems, rng);
    aerial_roots(sp, g, stems, rng);
    pipe_radii(stems, g.trunk_radius, 2.3);
}

fn roots(sp: &Species, g: &Growth, stems: &mut Vec<Stem>, rng: &mut Rng) {
    let rd = &sp.roots;
    if rd.count == 0 {
        return;
    }
    let scale = (g.height / sp.height[1].max(1.0)).clamp(0.3, 1.5) * g.age.clamp(0.3, 1.3);
    let az0 = rng.f32() * std::f32::consts::TAU;
    let buttress = sp.trunk.buttress > 0.0;
    for k in 0..rd.count {
        let az = az0 + (k as f32 + rng.signed() * 0.3) / rd.count as f32 * std::f32::consts::TAU;
        let len = rd.length * scale * rng.range(0.6, 1.3);
        let r0 = g.trunk_radius * rd.radius * rng.range(0.7, 1.2);
        let segs = 10;
        let mut pts = Vec::new();
        let mut radius = Vec::new();
        let mut d = Vec3::new(az.cos(), 0.0, az.sin());
        let mut p = Vec3::new(0.0, 0.0, 0.0) + d * g.trunk_radius * 0.4;
        for i in 0..=segs {
            let t = i as f32 / segs as f32;
            let r = r0 * (1.0 - t).powf(1.4) + 0.012;
            // Sink as it goes: the base sits on the flare, the tip dives.
            let y = r * if buttress { 0.1 } else { 0.25 } - t * t * r0 * 0.9;
            pts.push(Vec3::new(p.x, y, p.z));
            radius.push(r);
            d = rotate(d, Vec3::Y, rng.signed() * 0.35);
            p += d * len / segs as f32;
        }
        let mut s = Stem::new(StemKind::Root, 1, pts, Some(0), 0.0);
        s.radius = radius;
        s.explicit = true;
        s.aspect = if buttress { 2.2 } else { 1.15 };
        stems.push(s);
    }
}

fn aerial_roots(sp: &Species, g: &Growth, stems: &mut Vec<Stem>, rng: &mut Rng) {
    let rd = &sp.roots;
    if rd.aerial <= 0.0 {
        return;
    }
    let n = stems.len();
    for i in 0..n {
        let s = &stems[i];
        if s.kind != StemKind::Branch || s.level > 2 {
            continue;
        }
        let pts = s.pts.clone();
        let mut acc = 0.0;
        for w in 0..pts.len().saturating_sub(1) {
            let seg = pts[w + 1] - pts[w];
            let l = seg.length();
            acc += l;
            let horizontal = seg.y.abs() < l * 0.55;
            let near = (pts[w].x * pts[w].x + pts[w].z * pts[w].z).sqrt() < g.height * 0.28;
            if !horizontal || !near || pts[w].y < 2.5 || acc < 1.0 {
                continue;
            }
            if !rng.chance((rd.aerial * l).min(0.9)) {
                continue;
            }
            let top = pts[w] + seg * rng.f32();
            let pillar = rng.chance(rd.pillars);
            let bottom = if pillar { -0.15 } else { top.y - rng.range(0.6, 3.5).min(top.y * 0.6) };
            let segs = ((top.y - bottom) / 0.6).ceil().max(2.0) as usize;
            let mut rp = Vec::new();
            let mut rr = Vec::new();
            let r = if pillar { rng.range(0.1, 0.3) * (g.height / 18.0).clamp(0.5, 1.5) } else { rng.range(0.006, 0.02) };
            let sway = Vec3::new(rng.signed(), 0.0, rng.signed()) * 0.08;
            for k in 0..=segs {
                let t = k as f32 / segs as f32;
                let y = top.y + (bottom - top.y) * t;
                rp.push(Vec3::new(top.x, y, top.z) + sway * (t * 3.0).sin() + Vec3::new(rng.signed(), 0.0, rng.signed()) * 0.02);
                let flare = if pillar { 1.0 + 0.8 * ((t - 0.85) / 0.15).clamp(0.0, 1.0).powi(2) } else { 1.0 - 0.5 * t };
                rr.push(r * flare);
            }
            let mut st = Stem::new(StemKind::Aerial, 3, rp, Some(i), w as f32);
            st.radius = rr;
            st.explicit = true;
            stems.push(st);
        }
    }
}

/// Pipe model radii: `r^e` sums over the stems above each point.
pub fn pipe_radii(stems: &mut [Stem], trunk_radius: f32, e: f32) {
    let n = stems.len();
    let mut kids: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, s) in stems.iter().enumerate() {
        if let (Some(p), false) = (s.parent, s.explicit) {
            kids[p].push(i);
        }
    }
    let mut wprof: Vec<Vec<f32>> = vec![Vec::new(); n];
    let mut base_w = vec![0.0f32; n];
    for i in (0..n).rev() {
        if stems[i].explicit {
            continue;
        }
        let m = stems[i].pts.len();
        let tip = if stems[i].dead { 0.25 } else if stems[i].leafy { 1.0 } else { 0.5 };
        let mut w = vec![tip; m];
        for &c in &kids[i] {
            let at = stems[c].at.clamp(0.0, (m - 1) as f32);
            let cw = base_w[c];
            for (j, wj) in w.iter_mut().enumerate() {
                if (j as f32) <= at {
                    *wj += cw;
                } else if (j as f32) - 1.0 < at {
                    // The segment containing the fork: share smoothly.
                    *wj += cw * (1.0 - (j as f32 - at)).clamp(0.0, 1.0);
                }
            }
        }
        base_w[i] = w[0];
        wprof[i] = w;
    }
    let total: f32 = stems.iter().enumerate().filter(|(_, s)| s.kind == StemKind::Trunk && s.parent.is_none()).map(|(i, _)| base_w[i]).sum::<f32>().max(1e-3);
    let k = trunk_radius / total.powf(1.0 / e);
    for i in 0..n {
        if stems[i].explicit {
            continue;
        }
        let lvl = stems[i].level as usize;
        let min_r = [0.03, 0.012, 0.006, 0.004, 0.003][lvl.min(4)];
        let mut r: Vec<f32> = wprof[i].iter().map(|w| (k * w.powf(1.0 / e)).max(min_r)).collect();
        // Gentle extra taper toward the tip and a smoothed profile.
        let m = r.len();
        for (j, rj) in r.iter_mut().enumerate() {
            let t = j as f32 / (m - 1).max(1) as f32;
            *rj *= 1.0 - 0.18 * t;
        }
        let prev = r.clone();
        for j in 1..m.saturating_sub(1) {
            r[j] = (prev[j - 1] + 2.0 * prev[j] + prev[j + 1]) * 0.25;
        }
        if let Some(l) = r.last_mut() {
            *l = (*l * 0.6).max(min_r * 0.6);
        }
        // Children never outgrow the parent at the fork.
        if let Some(p) = stems[i].parent
            && !stems[p].radius.is_empty()
        {
            let pr = interp(&stems[p].radius, stems[i].at);
            let cap = pr * 0.9;
            if r[0] > cap {
                let s = cap / r[0];
                for v in &mut r {
                    *v *= s;
                }
            }
        }
        stems[i].radius = r;
    }
}

pub fn interp(v: &[f32], at: f32) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    let i = (at.floor() as usize).min(v.len() - 1);
    let j = (i + 1).min(v.len() - 1);
    let t = (at - i as f32).clamp(0.0, 1.0);
    v[i] + (v[j] - v[i]) * t
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::species::SpeciesLibrary;

    #[test]
    fn pipe_model_tapers_and_children_are_thinner() {
        let lib = SpeciesLibrary::builtin();
        let sp = lib.get("broadleaf").unwrap();
        let g = Growth { height: 12.0, trunk_radius: 0.3, age: 1.0, health: 1.0, pruning: Pruning::Natural, clearance: 0.0 };
        let sk = parametric(sp, &g, &mut Rng::new(3));
        assert!(sk.stems.len() > 50);
        let t = &sk.stems[0];
        assert!((t.radius[0] - 0.3).abs() < 0.05, "trunk base {}", t.radius[0]);
        assert!(t.radius.last().unwrap() < &t.radius[0]);
        for s in &sk.stems {
            if let (Some(p), false) = (s.parent, s.explicit) {
                assert!(s.radius[0] <= interp(&sk.stems[p].radius, s.at) + 1e-4);
            }
            assert!(s.radius.iter().all(|r| r.is_finite() && *r > 0.0));
        }
    }
}
