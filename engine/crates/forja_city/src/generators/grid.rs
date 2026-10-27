//! Planned grids (Ensanche, Madrid barrio): jittered column and row
//! widths, optionally rotated (`axis`), leaning (`shear`, giving acute and
//! obtuse corners) and curved (`bend`). One polyline per whole grid line;
//! the fabric finds the blocks.

use forja_math::Vec2;
use serde::Deserialize;

use super::Zone;
use crate::fabric::stage;
use crate::geom::Rng;
use crate::plan::{Draft, Plaza, Way};
use crate::profile::ProfileLibrary;

fn one() -> f32 {
    1.0
}

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GridSpec {
    /// Point of the near edge (u = 0) at the middle of the rows.
    pub origin: Vec2,
    /// +1: columns grow towards +x (rotated by `axis`), -1 towards -x.
    #[serde(default = "one")]
    pub dir: f32,
    /// Rotation of the grid (radians).
    #[serde(default)]
    pub axis: f32,
    /// Columns lean by `shear` metres per metre of rows.
    #[serde(default)]
    pub shear: f32,
    /// Columns curve: u += bend * v² / 2 (1/m).
    #[serde(default)]
    pub bend: f32,
    pub cols: usize,
    pub rows: usize,
    /// Block size range (m).
    pub block: [f32; 2],
    /// Profile of the inner streets.
    pub street: String,
    /// Profile of the streets around the grid.
    #[serde(default)]
    pub outer: Option<String>,
    /// From u = 0 to the first building line: half the street running
    /// there (a global way, or `near_street`).
    pub near: f32,
    #[serde(default)]
    pub near_street: Option<String>,
    /// Block (col, row) left open as a plaza.
    #[serde(default)]
    pub plaza: Option<[usize; 2]>,
    /// Alternate one-way streets.
    #[serde(default = "yes")]
    pub oneway: bool,
}

/// The grid's ways and plaza, and its mask (at the outer centrelines).
pub fn generate(g: &GridSpec, _zone: &Zone, zi: u16, lib: &ProfileLibrary, seed: u64) -> (Draft, Vec<Vec2>) {
    let street_w = lib.by_name(&g.street).map(|p| p.default_width()).unwrap_or(11.0);
    let outer = g.outer.clone().unwrap_or_else(|| g.street.clone());
    let mut rng = Rng::keyed(seed, stage::GRID, zi as u64);
    // Columns away from the near edge, rows centred.
    let mut xs = Vec::new();
    let mut cur = g.near;
    for _ in 0..g.cols {
        let w = rng.range(g.block[0], g.block[1]);
        xs.push((cur, cur + w));
        cur += w + street_w;
    }
    let u_far = xs.last().map(|x| x.1 + street_w * 0.5).unwrap_or(g.near);
    let heights: Vec<f32> = (0..g.rows).map(|_| rng.range(g.block[0], g.block[1])).collect();
    let total = heights.iter().sum::<f32>() + street_w * (g.rows + 1) as f32;
    let mut zs = Vec::new();
    let mut z = -total * 0.5 + street_w;
    for h in &heights {
        zs.push((z, z + h));
        z += h + street_w;
    }
    let v0 = zs.first().map(|z| z.0 - street_w * 0.5).unwrap_or(0.0);
    let v1 = zs.last().map(|z| z.1 + street_w * 0.5).unwrap_or(0.0);
    let (sa, ca) = g.axis.sin_cos();
    let world = |u: f32, v: f32| {
        let x = g.dir * (u + g.shear * v + 0.5 * g.bend * v * v);
        g.origin + Vec2::new(x * ca - v * sa, x * sa + v * ca)
    };
    // A column line, resampled so bends stay smooth.
    let column = |u: f32, va: f32, vb: f32| -> Vec<Vec2> {
        let n = if g.bend.abs() > 1e-6 { ((vb - va) / 6.0).ceil().max(1.0) as usize } else { 1 };
        (0..=n).map(|k| world(u, va + (vb - va) * k as f32 / n as f32)).collect()
    };
    let mut d = Draft::default();
    // Column streets: between columns and at the far edge.
    let mut us: Vec<f32> = xs.windows(2).map(|w| (w[0].1 + w[1].0) * 0.5).collect();
    us.push(u_far);
    for (k, u) in us.iter().enumerate() {
        let far = k + 1 == us.len();
        let mut w = Way::new(column(*u, v0, v1), if far { &outer } else { &g.street }).zone(zi);
        if g.oneway {
            w.oneway = if k % 2 == 0 { 1 } else { -1 };
        }
        d.ways.push(w);
    }
    if let Some(ns) = &g.near_street {
        d.ways.push(Way::new(column(0.0, v0, v1), ns).zone(zi));
    }
    // Row streets: from the near edge to the far edge.
    let mut vs: Vec<f32> = vec![v0];
    vs.extend(zs.windows(2).map(|w| (w[0].1 + w[1].0) * 0.5));
    vs.push(v1);
    for (k, v) in vs.iter().enumerate() {
        let edge = k == 0 || k + 1 == vs.len();
        let mut w = Way::new(vec![world(0.0, *v), world(u_far, *v)], if edge { &outer } else { &g.street }).zone(zi);
        if g.oneway {
            w.oneway = if k % 2 == 0 { 1 } else { -1 };
        }
        d.ways.push(w);
    }
    // Plaza: the open block, at its building lines.
    if let Some([c, r]) = g.plaza {
        if let (Some(&(u0, u1)), Some(&(za, zb))) = (xs.get(c), zs.get(r)) {
            let mut ring: Vec<Vec2> = Vec::new();
            ring.extend(column(u0, za, zb));
            ring.extend(column(u1, za, zb).into_iter().rev());
            if crate::geom::signed_area(&ring) < 0.0 {
                ring.reverse();
            }
            d.plazas.push(Plaza { ring, zone: zi, kind: _zone.plaza_kind });
        }
    }
    // Mask: the grid at its outer centrelines, from the near edge.
    let mut mask: Vec<Vec2> = Vec::new();
    mask.extend(column(0.0, v0, v1));
    mask.extend(column(u_far, v0, v1).into_iter().rev());
    if crate::geom::signed_area(&mask) < 0.0 {
        mask.reverse();
    }
    (d, mask)
}
