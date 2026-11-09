//! Road meshes of the island: every road, path, track and railway as a
//! ribbon lying on the ground (the ground is levelled along the roads, see
//! [`Ground`]), bridges as decks between their banks.
//!
//! Ribbons are cut into the terrain's 512 m tiles, one primitive per
//! surface, in two sets: the main roads (always drawn) and the streets and
//! paths (drawn within a kilometre or so). Ponds, fountains and narrow
//! channels are drawn the same way, as sheets over the ground.

use std::collections::HashMap;

use forja_assets::MeshData;
use forja_math::{Vec2, Vec3, Vec4};

use super::data::{Line, Poly, RoadClass};
use super::ground::{Ground, TILE};
use super::tex::{ROAD_V, Surface};

/// Surface and height above the ground of a road class. Cars drive on the
/// ground itself (one continuous height field, levelled along the roads):
/// the ribbons are only drawn, a few centimetres over it, higher classes
/// over lower ones where they cross, so no road has a step to climb.
pub fn style(class: RoadClass) -> (Surface, f32) {
    use RoadClass::*;
    match class {
        Motorway | Trunk => (Surface::Highway, 0.050),
        MotorwayLink | TrunkLink => (Surface::Street, 0.047),
        Primary => (Surface::Road, 0.044),
        Secondary => (Surface::Road, 0.041),
        Tertiary => (Surface::Road, 0.038),
        PrimaryLink | SecondaryLink | TertiaryLink => (Surface::Street, 0.035),
        Residential | Unclassified | Busway | Raceway => (Surface::Street, 0.032),
        LivingStreet => (Surface::Street, 0.030),
        Service => (Surface::Street, 0.028),
        Pedestrian | Promenade => (Surface::Paved, 0.024),
        Cycleway => (Surface::Cycle, 0.021),
        Footway | Path | Steps => (Surface::Path, 0.018),
        Track => (Surface::Dirt, 0.016),
        Rail | Subway => (Surface::Rail, 0.052),
        Tram => (Surface::Tram, 0.046),
        Runway => (Surface::Runway, 0.040),
        Taxiway => (Surface::Concrete, 0.034),
    }
}

/// Main roads are drawn at any distance.
fn major(class: RoadClass) -> bool {
    use RoadClass::*;
    matches!(class, Motorway | Trunk | MotorwayLink | TrunkLink | Primary | Secondary | Tertiary | Runway | Taxiway | Rail)
}

/// Water bodies smaller than this (m2) are drawn as sheets over the
/// ground; larger ones are part of the ground itself (see [`Ground`]).
pub const SHEET_MAX: f32 = 60_000.0;
/// Water bodies from this size (m2) lower the ground under them.
pub const BASIN_MIN: f32 = 12_000.0;

/// Whether a line is drawn (tunnels and the metro are underground).
pub fn visible(l: &Line) -> bool {
    !l.tunnel() && l.class != RoadClass::Subway
}

#[derive(Default)]
struct Part {
    mesh: MeshData,
}

/// The road meshes of one terrain tile.
#[derive(Default)]
pub struct RoadTile {
    /// Main roads, railways, the runway and water sheets.
    pub major: Vec<(Surface, MeshData)>,
    /// Streets, paths and tracks.
    pub minor: Vec<(Surface, MeshData)>,
}

#[derive(Default)]
pub struct Roads {
    /// Meshes per terrain tile `(tx, tz)` (world position / 512 m).
    pub tiles: HashMap<(i32, i32), RoadTile>,
    /// Collision of the bridge decks and their parapets.
    pub col_verts: Vec<Vec3>,
    pub col_tris: Vec<[u32; 3]>,
    pub triangles: usize,
}

/// Appends `a` and the points between `a` and `b` (not `b`) where the
/// ground leaves the straight line between its neighbours by `tolerance`
/// (pieces no shorter than `shortest`).
fn follow(g: &Ground, a: Vec2, b: Vec2, tolerance: f32, shortest: f32, out: &mut Vec<Vec2>) {
    let len = a.distance(b);
    if len > shortest {
        let (ha, hb) = (g.height(a.x, a.y), g.height(b.x, b.y));
        let bends = [0.25f32, 0.5, 0.75].iter().any(|&t| {
            let p = a.lerp(b, t);
            (g.height(p.x, p.y) - (ha + (hb - ha) * t)).abs() > tolerance
        });
        if bends {
            let m = (a + b) * 0.5;
            follow(g, a, m, tolerance, shortest, out);
            follow(g, m, b, tolerance, shortest, out);
            return;
        }
    }
    out.push(a);
}

fn key(p: Vec2) -> (i32, i32) {
    ((p.x * 4.0).round() as i32, (p.y * 4.0).round() as i32)
}

/// Deck height of the bridges: each bridge (lines joined end to end) spans
/// between its banks, the ends where a road on the ground arrives.
struct Bridges {
    /// The span of each bridge line (by line index).
    anchors: HashMap<usize, std::sync::Arc<Span>>,
}

/// One bridge: the lines joined end to end between its banks.
#[derive(Default)]
struct Span {
    /// Banks `(position, ground height)`.
    anchors: Vec<(Vec2, f32)>,
    /// How far the deck rises over the line between its banks (a flyover
    /// over other roads; 0 for a bridge that only spans water).
    raise: f32,
    /// Ways it crosses without the room to clear them: the deck comes
    /// down to the ground there (a crossing at grade).
    grade: Vec<Vec2>,
}

/// Clearance of a flyover over the roads it crosses (m) and the length of
/// its ramps (a smooth S of at most 8 %).
const FLYOVER: f32 = 5.6;
const RAMP: f32 = 1.5 * FLYOVER / 0.08;

fn ramp(d: f32) -> f32 {
    let t = (d / RAMP).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Height of the straight span between `anchors` at `p` (inverse-distance
/// weighted, so it passes through every bank).
fn span_line(anchors: &[(Vec2, f32)], p: Vec2) -> f32 {
    let (mut sum, mut w) = (0.0, 0.0);
    for (q, h) in anchors {
        let d2 = q.distance_squared(p).max(0.01);
        sum += h / d2;
        w += 1.0 / d2;
    }
    if w > 0.0 { sum / w } else { 0.0 }
}

fn crossing(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> Option<Vec2> {
    let (r, s) = (b - a, d - c);
    let den = r.perp_dot(s);
    if den.abs() < 1e-6 {
        return None;
    }
    let (t, u) = ((c - a).perp_dot(s) / den, (c - a).perp_dot(r) / den);
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then(|| a + r * t)
}

impl Bridges {
    fn new(lines: &[Line], g: &Ground) -> Self {
        let mut ends: HashMap<(i32, i32), (Vec<usize>, bool)> = HashMap::new();
        for (i, l) in lines.iter().enumerate() {
            if !visible(l) {
                continue;
            }
            if l.bridge() {
                for p in [l.pts[0], *l.pts.last().expect("points")] {
                    ends.entry(key(p)).or_default().0.push(i);
                }
            }
        }
        for l in lines.iter().filter(|l| visible(l) && !l.bridge()) {
            for p in &l.pts {
                if let Some(e) = ends.get_mut(&key(*p)) {
                    e.1 = true;
                }
            }
        }
        // Union the lines that share an end.
        let mut parent: HashMap<usize, usize> = HashMap::new();
        fn find(parent: &mut HashMap<usize, usize>, i: usize) -> usize {
            let p = *parent.entry(i).or_insert(i);
            if p == i {
                return i;
            }
            let r = find(parent, p);
            parent.insert(i, r);
            r
        }
        for (ids, _) in ends.values() {
            for w in ids.windows(2) {
                let (a, b) = (find(&mut parent, w[0]), find(&mut parent, w[1]));
                if a != b {
                    parent.insert(a, b);
                }
            }
            if let Some(&i) = ids.first() {
                find(&mut parent, i);
            }
        }
        // ... and the lines whose decks overlap (a carriageway with its
        // cycle track and footway alongside): one bridge, one deck surface.
        let cell = 64.0;
        let mut near: HashMap<(i32, i32), Vec<(usize, Vec2, Vec2, f32)>> = HashMap::new();
        for (i, l) in lines.iter().enumerate().filter(|(_, l)| l.bridge() && visible(l)) {
            for w in l.pts.windows(2) {
                let (lo, hi) = ((w[0].min(w[1]) - Vec2::splat(l.width)) / cell, (w[0].max(w[1]) + Vec2::splat(l.width)) / cell);
                for cz in lo.y.floor() as i32..=hi.y.floor() as i32 {
                    for cx in lo.x.floor() as i32..=hi.x.floor() as i32 {
                        near.entry((cx, cz)).or_default().push((i, w[0], w[1], l.width * 0.5));
                    }
                }
            }
        }
        for (i, l) in lines.iter().enumerate().filter(|(_, l)| l.bridge() && visible(l)) {
            for w in l.pts.windows(2) {
                for k in 0..=4 {
                    let p = w[0].lerp(w[1], k as f32 / 4.0);
                    for &(j, a, b, half) in near.get(&((p.x / cell).floor() as i32, (p.y / cell).floor() as i32)).into_iter().flatten() {
                        let ab = b - a;
                        let t = ((p - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
                        if j != i && (a + ab * t).distance(p) < half + l.width * 0.5 + 1.0 {
                            let (ra, rb) = (find(&mut parent, i), find(&mut parent, j));
                            if ra != rb {
                                parent.insert(ra, rb);
                            }
                        }
                    }
                }
            }
        }
        let mut groups: HashMap<usize, Vec<(Vec2, f32)>> = HashMap::new();
        let mut fallback: HashMap<usize, Vec<(Vec2, f32)>> = HashMap::new();
        for (i, l) in lines.iter().enumerate() {
            if !l.bridge() || !visible(l) {
                continue;
            }
            let root = find(&mut parent, i);
            for p in [l.pts[0], *l.pts.last().expect("points")] {
                let (ids, grounded) = &ends[&key(p)];
                let a = (p, g.height(p.x, p.y));
                if *grounded || ids.len() == 1 {
                    groups.entry(root).or_default().push(a);
                }
                fallback.entry(root).or_default().push(a);
            }
        }
        // What each bridge crosses: the ways on the ground under it. A
        // bridge whose ramps can reach the clearance over all of them is a
        // flyover; one too short for that comes down to the ground where
        // they cross (a crossing at grade, with no parapets in the way).
        let cell = 128.0;
        let mut grid: HashMap<(i32, i32), Vec<(Vec2, Vec2)>> = HashMap::new();
        for l in lines.iter().filter(|l| visible(l) && !l.bridge()) {
            for w in l.pts.windows(2) {
                let (lo, hi) = (w[0].min(w[1]) / cell, w[0].max(w[1]) / cell);
                for cz in lo.y.floor() as i32..=hi.y.floor() as i32 {
                    for cx in lo.x.floor() as i32..=hi.x.floor() as i32 {
                        grid.entry((cx, cz)).or_default().push((w[0], w[1]));
                    }
                }
            }
        }
        let mut crossings: HashMap<usize, Vec<Vec2>> = HashMap::new();
        for (i, l) in lines.iter().enumerate() {
            if !l.bridge() || !visible(l) {
                continue;
            }
            let root = find(&mut parent, i);
            for w in l.pts.windows(2) {
                let (lo, hi) = (w[0].min(w[1]) / cell, w[0].max(w[1]) / cell);
                for cz in lo.y.floor() as i32..=hi.y.floor() as i32 {
                    for cx in lo.x.floor() as i32..=hi.x.floor() as i32 {
                        for (c, d) in grid.get(&(cx, cz)).into_iter().flatten() {
                            if let Some(x) = crossing(w[0], w[1], *c, *d) {
                                crossings.entry(root).or_default().push(x);
                            }
                        }
                    }
                }
            }
        }
        let roots: HashMap<usize, usize> = lines.iter().enumerate().filter(|(_, l)| l.bridge() && visible(l)).map(|(i, _)| i).collect::<Vec<_>>().into_iter().map(|i| (i, find(&mut parent, i))).collect();
        let mut shared: HashMap<usize, std::sync::Arc<Span>> = HashMap::new();
        let mut anchors = HashMap::new();
        for (i, l) in lines.iter().enumerate() {
            if !l.bridge() || !visible(l) {
                continue;
            }
            let root = find(&mut parent, i);
            let a = shared
                .entry(root)
                .or_insert_with(|| {
                    let anchors = groups.remove(&root).or_else(|| fallback.remove(&root)).unwrap_or_default();
                    let reach = |x: &Vec2| anchors.iter().map(|(q, _)| q.distance(*x)).fold(f32::MAX, f32::min);
                    // Crossings at a bank are junctions, not ways under the bridge.
                    let under: Vec<Vec2> = crossings.get(&root).into_iter().flatten().copied().filter(|x| reach(x) > 3.0).collect();
                    let clears = under.iter().all(|x| FLYOVER * ramp(reach(x)) >= 4.6);
                    let raise = if !under.is_empty() && clears { FLYOVER } else { 0.0 };
                    let mut span = Span { anchors, raise, grade: Vec::new() };
                    if !clears {
                        // Ways far below the span (a road in a gorge) pass
                        // under it as they are.
                        let grade = under.into_iter().filter(|x| span_line(&span.anchors, *x) - g.height(x.x, x.y) < 4.5).collect();
                        span.grade = grade;
                    }
                    if raise == 0.0 {
                        // A bridge that spans nothing lower than its banks
                        // (the old river bed is a level park) lies on the
                        // ground: a deck hovering a hand above it would only
                        // be a lip for whatever crosses it.
                        let line = |p: Vec2| span_line(&span.anchors, p);
                        let members = lines.iter().enumerate().filter(|(j, m)| m.bridge() && visible(m) && roots.get(j) == Some(&root));
                        let mut deep = 0.0f32;
                        for (_, m) in members {
                            for s in m.pts.windows(2) {
                                for k in 0..=8 {
                                    let p = s[0].lerp(s[1], k as f32 / 8.0);
                                    deep = deep.max(line(p) - g.height(p.x, p.y));
                                }
                            }
                        }
                        if deep < 0.6 {
                            span.anchors.clear();
                        }
                    }
                    std::sync::Arc::new(span)
                })
                .clone();
            anchors.insert(i, a);
        }
        Bridges { anchors }
    }

    /// How far the deck of bridge line `i` is above the ground at `p`.
    fn rise(&self, i: usize, p: Vec2, g: &Ground) -> f32 {
        let ground = g.height(p.x, p.y);
        let Some(span) = self.anchors.get(&i).filter(|a| !a.anchors.is_empty()) else { return 0.0 };
        let (mut sum, mut w, mut near) = (0.0, 0.0, f32::MAX);
        for (q, h) in span.anchors.iter() {
            let d2 = q.distance_squared(p).max(0.01);
            sum += h / d2;
            w += 1.0 / d2;
            near = near.min(d2);
        }
        // The deck is the straight span between its banks.
        let rise = (sum / w + span.raise * ramp(near.sqrt()) - ground).max(0.0);
        // Down to the ground where a way crosses at grade.
        let to_crossing = span.grade.iter().map(|x| x.distance(p)).fold(f32::MAX, f32::min);
        let t = ((to_crossing - 8.0) / 52.0).clamp(0.0, 1.0);
        rise * t * t * (3.0 - 2.0 * t)
    }
}

impl Roads {
    pub fn build(lines: &[Line], polys: &[Poly], g: &Ground) -> Roads {
        let bridges = Bridges::new(lines, g);
        // Where bridge decks merge or run side by side, no parapet stands
        // on a neighbour's deck.
        let cell = 64.0;
        let mut decks: HashMap<(i32, i32), Vec<(usize, Vec2, Vec2, f32)>> = HashMap::new();
        for (li, l) in lines.iter().enumerate().filter(|(_, l)| visible(l) && l.bridge()) {
            for w in l.pts.windows(2) {
                let (lo, hi) = ((w[0].min(w[1]) - Vec2::splat(l.width)) / cell, (w[0].max(w[1]) + Vec2::splat(l.width)) / cell);
                for cz in lo.y.floor() as i32..=hi.y.floor() as i32 {
                    for cx in lo.x.floor() as i32..=hi.x.floor() as i32 {
                        decks.entry((cx, cz)).or_default().push((li, w[0], w[1], l.width * 0.5));
                    }
                }
            }
        }
        let on_other_deck = |p: Vec2, own: usize| -> bool {
            decks.get(&((p.x / cell).floor() as i32, (p.y / cell).floor() as i32)).into_iter().flatten().any(|&(li, a, b, half)| {
                let ab = b - a;
                let t = ((p - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
                li != own && (a + ab * t).distance(p) < half + 0.3
            })
        };
        let mut parts: HashMap<((i32, i32), Surface, bool), Part> = HashMap::new();
        let mut roads = Roads::default();
        // Junction discs: the widest road arriving at each line end.
        let mut caps: HashMap<(i32, i32), (f32, usize, Vec2)> = HashMap::new();
        for (li, l) in lines.iter().enumerate() {
            if !visible(l) {
                continue;
            }
            let (surface, lift) = style(l.class);
            let imp = major(l.class);
            let bridge = l.bridge();
            let half = l.width * 0.5;
            if !bridge && l.width >= 5.0 && !l.class.is_rail() {
                for p in [l.pts[0], *l.pts.last().expect("points")] {
                    let e = caps.entry(key(p)).or_insert((0.0, li, p));
                    if l.width > e.0 {
                        *e = (l.width, li, p);
                    }
                }
            }
            // Cut where the ground bends, so the ribbon follows it within
            // a centimetre.
            let mut pts: Vec<Vec2> = Vec::with_capacity(l.pts.len());
            for w in l.pts.windows(2) {
                if bridge {
                    // Decks follow the ground they leave and return to
                    // within a millimetre or two (no lip at their edges
                    // near the banks), in pieces of at most 10 m.
                    let k = (w[0].distance(w[1]) / 10.0).ceil().max(1.0) as usize;
                    for s in 0..k {
                        follow(g, w[0].lerp(w[1], s as f32 / k as f32), w[0].lerp(w[1], (s + 1) as f32 / k as f32), 0.0015, 1.0, &mut pts);
                    }
                } else {
                    follow(g, w[0], w[1], 0.008, 4.0, &mut pts);
                }
            }
            pts.push(*l.pts.last().expect("points"));
            let n = pts.len();
            let height = |p: Vec2| -> f32 { g.height(p.x, p.y) + lift };
            // Mitred sides.
            let dir = |i: usize| (pts[(i + 1).min(n - 1)] - pts[i.min(n - 2)]).normalize_or(Vec2::X);
            let mut prev: Option<((i32, i32), u32, u32)> = None;
            let mut along = 0.0f32;
            let mut side_prev: Option<(Vec3, Vec3)> = None;
            let mut rise_prev = (0.0f32, 0.0f32);
            for i in 0..n {
                let (d0, d1) = (dir(i.saturating_sub(1)), dir(i));
                let nrm = (Vec2::new(d0.y, -d0.x) + Vec2::new(d1.y, -d1.x)).normalize_or(Vec2::new(d1.y, -d1.x));
                let miter = (1.0 / nrm.dot(Vec2::new(d1.y, -d1.x)).max(0.5)).min(2.0);
                if i > 0 {
                    along += pts[i].distance(pts[i - 1]);
                }
                let (l2, r2) = (pts[i] - nrm * half * miter, pts[i] + nrm * half * miter);
                // A deck rides over the ground under each edge by the same
                // rise, so it is exactly the ground where it has none.
                // (The rise is a function of the place alone, so decks of
                // one bridge that merge or overlap coincide.)
                let rise = |p: Vec2| if bridge { bridges.rise(li, p, g) } else { 0.0 };
                let (rl, rr) = (rise(l2), rise(r2));
                let (hl, hr) = (height(l2) + rl, height(r2) + rr);
                let (pl, pr) = (Vec3::new(l2.x, hl, l2.y), Vec3::new(r2.x, hr, r2.y));
                // The quad from the previous cross-section belongs to the
                // tile of its middle.
                let mid = if i > 0 { (pts[i] + pts[i - 1]) * 0.5 } else { pts[i] };
                let tile = ((mid.x / TILE).floor() as i32, (mid.y / TILE).floor() as i32);
                let part = parts.entry((tile, surface, imp)).or_default();
                let v = along / ROAD_V;
                let push = |part: &mut Part, p: Vec3, u: f32, v: f32| -> u32 {
                    let k = part.mesh.positions.len() as u32;
                    part.mesh.positions.push(p);
                    part.mesh.normals.push(Vec3::Y);
                    part.mesh.uvs.push(Vec2::new(u, v));
                    part.mesh.tangents.push(Vec4::new(1.0, 0.0, 0.0, 1.0));
                    k
                };
                if i > 0 {
                    let (a, b) = match prev {
                        Some((t, a, b)) if t == tile => (a, b),
                        _ => {
                            // Previous cross-section again, in this tile.
                            let (pl0, pr0) = side_prev.expect("previous side");
                            let v0 = (along - pts[i].distance(pts[i - 1])) / ROAD_V;
                            (push(part, pl0, 0.0, v0), push(part, pr0, 1.0, v0))
                        }
                    };
                    let (c, d) = (push(part, pl, 0.0, v), push(part, pr, 1.0, v));
                    // Left is -normal: (a, b) = (left, right) of the previous
                    // section; counter-clockwise seen from above.
                    let idx = [a, c, d, a, d, b];
                    part.mesh.indices.extend(idx);
                    prev = Some((tile, c, d));
                    if bridge {
                        let (pl0, pr0) = side_prev.expect("previous side");
                        let conc = parts.entry((tile, Surface::Concrete, imp)).or_default();
                        let rises = [rise_prev.0, rl, rr, rise_prev.1];
                        let (clear, top) = (rises.iter().copied().fold(f32::MAX, f32::min), rises.iter().copied().fold(0.0, f32::max));
                        // A deck from where it leaves the ground.
                        if top > 0.0 {
                            let mid = |a: Vec3, b: Vec3| Vec2::new((a.x + b.x) * 0.5, (a.z + b.z) * 0.5);
                            let high = clear > 0.7;
                            roads.deck(conc, [pl0, pl, pr, pr0], lift, rises, [high && !on_other_deck(mid(pl0, pl), li), high && !on_other_deck(mid(pr0, pr), li)]);
                        }
                    }
                }
                side_prev = Some((pl, pr));
                rise_prev = (rl, rr);
            }
        }
        for (_, (w, li, p)) in caps {
            let l = &lines[li];
            let (surface, lift) = style(l.class);
            let tile = ((p.x / TILE).floor() as i32, (p.y / TILE).floor() as i32);
            let part = parts.entry((tile, surface, major(l.class))).or_default();
            let base = part.mesh.positions.len() as u32;
            let r = w * 0.5;
            let c = Vec3::new(p.x, g.height(p.x, p.y) + lift - 0.002, p.y);
            part.mesh.positions.push(c);
            for k in 0..8 {
                let a = k as f32 * std::f32::consts::TAU / 8.0;
                let q = p + Vec2::new(a.cos(), a.sin()) * r;
                part.mesh.positions.push(Vec3::new(q.x, g.height(q.x, q.y) + lift - 0.002, q.y));
            }
            for k in 0..9 {
                part.mesh.normals.push(Vec3::Y);
                // Plain asphalt of the texture, away from the markings.
                part.mesh.uvs.push(Vec2::new(0.25, 0.7 + k as f32 * 0.01));
                part.mesh.tangents.push(Vec4::new(1.0, 0.0, 0.0, 1.0));
            }
            for k in 0..8u32 {
                let idx = [base, base + 1 + (k + 1) % 8, base + 1 + k];
                part.mesh.indices.extend(idx);
            }
        }
        // Small water: ponds, fountains, docks and channels narrower than
        // the ground's cells, as sheets just over the ground.
        for p in polys.iter().filter(|p| p.kind.is_water() && super::data::ring_area(&p.rings[0]).abs() < SHEET_MAX) {
            let (pts, tris) = forja_city::landmark::earcut::triangulate_with_holes(&p.rings[0], &p.rings[1..]);
            if tris.is_empty() {
                continue;
            }
            let c = super::data::ring_centroid(&p.rings[0]);
            let tile = ((c.x / TILE).floor() as i32, (c.y / TILE).floor() as i32);
            let part = parts.entry((tile, Surface::Water, true)).or_default();
            let base = part.mesh.positions.len() as u32;
            for q in &pts {
                part.mesh.positions.push(Vec3::new(q.x, g.height(q.x, q.y) + 0.012, q.y));
                part.mesh.normals.push(Vec3::Y);
                part.mesh.uvs.push(*q / 6.0);
                part.mesh.tangents.push(Vec4::new(1.0, 0.0, 0.0, 1.0));
            }
            for t in &tris {
                part.mesh.indices.extend([base + t[0], base + t[2], base + t[1]]);
            }
        }
        for ((tile, surface, main), mut part) in parts {
            if part.mesh.indices.is_empty() {
                continue;
            }
            roads.triangles += part.mesh.indices.len() / 3;
            part.mesh.aabb = forja_math::Aabb::from_points(part.mesh.positions.iter().copied());
            let t = roads.tiles.entry(tile).or_default();
            if main { t.major.push((surface, part.mesh)) } else { t.minor.push((surface, part.mesh)) }
        }
        for t in roads.tiles.values_mut() {
            t.major.sort_by_key(|(s, _)| *s);
            t.minor.sort_by_key(|(s, _)| *s);
        }
        roads
    }

    /// Sides and parapets of a bridge deck quad `[left0, left1, right1,
    /// right0]`, with their collision; `walls`: a parapet on the left and
    /// on the right.
    fn deck(&mut self, part: &mut Part, q: [Vec3; 4], lift: f32, rises: [f32; 4], walls: [bool; 2]) {
        const THICK: f32 = 0.7;
        const RAIL: f32 = 1.0;
        let quad = |part: &mut Part, p: [Vec3; 4]| {
            let n = (p[1] - p[0]).cross(p[3] - p[0]).normalize_or(Vec3::Y);
            let b = part.mesh.positions.len() as u32;
            let len = p[0].distance(p[1]);
            let hgt = p[0].distance(p[3]);
            for (k, v) in p.iter().enumerate() {
                part.mesh.positions.push(*v);
                part.mesh.normals.push(n);
                part.mesh.uvs.push(Vec2::new(if k == 1 || k == 2 { len / 4.0 } else { 0.0 }, if k >= 2 { hgt / 4.0 } else { 0.0 }));
                part.mesh.tangents.push(Vec4::new(1.0, 0.0, 0.0, 1.0));
            }
            part.mesh.indices.extend([b, b + 1, b + 2, b, b + 2, b + 3]);
        };
        let down = Vec3::Y * THICK;
        // The deck collides where the ground would be (the ribbon is drawn
        // `lift` over it), so it meets the ground at both banks without a step.
        // Near the banks, where it has hardly left the ground, it collides
        // 5 cm lower still: the deck then emerges from under the ground
        // (one continuous surface) instead of lying a few millimetres over
        // it with a lip along its edges.
        let mut c = q;
        for (p, r) in c.iter_mut().zip(rises) {
            let t = ((r - 0.3) / 0.7).clamp(0.0, 1.0);
            p.y -= lift + 0.05 * (1.0 - t * t * (3.0 - 2.0 * t));
        }
        let base = self.col_verts.len() as u32;
        self.col_verts.extend(c);
        self.col_tris.extend([[base, base + 1, base + 2], [base, base + 2, base + 3]]);
        // Left side, then right side: the edge of the deck and, where the
        // deck is well above the ground and no other deck joins, a parapet.
        for (k, (a, b)) in [(q[1], q[0]), (q[3], q[2])].into_iter().enumerate() {
            let up = Vec3::Y * if walls[k] { RAIL } else { 0.0 };
            quad(part, [a - down, b - down, b + up, a + up]);
            quad(part, [b - down, a - down, a + up, b + up]);
            if walls[k] {
                let base = self.col_verts.len() as u32;
                self.col_verts.extend([a - Vec3::Y * lift, b - Vec3::Y * lift, b + up, a + up]);
                self.col_tris.extend([[base, base + 1, base + 2], [base, base + 2, base + 3]]);
            }
        }
    }
}
