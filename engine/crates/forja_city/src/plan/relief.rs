//! `Relief`: the city on sloping ground.
//!
//! Terrain is data (`Hill`s carried by the draft). The street graph turns
//! it into a city's levels in one step: every node starts at the terrain
//! height and a projection solver then grades the network, so no street
//! is steeper than its profile allows (avenues 5 %, streets 10 %, alleys
//! 15 %, `stairs` profiles up to 50 % on their stepped part). Avenues
//! therefore cut gently through a hill while alleys climb it, and an
//! alley steeper than its ramp limit becomes a stair street.
//!
//! A plaza is a level terrace: the street mouths that open onto it are
//! held at one level, so on a hill the squares stand at different levels
//! with the streets climbing between them, and where a terrace meets lower
//! or higher public ground the mesh draws a granite wall.
//!
//! Along a street the level is linear in arc length (stepped on stairs).
//! Everywhere else (sidewalks, plazas, courts, building lots) it is a
//! blend of the levels of the nearby streets, each weighted by how close
//! the point is to that street's space; inside one street's space that
//! street's level wins, so streets are level across. `elev` is the one
//! function every mesh, kerb, marking, item, lot base and collider adds
//! to its own class height.

use forja_math::Vec2;
use serde::{Deserialize, Serialize};

use super::graph::{End, StreetGraph, StreetId};
use super::index::SegIndex;
use super::line::{noise2, seg_dist};
use crate::profile::ProfileLibrary;

/// A smooth rise of the ground: `height` at `at`, falling to 0 at
/// `radius` (raised cosine), with value noise of amplitude `noise`
/// scaled by the rise.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hill {
    pub at: Vec2,
    pub radius: f32,
    pub height: f32,
    #[serde(default)]
    pub noise: f32,
    #[serde(default)]
    pub seed: u64,
}

/// Terrain height (m) at `p`.
pub fn terrain(hills: &[Hill], p: Vec2) -> f32 {
    hills
        .iter()
        .map(|h| {
            let d = (p.distance(h.at) / h.radius.max(1.0)).min(1.0);
            let bump = 0.5 * (1.0 + (std::f32::consts::PI * d).cos());
            bump * (h.height + h.noise * noise2(h.seed, p / 45.0))
        })
        .sum::<f32>()
        .max(0.0)
}

/// Height of one riser of a stair street (m).
pub const RISER: f32 = 0.16;
/// Steepest stepped part of a stair street (riser / tread).
pub const STAIR_GRADE: f32 = 0.5;

/// A stair street's steps: `n` risers evenly spread between arc lengths
/// `s0` and `s1`, flat landings before and after.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Steps {
    pub s0: f32,
    pub s1: f32,
    pub n: u32,
}

#[derive(Clone, Debug, PartialEq)]
struct Grade {
    z: [f32; 2],
    len: f32,
    steps: Option<Steps>,
}

/// A plaza on sloping ground: one level for all of it.
#[derive(Clone, Debug, PartialEq)]
pub struct Terrace {
    pub ring: Vec<Vec2>,
    pub z: f32,
    lo: Vec2,
    hi: Vec2,
}

impl Terrace {
    fn new(ring: Vec<Vec2>, z: f32) -> Self {
        let (lo, hi) = ring.iter().fold((Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)), |(a, b), p| (a.min(*p), b.max(*p)));
        Terrace { ring, z, lo, hi }
    }

    pub fn contains(&self, p: Vec2) -> bool {
        if p.x < self.lo.x || p.y < self.lo.y || p.x > self.hi.x || p.y > self.hi.y {
            return false;
        }
        let (r, n) = (&self.ring, self.ring.len());
        let mut c = false;
        for i in 0..n {
            let (a, b) = (r[i], r[(i + 1) % n]);
            if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
                c = !c;
            }
        }
        c
    }

    /// Distance to the edge, if within `max` of the bounding box.
    fn edge_dist(&self, p: Vec2, max: f32) -> Option<f32> {
        if p.x < self.lo.x - max || p.y < self.lo.y - max || p.x > self.hi.x + max || p.y > self.hi.y + max {
            return None;
        }
        let n = self.ring.len();
        Some((0..n).map(|i| seg_dist(p, self.ring[i], self.ring[(i + 1) % n])).fold(f32::INFINITY, f32::min))
    }
}

/// Street nodes this close to a plaza's edge open onto it (m).
const MOUTH: f32 = 1.5;

#[derive(Clone, Debug, Default)]
pub struct Relief {
    /// Level of each node (m).
    pub nodes: Vec<f32>,
    streets: Vec<Grade>,
    hills: Vec<Hill>,
    /// Level plazas.
    pub terraces: Vec<Terrace>,
    /// Copy of the street centrelines for `elev` (segments, payload =
    /// index into `segs`).
    index: SegIndex,
    segs: Vec<(StreetId, f32 /*station at a*/)>,
    widths: Vec<(Vec<f32>, Vec<f32>)>,
}

/// How far around a point the streets' levels are blended (m).
const BLEND: f32 = 30.0;

impl Relief {
    pub fn is_flat(&self) -> bool {
        self.hills.is_empty()
    }

    /// Grades the network over the terrain, with each of `plazas` level.
    pub fn solve(g: &StreetGraph, lib: &ProfileLibrary, hills: &[Hill], plazas: &[Vec<Vec2>]) -> Relief {
        if hills.is_empty() || g.streets.is_empty() {
            return Relief { nodes: vec![0.0; g.nodes.len()], streets: g.streets.iter().map(|s| Grade { z: [0.0; 2], len: s.len(), steps: None }).collect(), ..Default::default() };
        }
        // Stair streets keep flat landings in the mouths of the streets they meet.
        let landing = |s: StreetId, e: End| {
            let n = g.street(s).node(e);
            let others = g.star(n).iter().filter(|a| a.0 != s).map(|a| g.street(a.0).width_at(g.street(a.0).end_s(a.1)) * 0.5).fold(0.0, f32::max);
            others + 0.4
        };
        let spans: Vec<(f32, f32)> = g
            .streets
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let l = s.len();
                if !lib.get(s.profile).stairs {
                    return (0.0, l);
                }
                let (a, b) = (landing(i as StreetId, End::A).min(l * 0.25), landing(i as StreetId, End::B).min(l * 0.25));
                (a, l - b)
            })
            .collect();
        // Constraint per street: |z_b - z_a| <= limit.
        let limit: Vec<f32> = g
            .streets
            .iter()
            .zip(&spans)
            .map(|(s, (a, b))| {
                let p = lib.get(s.profile);
                if p.stairs { (STAIR_GRADE * (b - a)).max(p.max_grade() * s.len()) } else { p.max_grade() * s.len() }
            })
            .collect();
        let mut z: Vec<f32> = g.nodes.iter().map(|n| terrain(hills, n.p)).collect();
        // The nodes that open onto each plaza share its level.
        let mut terraces: Vec<Terrace> = plazas.iter().filter(|r| r.len() >= 3).map(|r| Terrace::new(r.clone(), 0.0)).collect();
        let mouths: Vec<Vec<usize>> = terraces
            .iter()
            .map(|t| (0..g.nodes.len()).filter(|&n| t.contains(g.nodes[n].p) || t.edge_dist(g.nodes[n].p, MOUTH).is_some_and(|d| d <= MOUTH)).collect())
            .collect();
        // Projection onto the grade constraints and the plaza levels
        // (Gauss–Seidel), in id order.
        for _ in 0..600 {
            let mut worst = 0.0f32;
            for m in &mouths {
                if m.len() < 2 {
                    continue;
                }
                let mean = m.iter().map(|&n| z[n]).sum::<f32>() / m.len() as f32;
                for &n in m {
                    worst = worst.max((z[n] - mean).abs());
                    z[n] = mean;
                }
            }
            for (s, m) in g.streets.iter().zip(&limit) {
                let (a, b) = (s.a as usize, s.b as usize);
                let d = z[b] - z[a];
                let excess = d.abs() - m;
                if excess > 0.0 {
                    let k = excess * 0.5 * d.signum();
                    z[a] += k;
                    z[b] -= k;
                    worst = worst.max(excess);
                }
            }
            if worst < 1e-4 {
                break;
            }
        }
        let streets = g
            .streets
            .iter()
            .zip(&spans)
            .map(|(s, &(s0, s1))| {
                let p = lib.get(s.profile);
                let za = [z[s.a as usize], z[s.b as usize]];
                let dz = za[1] - za[0];
                let steep = dz.abs() > p.max_grade() * s.len() + 0.05;
                let steps = (p.stairs && steep && s1 > s0).then(|| Steps { s0, s1, n: (dz.abs() / RISER).ceil().max(1.0) as u32 });
                Grade { z: za, len: s.len(), steps }
            })
            .collect();
        let mut index = SegIndex::new(16.0);
        let mut segs = Vec::new();
        let mut widths = Vec::new();
        for (si, s) in g.streets.iter().enumerate() {
            let p = s.line.pts();
            let st = s.line.stations();
            for k in 0..p.len() - 1 {
                index.push(p[k], p[k + 1], segs.len() as u32);
                segs.push((si as StreetId, st[k]));
            }
            widths.push((st.to_vec(), s.width.clone()));
        }
        let mut r = Relief { nodes: z, streets, hills: hills.to_vec(), terraces: Vec::new(), index, segs, widths };
        // A plaza's level: that of its mouths, else of the streets around it.
        for (t, m) in terraces.iter_mut().zip(&mouths) {
            t.z = if m.is_empty() {
                let c = t.ring.iter().copied().sum::<Vec2>() / t.ring.len() as f32;
                r.elev(c)
            } else {
                m.iter().map(|&n| r.nodes[n]).sum::<f32>() / m.len() as f32
            };
        }
        r.terraces = terraces;
        r
    }

    /// The terrace (level plaza) containing `p`.
    pub fn terrace_at(&self, p: Vec2) -> Option<&Terrace> {
        self.terraces.iter().find(|t| t.contains(p))
    }

    /// Level along street `s` at arc length `t`.
    pub fn street_z(&self, s: StreetId, t: f32) -> f32 {
        let gr = &self.streets[s as usize];
        let dz = gr.z[1] - gr.z[0];
        match gr.steps {
            Some(st) => {
                let tread = (st.s1 - st.s0) / st.n as f32;
                let k = ((t - st.s0) / tread + 0.5).floor().clamp(0.0, st.n as f32);
                gr.z[0] + dz * k / st.n as f32
            }
            None => gr.z[0] + dz * (t / gr.len.max(1e-3)).clamp(0.0, 1.0),
        }
    }

    /// Steepest grade of street `s` (rise over run on its ramp or steps).
    pub fn grade(&self, s: StreetId) -> f32 {
        let gr = &self.streets[s as usize];
        let run = gr.steps.map(|st| st.s1 - st.s0).unwrap_or(gr.len);
        (gr.z[1] - gr.z[0]).abs() / run.max(1e-3)
    }

    pub fn steps(&self, s: StreetId) -> Option<Steps> {
        self.streets.get(s as usize).and_then(|g| g.steps)
    }

    /// Arc lengths of every riser of street `s`.
    pub fn risers(&self, s: StreetId) -> Vec<f32> {
        let Some(st) = self.steps(s) else { return Vec::new() };
        let tread = (st.s1 - st.s0) / st.n as f32;
        (0..st.n).map(|k| st.s0 + tread * (k as f32 + 0.5)).collect()
    }

    fn width_at(&self, s: StreetId, t: f32) -> f32 {
        let (st, w) = &self.widths[s as usize];
        let k = st.partition_point(|v| *v <= t).saturating_sub(1).min(w.len().saturating_sub(1));
        let k1 = (k + 1).min(w.len() - 1);
        let span = (st.get(k1).copied().unwrap_or(t) - st[k]).max(1e-6);
        w[k] + (w[k1] - w[k]) * ((t - st[k]) / span).clamp(0.0, 1.0)
    }

    /// Ground level (m) at any point of the city.
    pub fn elev(&self, p: Vec2) -> f32 {
        if self.is_flat() {
            return 0.0;
        }
        if let Some(t) = self.terrace_at(p) {
            return t.z;
        }
        // The closest point of each nearby street.
        let mut near: Vec<(StreetId, f32, f32)> = Vec::new();
        for k in self.index.around(p, BLEND) {
            let (a, b, si) = self.index.seg(k);
            let (s, s_at) = self.segs[si as usize];
            let d = seg_dist(p, a, b);
            if d > BLEND {
                continue;
            }
            let e = b - a;
            let t = s_at + ((p - a).dot(e) / e.length_squared().max(1e-9)).clamp(0.0, 1.0) * e.length();
            match near.iter_mut().find(|x| x.0 == s) {
                Some(x) if d < x.2 => *x = (s, t, d),
                Some(_) => {}
                None => near.push((s, t, d)),
            }
        }
        // Inside a street's space its level dominates; outside, closer
        // streets count more, fading out at BLEND. The terrain fills in.
        let (mut acc, mut wsum) = (0.0f64, 0.0f64);
        for (s, t, d) in near {
            let gap = (d - self.width_at(s, t) * 0.5).max(0.0);
            let fade = ((BLEND - d) / (BLEND * 0.3)).clamp(0.0, 1.0);
            let w = fade as f64 / (gap as f64 * gap as f64 + 0.01);
            acc += w * self.street_z(s, t) as f64;
            wsum += w;
        }
        // Terraces count as streets do, by distance to their edge.
        for tr in &self.terraces {
            if let Some(d) = tr.edge_dist(p, BLEND).filter(|d| *d < BLEND) {
                let fade = ((BLEND - d) / (BLEND * 0.3)).clamp(0.0, 1.0);
                let w = fade as f64 / (d as f64 * d as f64 + 0.01);
                acc += w * tr.z as f64;
                wsum += w;
            }
        }
        let wt = 1.0 / (BLEND as f64 * BLEND as f64);
        ((acc + wt * terrain(&self.hills, p) as f64) / (wsum + wt)) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::Way;

    fn hill() -> Vec<Hill> {
        vec![Hill { at: Vec2::ZERO, radius: 160.0, height: 22.0, noise: 0.0, seed: 1 }]
    }

    #[test]
    fn grades_respect_every_profile_and_alleys_become_stairs() {
        let lib = ProfileLibrary::builtin();
        // An avenue straight over the hill, a ring street, and alleys
        // climbing straight to the top.
        let mut ways = vec![Way::new(vec![Vec2::new(-150.0, 0.0), Vec2::new(150.0, 0.0)], "avenida")];
        let ring: Vec<Vec2> = (0..=48).map(|i| Vec2::from_angle(i as f32 / 48.0 * std::f32::consts::TAU) * 60.0).collect();
        ways.push(Way::new(ring, "madrid_calle"));
        for k in 0..6 {
            let d = Vec2::from_angle(0.4 + k as f32);
            ways.push(Way::new(vec![d * 8.0, d * 60.0], "callejon"));
        }
        ways.push(Way::new(vec![Vec2::new(0.0, -8.0), Vec2::new(0.0, 8.0)], "callejon"));
        let g = StreetGraph::build(&ways, &[], &lib, 0.05);
        let r = Relief::solve(&g, &lib, &hill(), &[]);
        let mut stairs = 0;
        for (i, s) in g.streets.iter().enumerate() {
            let p = lib.get(s.profile);
            let gr = r.grade(i as StreetId);
            let max = if r.steps(i as StreetId).is_some() { STAIR_GRADE } else { p.max_grade() };
            assert!(gr <= max + 1e-3, "{} at {gr:.3} > {max}", p.id);
            stairs += r.steps(i as StreetId).is_some() as usize;
            // Continuous at both nodes.
            assert!((r.street_z(i as StreetId, 0.0) - r.nodes[s.a as usize]).abs() < 1e-4);
            assert!((r.street_z(i as StreetId, s.len()) - r.nodes[s.b as usize]).abs() < 1e-4);
        }
        assert!(stairs >= 2, "{stairs} stair streets");
        // The hill is still there: the centre stands well above the edge.
        assert!(r.elev(Vec2::new(20.0, 20.0)) > r.elev(Vec2::new(140.0, 0.0)) + 5.0);
        // Level across a street, continuous beside it.
        let (a, b) = (r.elev(Vec2::new(-100.0, 3.0)), r.elev(Vec2::new(-100.0, -3.0)));
        assert!((a - b).abs() < 0.02, "{a} {b}");
        let (c, d) = (r.elev(Vec2::new(30.0, 30.0)), r.elev(Vec2::new(30.2, 30.0)));
        assert!((c - d).abs() < 0.1);
    }

    #[test]
    fn plazas_are_level_terraces_at_their_own_levels() {
        let lib = ProfileLibrary::builtin();
        // Two squares on the flank of the hill, alleys and a lane running
        // up and down from each.
        let sq = |c: Vec2, r: f32| -> Vec<Vec2> { (0..5).map(|i| c + Vec2::from_angle(i as f32 / 5.0 * std::f32::consts::TAU + 0.3) * r).collect() };
        let (c1, c2) = (Vec2::new(40.0, 0.0), Vec2::new(110.0, 0.0));
        let plazas = vec![sq(c1, 14.0), sq(c2, 12.0)];
        let mut ways = Vec::new();
        for (c, ring) in [(c1, &plazas[0]), (c2, &plazas[1])] {
            for k in [0usize, 2, 3] {
                let e = ring[k];
                let out = (e - c).normalize();
                ways.push(Way::new(vec![e, e + out * 40.0], "callejuela"));
            }
        }
        ways.push(Way::new(vec![plazas[0][0], plazas[1][3]], "calle_vieja"));
        let g = StreetGraph::build(&ways, &[], &lib, 0.05);
        let r = Relief::solve(&g, &lib, &hill(), &plazas);
        assert_eq!(r.terraces.len(), 2);
        for (t, c) in r.terraces.iter().zip([c1, c2]) {
            // Level inside, and the mouths at that level.
            for k in 0..20 {
                let q = c + Vec2::from_angle(k as f32) * (k as f32 * 0.4);
                assert!((r.elev(q) - t.z).abs() < 1e-4, "{} vs {}", r.elev(q), t.z);
            }
            for (n, node) in g.nodes.iter().enumerate() {
                if t.ring.iter().any(|p| p.distance(node.p) < 0.5) {
                    assert!((r.nodes[n] - t.z).abs() < 1e-3, "mouth {} vs {}", r.nodes[n], t.z);
                }
            }
        }
        // Different levels, the upper square nearer the top.
        assert!(r.terraces[0].z > r.terraces[1].z + 2.0, "{:?}", r.terraces.iter().map(|t| t.z).collect::<Vec<_>>());
        // Grades still hold.
        for (i, s) in g.streets.iter().enumerate() {
            let max = if r.steps(i as StreetId).is_some() { STAIR_GRADE } else { lib.get(s.profile).max_grade() };
            assert!(r.grade(i as StreetId) <= max + 1e-3);
        }
    }

    #[test]
    fn no_hills_is_flat_and_free() {
        let lib = ProfileLibrary::builtin();
        let g = StreetGraph::build(&[Way::new(vec![Vec2::ZERO, Vec2::new(50.0, 0.0)], "avenida")], &[], &lib, 0.05);
        let r = Relief::solve(&g, &lib, &[], &[]);
        assert!(r.is_flat() && r.elev(Vec2::new(10.0, 1.0)) == 0.0 && r.grade(0) == 0.0);
    }
}
