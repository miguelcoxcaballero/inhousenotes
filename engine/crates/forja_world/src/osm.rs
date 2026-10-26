//! OpenStreetMap importer: real streets and buildings as a drivable city.
//!
//! Nodes are projected to meters around the bounding-box center
//! (equirectangular, accurate to centimetres at city scale). +X is east and
//! -Z is north so the map reads correctly from above in a right-handed,
//! Y-up world.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use forja_assets::{MaterialData, NodeData, SceneData};
use forja_math::{Iso, Quat, Vec2, Vec3, Vec4};
use forja_physics::{ColliderDesc, CollisionGroups, Shape, TriMesh, layers};
use quick_xml::events::Event;

use crate::builder::{MeshBuilder, signed_area};
use crate::library::ContentLibrary;
use crate::{SpawnKind, SpawnPoint, WorldDescription, WorldError, WorldSource};

#[derive(Clone, Debug, Default)]
pub struct OsmData {
    pub nodes: HashMap<i64, (f64, f64)>,
    pub ways: Vec<OsmWay>,
    pub bounds: Option<[f64; 4]>,
}

#[derive(Clone, Debug, Default)]
pub struct OsmWay {
    pub refs: Vec<i64>,
    pub tags: HashMap<String, String>,
}

impl OsmWay {
    pub fn tag(&self, k: &str) -> Option<&str> {
        self.tags.get(k).map(String::as_str)
    }
}

pub fn parse_osm(xml: &str) -> Result<OsmData, WorldError> {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut data = OsmData::default();
    let mut current: Option<OsmWay> = None;
    let attr = |e: &quick_xml::events::BytesStart, key: &[u8]| -> Option<String> {
        e.attributes().flatten().find(|a| a.key.as_ref() == key).and_then(|a| a.unescape_value().ok()).map(|v| v.into_owned())
    };
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => match e.name().as_ref() {
                b"bounds" => {
                    let g = |k: &[u8]| attr(&e, k).and_then(|v| v.parse::<f64>().ok());
                    if let (Some(a), Some(b), Some(c), Some(d)) = (g(b"minlon"), g(b"minlat"), g(b"maxlon"), g(b"maxlat")) {
                        data.bounds = Some([a, b, c, d]);
                    }
                }
                b"node" => {
                    let id = attr(&e, b"id").and_then(|v| v.parse().ok());
                    let lat = attr(&e, b"lat").and_then(|v| v.parse().ok());
                    let lon = attr(&e, b"lon").and_then(|v| v.parse().ok());
                    if let (Some(id), Some(lat), Some(lon)) = (id, lat, lon) {
                        data.nodes.insert(id, (lon, lat));
                    }
                }
                b"way" => current = Some(OsmWay::default()),
                b"nd" => {
                    if let (Some(w), Some(r)) = (current.as_mut(), attr(&e, b"ref").and_then(|v| v.parse().ok())) {
                        w.refs.push(r);
                    }
                }
                b"tag" => {
                    if let (Some(w), Some(k), Some(v)) = (current.as_mut(), attr(&e, b"k"), attr(&e, b"v")) {
                        w.tags.insert(k, v);
                    }
                }
                _ => {}
            },
            Ok(Event::End(e)) if e.name().as_ref() == b"way" => {
                if let Some(w) = current.take() {
                    data.ways.push(w);
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(WorldError::Parse(format!("osm xml at {}: {e}", reader.buffer_position()))),
            _ => {}
        }
    }
    Ok(data)
}

/// Road half-width in meters by `highway` class.
fn road_half_width(class: &str) -> Option<f32> {
    Some(match class {
        "motorway" | "trunk" => 7.5,
        "primary" => 6.5,
        "secondary" => 5.5,
        "tertiary" => 4.5,
        "residential" | "unclassified" | "living_street" => 3.75,
        "service" => 2.5,
        "footway" | "pedestrian" | "path" | "cycleway" => 1.25,
        _ => return None,
    })
}

fn building_height(w: &OsmWay) -> f32 {
    if let Some(h) = w.tag("height").and_then(|h| h.trim_end_matches(" m").trim_end_matches('m').trim().parse::<f32>().ok()) {
        return h.clamp(2.5, 400.0);
    }
    if let Some(l) = w.tag("building:levels").and_then(|l| l.parse::<f32>().ok()) {
        return (l * 3.2 + 1.0).clamp(3.0, 400.0);
    }
    match w.tag("building").unwrap_or("yes") {
        "house" | "detached" | "garage" | "shed" | "hut" => 6.0,
        "hotel" => 24.0,
        "apartments" | "commercial" | "office" => 15.0,
        "retail" => 6.0,
        _ => 9.0,
    }
}

/// Miami pastel palette, picked per building id for variety.
const FACADES: [[f32; 3]; 8] = [
    [0.93, 0.62, 0.72], // flamingo pink
    [0.47, 0.82, 0.80], // turquoise
    [0.97, 0.90, 0.62], // lemon
    [0.95, 0.95, 0.92], // art deco white
    [0.70, 0.78, 0.93], // sky blue
    [0.98, 0.75, 0.55], // peach
    [0.75, 0.90, 0.70], // mint
    [0.86, 0.80, 0.95], // lavender
];

pub struct OsmWorld {
    pub path: std::path::PathBuf,
    /// Photoreal materials and props (`assets/downloaded`), optional.
    pub content: Option<std::path::PathBuf>,
}

impl OsmWorld {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self { path: path.as_ref().to_path_buf(), content: None }
    }

    pub fn with_content(mut self, downloaded: impl AsRef<Path>) -> Self {
        self.content = Some(downloaded.as_ref().to_path_buf());
        self
    }

    pub fn build_from(data: &OsmData) -> Result<WorldDescription, WorldError> {
        Self::build_with(data, None)
    }

    pub fn build_with(data: &OsmData, lib: Option<&ContentLibrary>) -> Result<WorldDescription, WorldError> {
        let [min_lon, min_lat, max_lon, max_lat] = data.bounds.unwrap_or_else(|| {
            let (mut a, mut b, mut c, mut d) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for (lon, lat) in data.nodes.values() {
                a = a.min(*lon);
                b = b.min(*lat);
                c = c.max(*lon);
                d = d.max(*lat);
            }
            [a, b, c, d]
        });
        let (lon0, lat0) = ((min_lon + max_lon) * 0.5, (min_lat + max_lat) * 0.5);
        const R: f64 = 6_378_137.0;
        let kx = R * lat0.to_radians().cos() * std::f64::consts::PI / 180.0;
        let ky = R * std::f64::consts::PI / 180.0;
        let project = |id: &i64| data.nodes.get(id).map(|(lon, lat)| Vec2::new(((lon - lon0) * kx) as f32, (-(lat - lat0) * ky) as f32));
        let half = Vec2::new(((max_lon - min_lon) * kx * 0.5) as f32, ((max_lat - min_lat) * ky * 0.5) as f32);

        let mut scene = SceneData { name: "osm".into(), ..Default::default() };
        let mat = |name: &str, c: [f32; 3], rough: f32| MaterialData {
            name: name.into(),
            base_color: Vec4::new(c[0], c[1], c[2], 1.0),
            roughness: rough,
            ..Default::default()
        };
        // Scanned PBR sets when available, flat colors otherwise. Indices
        // 0..=6 are fixed; facades follow.
        let pbr = |scene: &mut SceneData, id: &str, fallback: MaterialData, tint: Vec4| -> (usize, bool) {
            match lib.and_then(|l| l.material(id)) {
                Some(set) => (set.add_to(scene, tint, false), true),
                None => {
                    scene.materials.push(fallback);
                    (scene.materials.len() - 1, false)
                }
            }
        };
        let (_, textured_ground) = pbr(&mut scene, "pbr_grass", mat("ground", [0.42, 0.45, 0.36], 0.95), Vec4::new(0.9, 0.95, 0.85, 1.0)); // 0
        pbr(&mut scene, "pbr_asphalt", mat("asphalt", [0.10, 0.10, 0.11], 0.85), Vec4::ONE); // 1
        pbr(&mut scene, "pbr_sidewalk", mat("sidewalk", [0.62, 0.60, 0.56], 0.9), Vec4::ONE); // 2
        pbr(&mut scene, "pbr_sand", mat("sand", [0.87, 0.80, 0.62], 0.95), Vec4::ONE); // 3
        pbr(&mut scene, "pbr_grass", mat("grass", [0.25, 0.45, 0.18], 0.95), Vec4::ONE); // 4
        scene.materials.push(MaterialData { roughness: 0.05, metallic: 0.0, ..mat("water", [0.05, 0.25, 0.32], 0.05) }); // 5
        pbr(&mut scene, "pbr_roof", mat("roof", [0.55, 0.55, 0.57], 0.8), Vec4::new(0.85, 0.85, 0.85, 1.0)); // 6
        let facade0 = scene.materials.len();
        let facade_sets = ["pbr_facade_classic", "pbr_facade_brick", "pbr_facade_stone", "pbr_facade_office", "pbr_facade_glass"];
        let mut facade_scale = Vec::new();
        for (i, c) in FACADES.iter().enumerate() {
            // Pastel tints over the scanned facades keep the Miami palette.
            let tint = Vec4::new(0.55 + 0.45 * c[0], 0.55 + 0.45 * c[1], 0.55 + 0.45 * c[2], 1.0);
            let (_, textured) = pbr(&mut scene, facade_sets[i % facade_sets.len()], mat(&format!("facade_{i}"), *c, 0.7), tint);
            facade_scale.push(if textured { 9.0 } else { 3.2 });
        }
        let tile = if textured_ground { 6.0 } else { 4.0 };

        let mut ground = MeshBuilder::new(0);
        let g = half * 1.2;
        ground.quad(Vec3::new(-g.x, 0.0, g.y), Vec3::new(g.x, 0.0, g.y), Vec3::new(g.x, 0.0, -g.y), Vec3::new(-g.x, 0.0, -g.y), tile);
        let mut areas: Vec<MeshBuilder> = (3..=5).map(MeshBuilder::new).collect();
        let mut roads = MeshBuilder::new(1);
        let mut paths = MeshBuilder::new(2);
        let mut roofs = MeshBuilder::new(6);
        let mut facades: Vec<MeshBuilder> = (0..FACADES.len()).map(|i| MeshBuilder::new(facade0 + i)).collect();
        let mut col_verts: Vec<Vec3> = Vec::new();
        let mut col_tris: Vec<[u32; 3]> = Vec::new();
        let mut spawns = Vec::new();
        let mut lamps = Vec::new();
        let mut stats = (0usize, 0usize);
        let mut footprints: Vec<(forja_math::Aabb, Vec<Vec2>)> = Vec::new();
        let mut prop_spots: Vec<(Vec2, Vec2)> = Vec::new();

        for (wi, w) in data.ways.iter().enumerate() {
            let pts: Vec<Vec2> = w.refs.iter().filter_map(project).collect();
            if pts.len() < 2 {
                continue;
            }
            let closed = w.refs.len() > 3 && w.refs.first() == w.refs.last();
            if w.tags.contains_key("building") && closed {
                let mut ring = pts[..pts.len() - 1].to_vec();
                if signed_area(&ring).abs() < 4.0 {
                    continue;
                }
                if signed_area(&ring) < 0.0 {
                    ring.reverse();
                }
                let h = building_height(w);
                footprints.push((forja_math::Aabb::from_points(ring.iter().map(|p| Vec3::new(p.x, 0.0, p.y))), ring.clone()));
                let fi = wi % FACADES.len();
                let fscale = facade_scale[fi];
                let fb = &mut facades[fi];
                for i in 0..ring.len() {
                    let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
                    // Ring is CCW in (x, z); walls face outward with this order.
                    let (p0, p1) = (Vec3::new(b.x, 0.0, b.y), Vec3::new(a.x, 0.0, a.y));
                    let up = Vec3::Y * h;
                    fb.quad(p0, p1, p1 + up, p0 + up, fscale);
                    let base = col_verts.len() as u32;
                    col_verts.extend([p0, p1, p1 + up, p0 + up]);
                    col_tris.extend([[base, base + 1, base + 2], [base, base + 2, base + 3]]);
                }
                let before = roofs.mesh.indices.len();
                if roofs.polygon(&ring, h, true, tile) {
                    let start = roofs.mesh.positions.len() as u32 - ring.len() as u32;
                    let base = col_verts.len() as u32;
                    col_verts.extend(ring.iter().map(|p| Vec3::new(p.x, h, p.y)));
                    for t in roofs.mesh.indices[before..].chunks_exact(3) {
                        col_tris.push([base + t[0] - start, base + t[1] - start, base + t[2] - start]);
                    }
                }
                stats.0 += 1;
                continue;
            }
            if let Some(hw) = w.tag("highway").and_then(road_half_width) {
                let is_path = hw < 2.0;
                let (builder, y) = if is_path { (&mut paths, 0.03) } else { (&mut roads, 0.02) };
                ribbon(builder, &pts, hw, y);
                if !is_path {
                    lamps_along(&pts, hw + 1.0, 35.0, &mut lamps);
                    // Sidewalks on both sides of every car road.
                    side_ribbon(&mut paths, &pts, hw, hw + 2.4, 0.025);
                    side_ribbon(&mut paths, &pts, -(hw + 2.4), -hw, 0.025);
                    spots_along(&pts, hw + 1.4, 24.0, &mut prop_spots);
                }
                if !is_path {
                    stats.1 += 1;
                    if pts.len() >= 2 && wi % 3 == 0 {
                        let (a, b) = (pts[0], pts[1]);
                        let d = (b - a).normalize_or(Vec2::X);
                        let p = a + d * 3.0;
                        spawns.push(SpawnPoint {
                            kind: SpawnKind::Vehicle,
                            iso: Iso::new(Vec3::new(p.x, 1.0, p.y), Quat::from_rotation_y(d.x.atan2(d.y))),
                        });
                    }
                }
                continue;
            }
            if closed {
                let ring = &pts[..pts.len() - 1];
                let natural = w.tag("natural").unwrap_or("");
                let landuse = w.tag("landuse").unwrap_or("");
                let leisure = w.tag("leisure").unwrap_or("");
                let target = if natural == "beach" || natural == "sand" {
                    Some((0, 0.005))
                } else if natural == "water" || w.tags.contains_key("water") {
                    Some((2, 0.01))
                } else if ["grass", "park", "garden", "pitch"].contains(&landuse) || ["park", "garden", "pitch"].contains(&leisure) {
                    Some((1, 0.008))
                } else {
                    None
                };
                if let Some((k, y)) = target {
                    areas[k].polygon(ring, y, true, tile);
                }
            }
        }
        // Player spawn: closest road spawn to the center.
        let center_spawn = spawns
            .iter()
            .min_by(|a, b| a.iso.pos.length_squared().total_cmp(&b.iso.pos.length_squared()))
            .map(|s| Iso::from_pos(s.iso.pos + Vec3::Y));
        spawns.insert(0, SpawnPoint { kind: SpawnKind::Player, iso: center_spawn.unwrap_or(Iso::from_pos(Vec3::Y * 2.0)) });

        let mut prims = vec![ground.finish()];
        for b in areas.into_iter().chain([roads, paths, roofs]).chain(facades) {
            if !b.is_empty() {
                prims.push(b.finish());
            }
        }
        scene.meshes.push(prims);
        scene.nodes.push(NodeData { name: "osm".into(), mesh: Some(0), ..Default::default() });
        scene.roots.push(0);

        let mut world = WorldDescription::new("OpenStreetMap");
        world.attribution.push("Map data © OpenStreetMap contributors, ODbL 1.0".into());
        world.add_scene(Arc::new(scene), forja_math::Mat4::IDENTITY);
        world.colliders.push((
            ColliderDesc::new(Shape::cuboid(Vec3::new(g.x, 1.0, g.y))).translation(Vec3::new(0.0, -1.0, 0.0)).friction(1.0).groups(CollisionGroups::new(layers::WORLD, u32::MAX)),
            Iso::IDENTITY,
        ));
        if !col_tris.is_empty() {
            world.colliders.push((
                ColliderDesc::new(Shape::TriMesh(Arc::new(TriMesh::new(col_verts, col_tris)))).friction(0.8).groups(CollisionGroups::new(layers::WORLD, u32::MAX)),
                Iso::IDENTITY,
            ));
        }
        if let Some(lib) = lib {
            place_props(&mut world, lib, &prop_spots, &footprints);
        }
        world.spawns = spawns;
        world.lights = lamps;
        world.extent = g;
        log::info!("osm: {} buildings, {} roads, {} props", stats.0, stats.1, world.stats.iter().find(|s| s.0 == "props").map_or(0, |s| s.1));
        world.stats.push(("buildings".into(), stats.0));
        world.stats.push(("roads".into(), stats.1));
        Ok(world)
    }
}

/// Street lamps every `spacing` meters, offset to the road side.
fn lamps_along(pts: &[Vec2], side: f32, spacing: f32, out: &mut Vec<(Vec3, Vec3)>) {
    let mut carry = spacing * 0.5;
    for w in pts.windows(2) {
        let d = w[1] - w[0];
        let len = d.length();
        if len < 1e-3 {
            continue;
        }
        let dir = d / len;
        let normal = Vec2::new(-dir.y, dir.x);
        let mut t = carry;
        while t < len {
            let p = w[0] + dir * t + normal * side;
            out.push((Vec3::new(p.x, 6.0, p.y), Vec3::new(1.0, 0.75, 0.45)));
            t += spacing;
        }
        carry = t - len;
    }
}

/// Points every `spacing` meters at `side` meters to the right of the line,
/// with the line direction.
fn spots_along(pts: &[Vec2], side: f32, spacing: f32, out: &mut Vec<(Vec2, Vec2)>) {
    let mut carry = spacing * 0.5;
    for w in pts.windows(2) {
        let d = w[1] - w[0];
        let len = d.length();
        if len < 1e-3 {
            continue;
        }
        let dir = d / len;
        let normal = Vec2::new(-dir.y, dir.x);
        let mut t = carry;
        while t < len {
            out.push((w[0] + dir * t + normal * side, dir));
            out.push((w[0] + dir * t - normal * side, -dir));
            t += spacing;
        }
        carry = t - len;
    }
}

fn point_in_ring(p: Vec2, ring: &[Vec2]) -> bool {
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

/// Palms, hydrants and trash cans on the sidewalks, never inside buildings.
fn place_props(world: &mut WorldDescription, lib: &ContentLibrary, spots: &[(Vec2, Vec2)], footprints: &[(forja_math::Aabb, Vec<Vec2>)]) {
    let kinds: Vec<(usize, f32, Shape, f32)> = [
        ("polyhaven_island_tree", 1.7, Shape::capsule(2.0, 0.28), 2.3),
        ("polyhaven_fire_hydrant", 1.0, Shape::cuboid(Vec3::new(0.2, 0.4, 0.2)), 0.4),
        ("polyhaven_trash_can", 1.0, Shape::cuboid(Vec3::new(0.3, 0.45, 0.3)), 0.45),
    ]
    .into_iter()
    .filter_map(|(id, scale, shape, cy)| lib.prop(id).map(|s| (world.register_scene(s), scale, shape, cy)))
    .collect();
    if kinds.is_empty() {
        return;
    }
    let mut placed: Vec<Vec2> = Vec::new();
    let mut seed = 0x2545_f491u32;
    for (i, (p, dir)) in spots.iter().enumerate() {
        if placed.iter().any(|q| (*q - *p).length_squared() < 36.0) {
            continue;
        }
        let blocked = footprints.iter().any(|(b, ring)| {
            p.x >= b.min.x - 1.0 && p.x <= b.max.x + 1.0 && p.y >= b.min.z - 1.0 && p.y <= b.max.z + 1.0 && point_in_ring(*p, ring)
        });
        if blocked {
            continue;
        }
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        // Mostly palms, with street furniture in between.
        let k = if i % 5 == 1 && kinds.len() > 1 { 1 } else if i % 7 == 3 && kinds.len() > 2 { 2 } else { 0 };
        let (scene, scale, shape, cy) = &kinds[k];
        let s = scale * (0.85 + (seed % 100) as f32 / 330.0);
        let yaw = dir.x.atan2(dir.y) + if k == 0 { (seed % 628) as f32 / 100.0 } else { 0.0 };
        let pos = Vec3::new(p.x, 0.02, p.y);
        world.place(*scene, forja_math::Mat4::from_scale_rotation_translation(Vec3::splat(s), Quat::from_rotation_y(yaw), pos));
        world.colliders.push((
            ColliderDesc::new(shape.clone()).groups(CollisionGroups::new(layers::WORLD, u32::MAX)),
            Iso::new(pos + Vec3::Y * cy * s, Quat::from_rotation_y(yaw)),
        ));
        placed.push(*p);
    }
    world.stats.push(("props".into(), placed.len()));
}

/// Strip between signed offsets `a` and `b` (meters, positive = left).
fn side_ribbon(builder: &mut MeshBuilder, pts: &[Vec2], a: f32, b: f32, y: f32) {
    let offset = |off: f32| -> Vec<Vec2> {
        (0..pts.len())
            .map(|i| {
                let dir_in = if i > 0 { (pts[i] - pts[i - 1]).normalize_or_zero() } else { Vec2::ZERO };
                let dir_out = if i + 1 < pts.len() { (pts[i + 1] - pts[i]).normalize_or_zero() } else { Vec2::ZERO };
                let t = (dir_in + dir_out).normalize_or(if dir_out != Vec2::ZERO { dir_out } else { dir_in });
                pts[i] + Vec2::new(-t.y, t.x) * off
            })
            .collect()
    };
    let (la, lb) = (offset(a), offset(b));
    let mid: Vec<Vec2> = la.iter().zip(&lb).map(|(p, q)| (*p + *q) * 0.5).collect();
    ribbon(builder, &mid, (b - a).abs() * 0.5, y);
}

/// Flat strip along a polyline with mitred joints.
fn ribbon(b: &mut MeshBuilder, pts: &[Vec2], half: f32, y: f32) {
    let n = pts.len();
    let mut dist = 0.0;
    let mut prev: Option<(u32, u32)> = None;
    for i in 0..n {
        let dir_in = if i > 0 { (pts[i] - pts[i - 1]).normalize_or_zero() } else { Vec2::ZERO };
        let dir_out = if i + 1 < n { (pts[i + 1] - pts[i]).normalize_or_zero() } else { Vec2::ZERO };
        let tangent = (dir_in + dir_out).normalize_or(if dir_out != Vec2::ZERO { dir_out } else { dir_in });
        let normal = Vec2::new(-tangent.y, tangent.x);
        let miter = if dir_in != Vec2::ZERO && dir_out != Vec2::ZERO { 1.0 / normal.dot(Vec2::new(-dir_out.y, dir_out.x)).abs().max(0.4) } else { 1.0 };
        if i > 0 {
            dist += (pts[i] - pts[i - 1]).length();
        }
        let l = pts[i] + normal * half * miter;
        let r = pts[i] - normal * half * miter;
        let v = dist / (half * 4.0);
        let il = b.vertex(Vec3::new(l.x, y, l.y), Vec3::Y, Vec2::new(0.0, v));
        let ir = b.vertex(Vec3::new(r.x, y, r.y), Vec3::Y, Vec2::new(1.0, v));
        if let Some((pl, pr)) = prev {
            // Upward facing: pick the winding whose normal is +Y.
            let (a, c) = (b.mesh.positions[pl as usize], b.mesh.positions[ir as usize]);
            let bb = b.mesh.positions[pr as usize];
            if (bb - a).cross(c - a).y >= 0.0 {
                b.mesh.indices.extend([pl, pr, ir, pl, ir, il]);
            } else {
                b.mesh.indices.extend([pl, ir, pr, pl, il, ir]);
            }
        }
        prev = Some((il, ir));
    }
}

impl WorldSource for OsmWorld {
    fn name(&self) -> String {
        format!("OSM {}", self.path.display())
    }

    fn build(&self) -> Result<WorldDescription, WorldError> {
        let xml = std::fs::read_to_string(&self.path).map_err(|e| WorldError::Io(format!("{}: {e}", self.path.display())))?;
        let lib = self.content.as_ref().map(|c| ContentLibrary::load(c));
        Self::build_with(&parse_osm(&xml)?, lib.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0"?>
<osm version="0.6">
 <bounds minlat="25.7800" minlon="-80.1350" maxlat="25.7810" maxlon="-80.1340"/>
 <node id="1" lat="25.7802" lon="-80.1348"/>
 <node id="2" lat="25.7802" lon="-80.1346"/>
 <node id="3" lat="25.7804" lon="-80.1346"/>
 <node id="4" lat="25.7804" lon="-80.1348"/>
 <node id="5" lat="25.7800" lon="-80.1350"/>
 <node id="6" lat="25.7810" lon="-80.1340"/>
 <way id="10"><nd ref="1"/><nd ref="2"/><nd ref="3"/><nd ref="4"/><nd ref="1"/><tag k="building" v="hotel"/><tag k="building:levels" v="5"/></way>
 <way id="11"><nd ref="5"/><nd ref="6"/><tag k="highway" v="residential"/></way>
</osm>"#;

    #[test]
    fn builds_buildings_roads_and_colliders() {
        let data = parse_osm(SAMPLE).unwrap();
        assert_eq!(data.nodes.len(), 6);
        assert_eq!(data.ways.len(), 2);
        let w = OsmWorld::build_from(&data).unwrap();
        assert_eq!(w.stats[0], ("buildings".into(), 1));
        assert_eq!(w.colliders.len(), 2);
        let (_, scene, _) = &w.instances()[0];
        let roof_h = scene.meshes[0].iter().flat_map(|m| m.positions.iter()).map(|p| p.y).fold(0.0, f32::max);
        assert!((roof_h - (5.0 * 3.2 + 1.0)).abs() < 1e-3);
        // 20 m x 22 m footprint, about.
        let b = scene.bounds();
        assert!(b.max.x - b.min.x > 50.0);
        // Ray down onto the roof hits the collider.
        let mut pw = forja_physics::PhysicsWorld::default();
        w.add_to_physics(&mut pw);
        pw.step(1.0 / 60.0);
        let c = {
            let p = data.nodes[&1];
            let q = data.nodes[&3];
            ((p.0 + q.0) / 2.0, (p.1 + q.1) / 2.0)
        };
        let lon0 = -80.1345;
        let lat0: f64 = 25.7805;
        let x = ((c.0 - lon0) * 6_378_137.0 * lat0.to_radians().cos() * std::f64::consts::PI / 180.0) as f32;
        let z = (-(c.1 - lat0) * 6_378_137.0 * std::f64::consts::PI / 180.0) as f32;
        let hit = pw.raycast(&forja_math::Ray::new(Vec3::new(x, 100.0, z), Vec3::NEG_Y), 200.0, Default::default()).unwrap();
        assert!((hit.point.y - 17.0).abs() < 0.01, "roof hit at {}", hit.point.y);
    }
}
