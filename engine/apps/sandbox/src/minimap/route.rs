//! GPS: the road graph of a [`MapData`] and the fastest drive between two
//! points (A* on travel time, one-way streets respected).
//!
//! Junctions are the vertices two roads share (as in OpenStreetMap, where a
//! bridge over a road shares none with it). Maps whose streets are loose
//! pieces (`MapData::from_world`) are noded first: crossings become
//! vertices and loose ends are tied to the street they reach.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use forja_math::Vec2;

use super::data::{MapData, ONEWAY, road};

#[derive(Clone, Copy, Debug)]
struct Edge {
    a: u32,
    b: u32,
    /// Points `start..start + len` of [`RoadGraph::pts`], from `a` to `b`.
    start: u32,
    len: u32,
    length: f32,
    speed: f32,
    oneway: bool,
    /// Part of the largest connected network: the GPS only starts and
    /// ends there (a car park's loose lane leads nowhere).
    main: bool,
}

#[derive(Clone, Copy, Debug)]
struct Arc {
    to: u32,
    edge: u32,
    /// Against a one-way street (only taken when the signs are ignored).
    against: bool,
}

/// A point on a road.
#[derive(Clone, Copy, Debug)]
pub struct Snap {
    edge: u32,
    /// Distance from the edge's `a` end along it.
    along: f32,
    pub pos: Vec2,
    /// Distance from the point asked for.
    pub dist: f32,
}

#[derive(Default)]
pub struct RoadGraph {
    nodes: Vec<Vec2>,
    edges: Vec<Edge>,
    pts: Vec<Vec2>,
    adj_start: Vec<u32>,
    adj: Vec<Arc>,
    /// Edge segments by cell: `(edge, segment)`.
    grid: HashMap<(i32, i32), Vec<(u32, u32)>>,
}

const CELL: f32 = 96.0;
/// The fastest road class: the A* bound on travel time.
const TOP_SPEED: f32 = 30.0;

fn cell_of(p: Vec2) -> (i32, i32) {
    ((p.x / CELL).floor() as i32, (p.y / CELL).floor() as i32)
}

fn key(p: Vec2) -> (i32, i32) {
    ((p.x * 8.0).round() as i32, (p.y * 8.0).round() as i32)
}

/// Closest point of segment `a-b` to `p`: `(t, point)`.
fn closest(a: Vec2, b: Vec2, p: Vec2) -> (f32, Vec2) {
    let d = b - a;
    let l2 = d.length_squared();
    let t = if l2 > 0.0 { ((p - a).dot(d) / l2).clamp(0.0, 1.0) } else { 0.0 };
    (t, a + d * t)
}

/// Where segments `a-b` and `c-d` cross (not at a shared end).
fn crossing(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> Option<Vec2> {
    let (r, s) = (b - a, d - c);
    let den = r.perp_dot(s);
    if den.abs() < 1e-6 {
        return None;
    }
    let (t, u) = ((c - a).perp_dot(s) / den, (c - a).perp_dot(r) / den);
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then(|| a + r * t)
}

/// One drivable polyline before the graph is built.
struct Line {
    pts: Vec<Vec2>,
    speed: f32,
    oneway: bool,
    width: f32,
}

/// Makes crossings and reached ends vertices of both lines.
fn node_lines(lines: &mut Vec<Line>) {
    let mut grid: HashMap<(i32, i32), Vec<(u32, u32)>> = HashMap::new();
    let cells = |a: Vec2, b: Vec2, pad: f32| {
        let (lo, hi) = (cell_of(a.min(b) - Vec2::splat(pad)), cell_of(a.max(b) + Vec2::splat(pad)));
        (lo.0..=hi.0).flat_map(move |i| (lo.1..=hi.1).map(move |j| (i, j)))
    };
    for (li, l) in lines.iter().enumerate() {
        for (si, w) in l.pts.windows(2).enumerate() {
            for c in cells(w[0], w[1], 0.0) {
                grid.entry(c).or_default().push((li as u32, si as u32));
            }
        }
    }
    // `(line, segment, t, point)` to insert.
    let mut cuts: Vec<(u32, u32, f32, Vec2)> = Vec::new();
    let mut links: Vec<(Vec2, Vec2, f32)> = Vec::new();
    for (li, l) in lines.iter().enumerate() {
        for (si, w) in l.pts.windows(2).enumerate() {
            for c in cells(w[0], w[1], 0.0) {
                for &(lj, sj) in grid.get(&c).into_iter().flatten() {
                    if (lj as usize) <= li {
                        continue;
                    }
                    let o = &lines[lj as usize].pts;
                    if let Some(p) = crossing(w[0], w[1], o[sj as usize], o[sj as usize + 1]) {
                        cuts.push((li as u32, si as u32, (p - w[0]).length(), p));
                        cuts.push((lj, sj, (p - o[sj as usize]).length(), p));
                    }
                }
            }
        }
        // Loose ends: tie each to the nearest other street within reach.
        for end in [l.pts[0], *l.pts.last().expect("two points")] {
            let mut best: Option<(f32, u32, u32, Vec2)> = None;
            for c in cells(end, end, 40.0) {
                for &(lj, sj) in grid.get(&c).into_iter().flatten() {
                    if lj as usize == li {
                        continue;
                    }
                    let o = &lines[lj as usize];
                    let (_, p) = closest(o.pts[sj as usize], o.pts[sj as usize + 1], end);
                    let d = p.distance(end);
                    if d <= (l.width + o.width) * 0.5 + 14.0 && best.is_none_or(|b| d < b.0) {
                        best = Some((d, lj, sj, p));
                    }
                }
            }
            if let Some((d, lj, sj, p)) = best
                && d > 0.05
            {
                cuts.push((lj, sj, (p - lines[lj as usize].pts[sj as usize]).length(), p));
                links.push((end, p, l.speed.min(lines[lj as usize].speed)));
            }
        }
    }
    cuts.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)).then(a.2.total_cmp(&b.2)));
    // Insert from the back so the segment indices stay valid.
    for &(li, si, _, p) in cuts.iter().rev() {
        let pts = &mut lines[li as usize].pts;
        if key(pts[si as usize]) != key(p) && key(pts[si as usize + 1]) != key(p) {
            pts.insert(si as usize + 1, p);
        }
    }
    for (a, b, speed) in links {
        lines.push(Line { pts: vec![a, b], speed, oneway: false, width: 0.0 });
    }
}

impl RoadGraph {
    /// The drivable roads of `map`. `noded`: junctions already are shared
    /// vertices (the island file); else crossings are found here.
    pub fn build(map: &MapData, noded: bool) -> RoadGraph {
        let mut lines: Vec<Line> = map
            .roads
            .iter()
            .filter(|r| road::speed(r.class) > 0.0)
            .map(|r| Line { pts: map.road_pts(r).to_vec(), speed: road::speed(r.class), oneway: r.flags & ONEWAY != 0, width: r.width() })
            .collect();
        if !noded {
            node_lines(&mut lines);
        }
        // Graph nodes: line ends and vertices used more than once.
        let mut uses: HashMap<(i32, i32), u32> = HashMap::new();
        for l in &lines {
            for (i, p) in l.pts.iter().enumerate() {
                *uses.entry(key(*p)).or_default() += if i == 0 || i + 1 == l.pts.len() { 2 } else { 1 };
            }
        }
        let mut g = RoadGraph::default();
        let mut ids: HashMap<(i32, i32), u32> = HashMap::new();
        for l in &lines {
            let mut from = 0;
            for i in 1..l.pts.len() {
                if uses[&key(l.pts[i])] < 2 {
                    continue;
                }
                let piece = &l.pts[from..=i];
                let length: f32 = piece.windows(2).map(|w| w[0].distance(w[1])).sum();
                let mut node = |p: Vec2, g: &mut RoadGraph| {
                    *ids.entry(key(p)).or_insert_with(|| {
                        g.nodes.push(p);
                        g.nodes.len() as u32 - 1
                    })
                };
                let (a, b) = (node(piece[0], &mut g), node(piece[piece.len() - 1], &mut g));
                if length > 0.0 {
                    g.edges.push(Edge { a, b, start: g.pts.len() as u32, len: piece.len() as u32, length, speed: l.speed, oneway: l.oneway, main: true });
                    g.pts.extend_from_slice(piece);
                }
                from = i;
            }
        }
        // The largest network, by road length.
        let mut parent: Vec<u32> = (0..g.nodes.len() as u32).collect();
        fn find(parent: &mut [u32], mut a: u32) -> u32 {
            while parent[a as usize] != a {
                parent[a as usize] = parent[parent[a as usize] as usize];
                a = parent[a as usize];
            }
            a
        }
        for e in &g.edges {
            let (a, b) = (find(&mut parent, e.a), find(&mut parent, e.b));
            parent[a as usize] = b;
        }
        let mut total = vec![0.0f32; g.nodes.len()];
        for e in &g.edges {
            total[find(&mut parent, e.a) as usize] += e.length;
        }
        let main = (0..total.len()).max_by(|a, b| total[*a].total_cmp(&total[*b])).unwrap_or(0) as u32;
        for i in 0..g.edges.len() {
            g.edges[i].main = find(&mut parent, g.edges[i].a) == main;
        }
        // Adjacency (CSR) and the segment grid.
        let mut count = vec![0u32; g.nodes.len() + 1];
        for e in &g.edges {
            count[e.a as usize + 1] += 1;
            count[e.b as usize + 1] += 1;
        }
        for i in 1..count.len() {
            count[i] += count[i - 1];
        }
        g.adj = vec![Arc { to: 0, edge: 0, against: false }; *count.last().unwrap_or(&0) as usize];
        let mut fill = count.clone();
        for (i, e) in g.edges.iter().enumerate() {
            g.adj[fill[e.a as usize] as usize] = Arc { to: e.b, edge: i as u32, against: false };
            fill[e.a as usize] += 1;
            g.adj[fill[e.b as usize] as usize] = Arc { to: e.a, edge: i as u32, against: e.oneway };
            fill[e.b as usize] += 1;
        }
        g.adj_start = count;
        for (i, e) in g.edges.iter().enumerate() {
            let pts = &g.pts[e.start as usize..(e.start + e.len) as usize];
            for (s, w) in pts.windows(2).enumerate() {
                let (lo, hi) = (cell_of(w[0].min(w[1])), cell_of(w[0].max(w[1])));
                for cx in lo.0..=hi.0 {
                    for cy in lo.1..=hi.1 {
                        g.grid.entry((cx, cy)).or_default().push((i as u32, s as u32));
                    }
                }
            }
        }
        g
    }

    pub fn is_empty(&self) -> bool {
        self.edges.is_empty()
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    fn edge_pts(&self, e: &Edge) -> &[Vec2] {
        &self.pts[e.start as usize..(e.start + e.len) as usize]
    }

    /// The closest road point within `reach` metres.
    pub fn snap(&self, p: Vec2, reach: f32) -> Option<Snap> {
        let c = cell_of(p);
        let mut best: Option<Snap> = None;
        let rings = (reach / CELL).ceil() as i32;
        for ring in 0..=rings {
            // Nothing in a farther ring beats a hit closer than the ring.
            if best.is_some_and(|b| b.dist <= (ring - 1).max(0) as f32 * CELL) {
                break;
            }
            for i in -ring..=ring {
                for j in -ring..=ring {
                    if i.abs().max(j.abs()) != ring {
                        continue;
                    }
                    for &(ei, si) in self.grid.get(&(c.0 + i, c.1 + j)).into_iter().flatten() {
                        let e = &self.edges[ei as usize];
                        if !e.main {
                            continue;
                        }
                        let pts = self.edge_pts(e);
                        let (t, q) = closest(pts[si as usize], pts[si as usize + 1], p);
                        let dist = q.distance(p);
                        if dist <= reach && best.is_none_or(|b| dist < b.dist) {
                            let before: f32 = pts[..=si as usize].windows(2).map(|w| w[0].distance(w[1])).sum();
                            best = Some(Snap { edge: ei, along: before + t * pts[si as usize].distance(pts[si as usize + 1]), pos: q, dist });
                        }
                    }
                }
            }
        }
        best
    }

    /// Points of edge `e` between the distances `from` and `to` along it
    /// (reversed when `from > to`).
    fn slice(&self, e: &Edge, from: f32, to: f32, out: &mut Vec<Vec2>) {
        let pts = self.edge_pts(e);
        let (lo, hi) = (from.min(to), from.max(to));
        let mut piece = Vec::new();
        let mut d = 0.0;
        for w in pts.windows(2) {
            let l = w[0].distance(w[1]);
            let (s0, s1) = (d, d + l);
            if s1 >= lo && s0 <= hi && l > 0.0 {
                let a = w[0] + (w[1] - w[0]) * ((lo - s0) / l).clamp(0.0, 1.0);
                let b = w[0] + (w[1] - w[0]) * ((hi - s0) / l).clamp(0.0, 1.0);
                if piece.last().is_none_or(|q: &Vec2| q.distance_squared(a) > 1e-6) {
                    piece.push(a);
                }
                if b.distance_squared(a) > 1e-6 {
                    piece.push(b);
                }
            }
            d = s1;
        }
        if from > to {
            piece.reverse();
        }
        for p in piece {
            if out.last().is_none_or(|q| q.distance_squared(p) > 1e-6) {
                out.push(p);
            }
        }
    }

    /// The fastest drive from `from` to `to` (both snapped to the nearest
    /// road within `reach`). `None` when there is no road near one of them
    /// or no way between them.
    pub fn route(&self, from: Vec2, to: Vec2, reach: f32) -> Option<Route> {
        let (s, g) = (self.snap(from, reach)?, self.snap(to, reach)?);
        self.route_between(s, g, true).or_else(|| self.route_between(s, g, false))
    }

    fn route_between(&self, s: Snap, g: Snap, oneways: bool) -> Option<Route> {
        #[derive(PartialEq)]
        struct Open(f32, u32);
        impl Eq for Open {}
        impl PartialOrd for Open {
            fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
                Some(self.cmp(o))
            }
        }
        impl Ord for Open {
            fn cmp(&self, o: &Self) -> Ordering {
                o.0.total_cmp(&self.0)
            }
        }
        let (se, ge) = (self.edges[s.edge as usize], self.edges[g.edge as usize]);
        let (s_one, g_one) = (se.oneway && oneways, ge.oneway && oneways);
        const NONE: u32 = u32::MAX;
        let mut cost = vec![f32::INFINITY; self.nodes.len()];
        // Edge taken into each node and the node it came from (`NONE`:
        // straight from the start point).
        let mut via = vec![(NONE, NONE); self.nodes.len()];
        let mut open = BinaryHeap::new();
        let h = |n: u32| self.nodes[n as usize].distance(g.pos) / TOP_SPEED;
        // Best way found: time, last node (`NONE`: both points on one
        // edge) and whether the goal's edge is entered at its `a` end.
        let mut best: Option<(f32, u32, bool)> = None;
        if s.edge == g.edge && (g.along >= s.along || !s_one) {
            best = Some(((g.along - s.along).abs() / se.speed, NONE, true));
        }
        // Leaving the start's edge by its `b` end, and by `a` if allowed.
        let (to_b, to_a) = ((se.length - s.along) / se.speed, s.along / se.speed);
        cost[se.b as usize] = to_b;
        open.push(Open(to_b + h(se.b), se.b));
        if !s_one && to_a < cost[se.a as usize] {
            cost[se.a as usize] = to_a;
            open.push(Open(to_a + h(se.a), se.a));
        }
        let start_forward = se.a != se.b || s_one || to_b <= to_a;
        while let Some(Open(f, n)) = open.pop() {
            if best.is_some_and(|b| f >= b.0) {
                break;
            }
            let c = cost[n as usize];
            if f > c + h(n) + 1e-3 {
                continue;
            }
            // Arriving at the goal's edge from either end.
            if n == ge.a && best.is_none_or(|b| c + g.along / ge.speed < b.0) {
                best = Some((c + g.along / ge.speed, n, true));
            }
            if n == ge.b && !g_one && best.is_none_or(|b| c + (ge.length - g.along) / ge.speed < b.0) {
                best = Some((c + (ge.length - g.along) / ge.speed, n, false));
            }
            for a in &self.adj[self.adj_start[n as usize] as usize..self.adj_start[n as usize + 1] as usize] {
                if a.against && oneways {
                    continue;
                }
                let e = &self.edges[a.edge as usize];
                let nc = c + e.length / e.speed;
                if nc < cost[a.to as usize] {
                    cost[a.to as usize] = nc;
                    via[a.to as usize] = (a.edge, n);
                    open.push(Open(nc + h(a.to), a.to));
                }
            }
        }
        let (time, last, from_a) = best?;
        let mut pts = vec![s.pos];
        if last == NONE {
            self.slice(&se, s.along, g.along, &mut pts);
        } else {
            // Back from the goal's node to the end of the start's edge.
            let mut chain = Vec::new();
            let mut n = last;
            while via[n as usize].0 != NONE {
                chain.push((via[n as usize], n));
                n = via[n as usize].1;
            }
            let forward = if se.a == se.b { start_forward } else { n == se.b };
            self.slice(&se, s.along, if forward { se.length } else { 0.0 }, &mut pts);
            for &((ei, from), to) in chain.iter().rev() {
                let e = &self.edges[ei as usize];
                if e.a == from && e.b == to {
                    self.slice(e, 0.0, e.length, &mut pts);
                } else {
                    self.slice(e, e.length, 0.0, &mut pts);
                }
            }
            self.slice(&ge, if from_a { 0.0 } else { ge.length }, g.along, &mut pts);
        }
        if pts.last().is_none_or(|q| q.distance_squared(g.pos) > 1e-6) {
            pts.push(g.pos);
        }
        Some(Route::new(pts, time))
    }
}

/// A drive: its line on the ground and how far along the player is.
#[derive(Clone, Debug)]
pub struct Route {
    pub pts: Vec<Vec2>,
    /// Distance from the start at each point.
    pub cum: Vec<f32>,
    /// Estimated drive time in seconds.
    pub time: f32,
    /// Segment the player was last closest to.
    cursor: usize,
}

impl Route {
    pub fn new(pts: Vec<Vec2>, time: f32) -> Route {
        let mut cum = Vec::with_capacity(pts.len());
        let mut d = 0.0;
        for (i, p) in pts.iter().enumerate() {
            if i > 0 {
                d += pts[i - 1].distance(*p);
            }
            cum.push(d);
        }
        Route { pts, cum, time, cursor: 0 }
    }

    pub fn length(&self) -> f32 {
        self.cum.last().copied().unwrap_or(0.0)
    }

    /// Moves the cursor to the route point closest to `p` (searching from
    /// where it was, so a route that passes twice near a place keeps its
    /// order) and returns `(distance along, distance from the route)`.
    pub fn track(&mut self, p: Vec2) -> (f32, f32) {
        if self.pts.len() < 2 {
            return (0.0, self.pts.first().map_or(0.0, |q| q.distance(p)));
        }
        let mut best = (f32::INFINITY, self.cursor, 0.0);
        let hi = (self.cursor + 40).min(self.pts.len() - 1);
        for i in self.cursor.saturating_sub(2)..hi {
            let (t, q) = closest(self.pts[i], self.pts[i + 1], p);
            let d = q.distance(p);
            if d < best.0 {
                best = (d, i, self.cum[i] + t * (self.cum[i + 1] - self.cum[i]));
            }
            // Far enough along: the rest of the window cannot be closer.
            if self.cum[i] - self.cum[self.cursor] > 400.0 && best.0 < 60.0 {
                break;
            }
        }
        self.cursor = best.1;
        (best.2, best.0)
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Distance left from `along`.
    pub fn remaining(&self, along: f32) -> f32 {
        (self.length() - along).max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 3 x 3 grid of two-way streets 100 m apart, as loose pieces from
    /// `(0, 0)` to `(200, 200)`; `oneway`: extra `(class, flags, points)`.
    fn grid(extra: &[(u8, u8, Vec<Vec2>)]) -> MapData {
        let mut m = MapData::default();
        for i in 0..3 {
            let c = i as f32 * 100.0;
            m.add_road(road::RESIDENTIAL, 0, &[Vec2::new(0.0, c), Vec2::new(200.0, c)], 0.0);
            m.add_road(road::RESIDENTIAL, 0, &[Vec2::new(c, 0.0), Vec2::new(c, 200.0)], 0.0);
        }
        for (class, flags, pts) in extra {
            m.add_road(*class, *flags, pts, 0.0);
        }
        m
    }

    fn follows_roads(r: &Route, g: &RoadGraph) {
        for w in r.pts.windows(2) {
            let mid = (w[0] + w[1]) * 0.5;
            assert!(g.snap(mid, 5.0).is_some_and(|s| s.dist < 0.01), "route leaves the roads at {mid:?}");
        }
    }

    #[test]
    fn crossings_become_junctions() {
        let g = RoadGraph::build(&grid(&[]), false);
        // 9 crossings, 12 blocks' sides.
        assert_eq!((g.node_count(), g.edge_count()), (9, 12));
        // Already noded data is taken as is: the middle streets share no
        // vertex with the ones they cross (a bridge), only the outer ring
        // meets at its corners.
        let apart = RoadGraph::build(&grid(&[]), true);
        assert_eq!(apart.edge_count(), 6);
        assert!(apart.route(Vec2::new(10.0, 0.0), Vec2::new(100.0, 150.0), 20.0).is_none());
        assert!(apart.route(Vec2::new(10.0, 0.0), Vec2::new(200.0, 150.0), 20.0).is_some());
    }

    #[test]
    fn shortest_route_on_a_grid() {
        let g = RoadGraph::build(&grid(&[]), false);
        let r = g.route(Vec2::new(10.0, 3.0), Vec2::new(197.0, 150.0), 20.0).expect("route");
        // Manhattan distance between the snapped ends.
        assert!((r.length() - (190.0 + 150.0)).abs() < 0.5, "{}", r.length());
        assert_eq!(r.pts[0], Vec2::new(10.0, 0.0));
        assert_eq!(*r.pts.last().unwrap(), Vec2::new(200.0, 150.0));
        follows_roads(&r, &g);
        // Same street: straight along it, either way.
        let r = g.route(Vec2::new(180.0, 100.0), Vec2::new(30.0, 101.0), 20.0).expect("route");
        assert!((r.length() - 150.0).abs() < 0.5);
        follows_roads(&r, &g);
    }

    #[test]
    fn faster_roads_are_preferred() {
        // A motorway arc around the grid: longer, but quicker than the
        // residential streets.
        let ring = vec![Vec2::new(0.0, 0.0), Vec2::new(-150.0, 100.0), Vec2::new(0.0, 200.0)];
        let g = RoadGraph::build(&grid(&[(road::MOTORWAY, 0, ring)]), false);
        let r = g.route(Vec2::new(0.0, 0.0), Vec2::new(0.0, 200.0), 20.0).expect("route");
        assert!(r.length() > 300.0, "took the streets: {}", r.length());
        assert!(r.time < 200.0 / road::speed(road::RESIDENTIAL));
    }

    #[test]
    fn one_way_streets_are_respected() {
        // The only link between two streets is one-way, west to east.
        let mut m = MapData::default();
        m.add_road(road::RESIDENTIAL, 0, &[Vec2::new(0.0, 0.0), Vec2::new(0.0, 100.0)], 0.0);
        m.add_road(road::RESIDENTIAL, 0, &[Vec2::new(100.0, 0.0), Vec2::new(100.0, 100.0)], 0.0);
        m.add_road(road::RESIDENTIAL, ONEWAY, &[Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)], 0.0);
        // The way back is a long detour, two-way.
        m.add_road(road::RESIDENTIAL, 0, &[Vec2::new(100.0, 100.0), Vec2::new(50.0, 400.0), Vec2::new(0.0, 100.0)], 0.0);
        let g = RoadGraph::build(&m, true);
        let (a, b) = (Vec2::new(0.0, 50.0), Vec2::new(100.0, 50.0));
        let there = g.route(a, b, 10.0).expect("route");
        assert!((there.length() - 200.0).abs() < 0.5, "{}", there.length());
        let back = g.route(b, a, 10.0).expect("route");
        assert!(back.length() > 600.0, "drove the one-way backwards: {}", back.length());
        follows_roads(&there, &g);
        follows_roads(&back, &g);
        // Starting on the one-way street itself: only forwards.
        let r = g.route(Vec2::new(60.0, 0.0), Vec2::new(20.0, 0.0), 10.0).expect("route");
        assert!(r.length() > 600.0, "{}", r.length());
        let r = g.route(Vec2::new(20.0, 0.0), Vec2::new(60.0, 0.0), 10.0).expect("route");
        assert!((r.length() - 40.0).abs() < 0.5);
    }

    #[test]
    fn a_one_way_trap_still_gets_a_route() {
        // The goal can only be left, never entered: the GPS falls back to
        // ignoring the signs instead of giving nothing.
        let mut m = MapData::default();
        m.add_road(road::RESIDENTIAL, ONEWAY, &[Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)], 0.0);
        m.add_road(road::RESIDENTIAL, 0, &[Vec2::new(100.0, 0.0), Vec2::new(100.0, 100.0)], 0.0);
        let g = RoadGraph::build(&m, true);
        let r = g.route(Vec2::new(100.0, 80.0), Vec2::new(30.0, 0.0), 10.0).expect("route");
        assert!((r.length() - 150.0).abs() < 0.5, "{}", r.length());
    }

    #[test]
    fn no_road_nearby_is_no_route() {
        let g = RoadGraph::build(&grid(&[]), false);
        assert!(g.route(Vec2::new(5000.0, 5000.0), Vec2::new(0.0, 0.0), 300.0).is_none());
        assert!(RoadGraph::default().route(Vec2::ZERO, Vec2::ONE, 300.0).is_none());
    }

    #[test]
    fn loose_ends_are_tied_to_the_street_they_reach() {
        // A side street stopping at the building line of an avenue, 9 m
        // short of its centreline.
        let mut m = MapData::default();
        m.add_road(road::PRIMARY, 0, &[Vec2::new(-200.0, 0.0), Vec2::new(200.0, 0.0)], 16.0);
        m.add_road(road::RESIDENTIAL, 0, &[Vec2::new(0.0, 9.0), Vec2::new(0.0, 150.0)], 6.0);
        let g = RoadGraph::build(&m, false);
        let r = g.route(Vec2::new(0.0, 120.0), Vec2::new(150.0, 0.0), 10.0).expect("route");
        assert!((r.length() - 270.0).abs() < 0.5, "{}", r.length());
    }

    #[test]
    fn tracking_follows_the_player_along_the_route() {
        let mut r = Route::new(vec![Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0), Vec2::new(100.0, 100.0)], 20.0);
        assert_eq!(r.length(), 200.0);
        let (along, off) = r.track(Vec2::new(40.0, 3.0));
        assert!((along - 40.0).abs() < 1e-3 && (off - 3.0).abs() < 1e-3);
        let (along, off) = r.track(Vec2::new(104.0, 50.0));
        assert!((along - 150.0).abs() < 1e-3 && (off - 4.0).abs() < 1e-3);
        assert_eq!(r.cursor(), 1);
        assert!((r.remaining(along) - 50.0).abs() < 1e-3);
        // Off the route.
        assert!(r.track(Vec2::new(200.0, 50.0)).1 > 90.0);
    }
}
