//! City assembly: plan → draft → fabric → meshes.
//!
//! `build_city` runs the generators of a `CityPlan` (grids, organic old
//! towns, or a given draft such as an OpenStreetMap import), turns the
//! draft into a `Fabric` (lots, public space, ground classes) and realizes
//! it: ground meshes and kerbs per 64 m tile, buildings per tile (every
//! lot through `building::build_into`, medianeras once per block), road
//! markings, then street furniture planned in 2D.

use std::collections::BTreeMap;

use forja_math::{Mat4, Quat, Vec2, Vec3};

use crate::building::{self, CityOut, ModuleLib};
use crate::dress::{self, Item, ItemKind};
use crate::fabric::{Fabric, stage};
use crate::furniture::{self, Aspect};
use crate::street_waste::{self, BinModel};
use crate::generators::{self, CityPlan};
use crate::geom::{Parts, Rng};
use crate::ground;
use crate::kit::{Kit, Mat, assemble};
use crate::nightlight::{self, FloodOptions};
use crate::plan::{Draft, End};
use crate::profile::{Class, ProfileLibrary};
use crate::style::{Style, StyleLibrary};
use crate::traffic::{HeightGrid, SignalPost, TrafficEnd, TrafficInfo, TrafficLane, TrafficSign, TrafficStreet, facing_of_yaw};

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

/// A street of the realized city.
#[derive(Clone, Debug)]
pub struct StreetInfo {
    pub line: Vec<Vec2>,
    /// Building line to building line (narrowest).
    pub width: f32,
    pub carriageway: f32,
    pub profile: String,
    pub class: Class,
    pub zone: u16,
    pub vehicles: bool,
}

#[derive(Default)]
pub struct CityLayout {
    pub out: CityOut,
    pub props: Vec<(PropKind, Mat4)>,
    pub spawns: Vec<Spawn>,
    /// Half size of the playable area (centred on the origin).
    pub extent: Vec2,
    pub stats: Vec<(String, usize)>,
    pub streets: Vec<StreetInfo>,
    pub fabric: Fabric,
    /// Planned furniture (for tests and tools).
    pub items: Vec<Item>,
    /// Lanes, junctions, signs and traffic lights for the traffic AI.
    pub traffic: TrafficInfo,
}

#[derive(Clone, Debug)]
pub struct CityConfig {
    pub seed: u64,
    pub plan: CityPlan,
}

impl Default for CityConfig {
    /// Grids, boulevard and the Austrias and Carmen old towns.
    fn default() -> Self {
        Self::preset("default").expect("default plan")
    }
}

impl CityConfig {
    /// Today's map: a Madrid barrio and a Valencian Ensanche on either side
    /// of a boulevard.
    pub fn classic() -> Self {
        Self::preset("classic").expect("classic plan")
    }

    /// A small city for tests and quick previews.
    pub fn small() -> Self {
        let mut c = Self::classic();
        for z in &mut c.plan.zones {
            if let Some(g) = &mut z.grid {
                if z.city == "madrid" {
                    g.cols = 2;
                    g.rows = 2;
                } else {
                    g.cols = 1;
                    g.rows = 1;
                }
                g.plaza = None;
            }
        }
        c
    }

    /// A built-in plan: `classic`, `default`, `small`, `carmen`,
    /// `austrias`, `organic`.
    pub fn preset(name: &str) -> Option<Self> {
        if name == "small" {
            return Some(Self::small());
        }
        CityPlan::builtin(name).map(|plan| CityConfig { seed: 7, plan })
    }

    /// A city from a given draft (OSM import, test fixture).
    pub fn from_draft(d: Draft, city: &str) -> Self {
        CityConfig { seed: 7, plan: CityPlan::from_draft(d, city) }
    }
}

/// Weighted pick from a style pool.
fn pick<'a>(rng: &mut Rng, w: &'a [(String, f32)]) -> Option<&'a str> {
    let total: f32 = w.iter().map(|x| x.1).sum();
    let mut r = rng.f() * total;
    for (s, x) in w {
        if r < *x {
            return Some(s);
        }
        r -= x;
    }
    w.first().map(|x| x.0.as_str())
}

/// Builds the whole city.
pub fn build_city(kit: &Kit, styles: &StyleLibrary, cfg: &CityConfig) -> CityLayout {
    let lib = ProfileLibrary::builtin();
    build_city_with(kit, styles, &lib, cfg)
}

pub fn build_city_with(kit: &Kit, styles: &StyleLibrary, lib: &ProfileLibrary, cfg: &CityConfig) -> CityLayout {
    let draft = generators::draft(&cfg.plan, lib, cfg.seed);
    let fabric = Fabric::build(&draft, &cfg.plan, lib, cfg.seed);
    #[cfg(debug_assertions)]
    if let Err(v) = fabric.validate() {
        log::warn!("fabric: {} violations, first {:?}", v.len(), v.first());
    }
    realize(kit, styles, fabric, &cfg.plan, lib, cfg.seed)
}

/// Meshes a fabric.
pub fn realize(kit: &Kit, styles: &StyleLibrary, fabric: Fabric, plan: &CityPlan, lib: &ProfileLibrary, seed: u64) -> CityLayout {
    let mut lay = CityLayout::default();
    let mut mlib = ModuleLib::new(kit);
    let [lo, hi] = fabric.extent;
    let ext = Vec2::new(lo.x.abs().max(hi.x.abs()), lo.y.abs().max(hi.y.abs())) + Vec2::splat(10.0);
    lay.extent = ext;
    // 1. Ground: on flat ground a plane at roadway level under everything;
    // on a slope the roadway and the land around the city are meshed on
    // the relief. Then ground tiles, kerbs, gutters.
    let map = [Vec2::new(-ext.x, -ext.y), Vec2::new(ext.x, -ext.y), Vec2::new(ext.x, ext.y), Vec2::new(-ext.x, ext.y)];
    if fabric.ground.relief.is_flat() {
        let mut plane: Parts<Mat> = Parts::new();
        plane.get(Mat::Asphalt).polygon(&map, 0.0, true, Mat::Asphalt.scale());
        let gi = lay.out.add_scene(assemble(kit, "ground_plane", plane, None));
        lay.out.placements.push((gi, Mat4::IDENTITY));
    }
    let outside = crate::plan::Region::ring(&map).minus(&fabric.domain);
    let (tiles, col) = ground::mesh(&fabric.ground, &fabric.graph, lib, &fabric.courts, &outside);
    for (_, parts) in tiles {
        if parts.tri_count() == 0 {
            continue;
        }
        let si = lay.out.add_scene(assemble(kit, "streets", parts, None));
        lay.out.placements.push((si, Mat4::IDENTITY));
    }
    for t in col {
        let b = lay.out.col_verts.len() as u32;
        lay.out.col_verts.extend(t);
        lay.out.col_tris.push([b, b + 1, b + 2]);
    }
    // 2. Buildings, grouped by the 64 m tile of their block.
    let mut eaves = vec![0.0f32; fabric.lots.len()];
    let mut by_tile: BTreeMap<(i32, i32), Vec<usize>> = BTreeMap::new();
    for (bi, b) in fabric.blocks.iter().enumerate() {
        if b.lots.is_empty() {
            continue;
        }
        let c = b.shape.outer.iter().copied().sum::<Vec2>() / b.shape.outer.len().max(1) as f32;
        by_tile.entry(((c.x / 64.0).floor() as i32, (c.y / 64.0).floor() as i32)).or_default().push(bi);
    }
    let (mut built, mut plinths) = (0usize, 0usize);
    for (_, blocks) in by_tile {
        let mut parts: Parts<Mat> = Parts::new();
        for bi in blocks {
            let b = &fabric.blocks[bi];
            let first_wall = lay.out.party_walls.len();
            for li in b.lots.clone() {
                let lot = &fabric.lots[li];
                let info = &fabric.info[li];
                let Some(style) = style_for(styles, plan, info.zone, &info.pool, info.key, seed) else { continue };
                let mut rng = Rng::keyed(seed, stage::LOT ^ 0x99, info.key);
                eaves[li] = building::build_into(&mut mlib, &mut lay.out, &mut parts, lot, style, rng.next_u32() as u64 | (rng.next_u32() as u64) << 32);
                plinths += plinth(&mut parts, &mut lay.out, lot, &fabric.ground);
                // Fronts on a plaza are floodlit at night.
                if kit.night.is_night() {
                    floodlight_lot(&mut lay.out, lot, eaves[li]);
                }
                built += 1;
            }
            building::medianeras(&mut mlib, &mut lay.out, &mut parts, first_wall);
        }
        if parts.tri_count() > 0 {
            let si = lay.out.add_scene(assemble(kit, "block", parts, None));
            lay.out.placements.push((si, Mat4::IDENTITY));
        }
    }
    // 3. Road markings.
    let marks = ground::markings(&fabric.ground, &fabric.graph, lib);
    if marks.tri_count() > 0 {
        let mi = lay.out.add_scene(assemble(kit, "markings", marks, None));
        lay.out.placements.push((mi, Mat4::IDENTITY));
    }
    // 4. Furniture.
    let fronts = lay.out.frontages.clone();
    let mut items = dress::street_items(&fabric, plan, lib, &fronts, seed);
    items.extend(dress::after_buildings(&fabric, lib, &eaves, &fronts, seed));
    place_items(&mut mlib, &mut lay, &items, seed);
    lay.items = items;
    // 5. What the traffic AI drives by: the street graph's roadways with
    // their lanes, and the signs and lights just placed.
    traffic_info(&mut lay, &fabric, plan, lib);
    // 6. Spawns: drivable cars in parking strips, the player on the start
    // zone's main square or a sidewalk near the middle.
    let start = plan.start.as_ref().and_then(|id| plan.zones.iter().position(|z| &z.id == id)).map(|i| (i + 1) as u16);
    spawns(&mut lay, &fabric, start);
    // 7. Streets and stats.
    lay.streets = fabric
        .graph
        .streets
        .iter()
        .map(|s| {
            let p = lib.get(s.profile);
            StreetInfo { line: s.line.pts().to_vec(), width: s.min_width(), carriageway: s.carriageway, profile: p.id.clone(), class: p.class, zone: s.zone, vehicles: p.vehicles }
        })
        .collect();
    lay.stats = fabric.stats.clone();
    lay.stats.push(("buildings".into(), lay.out.buildings));
    lay.stats.push(("built_lots".into(), built));
    lay.stats.push(("plinth_m".into(), plinths));
    lay.stats.push(("modules".into(), lay.out.scenes.len()));
    lay.stats.push(("lights".into(), lay.out.lights.len()));
    lay.fabric = fabric;
    lay
}

/// A stone plinth (zócalo) under the street and courtyard facades of a
/// lot wherever the ground falls below the building's foundation
/// (`base - SKIRT`): on a slope the facade stands level at its highest
/// ground and the plinth grows downhill, as on Madrid's sloping streets.
/// Returns the metres of plinth built.
fn plinth(parts: &mut Parts<Mat>, out: &mut CityOut, lot: &crate::lot::Lot, ground: &ground::Ground) -> usize {
    if ground.relief.is_flat() {
        return 0;
    }
    let top = lot.base - crate::lot::SKIRT + 0.02;
    let mut built = 0.0f32;
    for k in 0..lot.ring.len() {
        if lot.edges[k] == crate::lot::EdgeKind::Party {
            continue;
        }
        let (a, b) = lot.edge(k);
        let d = (b - a).normalize_or(Vec2::X);
        let o = Vec2::new(d.y, -d.x);
        let n = (a.distance(b) / 2.0).ceil().max(1.0) as usize;
        for i in 0..n {
            let (p0, p1) = (a.lerp(b, i as f32 / n as f32), a.lerp(b, (i + 1) as f32 / n as f32));
            let (g0, g1) = (ground.height_at(p0 + o * 0.15) - 0.05, ground.height_at(p1 + o * 0.15) - 0.05);
            if top - g0.min(g1) < 0.03 {
                continue;
            }
            let (b0, b1) = (Vec3::new(p0.x, g0.min(top), p0.y), Vec3::new(p1.x, g1.min(top), p1.y));
            let (t0, t1) = (Vec3::new(p0.x, top, p0.y), Vec3::new(p1.x, top, p1.y));
            // Facing out of the lot (right of a -> b).
            parts.get(Mat::Granite).quad(b1, b0, t0, t1, Mat::Granite.scale());
            let base = out.col_verts.len() as u32;
            out.col_verts.extend([b1, b0, t0, t1]);
            out.col_tris.extend([[base, base + 1, base + 2], [base, base + 2, base + 3]]);
            built += p0.distance(p1);
        }
    }
    built as usize
}

/// The style of a lot: from its zone's pool for its key, falling back to
/// the zone's street pool, then any style of the zone's city.
fn style_for<'a>(styles: &'a StyleLibrary, plan: &'a CityPlan, zone: u16, pool: &str, key: u64, seed: u64) -> Option<&'a Style> {
    let mut rng = Rng::keyed(seed, stage::LOT, key);
    let z = plan.zone(zone);
    if let Some(z) = z {
        for p in [pool, "street"] {
            let list = z.pool(p);
            if let Some(s) = pick(&mut rng, list).and_then(|id| styles.get(id)) {
                return Some(s);
            }
            // Pools may name styles another team has not added yet.
            let present: Vec<(String, f32)> = list.iter().filter(|(id, _)| styles.get(id).is_some()).cloned().collect();
            if let Some(s) = pick(&mut rng, &present).and_then(|id| styles.get(id)) {
                return Some(s);
            }
        }
        let city: Vec<&Style> = styles.of_city(&z.city).collect();
        if !city.is_empty() {
            return Some(city[rng.below(city.len())]);
        }
    }
    styles.styles.first()
}

/// Floodlights for the plaza fronts of a lot (`top`: the absolute eave
/// height).
fn floodlight_lot(out: &mut CityOut, lot: &crate::lot::Lot, top: f32) {
    let opts = FloodOptions::default();
    for i in 0..lot.ring.len() {
        if !lot.edges[i].is_street() || !lot.front(i).open {
            continue;
        }
        let (a, b) = lot.edge(i);
        let d = b - a;
        if d.length() < 3.0 {
            continue;
        }
        let outn = Vec3::new(d.y, 0.0, -d.x).normalize_or(Vec3::Z);
        let (pa, pb) = (Vec3::new(a.x, lot.base, a.y), Vec3::new(b.x, lot.base, b.y));
        out.city_lights.extend(nightlight::facade_floodlights(pa, pb, outn, top - lot.base, &opts));
    }
}

fn frame(it: &Item) -> Mat4 {
    Mat4::from_rotation_translation(Quat::from_rotation_y(it.yaw), it.pos)
}

fn place(lay: &mut CityLayout, m: usize, it: &Item) {
    lay.out.placements.push((m, frame(it)));
}

/// Turns planned items into module placements, lights and props.
fn place_items(lib: &mut ModuleLib, lay: &mut CityLayout, items: &[Item], seed: u64) {
    let night = lib.kit.night.is_night();
    for it in items {
        let p = it.pos;
        match it.kind {
            ItemKind::Lamp { candelabra } => {
                // Sodium or LED by lamp; at night a module tinted with it.
                let bulb = p + Vec3::Y * furniture::street_lamp_bulb(candelabra);
                let (color, _) = nightlight::street_lamp_color(bulb, !candelabra);
                lay.out.city_lights.push(nightlight::street_lamp(bulb, color, candelabra));
                let m = if night {
                    let g = nightlight::to_rgb(color);
                    lib.named(&mut lay.out, &format!("street_lamp_{candelabra}_{g:?}"), || furniture::street_lamp_lit(candelabra, g))
                } else {
                    lib.named(&mut lay.out, &format!("street_lamp_{candelabra}"), || furniture::street_lamp(candelabra))
                };
                place(lay, m, it);
                let h = if candelabra { 4.4 } else { 3.6 };
                lay.out.lights.push((p + Vec3::Y * h, Vec3::new(1.0, 0.82, 0.58)));
            }
            ItemKind::RoadLight => {
                // Modern LED road lights (3000 K).
                let led = nightlight::kelvin(3000.0);
                let m = if night {
                    let g = nightlight::to_rgb(led);
                    lib.named(&mut lay.out, "road_light_lit", || furniture::road_light_lit(g))
                } else {
                    lib.named(&mut lay.out, "road_light", furniture::road_light)
                };
                place(lay, m, it);
                let fwd = Vec3::new(it.yaw.sin(), 0.0, it.yaw.cos());
                lay.out.lights.push((p + Vec3::Y * 9.4 + fwd * 1.8, Vec3::new(1.0, 0.92, 0.8)));
                lay.out.city_lights.push(nightlight::road_light(frame(it).transform_point3(furniture::ROAD_LIGHT_HEAD) - Vec3::Y * 0.05, led));
            }
            ItemKind::TrafficLight { aspect, ped_walk, arm } => {
                // Every state is registered, so the game can show the one
                // the drivers obey. The pedestrian head faces across the
                // road (+X local), towards the far kerb.
                let ped_yaw = std::f32::consts::FRAC_PI_2;
                let mut variants = [[0usize; 2]; 3];
                for (i, a) in [Aspect::Red, Aspect::Amber, Aspect::Green].into_iter().enumerate() {
                    for (k, walk) in [false, true].into_iter().enumerate() {
                        variants[i][k] = lib.named(&mut lay.out, &format!("tl_{a:?}_{walk}_{arm}"), || furniture::traffic_light(a, walk, ped_yaw, arm));
                    }
                }
                let idx = match aspect {
                    Aspect::Red => 0,
                    Aspect::Amber => 1,
                    Aspect::Green => 2,
                };
                lay.traffic.signals.push(SignalPost { pos: Vec2::new(p.x, p.z), facing: facing_of_yaw(it.yaw), placement: lay.out.placements.len(), variants });
                place(lay, variants[idx][ped_walk as usize], it);
                if night {
                    let at = frame(it);
                    lay.out.city_lights.extend(furniture::signal_lights(aspect, ped_walk, ped_yaw, arm).into_iter().map(|mut l| {
                        l.pos = at.transform_point3(l.pos);
                        if let Some(s) = &mut l.spot {
                            s.dir = at.transform_vector3(s.dir);
                        }
                        l
                    }));
                }
            }
            ItemKind::Sign(kind) => {
                let h = if kind == crate::texgen::SignKind::Crossing { 2.6 } else { 2.3 };
                let m = lib.named(&mut lay.out, &format!("sign_{kind:?}"), || furniture::sign(kind, h));
                place(lay, m, it);
                lay.traffic.signs.push(TrafficSign { kind, pos: Vec2::new(p.x, p.z), facing: facing_of_yaw(it.yaw) });
            }
            ItemKind::Plate(n) => {
                let m = lib.named(&mut lay.out, &format!("plate_{n}"), || furniture::street_plate(n));
                place(lay, m, it);
            }
            ItemKind::Bollard { valencia } => {
                let m = lib.named(&mut lay.out, &format!("bollard_{valencia}"), || furniture::bollard(valencia));
                place(lay, m, it);
            }
            ItemKind::Bin { valencia } => {
                let (name, model) = if valencia { ("papelera_double", BinModel::PostDouble) } else { ("papelera_single", BinModel::PostSingle) };
                let m = lib.named(&mut lay.out, name, || street_waste::papelera(model, 0));
                place(lay, m, it);
            }
            ItemKind::Container(k) => {
                let m = lib.named(&mut lay.out, &format!("container_{k}"), || furniture::container(k));
                place(lay, m, it);
            }
            ItemKind::Tree => {
                let m = lib.named(&mut lay.out, "tree_grate", furniture::tree_grate);
                place(lay, m, it);
                let mut rng = Rng::keyed(seed, stage::DRESS, crate::geom::point_key(Vec2::new(p.x, p.z)));
                lay.props.push((PropKind::Tree, Mat4::from_rotation_translation(Quat::from_rotation_y(rng.range(0.0, std::f32::consts::TAU)), p - Vec3::Y * 0.01)));
            }
            ItemKind::Palm => lay.props.push((PropKind::Palm, Mat4::from_rotation_translation(Quat::from_rotation_y(it.yaw), p))),
            ItemKind::Bench => lay.props.push((PropKind::Bench, Mat4::from_rotation_translation(Quat::from_rotation_y(it.yaw), p))),
            ItemKind::Fountain(r) => {
                let r = (r * 2.0).round() * 0.5;
                let m = lib.named(&mut lay.out, &format!("fountain_{r}"), || furniture::fountain(r));
                place(lay, m, it);
                if night {
                    lay.out.city_lights.extend(nightlight::fountain_lights(p + Vec3::Y * 0.4, r));
                }
            }
            ItemKind::Parterre(s) => {
                let s = s.round();
                let m = lib.named(&mut lay.out, &format!("parterre_{s}"), || furniture::parterre(s));
                place(lay, m, it);
                if night {
                    // The centrepiece lit from four points of the bed.
                    for q in 0..4 {
                        let o = Vec2::from_angle(std::f32::consts::FRAC_PI_2 * q as f32 + 0.8) * 1.6;
                        lay.out.city_lights.push(nightlight::accent(p + Vec3::new(o.x, 0.35, o.y), p + Vec3::Y * 2.0, 2800.0, 30.0));
                    }
                }
            }
            ItemKind::FlowerBed(r) => {
                let m = lib.named(&mut lay.out, &format!("flower_bed_{r}"), || {
                    let mut pp = Parts::new();
                    let ring: Vec<Vec2> = (0..16).map(|i| Vec2::from_angle(std::f32::consts::TAU * i as f32 / 16.0) * (r + 0.2)).collect();
                    pp.get(Mat::Soil).polygon(&ring, 0.05, true, 2.0);
                    furniture::flower_bed(&mut pp, Vec2::ZERO, r, 0.05, 5);
                    pp
                });
                place(lay, m, it);
            }
            ItemKind::Catenary { span, laundry } => {
                let span = (span * 4.0).round() / 4.0;
                let v = ((p.x * 7.0 + p.z * 3.0).abs() as u32 % 4) as u8;
                let m = lib.named(&mut lay.out, &format!("catenary_{span}_{laundry}_{v}"), || furniture::catenary(span, laundry, v));
                place(lay, m, it);
                if !laundry {
                    lay.out.lights.push((p - Vec3::Y * 0.7, Vec3::new(1.0, 0.78, 0.5)));
                }
            }
            ItemKind::Terrace(v) => {
                let m = lib.named(&mut lay.out, &format!("terrace_{v}"), || furniture::terrace_set(v));
                place(lay, m, it);
            }
        }
    }
}

/// A clear spot on the largest square of zone `zi`: open paving, away
/// from fountains, trees and benches.
fn square_spot(lay: &CityLayout, f: &Fabric, zi: u16) -> Option<Vec2> {
    let (lo, hi) = f.lots.iter().zip(&f.info).filter(|(_, i)| i.zone == zi).flat_map(|(l, _)| l.ring.iter()).fold((Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)), |(a, b), p| (a.min(*p), b.max(*p)));
    let inside = |p: Vec2| p.x > lo.x && p.y > lo.y && p.x < hi.x && p.y < hi.y;
    let (c, r) = f.plazas.components().into_iter().filter_map(|c| c.pole()).filter(|(p, _)| inside(*p)).max_by(|a, b| a.1.total_cmp(&b.1))?;
    let clear = |q: Vec2| f.ground.in_plaza(q) && lay.items.iter().all(|it| Vec2::new(it.pos.x, it.pos.z).distance(q) > 2.5);
    (0..24).map(|k| c + Vec2::from_angle(k as f32 * 0.9) * r * (0.45 + 0.02 * k as f32)).find(|q| clear(*q))
}

fn spawns(lay: &mut CityLayout, f: &Fabric, start: Option<u16>) {
    let g = &f.graph;
    // The player: on the start zone's main square, else on a sidewalk or
    // plaza near the middle of the map.
    let centre = (f.extent[0] + f.extent[1]) * 0.5;
    let mut player = start.and_then(|zi| square_spot(lay, f, zi)).map(|q| Spawn { vehicle: false, pos: Vec3::new(q.x, f.ground.height_at(q) + 1.05, q.y), yaw: 0.0 });
    for (si, _, _) in g.near(centre, 400.0) {
        if player.is_some() {
            break;
        }
        let s = g.street(si);
        for k in 0..10 {
            let t = s.len() * (0.3 + 0.04 * k as f32);
            let n = s.line.normal(t);
            let q = s.line.at(t) + n * (s.carriageway * 0.5 + 1.0).min(s.width_at(t) * 0.5 - 0.8).max(0.0);
            if f.ground.surface_at(q).is_some_and(|x| x.h > 0.05) || (s.carriageway <= 0.0 && f.public.contains(q)) {
                player = Some(Spawn { vehicle: false, pos: Vec3::new(q.x, f.ground.height_at(q) + 1.05, q.y), yaw: 0.0 });
                break;
            }
        }
        if player.is_some() {
            break;
        }
    }
    lay.spawns.push(player.unwrap_or(Spawn { vehicle: false, pos: Vec3::new(centre.x, 1.2, centre.y), yaw: 0.0 }));
    // Cars: parked in the strip on the right of the traffic, halfway along
    // every third street with parking (the lanes belong to the traffic AI).
    let mut cars = Vec::new();
    for st in lay.traffic.streets.iter().filter(|s| !s.parking.is_empty()).step_by(3) {
        let line = crate::plan::Polyline::new(st.centreline());
        let t = line.len() * 0.5;
        let Some(lane) = st.lanes.first() else { continue };
        // The strip on the right of the lane's travel.
        let right_of = |o: f32| if lane.forward { -o } else { o };
        let Some(&offset) = st.parking.iter().max_by(|a, b| right_of(**a).total_cmp(&right_of(**b))) else { continue };
        let q = line.at(t) + line.normal(t) * offset;
        let fwd = line.tangent(t) * if lane.forward { 1.0 } else { -1.0 };
        if f.ground.in_carriage(q) {
            cars.push(Spawn { vehicle: true, pos: Vec3::new(q.x, f.ground.height_at(q) + 0.8, q.y), yaw: fwd.x.atan2(fwd.y) });
        }
    }
    lay.spawns.extend(cars);
}

/// The traffic description of the street graph: every roadway between
/// its junction mouths with the lanes and parking strips its markings
/// paint, the junctions themselves, plazas and benches for pedestrians,
/// and the ground height for vehicles on slopes. Signs and lights were
/// recorded as they were placed.
fn traffic_info(lay: &mut CityLayout, f: &Fabric, plan: &CityPlan, lib: &ProfileLibrary) {
    let g = &f.graph;
    // One traffic node per junction (nodes joined by links are one), at
    // the mean of its nodes.
    let rep = g.junctions(lib);
    let mut node_of: BTreeMap<crate::plan::NodeId, usize> = BTreeMap::new();
    let mut node = |n: crate::plan::NodeId| {
        let k = node_of.len();
        *node_of.entry(rep[n as usize]).or_insert(k)
    };
    let mut streets = Vec::new();
    for (si, s) in g.streets.iter().enumerate() {
        let p = lib.get(s.profile);
        let si = si as crate::plan::StreetId;
        if !p.vehicles || s.carriageway <= 0.0 || p.class == Class::Limit || s.len() < 1.0 || rep[s.a as usize] == rep[s.b as usize] {
            continue;
        }
        // From kerb corner to kerb corner (where turns end); the zebras
        // and stop lines are painted from the junction mouths beyond. A
        // link between two junctions too far apart to be one keeps a metre
        // around its middle.
        let (k0, k1) = (g.kerb_mouth(si, End::A, lib), s.len() - g.kerb_mouth(si, End::B, lib));
        let (m0, m1) = if k1 - k0 >= 1.0 { (k0, k1) } else { (((k0 + k1) * 0.5 - 0.5).max(0.0), ((k0 + k1) * 0.5 + 0.5).min(s.len())) };
        let end = |e: End, from: f32| {
            // `ground::markings`: zebra 1-5 m past the mouth, stop line at 6 m.
            let extra = (g.mouth(si, e, lib) - from).max(0.0);
            if ground::has_zebra(g, lib, si, e) { TrafficEnd { crosswalk: Some(extra + 3.0), stop_line: extra + 6.0 } } else { TrafficEnd { crosswalk: None, stop_line: extra + 1.0 } }
        };
        let ends = [end(End::A, m0), end(End::B, s.len() - m1)];
        let points = s.line.sub(m0, m1).pts().to_vec();
        let road = s.carriageway * 0.5;
        let half = s.width_at((m0 + m1) * 0.5) * 0.5;
        let valencia = plan.zone(s.zone).is_some_and(|z| z.valencia());
        streets.push(TrafficStreet {
            a: points[0],
            b: *points.last().expect("points"),
            half_width: half,
            road_half: road,
            // Kerbs too narrow to walk along carry no pedestrians.
            sidewalk: Some(half - road).filter(|w| *w >= 1.2).unwrap_or(0.0),
            median_half: p.median * 0.5,
            lanes: lanes(p, road, s.oneway),
            parking: parking(p).iter().map(|side| side * (road - PARK * 0.5)).collect(),
            speed_kmh: p.speed.map(f32::from).unwrap_or(if valencia { 50.0 } else { 30.0 }),
            crosswalk: p.crossings.then_some(3.0),
            stop_line: 6.0,
            ends: Some(ends),
            nodes: Some([node(s.a), node(s.b)]),
            points,
        });
    }
    let mut sum = vec![(Vec2::ZERO, 0.0f32); node_of.len()];
    for (n, nd) in g.nodes.iter().enumerate() {
        if let Some(&i) = node_of.get(&rep[n]) {
            sum[i].0 += nd.p;
            sum[i].1 += 1.0;
        }
    }
    let t = &mut lay.traffic;
    t.streets = streets;
    t.nodes = sum.into_iter().map(|(p, k)| p / k).collect();
    // Raised medians open wherever another roadway crosses them.
    t.median_openings = g.nodes.iter().enumerate().filter(|(n, _)| {
        let star = g.star(*n as crate::plan::NodeId);
        star.iter().any(|a| lib.get(g.street(a.0).profile).median > 0.0) && star.iter().filter(|a| lib.get(g.street(a.0).profile).vehicles && g.street(a.0).carriageway > 0.0).count() >= 3
    }).map(|(_, n)| n.p).collect();
    // Squares as the box of their inscribed circle (the walkers' graph
    // links its centre to the kerbs around).
    t.plazas = f.ground.plaza.components().iter().filter_map(|c| c.pole()).filter(|p| p.1 > 4.0).map(|(c, r)| (c - Vec2::splat(r * 0.7), c + Vec2::splat(r * 0.7))).collect();
    t.benches = lay.props.iter().filter(|p| p.0 == PropKind::Bench).map(|(_, m)| {
        let (_, q, tr) = m.to_scale_rotation_translation();
        let fwd = q * Vec3::Z;
        (Vec2::new(tr.x, tr.z), fwd.x.atan2(fwd.z))
    }).collect();
    if !f.ground.relief.is_flat() {
        let [lo, hi] = f.extent;
        t.ground = Some(HeightGrid::sample(lo - Vec2::splat(4.0), hi + Vec2::splat(4.0), 2.0, |q| f.ground.relief.elev(q)));
    }
}

/// Width of a parking strip.
const PARK: f32 = 2.2;

/// Kerb sides with a parking strip (-1 right of the line, +1 left).
fn parking(p: &crate::profile::Profile) -> &'static [f32] {
    match p.parking {
        crate::profile::Parking::None => &[],
        crate::profile::Parking::One => &[-1.0],
        crate::profile::Parking::Both => &[-1.0, 1.0],
    }
}

/// Driving lanes of a roadway `2 road` wide (offsets left of the line
/// positive; forward lanes run along it and keep to the right): the
/// profile's lanes each way beside a median, or between the parking
/// strips, all one way on a one-way street.
fn lanes(p: &crate::profile::Profile, road: f32, oneway: i8) -> Vec<TrafficLane> {
    let n = p.lanes.max(1) as usize;
    let band = |lo: f32, hi: f32, k: usize, forward: bool| -> Vec<TrafficLane> {
        let w = (hi - lo) / k as f32;
        (0..k).map(|i| TrafficLane { offset: lo + w * (i as f32 + 0.5), forward, width: w }).filter(|l| l.width >= 2.0).collect()
    };
    if p.median > 0.0 {
        // `ground::markings`, Avenue: lanes between the median edge line
        // and the kerb line on each side.
        let inner = p.median * 0.5 + 0.3;
        let mut v = band(-(road - 0.3), -inner, n, true);
        v.extend(band(inner, road - 0.3, n, false));
        return v;
    }
    let sides = parking(p);
    let lo = -road + if sides.contains(&-1.0) { PARK } else { 0.3 };
    let hi = road - if sides.contains(&1.0) { PARK } else { 0.3 };
    match oneway {
        0 => {
            let k = (n / 2).max(1);
            let mut v = band(lo, (lo + hi) * 0.5, k, true);
            v.extend(band((lo + hi) * 0.5, hi, k, false));
            v
        }
        d => band(lo, hi, n, d > 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nightlight::LightKind;

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
        assert!(count(LightKind::Street) > 0 && count(LightKind::Wall) > 0);
        assert!(night.out.city_lights.len() > day.out.city_lights.len() * 3);
    }

    #[test]
    fn terraces_stay_on_the_sidewalk() {
        let fr = crate::building::Frontage { pos: Vec3::new(0.0, 0.15, 0.0), right: Vec3::X, out: Vec3::Z, width: 3.0, bar: true, closed: false };
        let kit = Kit::load(None);
        let mut lib = ModuleLib::new(&kit);
        let mut lay = CityLayout::default();
        for seed in 0..8 {
            let mut items = Vec::new();
            dress::terrace(&fr, 4.5, &mut Rng::new(seed), &mut items);
            place_items(&mut lib, &mut lay, &items, seed);
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
