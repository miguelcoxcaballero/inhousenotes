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
}

impl OsmWorld {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self { path: path.as_ref().to_path_buf() }
    }

    pub fn build_from(data: &OsmData) -> Result<WorldDescription, WorldError> {
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
        scene.materials.push(mat("ground", [0.42, 0.45, 0.36], 0.95)); // 0
        scene.materials.push(mat("asphalt", [0.10, 0.10, 0.11], 0.85)); // 1
        scene.materials.push(mat("sidewalk", [0.62, 0.60, 0.56], 0.9)); // 2
        scene.materials.push(mat("sand", [0.87, 0.80, 0.62], 0.95)); // 3
        scene.materials.push(mat("grass", [0.25, 0.45, 0.18], 0.95)); // 4
        scene.materials.push(MaterialData { roughness: 0.05, metallic: 0.0, ..mat("water", [0.05, 0.25, 0.32], 0.05) }); // 5
        scene.materials.push(mat("roof", [0.55, 0.55, 0.57], 0.8)); // 6
        let facade0 = scene.materials.len();
        for (i, c) in FACADES.iter().enumerate() {
            scene.materials.push(mat(&format!("facade_{i}"), *c, 0.7));
        }

        let mut ground = MeshBuilder::new(0);
        let g = half * 1.2;
        ground.quad(Vec3::new(-g.x, 0.0, g.y), Vec3::new(g.x, 0.0, g.y), Vec3::new(g.x, 0.0, -g.y), Vec3::new(-g.x, 0.0, -g.y), 4.0);
        let mut areas: Vec<MeshBuilder> = (3..=5).map(MeshBuilder::new).collect();
        let mut roads = MeshBuilder::new(1);
        let mut paths = MeshBuilder::new(2);
        let mut roofs = MeshBuilder::new(6);
        let mut facades: Vec<MeshBuilder> = (0..FACADES.len()).map(|i| MeshBuilder::new(facade0 + i)).collect();
        let mut col_verts: Vec<Vec3> = Vec::new();
        let mut col_tris: Vec<[u32; 3]> = Vec::new();
        let mut spawns = Vec::new();
        let mut stats = (0usize, 0usize);

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
                let fb = &mut facades[wi % FACADES.len()];
                for i in 0..ring.len() {
                    let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
                    // Ring is CCW in (x, z); walls face outward with this order.
                    let (p0, p1) = (Vec3::new(b.x, 0.0, b.y), Vec3::new(a.x, 0.0, a.y));
                    let up = Vec3::Y * h;
                    fb.quad(p0, p1, p1 + up, p0 + up, 3.2);
                    let base = col_verts.len() as u32;
                    col_verts.extend([p0, p1, p1 + up, p0 + up]);
                    col_tris.extend([[base, base + 1, base + 2], [base, base + 2, base + 3]]);
                }
                let before = roofs.mesh.indices.len();
                if roofs.polygon(&ring, h, true, 4.0) {
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
                    areas[k].polygon(ring, y, true, 4.0);
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
        world.spawns = spawns;
        world.extent = g;
        log::info!("osm: {} buildings, {} roads", stats.0, stats.1);
        world.stats.push(("buildings".into(), stats.0));
        world.stats.push(("roads".into(), stats.1));
        Ok(world)
    }
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
        Self::build_from(&parse_osm(&xml)?)
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
