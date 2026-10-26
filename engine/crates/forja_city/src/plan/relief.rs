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

#[derive(Clone, Debug, Default)]
pub struct Relief {
    /// Level of each node (m).
    pub nodes: Vec<f32>,
    streets: Vec<Grade>,
    hills: Vec<Hill>,
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

    /// Grades the network over the terrain.
    pub fn solve(g: &StreetGraph, lib: &ProfileLibrary, hills: &[Hill]) -> Relief {
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
        // Projection onto the grade constraints (Gauss–Seidel), in id order.
        for _ in 0..600 {
            let mut worst = 0.0f32;
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
        Relief { nodes: z, streets, hills: hills.to_vec(), index, segs, widths }
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
        let r = Relief::solve(&g, &lib, &hill());
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
    fn no_hills_is_flat_and_free() {
        let lib = ProfileLibrary::builtin();
        let g = StreetGraph::build(&[Way::new(vec![Vec2::ZERO, Vec2::new(50.0, 0.0)], "avenida")], &[], &lib, 0.05);
        let r = Relief::solve(&g, &lib, &[]);
        assert!(r.is_flat() && r.elev(Vec2::new(10.0, 1.0)) == 0.0 && r.grade(0) == 0.0);
    }
}
