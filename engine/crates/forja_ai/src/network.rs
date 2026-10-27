//! Road network: directed lanes along streets, turn connectors through the
//! junction boxes with their conflict zones, approaches with stop lines
//! and controls, crosswalks, signal controllers, the pedestrian graph and
//! A* routing on the lane graph.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use forja_math::Vec2;

use crate::geom::{Polyline, bezier, right, segment_hit};
use crate::signals::{Aspect, PedSignal, Phase, SignalController};
use crate::spec::{Control, ParkingSpec, RoadGraphSpec};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Turn {
    Straight,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaneKind {
    Street { street: u32, forward: bool },
    Connector { node: u32, from: u32, to: u32, turn: Turn },
}

/// Two connectors whose swept paths overlap: `s` ranges on each.
#[derive(Clone, Copy, Debug)]
pub struct Conflict {
    pub other: u32,
    /// Both leave the same lane: they overlap from the start.
    pub sibling: bool,
    pub s_in: f32,
    pub s_out: f32,
    pub o_in: f32,
    pub o_out: f32,
}

#[derive(Clone, Debug)]
pub struct Lane {
    pub path: Polyline,
    pub kind: LaneKind,
    pub speed_limit: f32,
    /// Curvature-limited speed (connectors).
    pub max_speed: f32,
    pub width: f32,
    pub next: Vec<u32>,
    pub prev: Vec<u32>,
    /// Same-direction neighbours (driver's left / right).
    pub left: Option<u32>,
    pub right: Option<u32>,
    /// Approach this lane ends at (street lanes arriving at a junction).
    pub approach: Option<u32>,
    /// Stop line position along the lane (`len` when there is none).
    pub stop_s: f32,
    /// `(crosswalk, s0, s1)` crossed by this lane.
    pub crosswalks: Vec<(u32, f32, f32)>,
    pub conflicts: Vec<Conflict>,
}

impl Lane {
    pub fn len(&self) -> f32 {
        self.path.len()
    }

    pub fn is_connector(&self) -> bool {
        matches!(self.kind, LaneKind::Connector { .. })
    }

    pub fn street(&self) -> Option<u32> {
        match self.kind {
            LaneKind::Street { street, .. } => Some(street),
            _ => None,
        }
    }

    pub fn turn(&self) -> Turn {
        match self.kind {
            LaneKind::Connector { turn, .. } => turn,
            _ => Turn::Straight,
        }
    }
}

/// Traffic arriving at a junction from one street end.
#[derive(Clone, Debug)]
pub struct Approach {
    pub node: u32,
    pub street: u32,
    pub end: u8,
    pub control: Control,
    pub lanes: Vec<u32>,
    /// Travel direction at the stop line.
    pub dir: Vec2,
    pub stop_pos: Vec2,
    pub signal: Option<u32>,
    pub phase: Option<u8>,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub pos: Vec2,
    pub approaches: Vec<u32>,
    pub connectors: Vec<u32>,
    pub crosswalks: Vec<u32>,
    pub signal: Option<u32>,
    /// Street ends meeting here.
    pub ends: Vec<(u32, u8)>,
}

#[derive(Clone, Debug)]
pub struct Crosswalk {
    pub node: u32,
    pub street: u32,
    pub end: u8,
    /// Kerb to kerb centre line.
    pub a: Vec2,
    pub b: Vec2,
    pub width: f32,
    pub signal: Option<u32>,
    pub phase: Option<u8>,
    /// Flashing time before DON'T WALK (s).
    pub flash: f32,
}

impl Crosswalk {
    pub fn len(&self) -> f32 {
        self.a.distance(self.b)
    }

    pub fn is_empty(&self) -> bool {
        self.len() < 1e-3
    }

    /// Is `p` on the zebra (with `margin`)?
    pub fn contains(&self, p: Vec2, margin: f32) -> bool {
        let d = (self.b - self.a).normalize_or_zero();
        let rel = p - self.a;
        let u = rel.dot(d);
        let v = rel.perp_dot(d).abs();
        u > -margin && u < self.len() + margin && v < self.width * 0.5 + margin
    }
}

#[derive(Clone, Debug)]
pub struct Street {
    pub center: Polyline,
    pub nodes: [u32; 2],
    pub road_half: f32,
    pub sidewalk: f32,
    pub lanes: Vec<u32>,
    pub parking: Vec<ParkingSpec>,
    pub speed_limit: f32,
}

/// What a visible signal head shows: the approach it faces and the
/// crosswalk its pedestrian head serves (across that approach's street).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HeadBinding {
    pub approach: Option<u32>,
    pub crosswalk: Option<u32>,
}

/// Pedestrian graph edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WalkKind {
    Sidewalk { street: u32 },
    Crosswalk { crosswalk: u32 },
    Corner,
    Plaza,
}

#[derive(Clone, Debug)]
pub struct WalkEdge {
    pub a: u32,
    pub b: u32,
    pub kind: WalkKind,
    pub len: f32,
    /// Usable half width around the centre line.
    pub half: f32,
}

#[derive(Clone, Debug)]
pub struct WalkNode {
    pub pos: Vec2,
    pub edges: Vec<u32>,
    /// Direction from the road towards the buildings (shop windows).
    pub outward: Vec2,
}

#[derive(Clone, Debug, Default)]
pub struct PedGraph {
    pub nodes: Vec<WalkNode>,
    pub edges: Vec<WalkEdge>,
}

#[derive(Clone, Debug, Default)]
pub struct RoadNetwork {
    pub lanes: Vec<Lane>,
    pub approaches: Vec<Approach>,
    pub nodes: Vec<Node>,
    pub crosswalks: Vec<Crosswalk>,
    pub streets: Vec<Street>,
    pub signals: Vec<SignalController>,
    pub ped: PedGraph,
    pub plazas: Vec<(Vec2, Vec2)>,
    pub benches: Vec<(Vec2, f32)>,
    /// Lanes from which every other such lane can be reached (spawning
    /// and destinations stay inside it).
    pub strong: Vec<bool>,
    pub bounds: (Vec2, Vec2),
}

/// Distance under which two connector paths are in conflict (two car
/// widths with a margin).
const CONFLICT_DIST: f32 = 2.6;
pub const ALL_RED: f32 = 2.0;
pub const AMBER: f32 = 3.0;

impl RoadNetwork {
    pub fn build(spec: &RoadGraphSpec) -> Self {
        let mut net = RoadNetwork { plazas: spec.plazas.clone(), benches: spec.benches.clone(), ..Default::default() };
        net.nodes = spec.nodes.iter().map(|n| Node { pos: n.pos, approaches: Vec::new(), connectors: Vec::new(), crosswalks: Vec::new(), signal: None, ends: Vec::new() }).collect();
        // Street lanes, approaches and crosswalks.
        for (si, s) in spec.streets.iter().enumerate() {
            let center = Polyline::new(s.points.clone());
            let len = center.len();
            let mut lanes = Vec::new();
            for l in &s.lanes {
                let off = center.offset(l.offset);
                let path = if l.forward { off } else { off.reversed() };
                let id = net.lanes.len() as u32;
                net.lanes.push(Lane {
                    path,
                    kind: LaneKind::Street { street: si as u32, forward: l.forward },
                    speed_limit: s.speed_limit,
                    max_speed: s.speed_limit,
                    width: l.width,
                    next: Vec::new(),
                    prev: Vec::new(),
                    left: None,
                    right: None,
                    approach: None,
                    stop_s: 0.0,
                    crosswalks: Vec::new(),
                    conflicts: Vec::new(),
                });
                lanes.push(id);
            }
            for e in 0..2usize {
                let n = s.nodes[e] as u32;
                net.nodes[n as usize].ends.push((si as u32, e as u8));
                let end = &s.ends[e];
                let arriving: Vec<u32> = lanes.iter().copied().filter(|&l| matches!(net.lanes[l as usize].kind, LaneKind::Street { forward, .. } if forward == (e == 1))).collect();
                let leaving: Vec<u32> = lanes.iter().copied().filter(|&l| matches!(net.lanes[l as usize].kind, LaneKind::Street { forward, .. } if forward == (e == 0))).collect();
                if let Some(c) = end.crosswalk {
                    let st = if e == 0 { c } else { len - c };
                    let (p, d) = center.sample(st);
                    let r = right(d) * s.road_half;
                    let cw = net.crosswalks.len() as u32;
                    let cross = 2.0 * s.road_half / 1.2;
                    net.crosswalks.push(Crosswalk { node: n, street: si as u32, end: e as u8, a: p - r, b: p + r, width: end.crosswalk_width, signal: None, phase: None, flash: (cross - ALL_RED).max(4.0) });
                    net.nodes[n as usize].crosswalks.push(cw);
                    let w = end.crosswalk_width * 0.5;
                    for &l in &arriving {
                        let ll = net.lanes[l as usize].len();
                        net.lanes[l as usize].crosswalks.push((cw, ll - c - w, ll - c + w));
                    }
                    for &l in &leaving {
                        net.lanes[l as usize].crosswalks.push((cw, c - w, c + w));
                    }
                }
                if !arriving.is_empty() {
                    let a = net.approaches.len() as u32;
                    let first = &net.lanes[arriving[0] as usize];
                    let dir = first.path.end_dir();
                    let stop_s = (first.len() - end.stop_line).max(first.len() * 0.5);
                    for &l in &arriving {
                        let lane = &mut net.lanes[l as usize];
                        lane.approach = Some(a);
                        lane.stop_s = (lane.len() - end.stop_line).max(lane.len() * 0.5);
                    }
                    let stop_pos = center.point(if e == 1 { stop_s } else { len - stop_s });
                    net.approaches.push(Approach { node: n, street: si as u32, end: e as u8, control: end.control, lanes: arriving.clone(), dir, stop_pos, signal: None, phase: None });
                    net.nodes[n as usize].approaches.push(a);
                }
            }
            // Street lanes without an approach: no stop line.
            for &l in &lanes {
                let lane = &mut net.lanes[l as usize];
                if lane.approach.is_none() {
                    lane.stop_s = lane.len();
                }
            }
            // Same-direction neighbours, by position to the right of travel.
            for fwd in [true, false] {
                let mut group: Vec<(f32, u32)> = lanes
                    .iter()
                    .copied()
                    .filter(|&l| matches!(net.lanes[l as usize].kind, LaneKind::Street { forward, .. } if forward == fwd))
                    .map(|l| {
                        let p = &net.lanes[l as usize].path;
                        let mid = p.point(p.len() * 0.5);
                        let d = p.sample(p.len() * 0.5).1;
                        ((mid - center.point(len * 0.5)).dot(right(d)), l)
                    })
                    .collect();
                group.sort_by(|a, b| b.0.total_cmp(&a.0));
                for w in group.windows(2) {
                    if (w[0].0 - w[1].0).abs() < 4.6 {
                        net.lanes[w[0].1 as usize].left = Some(w[1].1);
                        net.lanes[w[1].1 as usize].right = Some(w[0].1);
                    }
                }
            }
            net.streets.push(Street { center, nodes: [s.nodes[0] as u32, s.nodes[1] as u32], road_half: s.road_half, sidewalk: s.sidewalk, lanes, parking: s.parking.clone(), speed_limit: s.speed_limit });
        }
        for ni in 0..net.nodes.len() {
            net.build_connectors(spec, ni);
        }
        for ni in 0..net.nodes.len() {
            net.build_conflicts(ni);
        }
        for ni in 0..net.nodes.len() {
            net.build_signal(spec, ni);
        }
        net.build_ped_graph();
        net.strong = net.strongly_connected();
        let mut lo = Vec2::splat(f32::MAX);
        let mut hi = Vec2::splat(f32::MIN);
        for l in &net.lanes {
            let (a, b) = l.path.bounds();
            lo = lo.min(a);
            hi = hi.max(b);
        }
        net.bounds = (lo, hi);
        net
    }

    fn build_connectors(&mut self, spec: &RoadGraphSpec, ni: usize) {
        let node = self.nodes[ni].clone();
        // Lanes leaving the node, per street end, rightmost first.
        let leaving_of = |net: &RoadNetwork, si: u32, e: u8| -> Vec<u32> {
            let mut v: Vec<(f32, u32)> = net.streets[si as usize]
                .lanes
                .iter()
                .copied()
                .filter(|&l| matches!(net.lanes[l as usize].kind, LaneKind::Street { forward, .. } if forward == (e == 0)))
                .map(|l| {
                    let p = &net.lanes[l as usize].path;
                    ((p.start() - node.pos).dot(right(p.start_dir())), l)
                })
                .collect();
            v.sort_by(|a, b| b.0.total_cmp(&a.0));
            v.into_iter().map(|x| x.1).collect()
        };
        let medians: Vec<(Vec2, Vec2)> = node
            .ends
            .iter()
            .filter(|(si, e)| spec.streets[*si as usize].median_half > 0.0 && !spec.streets[*si as usize].ends[*e as usize].median_open)
            .map(|(si, _)| (node.pos, self.streets[*si as usize].center.start_dir()))
            .collect();
        for &a in &node.approaches {
            let ap = self.approaches[a as usize].clone();
            let mut inbound: Vec<(f32, u32)> = ap
                .lanes
                .iter()
                .map(|&l| {
                    let p = &self.lanes[l as usize].path;
                    ((p.end() - node.pos).dot(right(ap.dir)), l)
                })
                .collect();
            inbound.sort_by(|x, y| y.0.total_cmp(&x.0));
            let inbound: Vec<u32> = inbound.into_iter().map(|x| x.1).collect();
            let ahead_only = spec.streets[ap.street as usize].ends[ap.end as usize].ahead_only;
            let mut moves: Vec<(u32, u32, Turn)> = Vec::new();
            for &(si, e) in &node.ends {
                if si == ap.street {
                    continue;
                }
                let out = leaving_of(self, si, e);
                if out.is_empty() {
                    continue;
                }
                let d_out = self.lanes[out[0] as usize].path.start_dir();
                let cos = ap.dir.dot(d_out);
                let side = d_out.dot(right(ap.dir));
                let turn = if cos > 0.7 {
                    Turn::Straight
                } else if side > 0.3 {
                    Turn::Right
                } else if side < -0.3 {
                    Turn::Left
                } else {
                    continue;
                };
                let (n, m) = (inbound.len(), out.len());
                match turn {
                    Turn::Straight => {
                        for (i, &l) in inbound.iter().enumerate() {
                            moves.push((l, out[i.min(m - 1)], turn));
                        }
                    }
                    Turn::Right => moves.push((inbound[0], out[0], turn)),
                    Turn::Left => moves.push((inbound[n - 1], out[m - 1], turn)),
                }
            }
            if ahead_only && moves.iter().any(|m| m.2 == Turn::Straight) {
                moves.retain(|m| m.2 == Turn::Straight);
            }
            for (from, to, turn) in moves {
                let p0 = self.lanes[from as usize].path.end();
                let d0 = self.lanes[from as usize].path.end_dir();
                let p3 = self.lanes[to as usize].path.start();
                let d3 = self.lanes[to as usize].path.start_dir();
                // No turning across a raised median.
                if medians.iter().any(|(c, d)| {
                    let s0 = (p0 - *c).perp_dot(*d);
                    let s3 = (p3 - *c).perp_dot(*d);
                    s0 * s3 < 0.0 && s0.abs() > 0.5 && s3.abs() > 0.5
                }) {
                    continue;
                }
                let dist = p0.distance(p3);
                let k = dist * if turn == Turn::Straight { 0.33 } else { 0.45 };
                let pts = bezier(p0, p0 + d0 * k, p3 - d3 * k, p3, ((dist / 1.2) as usize).clamp(6, 40));
                let path = Polyline::new(pts);
                let limit = self.lanes[from as usize].speed_limit.min(self.lanes[to as usize].speed_limit);
                let curv = path.max_curvature();
                let max_speed = if curv > 1e-3 { (2.8 / curv).sqrt().clamp(3.0, limit) } else { limit };
                let id = self.lanes.len() as u32;
                self.lanes.push(Lane {
                    path,
                    kind: LaneKind::Connector { node: ni as u32, from, to, turn },
                    speed_limit: limit,
                    max_speed,
                    width: self.lanes[from as usize].width,
                    next: vec![to],
                    prev: vec![from],
                    left: None,
                    right: None,
                    approach: None,
                    stop_s: f32::MAX,
                    crosswalks: Vec::new(),
                    conflicts: Vec::new(),
                });
                let l = self.lanes.len() - 1;
                self.lanes[l].stop_s = self.lanes[l].len();
                self.lanes[from as usize].next.push(id);
                self.lanes[to as usize].prev.push(id);
                self.nodes[ni].connectors.push(id);
            }
        }
    }

    fn build_conflicts(&mut self, ni: usize) {
        let conns = self.nodes[ni].connectors.clone();
        let samples: Vec<Vec<(f32, Vec2)>> = conns.iter().map(|&c| self.lanes[c as usize].path.resample(0.75)).collect();
        let bounds: Vec<(Vec2, Vec2)> = conns.iter().map(|&c| self.lanes[c as usize].path.bounds()).collect();
        for i in 0..conns.len() {
            for j in i + 1..conns.len() {
                let (LaneKind::Connector { from: fi, to: ti, .. }, LaneKind::Connector { from: fj, to: tj, .. }) = (self.lanes[conns[i] as usize].kind, self.lanes[conns[j] as usize].kind) else { continue };
                if fi == fj {
                    // Diverging from the same lane: overlapping until apart.
                    let (mut so, mut oo) = (0.0f32, 0.0f32);
                    for &(sa, pa) in &samples[i] {
                        for &(sb, pb) in &samples[j] {
                            if pa.distance_squared(pb) < CONFLICT_DIST * CONFLICT_DIST && (sa - sb).abs() < 4.0 {
                                so = so.max(sa);
                                oo = oo.max(sb);
                            }
                        }
                    }
                    self.lanes[conns[i] as usize].conflicts.push(Conflict { other: conns[j], sibling: true, s_in: 0.0, s_out: so, o_in: 0.0, o_out: oo });
                    self.lanes[conns[j] as usize].conflicts.push(Conflict { other: conns[i], sibling: true, s_in: 0.0, s_out: oo, o_in: 0.0, o_out: so });
                    continue;
                }
                let (a0, a1) = bounds[i];
                let (b0, b1) = bounds[j];
                if a1.x + CONFLICT_DIST < b0.x || b1.x + CONFLICT_DIST < a0.x || a1.y + CONFLICT_DIST < b0.y || b1.y + CONFLICT_DIST < a0.y {
                    continue;
                }
                let (mut si, mut so, mut oi, mut oo) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
                for &(sa, pa) in &samples[i] {
                    for &(sb, pb) in &samples[j] {
                        if pa.distance_squared(pb) < CONFLICT_DIST * CONFLICT_DIST {
                            si = si.min(sa);
                            so = so.max(sa);
                            oi = oi.min(sb);
                            oo = oo.max(sb);
                        }
                    }
                }
                // Merging into the same lane: the conflict lasts to the end.
                if ti == tj {
                    so = self.lanes[conns[i] as usize].len();
                    oo = self.lanes[conns[j] as usize].len();
                    si = si.min(so - 3.0);
                    oi = oi.min(oo - 3.0);
                }
                if si <= so {
                    self.lanes[conns[i] as usize].conflicts.push(Conflict { other: conns[j], sibling: false, s_in: si, s_out: so, o_in: oi, o_out: oo });
                    self.lanes[conns[j] as usize].conflicts.push(Conflict { other: conns[i], sibling: false, s_in: oi, s_out: oo, o_in: si, o_out: so });
                }
            }
        }
    }

    fn build_signal(&mut self, spec: &RoadGraphSpec, ni: usize) {
        let node = self.nodes[ni].clone();
        let signalled: Vec<u32> = node.approaches.iter().copied().filter(|&a| self.approaches[a as usize].control == Control::Signal).collect();
        if signalled.is_empty() {
            return;
        }
        // Approaches without a sign follow the lights; one with a STOP or
        // give-way plate keeps it and gives way to everyone with green.
        let mut phases: Vec<Phase> = Vec::new();
        let mut axes: Vec<Vec2> = Vec::new();
        for &a in &node.approaches {
            if matches!(self.approaches[a as usize].control, Control::Stop | Control::Yield) {
                continue;
            }
            self.approaches[a as usize].control = Control::Signal;
            let d = self.approaches[a as usize].dir;
            let p = match axes.iter().position(|x| x.dot(d).abs() > 0.7) {
                Some(p) => p,
                None => {
                    axes.push(d);
                    phases.push(Phase { approaches: Vec::new(), crosswalks: Vec::new(), green: 0.0 });
                    axes.len() - 1
                }
            };
            phases[p].approaches.push(a);
        }
        let sig = self.signals.len() as u32;
        let mut ped_only: Option<usize> = None;
        for &cw in &node.crosswalks {
            let c = &self.crosswalks[cw as usize];
            // Only zebras with a pedestrian head: those across a street
            // whose traffic arrives here under the lights.
            if !node.approaches.iter().any(|&a| self.approaches[a as usize].street == c.street && self.approaches[a as usize].control == Control::Signal) {
                continue;
            }
            let street_dir = self.streets[c.street as usize].center.start_dir();
            let p = match axes.iter().position(|x| x.dot(street_dir).abs() < 0.7) {
                Some(p) => p,
                None => *ped_only.get_or_insert_with(|| {
                    phases.push(Phase { approaches: Vec::new(), crosswalks: Vec::new(), green: 0.0 });
                    phases.len() - 1
                }),
            };
            phases[p].crosswalks.push(cw);
            let c = &mut self.crosswalks[cw as usize];
            c.signal = Some(sig);
            c.phase = Some(p as u8);
        }
        for ph in &mut phases {
            let wide = ph.approaches.iter().any(|&a| self.approaches[a as usize].lanes.len() >= 2);
            let base: f32 = if ph.approaches.is_empty() { 0.0 } else if wide { 24.0 } else { 16.0 };
            let ped = ph.crosswalks.iter().map(|&c| 6.0 + self.crosswalks[c as usize].flash).fold(0.0f32, f32::max);
            ph.green = base.max(ped).max(8.0);
            for &a in &ph.approaches {
                let p = phases_index(&axes, self.approaches[a as usize].dir);
                self.approaches[a as usize].signal = Some(sig);
                self.approaches[a as usize].phase = Some(p as u8);
            }
        }
        self.nodes[ni].signal = Some(sig);
        self.signals.push(SignalController { node: ni as u32, phases, amber: AMBER, all_red: ALL_RED, offset: spec.nodes[ni].signal_offset, forced: None });
    }

    /// Pedestrian graph: a kerb point per street end and side (on the
    /// sidewalk, level with the zebra), sidewalks between them, zebras
    /// across, corners around each junction, plazas.
    #[allow(clippy::needless_range_loop)]
    fn build_ped_graph(&mut self) {
        let mut g = PedGraph::default();
        let mut kerb = vec![[[u32::MAX; 2]; 2]; self.streets.len()];
        for (si, s) in self.streets.iter().enumerate() {
            if s.sidewalk <= 0.2 {
                continue;
            }
            let len = s.center.len();
            for e in 0..2 {
                let cw = self.crosswalks.iter().find(|c| c.street == si as u32 && c.end == e as u8);
                let end_pt = if e == 0 { s.center.start() } else { s.center.end() };
                let station = cw.map_or(3.0, |c| ((c.a + c.b) * 0.5).distance(end_pt));
                let st = if e == 0 { station.min(len * 0.5) } else { (len - station).max(len * 0.5) };
                let (p, d) = s.center.sample(st);
                for (k, side) in [1.0f32, -1.0].into_iter().enumerate() {
                    let n = crate::geom::left(d) * side;
                    let id = g.nodes.len() as u32;
                    g.nodes.push(WalkNode { pos: p + n * (s.road_half + s.sidewalk * 0.5), edges: Vec::new(), outward: n });
                    kerb[si][e][k] = id;
                }
            }
        }
        let add = |g: &mut PedGraph, a: u32, b: u32, kind: WalkKind, half: f32| {
            if a == u32::MAX || b == u32::MAX || a == b {
                return;
            }
            let len = g.nodes[a as usize].pos.distance(g.nodes[b as usize].pos);
            let id = g.edges.len() as u32;
            g.edges.push(WalkEdge { a, b, kind, len, half });
            g.nodes[a as usize].edges.push(id);
            g.nodes[b as usize].edges.push(id);
        };
        for (si, s) in self.streets.iter().enumerate() {
            for k in 0..2 {
                add(&mut g, kerb[si][0][k], kerb[si][1][k], WalkKind::Sidewalk { street: si as u32 }, (s.sidewalk * 0.5 - 0.3).max(0.2));
            }
        }
        for (ci, c) in self.crosswalks.iter().enumerate() {
            let k = kerb[c.street as usize][c.end as usize];
            add(&mut g, k[0], k[1], WalkKind::Crosswalk { crosswalk: ci as u32 }, c.width * 0.5 - 0.4);
        }
        for node in &self.nodes {
            // Street ends around the node by angle of their outward direction.
            let mut around: Vec<(f32, u32, u8, Vec2)> = node
                .ends
                .iter()
                .map(|&(si, e)| {
                    let c = &self.streets[si as usize].center;
                    let u = if e == 0 { c.start_dir() } else { -c.end_dir() };
                    (u.y.atan2(u.x), si, e, u)
                })
                .collect();
            if around.len() < 2 {
                continue;
            }
            around.sort_by(|a, b| a.0.total_cmp(&b.0));
            for i in 0..around.len() {
                let (_, si, ei, _) = around[i];
                let (_, sj, ej, _) = around[(i + 1) % around.len()];
                // Left of the outward direction is side 0 (`left(d)`) at end
                // 0 and side 1 at end 1, and vice versa for the right.
                let ki = if ei == 0 { 0 } else { 1 };
                let kj = if ej == 0 { 1 } else { 0 };
                add(&mut g, kerb[si as usize][ei as usize][ki], kerb[sj as usize][ej as usize][kj], WalkKind::Corner, 1.0);
            }
        }
        for (lo, hi) in self.plazas.clone() {
            let c = (lo + hi) * 0.5;
            let id = g.nodes.len() as u32;
            g.nodes.push(WalkNode { pos: c, edges: Vec::new(), outward: Vec2::ZERO });
            let mut near: Vec<(f32, u32)> = (0..id).map(|n| (g.nodes[n as usize].pos.distance(c), n)).filter(|(_, n)| {
                let p = g.nodes[*n as usize].pos;
                p.x > lo.x - 12.0 && p.x < hi.x + 12.0 && p.y > lo.y - 12.0 && p.y < hi.y + 12.0
            }).collect();
            near.sort_by(|a, b| a.0.total_cmp(&b.0));
            for (_, n) in near.into_iter().take(8) {
                add(&mut g, id, n, WalkKind::Plaza, 3.0);
            }
        }
        self.ped = g;
    }

    /// Lanes inside the largest strongly connected part of the lane graph.
    fn strongly_connected(&self) -> Vec<bool> {
        let n = self.lanes.len();
        if n == 0 {
            return Vec::new();
        }
        let reach = |fwd: bool, seed: usize| {
            let mut seen = vec![false; n];
            let mut stack = vec![seed];
            seen[seed] = true;
            while let Some(l) = stack.pop() {
                let lane = &self.lanes[l];
                let mut nb: Vec<u32> = if fwd { lane.next.clone() } else { lane.prev.clone() };
                nb.extend(lane.left.iter().chain(lane.right.iter()));
                // Lane changes are symmetric.
                for x in nb {
                    if !seen[x as usize] {
                        seen[x as usize] = true;
                        stack.push(x as usize);
                    }
                }
            }
            seen
        };
        let mut best = vec![false; n];
        let mut best_count = 0;
        let mut tried = vec![false; n];
        for seed in 0..n {
            if tried[seed] || self.lanes[seed].is_connector() {
                continue;
            }
            let f = reach(true, seed);
            let b = reach(false, seed);
            let comp: Vec<bool> = (0..n).map(|i| f[i] && b[i]).collect();
            let count = comp.iter().filter(|x| **x).count();
            for i in 0..n {
                if comp[i] {
                    tried[i] = true;
                }
            }
            if count > best_count {
                best_count = count;
                best = comp;
            }
            if best_count * 2 > n {
                break;
            }
        }
        best
    }

    /// Binds visible heads `(position, plate normal)` to approaches and the
    /// crosswalk of their pedestrian head (across the approach's street).
    pub fn bind_heads(&self, heads: &[(Vec2, Vec2)]) -> Vec<HeadBinding> {
        heads
            .iter()
            .map(|(p, f)| {
                let travel = -f.normalize_or_zero();
                let approach = self
                    .approaches
                    .iter()
                    .enumerate()
                    .filter(|(_, a)| a.signal.is_some() && a.dir.dot(travel) > 0.8 && a.stop_pos.distance(*p) < 20.0)
                    .min_by(|a, b| a.1.stop_pos.distance(*p).total_cmp(&b.1.stop_pos.distance(*p)))
                    .map(|(i, _)| i as u32);
                // The pedestrian head serves the zebra across the same street.
                let crosswalk = approach.and_then(|a| {
                    let ap = &self.approaches[a as usize];
                    self.nodes[ap.node as usize].crosswalks.iter().copied().find(|&c| self.crosswalks[c as usize].street == ap.street)
                });
                HeadBinding { approach, crosswalk }
            })
            .collect()
    }

    /// Aspect shown to an approach at time `t` (`Green` if unsignalled)
    /// and how long it lasts.
    pub fn approach_aspect(&self, a: u32, t: f64) -> (Aspect, f32) {
        let ap = &self.approaches[a as usize];
        match (ap.signal, ap.phase) {
            (Some(s), Some(p)) => self.signals[s as usize].aspect(p as usize, t),
            _ => (Aspect::Green, f32::INFINITY),
        }
    }

    /// Pedestrian signal of a crosswalk (`None` for an unsignalled zebra).
    pub fn crosswalk_signal(&self, c: u32, t: f64) -> Option<PedSignal> {
        let cw = &self.crosswalks[c as usize];
        match (cw.signal, cw.phase) {
            (Some(s), Some(p)) => Some(self.signals[s as usize].ped(p as usize, cw.flash, t)),
            _ => None,
        }
    }

    /// What a bound head shows: vehicle aspect and pedestrian signal.
    pub fn head_state(&self, h: HeadBinding, t: f64) -> (Aspect, PedSignal) {
        let a = h.approach.map_or(Aspect::Red, |a| self.approach_aspect(a, t).0);
        let p = h.crosswalk.and_then(|c| self.crosswalk_signal(c, t)).unwrap_or(PedSignal::DontWalk);
        (a, p)
    }

    /// Nearest crosswalk to `p` within `r` and the kerb end closest to `p`.
    pub fn crosswalk_near(&self, p: Vec2, r: f32) -> Option<(u32, Vec2, Vec2)> {
        self.crosswalks
            .iter()
            .enumerate()
            .filter_map(|(i, c)| {
                let (near, far) = if c.a.distance(p) < c.b.distance(p) { (c.a, c.b) } else { (c.b, c.a) };
                let d = near.distance(p);
                (d < r).then_some((d, i as u32, near, far))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|x| (x.1, x.2, x.3))
    }

    /// A* over the lane graph (connectors and lane changes) from the end
    /// of `from` to the end of `to`. Returns the lanes after `from`.
    pub fn route(&self, from: u32, to: u32) -> Option<Vec<u32>> {
        if from == to {
            return Some(Vec::new());
        }
        #[derive(PartialEq)]
        struct Item(f32, u32);
        impl Eq for Item {}
        impl Ord for Item {
            fn cmp(&self, o: &Self) -> Ordering {
                o.0.total_cmp(&self.0).then(o.1.cmp(&self.1))
            }
        }
        impl PartialOrd for Item {
            fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
                Some(self.cmp(o))
            }
        }
        let goal = self.lanes[to as usize].path.end();
        let h = |l: u32| self.lanes[l as usize].path.end().distance(goal) / 16.7;
        let n = self.lanes.len();
        let mut g = vec![f32::INFINITY; n];
        let mut came = vec![u32::MAX; n];
        let mut open = BinaryHeap::new();
        g[from as usize] = 0.0;
        open.push(Item(h(from), from));
        while let Some(Item(_, l)) = open.pop() {
            if l == to {
                let mut out = vec![to];
                let mut c = to;
                while came[c as usize] != from {
                    c = came[c as usize];
                    out.push(c);
                }
                out.reverse();
                return Some(out);
            }
            let lane = &self.lanes[l as usize];
            let gl = g[l as usize];
            let mut relax = |nb: u32, cost: f32, open: &mut BinaryHeap<Item>| {
                let ng = gl + cost;
                if ng < g[nb as usize] {
                    g[nb as usize] = ng;
                    came[nb as usize] = l;
                    open.push(Item(ng + h(nb), nb));
                }
            };
            for &nb in &lane.next {
                let nl = &self.lanes[nb as usize];
                relax(nb, nl.len() / nl.max_speed.max(2.0), &mut open);
            }
            // A lane change keeps the position along the street: it costs
            // a penalty, not the length of the parallel lane.
            if !lane.is_connector() {
                for nb in lane.left.iter().chain(lane.right.iter()) {
                    relax(*nb, 6.0, &mut open);
                }
            }
        }
        None
    }

    /// Lane closest to `p` heading roughly along `dir` (if non-zero).
    pub fn nearest_lane(&self, p: Vec2, dir: Vec2, max: f32) -> Option<(u32, f32, f32)> {
        let mut best: Option<(f32, u32, f32, f32)> = None;
        for (i, l) in self.lanes.iter().enumerate() {
            let (lo, hi) = l.path.bounds();
            if p.x < lo.x - max || p.x > hi.x + max || p.y < lo.y - max || p.y > hi.y + max {
                continue;
            }
            let (s, lat, d) = l.path.project(p);
            if d > max {
                continue;
            }
            if dir != Vec2::ZERO && l.path.sample(s).1.dot(dir) < 0.5 {
                continue;
            }
            if best.is_none_or(|b| d < b.0) {
                best = Some((d, i as u32, s, lat));
            }
        }
        best.map(|b| (b.1, b.2, b.3))
    }

    /// Total length of street lanes.
    pub fn lane_length(&self) -> f32 {
        self.lanes.iter().filter(|l| !l.is_connector()).map(|l| l.len()).sum()
    }

    /// Line segment test helper for the debug view.
    pub fn crosses(a0: Vec2, a1: Vec2, b0: Vec2, b1: Vec2) -> bool {
        segment_hit(a0, a1, b0, b1).is_some()
    }
}

fn phases_index(axes: &[Vec2], d: Vec2) -> usize {
    axes.iter().position(|x| x.dot(d).abs() > 0.7).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{GridOptions, grid_city};

    #[test]
    fn grid_network_is_connected_with_signals_and_crosswalks() {
        let net = RoadNetwork::build(&grid_city(&GridOptions::default()));
        let strong = net.strong.iter().filter(|x| **x).count();
        assert!(strong * 10 > net.lanes.len() * 9, "{strong} of {}", net.lanes.len());
        assert!(!net.signals.is_empty());
        assert!(net.crosswalks.len() > 50);
        // Every signalled crosswalk has a phase; every connector at a
        // 4-way signalled junction crossing another has a conflict.
        assert!(net.crosswalks.iter().filter(|c| c.signal.is_some()).all(|c| c.phase.is_some()));
        let conns: usize = net.lanes.iter().filter(|l| l.is_connector()).count();
        let with_conflicts = net.lanes.iter().filter(|l| l.is_connector() && l.conflicts.iter().any(|c| !c.sibling)).count();
        assert!(with_conflicts * 2 > conns, "{with_conflicts}/{conns}");
        assert!(!net.ped.edges.is_empty());
    }

    #[test]
    fn routes_are_valid_lane_sequences() {
        let net = RoadNetwork::build(&grid_city(&GridOptions::default()));
        let lanes: Vec<u32> = (0..net.lanes.len() as u32).filter(|&l| net.strong[l as usize] && !net.lanes[l as usize].is_connector()).collect();
        let mut rng = crate::rng::Rng::new(3);
        for _ in 0..200 {
            let a = lanes[rng.index(lanes.len())];
            let b = lanes[rng.index(lanes.len())];
            let r = net.route(a, b).unwrap_or_else(|| panic!("no route {a} -> {b}"));
            let mut cur = a;
            for &l in &r {
                let lane = &net.lanes[cur as usize];
                assert!(lane.next.contains(&l) || lane.left == Some(l) || lane.right == Some(l), "{cur} -> {l}");
                cur = l;
            }
            assert_eq!(cur, b);
        }
    }
}
