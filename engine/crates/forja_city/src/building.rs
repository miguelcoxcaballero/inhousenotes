//! Buildings from a lot footprint and a style: walls with real openings
//! and reveals, mouldings swept along the street front, instanced window,
//! balcony, shop and portal modules, and the roof.

use std::collections::HashMap;
use std::sync::Arc;

use forja_assets::SceneData;
use forja_math::{Mat4, Vec2, Vec3};

use crate::furniture;
use crate::geom::{Parts, Prim, Rng, polyline_normals, signed_area};
use crate::kit::{Kit, Mat, Rgb, assemble, hex};
use crate::nightlight::{self, CityLight, LightKind, NightState, Premises, hash01};
use crate::modules::{self, BalconyKey, P, Piece, PortalKey, REVEAL, ShopKey, ShopPiece, SurroundKind, WindowKey, facade_frame};
use crate::style::{ShutterKind, Style, parse_material, parse_pattern, parse_rgb, parse_shutter};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeKind {
    /// Main facade on a street.
    Street,
    /// Street facade on a chamfered or rounded block corner.
    Corner,
    /// Facade on the block's inner courtyard.
    Courtyard,
    /// Wall shared with the neighbour (medianera).
    Party,
}

impl EdgeKind {
    pub fn is_street(self) -> bool {
        matches!(self, EdgeKind::Street | EdgeKind::Corner)
    }
}

/// A building plot: counter-clockwise ring in (x, z) and one kind per edge
/// (`edges[i]` runs from `ring[i]` to `ring[i + 1]`).
#[derive(Clone, Debug)]
pub struct Lot {
    pub ring: Vec<Vec2>,
    pub edges: Vec<EdgeKind>,
    /// Ground level (top of the sidewalk).
    pub base: f32,
}

/// Everything a city build produces, in world space.
#[derive(Default)]
pub struct CityOut {
    pub scenes: Vec<Arc<SceneData>>,
    pub placements: Vec<(usize, Mat4)>,
    pub col_verts: Vec<Vec3>,
    pub col_tris: Vec<[u32; 3]>,
    /// `(position, color)` of lamps (kept for simple consumers; see
    /// `city_lights` for the full records).
    pub lights: Vec<(Vec3, Vec3)>,
    /// Every light of the city with intensity, range and optional spot
    /// cone: lamps always, and at night ([`Kit::set_hour`]) lit windows,
    /// shops, signs, neon, floodlights and fountains. Feed a renderer a
    /// bounded subset with [`nightlight::select`].
    pub city_lights: Vec<CityLight>,
    pub buildings: usize,
    /// Ground-floor premises on street fronts, for sidewalk dressing
    /// (bar terraces) by whoever knows the sidewalk widths.
    pub frontages: Vec<Frontage>,
    /// Pharmacies placed so far: real ones are licensed a few hundred
    /// metres apart, so a street never shows a row of green crosses.
    pub pharmacies: Vec<Vec3>,
    /// Party walls waiting for `medianeras` (built once the neighbours on
    /// both sides are known).
    pub party_walls: Vec<PartyWall>,
}

/// A medianera: the wall a building shares with its neighbour, recorded by
/// `build_into` and built by `medianeras`.
#[derive(Clone, Debug)]
pub struct PartyWall {
    /// Ring-order endpoints; the wall faces out of the lot, to the right
    /// of `a -> b`.
    pub a: Vec2,
    pub b: Vec2,
    pub base: f32,
    /// Absolute eave height; the wall rises a metre above it.
    pub eave: f32,
    pub mat: Mat,
    pub uv_off: f32,
    /// Absolute floor levels of the upper storeys.
    pub floors: Vec<f32>,
    /// The end of the wall at the street front, if any.
    pub street_at_a: bool,
    pub street_at_b: bool,
    pub seed: u64,
}

/// Minimum distance between two pharmacies.
pub const PHARMACY_SPACING: f32 = 140.0;

/// One shopfront at street level.
#[derive(Clone, Copy, Debug)]
pub struct Frontage {
    /// Bottom centre of the opening on the facade plane.
    pub pos: Vec3,
    /// Along the facade (right when facing it) and out of the wall.
    pub right: Vec3,
    pub out: Vec3,
    pub width: f32,
    /// Café, taberna or bodega (gets a terrace on wide sidewalks).
    pub bar: bool,
    /// Its shutter is down (closed premises get no terrace).
    pub closed: bool,
}

impl CityOut {
    pub fn add_scene(&mut self, s: SceneData) -> usize {
        self.scenes.push(Arc::new(s));
        self.scenes.len() - 1
    }

    pub fn collider_quad(&mut self, p: [Vec3; 4]) {
        let b = self.col_verts.len() as u32;
        self.col_verts.extend(p);
        self.col_tris.extend([[b, b + 1, b + 2], [b, b + 2, b + 3]]);
    }

    pub fn collider_polygon(&mut self, ring: &[Vec2], y: f32) {
        if let Some(t) = crate::geom::triangulate(ring) {
            let b = self.col_verts.len() as u32;
            self.col_verts.extend(ring.iter().map(|p| Vec3::new(p.x, y, p.y)));
            self.col_tris.extend(t.iter().map(|t| [b + t[0], b + t[1], b + t[2]]));
        }
    }
}

/// Cache of instanced modules (each a render scene).
pub struct ModuleLib<'k> {
    pub kit: &'k Kit,
    windows: HashMap<(WindowKey, Piece), Option<usize>>,
    shops: HashMap<(ShopKey, ShopPiece), Option<usize>>,
    portals: HashMap<PortalKey, usize>,
    named: HashMap<String, usize>,
}

impl<'k> ModuleLib<'k> {
    pub fn new(kit: &'k Kit) -> Self {
        Self { kit, windows: HashMap::new(), shops: HashMap::new(), portals: HashMap::new(), named: HashMap::new() }
    }

    fn make(&self, out: &mut CityOut, name: &str, p: P) -> usize {
        out.add_scene(assemble(self.kit, name, p, Some(400)))
    }

    /// Instanced pieces of a window (surround, glazing, interior, shutter,
    /// balcony).
    pub fn window(&mut self, out: &mut CityOut, k: WindowKey) -> Vec<usize> {
        let mut ids = Vec::with_capacity(5);
        for piece in Piece::ALL {
            let pk = k.for_piece(piece);
            let key = (pk, piece);
            if let Some(&i) = self.windows.get(&key) {
                ids.extend(i);
                continue;
            }
            let p = modules::window_piece(&pk, piece);
            let i = (!p.prims.is_empty() && p.tri_count() > 0).then(|| self.make(out, "window", p));
            self.windows.insert(key, i);
            ids.extend(i);
        }
        ids
    }

    pub fn shop(&mut self, out: &mut CityOut, k: ShopKey) -> Vec<usize> {
        let mut ids = Vec::with_capacity(6);
        for piece in ShopPiece::ALL {
            let pk = k.for_piece(piece);
            let key = (pk, piece);
            if let Some(&i) = self.shops.get(&key) {
                ids.extend(i);
                continue;
            }
            let p = modules::shop_piece(&pk, piece);
            let i = (p.tri_count() > 0).then(|| self.make(out, "shop", p));
            self.shops.insert(key, i);
            ids.extend(i);
        }
        ids
    }

    pub fn portal(&mut self, out: &mut CityOut, k: PortalKey) -> usize {
        if let Some(&i) = self.portals.get(&k) {
            return i;
        }
        let i = self.make(out, "portal", modules::portal(&k));
        self.portals.insert(k, i);
        i
    }

    /// Any other module, cached by `name`.
    pub fn named(&mut self, out: &mut CityOut, name: &str, build: impl FnOnce() -> P) -> usize {
        if let Some(&i) = self.named.get(name) {
            return i;
        }
        let i = self.make(out, name, build());
        self.named.insert(name.to_string(), i);
        i
    }
}

/// Rectangular hole in facade coordinates (u along the facade, v up).
#[derive(Clone, Copy, Debug)]
struct Hole {
    u0: f32,
    u1: f32,
    v0: f32,
    v1: f32,
}

/// Facade coordinate frame of one edge.
#[derive(Clone, Copy)]
struct Frame {
    origin: Vec3,
    t: Vec3,
    n: Vec3,
    len: f32,
}

impl Frame {
    fn p(&self, u: f32, v: f32, out: f32) -> Vec3 {
        self.origin + self.t * u + Vec3::Y * v + self.n * out
    }
}

fn edge_frame(a: Vec2, b: Vec2, base: f32) -> Frame {
    let d = b - a;
    let n = Vec3::new(d.y, 0.0, -d.x).normalize_or(Vec3::Z);
    // Right when facing the wall = up × n, which runs from b to a.
    let t = Vec3::Y.cross(n);
    Frame { origin: Vec3::new(b.x, base, b.y), t, n, len: d.length() }
}

/// Quad with a requested facing (flipped if needed).
fn quad_facing(prim: &mut Prim, p: [Vec3; 4], want: Vec3, uv: [Vec2; 4]) {
    let n = (p[1] - p[0]).cross(p[3] - p[0]);
    if n.dot(want) >= 0.0 {
        prim.quad_uv(p, uv);
    } else {
        prim.quad_uv([p[1], p[0], p[3], p[2]], [uv[1], uv[0], uv[3], uv[2]]);
    }
}

/// Wall between `v0..v1` over the whole edge with rectangular holes, the
/// reveals of those holes, and optional rusticated courses.
fn wall(prim: &mut Prim, m: Mat, f: &Frame, v0: f32, v1: f32, holes: &[Hole], uv_off: f32, rustic: bool) {
    let s = m.scale();
    let mut us = vec![0.0, f.len];
    let mut vs = vec![v0, v1];
    let inside: Vec<Hole> = holes.iter().copied().filter(|h| h.v1 > v0 + 1e-4 && h.v0 < v1 - 1e-4).collect();
    for h in &inside {
        us.extend([h.u0, h.u1]);
        vs.extend([h.v0.max(v0), h.v1.min(v1)]);
    }
    us.sort_by(f32::total_cmp);
    us.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
    vs.sort_by(f32::total_cmp);
    vs.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
    let uv = |u: f32, v: f32| Vec2::new((u + uv_off) / s, -v / s);
    for j in 0..vs.len() - 1 {
        let (va, vb) = (vs[j], vs[j + 1]);
        let vm = (va + vb) * 0.5;
        let mut i = 0;
        while i < us.len() - 1 {
            let solid = |i: usize| {
                let um = (us[i] + us[i + 1]) * 0.5;
                !inside.iter().any(|h| um > h.u0 && um < h.u1 && vm > h.v0 && vm < h.v1)
            };
            if !solid(i) {
                i += 1;
                continue;
            }
            let start = i;
            while i < us.len() - 1 && solid(i) {
                i += 1;
            }
            let (ua, ub) = (us[start], us[i]);
            prim.quad_uv([f.p(ua, va, 0.0), f.p(ub, va, 0.0), f.p(ub, vb, 0.0), f.p(ua, vb, 0.0)], [uv(ua, va), uv(ub, va), uv(ub, vb), uv(ua, vb)]);
            if rustic {
                // Courses of 45 cm proud of the wall with sunk joints.
                let course = 0.45;
                let mut c = (va / course).floor() * course;
                while c < vb {
                    let (ca, cb) = ((c + 0.03).max(va), (c + course - 0.03).min(vb));
                    if cb - ca > 0.05 {
                        let o = 0.025;
                        prim.quad_uv([f.p(ua, ca, o), f.p(ub, ca, o), f.p(ub, cb, o), f.p(ua, cb, o)], [uv(ua, ca), uv(ub, ca), uv(ub, cb), uv(ua, cb)]);
                        quad_facing(prim, [f.p(ua, cb, 0.0), f.p(ub, cb, 0.0), f.p(ub, cb, o), f.p(ua, cb, o)], Vec3::Y, [uv(ua, 0.0), uv(ub, 0.0), uv(ub, 0.02), uv(ua, 0.02)]);
                        quad_facing(prim, [f.p(ua, ca, 0.0), f.p(ub, ca, 0.0), f.p(ub, ca, o), f.p(ua, ca, o)], Vec3::NEG_Y, [uv(ua, 0.0), uv(ub, 0.0), uv(ub, 0.02), uv(ua, 0.02)]);
                    }
                    c += course;
                }
            }
        }
    }
    // Reveals: the wall thickness seen around each opening.
    let r = REVEAL;
    for h in &inside {
        let (ha, hb) = (h.v0.max(v0), h.v1.min(v1));
        let d = r / s;
        quad_facing(prim, [f.p(h.u0, ha, 0.0), f.p(h.u0, ha, -r), f.p(h.u0, hb, -r), f.p(h.u0, hb, 0.0)], f.t, [uv(h.u0, ha), uv(h.u0 + r, ha), uv(h.u0 + r, hb), uv(h.u0, hb)]);
        quad_facing(prim, [f.p(h.u1, ha, 0.0), f.p(h.u1, ha, -r), f.p(h.u1, hb, -r), f.p(h.u1, hb, 0.0)], -f.t, [uv(h.u1, ha), uv(h.u1 + r, ha), uv(h.u1 + r, hb), uv(h.u1, hb)]);
        if hb >= h.v1 - 1e-4 {
            quad_facing(prim, [f.p(h.u0, hb, 0.0), f.p(h.u1, hb, 0.0), f.p(h.u1, hb, -r), f.p(h.u0, hb, -r)], Vec3::NEG_Y, [uv(h.u0, 0.0), uv(h.u1, 0.0), Vec2::new((h.u1 + uv_off) / s, d), Vec2::new((h.u0 + uv_off) / s, d)]);
        }
        if ha <= h.v0 + 1e-4 && h.v0 > v0 + 0.05 {
            quad_facing(prim, [f.p(h.u0, ha, 0.0), f.p(h.u1, ha, 0.0), f.p(h.u1, ha, -r), f.p(h.u0, ha, -r)], Vec3::Y, [uv(h.u0, 0.0), uv(h.u1, 0.0), Vec2::new((h.u1 + uv_off) / s, d), Vec2::new((h.u0 + uv_off) / s, d)]);
        }
    }
}

/// Flat decorative band (tiles) on the wall, interrupted by holes.
fn band(prim: &mut Prim, m: Mat, f: &Frame, v0: f32, v1: f32, u0: f32, u1: f32, holes: &[Hole]) {
    let s = m.scale();
    let mut cuts: Vec<(f32, f32)> = holes.iter().filter(|h| h.v1 > v0 && h.v0 < v1).map(|h| (h.u0 - 0.17, h.u1 + 0.17)).collect();
    cuts.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut u = u0;
    let mut spans = Vec::new();
    for (a, b) in cuts {
        if a > u {
            spans.push((u, a.min(u1)));
        }
        u = u.max(b);
    }
    if u < u1 {
        spans.push((u, u1));
    }
    let o = 0.012;
    for (a, b) in spans {
        if b - a < 0.05 {
            continue;
        }
        let uv = |uu: f32, vv: f32| Vec2::new(uu / s, -vv / s);
        prim.quad_uv([f.p(a, v0, o), f.p(b, v0, o), f.p(b, v1, o), f.p(a, v1, o)], [uv(a, v0), uv(b, v0), uv(b, v1), uv(a, v1)]);
    }
}

/// Stone plinth (zócalo) along the foot of a facade, `h` high and a few
/// centimetres proud, broken by the openings that reach the ground.
fn plinth(prim: &mut Prim, m: Mat, f: &Frame, h: f32, holes: &[Hole]) {
    let s = m.scale();
    let o = 0.035;
    let mut cuts: Vec<(f32, f32)> = holes.iter().filter(|x| x.v0 < h).map(|x| (x.u0, x.u1)).collect();
    cuts.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut spans = Vec::new();
    let mut u = 0.0f32;
    for (a, b) in cuts {
        if a > u + 0.02 {
            spans.push((u, a));
        }
        u = u.max(b);
    }
    if f.len > u + 0.02 {
        spans.push((u, f.len));
    }
    let uv = |a: f32, b: f32| Vec2::new(a / s, -b / s);
    for (a, b) in spans {
        prim.quad_uv([f.p(a, 0.0, o), f.p(b, 0.0, o), f.p(b, h, o), f.p(a, h, o)], [uv(a, 0.0), uv(b, 0.0), uv(b, h), uv(a, h)]);
        quad_facing(prim, [f.p(a, h, 0.0), f.p(b, h, 0.0), f.p(b, h, o), f.p(a, h, o)], Vec3::Y, [uv(a, 0.0), uv(b, 0.0), uv(b, o), uv(a, o)]);
        for (x, dir) in [(a, -f.t), (b, f.t)] {
            quad_facing(prim, [f.p(x, 0.0, 0.0), f.p(x, 0.0, o), f.p(x, h, o), f.p(x, h, 0.0)], dir, [uv(0.0, 0.0), uv(o, 0.0), uv(o, h), uv(0.0, h)]);
        }
    }
}

/// Inset of a convex counter-clockwise ring by `d` (negative grows it).
pub fn inset(ring: &[Vec2], d: f32) -> Option<Vec<Vec2>> {
    let n = ring.len();
    let lines: Vec<(Vec2, Vec2)> = (0..n)
        .map(|i| {
            let (a, b) = (ring[i], ring[(i + 1) % n]);
            let dir = (b - a).normalize_or(Vec2::X);
            let inward = Vec2::new(-dir.y, dir.x);
            (a + inward * d, dir)
        })
        .collect();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let (p0, d0) = lines[(i + n - 1) % n];
        let (p1, d1) = lines[i];
        let den = d0.perp_dot(d1);
        if den.abs() < 1e-5 {
            out.push(p1);
            continue;
        }
        let t = (p1 - p0).perp_dot(d1) / den;
        out.push(p0 + d0 * t);
    }
    let ok = signed_area(&out) > 0.0 && (0..n).all(|i| (out[(i + 1) % n] - out[i]).dot(ring[(i + 1) % n] - ring[i]) > 0.0);
    ok.then_some(out)
}

/// Chains of consecutive street edges: lists of edge indices in ring order.
fn street_chains(edges: &[EdgeKind]) -> Vec<Vec<usize>> {
    let n = edges.len();
    let Some(start) = (0..n).find(|&i| !edges[i].is_street()) else {
        return vec![(0..n).collect()];
    };
    let mut chains = Vec::new();
    let mut cur: Vec<usize> = Vec::new();
    for k in 1..=n {
        let i = (start + k) % n;
        if edges[i].is_street() {
            cur.push(i);
        } else if !cur.is_empty() {
            chains.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        chains.push(cur);
    }
    chains
}

struct Choices {
    wall: Mat,
    ground: Mat,
    surround: Mat,
    sk: SurroundKind,
    /// Richer surround of the piso principal.
    sk_principal: SurroundKind,
    frame: Rgb,
    shutter: (ShutterKind, Rgb),
    belly: bool,
    floors: u32,
    gh: f32,
    fh: f32,
    /// `(base, height)` of every upper floor, piso principal first.
    levels: Vec<(f32, f32)>,
    ww: f32,
    wh: f32,
    mirador: bool,
    /// An extra storey set back behind a terrace.
    atico: bool,
    /// Medianera finish where it shows above a lower neighbour.
    party: Mat,
}

impl Choices {
    /// Facade height above the lot base (top of the last floor).
    fn top(&self) -> f32 {
        self.levels.last().map_or(self.gh, |(y, h)| y + h)
    }

    /// Opening height on floor `fl` (taller on the principal, shorter on top).
    fn door_h(&self, fl: usize) -> f32 {
        let h = self.levels[fl].1;
        (self.wh + (h - self.fh) * 0.8).min(h - 0.75)
    }

    fn surround_on(&self, fl: usize) -> SurroundKind {
        if fl == 0 && self.levels.len() >= 3 {
            self.sk_principal
        } else if fl + 1 == self.levels.len() && self.levels.len() >= 4 {
            match self.sk {
                SurroundKind::Pediment => SurroundKind::Lintel,
                SurroundKind::None => SurroundKind::None,
                _ => SurroundKind::Plain,
            }
        } else {
            self.sk
        }
    }
}

/// Upper floors as `(base, height)`: a taller piso principal, regular
/// floors and a lower top floor, like real 19th-century fronts. Heights
/// are quantised to 5 cm so window modules keep repeating.
pub fn floor_levels(gh: f32, fh: f32, floors: u32) -> Vec<(f32, f32)> {
    let q = |x: f32| (x * 20.0).round() / 20.0;
    let mut y = gh;
    (0..floors as usize)
        .map(|i| {
            let n = floors as usize;
            let h = if n >= 3 && i == 0 {
                fh + 0.35
            } else if n >= 4 && i + 1 == n {
                fh - 0.3
            } else if n >= 6 && i + 2 == n {
                fh - 0.1
            } else {
                fh
            };
            let l = (y, q(h));
            y += q(h);
            l
        })
        .collect()
}

fn choose(style: &Style, rng: &mut Rng) -> Choices {
    let wall = parse_material(rng.pick(&style.walls));
    let ground_spec = if style.grounds.is_empty() { style.ground.as_str() } else { rng.pick(&style.grounds).as_str() };
    let ground = match ground_spec {
        "" => Mat::Ashlar([255; 3]),
        "wall" => wall,
        g => parse_material(g),
    };
    let surround = style.surround.get(rng.below(style.surround.len().max(1))).map(|s| parse_material(s)).unwrap_or(Mat::Moulding([246, 242, 234]));
    // Stone mouldings on tiled walls stay white.
    let frame = style.frames.get(rng.below(style.frames.len().max(1))).and_then(|s| parse_rgb(s)).unwrap_or(hex(0xf1eee6));
    let shutter = style.shutters.get(rng.below(style.shutters.len().max(1))).map(|s| parse_shutter(s)).unwrap_or((ShutterKind::Louvre, hex(0x4e3420)));
    let floors = style.floors[0] + rng.below((style.floors[1] - style.floors[0] + 1) as usize) as u32;
    let sk = SurroundKind::parse(&style.surround_kind);
    let sk_principal = if !style.modern && rng.chance(0.75) {
        match sk {
            SurroundKind::Plain => SurroundKind::Lintel,
            SurroundKind::Lintel | SurroundKind::Keystone | SurroundKind::Eared => SurroundKind::Pediment,
            k => k,
        }
    } else {
        sk
    };
    Choices {
        wall,
        ground,
        surround,
        sk,
        sk_principal,
        frame,
        shutter,
        belly: style.railing == "belly",
        floors,
        gh: style.ground_height,
        fh: style.floor_height,
        levels: if style.modern {
            // Even floors with low ceilings: they never line up with the
            // old neighbours'.
            (0..floors).map(|i| (style.ground_height + i as f32 * style.floor_height, style.floor_height)).collect()
        } else {
            floor_levels(style.ground_height, style.floor_height, floors)
        },
        ww: style.window_width,
        wh: style.window_height,
        mirador: rng.chance(style.miradores),
        atico: rng.chance(style.atico),
        party: *rng.pick(&MEDIANERAS),
    }
}

/// Builds one building as its own scene and returns its eave height.
pub fn build(lib: &mut ModuleLib, out: &mut CityOut, lot: &Lot, style: &Style, seed: u64) -> f32 {
    let mut parts: Parts<Mat> = Parts::new();
    let first = out.party_walls.len();
    let h = build_into(lib, out, &mut parts, lot, style, seed);
    medianeras(lib, out, &mut parts, first);
    let scene = assemble(lib.kit, "building", parts, None);
    let si = out.add_scene(scene);
    out.placements.push((si, Mat4::IDENTITY));
    h
}

/// Builds one building, adding its unique geometry (walls, mouldings,
/// roof) to `parts` so a whole block can share one scene, and its
/// instanced modules to `out`. Its party walls are only recorded: call
/// `medianeras` once the neighbours are built too.
pub fn build_into(lib: &mut ModuleLib, out: &mut CityOut, parts: &mut Parts<Mat>, lot: &Lot, style: &Style, seed: u64) -> f32 {
    let mut rng = Rng::new(seed);
    let c = choose(style, &mut rng);
    let base = lot.base;
    let top = c.top(); // facade height above base
    let n = lot.ring.len();
    let court = Mat::Stucco(hex(0xe9e3d6));
    // Portal goes on the longest street edge.
    let portal_edge = (0..n).filter(|&i| lot.edges[i] == EdgeKind::Street).max_by(|&a, &b| {
        let la = (lot.ring[(a + 1) % n] - lot.ring[a]).length();
        let lb = (lot.ring[(b + 1) % n] - lot.ring[b]).length();
        la.total_cmp(&lb)
    });
    let uv_off = rng.range(0.0, 10.0);
    let mut lamps_placed = 0;
    for i in 0..n {
        let (a, b) = (lot.ring[i], lot.ring[(i + 1) % n]);
        let f = edge_frame(a, b, base);
        if f.len < 0.05 {
            continue;
        }
        let kind = lot.edges[i];
        match kind {
            EdgeKind::Party => {
                // Medianera: rises above the roofline a little; built later.
                out.party_walls.push(PartyWall {
                    a,
                    b,
                    base,
                    eave: base + top,
                    mat: c.party,
                    uv_off,
                    floors: c.levels.iter().map(|(y, _)| base + y).collect(),
                    street_at_a: lot.edges[(i + n - 1) % n].is_street(),
                    street_at_b: lot.edges[(i + 1) % n].is_street(),
                    seed: rng.next_u32() as u64,
                });
            }
            EdgeKind::Courtyard => {
                let mut holes = Vec::new();
                let bays = ((f.len - 1.0) / 3.2).floor().max(0.0) as usize;
                for k in 0..bays {
                    let u = 0.5 + (k as f32 + 0.5) * (f.len - 1.0) / bays as f32;
                    for fl in 0..c.floors as usize {
                        let y = c.levels[fl].0 + 0.9;
                        // Washing hung out on the patio side.
                        if rng.chance(0.3 * style.clutter.max(0.3)) {
                            let v = rng.below(6) as u8;
                            let m = lib.named(out, &format!("clothes_rack_{v}"), || modules::clothes_rack(v));
                            out.placements.push((m, facade_frame(f.p(u, y - 0.08, 0.0), f.t, f.n)));
                        }
                        let key = WindowKey {
                            w_cm: 100,
                            h_cm: 140,
                            door: false,
                            surround: c.surround,
                            surround_kind: SurroundKind::None,
                            frame: hex(0xeeeeee),
                            shutter: Some((ShutterKind::Roller, hex(0xe6e0d4), rng.below(4) as u8)),
                            balcony: None,
                            mirador: None,
                            interior: rng.below(4) as u8,
                            light: window_light(&lib.kit.night, f.p((k / 2) as f32 * 7.0, y, -50.0), f.p(u, y, 0.0)),
                            grille: false,
                        };
                        holes.push(Hole { u0: u - 0.5, u1: u + 0.5, v0: y, v1: y + 1.4 });
                        let at = facade_frame(f.p(u, y, 0.0), f.t, f.n);
                        for m in lib.window(out, key) {
                            out.placements.push((m, at));
                        }
                    }
                }
                wall(parts.get(court), court, &f, 0.0, top + 0.9, &holes, uv_off, false);
            }
            EdgeKind::Street | EdgeKind::Corner => {
                street_facade(lib, out, parts, &f, style, &c, &mut rng, Some(i) == portal_edge, kind == EdgeKind::Corner, top, uv_off, &mut lamps_placed);
            }
        }
        // Collider wall.
        out.collider_quad([f.p(0.0, 0.0, 0.0), f.p(f.len, 0.0, 0.0), f.p(f.len, top, 0.0), f.p(0.0, top, 0.0)]);
    }
    out.collider_polygon(&lot.ring, base + top);
    mouldings(lib, out, parts, lot, style, &c, top, &mut rng);
    roof(lib, out, parts, lot, style, &c, top, &mut rng);
    out.buildings += 1;
    base + top
}

#[allow(clippy::too_many_arguments)]
fn street_facade(lib: &mut ModuleLib, out: &mut CityOut, parts: &mut Parts<Mat>, f: &Frame, style: &Style, c: &Choices, rng: &mut Rng, has_portal: bool, corner: bool, top: f32, uv_off: f32, lamps: &mut usize) {
    let margin = 0.55;
    let target = rng.range(style.bay[0], style.bay[1]);
    let bays = if f.len < 2.0 { 0 } else { ((f.len - 2.0 * margin) / target).round().max(1.0) as usize };
    let bay_w = if bays > 0 { (f.len - 2.0 * margin) / bays as f32 } else { 0.0 };
    let ww = c.ww.min(bay_w - 0.75).max(0.7);
    let mut ground_holes = Vec::new();
    let mut upper_holes = Vec::new();
    let portal_bay = bays / 2;
    let center = |k: usize| margin + (k as f32 + 0.5) * bay_w;
    let mut portal_u = None;
    // Ground floor.
    for k in 0..bays {
        let u = center(k);
        if has_portal && k == portal_bay {
            portal_u = Some(u);
            let (w, h) = (1.9f32.min(bay_w - 0.6), (c.gh - 0.55).min(3.7));
            let key = PortalKey { w_cm: (w * 100.0) as u16, h_cm: (h * 100.0) as u16, surround: if c.ground == c.wall { c.surround } else { c.ground }, door: *rng.pick(&[hex(0x4a2e1c), hex(0x2f3d2e), hex(0x5b3a26), hex(0x6b2a22)]) };
            ground_holes.push(Hole { u0: u - w * 0.5, u1: u + w * 0.5, v0: 0.0, v1: h });
            let m = lib.portal(out, key);
            out.placements.push((m, facade_frame(f.p(u, 0.0, 0.0), f.t, f.n)));
        } else if rng.chance(style.shops) && bay_w > 2.0 {
            // Standard shopfront sizes keep the module count small.
            let w = [3.3f32, 3.0, 2.6, 2.2].into_iter().find(|w| *w <= bay_w - 0.6).unwrap_or(2.2);
            let h: f32 = if c.gh >= 4.4 { 3.6 } else { 3.2 };
            let mut sign = rng.below(16) as u8;
            // Lit crosses catch the eye: one pharmacy per neighbourhood.
            let here = f.p(u, 0.0, 0.0);
            if sign == 0 && out.pharmacies.iter().any(|p| p.distance(here) < PHARMACY_SPACING) {
                sign = 2;
            }
            let pharmacy = sign == 0;
            if pharmacy {
                out.pharmacies.push(here);
            }
            let bar = matches!(sign, 1 | 13 | 14);
            // Pharmacies and most bars are open; other shops vary.
            let shutter = if pharmacy || (bar && rng.chance(0.8)) || rng.chance(0.5) { 0 } else { 1 + rng.below(3) as u8 };
            let graffiti = if shutter == 3 && rng.chance(style.graffiti.max(0.05)) { 1 + rng.below(3) as u8 } else { 0 };
            let mut frame = *rng.pick(&[hex(0x1d1d1d), hex(0x3b2a20), hex(0x5a1e1e), hex(0x24402e), hex(0x9aa0a6)]);
            let mut surround = if c.ground == c.wall { c.surround } else { c.ground };
            if bar && rng.chance(0.6) {
                // Old taberna front: the whole frame in painted wood.
                frame = *rng.pick(&[hex(0x6b1d1d), hex(0x24402e), hex(0x3b2a20)]);
                surround = Mat::PaintedWood(frame);
            }
            let key = ShopKey {
                w_cm: (w * 100.0).round() as u16,
                h_cm: (h * 100.0).round() as u16,
                surround,
                sign,
                shutter,
                graffiti,
                frame,
                awning: rng.chance(style.awnings * 2.0).then(|| rng.below(3) as u8),
                interior: 0,
                lit_sign: false,
                neon: 0,
            };
            let (w, h) = (key.w_cm as f32 / 100.0, key.h_cm as f32 / 100.0);
            ground_holes.push(Hole { u0: u - w * 0.5, u1: u + w * 0.5, v0: 0.0, v1: h });
            let at = facade_frame(f.p(u, 0.0, 0.0), f.t, f.n);
            let premises = if pharmacy { Premises::Pharmacy } else if bar { Premises::Bar } else { Premises::Shop };
            let night = lib.kit.night;
            let (key, lit_inside) = night_shop(&night, key, here, premises);
            for m in lib.shop(out, key) {
                out.placements.push((m, at));
            }
            if night.is_night() {
                shop_lights(out, f, &key, u, premises, lit_inside);
            }
            let open = shutter == 0;
            // Projecting signs on the pier beside the shop.
            let pier = u + if k + 1 < bays { bay_w * 0.5 } else { -bay_w * 0.5 };
            if pharmacy {
                let m = lib.named(out, "pharmacy_cross", furniture::pharmacy_cross);
                out.placements.push((m, facade_frame(f.p(pier, h + 0.1, 0.0), f.t, f.n)));
                out.lights.push((f.p(pier, h + 0.1, 0.55), Vec3::new(0.1, 0.9, 0.3)));
                out.city_lights.push(CityLight::point(LightKind::Sign, f.p(pier, h + 0.1, 0.55), Vec3::new(0.1, 1.0, 0.3), 9.0, 6.0).with_size(0.6));
            } else if open && matches!(sign, 1 | 4 | 7 | 12 | 13 | 14) && rng.chance(0.45) {
                let m = if key.lit_sign { lib.named(out, &format!("blade_lit_{sign}"), || furniture::blade_sign_lit(sign, true)) } else { lib.named(out, &format!("blade_{sign}"), || furniture::blade_sign(sign)) };
                out.placements.push((m, facade_frame(f.p(pier, h + 0.05, 0.0), f.t, f.n)));
            }
            // A wine barrel as a standing table in front of the bar window.
            if bar && open && rng.chance(0.55) {
                // Away from the door (centre, left or right, see the shop front).
                let door_x = match sign % 3 {
                    0 => -(w * 0.5 - 0.35),
                    1 => w * 0.5 - 0.4,
                    _ => -(w * 0.5 - 0.4),
                };
                let m = lib.named(out, "barrel", furniture::barrel);
                out.placements.push((m, facade_frame(f.p(u + door_x, 0.0, 0.55), f.t, f.n)));
            }
            out.frontages.push(Frontage { pos: f.p(u, 0.0, 0.0), right: f.t, out: f.n, width: w, bar, closed: !open });
        } else {
            // House window with a grille (or a plain one high up).
            let (w, h, y) = (ww.min(1.1), 1.65, 0.95);
            let key = WindowKey {
                w_cm: (w * 100.0) as u16,
                h_cm: (h * 100.0) as u16,
                door: false,
                surround: c.surround,
                surround_kind: SurroundKind::Plain,
                frame: c.frame,
                shutter: Some((c.shutter.0, c.shutter.1, rng.below(4) as u8)),
                balcony: None,
                mirador: None,
                interior: rng.below(4) as u8,
                light: window_light(&lib.kit.night, f.p(0.0, 0.0, -60.0), f.p(u, y, 0.0)),
                grille: true,
            };
            room_spill(out, &key, f.p(u, y + h * 0.6, 0.5));
            ground_holes.push(Hole { u0: u - w * 0.5, u1: u + w * 0.5, v0: y, v1: y + h });
            let at = facade_frame(f.p(u, y, 0.0), f.t, f.n);
            for m in lib.window(out, key) {
                out.placements.push((m, at));
            }
        }
    }
    // Upper floors.
    let mirador_bay = if c.mirador && bays >= 3 { Some(bays / 2) } else if corner && c.mirador && bays >= 1 { Some(0) } else { None };
    let clutter = style.clutter;
    // Modern blocks: terraces on every bay, or on the middle bays with
    // plain windows at the ends.
    let terraces = style.balcony == "terrace";
    let span = if terraces && bays >= 4 && rng.chance(0.6) { (1, bays - 2) } else { (0, bays.saturating_sub(1)) };
    let terrace = if !terraces {
        0
    } else if matches!(c.wall, Mat::Brick(_)) {
        *rng.pick(&[1u8, 2, 2, 3, 4])
    } else {
        *rng.pick(&[1u8, 1, 3, 4])
    };
    for fl in 0..c.floors as usize {
        let (y, fh) = c.levels[fl];
        let first = fl == 0;
        let last = fl + 1 == c.floors as usize;
        let corrido = match style.balcony.as_str() {
            "noble" => first,
            "corrido" => first || (last && c.floors > 3),
            "terrace" => true,
            _ => false,
        };
        let has_balcony = style.balcony != "none";
        // Each floor is two flats (left and right of the stair); a flat may
        // have swapped its old joinery for white aluminium and roller blinds.
        let reformed = [rng.chance(style.reformed), rng.chance(style.reformed)];
        let alu = *rng.pick(&[hex(0xf1f1ee), hex(0xf1f1ee), hex(0xb9bcbe), hex(0x6b5442)]);
        for k in 0..bays {
            let u = center(k);
            let flat = (u > f.len * 0.5) as usize;
            let mirador = mirador_bay == Some(k) && !last && bays_have_room(bay_w);
            let has_balcony = has_balcony && (!terraces || (k >= span.0 && k <= span.1));
            let door = has_balcony || mirador;
            let (w, h) = if door { (ww, c.door_h(fl)) } else { (ww, 1.55) };
            let wy = if door { y } else { y + 0.9 };
            let principal = first && c.floors >= 3 && !style.modern;
            let balcony = (has_balcony && !mirador).then(|| {
                let (width, ends) = if corrido { (bay_w + 0.01, (k == span.0, k == span.1)) } else if principal { (w + 1.05, (true, true)) } else { (w + 0.75, (true, true)) };
                BalconyKey {
                    width_cm: (width * 10.0).round() as u16 * 10,
                    depth_cm: if terraces { 110 } else if corrido { 70 } else if principal { 60 } else if c.belly { 45 } else { 50 },
                    belly: c.belly && !corrido,
                    ends,
                    pots: 0,
                    awning: ((!corrido || terraces) && rng.chance(style.awnings)).then(|| if terraces { rng.below(3) as u8 } else { 2 }),
                    terrace,
                }
            });
            // Flower pots are their own small instanced module.
            if let Some(b) = &balcony {
                if rng.chance(style.flower_pots) {
                    let k = 1 + rng.below(3) as u8;
                    let key = BalconyKey { pots: k, ..*b };
                    let key = BalconyKey { width_cm: 120, ..key };
                    let m = lib.named(out, &format!("pots_{k}_{}", b.depth_cm), || modules::pots(&key));
                    out.placements.push((m, facade_frame(f.p(u, wy, 0.0), f.t, f.n)));
                }
            }
            let state = rng.below(4) as u8;
            let (frame, shutter) = if reformed[flat] && !mirador { (alu, (ShutterKind::Roller, hex(0xe8e6e0))) } else { (c.frame, c.shutter) };
            let key = WindowKey {
                w_cm: (w * 100.0) as u16,
                h_cm: (h * 100.0) as u16,
                door,
                surround: c.surround,
                surround_kind: c.surround_on(fl),
                frame,
                shutter: if mirador { None } else { Some((shutter.0, shutter.1, state)) },
                balcony,
                mirador: mirador.then(|| ((fh - 0.35) * 100.0) as u16),
                interior: state,
                light: window_light(&lib.kit.night, f.p(flat as f32 * 50.0, y, -40.0), f.p(u, wy, 0.0)),
                grille: false,
            };
            room_spill(out, &key, f.p(u, wy + h * 0.6, 0.6));
            upper_holes.push(Hole { u0: u - w * 0.5, u1: u + w * 0.5, v0: wy, v1: wy + h });
            let at = facade_frame(f.p(u, wy, 0.0), f.t, f.n);
            for m in lib.window(out, key) {
                out.placements.push((m, at));
            }
            if !mirador {
                facade_clutter(lib, out, f, rng, clutter, u, wy, w, balcony.as_ref(), bay_w, k + 1 < bays);
            }
        }
    }
    // Walls: ground floor (optionally rusticated) and upper floors.
    let rustic = style.rustication;
    wall(parts.get(c.ground), c.ground, f, 0.0, c.gh, &ground_holes, uv_off, rustic);
    // Stone plinth (zócalo) at the foot of rendered or brick walls.
    if matches!(c.ground, Mat::Stucco(_) | Mat::Plaster(_) | Mat::Brick(_)) {
        let (pm, ph) = if style.city == "madrid" { (Mat::Granite, 0.85) } else { (Mat::Ashlar(hex(0xcfcabd)), 0.95) };
        plinth(parts.get(pm), pm, f, ph, &ground_holes);
    }
    wall(parts.get(c.wall), c.wall, f, c.gh, top, &upper_holes, uv_off, false);
    services(parts, f, c, style, rng, top, portal_u);
    // Weathering decals: damp plinth, sill streaks, cornice soot.
    let rect = |h: &Hole| [h.u0, h.u1, h.v0, h.v1];
    let (gw, uw): (Vec<[f32; 4]>, Vec<[f32; 4]>) = (ground_holes.iter().map(rect).collect(), upper_holes.iter().map(rect).collect());
    let seed = ((f.origin.x.to_bits() as u64) << 32) ^ f.origin.z.to_bits() as u64;
    crate::weathering::facade(parts, &crate::weathering::FacadeWear { origin: f.origin, t: f.t, n: f.n, len: f.len, ground: c.gh, top, floor_h: c.fh, floors: c.floors, rustic, ground_holes: &gw, upper_holes: &uw, seed });
    // Tile bands under each upper floor and panels between bays.
    if let Some(p) = &style.tile_bands {
        let m = Mat::Tiles(parse_pattern(p));
        for &(y, _) in c.levels.iter().skip(1) {
            band(parts.get(m), m, f, y - 0.45, y - 0.2, 0.0, f.len, &upper_holes);
        }
    }
    if let Some(p) = &style.tile_panels {
        let m = Mat::Tiles(parse_pattern(p));
        for k in 0..bays.saturating_sub(1) {
            let u = margin + (k as f32 + 1.0) * bay_w;
            if bay_w - ww < 1.1 {
                break;
            }
            for &(y, _) in &c.levels {
                band(parts.get(m), m, f, y + 1.0, y + 1.6, u - 0.3, u + 0.3, &[]);
            }
        }
    }
    if let Some(p) = &style.frieze {
        let m = Mat::Tiles(parse_pattern(p));
        band(parts.get(m), m, f, top - 0.5, top - 0.06, 0.0, f.len, &upper_holes);
    }
    // Pilasters between bays.
    if style.pilasters && bays > 1 {
        let s = c.surround.scale();
        for k in 0..=bays {
            let u = margin + k as f32 * bay_w;
            let pw = 0.22;
            let prim = parts.get(c.surround);
            prim.obox(f.p(u, (c.gh + top) * 0.5, 0.04), [f.t, Vec3::Y, f.n], Vec3::new(pw, (top - c.gh) * 0.5, 0.04), s);
        }
    }
    // Wall lanterns on the piers at the top of the ground floor.
    if style.wall_lamps && bays >= 2 && *lamps < 2 {
        let u = margin + bay_w;
        let y = c.gh - 0.3;
        let p = f.p(u, y, 0.0);
        let bulb = p + f.n * 0.68 + Vec3::Y * -0.52;
        let (color, _) = nightlight::street_lamp_color(bulb, style.city == "madrid");
        let lamp = if lib.kit.night.is_night() {
            let g = nightlight::to_rgb(color);
            lib.named(out, &format!("wall_lantern_{g:?}"), || modules::wall_lantern_lit(g))
        } else {
            lib.named(out, "wall_lantern", modules::wall_lantern)
        };
        out.placements.push((lamp, facade_frame(p, f.t, f.n)));
        out.lights.push((p + f.n * 0.68 + Vec3::Y * -0.5, Vec3::new(1.0, 0.78, 0.5)));
        out.city_lights.push(nightlight::wall_lantern(bulb, color));
        *lamps += 1;
    }
}

/// Night light of one window: flats (identified by any point `flat`
/// shared by their windows) are awake with the hour's probability and
/// light most of their rooms in one tone; a few rooms differ, some have
/// the curtains drawn. 0 by day.
pub fn window_light(n: &NightState, flat: Vec3, at: Vec3) -> u8 {
    if !n.is_night() || hash01(flat, 21) >= n.lit_flats() * n.dark.sqrt() {
        return 0;
    }
    if hash01(at, 22) > 0.7 {
        return 0;
    }
    let tone = if hash01(at, 25) < 0.18 { nightlight::pick_room_tone(hash01(at, 26)) } else { nightlight::pick_room_tone(hash01(flat, 23) * 0.9) };
    let curtain = if hash01(at, 24) < 0.3 { modules::LIGHT_CURTAIN } else { 0 };
    1 + tone + curtain
}

/// Faint spill of a lit window onto its balcony and the street.
fn room_spill(out: &mut CityOut, key: &WindowKey, p: Vec3) {
    if key.light == 0 {
        return;
    }
    let (c, level) = nightlight::room_tone((key.light & 7) - 1);
    let dim = if key.light & modules::LIGHT_CURTAIN != 0 { 0.5 } else { 1.0 };
    out.city_lights.push(CityLight::point(LightKind::Window, p, c, 3.0 * level * dim, 4.5).with_size(0.8));
}

/// Shopfront at night: premises that closed since the afternoon pull
/// their shutter down; open ones keep their sign lit and some bars and
/// shops add neon. Returns the key and whether the inside is lit.
fn night_shop(n: &NightState, mut key: ShopKey, at: Vec3, kind: Premises) -> (ShopKey, bool) {
    if !n.is_night() {
        return (key, key.shutter == 0);
    }
    let day_open = key.shutter == 0;
    let open = day_open && hash01(at, 31) < n.open_chance(kind);
    if day_open && !open {
        key.shutter = 3;
        key.graffiti = if hash01(at, 32) < 0.4 { 1 + (hash01(at, 33) * 3.0) as u8 % 3 } else { 0 };
    }
    let h = hash01(at, 34);
    key.lit_sign = if open { h < 0.92 } else { h < 0.3 };
    if open {
        let neon = hash01(at, 35);
        let color = 1 + (hash01(at, 36) * modules::NEON.len() as f32) as u8 % modules::NEON.len() as u8;
        key.neon = match kind {
            Premises::Bar if neon < 0.6 => color + if neon < 0.3 { modules::NEON_FRAME } else { 0 },
            Premises::Shop if neon < 0.15 => color,
            _ => 0,
        };
    }
    // Partly lowered shutters (closing up) keep their lights on.
    let lit = open || (key.shutter > 0 && key.shutter < 3);
    (key, lit)
}

/// Light pouring out of an open shopfront, its neon and its lit sign.
fn shop_lights(out: &mut CityOut, f: &Frame, key: &ShopKey, u: f32, kind: Premises, lit_inside: bool) {
    let (w, h) = (key.w_cm as f32 / 100.0, key.h_cm as f32 / 100.0);
    if lit_inside && key.shutter < 3 {
        let color = match kind {
            Premises::Bar => nightlight::kelvin(2700.0),
            Premises::Pharmacy => nightlight::kelvin(5200.0),
            Premises::Shop => nightlight::kelvin(if hash01(f.p(u, 0.0, 0.0), 37) < 0.5 { 3500.0 } else { 4300.0 }),
        };
        let open = 1.0 - [0.0, 0.3, 0.65, 1.0][key.shutter as usize % 4];
        out.city_lights.push(CityLight::point(LightKind::Shop, f.p(u, (h - 0.55) * 0.6, 0.8), color, 12.0 * open * (w / 3.0), 8.0).with_size(w));
    }
    let sign_at = f.p(u, h - 0.27, 0.35);
    if key.neon & 15 != 0 {
        let c = modules::NEON[((key.neon & 15) as usize - 1) % modules::NEON.len()];
        let lin = |v: u8| crate::noise::srgb_to_linear(v as f32 / 255.0);
        let col = Vec3::new(lin(c[0]), lin(c[1]), lin(c[2]));
        let strength = if key.neon & modules::NEON_FRAME != 0 { 14.0 } else { 7.0 };
        out.city_lights.push(CityLight::point(LightKind::Sign, sign_at, col, strength, 6.0).with_size(w));
    } else if key.lit_sign {
        out.city_lights.push(CityLight::point(LightKind::Sign, sign_at, Vec3::new(1.0, 0.92, 0.82), 3.0, 3.5).with_size(w));
    }
}

fn bays_have_room(bay_w: f32) -> bool {
    bay_w > 2.2
}

/// Lived-in clutter around one upper-floor opening at `u` (bottom `wy`,
/// width `w`): an AC condenser standing on the balcony or bracketed to the
/// wall, a satellite dish, laundry over the railing.
#[allow(clippy::too_many_arguments)]
fn facade_clutter(lib: &mut ModuleLib, out: &mut CityOut, f: &Frame, rng: &mut Rng, clutter: f32, u: f32, wy: f32, w: f32, balcony: Option<&BalconyKey>, bay_w: f32, pier_right: bool) {
    if clutter <= 0.0 {
        return;
    }
    let side = if rng.chance(0.5) { 1.0 } else { -1.0 };
    let at = |du: f32, y: f32, o: f32| facade_frame(f.p(u + du, y, o), f.t, f.n);
    if rng.chance(0.17 * clutter) {
        let variant = rng.below(3) as u8;
        match balcony {
            Some(b) if rng.chance(0.55) => {
                let half = b.width_cm as f32 / 200.0;
                let x = if b.depth_cm >= 70 { (w * 0.5 + 0.45).min(half - 0.42) } else { half - 0.42 };
                let m = lib.named(out, &format!("ac_floor_{variant}"), || modules::ac_unit(false, variant));
                out.placements.push((m, at(side * x, wy + 0.06, 0.04)));
            }
            _ if balcony.is_some() || rng.chance(0.4) => {
                // High on the pier between two bays.
                if bay_w >= 2.6 {
                    let x = if pier_right { bay_w * 0.5 } else { -bay_w * 0.5 };
                    let m = lib.named(out, &format!("ac_wall_{variant}"), || modules::ac_unit(true, variant));
                    out.placements.push((m, at(x, wy + 1.75, 0.0)));
                }
            }
            _ => {
                // Under the sill of a plain window.
                let m = lib.named(out, &format!("ac_wall_{variant}"), || modules::ac_unit(true, variant));
                out.placements.push((m, at(0.0, wy - 0.78, 0.0)));
            }
        }
    }
    if rng.chance(0.05 * clutter) {
        let variant = rng.below(3) as u8;
        let m = lib.named(out, &format!("dish_{variant}"), || modules::dish(variant));
        match balcony {
            // Clamped to the railing post.
            Some(b) if rng.chance(0.5) => out.placements.push((m, at(side * (b.width_cm as f32 / 200.0 - 0.06), wy + 0.75, b.depth_cm as f32 / 100.0 - 0.05))),
            _ => out.placements.push((m, at(side * (w * 0.5 + 0.55), wy + 1.6, 0.0))),
        }
    }
    if let Some(b) = balcony {
        if rng.chance(0.11 * clutter) {
            let variant = rng.below(6) as u8;
            let m = lib.named(out, &format!("laundry_{variant}_{}", b.depth_cm), || modules::laundry(variant, b.depth_cm));
            out.placements.push((m, at(0.0, wy, 0.0)));
        }
    }
}

/// Finishes of medianeras: old exposed brick, grey cement render, render
/// painted once and left to fade.
const MEDIANERAS: [Mat; 6] = [Mat::Brick([240, 222, 210]), Mat::Brick([228, 208, 194]), Mat::Plaster([222, 216, 206]), Mat::Plaster([232, 224, 212]), Mat::Stucco([238, 232, 222]), Mat::Stucco([244, 241, 234])];

/// Builds the party walls recorded since `from`. Where a wall rises above
/// its neighbour's it gets what real exposed medianeras show: a cement
/// flashing where the lower roof met it, brick ventilation shafts climbing
/// to above the roof, and now and then a column of small windows opened
/// later; on top, a coping.
pub fn medianeras(lib: &mut ModuleLib, out: &mut CityOut, parts: &mut Parts<Mat>, from: usize) {
    let walls = out.party_walls[from..].to_vec();
    for w in &walls {
        let f = edge_frame(w.a, w.b, w.base);
        if f.len < 0.05 {
            continue;
        }
        let top = w.eave - w.base + 1.0;
        // The neighbour on the same segment, the other way round.
        let cover = out.party_walls.iter().filter(|o| o.a.distance(w.b) < 0.05 && o.b.distance(w.a) < 0.05).map(|o| o.eave + 1.0 - w.base).fold(0.0f32, f32::max);
        let exposed = top - cover;
        let mut rng = Rng::new(w.seed);
        let mut holes = Vec::new();
        // Distance from the street front -> u along the frame (u runs b -> a).
        let from_street = |d: f32| if w.street_at_b || !w.street_at_a { d } else { f.len - d };
        if exposed > 2.0 && f.len > 5.0 {
            let grey = Mat::Plaster([190, 184, 174]);
            if cover > 0.5 {
                parts.get(grey).obox(f.p(f.len * 0.5, cover + 0.05, 0.05), [f.t, Vec3::Y, f.n], Vec3::new(f.len * 0.5, 0.05, 0.05), 1.0);
            }
            // Brick shafts (shunt flues) from below the old roof line to
            // above the coping, with a capped vent.
            let shafts = if rng.chance(0.65) { 1 + rng.below(2) } else { 0 };
            for k in 0..shafts {
                let d = f.len * rng.range(0.35, 0.85) + k as f32 * 0.9;
                let u = from_street(d.min(f.len - 0.6));
                let (y0, y1) = ((cover - 0.4).max(0.0), top + rng.range(0.4, 1.0));
                let m = if matches!(w.mat, Mat::Brick(_)) { w.mat } else { Mat::Brick([226, 206, 192]) };
                parts.get(m).obox(f.p(u, (y0 + y1) * 0.5, 0.2), [f.t, Vec3::Y, f.n], Vec3::new(0.3, (y1 - y0) * 0.5, 0.2), m.scale());
                parts.get(grey).obox(f.p(u, y1 + 0.22, 0.2), [f.t, Vec3::Y, f.n], Vec3::new(0.38, 0.04, 0.28), 1.0);
                for du in [-0.26, 0.26] {
                    parts.get(m).obox(f.p(u + du, y1 + 0.09, 0.2), [f.t, Vec3::Y, f.n], Vec3::new(0.04, 0.09, 0.2), m.scale());
                }
            }
            // A column of small windows opened later (kitchens, stairs).
            if exposed > 5.0 && rng.chance(0.4) {
                let u = from_street(f.len * rng.range(0.45, 0.8));
                let alu = *rng.pick(&[hex(0xf1f1ee), hex(0xb9bcbe)]);
                let render = Mat::Plaster([232, 226, 214]);
                for &fy in &w.floors {
                    let v = fy - w.base + 1.1;
                    if v < cover + 0.3 || v + 0.9 > top - 1.6 {
                        continue;
                    }
                    let state = rng.below(4) as u8;
                    let key = WindowKey {
                        w_cm: 70,
                        h_cm: 90,
                        door: false,
                        surround: render,
                        surround_kind: SurroundKind::Plain,
                        frame: alu,
                        shutter: Some((ShutterKind::Roller, hex(0xe6e0d4), state)),
                        balcony: None,
                        mirador: None,
                        interior: state,
                        light: window_light(&lib.kit.night, f.p(u, v, 0.0), f.p(u, v, 0.0)),
                        grille: false,
                    };
                    holes.push(Hole { u0: u - 0.35, u1: u + 0.35, v0: v, v1: v + 0.9 });
                    let at = facade_frame(f.p(u, v, 0.0), f.t, f.n);
                    for m in lib.window(out, key) {
                        out.placements.push((m, at));
                    }
                }
            }
        }
        wall(parts.get(w.mat), w.mat, &f, 0.0, top, &holes, w.uv_off, false);
        // Coping on our half of the wall top.
        parts.get(Mat::Plaster([200, 194, 184])).obox(f.p(f.len * 0.5, top + 0.04, -0.15), [f.t, Vec3::Y, f.n], Vec3::new(f.len * 0.5, 0.04, 0.15), 1.0);
    }
}

/// Building services on a street front: a zinc downpipe with its cast-iron
/// boot next to the party wall and the telephone/power cables that run
/// along every old Spanish facade under the first-floor balconies.
fn services(parts: &mut Parts<Mat>, f: &Frame, c: &Choices, style: &Style, rng: &mut Rng, top: f32, portal_u: Option<f32>) {
    let gutter = matches!(style.roof.as_str(), "clay" | "mansard");
    if f.len > 4.0 && (gutter || rng.chance(style.clutter * 0.5)) {
        let u = if rng.chance(0.5) { 0.16 } else { f.len - 0.16 };
        let (r, o) = (0.055, 0.09);
        let p = |y: f32| f.p(u, y, o);
        parts.get(Mat::Zinc).cylinder(p(2.2), p(top + 0.12), r, 8, false);
        parts.get(Mat::Iron).cylinder(p(0.22), p(2.25), r + 0.012, 8, true);
        // Boot turning out over the sidewalk.
        parts.get(Mat::Iron).beam(p(0.3), f.p(u, 0.16, o + 0.14), f.t, 0.11, 0.11, 1.0);
        let mut y = 1.0;
        while y < top {
            parts.get(Mat::Galvanized).obox(f.p(u, y, o * 0.5), [f.t, Vec3::Y, f.n], Vec3::new(0.075, 0.025, o * 0.5 + r), 1.0);
            y += 1.7;
        }
    }
    if style.clutter > 0.0 && f.len > 3.0 {
        // Two cables tucked under the ground-floor cornice.
        let y = c.gh - 0.42;
        let iron = parts.get(Mat::Iron);
        for (dy, o) in [(0.0, 0.03), (-0.06, 0.035)] {
            iron.beam(f.p(0.0, y + dy, o), f.p(f.len, y + dy, o), Vec3::Y, 0.014, 0.014, 1.0);
        }
        let mut u = 0.4;
        while u < f.len {
            iron.obox(f.p(u, y - 0.03, 0.02), [f.t, Vec3::Y, f.n], Vec3::new(0.012, 0.05, 0.02), 1.0);
            u += 0.9;
        }
        // A drop down to the meter box beside the portal.
        if let Some(pu) = portal_u {
            let du = pu + if rng.chance(0.5) { 1.25 } else { -1.25 };
            if du > 0.3 && du < f.len - 0.3 {
                iron.beam(f.p(du, y, 0.03), f.p(du, 2.75, 0.03), f.t, 0.014, 0.014, 1.0);
            }
        }
    }
}

/// Floor bands, ground-floor cornice, main cornice with brackets, quoins.
#[allow(clippy::too_many_arguments)]
fn mouldings(lib: &mut ModuleLib, out: &mut CityOut, parts: &mut Parts<Mat>, lot: &Lot, style: &Style, c: &Choices, top: f32, rng: &mut Rng) {
    let _ = (lib, out, rng);
    let n = lot.ring.len();
    let m = if matches!(c.wall, Mat::Brick(_)) { c.surround } else { Mat::Moulding(hex(0xf4f0e6)) };
    let m = if matches!(c.surround, Mat::Granite) { Mat::Granite } else { m };
    for chain in street_chains(&lot.edges) {
        // Ring points of the chain, in ring order.
        let mut path: Vec<Vec3> = chain.iter().map(|&i| lot.ring[i]).map(|p| Vec3::new(p.x, lot.base, p.y)).collect();
        let last = lot.ring[(chain[chain.len() - 1] + 1) % n];
        path.push(Vec3::new(last.x, lot.base, last.y));
        let normals = polyline_normals(&path, false, 1.0);
        let at = |y: f32| path.iter().map(|p| *p + Vec3::Y * y).collect::<Vec<_>>();
        let s = m.scale();
        if style.modern {
            // Exposed concrete slab edges (frentes de forjado) instead of
            // mouldings, a deeper one over the shops.
            parts.get(m).sweep(&at(c.gh), &normals, &[(0.0, -0.36), (0.07, -0.36), (0.07, 0.0), (0.0, 0.0)], false, true, s);
            for &(y, _) in c.levels.iter().skip(1) {
                parts.get(m).sweep(&at(y), &normals, &[(0.0, -0.24), (0.025, -0.24), (0.025, 0.02), (0.0, 0.02)], false, true, s);
            }
        } else {
            // Ground floor cornice.
            let gprof = [(0.0, -0.32), (0.08, -0.32), (0.08, -0.24), (0.15, -0.17), (0.15, -0.06), (0.21, -0.03), (0.21, 0.04), (0.0, 0.04)];
            parts.get(m).sweep(&at(c.gh), &normals, &gprof, false, true, s);
            // Floor bands; the one above the piso principal is a deeper impost.
            let bprof = [(0.0, -0.2), (0.05, -0.2), (0.05, -0.12), (0.08, -0.09), (0.08, -0.03), (0.0, -0.03)];
            let iprof = [(0.0, -0.3), (0.05, -0.3), (0.05, -0.2), (0.1, -0.16), (0.13, -0.1), (0.13, -0.03), (0.0, -0.03)];
            for (fl, &(y, _)) in c.levels.iter().enumerate().skip(1) {
                let prof: &[(f32, f32)] = if fl == 1 && c.levels.len() >= 3 { &iprof } else { &bprof };
                parts.get(m).sweep(&at(y), &normals, prof, false, true, s);
            }
        }
        // Main cornice.
        let corona = match style.cornice.as_str() {
            "eave" => vec![(0.0, 0.0), (0.08, 0.0), (0.08, 0.12), (0.56, 0.2), (0.62, 0.2), (0.62, 0.3), (0.0, 0.3)],
            "modillions" => vec![(0.0, 0.0), (0.12, 0.0), (0.12, 0.08), (0.18, 0.12), (0.18, 0.3), (0.62, 0.32), (0.62, 0.44), (0.68, 0.47), (0.68, 0.56), (0.0, 0.56)],
            "slab" => vec![(0.0, 0.0), (0.45, 0.0), (0.45, 0.22), (0.0, 0.22)],
            _ => vec![(0.0, 0.0), (0.1, 0.0), (0.1, 0.1), (0.2, 0.17), (0.32, 0.23), (0.46, 0.27), (0.46, 0.38), (0.51, 0.41), (0.51, 0.5), (0.0, 0.5)],
        };
        let cm = if style.cornice == "eave" && matches!(c.wall, Mat::Brick(_)) { c.wall } else { m };
        parts.get(cm).sweep(&at(top), &normals, &corona, false, true, cm.scale());
        // Brackets under the corona / rafters under the eave.
        if matches!(style.cornice.as_str(), "eave" | "modillions") {
            let (bm, h0, h1, out_d, pitch, wdt) = if style.cornice == "eave" { (Mat::Wood(hex(0x6a4a30)), 0.13, 0.19, 0.55, 0.45, 0.07) } else { (m, 0.12, 0.31, 0.58, 0.55, 0.13) };
            let bs = bm.scale();
            for seg in path.windows(2) {
                let (a, b) = (seg[0], seg[1]);
                let len = (b - a).length();
                let dir = (b - a) / len.max(1e-5);
                let nrm = Vec3::new(dir.z, 0.0, -dir.x);
                let cnt = (len / pitch).floor() as i32;
                for k in 0..cnt {
                    let p = a + dir * (pitch * 0.5 + k as f32 * pitch) + Vec3::Y * top;
                    let prim = parts.get(bm);
                    prim.obox(p + nrm * out_d * 0.5 + Vec3::Y * (h0 + h1) * 0.5, [dir, Vec3::Y, nrm], Vec3::new(wdt * 0.5, (h1 - h0) * 0.5, out_d * 0.5), bs);
                    if style.cornice != "eave" {
                        // Volute drop at the wall.
                        prim.obox(p + nrm * 0.1 + Vec3::Y * 0.02, [dir, Vec3::Y, nrm], Vec3::new(wdt * 0.45, 0.1, 0.08), bs);
                    }
                }
            }
        }
        // Granite quoins at the chain ends.
        if style.quoins {
            for (pt, along) in [(path[0], (path[1] - path[0]).normalize()), (path[path.len() - 1], (path[path.len() - 2] - path[path.len() - 1]).normalize())] {
                let nrm = Vec3::new(along.z, 0.0, -along.x) * if pt == path[0] { 1.0 } else { -1.0 };
                let mut y = 0.0;
                let mut k = 0;
                while y < top - 0.4 {
                    let wq = if k % 2 == 0 { 0.6 } else { 0.35 };
                    let prim = parts.get(Mat::Granite);
                    prim.obox(pt + along * wq * 0.5 + Vec3::Y * (y + 0.2) + nrm * 0.02, [along, Vec3::Y, nrm], Vec3::new(wq * 0.5, 0.19, 0.03), 1.5);
                    y += 0.4;
                    k += 1;
                }
            }
        }
    }
}

/// Roof by style, with rooftop clutter.
#[allow(clippy::too_many_arguments)]
fn roof(lib: &mut ModuleLib, out: &mut CityOut, parts: &mut Parts<Mat>, lot: &Lot, style: &Style, c: &Choices, top: f32, rng: &mut Rng) {
    let base = lot.base;
    let y0 = base + top;
    let n = lot.ring.len();
    let moulding = if matches!(c.surround, Mat::Granite) { Mat::Moulding(hex(0xe8e4dc)) } else { Mat::Moulding(hex(0xf6f2e8)) };
    let cornice_top = y0 + match style.cornice.as_str() {
        "eave" => 0.3,
        "modillions" => 0.56,
        "slab" => 0.22,
        _ => 0.5,
    };
    let roof_kind = style.roof.as_str();
    let atico = if c.atico { setback_ring(lot, ATICO_SETBACK) } else { None };
    let aerial_count = |rng: &mut Rng| 1 + rng.below(1 + (style.clutter * 3.0) as usize);
    match roof_kind {
        "clay" if atico.is_none() && plain_street_edge(lot).is_some() => {
            gable_roof(lib, out, parts, lot, style, c, y0 + 0.28, rng);
        }
        "clay" | "mansard" if atico.is_none() => {
            let mat = if roof_kind == "clay" { Mat::RoofClay } else { Mat::RoofSlate };
            let eave = if roof_kind == "clay" { -0.45 } else { 0.1 };
            let outer = inset(&lot.ring, eave);
            // Tiled hips run up as close to a ridge as the plan allows (the
            // short courtyard returns of corner lots limit it), at the
            // pitch of the gabled roofs; mansards keep their steep sides.
            let tries: &[f32] = if roof_kind == "clay" { &[5.8, 5.0, 4.0, 3.0, 2.0] } else { &[0.9, 0.45] };
            let found = tries.iter().find_map(|&d| inset(&lot.ring, d).map(|r| (d, r)));
            if let (Some(outer), Some((slope_in, inner))) = (outer, found) {
                let rise = if roof_kind == "clay" { GABLE_PITCH * (slope_in + 0.45) } else { 2.9 * slope_in / 0.9 };
                let ylo = if roof_kind == "clay" { y0 + 0.28 } else { cornice_top };
                let yhi = ylo + rise;
                roof_slopes(parts, mat, &outer, &inner, ylo, yhi, (roof_kind == "clay").then_some(&lot.ring[..]));
                let top_mat = if roof_kind == "clay" { Mat::RoofTerrace } else { Mat::Zinc };
                parts.get(top_mat).polygon(&inner, yhi, true, top_mat.scale());
                // Dormers along the street edges.
                if style.dormers {
                    for i in 0..n {
                        if lot.edges[i] != EdgeKind::Street {
                            continue;
                        }
                        let f = edge_frame(lot.ring[i], lot.ring[(i + 1) % n], base);
                        if f.len < 5.0 {
                            continue;
                        }
                        let wallm = if roof_kind == "clay" { c.wall } else { Mat::Zinc };
                        let name = format!("dormer_{roof_kind}_{wallm:?}");
                        let d = lib.named(out, &name, || modules::dormer(wallm, mat, c.frame));
                        let cnt = ((f.len - 2.0) / 3.4).floor().max(1.0) as i32;
                        for k in 0..cnt {
                            if roof_kind == "clay" && k % 2 == 1 {
                                continue;
                            }
                            let u = 1.0 + (k as f32 + 0.5) * (f.len - 2.0) / cnt as f32;
                            let setback = if roof_kind == "clay" { 0.6 } else { 0.15 };
                            let yb = ylo + if roof_kind == "clay" { (setback + 0.45) * rise / (slope_in + 0.45) } else { 0.1 };
                            let p = f.p(u, 0.0, -setback);
                            out.placements.push((d, facade_frame(Vec3::new(p.x, yb, p.z), f.t, f.n)));
                        }
                    }
                }
                // Chimneys.
                let cm = if matches!(c.wall, Mat::Brick(_)) { Mat::Brick([255; 3]) } else { Mat::Stucco(hex(0xe6ddcc)) };
                let chim = lib.named(out, &format!("chimney_{cm:?}"), || modules::chimney(cm));
                for _ in 0..rng.below(3) + 1 {
                    let k = rng.below(inner.len());
                    let p = inner[k].lerp(inner[(k + 1) % inner.len()], rng.range(0.2, 0.8));
                    let ctr = inner.iter().copied().sum::<Vec2>() / inner.len() as f32;
                    let q = p.lerp(ctr, 0.3);
                    out.placements.push((chim, Mat4::from_translation(Vec3::new(q.x, yhi - 0.6, q.y))));
                }
                let cnt = aerial_count(rng);
                aerials(lib, out, &inner, yhi, cnt, rng);
            } else {
                parts.get(Mat::RoofTerrace).polygon(&lot.ring, y0, true, 3.0);
            }
        }
        "clay" | "mansard" => {
            // The pitched roof gave way to a terrace in front of the ático.
            roof_terrace(lib, out, parts, lot, c, cornice_top - 0.05, rng);
        }
        _ => {
            // Flat terrace (azotea) with balustrade or solid parapet.
            parts.get(Mat::RoofTerrace).polygon(&lot.ring, cornice_top - 0.05, true, 3.0);
            let balus = roof_kind == "balustrade";
            let bal = lib.named(out, &format!("balustrade_{moulding:?}"), || modules::balustrade(moulding));
            let ped = lib.named(out, &format!("pedestal_{moulding:?}_{}", style.pinnacles), || modules::pedestal(moulding, true));
            let pin = lib.named(out, &format!("pinnacle_{moulding:?}"), || modules::pedestal(moulding, false));
            let mut longest: Option<(f32, usize)> = None;
            for i in 0..n {
                let f = edge_frame(lot.ring[i], lot.ring[(i + 1) % n], base);
                if f.len < 0.3 {
                    continue;
                }
                if lot.edges[i].is_street() && balus {
                    let inset_d = 0.3;
                    let usable = f.len - 0.2;
                    let segs = (usable / 1.0).floor().max(1.0);
                    let sx = usable / segs;
                    for k in 0..segs as i32 {
                        let u = 0.1 + (k as f32 + 0.5) * sx;
                        let p = f.p(u, 0.0, -inset_d);
                        let m = facade_frame(Vec3::new(p.x, cornice_top, p.z), f.t, f.n) * Mat4::from_scale(Vec3::new(sx, 1.0, 1.0));
                        out.placements.push((bal, m));
                    }
                    // Pedestals every few meters, urns or pinnacles on top.
                    let cnt = (f.len / 3.4).round().max(1.0) as i32;
                    for k in 0..=cnt {
                        let u = 0.25 + (f.len - 0.5) * k as f32 / cnt as f32;
                        let p = f.p(u, 0.0, -inset_d);
                        let which = if style.pinnacles { if k % 2 == 0 { ped } else { pin } } else { ped };
                        out.placements.push((which, facade_frame(Vec3::new(p.x, cornice_top, p.z), f.t, f.n)));
                    }
                    if lot.edges[i] == EdgeKind::Street && longest.is_none_or(|(l, _)| f.len > l) {
                        longest = Some((f.len, i));
                    }
                } else {
                    // Solid parapet.
                    let m = if lot.edges[i] == EdgeKind::Party { Mat::Plaster(hex(0xd9d0c2)) } else if lot.edges[i].is_street() { c.wall } else { Mat::Stucco(hex(0xe9e3d6)) };
                    let ph = if lot.edges[i] == EdgeKind::Party { 0.0 } else { 0.95 };
                    if ph > 0.0 {
                        let s = m.scale();
                        let (a, b) = (f.p(0.0, cornice_top - base, -0.1), f.p(f.len, cornice_top - base, -0.1));
                        parts.get(m).obox((a + b) * 0.5 + Vec3::Y * ph * 0.5, [f.t, Vec3::Y, f.n], Vec3::new(f.len * 0.5, ph * 0.5, 0.12), s);
                    }
                }
            }
            if style.crest {
                if let Some((len, i)) = longest {
                    if len > 6.0 {
                        let f = edge_frame(lot.ring[i], lot.ring[(i + 1) % n], base);
                        let tiles = Mat::Tiles(parse_pattern(style.frieze.as_deref().unwrap_or("frieze")));
                        let cr = lib.named(out, &format!("crest_{moulding:?}_{tiles:?}"), || modules::crest(moulding, tiles, 3.6));
                        let p = f.p(f.len * 0.5, 0.0, -0.3);
                        out.placements.push((cr, facade_frame(Vec3::new(p.x, cornice_top + 0.86, p.z), f.t, f.n)));
                    }
                }
            }
            // Terrace clutter (the ático takes the terrace otherwise).
            if atico.is_none() {
                let ctr = lot.ring.iter().copied().sum::<Vec2>() / n as f32;
                for k in 0..rng.below(3) + 1 {
                    let kind = rng.below(4) as u8;
                    let m = lib.named(out, &format!("rooftop_{kind}"), || modules::rooftop(kind, Mat::Stucco(hex(0xece8df))));
                    let off = Vec2::new(rng.range(-2.5, 2.5), rng.range(-2.5, 2.5)) * if k == 0 { 0.3 } else { 1.0 };
                    let p = ctr + off;
                    if crate::geom::signed_area(&lot.ring) > 0.0 && point_in(&lot.ring, p) {
                        out.placements.push((m, Mat4::from_rotation_translation(forja_math::Quat::from_rotation_y(rng.range(0.0, std::f32::consts::TAU)), Vec3::new(p.x, cornice_top - 0.05, p.y))));
                    }
                }
                if let Some(back) = inset(&lot.ring, 1.5) {
                    let cnt = aerial_count(rng);
                    aerials(lib, out, &back, cornice_top - 0.05, cnt, rng);
                }
            }
        }
    }
    if let Some(ring) = &atico {
        let cnt = aerial_count(rng);
        atico_storey(lib, out, parts, lot, ring, style, c, cornice_top - 0.05, cnt, rng);
    }
    corner_tower(lib, out, parts, lot, style, c, cornice_top, rng);
}

/// How far an ático stands back from the street front, and its height.
pub const ATICO_SETBACK: f32 = 2.8;
const ATICO_H: f32 = 2.95;
/// Every aerial in town points at the same transmitter.
const AERIAL_YAW: f32 = 0.85;

/// The lot ring with its street edge pushed back by `d`: the footprint of
/// an ático. Only for plain lots (one street edge between two party walls)
/// deep enough to keep a room behind the terrace.
pub fn setback_ring(lot: &Lot, d: f32) -> Option<Vec<Vec2>> {
    let n = lot.ring.len();
    if n != 4 {
        return None;
    }
    let streets: Vec<usize> = (0..n).filter(|&i| lot.edges[i].is_street()).collect();
    let [i] = streets[..] else {
        return None;
    };
    let (ip, i1, i2) = ((i + n - 1) % n, (i + 1) % n, (i + 2) % n);
    if lot.edges[ip] != EdgeKind::Party || lot.edges[i1] != EdgeKind::Party {
        return None;
    }
    let r = &lot.ring;
    if (r[ip] - r[i]).length() < d + 4.5 || (r[i2] - r[i1]).length() < d + 4.5 || (r[i1] - r[i]).length() < 4.0 {
        return None;
    }
    let mut out = r.clone();
    out[i] = r[i] + (r[ip] - r[i]).normalize() * d;
    out[i1] = r[i1] + (r[i2] - r[i1]).normalize() * d;
    Some(out)
}

/// Sloped roof faces from the eave ring `outer` (at `ylo`) up to `inner`
/// (at `yhi`), with a timber soffit back to the wall ring when given.
fn roof_slopes(parts: &mut Parts<Mat>, mat: Mat, outer: &[Vec2], inner: &[Vec2], ylo: f32, yhi: f32, soffit: Option<&[Vec2]>) {
    let n = outer.len();
    let s = mat.scale();
    for i in 0..n {
        let j = (i + 1) % n;
        let (a, b) = (outer[i], outer[j]);
        let (ai, bi) = (inner[i], inner[j]);
        let p = [Vec3::new(a.x, ylo, a.y), Vec3::new(b.x, ylo, b.y), Vec3::new(bi.x, yhi, bi.y), Vec3::new(ai.x, yhi, ai.y)];
        let up = (p[1] - p[0]).cross(p[3] - p[0]);
        let pts = if up.y >= 0.0 { p } else { [p[1], p[0], p[3], p[2]] };
        let d = (b - a).normalize_or(Vec2::X);
        let uv = |q: Vec3| Vec2::new(Vec2::new(q.x, q.z).dot(d), (Vec2::new(q.x, q.z) - a).perp_dot(d) * 1.2) / s;
        parts.get(mat).quad_uv(pts, [uv(pts[0]), uv(pts[1]), uv(pts[2]), uv(pts[3])]);
        if let Some(wall) = soffit {
            let (r, r2) = (wall[i], wall[j]);
            let q = [Vec3::new(a.x, ylo - 0.02, a.y), Vec3::new(b.x, ylo - 0.02, b.y), Vec3::new(r2.x, ylo - 0.02, r2.y), Vec3::new(r.x, ylo - 0.02, r.y)];
            quad_facing(parts.get(Mat::Wood(hex(0x6a4a30))), q, Vec3::NEG_Y, [Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y]);
        }
    }
}

/// The street edge of a plain lot: one street front, party walls on both
/// sides and the courtyard at the back.
fn plain_street_edge(lot: &Lot) -> Option<usize> {
    let n = lot.ring.len();
    if n != 4 {
        return None;
    }
    let i = (0..n).find(|&i| lot.edges[i] == EdgeKind::Street)?;
    let k = |d: usize| lot.edges[(i + d) % n];
    (k(1) == EdgeKind::Party && k(2) == EdgeKind::Courtyard && k(3) == EdgeKind::Party).then_some(i)
}

/// Roof pitch of the old Madrid tiled roofs (rise over run).
const GABLE_PITCH: f32 = 0.42;

/// Tiled roof in two slopes (a dos aguas) on a plain lot: the ridge runs
/// parallel to the street, both eaves overhang, the gables stand on the
/// party walls; dormers on the street slope, chimneys and aerials on the
/// ridge. `ylo` is the eave height.
#[allow(clippy::too_many_arguments)]
fn gable_roof(lib: &mut ModuleLib, out: &mut CityOut, parts: &mut Parts<Mat>, lot: &Lot, style: &Style, c: &Choices, ylo: f32, rng: &mut Rng) {
    let Some(i) = plain_street_edge(lot) else {
        return;
    };
    let r = |d: usize| lot.ring[(i + d) % 4];
    let (p0, p1, p2, p3) = (r(0), r(1), r(2), r(3));
    let outward = |a: Vec2, b: Vec2| {
        let d = (b - a).normalize_or(Vec2::X);
        Vec2::new(d.y, -d.x)
    };
    let (os, oc) = (outward(p0, p1), outward(p2, p3));
    let (s0, s1) = (p0 + os * 0.45, p1 + os * 0.45);
    let (c0, c1) = (p2 + oc * 0.35, p3 + oc * 0.35);
    let (ma, mb) = ((p0 + p3) * 0.5, (p1 + p2) * 0.5);
    let half = ((p3 - p0).length() + (p2 - p1).length()) * 0.25;
    let rise = (GABLE_PITCH * (half + 0.4)).clamp(1.2, 2.8);
    let yr = ylo + rise;
    let v3 = |q: Vec2, y: f32| Vec3::new(q.x, y, q.y);
    let mat = Mat::RoofClay;
    let s = mat.scale();
    for (a, b, along) in [(s0, s1, p1 - p0), (c0, c1, p3 - p2)] {
        let (ra, rb) = if a == s0 { (mb, ma) } else { (ma, mb) };
        let pts = [v3(a, ylo), v3(b, ylo), v3(ra, yr), v3(rb, yr)];
        let up = (pts[1] - pts[0]).cross(pts[3] - pts[0]);
        let pts = if up.y >= 0.0 { pts } else { [pts[1], pts[0], pts[3], pts[2]] };
        let d = along.normalize_or(Vec2::X);
        let uv = |q: Vec3| Vec2::new(Vec2::new(q.x, q.z).dot(d), (Vec2::new(q.x, q.z) - a).perp_dot(d) * 1.2) / s;
        parts.get(mat).quad_uv(pts, [uv(pts[0]), uv(pts[1]), uv(pts[2]), uv(pts[3])]);
    }
    // Timber soffits under both eaves.
    let wood = Mat::Wood(hex(0x6a4a30));
    quad_facing(parts.get(wood), [v3(s0, ylo - 0.02), v3(s1, ylo - 0.02), v3(p1, ylo - 0.02), v3(p0, ylo - 0.02)], Vec3::NEG_Y, [Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y]);
    quad_facing(parts.get(wood), [v3(c0, ylo - 0.02), v3(c1, ylo - 0.02), v3(p3, ylo - 0.02), v3(p2, ylo - 0.02)], Vec3::NEG_Y, [Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y]);
    // Gables on the party walls, above where the medianera stops.
    let wall_top = c.top() + lot.base + 1.0;
    let t = (wall_top - ylo) / rise;
    if t < 1.0 {
        // The roof's section on the party plane runs eave to eave.
        for (a, b, apex, n) in [(s0, c1, ma, outward(p3, p0)), (c0, s1, mb, outward(p1, p2))] {
            let (qa, qb) = (v3(a, ylo).lerp(v3(apex, yr), t), v3(b, ylo).lerp(v3(apex, yr), t));
            let tip = v3(apex, yr);
            let m = c.party;
            let ms = m.scale();
            let uv = |q: Vec3| Vec2::new(Vec2::new(q.x, q.z).dot(Vec2::new(-n.y, n.x)), -q.y) / ms;
            let prim = parts.get(m);
            let tri = [qa, qb, tip];
            let fwd = (tri[1] - tri[0]).cross(tri[2] - tri[0]).dot(Vec3::new(n.x, 0.0, n.y)) >= 0.0;
            let tri = if fwd { tri } else { [qb, qa, tip] };
            prim.tri(tri[0], tri[1], tri[2], [uv(tri[0]), uv(tri[1]), uv(tri[2])]);
        }
    }
    // Ridge tiles.
    parts.get(Mat::RoofClay).beam(v3(ma, yr + 0.04), v3(mb, yr + 0.04), Vec3::Y, 0.26, 0.12, 1.0);
    // Dormers on the street slope.
    let f = edge_frame(p0, p1, lot.base);
    if style.dormers && f.len >= 5.0 {
        let wallm = c.wall;
        let d = lib.named(out, &format!("dormer_clay_{wallm:?}"), || modules::dormer(wallm, mat, c.frame));
        let cnt = ((f.len - 2.0) / 3.4).floor().max(1.0) as i32;
        for k in (0..cnt).step_by(2) {
            let u = 1.0 + (k as f32 + 0.5) * (f.len - 2.0) / cnt as f32;
            let setback = 0.6;
            let yb = ylo + (setback + 0.45) * rise / (half + 0.45);
            let p = f.p(u, 0.0, -setback);
            out.placements.push((d, facade_frame(Vec3::new(p.x, yb, p.z), f.t, f.n)));
        }
    }
    // Chimneys and aerials along the ridge.
    let cm = if matches!(c.wall, Mat::Brick(_)) { Mat::Brick([255; 3]) } else { Mat::Stucco(hex(0xe6ddcc)) };
    let chim = lib.named(out, &format!("chimney_{cm:?}"), || modules::chimney(cm));
    for _ in 0..rng.below(3) + 1 {
        let q = ma.lerp(mb, rng.range(0.15, 0.85)) + os * rng.range(-0.8, 0.8);
        out.placements.push((chim, Mat4::from_translation(v3(q, yr - 0.7))));
    }
    let count = 1 + rng.below(1 + (style.clutter * 3.0) as usize);
    for _ in 0..count {
        let v = rng.below(3) as u8;
        let m = lib.named(out, &format!("tv_antenna_{v}"), || modules::tv_antenna(v));
        let q = ma.lerp(mb, rng.range(0.1, 0.9));
        let yaw = AERIAL_YAW + rng.range(-0.15, 0.15);
        out.placements.push((m, Mat4::from_rotation_translation(forja_math::Quat::from_rotation_y(yaw), v3(q, yr - 0.05))));
    }
}

/// TV aerials standing on a roof inside `ring` at height `y`.
fn aerials(lib: &mut ModuleLib, out: &mut CityOut, ring: &[Vec2], y: f32, count: usize, rng: &mut Rng) {
    if ring.len() < 3 {
        return;
    }
    let ctr = ring.iter().copied().sum::<Vec2>() / ring.len() as f32;
    for _ in 0..count {
        let v = rng.below(3) as u8;
        let m = lib.named(out, &format!("tv_antenna_{v}"), || modules::tv_antenna(v));
        let k = rng.below(ring.len());
        let p = ctr.lerp(ring[k].lerp(ring[(k + 1) % ring.len()], rng.f()), rng.range(0.1, 0.7));
        let yaw = AERIAL_YAW + rng.range(-0.15, 0.15);
        out.placements.push((m, Mat4::from_rotation_translation(forja_math::Quat::from_rotation_y(yaw), Vec3::new(p.x, y, p.y))));
    }
}

/// Flat roof terrace over the main body in front of an ático: a parapet
/// with an iron railing and planters along the street, a solid parapet on
/// the courtyard side.
fn roof_terrace(lib: &mut ModuleLib, out: &mut CityOut, parts: &mut Parts<Mat>, lot: &Lot, c: &Choices, y: f32, rng: &mut Rng) {
    parts.get(Mat::RoofTerrace).polygon(&lot.ring, y, true, 3.0);
    let n = lot.ring.len();
    let rail = lib.named(out, "terrace_railing", modules::terrace_railing);
    let wallm = if matches!(c.wall, Mat::Tiles(_)) { Mat::Stucco(hex(0xf1ece0)) } else { c.wall };
    let coping = Mat::Moulding(hex(0xe9e4d8));
    for i in 0..n {
        let f = edge_frame(lot.ring[i], lot.ring[(i + 1) % n], 0.0);
        if f.len < 0.3 {
            continue;
        }
        match lot.edges[i] {
            EdgeKind::Street | EdgeKind::Corner => {
                let ph = 0.45;
                parts.get(wallm).obox(f.p(f.len * 0.5, y + ph * 0.5, -0.12), [f.t, Vec3::Y, f.n], Vec3::new(f.len * 0.5, ph * 0.5, 0.12), wallm.scale());
                parts.get(coping).obox(f.p(f.len * 0.5, y + ph + 0.03, -0.12), [f.t, Vec3::Y, f.n], Vec3::new(f.len * 0.5, 0.03, 0.15), 1.0);
                let segs = f.len.floor().max(1.0);
                let sx = f.len / segs;
                for k in 0..segs as i32 {
                    let p = f.p((k as f32 + 0.5) * sx, y + ph + 0.06, -0.12);
                    out.placements.push((rail, facade_frame(p, f.t, f.n) * Mat4::from_scale(Vec3::new(sx, 1.0, 1.0))));
                }
                // Potted shrubs and climbers behind the railing.
                for _ in 0..rng.below(3) {
                    let v = rng.below(3) as u8;
                    let m = lib.named(out, &format!("planter_{v}"), || modules::planter(v));
                    let u = rng.range(0.8, (f.len - 0.8).max(0.9));
                    out.placements.push((m, facade_frame(f.p(u, y, -0.55), f.t, f.n)));
                }
            }
            EdgeKind::Courtyard => {
                let m = Mat::Stucco(hex(0xe9e3d6));
                parts.get(m).obox(f.p(f.len * 0.5, y + 0.5, -0.12), [f.t, Vec3::Y, f.n], Vec3::new(f.len * 0.5, 0.5, 0.12), m.scale());
            }
            EdgeKind::Party => {}
        }
    }
}

/// The ático: a storey of plain render with wide aluminium doors onto the
/// terrace, set back from the street on `ring`, with its own low roof.
#[allow(clippy::too_many_arguments)]
fn atico_storey(lib: &mut ModuleLib, out: &mut CityOut, parts: &mut Parts<Mat>, lot: &Lot, ring: &[Vec2], style: &Style, c: &Choices, y: f32, aerial_count: usize, rng: &mut Rng) {
    let n = ring.len();
    // Often a later addition in plain white or cream render.
    let wallm = match c.wall {
        Mat::Stucco(_) | Mat::Plaster(_) if rng.chance(0.5) => c.wall,
        _ => *rng.pick(&[Mat::Stucco(hex(0xf2efe8)), Mat::Stucco(hex(0xe9e1cf)), Mat::Plaster(hex(0xe6dccb))]),
    };
    let party = c.party;
    let court = Mat::Stucco(hex(0xe9e3d6));
    let alu = *rng.pick(&[hex(0xf1f1ee), hex(0xf1f1ee), hex(0xb9bcbe), hex(0x6b5442)]);
    let h = ATICO_H;
    let uv_off = rng.range(0.0, 10.0);
    for i in 0..n {
        let f = edge_frame(ring[i], ring[(i + 1) % n], y);
        if f.len < 0.05 {
            continue;
        }
        match lot.edges[i] {
            EdgeKind::Street | EdgeKind::Corner => {
                let bays = ((f.len - 1.0) / 3.0).round().max(1.0) as usize;
                let bw = (f.len - 1.0) / bays as f32;
                let w = if bw > 2.6 { 1.6 } else { 1.2 };
                let mut holes = Vec::new();
                for k in 0..bays {
                    let u = 0.5 + (k as f32 + 0.5) * bw;
                    let state = rng.below(4) as u8;
                    let key = WindowKey {
                        w_cm: (w * 100.0) as u16,
                        h_cm: 215,
                        door: true,
                        surround: Mat::Iron,
                        surround_kind: SurroundKind::None,
                        frame: alu,
                        shutter: Some((ShutterKind::Roller, hex(0xe8e6e0), state)),
                        balcony: None,
                        mirador: None,
                        interior: state,
                        light: window_light(&lib.kit.night, f.p(0.0, 0.0, -30.0), f.p(u, 0.0, 0.0)),
                        grille: false,
                    };
                    holes.push(Hole { u0: u - w * 0.5, u1: u + w * 0.5, v0: 0.0, v1: 2.15 });
                    let at = facade_frame(f.p(u, 0.0, 0.0), f.t, f.n);
                    for m in lib.window(out, key) {
                        out.placements.push((m, at));
                    }
                    // An AC condenser on the terrace floor beside a door.
                    if rng.chance(0.3 * style.clutter) {
                        let v = rng.below(3) as u8;
                        let m = lib.named(out, &format!("ac_floor_{v}"), || modules::ac_unit(false, v));
                        let du = if rng.chance(0.5) { w * 0.5 + 0.55 } else { -(w * 0.5 + 0.55) };
                        out.placements.push((m, facade_frame(f.p(u + du, 0.06, 0.0), f.t, f.n)));
                    }
                }
                wall(parts.get(wallm), wallm, &f, 0.0, h, &holes, uv_off, false);
            }
            EdgeKind::Party => {
                // Continues the main medianera above where it stops.
                let v0 = (lot.base + c.top() + 1.0 - y).clamp(0.0, h);
                wall(parts.get(party), party, &f, v0, h + 0.4, &[], uv_off, false);
            }
            EdgeKind::Courtyard => wall(parts.get(court), court, &f, 0.0, h, &[], uv_off, false),
        }
        out.collider_quad([f.p(0.0, 0.0, 0.0), f.p(f.len, 0.0, 0.0), f.p(f.len, h, 0.0), f.p(0.0, h, 0.0)]);
    }
    let yt = y + h;
    out.collider_polygon(ring, yt);
    let pitched = matches!(style.roof.as_str(), "clay" | "mansard");
    let roof_ring = if pitched {
        // Low tiled hip with a short eave, chimney and aerials on the flat.
        let mat = if style.roof == "clay" { Mat::RoofClay } else { Mat::RoofSlate };
        match (inset(ring, -0.35), inset(ring, 1.9)) {
            (Some(outer), Some(inner)) => {
                let yhi = yt + 0.8;
                roof_slopes(parts, mat, &outer, &inner, yt, yhi, Some(ring));
                parts.get(Mat::RoofTerrace).polygon(&inner, yhi, true, 3.0);
                let cm = if matches!(c.wall, Mat::Brick(_)) { Mat::Brick([255; 3]) } else { Mat::Stucco(hex(0xe6ddcc)) };
                let chim = lib.named(out, &format!("chimney_{cm:?}"), || modules::chimney(cm));
                let k = rng.below(inner.len());
                let q = inner[k].lerp(inner[(k + 1) % inner.len()], rng.range(0.3, 0.7));
                out.placements.push((chim, Mat4::from_translation(Vec3::new(q.x, yhi - 0.6, q.y))));
                Some((inner, yhi))
            }
            _ => None,
        }
    } else {
        None
    };
    let (aring, ay) = roof_ring.unwrap_or_else(|| {
        // Flat roof with a moulded coping all round.
        parts.get(Mat::RoofTerrace).polygon(ring, yt + 0.25, true, 3.0);
        let coping = Mat::Moulding(hex(0xeeeae0));
        for i in 0..n {
            let f = edge_frame(ring[i], ring[(i + 1) % n], yt);
            if f.len > 0.05 {
                parts.get(coping).obox(f.p(f.len * 0.5, 0.17, -0.06), [f.t, Vec3::Y, f.n], Vec3::new(f.len * 0.5, 0.17, 0.12), 1.0);
            }
        }
        (inset(ring, 1.0).unwrap_or_else(|| ring.to_vec()), yt + 0.25)
    });
    aerials(lib, out, &aring, ay, aerial_count, rng);
}

pub fn point_in(ring: &[Vec2], p: Vec2) -> bool {
    let mut inside = false;
    let mut j = ring.len() - 1;
    for i in 0..ring.len() {
        let (a, b) = (ring[i], ring[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Domed drum or slate spire over a chamfered/rounded corner.
#[allow(clippy::too_many_arguments)]
fn corner_tower(lib: &mut ModuleLib, out: &mut CityOut, parts: &mut Parts<Mat>, lot: &Lot, style: &Style, c: &Choices, y0: f32, rng: &mut Rng) {
    if style.tower.is_empty() {
        return;
    }
    let n = lot.ring.len();
    let corner: Vec<usize> = (0..n).filter(|&i| lot.edges[i] == EdgeKind::Corner).collect();
    if corner.is_empty() {
        return;
    }
    let a = lot.ring[corner[0]];
    let b = lot.ring[(corner[corner.len() - 1] + 1) % n];
    let chord = (b - a).length();
    if chord < 4.0 {
        return;
    }
    let mid = (a + b) * 0.5;
    let d = (b - a) / chord;
    let inward = Vec2::new(-d.y, d.x);
    let r = (chord * 0.5).min(4.2) * 0.92;
    let ctr2 = mid + inward * r * 0.9;
    let ctr = Vec3::new(ctr2.x, y0 - 0.05, ctr2.y);
    let wall = if matches!(c.wall, Mat::Tiles(_)) { Mat::Stucco(hex(0xf4efe4)) } else { c.wall };
    let moulding = Mat::Moulding(hex(0xf6f2e8));
    if style.tower == "spire" {
        // Square tower with a slate spire (chapitel).
        let h = c.fh * 1.2;
        let half = r * 0.8;
        parts.get(wall).obox(ctr + Vec3::Y * h * 0.5, [Vec3::X, Vec3::Y, Vec3::Z], Vec3::new(half, h * 0.5, half), wall.scale());
        parts.get(moulding).obox(ctr + Vec3::Y * (h + 0.1), [Vec3::X, Vec3::Y, Vec3::Z], Vec3::new(half + 0.15, 0.12, half + 0.15), 1.0);
        let spire = [(half * 1.414 + 0.15, 0.0), (half * 0.9, 1.2), (0.35, 3.6), (0.3, 3.8), (0.45, 4.0), (0.42, 4.5), (0.1, 4.7), (0.05, 6.5), (0.0, 6.6)];
        let mut p = Prim::default();
        p.lathe(Vec3::ZERO, &spire, 4);
        let rot = Mat4::from_translation(ctr + Vec3::Y * (h + 0.22)) * Mat4::from_rotation_y(std::f32::consts::FRAC_PI_4);
        parts.get(Mat::RoofSlate).append(&p, &rot);
        return;
    }
    // Cylindrical drum with windows, cornice ring and dome with lantern.
    let h = c.fh * 1.35;
    parts.get(wall).lathe(ctr, &[(r, 0.0), (r, h)], 28);
    parts.get(moulding).lathe(ctr, &[(r, h - 0.02), (r + 0.12, h + 0.06), (r + 0.3, h + 0.18), (r + 0.34, h + 0.3), (r, h + 0.32)], 28);
    parts.get(moulding).lathe(ctr, &[(r + 0.02, -0.02), (r + 0.1, 0.06), (r + 0.1, 0.2), (r, 0.24)], 28);
    let dome_m = *rng.pick(&[Mat::RoofSlate, Mat::Zinc, Mat::Tiles(crate::texgen::TilePattern::CheckerBlue), moulding]);
    let dome: Vec<(f32, f32)> = (0..=10).map(|i| {
        let a = std::f32::consts::FRAC_PI_2 * i as f32 / 10.0;
        ((r + 0.05) * a.cos().max(0.0) + if i == 10 { 0.0 } else { 0.0 }, h + 0.32 + r * 0.95 * a.sin())
    })
    .collect();
    let mut dome = dome;
    if let Some(last) = dome.last_mut() {
        last.0 = 0.35;
    }
    parts.get(dome_m).lathe(ctr, &dome, 28);
    let lt = h + 0.32 + r * 0.95;
    parts.get(moulding).lathe(ctr, &[(0.45, lt - 0.1), (0.45, lt + 0.9), (0.6, lt + 1.0), (0.6, lt + 1.08), (0.0, lt + 1.1)], 12);
    parts.get(dome_m).lathe(ctr, &[(0.55, lt + 1.08), (0.35, lt + 1.5), (0.06, lt + 1.8), (0.04, lt + 2.6), (0.0, lt + 2.65)], 12);
    // Windows around the outward half of the drum.
    let fwd = Vec3::new(-inward.x, 0.0, -inward.y);
    for k in -2..=2 {
        let ang = k as f32 * 0.5;
        let dir = forja_math::Quat::from_rotation_y(ang) * fwd;
        let p = ctr + dir * r + Vec3::Y * 0.6;
        let right = Vec3::Y.cross(dir);
        let key = WindowKey {
            w_cm: 90,
            h_cm: 190,
            door: false,
            surround: moulding,
            surround_kind: SurroundKind::Keystone,
            frame: c.frame,
            shutter: None,
            balcony: None,
            mirador: None,
            interior: (k + 2) as u8 % 4,
            light: 0,
            grille: false,
        };
        // Dark backing so the window reads on the curved wall.
        parts.get(Mat::Iron).quad(p - right * 0.45 + dir * 0.005, p + right * 0.45 + dir * 0.005, p + right * 0.45 + Vec3::Y * 1.9 + dir * 0.005, p - right * 0.45 + Vec3::Y * 1.9 + dir * 0.005, 1.0);
        for m in lib.window(out, key) {
            out.placements.push((m, facade_frame(p + dir * 0.01, right, dir)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::StyleLibrary;

    fn lot() -> Lot {
        // 18 m street front, 20 m deep, CCW in (x, z).
        Lot {
            ring: vec![Vec2::new(0.0, 0.0), Vec2::new(18.0, 0.0), Vec2::new(18.0, 20.0), Vec2::new(0.0, 20.0)],
            edges: vec![EdgeKind::Courtyard, EdgeKind::Party, EdgeKind::Street, EdgeKind::Party],
            base: 0.15,
        }
    }

    #[test]
    fn ring_orientation_and_frames() {
        let l = lot();
        assert!(signed_area(&l.ring) > 0.0);
        // Edge 2 (z = 20) faces +Z, away from the lot.
        let f = edge_frame(l.ring[2], l.ring[3], 0.0);
        assert!(f.n.z > 0.99, "{:?}", f.n);
        let mid = f.p(f.len * 0.5, 0.0, 1.0);
        assert!(!point_in(&l.ring, Vec2::new(mid.x, mid.z)));
    }

    #[test]
    fn every_style_builds_a_building() {
        let kit = Kit::load(None);
        let styles = StyleLibrary::builtin();
        for (i, s) in styles.styles.iter().enumerate() {
            let mut lib = ModuleLib::new(&kit);
            let mut out = CityOut::default();
            let h = build(&mut lib, &mut out, &lot(), s, i as u64 + 1);
            assert!(h > 6.0, "{}: height {h}", s.id);
            let windows = out.placements.len();
            assert!(windows > 8, "{}: {windows} placements", s.id);
            assert!(!out.col_tris.is_empty());
            // Nothing pokes far out of the lot or under the street.
            for (si, m) in &out.placements {
                let b = out.scenes[*si].bounds();
                let c = m.transform_point3(b.center());
                assert!(c.y > -0.5 && c.x > -4.0 && c.x < 22.0, "{}: module at {c:?}", s.id);
            }
        }
    }

    #[test]
    fn floors_have_a_principal_and_a_lower_top() {
        let l = floor_levels(4.2, 3.2, 5);
        assert_eq!(l.len(), 5);
        assert!((l[0].0 - 4.2).abs() < 1e-5);
        assert!(l[0].1 > l[1].1 && l[1].1 > l[4].1, "{l:?}");
        for w in l.windows(2) {
            assert!((w[0].0 + w[0].1 - w[1].0).abs() < 1e-4, "floors stack: {l:?}");
        }
        // Heights are quantised to 5 cm so window modules repeat.
        for (_, h) in &l {
            assert!(((h * 20.0).round() - h * 20.0).abs() < 1e-3);
        }
        // Low houses keep regular floors.
        let low = floor_levels(3.9, 3.3, 2);
        assert!(low.iter().all(|(_, h)| (*h - 3.3).abs() < 1e-5));
    }

    #[test]
    fn clutter_follows_the_style() {
        let kit = Kit::load(None);
        let styles = StyleLibrary::builtin();
        let count = |id: &str| {
            let mut lib = ModuleLib::new(&kit);
            let mut out = CityOut::default();
            for seed in 0..6 {
                build(&mut lib, &mut out, &lot(), styles.get(id).unwrap(), seed);
            }
            let ids: Vec<usize> = out.scenes.iter().enumerate().filter(|(_, s)| ["ac_", "dish_", "laundry_"].iter().any(|p| s.name.starts_with(p))).map(|(i, _)| i).collect();
            out.placements.iter().filter(|(s, _)| ids.contains(s)).count()
        };
        let lavapies = count("madrid_centro");
        let modernista = count("valencia_modernista");
        assert!(lavapies >= 6, "{lavapies} clutter items in Lavapiés");
        assert!(lavapies > modernista * 2, "{lavapies} vs {modernista}");
    }

    #[test]
    fn window_pieces_are_shared_between_variants() {
        let kit = Kit::load(None);
        let mut lib = ModuleLib::new(&kit);
        let mut out = CityOut::default();
        let k = WindowKey {
            w_cm: 110,
            h_cm: 230,
            door: true,
            surround: Mat::Moulding([240; 3]),
            surround_kind: SurroundKind::Lintel,
            frame: hex(0xf1eee6),
            shutter: Some((ShutterKind::Louvre, hex(0x4e3420), 1)),
            balcony: Some(BalconyKey { width_cm: 190, depth_cm: 50, belly: false, ends: (true, true), pots: 0, awning: None, terrace: 0 }),
            mirador: None,
            interior: 0,
            light: 0,
            grille: false,
        };
        let a = lib.window(&mut out, k);
        let b = lib.window(&mut out, WindowKey { interior: 3, frame: hex(0xffffff), ..k });
        assert_eq!(a.len(), 5);
        // Same surround, shutter and balcony; only glazing and interior differ.
        assert_eq!(a[0], b[0]);
        assert_ne!(a[1], b[1]);
        assert_ne!(a[2], b[2]);
        assert_eq!((a[3], a[4]), (b[3], b[4]));
        assert_eq!(out.scenes.len(), 7);
    }

    #[test]
    fn plinth_stops_at_ground_openings() {
        let f = edge_frame(Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0), 0.0);
        let holes = [Hole { u0: 2.0, u1: 4.0, v0: 0.0, v1: 3.0 }, Hole { u0: 6.0, u1: 7.0, v0: 1.0, v1: 2.5 }];
        let mut p = Prim::default();
        plinth(&mut p, Mat::Granite, &f, 0.9, &holes);
        let us: Vec<f32> = p.positions.iter().map(|q| (*q - f.origin).dot(f.t)).collect();
        assert!(!us.is_empty());
        // The shop door cuts it, the raised window does not.
        assert!(us.iter().all(|u| !(*u > 2.01 && *u < 3.99)), "{us:?}");
        assert!(p.positions.iter().all(|q| q.y <= 0.9 + 1e-4));
        let front: Vec<f32> = p.positions.iter().filter(|q| (**q - f.origin).dot(f.n) > 0.03).map(|q| (*q - f.origin).dot(f.t)).collect();
        assert!(front.iter().any(|u| *u < 6.0) && front.iter().any(|u| *u > 7.0));
    }

    #[test]
    fn shops_record_frontages_and_bars_get_dressed() {
        let kit = Kit::load(None);
        let styles = StyleLibrary::builtin();
        let mut lib = ModuleLib::new(&kit);
        let mut out = CityOut::default();
        for seed in 0..12 {
            build(&mut lib, &mut out, &lot(), styles.get("madrid_centro").unwrap(), seed);
        }
        assert!(out.frontages.len() >= 12, "{} frontages", out.frontages.len());
        assert!(out.frontages.iter().any(|f| f.bar));
        for fr in &out.frontages {
            // On the street edge (z = 20), facing out of the lot.
            assert!((fr.pos.z - 20.0).abs() < 1e-3 && fr.out.z > 0.99, "{fr:?}");
        }
        let named = |p: &str| out.scenes.iter().any(|s| s.name.starts_with(p));
        assert!(named("barrel") || named("blade_") || named("pharmacy_cross"));
    }

    #[test]
    fn atico_stands_back_behind_a_terrace() {
        let l = lot();
        // Street edge is z = 20: the ático front moves to z = 20 - setback.
        let r = setback_ring(&l, ATICO_SETBACK).unwrap();
        assert!((r[2].y - (20.0 - ATICO_SETBACK)).abs() < 1e-4 && (r[3].y - (20.0 - ATICO_SETBACK)).abs() < 1e-4, "{r:?}");
        assert_eq!((r[0], r[1]), (l.ring[0], l.ring[1]));
        // Corner lots and shallow lots keep their roof.
        let shallow = Lot { ring: vec![Vec2::new(0.0, 0.0), Vec2::new(18.0, 0.0), Vec2::new(18.0, 6.0), Vec2::new(0.0, 6.0)], ..lot() };
        assert!(setback_ring(&shallow, ATICO_SETBACK).is_none());
        let kit = Kit::load(None);
        let styles = StyleLibrary::builtin();
        for id in ["madrid_centro", "valencia_ensanche"] {
            let mut style = styles.get(id).unwrap().clone();
            style.atico = 1.0;
            let mut lib = ModuleLib::new(&kit);
            let mut out = CityOut::default();
            let eave = build(&mut lib, &mut out, &l, &style, 3);
            // Its aluminium doors open onto the terrace, above the eave and
            // behind the street front.
            let ids: Vec<usize> = out.scenes.iter().enumerate().filter(|(_, s)| s.name == "window").map(|(i, _)| i).collect();
            let doors = out.placements.iter().filter(|(s, m)| ids.contains(s) && m.w_axis.y > eave && (m.w_axis.z - (20.0 - ATICO_SETBACK)).abs() < 1e-3).count();
            assert!(doors >= 4, "{id}: {doors} ático openings");
            // Its roof is the highest thing that collides.
            let top = out.col_verts.iter().map(|v| v.y).fold(f32::MIN, f32::max);
            assert!(top > eave + 2.5, "{id}: top {top} eave {eave}");
            assert!(out.scenes.iter().any(|s| s.name.starts_with("tv_antenna_")));
            if id == "madrid_centro" {
                assert!(out.scenes.iter().any(|s| s.name == "terrace_railing"));
            }
        }
    }

    #[test]
    fn medianeras_are_dressed_only_where_they_show() {
        let kit = Kit::load(None);
        let styles = StyleLibrary::builtin();
        let low_lot = Lot { ring: vec![Vec2::new(18.0, 0.0), Vec2::new(30.0, 0.0), Vec2::new(30.0, 20.0), Vec2::new(18.0, 20.0)], ..lot() };
        let (mut shafts, mut windows) = (0, 0);
        for seed in 0..16 {
            let mut lib = ModuleLib::new(&kit);
            let mut out = CityOut::default();
            let mut parts = Parts::new();
            build_into(&mut lib, &mut out, &mut parts, &lot(), styles.get("valencia_modernista").unwrap(), seed);
            let low_eave = build_into(&mut lib, &mut out, &mut parts, &low_lot, styles.get("cabanyal").unwrap(), seed + 50);
            assert_eq!(out.party_walls.len(), 4);
            let before = out.placements.len();
            let mut walls = Parts::new();
            medianeras(&mut lib, &mut out, &mut walls, 0);
            // Nothing pokes into the low neighbour below its roof line.
            for p in walls.prims.values() {
                for q in p.positions.iter().filter(|q| q.x > 18.05 && q.x < 29.0) {
                    assert!(q.y > low_eave + 1.0 - 0.45, "seed {seed}: {q:?} below {low_eave}");
                    shafts += 1;
                }
            }
            // Windows opened in the shared wall (the low house's free end at
            // x = 30 is exposed all the way down).
            for (_, m) in out.placements[before..].iter().filter(|(_, m)| (m.w_axis.x - 18.0).abs() < 0.5) {
                let p = m.w_axis.truncate();
                assert!(m.z_axis.x > 0.99 && p.y > low_eave + 1.0, "seed {seed}: window at {p:?}");
                windows += 1;
            }
        }
        assert!(shafts > 0 && windows > 0, "{shafts} shaft vertices, {windows} windows");
    }

    #[test]
    fn modern_blocks_have_even_floors_and_terraces() {
        let styles = StyleLibrary::builtin();
        let s = styles.get("desarrollismo").unwrap();
        for seed in 0..8 {
            let c = choose(s, &mut Rng::new(seed));
            assert!(c.levels.iter().all(|(_, h)| (*h - s.floor_height).abs() < 1e-5), "{:?}", c.levels);
            assert_eq!(c.sk_principal, SurroundKind::None);
        }
        let kit = Kit::load(None);
        let mut lib = ModuleLib::new(&kit);
        let mut out = CityOut::default();
        let eave = build(&mut lib, &mut out, &lot(), s, 4);
        // Taller than the old blocks around it.
        assert!(eave > 20.0, "eave {eave}");
        // Terraces: parapets or railings with galvanised-tube handrails.
        assert!(out.scenes.iter().any(|sc| sc.name == "window" && sc.materials.iter().any(|m| m.name == "galvanized")));
    }

    #[test]
    fn plain_madrid_lots_get_a_gabled_roof() {
        let l = lot();
        assert_eq!(plain_street_edge(&l), Some(2));
        let kit = Kit::load(None);
        let styles = StyleLibrary::builtin();
        let mut style = styles.get("madrid_centro").unwrap().clone();
        style.atico = 0.0;
        let mut lib = ModuleLib::new(&kit);
        let mut out = CityOut::default();
        let mut parts = Parts::new();
        let eave = build_into(&mut lib, &mut out, &mut parts, &l, &style, 9);
        let clay: Vec<Vec3> = parts.prims.get(&Mat::RoofClay).map(|p| p.positions.clone()).unwrap_or_default();
        assert!(!clay.is_empty());
        let top = clay.iter().map(|p| p.y).fold(f32::MIN, f32::max);
        // The ridge runs along the street at mid depth (z = 10), from party
        // wall to party wall, well above the eave; eaves overhang both fronts.
        let ridge: Vec<&Vec3> = clay.iter().filter(|p| p.y > top - 0.2).collect();
        assert!(ridge.iter().all(|p| (p.z - 10.0).abs() < 0.3), "{ridge:?}");
        assert!(ridge.iter().any(|p| p.x < 0.5) && ridge.iter().any(|p| p.x > 17.5));
        assert!(top > eave + 2.0 && top < eave + 3.6, "ridge {top} eave {eave}");
        assert!(clay.iter().any(|p| p.z > 20.3) && clay.iter().any(|p| p.z < -0.3));
        // No flat terrace on top of it.
        assert!(parts.prims.get(&Mat::RoofTerrace).is_none_or(|p| p.positions.is_empty()));
    }

    #[test]
    fn pharmacies_keep_their_distance() {
        let kit = Kit::load(None);
        let styles = StyleLibrary::builtin();
        let mut lib = ModuleLib::new(&kit);
        let mut out = CityOut::default();
        let style = styles.get("madrid_centro").unwrap();
        // A 400 m street of shops.
        for k in 0..24 {
            let x = k as f32 * 18.0;
            let l = Lot { ring: vec![Vec2::new(x, 0.0), Vec2::new(x + 18.0, 0.0), Vec2::new(x + 18.0, 20.0), Vec2::new(x, 20.0)], ..lot() };
            build(&mut lib, &mut out, &l, style, k + 100);
        }
        assert!(!out.pharmacies.is_empty());
        for (i, a) in out.pharmacies.iter().enumerate() {
            for b in &out.pharmacies[i + 1..] {
                assert!(a.distance(*b) >= PHARMACY_SPACING, "{a:?} {b:?}");
            }
        }
        let crosses = out.scenes.iter().position(|s| s.name == "pharmacy_cross");
        let placed = out.placements.iter().filter(|(s, _)| Some(*s) == crosses).count();
        assert_eq!(placed, out.pharmacies.len());
    }

    #[test]
    fn inset_shrinks_convex_rings() {
        let l = lot();
        let r = inset(&l.ring, 2.0).unwrap();
        assert!((signed_area(&r) - 14.0 * 16.0).abs() < 1e-2);
        assert!(inset(&l.ring, 30.0).is_none());
    }
}
