//! City layout: a Madrid barrio and a Valencian Ensanche on either side of
//! a tree-lined boulevard. Blocks are split into lots for the building
//! generator; streets get raised sidewalks with granite curbs, painted
//! markings, traffic lights, signs, lamps, trees, bollards and bins.

use forja_math::{Mat4, Quat, Vec2, Vec3};

use crate::building::{self, CityOut, EdgeKind, Frontage, Lot, ModuleLib, inset};
use crate::furniture::{self, Aspect};
use crate::geom::{Parts, Rng};
use crate::kit::{Kit, Mat, assemble, hex};
use crate::nightlight::{self, FloodOptions};
use crate::style::StyleLibrary;
use crate::texgen::SignKind;
use crate::traffic::{SignalPost, TrafficInfo, TrafficLane, TrafficSign, TrafficStreet, facing_of_yaw};

/// Library props placed by the caller (trees and benches are scanned
/// models that live outside this crate).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropKind {
    Tree,
    Palm,
    Bench,
}

#[derive(Clone, Copy, Debug)]
pub struct Spawn {
    pub vehicle: bool,
    pub pos: Vec3,
    pub yaw: f32,
}

#[derive(Default)]
pub struct CityLayout {
    pub out: CityOut,
    pub props: Vec<(PropKind, Mat4)>,
    pub spawns: Vec<Spawn>,
    /// Half size of the playable area.
    pub extent: Vec2,
    pub stats: Vec<(String, usize)>,
    /// Street centrelines: `(a, b, width between building lines, district, avenue)`.
    pub streets: Vec<(Vec2, Vec2, f32, usize, bool)>,
    /// Plazas: `(min, max)` corners and district.
    pub plazas: Vec<(Vec2, Vec2, usize)>,
    /// Lanes, signs and traffic lights for the traffic AI.
    pub traffic: TrafficInfo,
}

#[derive(Clone, Debug)]
pub struct District {
    pub name: &'static str,
    pub cols: usize,
    pub rows: usize,
    pub block: [f32; 2],
    /// Street width between building lines (roadway plus sidewalks).
    pub street: f32,
    pub sidewalk: f32,
    pub chamfer: f32,
    /// Chance that a corner is rounded instead of chamfered.
    pub rounded: f32,
    pub depth: f32,
    pub lot_width: [f32; 2],
    pub styles: Vec<(&'static str, f32)>,
    pub avenue_styles: Vec<(&'static str, f32)>,
    pub outer_styles: Vec<(&'static str, f32)>,
    pub plaza_styles: Vec<(&'static str, f32)>,
    pub traffic_lights: bool,
    pub valencia: bool,
    pub sidewalk_mat: Mat,
    /// Block (col, row) turned into a plaza.
    pub plaza: Option<(usize, usize)>,
}

impl District {
    pub fn madrid() -> Self {
        District {
            name: "Madrid",
            cols: 4,
            rows: 5,
            block: [46.0, 62.0],
            street: 11.0,
            sidewalk: 2.0,
            chamfer: 0.0,
            rounded: 0.0,
            depth: 13.0,
            lot_width: [8.0, 14.0],
            styles: vec![("madrid_centro", 0.55), ("madrid_ladrillo", 0.35), ("desarrollismo", 0.1)],
            avenue_styles: vec![("madrid_centro", 0.5), ("valencia_ensanche", 0.5)],
            outer_styles: vec![("madrid_ladrillo", 0.5), ("madrid_centro", 0.35), ("desarrollismo", 0.15)],
            plaza_styles: vec![("madrid_austrias", 1.0)],
            traffic_lights: false,
            valencia: false,
            sidewalk_mat: Mat::SidewalkSlab,
            plaza: Some((1, 2)),
        }
    }

    pub fn valencia() -> Self {
        District {
            name: "Valencia",
            cols: 3,
            rows: 3,
            block: [92.0, 104.0],
            street: 20.0,
            sidewalk: 4.5,
            chamfer: 9.0,
            rounded: 0.35,
            depth: 19.0,
            lot_width: [13.0, 21.0],
            styles: vec![("valencia_ensanche", 0.4), ("valencia_ruzafa", 0.32), ("valencia_modernista", 0.16), ("desarrollismo", 0.12)],
            avenue_styles: vec![("valencia_modernista", 0.55), ("valencia_ensanche", 0.45)],
            outer_styles: vec![("cabanyal", 0.65), ("valencia_ruzafa", 0.23), ("desarrollismo", 0.12)],
            plaza_styles: vec![("valencia_modernista", 0.6), ("valencia_ensanche", 0.4)],
            traffic_lights: true,
            valencia: true,
            sidewalk_mat: Mat::SidewalkHex,
            plaza: Some((1, 1)),
        }
    }
}

#[derive(Clone, Debug)]
pub struct CityConfig {
    pub seed: u64,
    pub west: District,
    pub east: District,
    /// Boulevard width between the districts' building lines.
    pub avenue: f32,
}

impl Default for CityConfig {
    fn default() -> Self {
        Self { seed: 7, west: District::madrid(), east: District::valencia(), avenue: 46.0 }
    }
}

impl CityConfig {
    /// A small city for tests and quick previews.
    pub fn small() -> Self {
        let mut c = Self::default();
        c.west.cols = 2;
        c.west.rows = 2;
        c.west.plaza = None;
        c.east.cols = 1;
        c.east.rows = 1;
        c.east.plaza = None;
        c
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Facing {
    Street,
    Avenue,
    Outer,
    Plaza,
}

#[derive(Clone, Copy, Debug)]
enum Corner {
    Sharp,
    Chamfer,
    Round,
}

struct Block {
    min: Vec2,
    max: Vec2,
    corners: [Corner; 4],
    facing: [Facing; 4],
    plaza: bool,
}

/// A street piece between two intersections, along its centreline.
#[derive(Clone, Copy, Debug)]
struct Segment {
    a: Vec2,
    b: Vec2,
    /// Distance between the building lines.
    width: f32,
    sidewalk: f32,
    /// +1: traffic a→b only, -1: b→a only, 0: both ways.
    oneway: i8,
    avenue: bool,
    district: usize,
}

fn pick<'a>(rng: &mut Rng, w: &'a [(&'static str, f32)]) -> &'a str {
    let total: f32 = w.iter().map(|x| x.1).sum();
    let mut r = rng.f() * total;
    for (s, x) in w {
        if r < *x {
            return s;
        }
        r -= x;
    }
    w[0].0
}

/// Builds the whole city.
pub fn build_city(kit: &Kit, styles: &StyleLibrary, cfg: &CityConfig) -> CityLayout {
    let mut rng = Rng::new(cfg.seed);
    let mut lay = CityLayout::default();
    let mut lib = ModuleLib::new(kit);
    let half_av = cfg.avenue * 0.5;
    let mut blocks: Vec<(usize, Block)> = Vec::new();
    let mut segments: Vec<Segment> = Vec::new();
    let mut zmax: f32 = 0.0;
    let mut xext = [0.0f32; 2];
    for (di, d) in [&cfg.west, &cfg.east].into_iter().enumerate() {
        let sign = if di == 0 { -1.0 } else { 1.0 };
        // Column intervals moving away from the avenue.
        let mut xs = Vec::new();
        let mut cur = half_av;
        for _ in 0..d.cols {
            let w = rng.range(d.block[0], d.block[1]);
            xs.push((cur, cur + w));
            cur += w + d.street;
        }
        xext[di] = cur;
        let heights: Vec<f32> = (0..d.rows).map(|_| rng.range(d.block[0], d.block[1])).collect();
        let total_z = heights.iter().sum::<f32>() + d.street * (d.rows + 1) as f32;
        zmax = zmax.max(total_z * 0.5);
        let mut zs = Vec::new();
        let mut z = -total_z * 0.5 + d.street;
        for h in &heights {
            zs.push((z, z + h));
            z += h + d.street;
        }
        let to_x = |a: f32, b: f32| if sign > 0.0 { (a, b) } else { (-b, -a) };
        for (ci, (xa, xb)) in xs.iter().enumerate() {
            for (ri, (za, zb)) in zs.iter().enumerate() {
                let (x0, x1) = to_x(*xa, *xb);
                let near_avenue = ci == 0;
                let far = ci + 1 == d.cols;
                let is_plaza = d.plaza == Some((ci, ri));
                // Sides: 0 bottom (-z), 1 right (+x), 2 top (+z), 3 left (-x).
                let mut facing = [Facing::Street; 4];
                let (avenue_side, far_side) = if sign > 0.0 { (3, 1) } else { (1, 3) };
                if near_avenue {
                    facing[avenue_side] = Facing::Avenue;
                }
                if far {
                    facing[far_side] = Facing::Outer;
                }
                if ri == 0 {
                    facing[0] = Facing::Outer;
                }
                if ri + 1 == d.rows {
                    facing[2] = Facing::Outer;
                }
                // Sides facing the plaza.
                if let Some((pc, pr)) = d.plaza {
                    if !is_plaza {
                        let (dc, dr) = (ci as i32 - pc as i32, ri as i32 - pr as i32);
                        if dr == 0 && dc.abs() == 1 {
                            let toward_plaza_x = -(dc as f32) * sign;
                            facing[if toward_plaza_x > 0.0 { 1 } else { 3 }] = Facing::Plaza;
                        }
                        if dc == 0 && dr.abs() == 1 {
                            facing[if dr > 0 { 0 } else { 2 }] = Facing::Plaza;
                        }
                    }
                }
                let corners = [0, 1, 2, 3].map(|_| {
                    if d.chamfer <= 0.0 {
                        Corner::Sharp
                    } else if rng.chance(d.rounded) {
                        Corner::Round
                    } else {
                        Corner::Chamfer
                    }
                });
                let b = Block { min: Vec2::new(x0, *za), max: Vec2::new(x1, *zb), corners, facing, plaza: is_plaza };
                if is_plaza {
                    lay.plazas.push((b.min, b.max, di));
                    plaza(&mut lib, &mut lay, &b, d, &mut rng);
                    blocks.push((di, Block { corners: [Corner::Sharp; 4], ..b }));
                    continue;
                }
                blocks.push((di, b));
            }
        }
        // Street segments: vertical streets (along z) between columns and at
        // the far edge, horizontal ones between rows and at both ends.
        let mut vx: Vec<f32> = xs.windows(2).map(|w| (w[0].1 + w[1].0) * 0.5).collect();
        vx.push(xs[xs.len() - 1].1 + d.street * 0.5);
        for (k, x) in vx.iter().enumerate() {
            for (ri, (za, zb)) in zs.iter().enumerate() {
                let xc = x * sign;
                let ow = if k % 2 == 0 { 1 } else { -1 };
                let _ = ri;
                segments.push(Segment { a: Vec2::new(xc, *za), b: Vec2::new(xc, *zb), width: d.street, sidewalk: d.sidewalk, oneway: ow, avenue: false, district: di });
            }
        }
        let mut hz: Vec<f32> = vec![zs[0].0 - d.street * 0.5];
        hz.extend(zs.windows(2).map(|w| (w[0].1 + w[1].0) * 0.5));
        hz.push(zs[zs.len() - 1].1 + d.street * 0.5);
        for (k, z) in hz.iter().enumerate() {
            for (xa, xb) in &xs {
                let (x0, x1) = to_x(*xa, *xb);
                let ow = if k % 2 == 0 { 1 } else { -1 };
                segments.push(Segment { a: Vec2::new(x0, *z), b: Vec2::new(x1, *z), width: d.street, sidewalk: d.sidewalk, oneway: ow, avenue: false, district: di });
            }
        }
    }
    // The boulevard.
    segments.push(Segment { a: Vec2::new(0.0, -zmax), b: Vec2::new(0.0, zmax), width: cfg.avenue, sidewalk: 6.0, oneway: 0, avenue: true, district: 1 });

    // Ground, sidewalks and curbs.
    let ext = Vec2::new(xext[0].max(xext[1]) + 10.0, zmax + 10.0);
    lay.extent = ext;
    let mut ground: Parts<Mat> = Parts::new();
    ground.get(Mat::Asphalt).polygon(&[Vec2::new(-ext.x, -ext.y), Vec2::new(ext.x, -ext.y), Vec2::new(ext.x, ext.y), Vec2::new(-ext.x, ext.y)], 0.0, true, Mat::Asphalt.scale());
    // Avenue median: palms or plane trees, a bike lane, benches; open
    // where signalled side streets cross it, so traffic can turn across.
    let openings: Vec<(f32, f32)> = segments
        .iter()
        .filter(|s| !s.avenue && (if s.district == 0 { &cfg.west } else { &cfg.east }).traffic_lights && (s.a.y - s.b.y).abs() < 0.1)
        .filter(|s| (s.a.x.abs() - half_av).abs() < 0.5 || (s.b.x.abs() - half_av).abs() < 0.5)
        .map(|s| (s.a.y - s.width * 0.5, s.a.y + s.width * 0.5))
        .collect();
    lay.traffic.median_openings = openings.iter().map(|o| Vec2::new(0.0, (o.0 + o.1) * 0.5)).collect();
    let in_gap = |z: f32, margin: f32| openings.iter().any(|o| z > o.0 - margin && z < o.1 + margin);
    {
        let (mw, z0, z1) = (9.0, -zmax + 2.0, zmax - 2.0);
        let mut cuts = vec![z0];
        let mut sorted = openings.clone();
        sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (a, b) in &sorted {
            cuts.push(a.max(z0));
            cuts.push(b.min(z1));
        }
        cuts.push(z1);
        for piece in cuts.chunks(2) {
            let (a, b) = (piece[0], piece[1]);
            if b - a < 6.0 {
                continue;
            }
            let ring = [Vec2::new(-mw * 0.5, a), Vec2::new(mw * 0.5, a), Vec2::new(mw * 0.5, b), Vec2::new(-mw * 0.5, b)];
            raised(&mut ground, &mut lay.out, &ring, Mat::SidewalkHex);
            ground.get(Mat::Grass).polygon(&[Vec2::new(-1.8, a + 2.0), Vec2::new(1.8, a + 2.0), Vec2::new(1.8, b - 2.0), Vec2::new(-1.8, b - 2.0)], 0.165, true, Mat::Grass.scale());
            ground.get(Mat::Paint(hex(0xa23b2c))).polygon(&[Vec2::new(2.2, a + 1.0), Vec2::new(4.2, a + 1.0), Vec2::new(4.2, b - 1.0), Vec2::new(2.2, b - 1.0)], 0.158, true, 2.0);
        }
        let mut z = z0 + 6.0;
        while z < z1 - 4.0 {
            if !in_gap(z, 3.0) {
                lay.props.push((PropKind::Palm, Mat4::from_rotation_translation(Quat::from_rotation_y(z * 0.37), Vec3::new(0.0, 0.17, z))));
                if (z as i32 / 9) % 3 == 0 && !in_gap(z + 4.0, 2.0) {
                    lay.props.push((PropKind::Bench, Mat4::from_rotation_translation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2), Vec3::new(-3.2, 0.15, z + 4.0))));
                }
            }
            z += 11.0;
        }
        // Modern LED road lights (3000 K) on the median.
        let led = nightlight::kelvin(3000.0);
        let lamp = if kit.night.is_night() {
            let g = nightlight::to_rgb(led);
            lib.named(&mut lay.out, "road_light_lit", || furniture::road_light_lit(g))
        } else {
            lib.named(&mut lay.out, "road_light", furniture::road_light)
        };
        let mut z = z0 + 10.0;
        while z < z1 - 5.0 {
            if in_gap(z, 2.0) {
                z += 30.0;
                continue;
            }
            for side in [-1.0f32, 1.0] {
                let yaw = if side < 0.0 { -std::f32::consts::FRAC_PI_2 } else { std::f32::consts::FRAC_PI_2 };
                let m = Mat4::from_rotation_translation(Quat::from_rotation_y(yaw), Vec3::new(side * 0.9, 0.15, z));
                lay.out.placements.push((lamp, m));
                lay.out.lights.push((Vec3::new(side * 2.7, 9.4, z), Vec3::new(1.0, 0.92, 0.8)));
                lay.out.city_lights.push(nightlight::road_light(m.transform_point3(furniture::ROAD_LIGHT_HEAD) - Vec3::Y * 0.05, led));
            }
            z += 30.0;
        }
    }
    for (di, b) in &blocks {
        let d = if *di == 0 { &cfg.west } else { &cfg.east };
        let ring = block_ring(b, d.chamfer).0;
        if b.plaza {
            continue;
        }
        let walk = inset(&ring, -d.sidewalk).unwrap_or(ring.clone());
        raised(&mut ground, &mut lay.out, &walk, d.sidewalk_mat);
    }
    let gi = lay.out.add_scene(assemble(kit, "streets", ground, None));
    lay.out.placements.push((gi, Mat4::IDENTITY));

    // Buildings.
    let mut lots = 0;
    for (di, b) in &blocks {
        let d = if *di == 0 { &cfg.west } else { &cfg.east };
        if b.plaza {
            continue;
        }
        // One scene per block for all the buildings' unique geometry.
        let mut parts: Parts<Mat> = Parts::new();
        let first_wall = lay.out.party_walls.len();
        for (lot, facing) in split_block(b, d, &mut rng) {
            let pool = match facing {
                Facing::Avenue => &d.avenue_styles,
                Facing::Outer => &d.outer_styles,
                Facing::Plaza => &d.plaza_styles,
                Facing::Street => &d.styles,
            };
            let id = pick(&mut rng, pool);
            if let Some(style) = styles.get(id) {
                let first = lay.out.frontages.len();
                let top = building::build_into(&mut lib, &mut lay.out, &mut parts, &lot, style, rng.next_u32() as u64);
                lots += 1;
                // Civic and landmark fronts on the plaza are floodlit.
                if facing == Facing::Plaza && kit.night.is_night() {
                    floodlight_lot(&mut lay.out, &lot, top, b);
                }
                // Bars spill onto sidewalks wide enough for a terrace.
                let fronts: Vec<Frontage> = lay.out.frontages[first..].to_vec();
                for fr in fronts.iter().filter(|fr| fr.bar && !fr.closed) {
                    if d.sidewalk >= 3.5 && rng.chance(0.85) {
                        terrace(&mut lib, &mut lay, fr, d.sidewalk, &mut rng);
                    }
                }
            }
        }
        // Medianeras once every neighbour in the block is known.
        building::medianeras(&mut lib, &mut lay.out, &mut parts, first_wall);
        let si = lay.out.add_scene(assemble(kit, "block", parts, None));
        lay.out.placements.push((si, Mat4::IDENTITY));
    }
    // Markings, signals and furniture.
    let mut marks: Parts<Mat> = Parts::new();
    for s in &segments {
        let d = if s.district == 0 { &cfg.west } else { &cfg.east };
        street_markings(&mut marks, s);
        street_furniture(&mut lib, &mut lay, s, d, &mut rng);
    }
    intersections(&mut lib, &mut lay, &segments, cfg);
    let mi = lay.out.add_scene(assemble(kit, "markings", marks, None));
    lay.out.placements.push((mi, Mat4::IDENTITY));
    // Spawns: the player on the boulevard sidewalk, cars parked in the
    // parking strips (the lanes belong to the traffic).
    lay.spawns.push(Spawn { vehicle: false, pos: Vec3::new(-half_av + 3.0, 1.2, 0.0), yaw: 0.0 });
    for s in segments.iter().filter(|s| !s.avenue).step_by(3) {
        let dir = (s.b - s.a).normalize();
        let mid = (s.a + s.b) * 0.5;
        let flip = if s.oneway < 0 { -1.0 } else { 1.0 };
        let fwd = dir * flip;
        let Some(&offset) = traffic_street(s).parking.iter().max_by(|a, b| (**a * -flip).total_cmp(&(**b * -flip))) else { continue };
        let p = mid + Vec2::new(-dir.y, dir.x) * offset;
        lay.spawns.push(Spawn { vehicle: true, pos: Vec3::new(p.x, 0.8, p.y), yaw: fwd.x.atan2(fwd.y) });
    }
    for side in [-1.0f32, 1.0] {
        let mut z = -zmax + 30.0;
        while z < zmax - 30.0 {
            let x = side * (4.5 + 5.5);
            let yaw = if side > 0.0 { 0.0 } else { std::f32::consts::PI };
            lay.spawns.push(Spawn { vehicle: true, pos: Vec3::new(x, 0.8, z), yaw });
            z += 45.0;
        }
    }
    lay.streets = segments.iter().map(|s| (s.a, s.b, s.width, s.district, s.avenue)).collect();
    lay.traffic.streets = segments.iter().map(traffic_street).collect();
    lay.traffic.plazas = lay.plazas.iter().map(|p| (p.0, p.1)).collect();
    lay.traffic.benches = lay.props.iter().filter(|p| p.0 == PropKind::Bench).map(|(_, m)| {
        let (_, q, t) = m.to_scale_rotation_translation();
        let f = q * Vec3::Z;
        (Vec2::new(t.x, t.z), f.x.atan2(f.z))
    }).collect();
    lay.stats.push(("buildings".into(), lay.out.buildings));
    lay.stats.push(("lots".into(), lots));
    lay.stats.push(("streets".into(), segments.len()));
    lay.stats.push(("modules".into(), lay.out.scenes.len()));
    lay
}

/// A street lamp module (candelabra or fernandina) placed at `base`, with
/// its night light recorded: by day the shared module, at night one tinted
/// with the lamp's sodium or LED color.
fn street_lamp_module(lib: &mut ModuleLib, out: &mut CityOut, candelabra: bool, base: Vec3, _y: f32) -> usize {
    let bulb = base + Vec3::Y * furniture::street_lamp_bulb(candelabra);
    let (color, _) = nightlight::street_lamp_color(bulb, !candelabra);
    out.city_lights.push(nightlight::street_lamp(bulb, color, candelabra));
    if lib.kit.night.is_night() {
        let g = nightlight::to_rgb(color);
        lib.named(out, &format!("street_lamp_{candelabra}_{g:?}"), || furniture::street_lamp_lit(candelabra, g))
    } else {
        lib.named(out, &format!("street_lamp_{candelabra}"), || furniture::street_lamp(candelabra))
    }
}

/// Floodlights for the street fronts of a lot facing a plaza (`top`: the
/// absolute eave height), on the side toward the plaza block `b`.
fn floodlight_lot(out: &mut CityOut, lot: &Lot, top: f32, b: &Block) {
    let _ = b;
    let n = lot.ring.len();
    let opts = FloodOptions::default();
    for i in 0..n {
        if !lot.edges[i].is_street() {
            continue;
        }
        let (a, bb) = (lot.ring[i], lot.ring[(i + 1) % n]);
        let d = bb - a;
        if d.length() < 3.0 {
            continue;
        }
        let outn = Vec3::new(d.y, 0.0, -d.x).normalize_or(Vec3::Z);
        let (pa, pb) = (Vec3::new(a.x, lot.base, a.y), Vec3::new(bb.x, lot.base, bb.y));
        out.city_lights.extend(nightlight::facade_floodlights(pa, pb, outn, top - lot.base, &opts));
    }
}

/// Table sets (some under parasols) in front of an open bar, between the
/// pedestrian strip along the facade and the trees at the kerb.
fn terrace(lib: &mut ModuleLib, lay: &mut CityLayout, fr: &Frontage, sidewalk: f32, rng: &mut Rng) {
    let parasols = rng.chance(0.5);
    let pitch = if parasols { 2.1 } else { 1.75 };
    let n = ((fr.width + 1.2) / pitch).floor().clamp(1.0, 3.0) as i32;
    let dist = (sidewalk * 0.52).clamp(1.6, 2.5);
    let yaw0 = fr.out.x.atan2(fr.out.z);
    for k in 0..n {
        let off = (k as f32 - (n - 1) as f32 * 0.5) * pitch;
        let v = rng.below(3) as u8 * 2 + parasols as u8;
        let m = lib.named(&mut lay.out, &format!("terrace_{v}"), || furniture::terrace_set(v));
        let p = fr.pos + fr.right * off + fr.out * dist;
        lay.out.placements.push((m, Mat4::from_rotation_translation(Quat::from_rotation_y(yaw0 + rng.range(-0.25, 0.25)), p)));
    }
}

/// Raised paved area with a granite curb, plus its collider.
fn raised(parts: &mut Parts<Mat>, out: &mut CityOut, ring: &[Vec2], top: Mat) {
    let h = 0.15;
    parts.get(top).polygon(ring, h, true, top.scale());
    out.collider_polygon(ring, h);
    let inner = inset(ring, 0.3);
    let n = ring.len();
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        let (pa, pb) = (Vec3::new(a.x, 0.0, a.y), Vec3::new(b.x, 0.0, b.y));
        // Outward-facing curb face (ring is CCW, so outward is to the right).
        parts.get(Mat::Curb).quad(pb, pa, pa + Vec3::Y * (h + 0.003), pb + Vec3::Y * (h + 0.003), Mat::Curb.scale());
        out.collider_quad([pb, pa, pa + Vec3::Y * h, pb + Vec3::Y * h]);
        if let Some(inner) = &inner {
            let (ia, ib) = (inner[i], inner[(i + 1) % n]);
            let y = h + 0.003;
            parts.get(Mat::Curb).quad_uv(
                [Vec3::new(ia.x, y, ia.y), Vec3::new(ib.x, y, ib.y), Vec3::new(b.x, y, b.y), Vec3::new(a.x, y, a.y)],
                [Vec2::new(0.0, 0.0), Vec2::new((b - a).length(), 0.0), Vec2::new((b - a).length(), 0.3), Vec2::new(0.0, 0.3)],
            );
        }
    }
}

/// Block outline (counter-clockwise) and the kind of every edge.
fn block_ring(b: &Block, chamfer: f32) -> (Vec<Vec2>, Vec<bool>) {
    let v = [b.min, Vec2::new(b.max.x, b.min.y), b.max, Vec2::new(b.min.x, b.max.y)];
    let mut ring = Vec::new();
    let mut corner_edge = Vec::new();
    for k in 0..4 {
        let vin = (v[k] - v[(k + 3) % 4]).normalize();
        let vout = (v[(k + 1) % 4] - v[k]).normalize();
        match b.corners[k] {
            Corner::Sharp => {
                ring.push(v[k]);
                corner_edge.push(false);
            }
            Corner::Chamfer => {
                ring.push(v[k] - vin * chamfer);
                corner_edge.push(true);
                ring.push(v[k] + vout * chamfer);
                corner_edge.push(false);
            }
            Corner::Round => {
                let c = v[k] - vin * chamfer + vout * chamfer;
                let n = 6;
                for i in 0..=n {
                    let t = i as f32 / n as f32 * std::f32::consts::FRAC_PI_2;
                    let p = c + (-vout * t.cos() + vin * t.sin()) * chamfer;
                    ring.push(p);
                    corner_edge.push(i < n);
                }
            }
        }
    }
    (ring, corner_edge)
}

/// Splits a block into lots along its perimeter.
fn split_block(b: &Block, d: &District, rng: &mut Rng) -> Vec<(Lot, Facing)> {
    let v = [b.min, Vec2::new(b.max.x, b.min.y), b.max, Vec2::new(b.min.x, b.max.y)];
    let depth = d.depth.min((b.max - b.min).min_element() * 0.42);
    let inner = [b.min + Vec2::splat(depth), Vec2::new(b.max.x - depth, b.min.y + depth), b.max - Vec2::splat(depth), Vec2::new(b.min.x + depth, b.max.y - depth)];
    let c = d.chamfer;
    // Corner reach along each side from its starting corner and back from
    // its end corner.
    let side_len = |k: usize| (v[(k + 1) % 4] - v[k]).length();
    let mut reach = [[0.0f32; 2]; 4];
    for k in 0..4 {
        let l = side_len(k);
        for e in 0..2 {
            reach[k][e] = (depth + rng.range(2.5, 6.0)).max(c + 2.0).min(l * 0.5);
        }
        let middle = l - reach[k][0] - reach[k][1];
        if middle < d.lot_width[0] {
            reach[k] = [l * 0.5, l * 0.5];
        }
    }
    let corner_pts = |k: usize| -> Vec<Vec2> {
        let vin = (v[k] - v[(k + 3) % 4]).normalize();
        let vout = (v[(k + 1) % 4] - v[k]).normalize();
        match b.corners[k] {
            Corner::Sharp => vec![v[k]],
            Corner::Chamfer => vec![v[k] - vin * c, v[k] + vout * c],
            Corner::Round => {
                let ctr = v[k] - vin * c + vout * c;
                (0..=6)
                    .map(|i| {
                        let t = i as f32 / 6.0 * std::f32::consts::FRAC_PI_2;
                        ctr + (-vout * t.cos() + vin * t.sin()) * c
                    })
                    .collect()
            }
        }
    };
    let mut lots = Vec::new();
    for k in 0..4 {
        let dir = (v[(k + 1) % 4] - v[k]).normalize();
        let inward = Vec2::new(-dir.y, dir.x);
        let kp = (k + 3) % 4;
        let dir_p = (v[k] - v[kp]).normalize();
        let inward_p = Vec2::new(-dir_p.y, dir_p.x);
        // Corner lot at corner k.
        {
            let sa = v[k] - dir_p * reach[kp][1];
            let sb = v[k] + dir * reach[k][0];
            let pts = corner_pts(k);
            let mut ring = vec![sa];
            let mut edges = vec![EdgeKind::Street];
            for (i, p) in pts.iter().enumerate() {
                ring.push(*p);
                edges.push(if i + 1 < pts.len() { EdgeKind::Corner } else { EdgeKind::Street });
            }
            ring.extend([sb, sb + inward * depth, inner[k], sa + inward_p * depth]);
            edges.extend([EdgeKind::Party, EdgeKind::Courtyard, EdgeKind::Courtyard, EdgeKind::Party]);
            // The corner lot faces whichever of its two sides matters more.
            let f = if b.facing[k] == Facing::Avenue || b.facing[kp] == Facing::Avenue {
                Facing::Avenue
            } else if b.facing[k] == Facing::Plaza || b.facing[kp] == Facing::Plaza {
                Facing::Plaza
            } else if b.facing[k] == Facing::Outer && b.facing[kp] == Facing::Outer {
                Facing::Outer
            } else {
                Facing::Street
            };
            lots.push((Lot { ring, edges, base: 0.15 }, f));
        }
        // Middle lots on side k.
        let l = side_len(k);
        let (t0, t1) = (reach[k][0], l - reach[k][1]);
        let mut t = t0;
        while t1 - t > 0.5 {
            let mut w = rng.range(d.lot_width[0], d.lot_width[1]);
            if t1 - (t + w) < d.lot_width[0] {
                w = t1 - t;
            }
            let (p0, p1) = (v[k] + dir * t, v[k] + dir * (t + w));
            let ring = vec![p0, p1, p1 + inward * depth, p0 + inward * depth];
            lots.push((Lot { ring, edges: vec![EdgeKind::Street, EdgeKind::Party, EdgeKind::Courtyard, EdgeKind::Party], base: 0.15 }, b.facing[k]));
            t += w;
        }
    }
    lots
}

/// Open square in place of a block: granite setts, fountain or garden,
/// benches and lamps.
fn plaza(lib: &mut ModuleLib, lay: &mut CityLayout, b: &Block, d: &District, rng: &mut Rng) {
    let mut p: Parts<Mat> = Parts::new();
    let ring = [b.min, Vec2::new(b.max.x, b.min.y), b.max, Vec2::new(b.min.x, b.max.y)];
    let walk = inset(&ring, -d.sidewalk).unwrap_or(ring.to_vec());
    raised(&mut p, &mut lay.out, &walk, d.sidewalk_mat);
    p.get(Mat::Cobble).polygon(&ring, 0.155, true, Mat::Cobble.scale());
    let ctr = (b.min + b.max) * 0.5;
    let size = b.max - b.min;
    if d.valencia {
        // Big round fountain ringed by red flower beds and palms.
        let r = size.min_element() * 0.18;
        let f = lib.named(&mut lay.out, &format!("fountain_{}", r as i32), || furniture::fountain(r));
        lay.out.placements.push((f, Mat4::from_translation(Vec3::new(ctr.x, 0.15, ctr.y))));
        if lib.kit.night.is_night() {
            lay.out.city_lights.extend(nightlight::fountain_lights(Vec3::new(ctr.x, 0.55, ctr.y), r));
        }
        for k in 0..4 {
            let a = std::f32::consts::FRAC_PI_2 * k as f32 + std::f32::consts::FRAC_PI_4;
            let c = ctr + Vec2::from_angle(a) * (r + 8.0);
            let bed: Vec<Vec2> = (0..16).map(|i| c + Vec2::from_angle(std::f32::consts::TAU * i as f32 / 16.0) * 3.2).collect();
            p.get(Mat::Soil).polygon(&bed, 0.2, true, 2.0);
            flower_bed(&mut p, c, 3.0, rng);
            lay.props.push((PropKind::Palm, Mat4::from_translation(Vec3::new(c.x, 0.2, c.y))));
        }
    } else {
        // Austrias square: a formal garden parterre with a statue plinth.
        let g = [ctr - size * 0.18, Vec2::new(ctr.x + size.x * 0.18, ctr.y - size.y * 0.18), ctr + size * 0.18, Vec2::new(ctr.x - size.x * 0.18, ctr.y + size.y * 0.18)];
        p.get(Mat::Granite).aabox(Vec3::new(g[0].x - 0.3, 0.15, g[0].y - 0.3), Vec3::new(g[2].x + 0.3, 0.45, g[2].y + 0.3), 1.5);
        p.get(Mat::Grass).polygon(&g, 0.46, true, 3.0);
        for q in 0..4 {
            let c = ctr + Vec2::new(if q & 1 == 0 { -1.0 } else { 1.0 }, if q & 2 == 0 { -1.0 } else { 1.0 }) * size * 0.09;
            flower_bed(&mut p, c + Vec2::ZERO, 2.2, rng);
        }
        let plinth = Mat::Granite;
        p.get(plinth).aabox(Vec3::new(ctr.x - 1.0, 0.45, ctr.y - 1.0), Vec3::new(ctr.x + 1.0, 2.6, ctr.y + 1.0), 1.5);
        p.get(Mat::Galvanized).lathe(Vec3::new(ctr.x, 2.6, ctr.y), &[(0.0, 0.0), (0.55, 0.0), (0.35, 0.4), (0.3, 1.4), (0.42, 1.7), (0.25, 2.1), (0.2, 2.4), (0.0, 2.5)], 12);
        if lib.kit.night.is_night() {
            // The statue lit from the corners of its plinth.
            for q in 0..4 {
                let o = Vec2::new(if q & 1 == 0 { -1.6 } else { 1.6 }, if q & 2 == 0 { -1.6 } else { 1.6 });
                lay.out.city_lights.push(nightlight::accent(Vec3::new(ctr.x + o.x, 0.5, ctr.y + o.y), Vec3::new(ctr.x, 3.8, ctr.y), 2800.0, 30.0));
            }
        }
    }
    // Benches and lamps around the edge.
    for k in 0..4 {
        let (a, bb) = (ring[k], ring[(k + 1) % 4]);
        let dir = (bb - a).normalize();
        let inward = Vec2::new(-dir.y, dir.x);
        let len = (bb - a).length();
        let mut t = 8.0;
        while t < len - 6.0 {
            let q = a + dir * t + inward * 2.5;
            let lamp = street_lamp_module(lib, &mut lay.out, d.valencia, Vec3::new(q.x, 0.155, q.y), 0.155);
            lay.out.placements.push((lamp, Mat4::from_translation(Vec3::new(q.x, 0.155, q.y))));
            lay.out.lights.push((Vec3::new(q.x, 4.0, q.y), Vec3::new(1.0, 0.8, 0.55)));
            let bq = a + dir * (t + 6.0) + inward * 3.5;
            let yaw = (-inward).x.atan2(-inward.y) + std::f32::consts::PI;
            lay.props.push((PropKind::Bench, Mat4::from_rotation_translation(Quat::from_rotation_y(yaw), Vec3::new(bq.x, 0.155, bq.y))));
            t += 14.0;
        }
    }
    let si = lay.out.add_scene(assemble(lib.kit, "plaza", p, None));
    lay.out.placements.push((si, Mat4::IDENTITY));
}

/// Dense low planting with red and white flowers.
fn flower_bed(p: &mut Parts<Mat>, c: Vec2, r: f32, rng: &mut Rng) {
    let leaves = p.get(Mat::Leaves);
    let n = (r * r * 5.0) as i32;
    for _ in 0..n {
        let q = c + Vec2::from_angle(rng.range(0.0, std::f32::consts::TAU)) * r * rng.f().sqrt();
        let s = rng.range(0.14, 0.24);
        leaves.lathe(Vec3::new(q.x, 0.2, q.y), &[(0.0, 0.0), (s, s * 0.4), (s * 0.8, s * 1.1), (0.0, s * 1.4)], 5);
    }
    let reds = p.get(Mat::Flowers(hex(0xd31f2a)));
    for _ in 0..n * 3 {
        let q = c + Vec2::from_angle(rng.range(0.0, std::f32::consts::TAU)) * r * rng.f().sqrt();
        let y = 0.2 + rng.range(0.2, 0.34);
        reds.aabox(Vec3::new(q.x - 0.035, y, q.y - 0.035), Vec3::new(q.x + 0.035, y + 0.05, q.y + 0.035), 1.0);
    }
}

fn paint(p: &mut Parts<Mat>, a: Vec2, b: Vec2, w: f32, yellow: bool) {
    let d = (b - a).normalize_or(Vec2::X);
    let n = Vec2::new(-d.y, d.x) * (w * 0.5);
    let m = Mat::Paint(if yellow { hex(0xe8b51c) } else { hex(0xf2f2ec) });
    let y = 0.008;
    let pts = [a - n, b - n, b + n, a + n];
    p.get(m).polygon(&pts, y, true, 1.0);
}

/// Lanes and parking strips of a segment, matching [`street_markings`].
fn traffic_street(s: &Segment) -> TrafficStreet {
    let road = s.width * 0.5 - s.sidewalk;
    let (lanes, parking, median) = if s.avenue {
        let (inner, outer) = (4.5 + 0.3, road - 0.3);
        let lane = (outer - inner) / 3.0;
        let mut lanes = Vec::new();
        for side in [-1.0f32, 1.0] {
            for k in 0..3 {
                lanes.push(TrafficLane { offset: side * (inner + lane * (k as f32 + 0.5)), forward: side < 0.0, width: lane });
            }
        }
        (lanes, Vec::new(), 4.5)
    } else if road > 4.5 {
        let fwd = s.oneway >= 0;
        (vec![TrafficLane { offset: -1.4, forward: fwd, width: 2.8 }, TrafficLane { offset: 1.4, forward: fwd, width: 2.8 }], vec![-(road - 1.1), road - 1.1], 0.0)
    } else {
        // One lane between the parking strip and the yellow kerb line.
        let lo = -(road - 2.0);
        let hi = road - 0.25;
        (vec![TrafficLane { offset: (lo + hi) * 0.5, forward: s.oneway >= 0, width: (hi - lo).min(3.4) }], vec![-(road - 1.0)], 0.0)
    };
    TrafficStreet {
        a: s.a,
        b: s.b,
        half_width: s.width * 0.5,
        road_half: road,
        sidewalk: s.sidewalk,
        median_half: median,
        lanes,
        parking,
        speed_kmh: if s.avenue || s.district == 1 { 50.0 } else { 30.0 },
        // `street_markings`: zebra 1-5 m from the end, stop line at 6 m.
        crosswalk: Some(3.0),
        stop_line: 6.0,
    }
}

/// Lane lines, parking bays, zebra crossings, stop lines and arrows.
fn street_markings(p: &mut Parts<Mat>, s: &Segment) {
    let len = (s.b - s.a).length();
    let dir = (s.b - s.a) / len;
    let left = Vec2::new(-dir.y, dir.x);
    let road = s.width * 0.5 - s.sidewalk;
    let at = |t: f32, o: f32| s.a + dir * t + left * o;
    let cross0 = 1.0;
    let cross_len = 4.0;
    let stop = cross0 + cross_len + 1.0;
    let (t0, t1) = (stop + 0.5, len - stop - 0.5);
    let dashed = |p: &mut Parts<Mat>, o: f32, dash: f32, gap: f32| {
        let mut t = t0;
        while t < t1 {
            paint(p, at(t, o), at((t + dash).min(t1), o), 0.12, false);
            t += dash + gap;
        }
    };
    if s.avenue {
        // Median edge lines and three lanes each way.
        for side in [-1.0f32, 1.0] {
            let inner = 4.5 + 0.3;
            let outer = road - 0.3;
            paint(p, at(t0, side * inner), at(t1, side * inner), 0.15, false);
            paint(p, at(t0, side * outer), at(t1, side * outer), 0.15, false);
            let lane = (outer - inner) / 3.0;
            for k in 1..3 {
                dashed(p, side * (inner + lane * k as f32), 4.5, 7.5);
            }
            // Bus lane lettering band on the kerb lane.
            let mut t = t0 + 20.0;
            while t < t1 - 20.0 {
                arrow(p, at(t, side * (inner + lane * 0.5)), dir * -side);
                arrow(p, at(t + 2.0, side * (inner + lane * 1.5)), dir * -side);
                arrow(p, at(t + 4.0, side * (inner + lane * 2.5)), dir * -side);
                t += 60.0;
            }
        }
    } else if road > 4.5 {
        // Parking lanes on both sides and two one-way lanes.
        for side in [-1.0f32, 1.0] {
            let o = side * (road - 2.2);
            paint(p, at(t0, o), at(t1, o), 0.1, false);
            let mut t = t0 + 2.0;
            while t < t1 {
                paint(p, at(t, o), at(t, side * road), 0.1, false);
                t += 4.8;
            }
        }
        dashed(p, 0.0, 3.0, 4.5);
        let fwd = dir * s.oneway.signum() as f32;
        let mut t = t0 + 12.0;
        while t < t1 - 8.0 {
            for o in [-1.4, 1.4] {
                arrow(p, at(t, o), fwd);
            }
            t += 35.0;
        }
    } else {
        // Narrow Madrid street: one lane plus parking on one side, yellow
        // zigzag-free kerb line on the other (no parking).
        let o = -(road - 2.0);
        paint(p, at(t0, o), at(t1, o), 0.1, false);
        paint(p, at(t0, road - 0.25), at(t1, road - 0.25), 0.12, true);
        let fwd = dir * s.oneway.signum() as f32;
        let mut t = t0 + 10.0;
        while t < t1 - 6.0 {
            arrow(p, at(t, 1.0), fwd);
            t += 30.0;
        }
    }
    // Zebra crossings and stop lines at both ends.
    for (end, inward) in [(0.0f32, 1.0f32), (len, -1.0)] {
        let c0 = end + inward * cross0;
        let c1 = end + inward * (cross0 + cross_len);
        let mut o = -road + 0.3;
        while o < road - 0.3 {
            paint(p, at(c0, o + 0.25), at(c1, o + 0.25), 0.5, false);
            o += 1.0;
        }
        // Stop line across the lanes arriving at this end.
        let ts = end + inward * stop;
        let arriving = match s.oneway {
            1 => inward < 0.0,
            -1 => inward > 0.0,
            _ => true,
        };
        if arriving {
            let (oa, ob) = if s.oneway == 0 { if inward < 0.0 { (-road, 0.0) } else { (0.0, road) } } else { (-road, road) };
            paint(p, at(ts, oa + 0.2), at(ts, ob - 0.2), 0.4, false);
        }
    }
}

/// Straight-ahead lane arrow (5 m long) at `c` pointing along `fwd`.
fn arrow(p: &mut Parts<Mat>, c: Vec2, fwd: Vec2) {
    let side = Vec2::new(-fwd.y, fwd.x);
    paint(p, c - fwd * 2.5, c + fwd * 1.0, 0.18, false);
    let tip = c + fwd * 2.5;
    let base = c + fwd * 0.9;
    let m = Mat::Paint(hex(0xf2f2ec));
    p.get(m).polygon(&[base - side * 0.45, base + side * 0.45, tip], 0.008, true, 1.0);
}

/// Lamps, trees, bins, containers and signs along a street.
fn street_furniture(lib: &mut ModuleLib, lay: &mut CityLayout, s: &Segment, d: &District, rng: &mut Rng) {
    if s.avenue {
        return;
    }
    let len = (s.b - s.a).length();
    let dir = (s.b - s.a) / len;
    let left = Vec2::new(-dir.y, dir.x);
    let road = s.width * 0.5 - s.sidewalk;
    let at = |t: f32, o: f32| s.a + dir * t + left * o;
    let yaw_of = |v: Vec2| v.x.atan2(v.y);
    let place = |lay: &mut CityLayout, m: usize, p: Vec2, y: f32, face: Vec2| {
        lay.out.placements.push((m, Mat4::from_rotation_translation(Quat::from_rotation_y(yaw_of(face)), Vec3::new(p.x, y, p.y))));
    };
    if d.valencia {
        let grate = lib.named(&mut lay.out, "tree_grate", furniture::tree_grate);
        for side in [-1.0f32, 1.0] {
            let o = side * (road + 0.7);
            let mut t = 12.0 + rng.range(0.0, 4.0);
            let mut k = 0;
            while t < len - 12.0 {
                let q = at(t, o);
                if k % 3 == 1 {
                    let lamp = street_lamp_module(lib, &mut lay.out, true, Vec3::new(q.x, 0.15, q.y), 0.15);
                    place(lay, lamp, q, 0.15, left * -side);
                    lay.out.lights.push((Vec3::new(q.x, 4.4, q.y), Vec3::new(1.0, 0.82, 0.58)));
                } else {
                    let qt = at(t, side * (road + 1.0));
                    place(lay, grate, qt, 0.15, dir);
                    lay.props.push((PropKind::Tree, Mat4::from_rotation_translation(Quat::from_rotation_y(rng.range(0.0, std::f32::consts::TAU)), Vec3::new(qt.x, 0.14, qt.y))));
                }
                t += 9.0;
                k += 1;
            }
        }
    } else {
        // Madrid: rows of black bollards along the narrow sidewalks.
        let bol = lib.named(&mut lay.out, "bollard_madrid", || furniture::bollard(false));
        for side in [-1.0f32, 1.0] {
            let o = side * (road + 0.3);
            let mut t = 8.0;
            while t < len - 8.0 {
                place(lay, bol, at(t, o), 0.15, dir);
                t += 2.2;
            }
        }
    }
    // Litter bins and a group of recycling containers in the parking lane.
    let bin = lib.named(&mut lay.out, "litter_bin", furniture::litter_bin);
    let mut t = 20.0;
    while t < len - 20.0 {
        let side = if (t as i32 / 20) % 2 == 0 { 1.0 } else { -1.0 };
        place(lay, bin, at(t + 4.0, side * (road + 0.5)), 0.15, left * -side);
        t += 40.0;
    }
    if len > 40.0 && road > 3.0 {
        let side = -1.0;
        let t = len * 0.5 + rng.range(-8.0, 8.0);
        for k in 0..4u8 {
            let kind = [0u8, 1, 2, 3][k as usize];
            let c = lib.named(&mut lay.out, &format!("container_{kind}"), || furniture::container(kind));
            place(lay, c, at(t + k as f32 * 2.0, side * (road - 1.1)), 0.0, left * -side);
        }
    }
    // Signs at the street entrance and exit.
    let one_way_entry = if s.oneway >= 0 { 0.0 } else { len };
    let exit = len - one_way_entry;
    let face_in = if s.oneway >= 0 { dir } else { -dir };
    if s.oneway != 0 {
        let no_entry = lib.named(&mut lay.out, "sign_NoEntry", || furniture::sign(SignKind::NoEntry, 2.3));
        let q = at(exit + (if exit > 0.0 { -1.5 } else { 1.5 }), -face_in.perp_dot(left).signum() * (road + 0.4));
        // Facing whoever would drive in against the one-way direction.
        place(lay, no_entry, q, 0.15, face_in);
        lay.traffic.signs.push(TrafficSign { kind: SignKind::NoEntry, pos: q, facing: face_in });
    }
    let speed = if d.valencia { SignKind::Speed50 } else { SignKind::Speed30 };
    let sp = lib.named(&mut lay.out, &format!("sign_{speed:?}"), || furniture::sign(speed, 2.3));
    let entry_t = if one_way_entry > 0.0 { len - 9.0 } else { 9.0 };
    let right = Vec2::new(face_in.y, -face_in.x);
    let q = s.a + dir * entry_t + right * (road + 0.4);
    place(lay, sp, q, 0.15, -face_in);
    lay.traffic.signs.push(TrafficSign { kind: speed, pos: q, facing: -face_in });
    if rng.chance(0.4) {
        let np = lib.named(&mut lay.out, "sign_NoParking", || furniture::sign(SignKind::NoParking, 2.3));
        let q = s.a + dir * (len * 0.5) - right * (road + 0.4);
        place(lay, np, q, 0.15, -face_in);
        lay.traffic.signs.push(TrafficSign { kind: SignKind::NoParking, pos: q, facing: -face_in });
    }
}

/// Traffic lights (Valencia, boulevard) or STOP/crossing signs (Madrid) at
/// each street end.
fn intersections(lib: &mut ModuleLib, lay: &mut CityLayout, segments: &[Segment], cfg: &CityConfig) {
    for s in segments {
        let d = if s.district == 0 { &cfg.west } else { &cfg.east };
        let len = (s.b - s.a).length();
        let dir = (s.b - s.a) / len;
        let road = s.width * 0.5 - s.sidewalk;
        for (end, inward) in [(0.0f32, 1.0f32), (len, -1.0)] {
            // Traffic travelling towards this end.
            let arriving = match s.oneway {
                1 => inward < 0.0,
                -1 => inward > 0.0,
                _ => true,
            };
            if !arriving {
                continue;
            }
            let travel = dir * -inward;
            let right = Vec2::new(travel.y, -travel.x);
            let t = end + inward * 6.0;
            let pos = s.a + dir * t + right * (road + 0.45);
            let yaw = (-travel).x.atan2(-travel.y);
            let along_z = dir.y.abs() > 0.5;
            if d.traffic_lights || s.avenue {
                // Streets along z get green, along x red; pedestrians opposite.
                let state = if along_z { Aspect::Green } else { Aspect::Red };
                let arm = if s.avenue { 5.0 } else { 0.0 };
                signal_post(lib, lay, pos, yaw, state, arm);
                if !s.avenue {
                    let c = lib.named(&mut lay.out, "sign_Crossing", || furniture::sign(SignKind::Crossing, 2.6));
                    let p2 = s.a + dir * (end + inward * 12.0) + right * (road + 0.4);
                    lay.out.placements.push((c, Mat4::from_rotation_translation(Quat::from_rotation_y(yaw), Vec3::new(p2.x, 0.15, p2.y))));
                    lay.traffic.signs.push(TrafficSign { kind: SignKind::Crossing, pos: p2, facing: facing_of_yaw(yaw) });
                }
            } else {
                let kind = if along_z { SignKind::Stop } else { SignKind::Yield };
                let m = lib.named(&mut lay.out, &format!("sign_{kind:?}"), || furniture::sign(kind, 2.2));
                lay.out.placements.push((m, Mat4::from_rotation_translation(Quat::from_rotation_y(yaw), Vec3::new(pos.x, 0.15, pos.y))));
                lay.traffic.signs.push(TrafficSign { kind, pos, facing: facing_of_yaw(yaw) });
                let c = lib.named(&mut lay.out, "sign_Crossing", || furniture::sign(SignKind::Crossing, 2.6));
                let p2 = s.a + dir * (end + inward * 9.0) - right * (road + 0.4);
                lay.out.placements.push((c, Mat4::from_rotation_translation(Quat::from_rotation_y(yaw), Vec3::new(p2.x, 0.15, p2.y))));
                lay.traffic.signs.push(TrafficSign { kind: SignKind::Crossing, pos: p2, facing: facing_of_yaw(yaw) });
            }
        }
    }
    // Where signalled side streets meet the boulevard, the boulevard gets
    // lights too (cantilevered over its three lanes), both directions.
    let Some(avenue) = segments.iter().find(|s| s.avenue) else { return };
    let road = avenue.width * 0.5 - avenue.sidewalk;
    let half_av = avenue.width * 0.5;
    let mut junctions: Vec<(f32, f32)> = Vec::new();
    for s in segments.iter().filter(|s| !s.avenue) {
        let d = if s.district == 0 { &cfg.west } else { &cfg.east };
        if !d.traffic_lights {
            continue;
        }
        for p in [s.a, s.b] {
            if (p.x.abs() - half_av).abs() < 0.5 && (p.y - s.a.y).abs() < 0.1 && (s.b.y - s.a.y).abs() < 0.1 && !junctions.iter().any(|j| (j.0 - p.y).abs() < 1.0) {
                junctions.push((p.y, s.width * 0.5));
            }
        }
    }
    for (z, side_half) in junctions {
        for travel in [Vec2::Y, -Vec2::Y] {
            let right = Vec2::new(travel.y, -travel.x);
            let pos = Vec2::new(0.0, z) - travel * (side_half + 2.5) + right * (road + 0.45);
            if pos.y.abs() > avenue.a.y.abs().max(avenue.b.y.abs()) - 3.0 {
                continue;
            }
            let yaw = (-travel).x.atan2(-travel.y);
            signal_post(lib, lay, pos, yaw, Aspect::Green, 5.0);
        }
    }
}

/// A traffic light facing `yaw` at `pos`, with the module variants of all
/// its states registered for the game to switch between.
fn signal_post(lib: &mut ModuleLib, lay: &mut CityLayout, pos: Vec2, yaw: f32, state: Aspect, arm: f32) {
    let ped_yaw = -std::f32::consts::FRAC_PI_2;
    let mut variants = [[0usize; 2]; 3];
    for (i, a) in [Aspect::Red, Aspect::Amber, Aspect::Green].into_iter().enumerate() {
        for (k, walk) in [false, true].into_iter().enumerate() {
            variants[i][k] = lib.named(&mut lay.out, &format!("tl_{a:?}_{walk}_{arm}"), || furniture::traffic_light(a, walk, ped_yaw, arm));
        }
    }
    // The pedestrian head serves the zebra across this street: WALK while
    // this traffic is held at red.
    let walk = state == Aspect::Red;
    let idx = match state {
        Aspect::Red => 0,
        Aspect::Amber => 1,
        Aspect::Green => 2,
    };
    let at = Mat4::from_rotation_translation(Quat::from_rotation_y(yaw), Vec3::new(pos.x, 0.15, pos.y));
    lay.traffic.signals.push(SignalPost { pos, facing: facing_of_yaw(yaw), placement: lay.out.placements.len(), variants });
    lay.out.placements.push((variants[idx][walk as usize], at));
    if lib.kit.night.is_night() {
        lay.out.city_lights.extend(furniture::signal_lights(state, walk, ped_yaw, arm).into_iter().map(|mut l| {
            l.pos = at.transform_point3(l.pos);
            if let Some(s) = &mut l.spot {
                s.dir = at.transform_vector3(s.dir);
            }
            l
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nightlight::LightKind;

    #[test]
    fn block_rings_are_ccw_and_lots_tile_the_perimeter() {
        let d = District::valencia();
        let mut rng = Rng::new(3);
        for corners in [[Corner::Chamfer; 4], [Corner::Round; 4], [Corner::Sharp; 4]] {
            let b = Block { min: Vec2::new(0.0, 0.0), max: Vec2::new(100.0, 96.0), corners, facing: [Facing::Street; 4], plaza: false };
            let (ring, _) = block_ring(&b, d.chamfer);
            assert!(crate::geom::signed_area(&ring) > 0.0);
            let lots = split_block(&b, &d, &mut rng);
            assert!(lots.len() >= 12, "{} lots", lots.len());
            let mut area = 0.0;
            for (l, _) in &lots {
                let a = crate::geom::signed_area(&l.ring);
                assert!(a > 20.0, "lot area {a} ring {:?}", l.ring);
                assert_eq!(l.ring.len(), l.edges.len());
                area += a;
            }
            // Lots cover the ring between the outline and the courtyard.
            let depth = d.depth;
            let outline = crate::geom::signed_area(&ring);
            let court = (100.0 - 2.0 * depth) * (96.0 - 2.0 * depth);
            assert!((area - (outline - court)).abs() < 2.0, "area {area} expected {}", outline - court);
        }
    }

    #[test]
    fn terraces_stay_on_the_sidewalk() {
        let kit = Kit::load(None);
        let mut lib = ModuleLib::new(&kit);
        let mut lay = CityLayout::default();
        let fr = Frontage { pos: Vec3::new(0.0, 0.15, 0.0), right: Vec3::X, out: Vec3::Z, width: 3.0, bar: true, closed: false };
        for seed in 0..8 {
            terrace(&mut lib, &mut lay, &fr, 4.5, &mut Rng::new(seed));
        }
        assert!(lay.out.placements.len() >= 8);
        for (s, m) in &lay.out.placements {
            let b = lay.out.scenes[*s].bounds();
            for c in [b.min, b.max, Vec3::new(b.min.x, 0.0, b.max.z), Vec3::new(b.max.x, 0.0, b.min.z)] {
                let w = m.transform_point3(c);
                // Clear of the facade and of the tree line 1 m from the kerb.
                assert!(w.z > 0.5 && w.z < 3.5, "terrace piece at {w:?}");
                assert!((w.y - 0.15).abs() < 3.0);
            }
        }
    }

    #[test]
    fn night_lights_the_city_and_day_is_untouched() {
        let styles = StyleLibrary::builtin();
        let mut kit = Kit::load(None);
        let day = build_city(&kit, &styles, &CityConfig::small());
        kit.set_hour(14.0);
        let afternoon = build_city(&kit, &styles, &CityConfig::small());
        assert_eq!(day.out.placements.len(), afternoon.out.placements.len());
        assert_eq!(day.out.scenes.len(), afternoon.out.scenes.len());
        let night_mats = ["room_lit", "neon", "shop_sign_lit", "lantern_glass"];
        let has = |c: &CityLayout, n: &str| c.out.scenes.iter().any(|s| s.materials.iter().any(|m| m.name == n));
        for n in night_mats {
            assert!(!has(&day, n), "day city has night material {n}");
        }
        kit.set_hour(22.5);
        let night = build_city(&kit, &styles, &CityConfig::small());
        // Same city, same layout: only modules and lights differ.
        assert_eq!(day.out.buildings, night.out.buildings);
        assert_eq!(day.out.lights.len(), night.out.lights.len());
        assert!(has(&night, "room_lit") && has(&night, "lantern_glass"));
        let count = |k: LightKind| night.out.city_lights.iter().filter(|l| l.kind == k).count();
        assert!(count(LightKind::Window) > 50, "{} lit windows", count(LightKind::Window));
        assert!(count(LightKind::Road) > 0 && count(LightKind::Wall) > 0);
        assert!(night.out.city_lights.len() > day.out.city_lights.len() * 3);
    }

    #[test]
    fn small_city_builds_with_streets_and_furniture() {
        let kit = Kit::load(None);
        let styles = StyleLibrary::builtin();
        let city = build_city(&kit, &styles, &CityConfig::small());
        assert!(city.out.buildings > 20, "{} buildings", city.out.buildings);
        assert!(city.spawns.iter().any(|s| s.vehicle) && city.spawns.iter().any(|s| !s.vehicle));
        assert!(!city.out.lights.is_empty());
        assert!(city.props.iter().any(|p| p.0 == PropKind::Tree));
        // Every placement sits inside the map.
        for (_, m) in &city.out.placements {
            let p = m.w_axis.truncate();
            assert!(p.x.abs() <= city.extent.x + 1.0 && p.z.abs() <= city.extent.y + 1.0, "{p:?}");
        }
    }
}
