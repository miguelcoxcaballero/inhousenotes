//! Abstract road description the AI is built from.
//!
//! The AI never reads a city generator directly: any street source (the
//! current grid layout, a planar street graph, an OpenStreetMap import)
//! implements [`RoadSource`] and hands out a [`RoadGraphSpec`]: junction
//! nodes, streets between them with explicit lanes, and what controls the
//! traffic arriving at each street end. [`crate::adapter`] turns a "soup"
//! of street segments plus the signs and signal heads actually placed in
//! the world into this graph, so what drivers obey is what the player sees.

use forja_math::Vec2;

/// What controls the traffic arriving at a junction from one street end.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Control {
    /// Priority road (no sign).
    None,
    /// Give way (R-1).
    Yield,
    /// Full stop, then go when clear (R-2).
    Stop,
    /// Traffic light.
    Signal,
}

/// One driving lane of a street, offset from the centreline (left of
/// `a -> b` positive); `forward` lanes run `a -> b`.
#[derive(Clone, Copy, Debug)]
pub struct LaneSpec {
    pub offset: f32,
    pub forward: bool,
    pub width: f32,
}

/// One end of a street, where it meets a junction.
#[derive(Clone, Copy, Debug)]
pub struct EndSpec {
    pub control: Control,
    /// Zebra crossing: distance of its centre line from the street end.
    pub crosswalk: Option<f32>,
    pub crosswalk_width: f32,
    /// Stop line distance from the street end.
    pub stop_line: f32,
    /// Only straight ahead allowed from here (R-400).
    pub ahead_only: bool,
    /// The street's raised median is open at this junction.
    pub median_open: bool,
}

impl Default for EndSpec {
    fn default() -> Self {
        Self { control: Control::None, crosswalk: None, crosswalk_width: 4.0, stop_line: 1.0, ahead_only: false, median_open: false }
    }
}

/// Parking strip along a street side (offset of its centre, left positive).
#[derive(Clone, Copy, Debug)]
pub struct ParkingSpec {
    pub offset: f32,
    pub allowed: bool,
}

/// A street between two junction nodes. `points` is the centreline from
/// the edge of the junction box at `nodes[0]` to the one at `nodes[1]`.
#[derive(Clone, Debug)]
pub struct StreetSpec {
    pub nodes: [usize; 2],
    pub points: Vec<Vec2>,
    /// Centreline to kerb.
    pub road_half: f32,
    pub sidewalk: f32,
    /// Half width of a raised median (no turns across it).
    pub median_half: f32,
    pub lanes: Vec<LaneSpec>,
    pub parking: Vec<ParkingSpec>,
    /// m/s.
    pub speed_limit: f32,
    pub ends: [EndSpec; 2],
}

#[derive(Clone, Copy, Debug)]
pub struct NodeSpec {
    pub pos: Vec2,
    /// Signal cycle offset (s), for green waves.
    pub signal_offset: f32,
}

#[derive(Clone, Debug, Default)]
pub struct RoadGraphSpec {
    pub nodes: Vec<NodeSpec>,
    pub streets: Vec<StreetSpec>,
    /// Open pedestrian areas: `(min, max)`.
    pub plazas: Vec<(Vec2, Vec2)>,
    /// Benches: position and facing yaw.
    pub benches: Vec<(Vec2, f32)>,
}

/// Anything that can describe its streets to the AI.
pub trait RoadSource {
    fn road_graph(&self) -> RoadGraphSpec;
}

impl RoadSource for RoadGraphSpec {
    fn road_graph(&self) -> RoadGraphSpec {
        self.clone()
    }
}

pub const KMH: f32 = 1.0 / 3.6;

/// Options of [`grid_city`], a synthetic Spanish-style grid for tests,
/// benchmarks and demos.
#[derive(Clone, Debug)]
pub struct GridOptions {
    pub cols: usize,
    pub rows: usize,
    pub block: f32,
    /// Building line to building line.
    pub street: f32,
    pub sidewalk: f32,
    /// Node columns `>= signal_from` get traffic lights; the others have
    /// STOP (streets along z) and give-way (along x) signs.
    pub signal_from: usize,
    /// Column of a two-lane-each-way avenue (along z), if any.
    pub avenue: Option<usize>,
}

impl Default for GridOptions {
    fn default() -> Self {
        Self { cols: 4, rows: 4, block: 60.0, street: 14.0, sidewalk: 3.0, signal_from: 2, avenue: Some(2) }
    }
}

/// Builds a grid city: two-way streets with one lane each way, an avenue
/// with two, signals in the east, STOP/give-way in the west, a zebra at
/// every street end.
pub fn grid_city(o: &GridOptions) -> RoadGraphSpec {
    let pitch = o.block + o.street;
    let nx = o.cols + 1;
    let nz = o.rows + 1;
    let id = |i: usize, j: usize| j * nx + i;
    let mut spec = RoadGraphSpec::default();
    let w = (o.cols as f32) * pitch * 0.5;
    let h = (o.rows as f32) * pitch * 0.5;
    for j in 0..nz {
        for i in 0..nx {
            let pos = Vec2::new(i as f32 * pitch - w, j as f32 * pitch - h);
            spec.nodes.push(NodeSpec { pos, signal_offset: (pos.y + h) / 12.0 + (pos.x + w) / 40.0 });
        }
    }
    let half = o.street * 0.5;
    let street = |a: usize, b: usize, along_z: bool, col: usize, spec: &mut RoadGraphSpec| {
        let pa = spec.nodes[a].pos;
        let pb = spec.nodes[b].pos;
        let avenue = along_z && o.avenue == Some(col);
        let bl = if avenue { half + 4.0 } else { half };
        let d = (pb - pa).normalize();
        // Cross streets meeting an avenue node are wider there.
        let trim = |n: usize| {
            let ni = n % nx;
            if !along_z && o.avenue == Some(ni) { half + 4.0 } else { half }
        };
        let points = vec![pa + d * trim(a), pb - d * trim(b)];
        let road_half = bl - o.sidewalk;
        let lanes = if avenue {
            vec![
                LaneSpec { offset: -1.7, forward: true, width: 3.3 },
                LaneSpec { offset: -5.0, forward: true, width: 3.3 },
                LaneSpec { offset: 1.7, forward: false, width: 3.3 },
                LaneSpec { offset: 5.0, forward: false, width: 3.3 },
            ]
        } else {
            vec![LaneSpec { offset: -road_half * 0.5, forward: true, width: 3.2 }, LaneSpec { offset: road_half * 0.5, forward: false, width: 3.2 }]
        };
        let control = |n: usize| {
            let ni = n % nx;
            if ni >= o.signal_from {
                Control::Signal
            } else if along_z {
                Control::Stop
            } else {
                Control::Yield
            }
        };
        let end = |n: usize| EndSpec { control: control(n), crosswalk: Some(3.0), crosswalk_width: 4.0, stop_line: 6.0, ahead_only: false, median_open: false };
        spec.streets.push(StreetSpec {
            nodes: [a, b],
            points,
            road_half,
            sidewalk: o.sidewalk,
            median_half: 0.0,
            lanes,
            parking: Vec::new(),
            speed_limit: if avenue { 50.0 * KMH } else if col >= o.signal_from { 40.0 * KMH } else { 30.0 * KMH },
            ends: [end(a), end(b)],
        });
    };
    for j in 0..nz {
        for i in 0..o.cols {
            street(id(i, j), id(i + 1, j), false, i, &mut spec);
        }
    }
    for i in 0..nx {
        for j in 0..o.rows {
            street(id(i, j), id(i, j + 1), true, i, &mut spec);
        }
    }
    // Corner nodes of the outer ring and edge T-junctions: signals only
    // where three or more streets meet.
    let mut degree = vec![0; spec.nodes.len()];
    for s in &spec.streets {
        degree[s.nodes[0]] += 1;
        degree[s.nodes[1]] += 1;
    }
    for s in &mut spec.streets {
        for e in 0..2 {
            if degree[s.nodes[e]] <= 2 {
                s.ends[e].control = Control::None;
            }
        }
    }
    spec
}
