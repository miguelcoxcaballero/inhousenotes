//! Least-cost paths on a jittered lattice: how organic alleys are carved.
//!
//! The lattice covers one piece of land (shrunk by a metre) with points
//! every `step` metres, each jittered by keyed noise, and links every point
//! to its 16 neighbours (8 compass directions plus the 8 knight moves, so
//! paths are not stuck to 45° multiples). A path's cost grows near existing
//! streets, so new alleys leave room for lots on both sides; it wobbles
//! with smooth noise (`meander`); and it is cheaper along paths already
//! carved in this pass, so networks grow dendritically and merge, with
//! dead ends, as in the Barrio del Carmen.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

use forja_math::Vec2;

use crate::plan::line::mix64;
use crate::plan::{Located, noise2};

pub struct Lattice {
    pub pts: Vec<Vec2>,
    /// Cost factor per point (distance to the land's edge, noise).
    factor: Vec<f32>,
    pub carved: Vec<bool>,
    adj: Vec<Vec<u32>>,
    pub edge_dist: Vec<f32>,
}

const NEIGHBOURS: [(i32, i32); 16] = [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1), (2, 1), (2, -1), (-2, 1), (-2, -1), (1, 2), (1, -2), (-1, 2), (-1, -2)];

impl Lattice {
    /// Points inside `inside` on a grid of `step`, with costs from the
    /// distance to `edge` (the land outline).
    pub fn new(inside: &Located, edge: &Located, step: f32, meander: f32, seed: u64) -> Self {
        let Some([lo, hi]) = inside.region.bbox() else {
            return Lattice { pts: Vec::new(), factor: Vec::new(), carved: Vec::new(), adj: Vec::new(), edge_dist: Vec::new() };
        };
        let (nx, nz) = (((hi.x - lo.x) / step).ceil() as i32 + 1, ((hi.y - lo.y) / step).ceil() as i32 + 1);
        let mut index: HashMap<(i32, i32), u32> = HashMap::new();
        let mut pts = Vec::new();
        let mut cells = Vec::new();
        for i in 0..nx {
            for j in 0..nz {
                // Keyed jitter: depends on the absolute cell, not the piece.
                let gi = ((lo.x / step).floor() as i64 + i as i64, (lo.y / step).floor() as i64 + j as i64);
                let h = mix64(seed ^ (gi.0 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (gi.1 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F));
                let jx = ((h & 0xffff) as f32 / 65535.0 - 0.5) * 0.6 * step;
                let jz = (((h >> 16) & 0xffff) as f32 / 65535.0 - 0.5) * 0.6 * step;
                let p = Vec2::new(gi.0 as f32 * step + jx, gi.1 as f32 * step + jz);
                if inside.contains(p) {
                    index.insert((i, j), pts.len() as u32);
                    pts.push(p);
                    cells.push((i, j));
                }
            }
        }
        let mut adj = vec![Vec::new(); pts.len()];
        for (k, &(i, j)) in cells.iter().enumerate() {
            for (di, dj) in NEIGHBOURS {
                if let Some(&o) = index.get(&(i + di, j + dj)) {
                    adj[k].push(o);
                }
            }
        }
        let edge_dist: Vec<f32> = pts.iter().map(|p| edge.distance(*p, 40.0)).collect();
        let factor = pts
            .iter()
            .zip(&edge_dist)
            .map(|(p, d)| (1.0 + meander * noise2(seed ^ 0x51, *p / 22.0)).max(0.15) * (1.0 + 2.0 * (-d / 6.0).exp()))
            .collect();
        let n = pts.len();
        Lattice { pts, factor, carved: vec![false; n], adj, edge_dist }
    }

    pub fn is_empty(&self) -> bool {
        self.pts.is_empty()
    }

    /// The lattice point nearest to `p`.
    pub fn nearest(&self, p: Vec2) -> Option<u32> {
        (0..self.pts.len() as u32).min_by(|&a, &b| self.pts[a as usize].distance_squared(p).total_cmp(&self.pts[b as usize].distance_squared(p)).then(a.cmp(&b)))
    }

    fn cost(&self, a: u32, b: u32) -> f32 {
        let (a, b) = (a as usize, b as usize);
        let len = self.pts[a].distance(self.pts[b]);
        let reuse = if self.carved[a] && self.carved[b] { 0.25 } else { 1.0 };
        len * 0.5 * (self.factor[a] + self.factor[b]) * reuse
    }

    /// Cheapest path from `from` to the first point accepted by `goal`
    /// (Dijkstra with an index tie-break, so it is deterministic).
    pub fn path(&self, from: u32, goal: impl Fn(u32) -> bool) -> Option<Vec<u32>> {
        let n = self.pts.len();
        let mut dist = vec![f32::INFINITY; n];
        let mut prev = vec![u32::MAX; n];
        let mut heap = BinaryHeap::new();
        dist[from as usize] = 0.0;
        heap.push(Reverse((ordered(0.0), from)));
        while let Some(Reverse((d, u))) = heap.pop() {
            let d = f32::from_bits(d as u32);
            if d > dist[u as usize] {
                continue;
            }
            if u != from && goal(u) {
                let mut p = vec![u];
                let mut c = u;
                while prev[c as usize] != u32::MAX {
                    c = prev[c as usize];
                    p.push(c);
                }
                p.reverse();
                return Some(p);
            }
            for &v in &self.adj[u as usize] {
                let nd = d + self.cost(u, v);
                if nd < dist[v as usize] {
                    dist[v as usize] = nd;
                    prev[v as usize] = u;
                    heap.push(Reverse((ordered(nd), v)));
                }
            }
        }
        None
    }
}

/// Order-preserving key of a non-negative float.
fn ordered(x: f32) -> u64 {
    x.max(0.0).to_bits() as u64
}
