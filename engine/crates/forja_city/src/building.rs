//! Buildings from a lot footprint and a style: walls with real openings
//! and reveals, mouldings swept along the street front, instanced window,
//! balcony, shop and portal modules, and the roof.

use std::collections::HashMap;
use std::sync::Arc;

use forja_assets::SceneData;
use forja_math::{Mat4, Vec2, Vec3};

use crate::geom::{Parts, Prim, Rng, polyline_normals, signed_area};
use crate::kit::{Kit, Mat, Rgb, assemble, hex};
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
    /// `(position, color)` of lamps.
    pub lights: Vec<(Vec3, Vec3)>,
    pub buildings: usize,
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

    /// Instanced pieces of a window (base, shutter, balcony).
    pub fn window(&mut self, out: &mut CityOut, k: WindowKey) -> Vec<usize> {
        let mut ids = Vec::with_capacity(3);
        for piece in [Piece::Base, Piece::Shutter, Piece::Balcony] {
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
        let mut ids = Vec::with_capacity(4);
        for piece in [ShopPiece::Base, ShopPiece::Sign, ShopPiece::Shutter, ShopPiece::Awning] {
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
    frame: Rgb,
    shutter: (ShutterKind, Rgb),
    belly: bool,
    floors: u32,
    gh: f32,
    fh: f32,
    ww: f32,
    wh: f32,
    mirador: bool,
}

fn choose(style: &Style, rng: &mut Rng) -> Choices {
    let wall = parse_material(rng.pick(&style.walls));
    let ground = match style.ground.as_str() {
        "" => Mat::Ashlar([255; 3]),
        "wall" => wall,
        g => parse_material(g),
    };
    let surround = style.surround.get(rng.below(style.surround.len().max(1))).map(|s| parse_material(s)).unwrap_or(Mat::Moulding([246, 242, 234]));
    // Stone mouldings on tiled walls stay white.
    let frame = style.frames.get(rng.below(style.frames.len().max(1))).and_then(|s| parse_rgb(s)).unwrap_or(hex(0xf1eee6));
    let shutter = style.shutters.get(rng.below(style.shutters.len().max(1))).map(|s| parse_shutter(s)).unwrap_or((ShutterKind::Louvre, hex(0x4e3420)));
    let floors = style.floors[0] + rng.below((style.floors[1] - style.floors[0] + 1) as usize) as u32;
    Choices {
        wall,
        ground,
        surround,
        sk: SurroundKind::parse(&style.surround_kind),
        frame,
        shutter,
        belly: style.railing == "belly",
        floors,
        gh: style.ground_height,
        fh: style.floor_height,
        ww: style.window_width,
        wh: style.window_height,
        mirador: rng.chance(style.miradores),
    }
}

/// Builds one building as its own scene and returns its eave height.
pub fn build(lib: &mut ModuleLib, out: &mut CityOut, lot: &Lot, style: &Style, seed: u64) -> f32 {
    let mut parts: Parts<Mat> = Parts::new();
    let h = build_into(lib, out, &mut parts, lot, style, seed);
    let scene = assemble(lib.kit, "building", parts, None);
    let si = out.add_scene(scene);
    out.placements.push((si, Mat4::IDENTITY));
    h
}

/// Builds one building, adding its unique geometry (walls, mouldings,
/// roof) to `parts` so a whole block can share one scene, and its
/// instanced modules to `out`.
pub fn build_into(lib: &mut ModuleLib, out: &mut CityOut, parts: &mut Parts<Mat>, lot: &Lot, style: &Style, seed: u64) -> f32 {
    let mut rng = Rng::new(seed);
    let c = choose(style, &mut rng);
    let base = lot.base;
    let top = c.gh + c.floors as f32 * c.fh; // facade height above base
    let n = lot.ring.len();
    let party = Mat::Plaster(hex(0xd9d0c2));
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
                // Medianera: rises above the roofline a little, plain.
                wall(parts.get(party), party, &f, 0.0, top + 1.0, &[], uv_off, false);
            }
            EdgeKind::Courtyard => {
                let mut holes = Vec::new();
                let bays = ((f.len - 1.0) / 3.2).floor().max(0.0) as usize;
                for k in 0..bays {
                    let u = 0.5 + (k as f32 + 0.5) * (f.len - 1.0) / bays as f32;
                    for fl in 0..c.floors {
                        let y = c.gh + fl as f32 * c.fh + 0.9;
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
    // Ground floor.
    for k in 0..bays {
        let u = center(k);
        if has_portal && k == portal_bay {
            let (w, h) = (1.9f32.min(bay_w - 0.6), (c.gh - 0.55).min(3.7));
            let key = PortalKey { w_cm: (w * 100.0) as u16, h_cm: (h * 100.0) as u16, surround: if c.ground == c.wall { c.surround } else { c.ground }, door: *rng.pick(&[hex(0x4a2e1c), hex(0x2f3d2e), hex(0x5b3a26), hex(0x6b2a22)]) };
            ground_holes.push(Hole { u0: u - w * 0.5, u1: u + w * 0.5, v0: 0.0, v1: h });
            let m = lib.portal(out, key);
            out.placements.push((m, facade_frame(f.p(u, 0.0, 0.0), f.t, f.n)));
        } else if rng.chance(style.shops) && bay_w > 2.0 {
            // Standard shopfront sizes keep the module count small.
            let w = [3.3f32, 3.0, 2.6, 2.2].into_iter().find(|w| *w <= bay_w - 0.6).unwrap_or(2.2);
            let h: f32 = if c.gh >= 4.4 { 3.6 } else { 3.2 };
            let shutter = if rng.chance(0.55) { 0 } else { 1 + rng.below(3) as u8 };
            let graffiti = if shutter == 3 && rng.chance(style.graffiti.max(0.05)) { 1 + rng.below(3) as u8 } else { 0 };
            let key = ShopKey {
                w_cm: (w * 100.0).round() as u16,
                h_cm: (h * 100.0).round() as u16,
                surround: if c.ground == c.wall { c.surround } else { c.ground },
                sign: rng.below(16) as u8,
                shutter,
                graffiti,
                frame: *rng.pick(&[hex(0x1d1d1d), hex(0x3b2a20), hex(0x5a1e1e), hex(0x24402e), hex(0x9aa0a6)]),
                awning: rng.chance(style.awnings * 2.0).then(|| rng.below(3) as u8),
                interior: 0,
            };
            let (w, h) = (key.w_cm as f32 / 100.0, key.h_cm as f32 / 100.0);
            ground_holes.push(Hole { u0: u - w * 0.5, u1: u + w * 0.5, v0: 0.0, v1: h });
            let at = facade_frame(f.p(u, 0.0, 0.0), f.t, f.n);
            for m in lib.shop(out, key) {
                out.placements.push((m, at));
            }
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
                grille: true,
            };
            ground_holes.push(Hole { u0: u - w * 0.5, u1: u + w * 0.5, v0: y, v1: y + h });
            let at = facade_frame(f.p(u, y, 0.0), f.t, f.n);
            for m in lib.window(out, key) {
                out.placements.push((m, at));
            }
        }
    }
    // Upper floors.
    let mirador_bay = if c.mirador && bays >= 3 { Some(bays / 2) } else if corner && c.mirador && bays >= 1 { Some(0) } else { None };
    for fl in 0..c.floors {
        let y = c.gh + fl as f32 * c.fh;
        let first = fl == 0;
        let last = fl + 1 == c.floors;
        let corrido = match style.balcony.as_str() {
            "noble" => first,
            "corrido" => first || (last && c.floors > 3),
            _ => false,
        };
        let has_balcony = style.balcony != "none";
        for k in 0..bays {
            let u = center(k);
            let mirador = mirador_bay == Some(k) && !last && bays_have_room(bay_w);
            let door = has_balcony || mirador;
            let (w, h) = if door { (ww, c.wh.min(c.fh - 0.75)) } else { (ww, 1.55) };
            let wy = if door { y } else { y + 0.9 };
            let balcony = (has_balcony && !mirador).then(|| {
                let (width, ends) = if corrido { (bay_w + 0.01, (k == 0, k + 1 == bays)) } else { (w + 0.75, (true, true)) };
                BalconyKey {
                    width_cm: (width * 10.0).round() as u16 * 10,
                    depth_cm: if corrido { 70 } else if c.belly { 45 } else { 50 },
                    belly: c.belly && !corrido,
                    ends,
                    pots: 0,
                    awning: (!corrido && rng.chance(style.awnings)).then(|| 2),
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
            let shutter = Some((c.shutter.0, c.shutter.1, state));
            let key = WindowKey {
                w_cm: (w * 100.0) as u16,
                h_cm: (h * 100.0) as u16,
                door,
                surround: c.surround,
                surround_kind: c.sk,
                frame: c.frame,
                shutter: if mirador { None } else { shutter },
                balcony,
                mirador: mirador.then(|| ((c.fh - 0.35) * 100.0) as u16),
                interior: state,
                grille: false,
            };
            upper_holes.push(Hole { u0: u - w * 0.5, u1: u + w * 0.5, v0: wy, v1: wy + h });
            let at = facade_frame(f.p(u, wy, 0.0), f.t, f.n);
            for m in lib.window(out, key) {
                out.placements.push((m, at));
            }
        }
    }
    // Walls: ground floor (optionally rusticated) and upper floors.
    let rustic = style.rustication;
    wall(parts.get(c.ground), c.ground, f, 0.0, c.gh, &ground_holes, uv_off, rustic);
    wall(parts.get(c.wall), c.wall, f, c.gh, top, &upper_holes, uv_off, false);
    // Tile bands under each upper floor and panels between bays.
    if let Some(p) = &style.tile_bands {
        let m = Mat::Tiles(parse_pattern(p));
        for fl in 1..c.floors {
            let y = c.gh + fl as f32 * c.fh;
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
            for fl in 0..c.floors {
                let y = c.gh + fl as f32 * c.fh + 1.0;
                band(parts.get(m), m, f, y, y + 0.6, u - 0.3, u + 0.3, &[]);
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
        let lamp = lib.named(out, "wall_lantern", modules::wall_lantern);
        let u = margin + bay_w;
        let y = c.gh - 0.3;
        let p = f.p(u, y, 0.0);
        out.placements.push((lamp, facade_frame(p, f.t, f.n)));
        out.lights.push((p + f.n * 0.68 + Vec3::Y * -0.5, Vec3::new(1.0, 0.78, 0.5)));
        *lamps += 1;
    }
}

fn bays_have_room(bay_w: f32) -> bool {
    bay_w > 2.2
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
        // Ground floor cornice.
        let gprof = [(0.0, -0.32), (0.08, -0.32), (0.08, -0.24), (0.15, -0.17), (0.15, -0.06), (0.21, -0.03), (0.21, 0.04), (0.0, 0.04)];
        parts.get(m).sweep(&at(c.gh), &normals, &gprof, false, true, s);
        // Floor bands.
        let bprof = [(0.0, -0.2), (0.05, -0.2), (0.05, -0.12), (0.08, -0.09), (0.08, -0.03), (0.0, -0.03)];
        for fl in 1..c.floors {
            parts.get(m).sweep(&at(c.gh + fl as f32 * c.fh), &normals, &bprof, false, true, s);
        }
        // Main cornice.
        let corona = match style.cornice.as_str() {
            "eave" => vec![(0.0, 0.0), (0.08, 0.0), (0.08, 0.12), (0.56, 0.2), (0.62, 0.2), (0.62, 0.3), (0.0, 0.3)],
            "modillions" => vec![(0.0, 0.0), (0.12, 0.0), (0.12, 0.08), (0.18, 0.12), (0.18, 0.3), (0.62, 0.32), (0.62, 0.44), (0.68, 0.47), (0.68, 0.56), (0.0, 0.56)],
            _ => vec![(0.0, 0.0), (0.1, 0.0), (0.1, 0.1), (0.2, 0.17), (0.32, 0.23), (0.46, 0.27), (0.46, 0.38), (0.51, 0.41), (0.51, 0.5), (0.0, 0.5)],
        };
        let cm = if style.cornice == "eave" && matches!(c.wall, Mat::Brick(_)) { c.wall } else { m };
        parts.get(cm).sweep(&at(top), &normals, &corona, false, true, cm.scale());
        // Brackets under the corona / rafters under the eave.
        if style.cornice != "classic" {
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
    let cornice_top = y0 + if style.cornice == "eave" { 0.3 } else if style.cornice == "modillions" { 0.56 } else { 0.5 };
    let roof_kind = style.roof.as_str();
    match roof_kind {
        "clay" | "mansard" => {
            let (slope_in, rise, mat) = if roof_kind == "clay" { (4.0, 1.5, Mat::RoofClay) } else { (0.9, 2.9, Mat::RoofSlate) };
            let eave = if roof_kind == "clay" { -0.45 } else { 0.1 };
            let outer = inset(&lot.ring, eave);
            let inner = inset(&lot.ring, slope_in).or_else(|| inset(&lot.ring, slope_in * 0.5));
            if let (Some(outer), Some(inner)) = (outer, inner) {
                let ylo = if roof_kind == "clay" { y0 + 0.28 } else { cornice_top };
                let yhi = ylo + rise * if inner.len() == n { 1.0 } else { 0.5 };
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
                    // Soffit under the clay eave.
                    if roof_kind == "clay" {
                        let r = lot.ring[i];
                        let r2 = lot.ring[j];
                        let q = [Vec3::new(a.x, ylo - 0.02, a.y), Vec3::new(b.x, ylo - 0.02, b.y), Vec3::new(r2.x, ylo - 0.02, r2.y), Vec3::new(r.x, ylo - 0.02, r.y)];
                        quad_facing(parts.get(Mat::Wood(hex(0x6a4a30))), q, Vec3::NEG_Y, [Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y]);
                    }
                }
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
            } else {
                parts.get(Mat::RoofTerrace).polygon(&lot.ring, y0, true, 3.0);
            }
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
                        let (a, b) = (f.p(0.0, top + 0.5, -0.1), f.p(f.len, top + 0.5, -0.1));
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
            // Terrace clutter.
            let ctr = lot.ring.iter().copied().sum::<Vec2>() / n as f32;
            for k in 0..rng.below(3) + 1 {
                let kind = rng.below(4) as u8;
                let m = lib.named(out, &format!("rooftop_{kind}"), || modules::rooftop(kind, Mat::Stucco(hex(0xece8df))));
                let off = Vec2::new(rng.range(-2.5, 2.5), rng.range(-2.5, 2.5)) * if k == 0 { 0.3 } else { 1.0 };
                let p = ctr + off;
                if crate::geom::signed_area(&lot.ring) > 0.0 && point_in(&lot.ring, p) {
                    out.placements.push((m, Mat4::from_rotation_translation(forja_math::Quat::from_rotation_y(rng.range(0.0, 6.28)), Vec3::new(p.x, cornice_top - 0.05, p.y))));
                }
            }
        }
    }
    corner_tower(lib, out, parts, lot, style, c, cornice_top, rng);
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
    fn inset_shrinks_convex_rings() {
        let l = lot();
        let r = inset(&l.ring, 2.0).unwrap();
        assert!((signed_area(&r) - 14.0 * 16.0).abs() < 1e-2);
        assert!(inset(&l.ring, 30.0).is_none());
    }
}
