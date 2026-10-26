//! Pedestrians: routes on the sidewalk graph, waiting at the kerb until
//! the walk signal (or a safe gap at a zebra), crossing, everyday
//! activities (waiting, chatting in pairs, sitting on benches, looking in
//! shop windows), fleeing from danger, and social-force local avoidance.
//! Each pedestrian names the animation clip that matches its speed.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use forja_math::Vec2;

use crate::geom::{obb_distance, right};
use crate::network::WalkKind;
use crate::signals::PedSignal;
use crate::sim::{ExternalKind, TrafficSim};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activity {
    Idle,
    Wait,
    Talk,
    Sit,
    Window,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PedState {
    Walk,
    /// At the kerb of crosswalk `cw`.
    Wait { cw: u32 },
    /// On crosswalk `cw` towards the far kerb node.
    Cross { cw: u32, to: u32 },
    Activity { kind: Activity, left: f32 },
    Flee { from: Vec2, left: f32 },
}

/// Clip names of the sandbox's motion capture set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PedAnim {
    Idle,
    Walking,
    WalkSlow,
    Running,
    Talk,
    Wait,
}

impl PedAnim {
    pub fn clip(self) -> &'static str {
        match self {
            PedAnim::Idle => "Idle",
            PedAnim::Walking => "Walking",
            PedAnim::WalkSlow => "WalkSlow",
            PedAnim::Running => "Running",
            PedAnim::Talk => "Talk",
            PedAnim::Wait => "Wait",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Ped {
    pub id: u64,
    pub pos: Vec2,
    pub vel: Vec2,
    pub heading: Vec2,
    pub cruise: f32,
    pub state: PedState,
    /// Walk nodes still to visit.
    pub path: Vec<u32>,
    /// Last walk node reached.
    pub from: u32,
    /// Keep-right fraction of the sidewalk half width.
    pub keep: f32,
    pub partner: Option<u64>,
    pub target: Option<Vec2>,
    pub facing: Option<Vec2>,
    pub jaywalker: bool,
    pub wait_t: f32,
    /// Where an activity takes place (kept there).
    pub anchor: Vec2,
}

impl Ped {
    /// Animation consistent with speed and activity.
    pub fn anim(&self) -> PedAnim {
        let sp = self.vel.length();
        if sp > 2.3 {
            PedAnim::Running
        } else if sp > 0.95 {
            PedAnim::Walking
        } else if sp > 0.25 {
            PedAnim::WalkSlow
        } else {
            match self.state {
                PedState::Activity { kind: Activity::Talk, .. } => PedAnim::Talk,
                PedState::Activity { kind: Activity::Wait, .. } | PedState::Wait { .. } => PedAnim::Wait,
                _ => PedAnim::Idle,
            }
        }
    }

    pub fn is_sitting(&self) -> bool {
        matches!(self.state, PedState::Activity { kind: Activity::Sit, .. }) && self.target.is_none()
    }
}

#[derive(Clone, Copy, Default)]
struct Plan {
    vdes: Vec2,
    max_speed: f32,
    /// Lateral clamp: segment and half width.
    corridor: Option<(Vec2, Vec2, f32)>,
}

impl TrafficSim {
    /// A* on the pedestrian graph: nodes after `from`.
    pub fn walk_route(&self, from: u32, to: u32) -> Option<Vec<u32>> {
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
        let g = &self.net.ped;
        if from == to {
            return Some(Vec::new());
        }
        let goal = g.nodes[to as usize].pos;
        let mut cost = vec![f32::INFINITY; g.nodes.len()];
        let mut came = vec![u32::MAX; g.nodes.len()];
        let mut open = BinaryHeap::new();
        cost[from as usize] = 0.0;
        open.push(Item(0.0, from));
        while let Some(Item(_, n)) = open.pop() {
            if n == to {
                let mut out = vec![to];
                let mut c = to;
                while came[c as usize] != from {
                    c = came[c as usize];
                    out.push(c);
                }
                out.reverse();
                return Some(out);
            }
            for &e in &g.nodes[n as usize].edges {
                let edge = &g.edges[e as usize];
                let m = if edge.a == n { edge.b } else { edge.a };
                // Crossing costs a little extra (waiting).
                let extra = if matches!(edge.kind, WalkKind::Crosswalk { .. }) { 8.0 } else { 0.0 };
                let c = cost[n as usize] + edge.len + extra;
                if c < cost[m as usize] {
                    cost[m as usize] = c;
                    came[m as usize] = n;
                    open.push(Item(c + g.nodes[m as usize].pos.distance(goal), m));
                }
            }
        }
        None
    }

    fn edge_between(&self, a: u32, b: u32) -> Option<&crate::network::WalkEdge> {
        self.net.ped.nodes[a as usize].edges.iter().map(|&e| &self.net.ped.edges[e as usize]).find(|e| (e.a == a && e.b == b) || (e.a == b && e.b == a))
    }

    /// A random destination 40-200 m away and the way there.
    fn new_walk(&mut self, from: u32) -> Vec<u32> {
        let n = self.net.ped.nodes.len();
        let here = self.net.ped.nodes[from as usize].pos;
        for _ in 0..10 {
            let d = self.rng.index(n) as u32;
            let dist = self.net.ped.nodes[d as usize].pos.distance(here);
            if !(40.0..200.0).contains(&dist) {
                continue;
            }
            if let Some(r) = self.walk_route(from, d) {
                return r;
            }
        }
        // Anywhere reachable next door.
        self.net.ped.nodes[from as usize].edges.first().map(|&e| {
            let e = &self.net.ped.edges[e as usize];
            vec![if e.a == from { e.b } else { e.a }]
        }).unwrap_or_default()
    }

    /// Adds a pedestrian on a random sidewalk between `rmin` and `rmax` of
    /// the focus.
    pub(crate) fn spawn_ped_random(&mut self, rmin: f32, rmax: f32) -> Option<u64> {
        let edges: Vec<u32> = (0..self.net.ped.edges.len() as u32)
            .filter(|&e| {
                let ed = &self.net.ped.edges[e as usize];
                if !matches!(ed.kind, WalkKind::Sidewalk { .. }) {
                    return false;
                }
                let m = (self.net.ped.nodes[ed.a as usize].pos + self.net.ped.nodes[ed.b as usize].pos) * 0.5;
                let d = m.distance(self.focus);
                d >= rmin && d <= rmax
            })
            .collect();
        if edges.is_empty() {
            return None;
        }
        let e = self.net.ped.edges[edges[self.rng.index(edges.len())] as usize].clone();
        let (a, b) = if self.rng.chance(0.5) { (e.a, e.b) } else { (e.b, e.a) };
        let t = self.rng.range(0.1, 0.9);
        let pa = self.net.ped.nodes[a as usize].pos;
        let pb = self.net.ped.nodes[b as usize].pos;
        let pos = pa + (pb - pa) * t;
        if self.peds.iter().any(|p| p.pos.distance(pos) < 1.0) {
            return None;
        }
        self.spawn_ped(pos, a, b)
    }

    /// Adds a pedestrian at `pos` walking from node `a` towards `b`, then
    /// on to a random destination.
    pub fn spawn_ped(&mut self, pos: Vec2, a: u32, b: u32) -> Option<u64> {
        let mut path = vec![b];
        path.extend(self.new_walk(b));
        let id = self.next_id;
        self.next_id += 1;
        let cruise = self.rng.range(1.05, 1.5);
        let heading = (self.net.ped.nodes[b as usize].pos - pos).normalize_or(Vec2::Y);
        let jaywalker = self.rng.chance(self.cfg.jaywalk * 10.0);
        let keep = self.rng.range(0.2, 0.8);
        self.peds.push(Ped { id, pos, vel: heading * cruise, heading, cruise, state: PedState::Walk, path, from: a, keep, partner: None, target: None, facing: None, jaywalker, wait_t: 0.0, anchor: pos });
        Some(id)
    }

    /// Gunshot, explosion...: everyone within `radius` runs away from `at`.
    pub fn alarm(&mut self, at: Vec2, radius: f32) {
        for p in &mut self.peds {
            if p.pos.distance(at) < radius {
                let left = 5.0 + (p.id % 5) as f32;
                p.state = PedState::Flee { from: at, left };
                p.partner = None;
                p.target = None;
            }
        }
    }

    fn nearest_walk_node(&self, p: Vec2) -> u32 {
        let mut best = (f32::MAX, 0u32);
        for (i, n) in self.net.ped.nodes.iter().enumerate() {
            let d = n.pos.distance_squared(p);
            if d < best.0 {
                best = (d, i as u32);
            }
        }
        best.1
    }

    pub(crate) fn step_peds(&mut self, dt: f32) {
        let n = self.peds.len();
        let plans: Vec<Plan> = (0..n).map(|i| self.plan_ped(i, dt)).collect();
        // Social forces from the snapshot.
        let mut new_vel = vec![Vec2::ZERO; n];
        for i in 0..n {
            let p = &self.peds[i];
            let plan = &plans[i];
            let mut f = (plan.vdes - p.vel) / 0.45;
            self.pgrid.query(p.pos, 2.5, |j| {
                if j as usize == i {
                    return;
                }
                let o = &self.peds[j as usize];
                let rel = p.pos - o.pos;
                let d = rel.length();
                if d < 2.5 && d > 1e-4 {
                    // Anisotropic: people ahead matter more, and someone
                    // coming the other way is passed on the right.
                    let front = (-rel / d).dot(p.heading).max(0.0);
                    let mag = 2.2 * ((0.62 - d) / 0.32).exp() * (0.5 + 0.5 * front);
                    f += rel / d * mag;
                    if p.vel.dot(o.vel) < 0.0 && front > 0.3 {
                        f += right(p.heading) * (mag * 0.8 + 0.6 * front);
                    }
                }
            });
            for e in &self.externals {
                let d = e.pos.distance(p.pos);
                if d < 3.0 && d > 1e-4 {
                    let r = if matches!(e.kind, ExternalKind::Ped { .. } | ExternalKind::Player) { 0.62 } else { e.half.y + 0.3 };
                    f += (p.pos - e.pos) / d * (2.5 * ((r - d) / 0.35).exp());
                }
            }
            self.vgrid.query(p.pos, 4.0, |vi| {
                let v = &self.vehicles[vi as usize];
                let d = obb_distance(v.pos, v.fwd, v.half(), p.pos);
                if d < 2.0 {
                    let away = (p.pos - v.pos).normalize_or(Vec2::X);
                    f += away * (6.0 * ((0.4 - d) / 0.35).exp());
                }
            });
            let np = self.parked.len() as u32;
            self.xgrid.query(p.pos, 4.0, |xi| {
                if xi < np {
                    let c = &self.parked[xi as usize];
                    let d = obb_distance(c.pos, c.fwd, Vec2::new(c.width * 0.5, c.len * 0.5), p.pos);
                    if d < 1.2 {
                        f += (p.pos - c.pos).normalize_or(Vec2::X) * (4.0 * ((0.3 - d) / 0.3).exp());
                    }
                }
            });
            let mut v = p.vel + f * dt;
            let sp = v.length();
            if sp > plan.max_speed {
                v *= plan.max_speed / sp;
            }
            if plan.max_speed < 0.05 && plan.vdes == Vec2::ZERO && sp < 0.3 {
                v *= 0.5;
            }
            new_vel[i] = v;
        }
        for i in 0..n {
            let plan = plans[i];
            let p = &mut self.peds[i];
            p.vel = new_vel[i];
            p.pos += p.vel * dt;
            if let Some((a, b, half)) = plan.corridor {
                // Stay on the sidewalk or zebra.
                let ab = b - a;
                let l2 = ab.length_squared().max(1e-6);
                let t = ((p.pos - a).dot(ab) / l2).clamp(-0.1, 1.1);
                let q = a + ab * t;
                let off = p.pos - q;
                if off.length() > half {
                    p.pos = q + off.normalize_or_zero() * half;
                }
            }
            let sp = p.vel.length();
            if sp > 0.2 {
                p.heading = p.vel / sp;
            } else if let Some(f) = p.facing {
                let t = (p.heading + (f - p.heading) * (3.0 * dt).min(1.0)).normalize_or(f);
                p.heading = t;
            }
        }
    }

    /// One pedestrian's intent for this step (and its state changes).
    fn plan_ped(&mut self, i: usize, dt: f32) -> Plan {
        let walk_speed = self.peds[i].cruise;
        let state = self.peds[i].state;
        let pos = self.peds[i].pos;
        // A car speeding at us: jump aside.
        if !matches!(state, PedState::Flee { .. }) {
            for e in &self.externals {
                if matches!(e.kind, ExternalKind::PlayerCar | ExternalKind::Car) {
                    let rel = pos - e.pos;
                    let closing = e.vel.dot(rel.normalize_or_zero());
                    if rel.length() < 10.0 && closing > 6.0 {
                        let side = right(e.vel.normalize_or_zero()) * if right(e.vel).dot(rel) > 0.0 { 1.0 } else { -1.0 };
                        self.peds[i].state = PedState::Flee { from: pos - side * 5.0, left: 1.5 };
                    }
                }
            }
        }
        let state = self.peds[i].state;
        match state {
            PedState::Flee { from, left } => {
                let left = left - dt;
                let away = (pos - from).normalize_or(self.peds[i].heading);
                if left <= 0.0 {
                    let n = self.nearest_walk_node(pos);
                    let path = self.new_walk(n);
                    let p = &mut self.peds[i];
                    p.state = PedState::Walk;
                    p.from = n;
                    p.path = std::iter::once(n).chain(path).collect();
                } else {
                    self.peds[i].state = PedState::Flee { from, left };
                }
                Plan { vdes: away * 3.6, max_speed: 4.0, corridor: None }
            }
            PedState::Walk => {
                let Some(&target) = self.peds[i].path.first() else {
                    self.start_activity(i);
                    return Plan { vdes: Vec2::ZERO, max_speed: walk_speed, corridor: None };
                };
                let from = self.peds[i].from;
                let a = self.net.ped.nodes[from as usize].pos;
                let b = self.net.ped.nodes[target as usize].pos;
                let half = self.edge_between(from, target).map_or(1.0, |e| e.half);
                let dir = (b - a).normalize_or(Vec2::X);
                let goal = b + right(dir) * (self.peds[i].keep * half);
                if pos.distance(goal) < 0.9 || pos.distance(b) < 1.0 {
                    // Arrived at a walk node.
                    let p = &mut self.peds[i];
                    p.path.remove(0);
                    p.from = target;
                    if let Some(&nx) = p.path.first()
                        && let Some(e) = self.edge_between(target, nx)
                        && let WalkKind::Crosswalk { crosswalk } = e.kind
                    {
                        self.peds[i].state = PedState::Wait { cw: crosswalk };
                        self.peds[i].wait_t = 0.0;
                    }
                    return Plan { vdes: dir * walk_speed, max_speed: walk_speed * 1.3, corridor: Some((a, b, half + 0.3)) };
                }
                let to = goal - pos;
                Plan { vdes: to.normalize_or_zero() * walk_speed, max_speed: walk_speed * 1.3, corridor: Some((a, b, half + 0.3)) }
            }
            PedState::Wait { cw } => {
                self.peds[i].wait_t += dt;
                let node = self.peds[i].from;
                let at = self.net.ped.nodes[node as usize].pos;
                let c = &self.net.crosswalks[cw as usize];
                let across = self.peds[i].path.first().copied();
                // Face the far side while waiting.
                if let Some(far) = across {
                    self.peds[i].facing = Some((self.net.ped.nodes[far as usize].pos - at).normalize_or_zero());
                }
                let signal = self.net.crosswalk_signal(cw, self.t);
                let p = &self.peds[i];
                let jay = p.jaywalker && p.wait_t > 6.0 && signal.is_some();
                if self.crossing_allowed(cw, jay) {
                    if signal.is_some_and(|s| s != PedSignal::Walk) {
                        self.stats.jaywalks += 1;
                    }
                    let to = across.unwrap_or(node);
                    let p = &mut self.peds[i];
                    p.state = PedState::Cross { cw, to };
                    p.facing = None;
                }
                let _ = c;
                let to = at - pos;
                let sp = if to.length() > 0.4 { walk_speed * 0.6 } else { 0.0 };
                Plan { vdes: to.normalize_or_zero() * sp, max_speed: walk_speed, corridor: Some((at, at, 1.2)) }
            }
            PedState::Cross { cw, to } => {
                let b = self.net.ped.nodes[to as usize].pos;
                let a = self.net.ped.nodes[self.peds[i].from as usize].pos;
                let flash = self.net.crosswalk_signal(cw, self.t).is_some_and(|s| s != PedSignal::Walk);
                let speed = walk_speed * if flash { 1.35 } else { 1.1 };
                if pos.distance(b) < 1.4 {
                    let p = &mut self.peds[i];
                    if p.path.first() == Some(&to) {
                        p.path.remove(0);
                    }
                    p.from = to;
                    p.state = PedState::Walk;
                    self.stats.crossings += 1;
                }
                let half = self.net.crosswalks[cw as usize].width * 0.5 - 0.3;
                let dir = (b - a).normalize_or_zero();
                let goal = b + right(dir) * (self.peds[i].keep * half * 0.8);
                Plan { vdes: (goal - pos).normalize_or_zero() * speed, max_speed: speed * 1.2, corridor: Some((a, b, half)) }
            }
            PedState::Activity { kind, left } => {
                let mut left = left - dt;
                if let Some(t) = self.peds[i].target {
                    let to = t - pos;
                    if to.length() < 0.4 {
                        self.peds[i].target = None;
                        self.peds[i].anchor = t;
                    } else {
                        self.peds[i].state = PedState::Activity { kind, left: left.max(1.0) };
                        return Plan { vdes: to.normalize_or_zero() * walk_speed * 0.8, max_speed: walk_speed, corridor: None };
                    }
                }
                // Partners face each other; a partner leaving ends the chat.
                if let Some(pid) = self.peds[i].partner {
                    match self.peds.iter().find(|o| o.id == pid) {
                        Some(o) if matches!(o.state, PedState::Activity { kind: Activity::Talk, .. }) => {
                            let f = (o.pos - pos).normalize_or_zero();
                            self.peds[i].facing = Some(f);
                        }
                        _ => {
                            self.peds[i].partner = None;
                            left = left.min(1.0);
                        }
                    }
                }
                if left <= 0.0 {
                    let n = self.peds[i].from;
                    let path = self.new_walk(n);
                    let p = &mut self.peds[i];
                    p.state = PedState::Walk;
                    p.partner = None;
                    p.facing = None;
                    p.path = path;
                } else {
                    self.peds[i].state = PedState::Activity { kind, left };
                }
                let anchor = self.peds[i].anchor;
                Plan { vdes: Vec2::ZERO, max_speed: 0.0, corridor: Some((anchor, anchor, 0.8)) }
            }
        }
    }

    /// At the end of a walk: wait, look in a window, chat or sit, or just
    /// carry on somewhere else.
    fn start_activity(&mut self, i: usize) {
        let node = self.peds[i].from;
        let pos = self.peds[i].pos;
        let r = self.rng.f();
        let wn = self.net.ped.nodes[node as usize].clone();
        self.peds[i].anchor = pos;
        if r < 0.22 && wn.outward != Vec2::ZERO {
            let left = self.rng.range(4.0, 15.0);
            let p = &mut self.peds[i];
            p.state = PedState::Activity { kind: Activity::Window, left };
            p.target = Some(wn.pos + wn.outward * 1.0);
            p.facing = Some(wn.outward);
            return;
        }
        if r < 0.42 {
            // Someone standing nearby to chat with.
            let other = (0..self.peds.len()).find(|&j| {
                j != i && self.peds[j].partner.is_none() && self.peds[j].pos.distance(pos) < 6.0 && matches!(self.peds[j].state, PedState::Activity { kind: Activity::Wait | Activity::Idle | Activity::Window, .. })
            });
            if let Some(j) = other {
                let left = self.rng.range(15.0, 45.0);
                let (a, b) = (self.peds[i].id, self.peds[j].id);
                let pj = self.peds[j].pos;
                let p = &mut self.peds[i];
                p.state = PedState::Activity { kind: Activity::Talk, left };
                p.partner = Some(b);
                p.target = Some(pj + (pos - pj).normalize_or(Vec2::X) * 0.9);
                let q = &mut self.peds[j];
                q.state = PedState::Activity { kind: Activity::Talk, left: left + 3.0 };
                q.partner = Some(a);
                q.target = None;
                return;
            }
            let left = self.rng.range(3.0, 12.0);
            self.peds[i].state = PedState::Activity { kind: Activity::Wait, left };
            return;
        }
        if r < 0.58
            && let Some(&(b, yaw)) = self.net.benches.iter().filter(|(b, _)| b.distance(pos) < 30.0).min_by(|x, y| x.0.distance(pos).total_cmp(&y.0.distance(pos)))
            && !self.peds.iter().any(|o| o.is_sitting() && o.pos.distance(b) < 0.8)
        {
            let left = self.rng.range(20.0, 60.0);
            let p = &mut self.peds[i];
            p.state = PedState::Activity { kind: Activity::Sit, left };
            p.target = Some(b);
            p.facing = Some(Vec2::new(yaw.sin(), yaw.cos()));
            return;
        }
        let path = self.new_walk(node);
        self.peds[i].path = path;
        if self.peds[i].path.is_empty() {
            self.peds[i].state = PedState::Activity { kind: Activity::Idle, left: 5.0 };
        }
    }
}
