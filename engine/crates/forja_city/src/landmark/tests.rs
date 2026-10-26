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
