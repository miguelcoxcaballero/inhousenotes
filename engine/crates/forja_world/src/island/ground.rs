//! The island's ground: one height grid for the whole island (16 m cells,
//! the same triangles for rendering and collision) and a land-cover class
//! per triangle.
//!
//! Heights are metres above sea level ([`SEA`] = 0). Where the island is
//! real geography (Valencia and its surroundings, kept at real scale and
//! position) the base is the real terrain (IGN MDT25, see
//! [`super::data::RealDem`]); elsewhere it is a low designed plain
//! ([`PLAIN`]). The island's designed relief (its height map: the
//! mountains of the interior, which are not real geography) is added on
//! top of that base. The result is reshaped for play: the shore crosses the
//! sea surface on the coastline, the relief fades out towards the coast
//! instead of ending in a cut, cliffs are limited to ~40 degrees, the
//! ground is levelled along the roads and lowered under lakes and rivers.

use forja_assets::MeshData;
use forja_math::{Vec2, Vec3, Vec4};

use super::data::{HeightMap, IslandData, Line, PolyKind};

/// Grid cell (m).
pub const CELL: f32 = 16.0;
/// Cells per terrain tile side (512 m tiles).
pub const TILE_CELLS: usize = 32;
pub const TILE: f32 = CELL * TILE_CELLS as f32;
/// Height of the sea surface.
pub const SEA: f32 = 0.0;
/// Height of the designed plain where there is no real terrain (m).
pub const PLAIN: f32 = 5.0;
/// Dry land is never lower than this (the sea sheet lies at 0).
pub const LAND_MIN: f32 = 0.8;
/// Lakes and rivers lie this far below their banks.
pub const WATER_DROP: f32 = 1.5;
/// Height of the land base in the source height map (m).
const BASE: f32 = 14.0;

/// Land cover of a ground triangle (also its material).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Cover {
    Scrub = 0,
    Rock,
    Grass,
    Park,
    FieldGreen,
    FieldSoil,
    FieldDry,
    Orchard,
    Forest,
    Sand,
    Wetland,
    Paving,
    Concrete,
    Sport,
    Gravel,
    Water,
    Rice,
}

pub const COVERS: usize = 17;

impl Cover {
    pub fn from_u8(v: u8) -> Cover {
        use Cover::*;
        const ALL: [Cover; COVERS] = [Scrub, Rock, Grass, Park, FieldGreen, FieldSoil, FieldDry, Orchard, Forest, Sand, Wetland, Paving, Concrete, Sport, Gravel, Water, Rice];
        ALL[(v as usize).min(COVERS - 1)]
    }
}

pub struct Ground {
    /// World position of vertex `(0, 0)`.
    pub x0: f32,
    pub z0: f32,
    /// Vertices per side.
    pub cols: usize,
    pub rows: usize,
    /// Row-major heights (`r * cols + c`; rows run south).
    pub h: Vec<f32>,
    /// Cover of each triangle: `(r * (cols - 1) + c) * 2 + (0 | 1)`, in
    /// the split of `forja_physics::HeightField::triangle`.
    pub cover: Vec<u8>,
    /// Signed distance to the coastline at each vertex (m, positive on
    /// land; exact near the coast, approximate far from it).
    pub coast: Vec<f32>,
}

/// Calls `emit(row, c0, c1)` for every run of lattice samples inside the
/// rings (even-odd). Sample `(r, c)` is at `(x0 + (c + ox) * cell, z0 +
/// (r + oz) * cell)`; `c1` is exclusive.
pub fn scan(rings: &[Vec<Vec2>], x0: f32, z0: f32, ox: f32, oz: f32, cell: f32, cols: usize, rows: usize, mut emit: impl FnMut(usize, usize, usize)) {
    let (mut zmin, mut zmax) = (f32::MAX, f32::MIN);
    for p in rings.iter().flatten() {
        zmin = zmin.min(p.y);
        zmax = zmax.max(p.y);
    }
    if zmin > zmax {
        return;
    }
    let r0 = (((zmin - z0) / cell - oz).ceil().max(0.0)) as usize;
    let r1 = (((zmax - z0) / cell - oz).floor()).min(rows as f32 - 1.0);
    if r1 < 0.0 {
        return;
    }
    let mut xs: Vec<f32> = Vec::new();
    for r in r0..=r1 as usize {
        let z = z0 + (r as f32 + oz) * cell;
        xs.clear();
        for ring in rings {
            let n = ring.len();
            let mut j = n - 1;
            for i in 0..n {
                let (a, b) = (ring[i], ring[j]);
                if (a.y > z) != (b.y > z) {
                    xs.push((b.x - a.x) * (z - a.y) / (b.y - a.y) + a.x);
                }
                j = i;
            }
        }
        xs.sort_by(f32::total_cmp);
        for pair in xs.chunks_exact(2) {
            let c0 = (((pair[0] - x0) / cell - ox).ceil().max(0.0)) as usize;
            let c1 = ((pair[1] - x0) / cell - ox).ceil().min(cols as f32);
            if c1 > c0 as f32 {
                emit(r, c0, c1 as usize);
            }
        }
    }
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn seg_dist(p: Vec2, a: Vec2, b: Vec2) -> (f32, f32) {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared().max(1e-9)).clamp(0.0, 1.0);
    ((a + ab * t).distance(p), t)
}

impl HeightMap {
    /// Land height above the base at a world position; `None` over the
    /// sea and the lagoons (no valid sample around).
    fn land(&self, x: f32, z: f32) -> Option<f32> {
        let n = self.size;
        let fx = ((x - self.extent[0]) / (self.extent[2] - self.extent[0]) * (n - 1) as f32).clamp(0.0, (n - 1) as f32);
        let fz = ((z - self.extent[1]) / (self.extent[3] - self.extent[1]) * (n - 1) as f32).clamp(0.0, (n - 1) as f32);
        let (c, r) = ((fx as usize).min(n - 2), (fz as usize).min(n - 2));
        let (u, v) = (fx - c as f32, fz - r as f32);
        let (mut sum, mut w) = (0.0, 0.0);
        for (dr, dc, k) in [(0, 0, (1.0 - u) * (1.0 - v)), (0, 1, u * (1.0 - v)), (1, 0, (1.0 - u) * v), (1, 1, u * v)] {
            let s = self.dm[(r + dr) * n + c + dc] as f32 * 0.1;
            if s >= BASE - 0.05 && k > 0.0 {
                sum += (s - BASE) * k;
                w += k;
            }
        }
        (w > 0.2).then(|| (sum / w).max(0.0))
    }
}

impl Ground {
    #[inline]
    pub fn idx(&self, r: usize, c: usize) -> usize {
        r * self.cols + c
    }

    pub fn pos(&self, r: usize, c: usize) -> Vec3 {
        Vec3::new(self.x0 + c as f32 * CELL, self.h[r * self.cols + c], self.z0 + r as f32 * CELL)
    }

    /// Height of the ground surface (the triangles drawn and collided).
    pub fn height(&self, x: f32, z: f32) -> f32 {
        let fx = ((x - self.x0) / CELL).clamp(0.0, (self.cols - 1) as f32);
        let fz = ((z - self.z0) / CELL).clamp(0.0, (self.rows - 1) as f32);
        let c = (fx as usize).min(self.cols - 2);
        let r = (fz as usize).min(self.rows - 2);
        let (u, v) = (fx - c as f32, fz - r as f32);
        let h = |r: usize, c: usize| self.h[r * self.cols + c];
        if v >= u { h(r, c) + (h(r + 1, c) - h(r, c)) * v + (h(r + 1, c + 1) - h(r + 1, c)) * u } else { h(r, c) + (h(r, c + 1) - h(r, c)) * u + (h(r + 1, c + 1) - h(r, c + 1)) * v }
    }

    /// Cover of the triangle under a point.
    pub fn cover_at(&self, x: f32, z: f32) -> Cover {
        let fx = ((x - self.x0) / CELL).clamp(0.0, (self.cols - 1) as f32 - 1e-3);
        let fz = ((z - self.z0) / CELL).clamp(0.0, (self.rows - 1) as f32 - 1e-3);
        let (c, r) = (fx as usize, fz as usize);
        let (u, v) = (fx - c as f32, fz - r as f32);
        Cover::from_u8(self.cover[(r * (self.cols - 1) + c) * 2 + usize::from(v < u)])
    }

    /// Signed distance to the coast (positive on land).
    pub fn coast_at(&self, x: f32, z: f32) -> f32 {
        let c = (((x - self.x0) / CELL).round().max(0.0) as usize).min(self.cols - 1);
        let r = (((z - self.z0) / CELL).round().max(0.0) as usize).min(self.rows - 1);
        self.coast[r * self.cols + c]
    }

    pub fn normal(&self, r: usize, c: usize) -> Vec3 {
        let h = |r: usize, c: usize| self.h[r.min(self.rows - 1) * self.cols + c.min(self.cols - 1)];
        let dx = h(r, c + 1) - h(r, c.saturating_sub(1));
        let dz = h(r + 1, c) - h(r.saturating_sub(1), c);
        Vec3::new(-dx, 2.0 * CELL, -dz).normalize()
    }

    /// Tiles per side.
    pub fn tiles(&self) -> (usize, usize) {
        ((self.cols - 1) / TILE_CELLS, (self.rows - 1) / TILE_CELLS)
    }

    /// Builds the ground of the island: heights, coast and land cover.
    /// `roads` level the ground along them (bridges and tunnels do not).
    pub fn build(d: &IslandData) -> Ground {
        let coast: Vec<&super::data::Poly> = d.polys.iter().filter(|p| p.kind == PolyKind::Coast).collect();
        let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
        for p in &coast {
            let (a, b) = p.bounds();
            lo = lo.min(a);
            hi = hi.max(b);
        }
        if coast.is_empty() {
            lo = Vec2::new(d.height.extent[0], d.height.extent[1]);
            hi = Vec2::new(d.height.extent[2], d.height.extent[3]);
        }
        // Whole tiles, with a margin of sea bed around the island.
        let margin = 192.0;
        let x0 = ((lo.x - margin) / TILE).floor() * TILE;
        let z0 = ((lo.y - margin) / TILE).floor() * TILE;
        let cols = (((hi.x + margin - x0) / TILE).ceil() as usize).max(1) * TILE_CELLS + 1;
        let rows = (((hi.y + margin - z0) / TILE).ceil() as usize).max(1) * TILE_CELLS + 1;
        let n = cols * rows;
        // Land mask at the vertices.
        let mut inside = vec![false; n];
        for p in &coast {
            scan(&p.rings, x0, z0, 0.0, 0.0, CELL, cols, rows, |r, c0, c1| inside[r * cols + c0..r * cols + c1].fill(true));
        }
        if coast.is_empty() {
            inside.fill(true);
        }
        // Distance to the coastline: exact within two cells of it, then
        // propagated (chamfer).
        let mut dist = vec![f32::MAX; n];
        for p in &coast {
            for ring in &p.rings {
                for i in 0..ring.len() {
                    let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
                    let (mn, mx) = (a.min(b) - Vec2::splat(CELL * 2.0), a.max(b) + Vec2::splat(CELL * 2.0));
                    let c0 = ((mn.x - x0) / CELL).floor().max(0.0) as usize;
                    let c1 = (((mx.x - x0) / CELL).ceil().max(0.0) as usize).min(cols - 1);
                    let r0 = ((mn.y - z0) / CELL).floor().max(0.0) as usize;
                    let r1 = (((mx.y - z0) / CELL).ceil().max(0.0) as usize).min(rows - 1);
                    for r in r0..=r1 {
                        for c in c0..=c1 {
                            let q = Vec2::new(x0 + c as f32 * CELL, z0 + r as f32 * CELL);
                            let dd = seg_dist(q, a, b).0;
                            let e = &mut dist[r * cols + c];
                            *e = e.min(dd);
                        }
                    }
                }
            }
        }
        if coast.is_empty() {
            dist.fill(1.0e4);
        }
        let diag = CELL * std::f32::consts::SQRT_2;
        for pass in 0..2 {
            let fwd = pass == 0;
            for k in 0..n {
                let i = if fwd { k } else { n - 1 - k };
                let (r, c) = (i / cols, i % cols);
                let mut best = dist[i];
                let mut look = |dr: isize, dc: isize, w: f32| {
                    let (rr, cc) = (r as isize + dr, c as isize + dc);
                    if rr >= 0 && cc >= 0 && (rr as usize) < rows && (cc as usize) < cols {
                        let v = dist[rr as usize * cols + cc as usize];
                        if v < f32::MAX {
                            best = best.min(v + w);
                        }
                    }
                };
                let s = if fwd { -1 } else { 1 };
                look(0, s, CELL);
                look(s, 0, CELL);
                look(s, s, diag);
                look(s, -s, diag);
                dist[i] = best;
            }
        }
        for i in 0..n {
            if !inside[i] {
                dist[i] = -dist[i];
            }
        }
        // Land heights; the lagoons (no data inside the coast) take the
        // height of the nearest land.
        let mut h = vec![f32::NAN; n];
        let mut queue = std::collections::VecDeque::new();
        for r in 0..rows {
            for c in 0..cols {
                let i = r * cols + c;
                if inside[i]
                    && let Some(v) = d.height.land(x0 + c as f32 * CELL, z0 + r as f32 * CELL)
                {
                    h[i] = v;
                    queue.push_back(i);
                }
            }
        }
        while let Some(i) = queue.pop_front() {
            let (r, c) = (i / cols, i % cols);
            for (dr, dc) in [(0isize, 1isize), (0, -1), (1, 0), (-1, 0)] {
                let (rr, cc) = (r as isize + dr, c as isize + dc);
                if rr < 0 || cc < 0 || rr as usize >= rows || cc as usize >= cols {
                    continue;
                }
                let j = rr as usize * cols + cc as usize;
                if inside[j] && h[j].is_nan() {
                    h[j] = h[i];
                    queue.push_back(j);
                }
            }
        }
        for i in 0..n {
            if h[i].is_nan() {
                h[i] = 0.0;
            }
            // The designed relief fades out towards the coast, and stands
            // on the real terrain where there is some, else on the plain.
            let fade = smoothstep(0.0, 420.0, dist[i]);
            let (x, z) = (x0 + (i % cols) as f32 * CELL, z0 + (i / cols) as f32 * CELL);
            let (real, trust) = d.dem.as_ref().map_or((0.0, 0.0), |m| m.sample(x, z));
            let base = PLAIN + (real - PLAIN) * trust;
            h[i] = h[i] * fade + base.max(LAND_MIN) * smoothstep(0.0, 60.0, dist[i]);
        }
        // No cliffs: the designed relief has cut edges (the levelled
        // ground around the airport); slopes are limited to ~40 degrees
        // by lowering the high side.
        let talus = 0.85;
        for pass in 0..4 {
            let fwd = pass % 2 == 0;
            for k in 0..n {
                let i = if fwd { k } else { n - 1 - k };
                let (r, c) = (i / cols, i % cols);
                let s: isize = if fwd { -1 } else { 1 };
                let mut best = h[i];
                for (dr, dc, w) in [(0, s, CELL), (s, 0, CELL), (s, s, diag), (s, -s, diag)] {
                    let (rr, cc2) = (r as isize + dr, c as isize + dc);
                    if rr >= 0 && cc2 >= 0 && (rr as usize) < rows && (cc2 as usize) < cols {
                        let j = rr as usize * cols + cc2 as usize;
                        if inside[j] {
                            best = best.min(h[j] + w * talus);
                        }
                    }
                }
                h[i] = best;
            }
        }
        let mut g = Ground { x0, z0, cols, rows, h, cover: vec![Cover::Scrub as u8; (cols - 1) * (rows - 1) * 2], coast: dist };
        // Shore: the ground crosses the sea surface on the coastline, rises
        // from it to the land within some metres, and falls to the sea bed
        // beyond it.
        for i in 0..n {
            let dd = g.coast[i];
            g.h[i] = if dd >= 0.0 {
                // A smooth rise from the waterline (no kink where a road
                // runs down to a beach or a quay).
                let land = g.h[i].max(LAND_MIN);
                SEA + (land - SEA) * smoothstep(0.0, (land / 0.1).max(14.0), dd)
            } else {
                (SEA - 0.15 * dd * dd / (10.0 - dd)).max(SEA - 5.0)
            };
        }
        // Twice: the second pass smooths what the first left between
        // neighbouring roads.
        let mut road = vec![0.0f32; n];
        g.level_roads(&d.lines, &mut road);
        g.level_roads(&d.lines, &mut road);
        // Vertical curves: the road vertices relax towards their
        // neighbours (weighted by how much they are road, so the verges
        // blend out), which rounds the changes of grade a car would feel.
        for _ in 0..24 {
            let prev = g.h.clone();
            for r in 1..rows - 1 {
                for c in 1..cols - 1 {
                    let i = r * cols + c;
                    if road[i] > 0.0 {
                        let mean = (prev[i - 1] + prev[i + 1] + prev[i - cols] + prev[i + cols]) * 0.25;
                        g.h[i] += (mean - prev[i]) * 0.5 * road[i];
                    }
                }
            }
        }
        g.classify(d, &road);
        g
    }

    /// Levels the ground along the roads: a road keeps the smoothed height
    /// of the ground along its centreline, and the ground within a cell of
    /// it takes that height, so roads never tilt across or float.
    fn level_roads(&mut self, lines: &[Line], road: &mut [f32]) {
        let n = self.h.len();
        let mut sum = vec![0.0f32; n];
        let mut wsum = vec![0.0f32; n];
        let mut wmax = vec![0.0f32; n];
        // Every triangle a road touches must be all road: its corners are
        // up to a cell diagonal (plus half the road) from the centreline.
        let reach = CELL * 1.45;
        for l in lines {
            if l.bridge() || l.tunnel() || l.width < 3.5 || l.class == super::data::RoadClass::Subway {
                continue;
            }
            // Samples every half cell along the line.
            let mut pts: Vec<Vec2> = Vec::new();
            for w in l.pts.windows(2) {
                let len = w[0].distance(w[1]);
                let k = (len / (CELL * 0.5)).ceil().max(1.0) as usize;
                for s in 0..k {
                    pts.push(w[0].lerp(w[1], s as f32 / k as f32));
                }
            }
            pts.push(*l.pts.last().expect("two points"));
            let hs: Vec<f32> = pts.iter().map(|p| self.height(p.x, p.y)).collect();
            // Smoothed along the line (a window of ~100 m).
            let win = 6usize;
            let smooth: Vec<f32> = (0..hs.len())
                .map(|i| {
                    let (a, b) = (i.saturating_sub(win), (i + win + 1).min(hs.len()));
                    hs[a..b].iter().sum::<f32>() / (b - a) as f32
                })
                .collect();
            let (inner, outer) = (reach + l.width * 0.5, reach + l.width * 0.5 + CELL * 2.0);
            for (p, hr) in pts.iter().zip(&smooth) {
                let c0 = ((p.x - outer - self.x0) / CELL).ceil().max(0.0) as usize;
                let c1 = (((p.x + outer - self.x0) / CELL).floor().max(0.0) as usize).min(self.cols - 1);
                let r0 = ((p.y - outer - self.z0) / CELL).ceil().max(0.0) as usize;
                let r1 = (((p.y + outer - self.z0) / CELL).floor().max(0.0) as usize).min(self.rows - 1);
                for r in r0..=r1 {
                    for c in c0..=c1 {
                        let q = Vec2::new(self.x0 + c as f32 * CELL, self.z0 + r as f32 * CELL);
                        let w = 1.0 - smoothstep(inner, outer, q.distance(*p));
                        if w > 0.0 {
                            let i = r * self.cols + c;
                            sum[i] += hr * w;
                            wsum[i] += w;
                            wmax[i] = wmax[i].max(w);
                        }
                    }
                }
            }
        }
        for i in 0..n {
            if wsum[i] > 0.0 {
                self.h[i] += (sum[i] / wsum[i] - self.h[i]) * wmax[i];
                road[i] = road[i].max(wmax[i]);
            }
        }
    }

    /// `road`: how much each vertex belongs to a road (0..1); those are
    /// not lowered under water (a road beside or across water stays level).
    fn classify(&mut self, d: &IslandData, road: &[f32]) {
        let (cc, cr) = (self.cols - 1, self.rows - 1);
        let (x0, z0) = (self.x0, self.z0);
        // Lowest priority first.
        let order = |k: PolyKind| -> Option<u32> {
            use PolyKind::*;
            Some(match k {
                Urban => 0,
                Industrial => 1,
                Airfield => 2,
                Agriculture => 3,
                Green => 4,
                Park => 5,
                Forest => 6,
                Wetland => 7,
                Sports => 8,
                Rock => 9,
                Beach => 10,
                FloodBank => 11,
                FloodBed => 12,
                Apron => 13,
                Water | River | Lake | Reservoir => 14,
                Coast | Building => return None,
            })
        };
        let mut polys: Vec<(u32, usize)> = d.polys.iter().enumerate().filter_map(|(i, p)| order(p.kind).map(|o| (o, i))).collect();
        polys.sort();
        let mut cover = std::mem::take(&mut self.cover);
        for (_, pi) in polys {
            let p = &d.polys[pi];
            use PolyKind::*;
            let hash = (pi as u32).wrapping_mul(0x9e37_79b9) >> 16;
            let c = match p.kind {
                Urban => Cover::Paving,
                Industrial | Apron => Cover::Concrete,
                Airfield => Cover::FieldDry,
                Agriculture => match p.flags {
                    1 => Cover::Orchard,
                    2 => Cover::FieldSoil,
                    3 => Cover::FieldDry,
                    _ => [Cover::FieldGreen, Cover::FieldSoil, Cover::Orchard, Cover::FieldGreen, Cover::FieldDry, Cover::Orchard][(hash % 6) as usize],
                },
                Green => Cover::Grass,
                Park => Cover::Park,
                Forest => Cover::Forest,
                Wetland => {
                    if hash % 3 == 0 {
                        Cover::Wetland
                    } else {
                        Cover::Rice
                    }
                }
                Sports => Cover::Sport,
                Rock => Cover::Rock,
                Beach => Cover::Sand,
                FloodBank => Cover::Grass,
                FloodBed => Cover::Gravel,
                Water | River | Lake | Reservoir => {
                    // Small water is a sheet over the ground (see `roads`).
                    if super::data::ring_area(&p.rings[0]).abs() < super::roads::BASIN_MIN {
                        continue;
                    }
                    Cover::Water
                }
                Coast | Building => continue,
            } as u8;
            // Triangle 0 of a cell has its centroid at (1/3, 2/3), triangle
            // 1 at (2/3, 1/3).
            for (t, ox, oz) in [(0usize, 1.0 / 3.0, 2.0 / 3.0), (1, 2.0 / 3.0, 1.0 / 3.0)] {
                scan(&p.rings, x0, z0, ox, oz, CELL, cc, cr, |r, c0, c1| {
                    for cx in c0..c1 {
                        cover[(r * cc + cx) * 2 + t] = c;
                    }
                });
            }
        }
        // Built-up ground is paved: cells with buildings among neighbours
        // with buildings (a lone farmhouse keeps its field).
        let mut built = vec![0u8; cc * cr];
        for p in d.polys.iter().filter(|p| p.kind == PolyKind::Building) {
            let c = super::data::ring_centroid(&p.rings[0]);
            let (cx, cz) = (((c.x - x0) / CELL) as isize, ((c.y - z0) / CELL) as isize);
            if cx >= 0 && cz >= 0 && (cx as usize) < cc && (cz as usize) < cr {
                let e = &mut built[cz as usize * cc + cx as usize];
                *e = e.saturating_add(1);
            }
        }
        for r in 1..cr.saturating_sub(1) {
            for c in 1..cc.saturating_sub(1) {
                let mut near = 0u32;
                for dr in 0..3 {
                    for dc in 0..3 {
                        near += built[(r + dr - 1) * cc + c + dc - 1] as u32;
                    }
                }
                if near >= 4 {
                    for t in 0..2 {
                        let e = &mut cover[(r * cc + c) * 2 + t];
                        if !matches!(Cover::from_u8(*e), Cover::Water | Cover::Park | Cover::Sport | Cover::Sand | Cover::Concrete | Cover::Forest | Cover::Gravel) {
                            *e = Cover::Paving as u8;
                        }
                    }
                }
            }
        }
        // Water lies below its banks: every vertex of a water triangle is
        // lowered, so the water is level and the banks slope into it.
        let mut wet = vec![false; self.h.len()];
        for r in 0..cr {
            for c in 0..cc {
                for t in 0..2 {
                    if cover[(r * cc + c) * 2 + t] == Cover::Water as u8 {
                        let v = if t == 0 { [(r, c), (r + 1, c), (r + 1, c + 1)] } else { [(r, c), (r + 1, c + 1), (r, c + 1)] };
                        for (vr, vc) in v {
                            wet[vr * self.cols + vc] = true;
                        }
                    }
                }
            }
        }
        for i in 0..wet.len() {
            if wet[i] && self.coast[i] > 0.0 {
                if road[i] <= 0.0 {
                    self.h[i] -= WATER_DROP;
                }
            }
        }
        // Steep ground is rock; the sea bed and the shore are sand.
        for r in 0..cr {
            for c in 0..cc {
                let p = [self.pos(r, c), self.pos(r + 1, c), self.pos(r + 1, c + 1), self.pos(r, c + 1)];
                for t in 0..2 {
                    let (a, b, cc3) = if t == 0 { (p[0], p[1], p[2]) } else { (p[0], p[2], p[3]) };
                    let ny = (b - a).cross(cc3 - a).normalize_or(Vec3::Y).y;
                    let e = &mut cover[(r * cc + c) * 2 + t];
                    let cur = Cover::from_u8(*e);
                    let low = a.y.max(b.y).max(cc3.y) < SEA + 0.45;
                    if low && cur != Cover::Water {
                        *e = Cover::Sand as u8;
                    } else if ny < 0.72 && matches!(cur, Cover::Scrub | Cover::Grass | Cover::Forest | Cover::FieldDry | Cover::FieldGreen | Cover::FieldSoil | Cover::Orchard) {
                        *e = Cover::Rock as u8;
                    }
                }
            }
        }
        self.cover = cover;
    }

    /// Whether any vertex of the square of `s` cells at `(r, c)` is above
    /// the sea (cells entirely under it are not drawn: the sea covers them).
    fn dry(&self, r: usize, c: usize, s: usize) -> bool {
        let step = s.clamp(1, 4);
        for rr in (r..=r + s).step_by(step) {
            for c2 in (c..=c + s).step_by(step) {
                if self.h[rr * self.cols + c2] > SEA - 0.4 {
                    return true;
                }
            }
        }
        false
    }

    /// The near meshes of terrain tile `(tx, tz)`: one primitive per land
    /// cover present, at full resolution, with a skirt around the tile that
    /// hides the seam to a neighbour drawn as its far stand-in. Empty for a
    /// tile of open sea.
    pub fn tile_meshes(&self, tx: usize, tz: usize) -> Vec<(Cover, MeshData)> {
        const N: usize = TILE_CELLS;
        let (c_base, r_base) = (tx * N, tz * N);
        let cc = self.cols - 1;
        if !self.dry(r_base, c_base, N) {
            return Vec::new();
        }
        struct Part {
            map: Vec<u32>,
            mesh: MeshData,
        }
        let verts = (N + 1) * (N + 1) + 4 * (N + 1);
        let mut parts: Vec<Option<Part>> = (0..COVERS).map(|_| None).collect();
        let vert = |part: &mut Part, id: usize| -> u32 {
            if part.map[id] != u32::MAX {
                return part.map[id];
            }
            let (r, c, drop) = self.tile_vertex(r_base, c_base, N, 1, id);
            let p = self.pos(r, c);
            let i = part.mesh.positions.len() as u32;
            part.mesh.positions.push(p - Vec3::Y * drop);
            part.mesh.normals.push(self.normal(r, c));
            part.mesh.uvs.push(Vec2::new(p.x, p.z) / 6.0);
            part.mesh.tangents.push(Vec4::new(1.0, 0.0, 0.0, 1.0));
            part.map[id] = i;
            i
        };
        let mut tri = |cover: u8, ids: [usize; 3]| {
            let part = parts[cover as usize].get_or_insert_with(|| Part { map: vec![u32::MAX; verts], mesh: MeshData::default() });
            let v = ids.map(|id| vert(part, id));
            part.mesh.indices.extend(v);
        };
        for r in 0..N {
            for c in 0..N {
                if !self.dry(r_base + r, c_base + c, 1) {
                    continue;
                }
                let cell = ((r_base + r) * cc + c_base + c) * 2;
                Self::cell_tris(N, 1, r, c, self.cover[cell], self.cover[cell + 1], &mut tri);
            }
        }
        let mut out = Vec::new();
        for (k, part) in parts.into_iter().enumerate() {
            let Some(mut part) = part else { continue };
            part.mesh.aabb = forja_math::Aabb::from_points(part.mesh.positions.iter().copied());
            out.push((Cover::from_u8(k as u8), part.mesh));
        }
        out
    }

    /// Grid position and drop of local vertex `id` of a tile meshed with
    /// `n` cells of `step` grid cells per side: the `(n + 1)^2` surface
    /// vertices, then four rows of skirt vertices (north, south, west, east).
    fn tile_vertex(&self, r_base: usize, c_base: usize, n: usize, step: usize, id: usize) -> (usize, usize, f32) {
        const SKIRT: f32 = 30.0;
        if id < (n + 1) * (n + 1) {
            return (r_base + id / (n + 1) * step, c_base + id % (n + 1) * step, 0.0);
        }
        let k = id - (n + 1) * (n + 1);
        let (edge, i) = (k / (n + 1), k % (n + 1) * step);
        match edge {
            0 => (r_base, c_base + i, SKIRT),
            1 => (r_base + n * step, c_base + i, SKIRT),
            2 => (r_base + i, c_base, SKIRT),
            _ => (r_base + i, c_base + n * step, SKIRT),
        }
    }

    /// The two triangles of local cell `(r, c)` of an `n`-cell tile (cells
    /// of `s` local vertices) and the skirt quads on the tile's edges.
    fn cell_tris(n: usize, s: usize, r: usize, c: usize, ca: u8, cb: u8, tri: &mut impl FnMut(u8, [usize; 3])) {
        let g = |r: usize, c: usize| r * (n + 1) + c;
        let skirt = (n + 1) * (n + 1);
        tri(ca, [g(r, c), g(r + s, c), g(r + s, c + s)]);
        tri(cb, [g(r, c), g(r + s, c + s), g(r, c + s)]);
        if r == 0 {
            let (a, b) = (g(0, c), g(0, c + s));
            let (a2, b2) = (skirt + c, skirt + c + s);
            tri(cb, [a, b, b2]);
            tri(cb, [a, b2, a2]);
        }
        if r + s == n {
            let (a, b) = (g(n, c), g(n, c + s));
            let (a2, b2) = (skirt + (n + 1) + c, skirt + (n + 1) + c + s);
            tri(ca, [b, a, a2]);
            tri(ca, [b, a2, b2]);
        }
        if c == 0 {
            let (a, b) = (g(r, 0), g(r + s, 0));
            let (a2, b2) = (skirt + 2 * (n + 1) + r, skirt + 2 * (n + 1) + r + s);
            tri(ca, [b, a, a2]);
            tri(ca, [b, a2, b2]);
        }
        if c + s == n {
            let (a, b) = (g(r, n), g(r + s, n));
            let (a2, b2) = (skirt + 3 * (n + 1) + r, skirt + 3 * (n + 1) + r + s);
            tri(cb, [a, b, b2]);
            tri(cb, [a, b2, a2]);
        }
    }

    /// The far stand-in of terrain tile `(tx, tz)`: one mesh at half
    /// resolution (32 m cells) with three coarser index levels, textured
    /// with the tile's cover map ([`Ground::far_texture`], `uv` 0..1 over
    /// the tile). `None` for a tile of open sea.
    pub fn far_mesh(&self, tx: usize, tz: usize) -> Option<MeshData> {
        const STEP: usize = 2;
        const N: usize = TILE_CELLS / STEP;
        const LEVELS: usize = 4;
        let (c_base, r_base) = (tx * TILE_CELLS, tz * TILE_CELLS);
        if !self.dry(r_base, c_base, TILE_CELLS) {
            return None;
        }
        let verts = (N + 1) * (N + 1) + 4 * (N + 1);
        let mut map = vec![u32::MAX; verts];
        let mut mesh = MeshData::default();
        let mut levels: Vec<Vec<u32>> = vec![Vec::new(); LEVELS];
        // The tile's square of its cover atlas (the atlas repeats: stay
        // half a texel inside it).
        let size = (FAR_TEXELS * ATLAS_TILES) as f32;
        let inset = |tile: usize, f: f32| (((tile % ATLAS_TILES) as f32 + f) * FAR_TEXELS as f32).clamp(0.5, size - 0.5) / size;
        for (level, list) in levels.iter_mut().enumerate() {
            let s = 1usize << level;
            let mut tri = |_: u8, ids: [usize; 3]| {
                for id in ids {
                    if map[id] == u32::MAX {
                        let (r, c, drop) = self.tile_vertex(r_base, c_base, N, STEP, id);
                        let p = self.pos(r, c);
                        map[id] = mesh.positions.len() as u32;
                        // Half a metre low: roads (drawn a few centimetres over
                        // the full-resolution ground) stay above the stand-in.
                        mesh.positions.push(p - Vec3::Y * (drop + 0.6));
                        mesh.normals.push(self.normal(r, c));
                        mesh.uvs.push(Vec2::new(inset(tx, (c - c_base) as f32 / TILE_CELLS as f32), inset(tz, (r - r_base) as f32 / TILE_CELLS as f32)));
                        mesh.tangents.push(Vec4::new(1.0, 0.0, 0.0, 1.0));
                    }
                    list.push(map[id]);
                }
            };
            for r in (0..N).step_by(s) {
                for c in (0..N).step_by(s) {
                    if self.dry(r_base + r * STEP, c_base + c * STEP, s * STEP) {
                        Self::cell_tris(N, s, r, c, 0, 0, &mut tri);
                    }
                }
            }
        }
        let mut levels = levels.into_iter();
        mesh.indices = levels.next().unwrap_or_default();
        mesh.lods = levels.collect();
        mesh.aabb = forja_math::Aabb::from_points(mesh.positions.iter().copied());
        (!mesh.indices.is_empty()).then_some(mesh)
    }

    /// Cover atlas `(ax, az)`: the cover map of a square of
    /// [`ATLAS_TILES`] tiles a side, [`FAR_TEXELS`] texels per tile (8 m
    /// each), every texel the colour of the cover under it (`colors` by
    /// [`Cover`]); sRGB RGBA, `FAR_TEXELS * ATLAS_TILES` a side.
    pub fn cover_atlas(&self, ax: usize, az: usize, colors: &[[u8; 3]; COVERS]) -> Vec<u8> {
        let n = FAR_TEXELS * ATLAS_TILES;
        let texel = TILE / FAR_TEXELS as f32;
        let mut px = Vec::with_capacity(n * n * 4);
        let (wx, wz) = (self.x0 + (ax * ATLAS_TILES) as f32 * TILE, self.z0 + (az * ATLAS_TILES) as f32 * TILE);
        for j in 0..n {
            for i in 0..n {
                let c = colors[self.cover_at(wx + (i as f32 + 0.5) * texel, wz + (j as f32 + 0.5) * texel) as usize];
                // A little grain so large fields are not flat.
                let h = ((i * 7919 + j * 104_729 + ax * 31 + az * 17) as u32).wrapping_mul(0x9e37_79b9) >> 24;
                let k = 0.955 + h as f32 / 255.0 * 0.09;
                px.extend([(c[0] as f32 * k).min(255.0) as u8, (c[1] as f32 * k).min(255.0) as u8, (c[2] as f32 * k).min(255.0) as u8, 255]);
            }
        }
        px
    }
}

/// Texels per terrain tile side in the cover atlases.
pub const FAR_TEXELS: usize = 64;
/// Terrain tiles per cover atlas side.
pub const ATLAS_TILES: usize = 16;
