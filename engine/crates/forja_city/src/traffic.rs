//! What traffic AI needs to know about the streets, exported by the layout
//! from the very data it paints the markings and places the signs with:
//! street pieces with their lanes and parking strips, every sign plate and
//! every traffic light (with the module variants for each aspect, so the
//! game can show the state the drivers obey).
//!
//! The minimal description is a soup of straight pieces with no junction
//! topology: the AI then builds its own road graph from it
//! (`forja_ai::adapter`). A generator that knows its street graph (the
//! organic layout does) also fills [`TrafficInfo::nodes`] and every
//! street's `nodes` and `points`, and the AI uses that graph as is: curved
//! streets and junctions of any valence then need no guessing.

use forja_math::Vec2;

use crate::texgen::SignKind;

/// One driving lane: offset from the centreline (left of `a -> b`
/// positive), direction and width.
#[derive(Clone, Copy, Debug)]
pub struct TrafficLane {
    pub offset: f32,
    pub forward: bool,
    pub width: f32,
}

/// A straight street piece between building lines (`a`, `b` on the
/// centreline). Long avenues may cross several junctions.
#[derive(Clone, Debug)]
pub struct TrafficStreet {
    pub a: Vec2,
    pub b: Vec2,
    /// Building line to centreline.
    pub half_width: f32,
    /// Kerb to centreline.
    pub road_half: f32,
    pub sidewalk: f32,
    /// Half width of the raised median (0 if none).
    pub median_half: f32,
    pub lanes: Vec<TrafficLane>,
    /// Centre offsets of the parking strips.
    pub parking: Vec<f32>,
    /// Default limit before speed signs (km/h).
    pub speed_kmh: f32,
    /// Zebra centre and stop line, distance from each street end.
    pub crosswalk: Option<f32>,
    pub stop_line: f32,
    /// With topology, each end's own zebra and stop line (in place of
    /// `crosswalk` and `stop_line`).
    pub ends: Option<[TrafficEnd; 2]>,
    /// Junctions at `a` and `b` (indices into [`TrafficInfo::nodes`]) when
    /// the generator knows its topology.
    pub nodes: Option<[usize; 2]>,
    /// Centreline from `a` to `b` (both included) when the street curves;
    /// empty for a straight piece.
    pub points: Vec<Vec2>,
}

/// Where a street end's zebra centre and stop line are, from the end.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrafficEnd {
    pub crosswalk: Option<f32>,
    pub stop_line: f32,
}

impl TrafficStreet {
    /// The centreline: `points`, or the straight piece `a -> b`.
    pub fn centreline(&self) -> Vec<Vec2> {
        if self.points.len() >= 2 { self.points.clone() } else { vec![self.a, self.b] }
    }
}

/// Ground height on a regular grid (cell centres at `origin + (i, j) *
/// cell`), for placing vehicles on sloping streets.
#[derive(Clone, Debug, Default)]
pub struct HeightGrid {
    pub origin: Vec2,
    pub cell: f32,
    pub nx: usize,
    pub nz: usize,
    /// Row-major (`j * nx + i`).
    pub h: Vec<f32>,
}

impl HeightGrid {
    /// Samples `f` every `cell` metres over `[lo, hi]`.
    pub fn sample(lo: Vec2, hi: Vec2, cell: f32, f: impl Fn(Vec2) -> f32) -> Self {
        let n = ((hi - lo) / cell).ceil();
        let (nx, nz) = (n.x as usize + 1, n.y as usize + 1);
        let mut h = Vec::with_capacity(nx * nz);
        for j in 0..nz {
            for i in 0..nx {
                h.push(f(lo + Vec2::new(i as f32, j as f32) * cell));
            }
        }
        Self { origin: lo, cell, nx, nz, h }
    }

    /// Bilinear height at `p` (clamped to the grid; 0 on an empty grid).
    pub fn at(&self, p: Vec2) -> f32 {
        if self.nx == 0 || self.nz == 0 {
            return 0.0;
        }
        let q = ((p - self.origin) / self.cell).clamp(Vec2::ZERO, Vec2::new((self.nx - 1) as f32, (self.nz - 1) as f32));
        let (i, j) = ((q.x as usize).min(self.nx.saturating_sub(2)), (q.y as usize).min(self.nz.saturating_sub(2)));
        let (fx, fz) = (q.x - i as f32, q.y - j as f32);
        let g = |i: usize, j: usize| self.h[j.min(self.nz - 1) * self.nx + i.min(self.nx - 1)];
        let a = g(i, j) + (g(i + 1, j) - g(i, j)) * fx;
        let b = g(i, j + 1) + (g(i + 1, j + 1) - g(i, j + 1)) * fx;
        a + (b - a) * fz
    }
}

/// A sign plate: `facing` is its normal (towards the drivers it addresses).
#[derive(Clone, Copy, Debug)]
pub struct TrafficSign {
    pub kind: SignKind,
    pub pos: Vec2,
    pub facing: Vec2,
}

/// A traffic light placement (index into `CityOut::placements`) facing
/// the traffic it controls, with the module scene of every state:
/// `variants[aspect][walk]`, aspect 0 red, 1 amber, 2 green; `walk` is the
/// pedestrian head, which serves the zebra across this same street.
#[derive(Clone, Copy, Debug)]
pub struct SignalPost {
    pub pos: Vec2,
    pub facing: Vec2,
    pub placement: usize,
    pub variants: [[usize; 2]; 3],
}

#[derive(Clone, Debug, Default)]
pub struct TrafficInfo {
    pub streets: Vec<TrafficStreet>,
    pub signs: Vec<TrafficSign>,
    pub signals: Vec<SignalPost>,
    /// Plazas `(min, max)`.
    pub plazas: Vec<(Vec2, Vec2)>,
    /// Benches: position and the yaw they face.
    pub benches: Vec<(Vec2, f32)>,
    /// Centres of the openings in raised medians (turns across allowed).
    pub median_openings: Vec<Vec2>,
    /// Junction centres, when the streets carry their `nodes`.
    pub nodes: Vec<Vec2>,
    /// Roadway height where the ground is not flat (`None`: level at 0).
    pub ground: Option<HeightGrid>,
}

impl TrafficInfo {
    /// Height of the roadway at `p` (0 on flat maps).
    pub fn height_at(&self, p: Vec2) -> f32 {
        self.ground.as_ref().map_or(0.0, |g| g.at(p))
    }

    /// Whether every street names its junctions (the AI then uses this
    /// graph instead of rebuilding one from the pieces).
    pub fn has_topology(&self) -> bool {
        !self.streets.is_empty() && self.streets.iter().all(|s| s.nodes.is_some_and(|n| n.iter().all(|&i| i < self.nodes.len())))
    }
}

/// Plate normal of a module placed with `yaw` (furniture faces +Z).
pub fn facing_of_yaw(yaw: f32) -> Vec2 {
    Vec2::new(yaw.sin(), yaw.cos())
}
