//! Builds a [`RoadGraphSpec`] from a "soup" of street segments (what the
//! current grid layout produces: straight pieces between building lines,
//! long avenues crossing several junctions) plus the signs placed in the
//! world. Junction centres are found where a segment's end ray meets
//! another street, segments are split at junctions they pass through, and
//! every sign is bound to the street end or street it faces, so the rules
//! the AI follows come from the same placements the renderer draws.

use forja_math::Vec2;

use crate::geom::left;
use crate::spec::{Control, EndSpec, KMH, LaneSpec, NodeSpec, ParkingSpec, RoadGraphSpec, StreetSpec};

/// A straight street piece from one building line (or junction) to another.
#[derive(Clone, Debug)]
pub struct SegmentIn {
    pub a: Vec2,
    pub b: Vec2,
    /// Building line to centreline.
    pub half_width: f32,
    pub road_half: f32,
    pub sidewalk: f32,
    pub median_half: f32,
    pub lanes: Vec<LaneSpec>,
    pub parking: Vec<ParkingSpec>,
    /// Default limit before speed signs (m/s).
    pub speed_limit: f32,
    /// Zebra centre distance and stop line distance from each end.
    pub crosswalk: Option<f32>,
    pub stop_line: f32,
    /// Where the raised median is open (turns across it allowed).
    pub median_openings: Vec<Vec2>,
}

/// Road sign kinds the AI understands (Spanish codes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignRule {
    Stop,
    Yield,
    /// Speed limit in km/h.
    Speed(u32),
    Crossing,
    NoEntry,
    AheadOnly,
    NoParking,
    Parking,
}

/// A sign in the world: `facing` is the plate normal (towards the drivers
/// who must obey it).
#[derive(Clone, Copy, Debug)]
pub struct SignIn {
    pub rule: SignRule,
    pub pos: Vec2,
    pub facing: Vec2,
}

/// What the adapter did with each sign (for review and tests).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignUse {
    /// Bound to `(street, end)` of the output spec.
    End(usize, usize),
    /// Bound to a whole street.
    Street(usize),
    Unbound,
}

struct Hit {
    seg: usize,
    end: usize,
    centre: Vec2,
}

/// Junction centres, split streets, controls from `signs`; signal heads are
/// bound afterwards by [`crate::network::RoadNetwork::bind_heads`] and
/// approaches with a head are set to [`Control::Signal`] by
/// [`apply_signal_heads`].
pub fn build_graph(segs: &[SegmentIn], signs: &[SignIn]) -> (RoadGraphSpec, Vec<SignUse>) {
    // 1. Junction centre of every segment end.
    let mut hits: Vec<Hit> = Vec::new();
    for (i, s) in segs.iter().enumerate() {
        let len = s.a.distance(s.b);
        if len < 0.5 {
            continue;
        }
        let dir = (s.b - s.a) / len;
        for end in 0..2 {
            let (e, d) = if end == 0 { (s.a, -dir) } else { (s.b, dir) };
            let mut best: Option<f32> = None;
            for (j, o) in segs.iter().enumerate() {
                if j == i {
                    continue;
                }
                let ol = o.a.distance(o.b);
                if ol < 0.5 {
                    continue;
                }
                let od = (o.b - o.a) / ol;
                let den = d.perp_dot(od);
                if den.abs() < 0.5 {
                    // Collinear continuation facing us: the junction lies halfway.
                    if d.dot(od).abs() > 0.99 {
                        for (q, qd) in [(o.a, -od), (o.b, od)] {
                            let rel = q - e;
                            let t = rel.dot(d);
                            if qd.dot(d) < -0.99 && t > 0.0 && rel.perp_dot(d).abs() < 1.0 && t < 2.0 * (s.half_width.max(o.half_width) + 2.0) {
                                best = Some(best.map_or(t * 0.5, |b: f32| b.min(t * 0.5)));
                            }
                        }
                    }
                    continue;
                }
                // e + d t = o.a + od u
                let q = o.a - e;
                let t = q.perp_dot(od) / den;
                let u = q.perp_dot(d) / den;
                if t > -0.5 && t < o.half_width * 1.25 + 1.5 && u > -o.half_width - 1.0 && u < ol + o.half_width + 1.0 {
                    best = Some(best.map_or(t.max(0.0), |b: f32| b.min(t.max(0.0))));
                }
            }
            hits.push(Hit { seg: i, end, centre: e + d * best.unwrap_or(0.0) });
        }
    }
    // 2. Cluster centres into nodes.
    let mut nodes: Vec<(Vec2, usize)> = Vec::new();
    let mut end_node = vec![[usize::MAX; 2]; segs.len()];
    for h in &hits {
        let n = match nodes.iter().position(|(p, _)| p.distance(h.centre) < 3.0) {
            Some(n) => {
                let (p, c) = nodes[n];
                nodes[n] = ((p * c as f32 + h.centre) / (c + 1) as f32, c + 1);
                n
            }
            None => {
                nodes.push((h.centre, 1));
                nodes.len() - 1
            }
        };
        end_node[h.seg][h.end] = n;
    }
    // 3. Interior stations: nodes a segment passes through, with the half
    // length of their box along it (the widest street ending there).
    let interior = |i: usize, nodes: &[(Vec2, usize)], end_node: &[[usize; 2]]| -> Vec<(f32, usize, f32)> {
        let s = &segs[i];
        let len = s.a.distance(s.b);
        let dir = (s.b - s.a) / len;
        let mut out = Vec::new();
        for (n, (p, _)) in nodes.iter().enumerate() {
            if end_node[i].contains(&n) {
                continue;
            }
            let rel = *p - s.a;
            let u = rel.dot(dir);
            if u > 1.0 && u < len - 1.0 && rel.perp_dot(dir).abs() < 1.5 {
                let other = segs.iter().enumerate().filter(|(j, _)| *j != i && end_node[*j].contains(&n)).map(|(_, o)| o.half_width).fold(0.0f32, f32::max);
                out.push((u, n, other.max(2.0)));
            }
        }
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
        out
    };
    // Staggered junctions whose boxes overlap along a through street are
    // one junction.
    let mut root: Vec<usize> = (0..nodes.len()).collect();
    fn find(root: &mut [usize], mut x: usize) -> usize {
        while root[x] != x {
            root[x] = root[root[x]];
            x = root[x];
        }
        x
    }
    for i in 0..segs.len() {
        if end_node[i].contains(&usize::MAX) {
            continue;
        }
        let st = interior(i, &nodes, &end_node);
        for w in st.windows(2) {
            if w[1].0 - w[1].2 < w[0].0 + w[0].2 + 1.0 {
                let (a, b) = (find(&mut root, w[0].1), find(&mut root, w[1].1));
                root[b] = a;
            }
        }
    }
    let mut merged: Vec<(Vec2, usize)> = Vec::new();
    let mut remap = vec![usize::MAX; nodes.len()];
    for n in 0..nodes.len() {
        let r = find(&mut root, n);
        if remap[r] == usize::MAX {
            remap[r] = merged.len();
            merged.push((Vec2::ZERO, 0));
        }
        remap[n] = remap[r];
        let m = &mut merged[remap[n]];
        m.0 += nodes[n].0;
        m.1 += 1;
    }
    let orig_nodes = nodes.clone();
    let orig_end = end_node.clone();
    let nodes: Vec<(Vec2, usize)> = merged.into_iter().map(|(p, c)| (p / c as f32, c)).collect();
    for e in &mut end_node {
        for n in e.iter_mut() {
            if *n != usize::MAX {
                *n = remap[*n];
            }
        }
    }
    // 4. Split segments at the junctions they pass through.
    let mut spec = RoadGraphSpec { nodes: nodes.iter().map(|(p, _)| NodeSpec { pos: *p, signal_offset: 0.0 }).collect(), ..Default::default() };
    for (i, s) in segs.iter().enumerate() {
        if end_node[i].contains(&usize::MAX) {
            continue;
        }
        let len = s.a.distance(s.b);
        let dir = (s.b - s.a) / len;
        // (box start, box end along the segment, node, original end?)
        let mut stations: Vec<(f32, f32, usize, bool)> = vec![(0.0, 0.0, end_node[i][0], true), (len, len, end_node[i][1], true)];
        // Boxes from the junctions as found, merged per joined junction.
        for (u, n, t) in interior(i, &orig_nodes, &orig_end) {
            let n = remap[n];
            if end_node[i].contains(&n) {
                continue;
            }
            match stations.iter_mut().find(|x| x.2 == n && !x.3) {
                Some(x) => {
                    x.0 = x.0.min(u - t);
                    x.1 = x.1.max(u + t);
                }
                None => stations.push((u - t, u + t, n, false)),
            }
        }
        stations.sort_by(|a, b| (a.0 + a.1).total_cmp(&(b.0 + b.1)));
        for w in stations.windows(2) {
            let (_, s0, n0, o0) = w[0];
            let (s1, _, n1, o1) = w[1];
            if s1 - s0 < 3.0 || n0 == n1 {
                continue;
            }
            let p0 = s.a + dir * s0;
            let p1 = s.a + dir * s1;
            let open = |n: usize| s.median_openings.iter().any(|o| o.distance(nodes[n].0) < 12.0);
            let end = |orig: bool, n: usize| EndSpec {
                control: Control::None,
                crosswalk: if orig { s.crosswalk } else { None },
                crosswalk_width: 4.0,
                stop_line: if orig { s.stop_line } else { 1.0 },
                ahead_only: false,
                median_open: open(n),
            };
            spec.streets.push(StreetSpec {
                nodes: [n0, n1],
                points: vec![p0, p1],
                road_half: s.road_half,
                sidewalk: s.sidewalk,
                median_half: s.median_half,
                lanes: s.lanes.clone(),
                parking: s.parking.clone(),
                speed_limit: s.speed_limit,
                ends: [end(o0, n0), end(o1, n1)],
            });
        }
    }
    // 5. Signs.
    let uses = signs.iter().map(|sg| apply_sign(&mut spec, sg)).collect();
    (spec, uses)
}

/// Travel direction of the lanes arriving at `end` (`None` if no lane does).
fn arriving(s: &StreetSpec, end: usize) -> bool {
    s.lanes.iter().any(|l| l.forward == (end == 1))
}

/// Street end `(street, end)` whose stop zone a sign at `pos` facing
/// `facing` governs: drivers arriving there travel along `-facing`.
pub fn end_for(spec: &RoadGraphSpec, pos: Vec2, facing: Vec2, max_dist: f32) -> Option<(usize, usize)> {
    let travel = -facing.normalize_or_zero();
    let mut best: Option<(f32, usize, usize)> = None;
    for (i, s) in spec.streets.iter().enumerate() {
        let (a, b) = (s.points[0], *s.points.last().expect("points"));
        let len = a.distance(b).max(1e-3);
        let dir = (b - a) / len;
        for end in 0..2 {
            let (e, d) = if end == 0 { (a, -dir) } else { (b, dir) };
            if !arriving(s, end) || d.dot(travel) < 0.8 {
                continue;
            }
            let rel = pos - e;
            let along = -rel.dot(d);
            let lat = rel.perp_dot(d).abs();
            if along > -3.0 && along < max_dist && lat < s.road_half + s.sidewalk + 2.0 {
                let score = along.abs() + lat * 0.5;
                if best.is_none_or(|b| score < b.0) {
                    best = Some((score, i, end));
                }
            }
        }
    }
    best.map(|b| (b.1, b.2))
}

/// Street a sign stands beside (for speed and parking rules).
fn street_for(spec: &RoadGraphSpec, pos: Vec2) -> Option<(usize, f32)> {
    let mut best: Option<(f32, usize, f32)> = None;
    for (i, s) in spec.streets.iter().enumerate() {
        let (a, b) = (s.points[0], *s.points.last().expect("points"));
        let len = a.distance(b).max(1e-3);
        let dir = (b - a) / len;
        let rel = pos - a;
        let u = rel.dot(dir);
        let lat = rel.perp_dot(dir);
        if u > -2.0 && u < len + 2.0 && lat.abs() < s.road_half + s.sidewalk + 2.0 && best.is_none_or(|bb| lat.abs() < bb.0) {
            best = Some((lat.abs(), i, lat));
        }
    }
    best.map(|b| (b.1, -b.2))
}

fn apply_sign(spec: &mut RoadGraphSpec, sg: &SignIn) -> SignUse {
    match sg.rule {
        SignRule::Stop | SignRule::Yield | SignRule::AheadOnly => {
            let Some((i, e)) = end_for(spec, sg.pos, sg.facing, 16.0) else { return SignUse::Unbound };
            let end = &mut spec.streets[i].ends[e];
            match sg.rule {
                SignRule::Stop => end.control = Control::Stop,
                SignRule::Yield => {
                    if end.control != Control::Stop {
                        end.control = Control::Yield
                    }
                }
                _ => end.ahead_only = true,
            }
            SignUse::End(i, e)
        }
        SignRule::Speed(kmh) => {
            let Some((i, _)) = street_for(spec, sg.pos) else { return SignUse::Unbound };
            spec.streets[i].speed_limit = kmh as f32 * KMH;
            SignUse::Street(i)
        }
        SignRule::NoParking | SignRule::Parking => {
            let Some((i, _)) = street_for(spec, sg.pos) else { return SignUse::Unbound };
            let s = &mut spec.streets[i];
            let (a, b) = (s.points[0], *s.points.last().expect("points"));
            let lat = (sg.pos - a).dot(left((b - a).normalize_or(forja_math::Vec2::X)));
            // The strip on the sign's side of the street.
            if let Some(p) = s.parking.iter_mut().filter(|p| p.offset.signum() == lat.signum()).min_by(|x, y| (x.offset - lat).abs().total_cmp(&(y.offset - lat).abs())) {
                p.allowed = sg.rule == SignRule::Parking;
            }
            SignUse::Street(i)
        }
        SignRule::NoEntry => {
            // Plate faces drivers who would enter the street against its
            // one-way direction: drop any lane that enters there.
            let Some((i, _)) = street_for(spec, sg.pos) else { return SignUse::Unbound };
            let s = &mut spec.streets[i];
            let (a, b) = (s.points[0], *s.points.last().expect("points"));
            let dir = (b - a).normalize_or(forja_math::Vec2::X);
            let wrong = -sg.facing.normalize_or_zero();
            s.lanes.retain(|l| {
                let d = if l.forward { dir } else { -dir };
                d.dot(wrong) < 0.7
            });
            SignUse::Street(i)
        }
        SignRule::Crossing => SignUse::Unbound,
    }
}

/// Sets every approach with a signal head to [`Control::Signal`], then the
/// other unsigned approaches of the junction (a STOP or give-way plate at a
/// signalled junction stays: that driver gives way to the green traffic).
/// `heads` are `(position, plate normal)`.
pub fn apply_signal_heads(spec: &mut RoadGraphSpec, heads: &[(Vec2, Vec2)]) -> Vec<Option<(usize, usize)>> {
    let bound: Vec<Option<(usize, usize)>> = heads.iter().map(|(p, f)| end_for(spec, *p, *f, 16.0)).collect();
    let mut signal_nodes = vec![false; spec.nodes.len()];
    for (i, e) in bound.iter().flatten() {
        let s = &mut spec.streets[*i];
        s.ends[*e].control = Control::Signal;
        signal_nodes[s.nodes[*e]] = true;
    }
    for s in &mut spec.streets {
        for e in 0..2 {
            if signal_nodes[s.nodes[e]] && arriving(s, e) && s.ends[e].control == Control::None {
                s.ends[e].control = Control::Signal;
            }
        }
    }
    bound
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(a: Vec2, b: Vec2, hw: f32) -> SegmentIn {
        SegmentIn {
            a,
            b,
            half_width: hw,
            road_half: hw - 2.0,
            sidewalk: 2.0,
            median_half: 0.0,
            lanes: vec![LaneSpec { offset: -1.5, forward: true, width: 3.0 }, LaneSpec { offset: 1.5, forward: false, width: 3.0 }],
            parking: vec![],
            speed_limit: 50.0 * KMH,
            crosswalk: Some(3.0),
            stop_line: 6.0,
            median_openings: vec![],
        }
    }

    #[test]
    fn finds_junctions_and_splits_through_streets() {
        // An avenue along z crossed by two side streets ending at its
        // building lines, one per side at different z.
        let segs = vec![
            seg(Vec2::new(0.0, -100.0), Vec2::new(0.0, 100.0), 12.0),
            seg(Vec2::new(12.0, 20.0), Vec2::new(80.0, 20.0), 6.0),
            seg(Vec2::new(-80.0, -30.0), Vec2::new(-12.0, -30.0), 6.0),
        ];
        let signs = [SignIn { rule: SignRule::Stop, pos: Vec2::new(18.0, 16.0), facing: Vec2::X }, SignIn { rule: SignRule::Speed(30), pos: Vec2::new(40.0, 25.0), facing: -Vec2::X }];
        let (spec, uses) = build_graph(&segs, &signs);
        // Nodes: 2 avenue ends, 2 junctions on the avenue, 2 side street ends.
        assert_eq!(spec.nodes.len(), 6, "{:?}", spec.nodes);
        assert!(spec.nodes.iter().any(|n| n.pos.distance(Vec2::new(0.0, 20.0)) < 0.1));
        // Avenue split in three, each trimmed at the junction boxes.
        assert_eq!(spec.streets.len(), 5);
        let side = uses[0];
        let SignUse::End(i, e) = side else { panic!("{side:?}") };
        assert_eq!(spec.streets[i].ends[e].control, Control::Stop);
        assert!(spec.streets[i].points[0].distance(Vec2::new(12.0, 20.0)) < 0.1);
        assert!((spec.streets[i].speed_limit - 30.0 * KMH).abs() < 1e-3);
        let av = spec.streets.iter().find(|s| s.points[0].x.abs() < 0.1 && s.points[0].y > -50.0 && s.points[0].y < 0.0).expect("avenue piece");
        assert!((av.points[0].y - (-30.0 + 6.0)).abs() < 0.1, "{:?}", av.points);
    }
}
