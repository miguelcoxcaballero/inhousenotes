//! Moulding profiles (cornices, string courses, plinths...) and opening
//! outlines (round, pointed, horseshoe, ogee... arches).

use std::f32::consts::{FRAC_PI_2, PI};

use forja_math::Vec2;

use super::spec::ArchKind;

/// Preset moulding profile names.
pub const PROFILES: &[&str] = &[
    "cornice", "classical", "gothic", "cyma", "ovolo", "cavetto", "torus", "fillet", "band", "chamfer", "plinth", "base", "string", "drip", "eave", "stepped", "art_deco", "modern", "slab", "corbel_table",
];

pub fn check_name(s: &str) -> Result<(), String> {
    let k = s.trim().to_ascii_lowercase();
    if PROFILES.contains(&k.as_str()) {
        Ok(())
    } else {
        Err(format!("unknown profile `{s}`{}; expected one of {} or [[out, up], ...]", super::parse::did_you_mean(&k, PROFILES), PROFILES.join(", ")))
    }
}

/// Builder walking a profile from the wall outward and upward.
struct P {
    pts: Vec<Vec2>,
}

impl P {
    fn new() -> Self {
        P { pts: vec![Vec2::ZERO] }
    }
    fn cur(&self) -> Vec2 {
        *self.pts.last().unwrap()
    }
    fn to(mut self, out: f32, up: f32) -> Self {
        self.pts.push(Vec2::new(out, up));
        self
    }
    fn by(self, dout: f32, dup: f32) -> Self {
        let c = self.cur();
        self.to(c.x + dout, c.y + dup)
    }
    /// Quarter round bulging outward-down (ovolo): out then up, convex.
    fn ovolo(mut self, w: f32, h: f32) -> Self {
        let c = self.cur();
        for k in 1..=5 {
            let a = k as f32 / 5.0 * FRAC_PI_2;
            self.pts.push(c + Vec2::new(w * a.sin(), h * (1.0 - a.cos())));
        }
        self
    }
    /// Concave quarter (cavetto): up then out.
    fn cavetto(mut self, w: f32, h: f32) -> Self {
        let c = self.cur();
        for k in 1..=5 {
            let a = k as f32 / 5.0 * FRAC_PI_2;
            self.pts.push(c + Vec2::new(w * (1.0 - a.cos()), h * a.sin()));
        }
        self
    }
    /// Cyma recta: concave below, convex above (outward S).
    fn cyma(self, w: f32, h: f32) -> Self {
        self.cavetto(w * 0.5, h * 0.5).ovolo(w * 0.5, h * 0.5)
    }
    fn close(mut self) -> Vec<Vec2> {
        let top = self.cur().y;
        self.pts.push(Vec2::new(0.0, top));
        self.pts
    }
}

/// Profile points (out, up), closed along the wall, height 1 (scaled by
/// the moulding size). Counter-clockwise in (out, up).
pub fn named(name: &str) -> Vec<Vec2> {
    match name.trim().to_ascii_lowercase().as_str() {
        "cornice" | "classical" => P::new()
            .to(0.04, 0.0)
            .by(0.0, 0.05)
            .ovolo(0.1, 0.1)
            .by(0.02, 0.0)
            .by(0.0, 0.12)
            .by(0.36, 0.0)
            .by(0.06, 0.03)
            .by(0.0, 0.2)
            .by(0.03, 0.0)
            .by(0.0, 0.04)
            .cyma(0.22, 0.32)
            .by(0.0, 0.05)
            .close(),
        "gothic" => P::new().to(0.05, 0.0).by(0.0, 0.12).cavetto(0.32, 0.42).by(0.04, 0.0).by(0.0, 0.16).to(0.12, 1.0).close(),
        "cyma" => P::new().to(0.05, 0.0).by(0.0, 0.1).cyma(0.4, 0.75).by(0.0, 0.15).close(),
        "ovolo" => P::new().to(0.04, 0.0).ovolo(0.4, 0.7).by(0.0, 0.3).close(),
        "cavetto" => P::new().to(0.04, 0.0).cavetto(0.45, 0.8).by(0.0, 0.2).close(),
        "torus" => {
            let mut v = vec![Vec2::ZERO];
            for k in 0..=10 {
                let a = -FRAC_PI_2 + k as f32 / 10.0 * PI;
                v.push(Vec2::new(0.02 + 0.48 * a.cos(), 0.5 + 0.5 * a.sin()));
            }
            v.push(Vec2::new(0.0, 1.0));
            v
        }
        "fillet" | "band" | "modern" | "slab" => vec![Vec2::ZERO, Vec2::new(0.3, 0.0), Vec2::new(0.3, 1.0), Vec2::new(0.0, 1.0)],
        "chamfer" => vec![Vec2::ZERO, Vec2::new(0.5, 0.0), Vec2::new(0.5, 0.45), Vec2::new(0.0, 1.0)],
        "plinth" | "base" => P::new().to(0.2, 0.0).by(0.0, 0.78).by(-0.06, 0.08).to(0.06, 1.0).close(),
        "string" => P::new().to(0.1, 0.0).by(0.03, 0.15).by(0.0, 0.5).by(-0.04, 0.35).close(),
        "drip" => P::new().to(0.32, -0.0).by(0.0, 0.35).to(0.2, 1.0).close(),
        "eave" => P::new().to(0.05, 0.0).to(0.85, 0.55).by(0.0, 0.45).close(),
        "stepped" | "art_deco" => vec![
            Vec2::ZERO,
            Vec2::new(0.1, 0.0),
            Vec2::new(0.1, 0.3),
            Vec2::new(0.2, 0.3),
            Vec2::new(0.2, 0.6),
            Vec2::new(0.3, 0.6),
            Vec2::new(0.3, 1.0),
            Vec2::new(0.0, 1.0),
        ],
        "corbel_table" => P::new().to(0.08, 0.0).by(0.0, 0.6).by(0.12, 0.0).by(0.0, 0.25).by(0.05, 0.0).by(0.0, 0.15).close(),
        _ => named("cornice"),
    }
}

/// Opening outline in facade coordinates: centred on u = 0, bottom at
/// v = 0. Counter-clockwise, starting at the bottom right; the bottom edge
/// closes the ring (except for round shapes). Point counts depend only on
/// `kind` and `n`, so scaled copies correspond point by point.
#[derive(Clone, Debug)]
pub struct Outline {
    pub pts: Vec<Vec2>,
    /// Height where the arch starts.
    pub spring: f32,
    /// Index range of the arch head points (for hoods, voussoirs).
    pub head: (usize, usize),
    /// Closed curve without a flat bottom (oculus, rose).
    pub round: bool,
}

/// Default rise of an arch of span `w`.
pub fn default_rise(kind: ArchKind, w: f32) -> f32 {
    match kind {
        ArchKind::Flat | ArchKind::Shouldered => 0.0,
        ArchKind::Round | ArchKind::Trefoil => w * 0.5,
        ArchKind::Segmental => w / 6.0,
        ArchKind::Pointed => w * 0.866,
        ArchKind::Lancet => w * 1.25,
        ArchKind::Horseshoe => w * 0.93,
        ArchKind::PointedHorseshoe => w * 1.05,
        ArchKind::Ogee => w * 0.75,
        ArchKind::Basket => w / 3.0,
        ArchKind::Tudor => w / 3.2,
        ArchKind::Circle | ArchKind::Quatrefoil => w,
    }
}

fn arc(c: Vec2, r: f32, a0: f32, a1: f32, n: usize) -> Vec<Vec2> {
    (0..=n).map(|k| c + Vec2::from_angle(a0 + (a1 - a0) * k as f32 / n as f32) * r).collect()
}

fn bez(p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2, n: usize) -> Vec<Vec2> {
    (0..=n)
        .map(|k| {
            let t = k as f32 / n as f32;
            let u = 1.0 - t;
            p0 * (u * u * u) + p1 * (3.0 * u * u * t) + p2 * (3.0 * u * t * t) + p3 * (t * t * t)
        })
        .collect()
}

/// Right half of an arch head from the springing (w/2, s) to the apex,
/// `n + 1` points. The left half mirrors it.
fn half_head(kind: ArchKind, w: f32, s: f32, r: f32, n: usize) -> Vec<Vec2> {
    let hw = w * 0.5;
    match kind {
        ArchKind::Flat => (0..=n).map(|k| Vec2::new(hw * (1.0 - k as f32 / n as f32), s)).collect(),
        ArchKind::Shouldered => {
            let c = (w * 0.18).min(r.max(w * 0.12));
            let mut v: Vec<Vec2> = (0..=n / 2).map(|k| {
                let a = k as f32 / (n / 2).max(1) as f32 * FRAC_PI_2;
                Vec2::new(hw - c * (1.0 - a.cos()), s - c + c * a.sin())
            }).collect();
            let rest = n - n / 2;
            let start = *v.last().unwrap();
            for k in 1..=rest {
                v.push(Vec2::new(start.x * (1.0 - k as f32 / rest as f32), start.y));
            }
            v
        }
        ArchKind::Round | ArchKind::Basket => (0..=n)
            .map(|k| {
                let a = k as f32 / n as f32 * FRAC_PI_2;
                Vec2::new(hw * a.cos(), s + r * a.sin())
            })
            .collect(),
        ArchKind::Trefoil => (0..=n)
            .map(|k| {
                let a = k as f32 / n as f32 * FRAC_PI_2;
                // Three lobes over the half turn: cusps between them.
                let t = (a / PI * 3.0).fract();
                let lobe = 1.0 - 0.16 * (1.0 - (t * PI).sin());
                Vec2::new(hw * a.cos() * lobe, s + r * a.sin() * lobe)
            })
            .collect(),
        ArchKind::Segmental => {
            let r = r.max(1e-3);
            let big = (hw * hw + r * r) / (2.0 * r);
            let c = Vec2::new(0.0, s + r - big);
            let a0 = (s - c.y).atan2(hw);
            arc(c, big, a0, FRAC_PI_2, n)
        }
        ArchKind::Pointed | ArchKind::Lancet => {
            let big = (hw * hw + r * r) / w;
            let c = Vec2::new(hw - big, s);
            let a1 = r.atan2(big - hw);
            arc(c, big, 0.0, a1, n)
        }
        ArchKind::Horseshoe => {
            // Circle wider than the jambs, re-entering below its centre.
            let mut lo = hw;
            let mut hi = w * 2.0;
            for _ in 0..40 {
                let mid = (lo + hi) * 0.5;
                let drop = mid * (1.0 - (hw / mid).powi(2)).max(0.0).sqrt();
                if mid + drop < r {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            let big = (lo + hi) * 0.5;
            let drop = big * (1.0 - (hw / big).powi(2)).max(0.0).sqrt();
            let c = Vec2::new(0.0, s + drop);
            let a0 = -(hw / big).clamp(-1.0, 1.0).acos();
            arc(c, big, a0, FRAC_PI_2, n)
        }
        ArchKind::PointedHorseshoe => {
            // Pointed arch of a wider span whose arcs continue below the
            // springing back to the jambs.
            let w2 = w * 1.22;
            let hw2 = w2 * 0.5;
            let mut rr = r * 0.82;
            let (mut big, mut cx, mut a_j, mut drop) = (1.0, 0.0, 0.0, 0.0);
            for _ in 0..30 {
                big = (hw2 * hw2 + rr * rr) / w2;
                cx = hw2 - big;
                a_j = -((hw - cx) / big).clamp(-1.0, 1.0).acos();
                drop = -big * a_j.sin();
                rr = (r - drop).max(r * 0.3);
            }
            let c = Vec2::new(cx, s + drop);
            let a1 = rr.atan2(big - hw2);
            arc(c, big, a_j, a1, n)
        }
        ArchKind::Ogee => bez(Vec2::new(hw, s), Vec2::new(hw, s + 0.62 * r), Vec2::new(0.0, s + 0.42 * r), Vec2::new(0.0, s + r), n),
        ArchKind::Tudor => bez(Vec2::new(hw, s), Vec2::new(hw, s + 0.55 * r), Vec2::new(0.28 * w, s + 0.96 * r), Vec2::new(0.0, s + r), n),
        ArchKind::Circle | ArchKind::Quatrefoil => unreachable!(),
    }
}

/// Outline of an opening of span `w` and total height `h`.
pub fn outline(kind: ArchKind, w: f32, h: f32, rise: Option<f32>, n: usize) -> Outline {
    let n = n.max(2);
    let w = w.max(0.05);
    if matches!(kind, ArchKind::Circle | ArchKind::Quatrefoil) {
        let r = w * 0.5;
        let c = Vec2::new(0.0, r);
        let m = n * 4;
        let pts: Vec<Vec2> = (0..m)
            .map(|k| {
                let a = -FRAC_PI_2 + k as f32 / m as f32 * std::f32::consts::TAU;
                let lobe = if kind == ArchKind::Quatrefoil { 1.0 - 0.18 * (1.0 - (2.0 * (a + FRAC_PI_2 * 0.5)).cos().abs()) } else { 1.0 };
                c + Vec2::from_angle(a) * r * lobe
            })
            .collect();
        // Rest the lobed shape on v = 0.
        let lift = pts.iter().map(|p| p.y).fold(f32::MAX, f32::min);
        let pts: Vec<Vec2> = pts.into_iter().map(|p| p - Vec2::new(0.0, lift)).collect();
        return Outline { pts, spring: r - lift, head: (0, m), round: true };
    }
    let h = h.max(0.05);
    let rise = rise.unwrap_or_else(|| default_rise(kind, w)).clamp(0.0, h);
    let s = h - rise;
    let right = half_head(kind, w, s, rise, n);
    let mut pts = Vec::with_capacity(2 * n + 4);
    pts.push(Vec2::new(w * 0.5, 0.0));
    let head0 = pts.len();
    // Right half: springing to apex.
    for q in &right {
        pts.push(*q);
    }
    // Left half mirrored, apex excluded, back down to the springing.
    for q in right.iter().rev().skip(1) {
        pts.push(Vec2::new(-q.x, q.y));
    }
    let head1 = pts.len();
    pts.push(Vec2::new(-w * 0.5, 0.0));
    // Remove consecutive duplicates (springing at v = 0, flat heads).
    let mut out: Vec<Vec2> = Vec::with_capacity(pts.len());
    let (mut h0, mut h1) = (head0, head1);
    for (i, q) in pts.iter().enumerate() {
        if out.last().is_some_and(|l: &Vec2| (*l - *q).length() < 1e-5) {
            if i < head0 {
                h0 -= 1;
            }
            if i < head1 {
                h1 -= 1;
            }
            continue;
        }
        out.push(*q);
    }
    Outline { pts: out, spring: s, head: (h0, h1.min(pts.len())), round: false }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::landmark::poly;

    #[test]
    fn every_arch_is_a_valid_ccw_ring_of_the_right_size() {
        for (name, kind) in ArchKind::NAMES {
            let o = outline(*kind, 1.2, 2.6, None, 8);
            let a = poly::area(&o.pts);
            assert!(a > 0.0, "{name}: area {a}");
            let (mn, mx) = poly::bbox(&o.pts);
            assert!(mn.y.abs() < 1e-4, "{name} bottom {}", mn.y);
            if !o.round {
                assert!((mx.y - 2.6).abs() < 1e-3, "{name} top {}", mx.y);
                assert!((mx.x - 0.6).abs() < 0.05 || matches!(kind, ArchKind::Horseshoe | ArchKind::PointedHorseshoe), "{name} width {}", mx.x);
            }
            // Same point count for scaled copies.
            assert_eq!(o.pts.len(), outline(*kind, 2.0, 4.0, None, 8).pts.len(), "{name}");
        }
    }

    #[test]
    fn profiles_are_ccw() {
        for p in PROFILES {
            let v = named(p);
            assert!(poly::area(&v) > 0.0, "{p}");
            assert!(v.iter().all(|q| q.x >= -1e-4 && q.y >= -1e-4 && q.y <= 1.0001), "{p}");
        }
    }
}
