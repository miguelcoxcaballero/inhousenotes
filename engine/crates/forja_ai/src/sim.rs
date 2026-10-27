//! The traffic simulation: a fixed-timestep world of vehicles and
//! pedestrians on a [`RoadNetwork`].
//!
//! Every step: spawn/despawn around the focus (the player), rebuild the
//! lane occupancy and spatial hashes, let every driver decide from the
//! same snapshot (leader, corridor obstacles, stop line gate, crosswalks,
//! lane change), then integrate. Decisions never depend on update order,
//! so a seed always replays the same traffic.

use std::collections::VecDeque;
use std::time::Instant;

use forja_math::Vec2;

use crate::driver::{self, AmberChoice, DriverParams, EMERGENCY};
use crate::geom::{left, obb_distance, obb_overlap};
use crate::network::{LaneKind, RoadNetwork, Turn};
use crate::peds::{Ped, PedState};
use crate::rng::Rng;
use crate::signals::{Aspect, PedSignal};
use crate::spatial::SpatialHash;
use crate::spec::Control;

#[derive(Clone, Debug)]
pub struct SimConfig {
    /// Fixed step (s).
    pub dt: f32,
    pub seed: u64,
    pub max_vehicles: usize,
    /// Vehicles per km of lane inside the spawn radius.
    pub vehicle_density: f32,
    pub max_peds: usize,
    /// Pedestrians per km of sidewalk inside the spawn radius.
    pub ped_density: f32,
    /// Agents appear between these distances of the focus...
    pub spawn_min: f32,
    pub spawn_max: f32,
    /// ...and leave beyond this one.
    pub despawn: f32,
    /// Vehicles closer than this to the focus steer with pure pursuit;
    /// the others ride their lanes.
    pub near: f32,
    /// Pedestrians live within this distance of the focus (0: the vehicle
    /// spawn ring); new ones appear beyond 65% of it.
    pub ped_radius: f32,
    /// Probability that a pedestrian at a red man crosses anyway when no
    /// car is near.
    pub jaywalk: f32,
    /// Fraction of parking places with a parked car.
    pub parked_fill: f32,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            dt: 1.0 / 20.0,
            seed: 1,
            max_vehicles: 120,
            vehicle_density: 18.0,
            max_peds: 160,
            ped_density: 40.0,
            spawn_min: 70.0,
            spawn_max: 180.0,
            despawn: 210.0,
            near: 90.0,
            ped_radius: 0.0,
            jaywalk: 0.03,
            parked_fill: 0.55,
        }
    }
}

/// Something the AI must see but does not simulate: the player on foot or
/// in a car, physics cars, pedestrians of another crowd system.
#[derive(Clone, Copy, Debug)]
pub struct External {
    pub pos: Vec2,
    pub vel: Vec2,
    pub fwd: Vec2,
    /// Half width, half length.
    pub half: Vec2,
    pub kind: ExternalKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExternalKind {
    Player,
    PlayerCar,
    Car,
    /// A pedestrian; `waiting` at the kerb of this crosswalk to cross it.
    Ped { waiting: Option<u32> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentRef {
    Vehicle(u64),
    Ped(u64),
    External(u32),
    Parked(u32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SimEvent {
    Honk { vehicle: u64, pos: Vec2 },
    Collision { a: AgentRef, b: AgentRef, pos: Vec2 },
}

#[derive(Clone, Copy, Debug)]
pub struct ParkedCar {
    pub pos: Vec2,
    pub fwd: Vec2,
    pub len: f32,
    pub width: f32,
    pub style: u32,
}

#[derive(Clone, Debug)]
pub struct Vehicle {
    pub id: u64,
    pub lane: u32,
    /// Front bumper position along the lane.
    pub s: f32,
    pub v: f32,
    pub acc: f32,
    pub len: f32,
    pub width: f32,
    pub params: DriverParams,
    pub route: VecDeque<u32>,
    pub dest: u32,
    pub prev_lane: Option<u32>,
    /// Lateral offset from the lane centre (lane change in progress).
    pub lat: f32,
    /// Lane being left and the `s` offset on it.
    pub lc_from: Option<(u32, f32)>,
    pub gate_open: bool,
    /// Allowed through the stop line (permission latched).
    pub passed_line: bool,
    /// Front already over the stop line (with or without permission).
    pub crossed_line: bool,
    /// Connector reserved from the stop line until its conflict zones
    /// are cleared: conflicting connectors are never held together.
    pub holds: Option<u32>,
    pub stopped_at_sign: bool,
    pub stop_timer: f32,
    pub arrive_t: Option<f64>,
    pub amber: Option<AmberChoice>,
    pub wait_t: f32,
    pub stuck_t: f32,
    pub blocked_by: Option<AgentRef>,
    pub honk_cd: f32,
    pub lc_cd: f32,
    /// Pose: centre, heading, steering angle (pure pursuit) and the
    /// previous step's pose for render interpolation.
    pub pos: Vec2,
    pub fwd: Vec2,
    pub steer: f32,
    pub prev_pos: Vec2,
    pub prev_fwd: Vec2,
    pub near: bool,
    pub style: u32,
    pub braking: bool,
    /// -1 left, +1 right indicator.
    pub indicator: i8,
    /// Why the stop line is closed (`Go` when it is not).
    pub why: Why,
    pub odometer: f32,
}

impl Vehicle {
    pub fn half(&self) -> Vec2 {
        Vec2::new(self.width * 0.5, self.len * 0.5)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SimStats {
    pub steps: u64,
    pub vehicle_collisions: u32,
    pub ped_collisions: u32,
    pub player_collisions: u32,
    pub red_light_violations: u32,
    pub stop_sign_violations: u32,
    /// Entries into a connector while a conflicting one was occupied.
    pub box_violations: u32,
    pub jaywalks: u32,
    pub crossings: u32,
    pub spawned_vehicles: u32,
    pub despawned_vehicles: u32,
    pub stuck_removed: u32,
    pub distance: f64,
    pub last_step_us: f32,
    pub max_step_us: f32,
    pub total_step_us: f64,
    pub vehicles_us: f64,
    pub peds_us: f64,
}

impl SimStats {
    pub fn mean_step_us(&self) -> f32 {
        (self.total_step_us / self.steps.max(1) as f64) as f32
    }
}

/// Why a vehicle is held at its stop line (debug view, diagnostics).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Why {
    #[default]
    Go,
    NoRoute,
    LaneChange,
    Amber,
    Red,
    StopSign,
    Held,
    Box,
    Committed,
    Priority,
    ExitFull,
    PedExit,
    PedOwn,
}

#[derive(Clone, Copy, Debug, Default)]
struct Decision {
    why: Why,
    acc: f32,
    gate_open: bool,
    amber: Option<AmberChoice>,
    stopped_at_sign: bool,
    stop_timer: f32,
    arrive: Option<f64>,
    change_to: Option<u32>,
    reroute: bool,
    blocked_by: Option<AgentRef>,
    honk: bool,
}

/// An obstacle on the path: gap from the front bumper, its speed along the
/// path and who it is.
#[derive(Clone, Copy, Debug)]
struct Obstacle {
    gap: f32,
    v: f32,
    who: Option<AgentRef>,
}

pub struct TrafficSim {
    pub net: RoadNetwork,
    pub cfg: SimConfig,
    pub t: f64,
    pub vehicles: Vec<Vehicle>,
    pub peds: Vec<Ped>,
    pub parked: Vec<ParkedCar>,
    pub externals: Vec<External>,
    pub events: Vec<SimEvent>,
    pub stats: SimStats,
    pub focus: Vec2,
    pub rng: Rng,
    pub(crate) next_id: u64,
    pub(crate) occ: Vec<Vec<(f32, u32)>>,
    /// Vehicles holding each connector (reservation of its conflict zones).
    pub(crate) holders: Vec<u16>,
    dest_lanes: Vec<u32>,
    pub(crate) vgrid: SpatialHash,
    pub(crate) pgrid: SpatialHash,
    pub(crate) xgrid: SpatialHash,
    pub(crate) touching: Vec<(AgentRef, AgentRef)>,
    accumulator: f32,
    /// Last rule violations `(time, kind, vehicle, lane)` (bounded), for
    /// review and gameplay.
    pub violations: Vec<(f64, &'static str, u64, u32)>,
    /// Set by tests: spawn nothing automatically.
    pub manual: bool,
}

impl TrafficSim {
    pub fn new(net: RoadNetwork, cfg: SimConfig) -> Self {
        let n = net.lanes.len();
        let mut sim = TrafficSim {
            net,
            rng: Rng::new(cfg.seed),
            cfg,
            t: 0.0,
            vehicles: Vec::new(),
            peds: Vec::new(),
            parked: Vec::new(),
            externals: Vec::new(),
            events: Vec::new(),
            stats: SimStats::default(),
            focus: Vec2::ZERO,
            next_id: 1,
            occ: vec![Vec::new(); n],
            holders: vec![0; n],
            dest_lanes: Vec::new(),
            vgrid: SpatialHash::new(8.0),
            pgrid: SpatialHash::new(4.0),
            xgrid: SpatialHash::new(8.0),
            touching: Vec::new(),
            accumulator: 0.0,
            violations: Vec::new(),
            manual: false,
        };
        sim.place_parked();
        sim
    }

    fn place_parked(&mut self) {
        let mut out = Vec::new();
        for s in &self.net.streets {
            let len = s.center.len();
            for p in s.parking.iter().filter(|p| p.allowed) {
                let path = s.center.offset(p.offset);
                let mut t = 9.0;
                while t < len - 9.0 {
                    if self.rng.chance(self.cfg.parked_fill) {
                        let (pos, d) = path.sample(t);
                        // Parked facing the traffic of their side.
                        let fwd = if p.offset < 0.0 { d } else { -d };
                        out.push(ParkedCar { pos, fwd, len: 4.3, width: 1.8, style: self.rng.next_u64() as u32 });
                    }
                    t += 5.6;
                }
            }
        }
        self.parked = out;
    }

    /// Removes parked cars within `r` of `p` (to make room for something
    /// else placed there).
    pub fn clear_parked_near(&mut self, p: Vec2, r: f32) {
        self.parked.retain(|c| c.pos.distance(p) > r);
    }

    /// Interpolation factor between the last two steps.
    pub fn alpha(&self) -> f32 {
        (self.accumulator / self.cfg.dt).clamp(0.0, 1.0)
    }

    /// Runs as many fixed steps as `dt` of real time needs (at most 8).
    pub fn advance(&mut self, dt: f32) -> usize {
        self.accumulator += dt.min(0.25);
        let mut n = 0;
        while self.accumulator >= self.cfg.dt && n < 8 {
            self.accumulator -= self.cfg.dt;
            self.step();
            n += 1;
        }
        n
    }

    pub fn vehicle_index(&self, id: u64) -> Option<usize> {
        self.vehicles.binary_search_by(|v| v.id.cmp(&id)).ok()
    }

    /// One fixed step.
    pub fn step(&mut self) {
        let t0 = Instant::now();
        let dt = self.cfg.dt;
        if !self.manual && self.stats.steps.is_multiple_of(10) {
            self.spawn_and_despawn();
        }
        self.rebuild_indices();
        let tv = Instant::now();
        let decisions: Vec<Decision> = (0..self.vehicles.len()).map(|i| self.decide(i)).collect();
        for (i, d) in decisions.into_iter().enumerate() {
            self.apply(i, d, dt);
        }
        let before = self.vehicles.len();
        self.vehicles.retain(|v| v.lane != u32::MAX);
        if self.vehicles.len() != before {
            // The pedestrians query the vehicle grid: keep it in step.
            self.rebuild_indices();
        }
        let tp = Instant::now();
        self.stats.vehicles_us += (tp - tv).as_secs_f64() * 1e6;
        self.step_peds(dt);
        self.stats.peds_us += tp.elapsed().as_secs_f64() * 1e6;
        self.rebuild_indices();
        self.check_collisions();
        self.t += dt as f64;
        self.stats.steps += 1;
        let us = t0.elapsed().as_secs_f32() * 1e6;
        self.stats.last_step_us = us;
        self.stats.max_step_us = self.stats.max_step_us.max(us);
        self.stats.total_step_us += us as f64;
    }

    pub(crate) fn rebuild_indices(&mut self) {
        for o in &mut self.occ {
            o.clear();
        }
        for h in &mut self.holders {
            *h = 0;
        }
        for v in &self.vehicles {
            if let Some(h) = v.holds {
                self.holders[h as usize] += 1;
            }
        }
        for (i, v) in self.vehicles.iter().enumerate() {
            self.occ[v.lane as usize].push((v.s, i as u32));
            if let Some((l, ds)) = v.lc_from {
                self.occ[l as usize].push((v.s + ds, i as u32));
            }
        }
        for o in &mut self.occ {
            if o.len() > 1 {
                o.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            }
        }
        self.vgrid.rebuild(self.vehicles.iter().enumerate().map(|(i, v)| (i as u32, v.pos)));
        self.pgrid.rebuild(self.peds.iter().enumerate().map(|(i, p)| (i as u32, p.pos)));
        let np = self.parked.len() as u32;
        self.xgrid.rebuild(self.parked.iter().enumerate().map(|(i, c)| (i as u32, c.pos)).chain(self.externals.iter().enumerate().map(|(i, e)| (np + i as u32, e.pos))));
    }

    // ----------------------------------------------------------------
    // Spawning.

    fn spawn_and_despawn(&mut self) {
        let focus = self.focus;
        // Leave far away, or after being stuck for long out of sight.
        let before = self.vehicles.len();
        let despawn = self.cfg.despawn;
        let mut stuck = 0;
        self.vehicles.retain(|v| {
            let far = v.pos.distance(focus) > despawn;
            let gone = v.stuck_t > 90.0 && v.pos.distance(focus) > 60.0;
            if gone && !far {
                stuck += 1;
            }
            !(far || gone)
        });
        self.stats.stuck_removed += stuck;
        self.stats.despawned_vehicles += (before - self.vehicles.len()) as u32;
        let (ped_min, ped_max, ped_out) = if self.cfg.ped_radius > 0.0 { (self.cfg.ped_radius * 0.65, self.cfg.ped_radius, self.cfg.ped_radius + 15.0) } else { (self.cfg.spawn_min, self.cfg.spawn_max, despawn) };
        self.peds.retain(|p| p.pos.distance(focus) < ped_out);
        // Density budget from the lanes in reach.
        let (rmin, rmax) = (self.cfg.spawn_min, self.cfg.spawn_max);
        let first = self.stats.steps == 0;
        let cands: Vec<(u32, f32)> = self
            .net
            .lanes
            .iter()
            .enumerate()
            .filter(|(i, l)| !l.is_connector() && self.net.strong[*i] && l.len() > 25.0)
            .map(|(i, l)| (i as u32, l.len()))
            .filter(|(i, _)| self.net.lanes[*i as usize].path.point(self.net.lanes[*i as usize].len() * 0.5).distance(focus) < rmax)
            .collect();
        let lane_km: f32 = cands.iter().map(|c| c.1).sum::<f32>() / 1000.0;
        let want = ((lane_km * self.cfg.vehicle_density) as usize).min(self.cfg.max_vehicles);
        let mut tries = if first { want * 4 } else { 6 };
        while self.vehicles.len() < want && tries > 0 && !cands.is_empty() {
            tries -= 1;
            let (lane, len) = cands[self.rng.index(cands.len())];
            let s = self.rng.range(8.0, (len - 22.0).max(9.0));
            let p = self.net.lanes[lane as usize].path.point(s);
            let d = p.distance(focus);
            if d > rmax || (!first && d < rmin) {
                continue;
            }
            self.spawn_vehicle(lane, s);
        }
        let walk_km: f32 = self.net.ped.edges.iter().filter(|e| matches!(e.kind, crate::network::WalkKind::Sidewalk { .. })).filter(|e| self.net.ped.nodes[e.a as usize].pos.distance(focus) < ped_max).map(|e| e.len).sum::<f32>() / 1000.0;
        let want_p = ((walk_km * self.cfg.ped_density) as usize).min(self.cfg.max_peds);
        let mut tries = if first { want_p * 3 } else { 6 };
        while self.peds.len() < want_p && tries > 0 {
            tries -= 1;
            self.spawn_ped_random(if first { 0.0 } else { ped_min }, ped_max);
        }
    }

    /// Adds a vehicle on `lane` with its front at `s`, if there is room.
    /// Returns its id.
    pub fn spawn_vehicle(&mut self, lane: u32, s: f32) -> Option<u64> {
        let l = &self.net.lanes[lane as usize];
        let p = l.path.point(s);
        let clear = self.vehicles.iter().all(|v| v.pos.distance(p) > 14.0) && self.externals.iter().all(|e| e.pos.distance(p) > 10.0);
        if !clear {
            return None;
        }
        let kind = self.rng.f();
        let (len, width) = if kind < 0.12 { (5.3, 2.05) } else if kind < 0.35 { (3.9, 1.72) } else { (4.5, 1.85) };
        let mut params = DriverParams { speed_factor: self.rng.range(0.88, 1.03), headway: self.rng.range(1.1, 1.7), a_max: self.rng.range(1.3, 2.0), politeness: self.rng.range(0.1, 0.5), ..Default::default() };
        if kind < 0.12 {
            params.a_max *= 0.8;
        }
        let v0 = l.speed_limit * params.speed_factor * 0.6;
        let fwd = l.path.sample(s).1;
        let id = self.next_id;
        self.next_id += 1;
        let pos = l.path.point(s - len * 0.5);
        let mut veh = Vehicle {
            id,
            lane,
            s,
            v: v0,
            acc: 0.0,
            len,
            width,
            params,
            route: VecDeque::new(),
            dest: lane,
            prev_lane: None,
            lat: 0.0,
            lc_from: None,
            gate_open: true,
            passed_line: false,
            crossed_line: false,
            holds: None,
            stopped_at_sign: false,
            stop_timer: 0.0,
            arrive_t: None,
            amber: None,
            wait_t: 0.0,
            stuck_t: 0.0,
            blocked_by: None,
            honk_cd: 0.0,
            lc_cd: self.rng.range(0.0, 2.0),
            pos,
            fwd,
            steer: 0.0,
            prev_pos: pos,
            prev_fwd: fwd,
            near: false,
            style: self.rng.next_u64() as u32,
            braking: false,
            indicator: 0,
            why: Why::Go,
            odometer: 0.0,
        };
        if !self.new_destination(&mut veh) {
            return None;
        }
        self.vehicles.push(veh);
        self.stats.spawned_vehicles += 1;
        Some(id)
    }

    /// Picks a random destination in the connected part of the network
    /// and plans the route there from the vehicle's lane.
    fn new_destination(&mut self, v: &mut Vehicle) -> bool {
        if self.dest_lanes.is_empty() {
            self.dest_lanes = (0..self.net.lanes.len() as u32).filter(|&l| self.net.strong[l as usize] && !self.net.lanes[l as usize].is_connector()).collect();
        }
        for _ in 0..8 {
            if self.dest_lanes.is_empty() {
                break;
            }
            let d = self.dest_lanes[self.rng.index(self.dest_lanes.len())];
            if d == v.lane {
                continue;
            }
            if let Some(r) = self.net.route(v.lane, d)
                && !r.is_empty()
            {
                v.route = r.into();
                v.dest = d;
                return true;
            }
        }
        // Just follow the road.
        let next = &self.net.lanes[v.lane as usize].next;
        if next.is_empty() {
            return false;
        }
        let c = next[self.rng.index(next.len())];
        let mut route = vec![c];
        if let Some(&e) = self.net.lanes[c as usize].next.first() {
            route.push(e);
        }
        v.dest = *route.last().expect("route");
        v.route = route.into();
        true
    }

    /// Adds a vehicle at `s` on `lane` driving at `v` towards `dest` (tests
    /// and scripted scenes). Returns its id.
    pub fn spawn_vehicle_to(&mut self, lane: u32, s: f32, v: f32, dest: u32) -> Option<u64> {
        let route = self.net.route(lane, dest)?;
        let id = self.next_id;
        self.next_id += 1;
        let l = &self.net.lanes[lane as usize];
        let (len, width) = (4.5, 1.85);
        let pos = l.path.point(s - len * 0.5);
        let fwd = l.path.sample(s).1;
        let mut veh = placeholder();
        veh.id = id;
        veh.lane = lane;
        veh.s = s;
        veh.v = v;
        veh.len = len;
        veh.width = width;
        veh.route = route.into();
        veh.dest = dest;
        veh.pos = pos;
        veh.prev_pos = pos;
        veh.fwd = fwd;
        veh.prev_fwd = fwd;
        veh.style = id as u32;
        self.vehicles.push(veh);
        self.stats.spawned_vehicles += 1;
        Some(id)
    }

    /// Plans a route along explicit lanes (tests and scripted scenes).
    pub fn set_route(&mut self, id: u64, to: u32) -> bool {
        let Some(i) = self.vehicle_index(id) else { return false };
        let Some(r) = self.net.route(self.vehicles[i].lane, to) else { return false };
        self.vehicles[i].route = r.into();
        self.vehicles[i].dest = to;
        true
    }

    // ----------------------------------------------------------------
    // Decisions.

    /// Lanes ahead: `(lane, distance from the front bumper to its start)`.
    fn path_lanes(&self, v: &Vehicle, max: f32) -> Vec<(u32, f32)> {
        let mut out = vec![(v.lane, -v.s)];
        let mut acc = self.net.lanes[v.lane as usize].len() - v.s;
        let mut cur = v.lane;
        for &l in v.route.iter().take(5) {
            if acc > max {
                break;
            }
            // Lane changes are lateral, not ahead.
            if !self.net.lanes[cur as usize].next.contains(&l) {
                break;
            }
            out.push((l, acc));
            acc += self.net.lanes[l as usize].len();
            cur = l;
        }
        out
    }

    fn leader(&self, vi: usize, path: &[(u32, f32)], max: f32) -> Option<Obstacle> {
        let v = &self.vehicles[vi];
        let mut best: Option<Obstacle> = None;
        let consider = |best: &mut Option<Obstacle>, gap: f32, ov: &Vehicle| {
            if best.is_none_or(|b| gap < b.gap) {
                *best = Some(Obstacle { gap, v: ov.v, who: Some(AgentRef::Vehicle(ov.id)) });
            }
        };
        for (k, &(lane, start)) in path.iter().enumerate() {
            if start > max {
                break;
            }
            for &(so, oi) in &self.occ[lane as usize] {
                if oi as usize == vi {
                    continue;
                }
                let o = &self.vehicles[oi as usize];
                if start + so <= 0.0 && !(start + so == 0.0 && o.id > v.id) {
                    continue;
                }
                consider(&mut best, start + so - o.len, o);
                break;
            }
            // Vehicles on sibling connectors still overlapping the shared start.
            if k > 0 {
                for cf in self.net.lanes[lane as usize].conflicts.iter().filter(|c| c.sibling) {
                    for &(so, oi) in &self.occ[cf.other as usize] {
                        let o = &self.vehicles[oi as usize];
                        if so - o.len < cf.o_out && oi as usize != vi {
                            consider(&mut best, start + so - o.len, o);
                        }
                    }
                }
            }
            if best.is_some_and(|b| b.gap < start) {
                break;
            }
        }
        best
    }

    /// Samples along the path ahead: `(distance from the front, point,
    /// tangent)`.
    fn path_samples(&self, v: &Vehicle, path: &[(u32, f32)], max: f32) -> Vec<(f32, Vec2, Vec2)> {
        let mut out = Vec::with_capacity(32);
        let step = 1.5;
        let mut d = 0.0;
        while d <= max {
            // Find the lane containing distance d.
            let mut found = None;
            for (k, &(lane, start)) in path.iter().enumerate() {
                let len = self.net.lanes[lane as usize].len();
                if d >= start && (d < start + len || k + 1 == path.len()) {
                    found = Some((lane, d - start));
                    break;
                }
            }
            let Some((lane, s)) = found else { break };
            let (p, t) = self.net.lanes[lane as usize].path.sample(s);
            // A lane change in progress: the old position fades out.
            let p = if lane == v.lane { p + left(t) * v.lat * (1.0 - d / 12.0).max(0.0) } else { p };
            out.push((d, p, t));
            d += step;
        }
        out
    }

    /// The nearest thing in the swept corridor ahead that is not already
    /// the lane leader: vehicles crossing the box, pedestrians, the player.
    fn corridor(&self, vi: usize, samples: &[(f32, Vec2, Vec2)]) -> Option<Obstacle> {
        let v = &self.vehicles[vi];
        if samples.is_empty() {
            return None;
        }
        let half_w = v.width * 0.5 + 0.35;
        let front = samples[0].1;
        let mut best: Option<Obstacle> = None;
        let test = |c: Vec2, r: f32, vel: Vec2, who: AgentRef, best: &mut Option<Obstacle>| {
            if (c - front).dot(samples[0].2) < -r - 0.5 {
                return;
            }
            let lim = half_w + r;
            for &(d, p, t) in samples {
                if best.is_some_and(|b| d - r > b.gap) {
                    return;
                }
                if p.distance_squared(c) < lim * lim {
                    let gap = (d - r - (c - p).dot(t).max(0.0) * 0.0).max(0.0);
                    *best = Some(Obstacle { gap, v: vel.dot(t).max(0.0), who: Some(who) });
                    return;
                }
            }
        };
        let mut lo = Vec2::splat(f32::MAX);
        let mut hi = Vec2::splat(f32::MIN);
        for sm in samples {
            lo = lo.min(sm.1);
            hi = hi.max(sm.1);
        }
        let seen = {
            let mut seen: Vec<u32> = Vec::new();
            self.vgrid.query_box(lo - Vec2::splat(6.0), hi + Vec2::splat(6.0), |oi| {
                if oi as usize != vi {
                    seen.push(oi);
                }
            });
            seen
        };
        let coarse: Vec<Vec2> = samples.iter().step_by(4).map(|s| s.1).collect();
        for &oi in &seen {
            let o = &self.vehicles[oi as usize];
            // Far from every sample: cannot be in the corridor.
            if coarse.iter().all(|p| p.distance_squared(o.pos) > (o.len * 0.5 + half_w + 4.0).powi(2)) {
                continue;
            }
            // Two vehicles stuck seeing each other: the older one goes.
            if o.blocked_by == Some(AgentRef::Vehicle(v.id)) && v.blocked_by == Some(AgentRef::Vehicle(o.id)) && v.v < 0.3 && o.v < 0.3 && v.id < o.id {
                continue;
            }
            // Four discs covering the whole body, corners included.
            let q = o.len * 0.125;
            let r = (q * q + o.width * o.width * 0.25).sqrt();
            let vel = o.fwd * o.v;
            for k in [3.0f32, 1.0, -1.0, -3.0] {
                test(o.pos + o.fwd * (q * k), r, vel, AgentRef::Vehicle(o.id), &mut best);
            }
        }
        let mut peds: Vec<u32> = Vec::new();
        self.pgrid.query_box(lo - Vec2::splat(2.0), hi + Vec2::splat(2.0), |pi| peds.push(pi));
        for &pi in &peds {
            let p = &self.peds[pi as usize];
            test(p.pos, 0.35, p.vel, AgentRef::Ped(p.id), &mut best);
        }
        let np = self.parked.len() as u32;
        let mut xs: Vec<u32> = Vec::new();
        self.xgrid.query_box(lo - Vec2::splat(6.0), hi + Vec2::splat(6.0), |xi| {
            if xi >= np {
                xs.push(xi);
            }
        });
        for &xi in &xs {
            let e = &self.externals[(xi - np) as usize];
            let r = e.half.x.max(0.3);
            let off = (e.half.y - r).max(0.0);
            let who = AgentRef::External(xi - np);
            for c in [e.pos + e.fwd * off, e.pos, e.pos - e.fwd * off] {
                test(c, r + 0.2, e.vel, who, &mut best);
            }
        }
        best
    }

    /// Pedestrians on or about to step onto a crosswalk ahead: the gap to
    /// stop before it.
    fn crosswalk_yield(&self, v: &Vehicle, path: &[(u32, f32)], max: f32) -> Option<Obstacle> {
        let mut best: Option<Obstacle> = None;
        for &(lane, start) in path {
            for &(cw, s0, s1) in &self.net.lanes[lane as usize].crosswalks {
                let d0 = start + s0;
                // Already on it: the corridor handles anyone in front.
                if d0 < 0.5 || d0 > max {
                    continue;
                }
                if self.crosswalk_busy(cw, lane, (s0 + s1) * 0.5, v.wait_t > 6.0) {
                    let gap = d0 - 1.0;
                    // Too late to stop: do not slam the brakes on it.
                    if gap > 0.0 && v.v * v.v / (2.0 * gap.max(0.1)) > v.params.b_max {
                        continue;
                    }
                    if best.is_none_or(|b| gap < b.gap) {
                        best = Some(Obstacle { gap: gap.max(0.0), v: 0.0, who: None });
                    }
                }
            }
        }
        best
    }

    /// Someone is on the zebra near this lane, or waiting to cross an
    /// unsignalled one.
    pub(crate) fn crosswalk_busy(&self, cw: u32, lane: u32, s_mid: f32, patient: bool) -> bool {
        let c = &self.net.crosswalks[cw as usize];
        let at = self.net.lanes[lane as usize].path.point(s_mid);
        let signalled = c.signal.is_some();
        let mut busy = false;
        let centre = (c.a + c.b) * 0.5;
        self.pgrid.query(centre, c.len() * 0.5 + 3.0, |pi| {
            if busy {
                return;
            }
            let p = &self.peds[pi as usize];
            let on = c.contains(p.pos, 0.4);
            let toward = p.vel.dot(at - p.pos) > 0.1;
            let close = p.pos.distance(at) < 4.5 || (toward && p.pos.distance(at) < 9.0);
            match p.state {
                PedState::Cross { cw: pc, .. } if pc == cw && (close || on && c.len() < 12.0) => busy = true,
                PedState::Wait { cw: pc, .. } if pc == cw && !signalled && !patient => busy = true,
                _ if on && close => busy = true,
                _ => {}
            }
        });
        if !busy {
            for e in &self.externals {
                if let ExternalKind::Ped { waiting } = e.kind {
                    let on = c.contains(e.pos, 0.4);
                    if (on && e.pos.distance(at) < 6.0) || (!signalled && !patient && waiting == Some(cw)) {
                        return true;
                    }
                }
            }
        }
        busy
    }

    /// Decides whether `vi` may cross its stop line now.
    fn gate(&self, vi: usize, d: &mut Decision) -> bool {
        let v = &self.vehicles[vi];
        let lane = &self.net.lanes[v.lane as usize];
        let Some(a) = lane.approach else { return true };
        if lane.next.is_empty() {
            return true;
        }
        let Some(&conn) = v.route.front() else { d.why = Why::NoRoute; return false };
        if !lane.next.contains(&conn) {
            // Still needs a lane change: wait at the line.
            d.why = Why::LaneChange;
            return false;
        }
        let ap = &self.net.approaches[a as usize];
        let dist = lane.stop_s - v.s;
        if v.passed_line {
            return true;
        }
        // Control.
        match ap.control {
            Control::Signal => {
                let (aspect, left) = self.net.approach_aspect(a, self.t);
                match aspect {
                    Aspect::Green => d.amber = None,
                    Aspect::Amber => {
                        let choice = v.amber.unwrap_or_else(|| driver::amber_decision(dist, v.v, left, &v.params));
                        d.amber = Some(choice);
                        if choice == AmberChoice::Stop {
                            d.why = Why::Amber;
                            return false;
                        }
                    }
                    Aspect::Red => {
                        // Only a car that physically cannot stop goes on.
                        let need = v.v * v.v / (2.0 * dist.max(0.05));
                        if v.amber != Some(AmberChoice::Go) || need <= v.params.b_max {
                            d.amber = None;
                            d.why = Why::Red;
                            return false;
                        }
                    }
                }
            }
            Control::Stop => {
                if !v.stopped_at_sign {
                    if dist < 2.5 && v.v < 0.15 {
                        d.stop_timer = v.stop_timer + self.cfg.dt;
                        if d.stop_timer > 0.8 {
                            d.stopped_at_sign = true;
                            d.arrive = Some(self.t);
                        }
                    }
                    if !d.stopped_at_sign {
                        d.why = Why::StopSign;
                        return false;
                    }
                }
            }
            Control::Yield | Control::None => {}
        }
        self.right_of_way(vi, conn, dist, d)
    }

    /// Is a connector conflicting with `c` reserved by someone?
    pub(crate) fn held_conflict(&self, c: u32) -> bool {
        self.net.lanes[c as usize].conflicts.iter().any(|cf| self.holders[cf.other as usize] > 0)
    }

    /// Where a vehicle on connector `c` has cleared all its conflict zones.
    fn clear_s(&self, c: u32) -> f32 {
        self.net.lanes[c as usize].conflicts.iter().map(|cf| cf.s_out).fold(0.0, f32::max)
    }

    /// Priority key at a junction: higher goes first.
    fn key(&self, vi: usize, conn: u32, arrive: Option<f64>) -> (i32, i64, i64) {
        let v = &self.vehicles[vi];
        let lane = &self.net.lanes[v.lane as usize];
        let control = lane.approach.map_or(Control::None, |a| self.net.approaches[a as usize].control);
        let turn = self.net.lanes[conn as usize].turn();
        let rank = match control {
            Control::None => 30,
            Control::Signal => 20,
            Control::Yield => 10,
            Control::Stop => 0,
        } + if turn == Turn::Left { 0 } else { 2 };
        let committed = if v.passed_line { 100 } else { 0 };
        let arr = arrive.or(v.arrive_t).unwrap_or(self.t + 100.0);
        (rank + committed, -((arr * 10.0) as i64), -(v.id as i64))
    }

    fn right_of_way(&self, vi: usize, conn: u32, dist: f32, d: &mut Decision) -> bool {
        let v = &self.vehicles[vi];
        let c = &self.net.lanes[conn as usize];
        let my_key = self.key(vi, conn, d.arrive);
        if self.held_conflict(conn) {
            d.why = Why::Held;
            return false;
        }
        for cf in &c.conflicts {
            // The box: someone on the conflicting connector, or committed to it.
            for &(so, oi) in &self.occ[cf.other as usize] {
                let o = &self.vehicles[oi as usize];
                if so - o.len < cf.o_out + 0.5 {
                    d.why = Why::Box;
                    return false;
                }
            }
            let LaneKind::Connector { from, .. } = self.net.lanes[cf.other as usize].kind else { continue };
            let from_lane = &self.net.lanes[from as usize];
            for &(so, oi) in &self.occ[from as usize] {
                let o = &self.vehicles[oi as usize];
                if o.route.front() != Some(&cf.other) {
                    continue;
                }
                if o.passed_line {
                    d.why = Why::Committed;
                    return false;
                }
                if !o.gate_open {
                    continue;
                }
                let d_o = from_lane.len() - so + cf.o_in;
                if d_o > 60.0 {
                    continue;
                }
                if self.key(oi as usize, cf.other, None) <= my_key {
                    continue;
                }
                // Gap acceptance: the other must not arrive before we clear.
                let t_o = d_o / o.v.max(1.5);
                let clear = dist.max(0.0) + cf.s_out + v.len;
                let a = v.params.a_max.max(0.5);
                let t_me = (-v.v + (v.v * v.v + 2.0 * a * clear).sqrt()) / a;
                let critical: f32 = match c.turn() {
                    Turn::Left => 5.0,
                    Turn::Right => 3.5,
                    Turn::Straight => 4.5,
                };
                if t_o < critical.max(t_me + 1.5) {
                    d.why = Why::Priority;
                    return false;
                }
            }
        }
        // Do not enter the box without room on the far side.
        if let Some(&exit) = c.next.first() {
            let mut room = self.net.lanes[exit as usize].len();
            let mut slow = false;
            if let Some(&(so, oi)) = self.occ[exit as usize].first() {
                let o = &self.vehicles[oi as usize];
                room = so - o.len;
                slow = o.v < 3.0;
            }
            for &into in &self.net.lanes[exit as usize].prev {
                for &(_, oi) in &self.occ[into as usize] {
                    if oi as usize != vi {
                        room -= self.vehicles[oi as usize].len + 2.0;
                        slow = true;
                    }
                }
            }
            if slow && room < v.len + 2.5 {
                d.why = Why::ExitFull;
                return false;
            }
            // A short link to the next junction whose light is not green:
            // waiting there would leave us in this box.
            let el = &self.net.lanes[exit as usize];
            if let Some(a) = el.approach
                && el.stop_s < v.len + 3.0
                && self.net.approaches[a as usize].control == Control::Signal
                && self.net.approach_aspect(a, self.t).0 != Aspect::Green
            {
                d.why = Why::ExitFull;
                return false;
            }
            // Pedestrians on the crosswalk just past the box.
            for &(cw, s0, s1) in &self.net.lanes[exit as usize].crosswalks {
                if s0 < 8.0 && self.crosswalk_busy(cw, exit, (s0 + s1) * 0.5, v.wait_t > 6.0) {
                    d.why = Why::PedExit;
                    return false;
                }
            }
        }
        // Pedestrians on our own crosswalk between the line and the box.
        let lane = &self.net.lanes[v.lane as usize];
        for &(cw, s0, s1) in &lane.crosswalks {
            if s0 > lane.stop_s - 1.0 && self.crosswalk_busy(cw, v.lane, (s0 + s1) * 0.5, v.wait_t > 6.0) {
                d.why = Why::PedOwn;
                return false;
            }
        }
        true
    }

    fn decide(&self, vi: usize) -> Decision {
        let v = &self.vehicles[vi];
        let lane = &self.net.lanes[v.lane as usize];
        let mut d = Decision { gate_open: true, amber: v.amber, stopped_at_sign: v.stopped_at_sign, stop_timer: v.stop_timer, arrive: v.arrive_t, ..Default::default() };
        let look = (v.v * v.v / (2.0 * v.params.b_comf) + v.v * 2.0 + 15.0).clamp(25.0, 90.0);
        let path = self.path_lanes(v, look + 10.0);
        // Free road, speed limits, curves ahead.
        let v0 = (lane.speed_limit.min(lane.max_speed) * v.params.speed_factor).max(2.0);
        let mut acc = driver::idm_free(v.v, v0, &v.params);
        let mut blocked_by: Option<AgentRef> = None;
        for &(l, start) in path.iter().skip(1) {
            let nl = &self.net.lanes[l as usize];
            let vn = nl.max_speed.min(nl.speed_limit) * v.params.speed_factor;
            if v.v > vn && start > 0.0 {
                let a = -(v.v * v.v - vn * vn) / (2.0 * start.max(1.0));
                acc = acc.min(a.max(-v.params.b_comf * 1.5));
            }
        }
        let obstacle = |o: Option<Obstacle>, acc: &mut f32, blocked: &mut Option<AgentRef>| {
            if let Some(o) = o {
                let a = driver::idm(v.v, v0, o.gap, v.v - o.v, &v.params);
                if a < *acc {
                    *acc = a;
                    *blocked = o.who;
                }
            }
        };
        obstacle(self.leader(vi, &path, look), &mut acc, &mut blocked_by);
        let samples = self.path_samples(v, &path, look.min(45.0));
        obstacle(self.corridor(vi, &samples), &mut acc, &mut blocked_by);
        obstacle(self.crosswalk_yield(v, &path, look), &mut acc, &mut blocked_by);
        // Stop line.
        if lane.approach.is_some() && !v.passed_line && lane.stop_s - v.s < look + 5.0 {
            if v.arrive_t.is_none() && lane.stop_s - v.s < 15.0 {
                d.arrive = Some(self.t);
            }
            let open = self.gate(vi, &mut d);
            d.gate_open = open;
            if !open {
                let a = driver::stop_at(v.v, v0, lane.stop_s - v.s, &v.params);
                if a < acc {
                    acc = a;
                    blocked_by = None;
                }
            }
        }
        // The next stop line beyond this lane (short links between close
        // junctions): a light that is not green there is obeyed already.
        if let Some(&(l, start)) = path.iter().skip(1).find(|(l, _)| self.net.lanes[*l as usize].approach.is_some()) {
            let nl = &self.net.lanes[l as usize];
            let dist = start + nl.stop_s;
            if let Some(a) = nl.approach
                && dist > 0.0
                && dist < look + 5.0
                && self.net.approaches[a as usize].control == Control::Signal
            {
                let (aspect, left) = self.net.approach_aspect(a, self.t);
                let stop = match aspect {
                    Aspect::Green => false,
                    Aspect::Amber => driver::amber_decision(dist, v.v, left, &v.params) == AmberChoice::Stop,
                    Aspect::Red => v.v * v.v / (2.0 * dist) <= v.params.b_max,
                };
                if stop {
                    let a = driver::stop_at(v.v, v0, dist, &v.params);
                    if a < acc {
                        acc = a;
                        blocked_by = None;
                    }
                }
            }
        }
        // Route needs a lane change before the junction.
        if let Some(&next) = v.route.front()
            && (lane.left == Some(next) || lane.right == Some(next))
        {
            d.change_to = self.lane_change_ok(vi, next, true);
            if d.change_to.is_none() {
                let to_end = lane.stop_s.min(lane.len()) - v.s;
                if to_end < 25.0 {
                    let a = driver::stop_at(v.v, v0, to_end, &v.params);
                    acc = acc.min(a);
                }
                if v.wait_t > 8.0 {
                    d.reroute = true;
                }
            }
        } else if v.lc_cd <= 0.0 && v.lc_from.is_none() && !lane.is_connector() && (self.stats.steps + v.id).is_multiple_of(10) {
            // Discretionary (MOBIL) towards a lane that also leads where we go.
            if let Some(&conn) = v.route.front()
                && lane.next.contains(&conn)
            {
                let target_street = self.net.lanes[self.net.lanes[conn as usize].next[0] as usize].street();
                for nb in [lane.left, lane.right].into_iter().flatten() {
                    let ok = self.net.lanes[nb as usize].next.iter().any(|&c| self.net.lanes[self.net.lanes[c as usize].next[0] as usize].street() == target_street);
                    if ok && let Some(t) = self.lane_change_ok(vi, nb, false) {
                        d.change_to = Some(t);
                        break;
                    }
                }
            }
        }
        // Honk at the player blocking the way, or at a cut-in.
        if let Some(AgentRef::External(x)) = blocked_by
            && v.honk_cd <= 0.0
        {
            let e = &self.externals[x as usize];
            if matches!(e.kind, ExternalKind::Player | ExternalKind::PlayerCar) && ((v.v < 0.5 && v.wait_t > 2.5) || acc < -4.0) {
                d.honk = true;
            }
        }
        d.acc = acc.clamp(-EMERGENCY, v.params.a_max);
        d.blocked_by = blocked_by;
        d
    }

    /// Lane change to the neighbour `to`: safety for us and the new
    /// follower always; MOBIL incentive unless `mandatory`.
    fn lane_change_ok(&self, vi: usize, to: u32, mandatory: bool) -> Option<u32> {
        let v = &self.vehicles[vi];
        let lane = &self.net.lanes[v.lane as usize];
        if lane.is_connector() || v.lc_from.is_some() || v.s < 6.0 || v.s > lane.stop_s - 8.0 {
            return None;
        }
        let target = &self.net.lanes[to as usize];
        let my_pos = lane.path.point(v.s);
        let (s_t, _, _) = target.path.project(my_pos);
        if s_t > target.stop_s - 8.0 {
            return None;
        }
        let v0 = (lane.speed_limit * v.params.speed_factor).max(2.0);
        let mut lead: Option<(f32, u32)> = None;
        let mut foll: Option<(f32, u32)> = None;
        for &(so, oi) in &self.occ[to as usize] {
            if oi as usize == vi {
                continue;
            }
            if so > s_t {
                lead = Some((so, oi));
                break;
            }
            foll = Some((so, oi));
        }
        let me_new = match lead {
            Some((so, oi)) => {
                let o = &self.vehicles[oi as usize];
                let gap = so - o.len - s_t;
                if gap < 2.0 {
                    return None;
                }
                driver::idm(v.v, v0, gap, v.v - o.v, &v.params)
            }
            None => driver::idm_free(v.v, v0, &v.params),
        };
        let (newf_old, newf_new) = match foll {
            Some((so, oi)) => {
                let o = &self.vehicles[oi as usize];
                let gap = s_t - v.len - so;
                if gap < 2.5 {
                    return None;
                }
                let ov0 = (target.speed_limit * o.params.speed_factor).max(2.0);
                let old = match lead {
                    Some((sl, li)) => driver::idm(o.v, ov0, sl - self.vehicles[li as usize].len - so, o.v - self.vehicles[li as usize].v, &o.params),
                    None => driver::idm_free(o.v, ov0, &o.params),
                };
                (old, driver::idm(o.v, ov0, gap, o.v - v.v, &o.params))
            }
            None => (0.0, 0.0),
        };
        if newf_new < -v.params.b_safe || me_new < -v.params.b_safe {
            return None;
        }
        // Nothing beside us in the corridor of the target lane.
        let beside = target.path.point(s_t - v.len * 0.5);
        let mut blocked = false;
        self.vgrid.query(beside, 6.0, |oi| {
            if oi as usize != vi {
                let o = &self.vehicles[oi as usize];
                if obb_overlap(beside, target.path.sample(s_t).1, Vec2::new(v.width * 0.5 + 0.3, v.len * 0.5 + 1.0), o.pos, o.fwd, o.half()) {
                    blocked = true;
                }
            }
        });
        if blocked {
            return None;
        }
        if mandatory {
            return Some(to);
        }
        // Current situation.
        let mut me_old = driver::idm_free(v.v, v0, &v.params);
        for &(so, oi) in &self.occ[v.lane as usize] {
            if so > v.s && oi as usize != vi {
                let o = &self.vehicles[oi as usize];
                me_old = driver::idm(v.v, v0, so - o.len - v.s, v.v - o.v, &v.params);
                break;
            }
        }
        // Keep right unless overtaking.
        let bias = if lane.left == Some(to) { -0.1 } else { 0.1 };
        driver::mobil(me_old, me_new, newf_old, newf_new, 0.0, 0.0, &v.params, bias).then_some(to)
    }

    // ----------------------------------------------------------------
    // Integration.

    fn apply(&mut self, vi: usize, d: Decision, dt: f32) {
        let t = self.t;
        let near_r = self.cfg.near;
        let focus = self.focus;
        if d.honk {
            let v = &mut self.vehicles[vi];
            v.honk_cd = 6.0;
            self.events.push(SimEvent::Honk { vehicle: v.id, pos: v.pos });
        }
        let v = &mut self.vehicles[vi];
        v.honk_cd -= dt;
        v.lc_cd -= dt;
        v.acc = d.acc;
        v.gate_open = d.gate_open;
        v.amber = d.amber;
        v.stopped_at_sign = d.stopped_at_sign;
        v.stop_timer = d.stop_timer;
        v.arrive_t = d.arrive;
        v.blocked_by = d.blocked_by;
        v.why = if d.gate_open { Why::Go } else { d.why };
        v.braking = d.acc < -0.6 || v.v < 0.1;
        let v_old = v.v;
        v.v = (v.v + d.acc * dt).max(0.0);
        let ds = if v.v == 0.0 { 0.0 } else { (v_old + v.v) * 0.5 * dt };
        v.wait_t = if v.v < 0.3 { v.wait_t + dt } else { 0.0 };
        v.stuck_t = if v.v < 0.3 { v.stuck_t + dt } else { 0.0 };
        if d.reroute {
            v.wait_t = 0.0;
            v.route.clear();
        }
        // Lane change.
        if let Some(to) = d.change_to
            && v.lc_from.is_none()
        {
            let from = v.lane;
            let lane = &self.net.lanes[from as usize];
            let target = &self.net.lanes[to as usize];
            let p = lane.path.point(v.s);
            let (s_t, lat, _) = target.path.project(p);
            v.indicator = if lane.left == Some(to) { -1 } else { 1 };
            v.lc_from = Some((from, v.s - s_t));
            v.lane = to;
            v.s = s_t;
            v.lat = lat;
            v.lc_cd = 4.0;
            if v.route.front() == Some(&to) {
                v.route.pop_front();
            } else {
                // Discretionary: plan again from the new lane.
                v.route.clear();
            }
        }
        if v.lat != 0.0 {
            let step = 1.1 * dt;
            v.lat = if v.lat.abs() <= step { 0.0 } else { v.lat - step * v.lat.signum() };
            if v.lat.abs() < 0.9 {
                v.lc_from = None;
            }
            if v.lat == 0.0 {
                v.indicator = 0;
            }
        }
        // Stop line: latch the permission, and count anyone crossing the
        // line itself on red or without the full stop.
        let lane = &self.net.lanes[v.lane as usize];
        if let Some(a) = lane.approach {
            if !v.passed_line && d.gate_open && v.s + ds >= lane.stop_s - 0.2 {
                // Reserve the connector; a conflicting reservation made
                // earlier this step keeps us behind the line.
                let conn = v.route.front().copied().filter(|c| lane.next.contains(c));
                match conn {
                    Some(c) => {
                        if !self.net.lanes[c as usize].conflicts.iter().any(|cf| self.holders[cf.other as usize] > 0) {
                            v.passed_line = true;
                            v.holds = Some(c);
                            self.holders[c as usize] += 1;
                        }
                    }
                    None => v.passed_line = lane.next.is_empty(),
                }
            }
            if !v.crossed_line && v.s + ds >= lane.stop_s {
                v.crossed_line = true;
                let ap = &self.net.approaches[a as usize];
                match ap.control {
                    Control::Signal if self.net.approach_aspect(a, t).0 == Aspect::Red => {
                        self.stats.red_light_violations += 1;
                        self.violations.push((t, "red", v.id, v.lane));
                    }
                    Control::Stop if !v.stopped_at_sign => {
                        self.stats.stop_sign_violations += 1;
                        self.violations.push((t, "stop", v.id, v.lane));
                    }
                    _ => {}
                }
            }
        }
        v.s += ds;
        v.odometer += ds;
        self.stats.distance += ds as f64;
        // Next lanes.
        let mut guard = 0;
        loop {
            let (lane_id, s_now, next) = {
                let v = &self.vehicles[vi];
                (v.lane, v.s, v.route.front().copied())
            };
            let len = self.net.lanes[lane_id as usize].len();
            if s_now < len || guard >= 4 {
                break;
            }
            guard += 1;
            let Some(n) = next.filter(|n| self.net.lanes[lane_id as usize].next.contains(n)) else {
                // Dead end or lost: leave the simulation.
                self.vehicles[vi].lane = u32::MAX;
                return;
            };
            // Entering a connector: was its conflict zone free?
            if self.net.lanes[n as usize].is_connector() {
                for cf in &self.net.lanes[n as usize].conflicts {
                    for &(so, oi) in &self.occ[cf.other as usize] {
                        if oi as usize == vi {
                            continue;
                        }
                        let o = &self.vehicles[oi as usize];
                        if so > 0.0 && so - o.len < cf.o_out {
                            self.stats.box_violations += 1;
                            if self.violations.len() < 256 {
                                self.violations.push((self.t, "box", self.vehicles[vi].id, n));
                            }
                        }
                    }
                }
            }
            let v = &mut self.vehicles[vi];
            v.route.pop_front();
            v.prev_lane = Some(v.lane);
            v.lane = n;
            v.s -= len;
            v.passed_line = false;
            v.crossed_line = false;
            v.stopped_at_sign = false;
            v.stop_timer = 0.0;
            v.arrive_t = None;
            v.amber = None;
            v.lc_from = None;
            v.lat = 0.0;
        }
        // Release the reservation once clear of the junction's conflicts.
        if let Some(h) = self.vehicles[vi].holds {
            let clear = self.clear_s(h);
            let v = &self.vehicles[vi];
            let on_h = v.lane == h;
            let before = self.net.lanes[h as usize].prev.contains(&v.lane);
            let release = (on_h && v.s - v.len >= clear) || (!on_h && !before && (v.s - v.len >= 0.0 || !self.net.lanes[h as usize].next.contains(&v.lane)));
            if release {
                self.holders[h as usize] = self.holders[h as usize].saturating_sub(1);
                self.vehicles[vi].holds = None;
            }
        }
        // At the destination (or lost after a lane change): somewhere new.
        let needs_route = {
            let v = &self.vehicles[vi];
            v.route.is_empty() && v.dest != LEAVING
        };
        if needs_route {
            let mut veh = std::mem::replace(&mut self.vehicles[vi], placeholder());
            if !self.new_destination(&mut veh) {
                // Nowhere to go: drive to the end of the lane and leave.
                veh.dest = LEAVING;
            }
            self.vehicles[vi] = veh;
        }
        // Pose.
        let v = &mut self.vehicles[vi];
        v.prev_pos = v.pos;
        v.prev_fwd = v.fwd;
        let lane = &self.net.lanes[v.lane as usize];
        let (prev_lane, next_lane) = (v.prev_lane, v.route.front().copied());
        let point = |s: f32| -> (Vec2, Vec2) {
            if s < 0.0
                && let Some(p) = prev_lane
            {
                let pl = &self.net.lanes[p as usize];
                return pl.path.sample(pl.len() + s);
            }
            if s > lane.len()
                && let Some(n) = next_lane
            {
                return self.net.lanes[n as usize].path.sample(s - lane.len());
            }
            lane.path.sample(s)
        };
        let wb = v.len * 0.6;
        let rear_s = v.s - v.len * 0.8;
        let (rp, rt) = point(rear_s);
        let rail_rear = rp + left(rt) * v.lat;
        v.near = v.pos.distance(focus) < near_r;
        if v.near && v.odometer > 0.0 && v.prev_pos != Vec2::ZERO {
            // Pure pursuit on a kinematic bicycle, pulled back to the lane.
            let rear = v.pos - v.fwd * (v.len * 0.5 - v.len * 0.2);
            let ld = (v.v * 0.9).clamp(3.0, 12.0);
            let (gp, gt) = point(rear_s + ld);
            let goal = gp + left(gt) * v.lat;
            v.steer = driver::pure_pursuit(rear, v.fwd, wb, goal);
            let yaw_rate = v.v / wb * v.steer.tan();
            let fwd = forja_math::Vec2::from_angle(yaw_rate * dt).rotate(v.fwd).normalize_or(v.fwd);
            let mut rear = rear + fwd * ds;
            // Keep the logical and the steered positions together.
            if rear.distance(rail_rear) > 1.5 {
                rear = rail_rear;
            } else {
                rear += (rail_rear - rear) * 0.15;
            }
            v.fwd = fwd;
            v.pos = rear + fwd * (v.len * 0.5 - v.len * 0.2);
        } else {
            let (fp, ft) = point(v.s - v.len * 0.15);
            let front = fp + left(ft) * v.lat;
            v.fwd = (front - rail_rear).normalize_or(rt);
            v.pos = rail_rear + v.fwd * (v.len * 0.8 - v.len * 0.5);
            v.steer = 0.0;
        }
    }

    // ----------------------------------------------------------------
    // Collisions (statistics; the drivers try never to cause one).

    fn check_collisions(&mut self) {
        let mut now: Vec<(AgentRef, AgentRef, Vec2)> = Vec::new();
        for (i, v) in self.vehicles.iter().enumerate() {
            let h = v.half() * Vec2::new(0.9, 0.93);
            self.vgrid.query(v.pos, 6.0, |oi| {
                if (oi as usize) <= i {
                    return;
                }
                let o = &self.vehicles[oi as usize];
                if v.pos.distance(o.pos) < 7.0 && obb_overlap(v.pos, v.fwd, h, o.pos, o.fwd, o.half() * Vec2::new(0.9, 0.93)) {
                    now.push((AgentRef::Vehicle(v.id), AgentRef::Vehicle(o.id), (v.pos + o.pos) * 0.5));
                }
            });
            self.pgrid.query(v.pos, 4.0, |pi| {
                let p = &self.peds[pi as usize];
                if obb_distance(v.pos, v.fwd, h, p.pos) < 0.2 {
                    now.push((AgentRef::Vehicle(v.id), AgentRef::Ped(p.id), p.pos));
                }
            });
            for (xi, e) in self.externals.iter().enumerate() {
                if matches!(e.kind, ExternalKind::Player | ExternalKind::PlayerCar) && e.pos.distance(v.pos) < 6.0 && obb_overlap(v.pos, v.fwd, h, e.pos, e.fwd, e.half) {
                    now.push((AgentRef::Vehicle(v.id), AgentRef::External(xi as u32), e.pos));
                }
            }
        }
        for (a, b, p) in &now {
            if !self.touching.iter().any(|(x, y)| x == a && y == b) {
                match b {
                    AgentRef::Vehicle(_) => self.stats.vehicle_collisions += 1,
                    AgentRef::Ped(_) => self.stats.ped_collisions += 1,
                    _ => self.stats.player_collisions += 1,
                }
                self.events.push(SimEvent::Collision { a: *a, b: *b, pos: *p });
            }
        }
        self.touching = now.into_iter().map(|(a, b, _)| (a, b)).collect();
    }

    // ----------------------------------------------------------------
    // Queries for the game and for pedestrians.

    /// Time for the vehicles heading over crosswalk `cw` to reach it, and
    /// whether each could still stop comfortably: `true` if a pedestrian
    /// may step onto an unsignalled zebra now.
    pub fn zebra_clear(&self, cw: u32) -> bool {
        let c = &self.net.crosswalks[cw as usize];
        let centre = (c.a + c.b) * 0.5;
        let mut ok = true;
        // Courtesy: a driver kept waiting at this zebra gets a turn.
        let mut kept_waiting = false;
        self.vgrid.query(centre, 20.0, |vi| {
            let v = &self.vehicles[vi as usize];
            if v.wait_t > 6.0 && self.net.lanes[v.lane as usize].crosswalks.iter().any(|x| x.0 == cw && x.1 - v.s > -1.0 && x.1 - v.s < 12.0) {
                kept_waiting = true;
            }
        });
        if kept_waiting {
            return false;
        }
        self.vgrid.query(centre, 60.0, |vi| {
            if !ok {
                return;
            }
            let v = &self.vehicles[vi as usize];
            if v.pos.distance(centre) > 60.0 {
                return;
            }
            // Distance along its path to this crosswalk, if on it.
            let mut start = -v.s;
            let mut cur = v.lane;
            for k in 0..3 {
                let lane = &self.net.lanes[cur as usize];
                if let Some(&(_, s0, s1)) = lane.crosswalks.iter().find(|x| x.0 == cw) {
                    let d0 = start + s0;
                    if start + s1 + v.len > 0.0 && d0 < 1.0 {
                        ok = false; // on it
                        return;
                    }
                    let time = d0 / v.v.max(0.1);
                    let need = v.v * v.v / (2.0 * (d0 - 1.5).max(0.1));
                    if d0 > 0.0 && time < 4.0 && need > 2.5 {
                        ok = false;
                    }
                    return;
                }
                start += lane.len();
                let Some(&n) = v.route.get(k) else { return };
                cur = n;
            }
        });
        if ok {
            for e in &self.externals {
                if matches!(e.kind, ExternalKind::PlayerCar | ExternalKind::Car) && e.vel.length() > 2.0 {
                    let rel = centre - e.pos;
                    let closing = e.vel.dot(rel.normalize_or_zero());
                    if closing > 2.0 && rel.length() / closing < 4.5 {
                        return false;
                    }
                }
            }
        }
        ok
    }

    /// May a pedestrian start crossing `cw` now? Signalled: on WALK only
    /// (never on the flashing man), and not with a car on the zebra.
    /// Unsignalled: when every approaching car can stop comfortably.
    /// `jaywalk` lets a pedestrian cross against DON'T WALK when no car is
    /// within 45 m.
    pub fn crossing_allowed(&self, cw: u32, jaywalk: bool) -> bool {
        match self.net.crosswalk_signal(cw, self.t) {
            Some(PedSignal::Walk) => self.no_vehicle_on(cw),
            Some(_) => jaywalk && self.no_vehicle_near(cw, 45.0),
            None => self.zebra_clear(cw),
        }
    }

    fn no_vehicle_on(&self, cw: u32) -> bool {
        let c = &self.net.crosswalks[cw as usize];
        let mut ok = true;
        self.vgrid.query((c.a + c.b) * 0.5, c.len() * 0.5 + 6.0, |vi| {
            let v = &self.vehicles[vi as usize];
            if obb_distance(v.pos, v.fwd, v.half(), (c.a + c.b) * 0.5) < c.len() * 0.5 && c.contains(v.pos, v.len * 0.5 + 0.5) {
                ok = false;
            }
        });
        ok
    }

    fn no_vehicle_near(&self, cw: u32, r: f32) -> bool {
        let c = &self.net.crosswalks[cw as usize];
        let centre = (c.a + c.b) * 0.5;
        let mut ok = true;
        self.vgrid.query(centre, r, |vi| {
            if self.vehicles[vi as usize].pos.distance(centre) < r {
                ok = false;
            }
        });
        ok && self.externals.iter().all(|e| !matches!(e.kind, ExternalKind::PlayerCar | ExternalKind::Car) || e.pos.distance(centre) > r)
    }

    /// A deterministic digest of the agent state (for replay tests).
    pub fn digest(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut mix = |x: u64| {
            h ^= x;
            h = h.wrapping_mul(0x100_0000_01b3);
        };
        for v in &self.vehicles {
            mix(v.id);
            mix(v.lane as u64);
            mix(v.s.to_bits() as u64);
            mix(v.v.to_bits() as u64);
        }
        for p in &self.peds {
            mix(p.id);
            mix(p.pos.x.to_bits() as u64);
            mix(p.pos.y.to_bits() as u64);
        }
        h
    }
}

/// Destination of a vehicle that has nowhere to go and leaves at the end
/// of its lane.
pub const LEAVING: u32 = u32::MAX - 1;

fn placeholder() -> Vehicle {
    Vehicle {
        id: 0,
        lane: u32::MAX,
        s: 0.0,
        v: 0.0,
        acc: 0.0,
        len: 4.5,
        width: 1.8,
        params: DriverParams::default(),
        route: VecDeque::new(),
        dest: 0,
        prev_lane: None,
        lat: 0.0,
        lc_from: None,
        gate_open: true,
        passed_line: false,
        crossed_line: false,
        holds: None,
        stopped_at_sign: false,
        stop_timer: 0.0,
        arrive_t: None,
        amber: None,
        wait_t: 0.0,
        stuck_t: 0.0,
        blocked_by: None,
        honk_cd: 0.0,
        lc_cd: 0.0,
        pos: Vec2::ZERO,
        fwd: Vec2::Y,
        steer: 0.0,
        prev_pos: Vec2::ZERO,
        prev_fwd: Vec2::Y,
        near: false,
        style: 0,
        braking: false,
        indicator: 0,
        why: Why::Go,
        odometer: 0.0,
    }
}
