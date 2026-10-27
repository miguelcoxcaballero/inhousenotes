//! Integration tests: every element type builds finite, non-degenerate,
//! outward geometry; samples build; components, arrays and mirrors work.

use forja_math::Vec3;

use super::build::{self, Model};
use super::footprint::Footprint;
use super::parse::parse_spec;

fn build_text(text: &str, fp: Option<&Footprint>) -> Model {
    let spec = parse_spec(text, "test.toml").unwrap_or_else(|e| panic!("{e}"));
    build::build(&spec, fp, text, "test.toml").unwrap_or_else(|e| panic!("{e}"))
}

fn assert_sane(m: &Model, what: &str) {
    assert!(m.geo.tri_count() > 0, "{what}: no triangles");
    for p in m.geo.prims.values() {
        assert_eq!(p.positions.len(), p.normals.len());
        assert_eq!(p.positions.len(), p.uvs.len());
        for t in p.indices.chunks_exact(3) {
            let (a, b, c) = (p.positions[t[0] as usize], p.positions[t[1] as usize], p.positions[t[2] as usize]);
            assert!(a.is_finite() && b.is_finite() && c.is_finite(), "{what}: NaN position");
            assert!((b - a).cross(c - a).length_squared() > 1e-12, "{what}: degenerate triangle");
        }
        assert!(p.normals.iter().all(|n| n.is_finite() && (n.length() - 1.0).abs() < 1e-3), "{what}: bad normal");
        assert!(p.uvs.iter().all(|u| u.is_finite()), "{what}: bad uv");
    }
}

/// Signed volume of a triangle soup (positive when closed and outward).
fn volume(tris: &[[Vec3; 3]]) -> f32 {
    tris.iter().map(|t| t[0].dot(t[1].cross(t[2])) / 6.0).sum()
}

const HEAD: &str = "[landmark]\nname = \"t\"\n";

#[test]
fn every_element_type_builds() {
    let cases: &[(&str, &str)] = &[
        ("mass", "type = \"mass\"\npolygon = [[0,0],[10,0],[10,8],[0,8]]\nheight = 9\ncornice = true\nplinth = true\nstring_courses = [4]\nparapet = true\nroof = \"hip\"\n[[element.openings]]\nkind = \"window\"\narch = \"round\"\nbalcony = true\nsurround = true\nsill = true\nkeystone = true\n[[element.openings]]\nedges = \"facing:s\"\nkind = \"door\"\nbays = 1"),
        ("slab", "type = \"slab\"\npolygon = \"square:6\"\ny = 1"),
        ("wall", "type = \"wall\"\npath = [[0,0],[10,0],[10,6]]\nheight = 4\ncoping = true\n[[element.openings]]\nkind = \"arch\""),
        ("cornice", "type = \"cornice\"\npolygon = \"octagon:5\"\ny = 6\nmodillions = true\ndentils = true"),
        ("string_course", "type = \"string_course\"\npath = [[0,0],[5,0]]\ny = 3"),
        ("plinth", "type = \"plinth\"\npolygon = \"square:5\""),
        ("parapet", "type = \"parapet\"\npolygon = \"square:5\"\ny = 5"),
        ("balustrade", "type = \"balustrade\"\npolygon = \"square:6\"\ny = 5\nfinials = \"urn\""),
        ("crenellations", "type = \"crenellations\"\npolygon = \"square:8\"\ny = 5\nshape = \"stepped\"\nloopholes = true"),
        ("machicolations", "type = \"machicolations\"\npolygon = \"square:8\"\ny = 5\nmerlons = \"swallowtail\""),
        ("gargoyles", "type = \"gargoyles\"\npolygon = \"square:8\"\ny = 5"),
        ("roof gable", "type = \"roof\"\npolygon = [[0,0],[12,0],[12,6],[6,9],[0,6]]\nshape = \"gable\"\ny = 5"),
        ("roof shed", "type = \"roof\"\npolygon = \"rect:8x5\"\nshape = \"shed\"\ny = 5\nridge = \"s\""),
        ("roof mansard", "type = \"roof\"\npolygon = \"rect:12x8\"\nshape = \"mansard\"\ny = 5"),
        ("roof barrel", "type = \"roof\"\npolygon = \"rect:12x6\"\nshape = \"barrel\"\ny = 5\n[[element.gable_openings]]\nkind = \"oculus\"\nwidth = 1\nlevels = [6]"),
        ("roof spire", "type = \"roof\"\npolygon = \"octagon:3\"\nshape = \"spire\"\ny = 5\nribs = 8"),
        ("roof cone", "type = \"roof\"\npolygon = \"circle:3\"\nshape = \"cone\"\ny = 5"),
        ("roof dome", "type = \"roof\"\npolygon = \"square:8\"\nshape = \"dome\"\ny = 5"),
        ("dome", "type = \"dome\"\ncenter = [0,0]\ny = 10\nradius = 4\nprofile = \"onion\"\ngores = 8\noculus = 0.8\nbase_ring = true\ndrum = { height = 3, windows = 8 }\nfinial = \"cross\"\n[[element.lantern]]\ntype = \"lantern\"\nradius = 0.6\ncap = \"onion\""),
        ("tower", "type = \"tower\"\npolygon = { shape = \"square\", center = [0,0], width = 6 }\n[[element.stages]]\nheight = 10\n[[element.stages]]\nheight = 6\ninset = 0.3\nbelfry = true\npinnacles = true\n[[element.stages]]\nheight = 3\npolygon = { shape = \"octagon\", width = 5 }\ncrenellations = true\n[[element.top]]\ntype = \"roof\"\nshape = \"pyramid\""),
        ("column", "type = \"column\"\nheight = 6\norder = \"salomonic\"\npedestal = 1"),
        ("column ionic", "type = \"column\"\nheight = 6\norder = \"ionic\"\nflutes = 16"),
        ("pilaster", "type = \"pilaster\"\nheight = 6\nflutes = 4\norder = \"corinthian\""),
        ("buttress", "type = \"buttress\"\nheight = 10\nsetoffs = 2\npinnacle = true"),
        ("flying_buttress", "type = \"flying_buttress\"\nfrom = [0,8,-6]\nto = [0,12,0]\npier = true\npinnacle = true"),
        ("pinnacle", "type = \"pinnacle\"\nsides = 8"),
        ("finial", "type = \"finial\"\nstyle = \"weathervane\""),
        ("statue", "type = \"statue\"\nstyle = \"angel\"\npedestal = true"),
        ("stairs", "type = \"stairs\"\nfrom = [0,0]\nto = [3,0]\nrise = 1\ncheeks = true"),
        ("stairs around", "type = \"stairs\"\npolygon = \"square:6\"\nsteps = 3"),
        ("arcade", "type = \"arcade\"\nfrom = [0,0]\nto = [20,0]\nbays = 5\ngallery = 3\nhood = true\ncornice = true"),
        ("arcade piers", "type = \"arcade\"\npolygon = \"rect:20x10\"\nedges = \"facing:s\"\npiers = true\narch = \"horseshoe\""),
        ("lantern", "type = \"lantern\"\ncap = \"spire\""),
        ("box", "type = \"box\"\nsize = [1,2,3]"),
        ("cylinder", "type = \"cylinder\"\nheight = 2\nfaceted = true\nsegments = 6"),
        ("cone", "type = \"cone\"\nheight = 2"),
        ("sphere", "type = \"sphere\"\nradii = [1,2,1]\nlat = [0, 90]"),
        ("torus", "type = \"torus\"\nradius = 2\ntube = 0.3"),
        ("lathe", "type = \"lathe\"\nprofile = [[0,0],[1,0],[1,1],[0,1.5]]\nangle = [0, 180]"),
        ("extrude", "type = \"extrude\"\nprofile = { shape = \"square\", width = 2, holes = [\"circle:0.5\"] }\nlength = 3"),
        ("sweep", "type = \"sweep\"\nprofile = \"circle\"\npath = { curve = [[0,0,0],[2,3,0],[4,0,2]] }\ntwist = 90\nscale_end = 0.3"),
        ("loft", "type = \"loft\"\nsection = { shape = \"superellipse\", size = [2,1], exponent = 4 }\nstations = [{ at = [0,0,0] }, { at = [0,4,1], scale = [0.5,0.5] }, { at = [0,8,0], scale = [0.2,0.2] }]"),
        ("shell", "type = \"shell\"\nsurface = \"superellipsoid\"\nradii = [10,5,6]\nexponents = [2.5,1.5]\nclip = [[0,0,1,4]]\nribs = 8"),
        ("shell hypar", "type = \"shell\"\nsurface = \"hypar\"\ncorners = [[0,0,0],[4,2,0],[4,0,4],[0,2,4]]"),
        ("shell torus", "type = \"shell\"\nsurface = \"torus\"\nradii = [5,1,1]"),
        ("mesh", "type = \"mesh\"\nvertices = [[0,0,0],[1,0,0],[0,1,0],[0,0,1]]\ntriangles = [[0,2,1],[0,1,3],[1,2,3],[2,0,3]]\nsmooth = 30"),
    ];
    for (what, body) in cases {
        let m = build_text(&format!("{HEAD}[[element]]\n{body}\n"), None);
        assert_sane(&m, what);
    }
}

#[test]
fn closed_masses_have_positive_volume_and_metrics() {
    let fp = Footprint::parse(r#"{"anchor":[0,0],"outlines":[{"id":"way/1","tags":{"height":"9"},"outer":[[0,0],[10,0],[10,8],[0,8]]}]}"#, "fp").unwrap();
    let m = build_text(&format!("{HEAD}[[element]]\ntype = \"mass\"\npolygon = \"outline:0\"\nheight = 9\n[[element.openings]]\nkind = \"window\"\n"), Some(&fp));
    let v = volume(&m.collider);
    assert!((v - 720.0).abs() < 1.0, "volume {v}");
    let j = super::metrics::compute(&m, Some(&fp), "t", 1.0);
    assert!(j["footprint"]["osm"]["iou"].as_f64().unwrap() > 0.95, "{}", j["footprint"]);
    assert_eq!(j["outlines"][0]["height_check"], "ok");
}

#[test]
fn components_arrays_and_mirrors() {
    let text = format!(
        "{HEAD}[component.post]\n[[component.post.children]]\ntype = \"box\"\nsize = [0.2, 1, 0.2]\n\
         [[element]]\ntype = \"array\"\nmode = \"grid\"\ncounts = [3, 2]\nsteps = [2, 2]\ncomponent = \"post\"\nmirror = \"x\"\n\
         [[element]]\ntype = \"array\"\nmode = \"edges\"\npolygon = \"square:10\"\nspacing = 2.5\ncorners = \"include\"\ncomponent = \"post\"\n\
         [[element]]\ntype = \"instance\"\ncomponent = \"post\"\nat = [20, 0, 0]\n"
    );
    let m = build_text(&text, None);
    // 6 posts x 2 (mirror) + 4 edges x 5 + 4 corners + 1, 12 triangles each.
    assert_eq!(m.geo.tri_count(), (12 + 24 + 1) * 12, "{}", m.geo.tri_count());
    // Mirrored boxes keep outward winding.
    for p in m.geo.prims.values() {
        for t in p.indices.chunks_exact(3) {
            let (a, b, c) = (p.positions[t[0] as usize], p.positions[t[1] as usize], p.positions[t[2] as usize]);
            let n = (b - a).cross(c - a).normalize();
            assert!(n.dot(p.normals[t[0] as usize]) > 0.9);
        }
    }
}

#[test]
fn semantic_errors_name_the_element_and_line() {
    let text = format!("{HEAD}\n[[element]]\ntype = \"mass\"\nname = \"nave\"\npolygon = \"square:4\"\nheight = 3\nmaterial = \"stoen\"\n");
    let spec = parse_spec(&text, "x.toml").unwrap();
    let e = build::build(&spec, None, &text, "x.toml").err().unwrap().to_string();
    assert!(e.contains("x.toml:4") && e.contains("nave") && e.contains("did you mean `stone`"), "{e}");
    let text = format!("{HEAD}[[element]]\ntype = \"roof\"\non = \"naev\"\nshape = \"gable\"\n[[element]]\ntype = \"mass\"\nname = \"nave\"\npolygon = \"square:4\"\nheight = 3\n");
    let spec = parse_spec(&text, "x.toml").unwrap();
    let e = build::build(&spec, None, &text, "x.toml").err().unwrap().to_string();
    assert!(e.contains("did you mean `nave`"), "{e}");
}

#[test]
fn samples_build_within_budget() {
    for (spec, fp) in [
        (include_str!("../../../../data/landmarks/_samples/gothic_church/landmark.toml"), include_str!("../../../../data/landmarks/_samples/gothic_church/footprint.json")),
        (include_str!("../../../../data/landmarks/_samples/white_shell/landmark.toml"), include_str!("../../../../data/landmarks/_samples/white_shell/footprint.json")),
    ] {
        let fp = Footprint::parse(fp, "fp").unwrap();
        let m = build_text(spec, Some(&fp));
        assert_sane(&m, "sample");
        assert!(m.geo.tri_count() < 400_000);
        let j = super::metrics::compute(&m, Some(&fp), "s", 1.0);
        assert!(j["footprint"]["osm"]["iou"].as_f64().unwrap() > 0.6, "{}", j["footprint"]);
    }
}

// ---------------------------------------------------------- organic plans

/// A block whose south front is a curve of `n` short facets.
fn curved_block(n: usize, facet: f32) -> String {
    let r = facet / (2.0 * (0.5f32.to_radians() * 0.5 * 26.0 / n as f32).sin().max(1e-4));
    let span = 26.0f32.to_radians();
    let mut pts: Vec<[f32; 2]> = (0..=n).map(|k| {
        let a = span * 0.5 - span * k as f32 / n as f32;
        [r * a.sin(), -r + r * a.cos() + 4.0]
    }).collect();
    let (w, e) = (pts[n], pts[0]);
    pts.push([w[0], w[1] - 14.0]);
    pts.push([e[0], e[1] - 14.0]);
    let ring: Vec<String> = pts.iter().map(|p| format!("[{:.3}, {:.3}]", p[0], p[1])).collect();
    format!("[{}]", ring.join(", "))
}

#[test]
fn openings_follow_a_curved_facade_by_arc_length() {
    let poly = curved_block(10, 2.5);
    let t = format!(
        "{HEAD}[[element]]\ntype = \"mass\"\npolygon = {poly}\nheight = 14\n[[element.openings]]\nedges = \"facing:s\"\nkind = \"door\"\nwidth = 1.4\nheight = 2.8\nlevels = [0]\nbays = {{ spacing = 3.4 }}\n[[element.openings]]\nedges = \"facing:s\"\nkind = \"window\"\nlevels = {{ start = 4.6, step = 3.2, count = 3 }}\nbays = {{ spacing = 3.4 }}\nbalcony = true\n"
    );
    let m = build_text(&t, None);
    assert_sane(&m, "curved");
    assert!(m.warnings.is_empty(), "{:?}", m.warnings);
    // About 25 m of curved front at 3.4 m: 7 bays per row, 4 rows.
    let glass = m.palette_id("glass").map(|g| m.geo.prims.get(&g).map(|p| p.tri_count()).unwrap_or(0)).unwrap_or(0);
    let door = m.palette_id("door").map(|g| m.geo.prims.get(&g).map(|p| p.tri_count()).unwrap_or(0)).unwrap_or(0);
    assert!(glass >= 21 * 2, "glass triangles {glass}");
    assert!(door > 0, "doors");
    // Every vertex lies on or behind the curved wall, never in the street
    // beyond the balconies (0.6 m + ironwork).
    let (_, mx) = super::mesh::bounds(&m.geo).unwrap();
    assert!(mx.z < 4.0 + 0.9, "{mx}");
}

#[test]
fn a_wide_gate_in_a_short_facade_is_kept_and_misses_are_reported() {
    let t = format!("{HEAD}[[element]]\ntype = \"mass\"\npolygon = [[0,0],[7,0],[7,6],[0,6]]\nheight = 9\n[[element.openings]]\nedges = \"facing:s\"\nkind = \"arch\"\nwidth = 5.5\nheight = 6\n[[element.openings]]\nedges = \"facing:n\"\nkind = \"arch\"\nwidth = 7.5\nheight = 6\n");
    let m = build_text(&t, None);
    let dark = m.palette_id("dark").and_then(|g| m.geo.prims.get(&g)).map(|p| p.tri_count()).unwrap_or(0);
    assert!(dark > 0, "the 5.5 m arch was dropped");
    assert!(m.warnings.iter().any(|w| w.contains("openings[1]") && w.contains("wider than")), "{:?}", m.warnings);
}

#[test]
fn rooms_behind_windows_stay_inside_thin_wings() {
    let t = format!("{HEAD}[[element]]\ntype = \"mass\"\npolygon = [[0,0],[20,0],[20,1.5],[1.5,1.5],[1.5,20],[0,20]]\nheight = 8\n[[element.openings]]\nedges = \"facing:s\"\nkind = \"window\"\nlevels = [1.0, 4.5]\n");
    let m = build_text(&t, None);
    let (mn, mx) = super::mesh::bounds(&m.geo).unwrap();
    assert!(mn.x > -0.01 && mn.z > -0.01 && mx.x < 20.01, "{mn} {mx}");
}

#[test]
fn passages_cut_through_and_are_walkable() {
    let t = format!("{HEAD}[[element]]\ntype = \"mass\"\nname = \"gate\"\npolygon = [[0,0],[8,0],[8,10],[0,10]]\nheight = 12\n[[element.openings]]\nedges = \"facing:s\"\nkind = \"passage\"\narch = \"pointed\"\nwidth = 4\nheight = 6\nbays = 1\n");
    let m = build_text(&t, None);
    assert_sane(&m, "passage");
    assert!(m.warnings.is_empty(), "{:?}", m.warnings);
    // The collider has the tunnel: a ray along the passage axis at 2 m
    // height meets no triangle between the two facades.
    let hit = m.collider.iter().any(|t| {
        let o = Vec3::new(4.0, 2.0, -1.0);
        let d = Vec3::Z;
        let (e1, e2) = (t[1] - t[0], t[2] - t[0]);
        let p = d.cross(e2);
        let det = e1.dot(p);
        if det.abs() < 1e-9 {
            return false;
        }
        let s = o - t[0];
        let u = s.dot(p) / det;
        let q = s.cross(e1);
        let v = d.dot(q) / det;
        let tt = e2.dot(q) / det;
        (0.0..=1.0).contains(&u) && v >= 0.0 && u + v <= 1.0 && tt > 0.0 && tt < 12.0
    });
    assert!(!hit, "the passage is blocked in the collider");
}

#[test]
fn open_shells_unions_and_subtractions() {
    let t = format!(
        "{HEAD}[[element]]\ntype = \"mass\"\npolygon = \"square:10\"\nheight = 10\nopen_edges = \"facing:e\"\nwall_thickness = 1.5\n[[element]]\ntype = \"slab\"\npolygon = {{ shape = \"square\", width = 6, center = [20, 0], union = [[[22, -1], [27, -1], [27, 1], [22, 1]]], subtract = [\"circle:0.1\"] }}\ny = 0.3\n"
    );
    let m = build_text(&t, None);
    assert_sane(&m, "shell");
    // The open shell has an interior: its plan area is the walls only.
    let s = super::poly::Shape::new(super::poly::rect(forja_math::Vec2::ZERO, forja_math::Vec2::splat(10.0), 0.0));
    let shell = super::masses::open_shell(&s, &super::resolve::select(&s, &super::spec::EdgeSel(vec![super::spec::EdgeTok::parse("facing:e").unwrap()])), 1.5);
    let area: f32 = shell.iter().map(|p| p.area()).sum();
    assert!((area - (100.0 - 7.0 * 8.5)).abs() < 0.5, "{area}");
}

#[test]
fn hip_roofs_follow_concave_and_courtyard_plans() {
    for poly in ["[[0,0],[20,0],[20,6],[8,6],[8,20],[0,20]]", "[[0,0],[5,0],[5.1,0.14],[5.3,0],[12,0],[12,9],[0,9]]"] {
        let t = format!("{HEAD}[[element]]\ntype = \"mass\"\npolygon = {poly}\nheight = 6\nroof = {{ shape = \"hip\", pitch = 30 }}\n");
        let m = build_text(&t, None);
        assert!(m.warnings.is_empty(), "{poly}: {:?}", m.warnings);
    }
    let t = format!("{HEAD}[[element]]\ntype = \"mass\"\npolygon = {{ points = [[0,0],[30,0],[30,24],[0,24]], holes = [[[8,7],[22,7],[22,17],[8,17]]] }}\nheight = 9\nroof = \"hip\"\n");
    let m = build_text(&t, None);
    assert!(m.warnings.is_empty(), "{:?}", m.warnings);
    // Nothing of the roof over the courtyard.
    let roof = m.palette_id("roof").and_then(|g| m.geo.prims.get(&g)).unwrap();
    let inside = roof.positions.iter().any(|p| p.x > 9.0 && p.x < 21.0 && p.z > 8.0 && p.z < 16.0);
    assert!(!inside);
}

#[test]
fn bounded_resolutions_arrays_and_budget() {
    for (bad, msg) in [
        ("type = \"cylinder\"\nradius = 1\nheight = 2\nsegments = 5000000", "out of range"),
        ("type = \"shell\"\nsurface = \"ellipsoid\"\nradius = 5\nsegments = [20000, 20000]", "out of range"),
        ("type = \"array\"\nmode = \"linear\"\ncount = 1000000000\nstep = [1, 0, 0]\n[[element.children]]\ntype = \"box\"\nsize = [1, 1, 1]", "instances"),
        ("type = \"array\"\nmode = \"linear\"\ncount = 150\nstep = [2, 0, 0]\n[[element.children]]\ntype = \"array\"\nmode = \"linear\"\ncount = 150\nstep = [0, 0, 2]\n[[element.children.children]]\ntype = \"sphere\"\nradius = 0.5\nsegments = 64", "triangles"),
    ] {
        let t = format!("{HEAD}[[element]]\n{bad}\n");
        let spec = parse_spec(&t, "t").unwrap();
        let e = build::build(&spec, None, &t, "t").err().unwrap_or_else(|| panic!("{bad} built"));
        assert!(e.to_string().contains(msg), "{e}");
    }
}

#[test]
fn array_ramps_and_radial_without_rotation() {
    let t = format!("{HEAD}[[element]]\ntype = \"array\"\nmode = \"linear\"\ncount = 5\nstep = [3, 0, 0]\nscale_end = [1, 3, 1]\nrotate_step = 10\n[[element.children]]\ntype = \"box\"\nsize = [1, 1, 1]\n");
    let m = build_text(&t, None);
    let (_, mx) = super::mesh::bounds(&m.geo).unwrap();
    assert!((mx.y - 3.0).abs() < 1e-3, "{mx}");
    let t = format!("{HEAD}[[element]]\ntype = \"array\"\nmode = \"radial\"\ncount = 8\nrotate = false\n[[element.children]]\ntype = \"box\"\ncenter = [10, 0.5, 0]\nsize = [1, 1, 1]\n");
    let m = build_text(&t, None);
    let (mn, mx) = super::mesh::bounds(&m.geo).unwrap();
    assert!(mn.x < -9.0 && mx.z > 9.0, "{mn} {mx}");
}

#[test]
fn mirrored_colliders_stay_outward() {
    let t = format!("{HEAD}[[element]]\ntype = \"mass\"\npolygon = [[-2,0],[10,0],[10,10],[-2,10]]\nheight = 5\nmirror = \"x\"\n");
    let m = build_text(&t, None);
    // Both copies are closed and outward: positive volume each.
    assert!(volume(&m.collider) > 0.0);
    let half = m.collider.len() / 2;
    assert!(volume(&m.collider[half..]) > 0.0 && volume(&m.collider[..half]) > 0.0);
}

#[test]
fn closed_walls_any_winding_and_curved_arcades() {
    for path in ["[[0,0],[10,0],[10,10],[0,10]]", "[[0,0],[0,10],[10,10],[10,0]]"] {
        let t = format!("{HEAD}[[element]]\ntype = \"wall\"\npath = {path}\nclosed = true\nheight = 3\nthickness = 1\n");
        let m = build_text(&t, None);
        assert_eq!(m.geo.tri_count(), 24, "{path}");
    }
    // An arcade round a circular square: one bent arcade, no warnings.
    let t = format!("{HEAD}[[element]]\ntype = \"arcade\"\npolygon = \"circle:14\"\nbays = 24\nimpost = 3\n");
    let m = build_text(&t, None);
    assert_sane(&m, "round arcade");
    assert!(m.warnings.is_empty(), "{:?}", m.warnings);
}
