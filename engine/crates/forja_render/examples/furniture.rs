//! Valencian street furniture: lineups of every container, litter-bin and
//! traffic-signal variant and a short street with a container island, a
//! signalled crossing and bins, at eye level:
//! `cargo run --release -p forja_render --example furniture -- out_prefix [shot,shot]`
//! (`NIGHT=1` for 22:30).

use std::path::PathBuf;

use forja_city::building::{self, CityOut, EdgeKind, Lot, ModuleLib};
use forja_city::furniture::{self, Aspect};
use forja_city::geom::Parts;
use forja_city::kit::{Kit, Mat, assemble, hex};
use forja_city::signals::{self, Mount, SignalSpec};
use forja_city::street_waste::{self as sw, BinModel, ContainerModel};
use forja_city::style::StyleLibrary;
use forja_math::{Mat4, Quat, Vec2, Vec3};
use forja_render::*;

fn main() {
    let prefix = std::env::args().nth(1).unwrap_or_else(|| "furniture".into());
    let only: Option<String> = std::env::args().nth(2);
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dl = root.join("assets/downloaded");
    let mut kit = Kit::load(Some(&dl));
    if std::env::var("NIGHT").is_ok_and(|v| v != "0") {
        kit.set_hour(22.5);
    }
    let night = kit.night.is_night();
    let styles = StyleLibrary::load(Some(&root.join("data/city/styles.toml")));
    let mut lib = ModuleLib::new(&kit);
    let mut out = CityOut::default();
    let mut lights = Vec::new();
    let put = |out: &mut CityOut, lib: &mut ModuleLib, name: &str, p: &dyn Fn() -> Parts<Mat>, m: Mat4| {
        let i = lib.named(out, name, p);
        out.placements.push((i, m));
    };
    let at = |x: f32, y: f32, z: f32, yaw: f32| Mat4::from_rotation_translation(Quat::from_rotation_y(yaw), Vec3::new(x, y, z));

    // --- The street: Valencian facades on the north side (z < 0), 5 m
    // pavement, 2.2 m parking lane, two traffic lanes, far pavement.
    let valencia: Vec<_> = styles.styles.iter().filter(|s| s.id.starts_with("valencia")).collect();
    let list = if valencia.is_empty() { styles.styles.iter().collect::<Vec<_>>() } else { valencia };
    let mut x = -60.0;
    let mut k = 0;
    while x < 40.0 {
        let s = list[k % list.len()];
        let w = 14.0 + (k % 3) as f32 * 2.5;
        let lot = Lot { ring: vec![Vec2::new(x, -18.0), Vec2::new(x + w, -18.0), Vec2::new(x + w, 0.0), Vec2::new(x, 0.0)], edges: vec![EdgeKind::Courtyard, EdgeKind::Party, EdgeKind::Street, EdgeKind::Party], base: 0.15 };
        building::build(&mut lib, &mut out, &lot, s, k as u64 * 7 + 3);
        x += w;
        k += 1;
    }
    let mut p: Parts<Mat> = Parts::new();
    let (x0, x1) = (-80.0, 300.0);
    let pave = |p: &mut Parts<Mat>, z0: f32, z1: f32| {
        p.get(Mat::SidewalkHex).polygon(&[Vec2::new(x0, z0), Vec2::new(x1, z0), Vec2::new(x1, z1), Vec2::new(x0, z1)], 0.15, true, Mat::SidewalkHex.scale());
    };
    pave(&mut p, 0.0, 5.0);
    p.get(Mat::Curb).aabox(Vec3::new(x0, 0.0, 5.0), Vec3::new(x1, 0.15, 5.25), 1.0);
    p.get(Mat::Asphalt).polygon(&[Vec2::new(x0, 5.25), Vec2::new(x1, 5.25), Vec2::new(x1, 14.75), Vec2::new(x0, 14.75)], 0.0, true, Mat::Asphalt.scale());
    p.get(Mat::Curb).aabox(Vec3::new(x0, 0.0, 14.75), Vec3::new(x1, 0.15, 15.0), 1.0);
    pave(&mut p, 15.0, 30.0);
    let white = Mat::Paint(hex(0xf2f2ec));
    // Parking-lane line and centre dashes.
    p.get(white).aabox(Vec3::new(x0, 0.0, 7.4), Vec3::new(x1, 0.006, 7.5), 1.0);
    let mut d = x0;
    while d < x1 {
        p.get(white).aabox(Vec3::new(d, 0.0, 11.05), Vec3::new(d + 4.5, 0.006, 11.2), 1.0);
        d += 10.0;
    }
    // Zebra crossing at x = 20.
    for k in 0..8 {
        let z = 5.6 + k as f32 * 1.15;
        p.get(white).aabox(Vec3::new(17.5, 0.0, z), Vec3::new(22.5, 0.006, z + 0.55), 1.0);
    }
    // Container bay: painted frame in the parking lane.
    p.get(white).aabox(Vec3::new(-14.6, 0.0, 5.3), Vec3::new(-4.4, 0.006, 5.4), 1.0);
    for xx in [-14.6f32, -4.5] {
        p.get(white).aabox(Vec3::new(xx, 0.0, 5.3), Vec3::new(xx + 0.1, 0.006, 7.4), 1.0);
    }
    let street = out.add_scene(assemble(&kit, "street", p, None));
    out.placements.push((street, Mat4::IDENTITY));

    // Container island in the parking lane, fronts facing the pavement (-Z).
    let island = [(ContainerModel::Kerbside, 3u8, 1u8), (ContainerModel::Kerbside, 0, 2), (ContainerModel::SideLoader, 1, 0), (ContainerModel::SideLoader, 4, 3), (ContainerModel::Kerbside, 2, 1)];
    let mut cx = -13.8;
    for (m, f, w) in island {
        let size = sw::container_size(m, f);
        cx += size.x * 0.5;
        put(&mut out, &mut lib, &format!("c_{m:?}_{f}_{w}"), &|| sw::container_model(m, f, w), at(cx, 0.0, 5.35 + size.y * 0.5 + 0.05, std::f32::consts::PI));
        cx += size.x * 0.5 + 0.25;
    }
    // Litter bins: double bin by the crossing, one on a lamp post, one on
    // a wall.
    put(&mut out, &mut lib, "bin_double", &|| sw::papelera(BinModel::PostDouble, 0), at(14.5, 0.15, 4.3, 0.0));
    put(&mut out, &mut lib, "bin_wall", &|| sw::papelera(BinModel::Wall, 1), at(-24.0, 0.15, 0.0, 0.0));
    put(&mut out, &mut lib, "bin_frame", &|| sw::papelera(BinModel::Frame, 2), at(-2.0, 0.15, 4.4, std::f32::consts::PI));
    // Lamp post with a clamped bin.
    let lamp = lib.named(&mut out, "lamp", || furniture::street_lamp(true));
    for lx in [-30.0f32, -8.0, 12.0, 34.0] {
        out.placements.push((lamp, at(lx, 0.15, 4.4, 0.0)));
    }
    put(&mut out, &mut lib, "bin_clamp", &|| sw::papelera(BinModel::Clamp(9), 0), at(34.0, 0.15, 4.4, std::f32::consts::PI));
    // Signals at the crossing: corner columns on both pavements, the mast
    // arm over the far lanes, pedestrian heads facing across.
    let ped_yaw_n = 0.0; // north column: pedestrians on it look south (+Z)? they face the crossing.
    let _ = ped_yaw_n;
    let state = Aspect::Red;
    let walk = true;
    let corner = SignalSpec { bike: false, ..SignalSpec::CORNER };
    // North pavement, west of the crossing: vehicle head faces traffic
    // coming from -X (+X travel => face -X), pedestrian head faces south.
    let face_west = -std::f32::consts::FRAC_PI_2;
    let ped_south = std::f32::consts::FRAC_PI_2; // local yaw relative to the post
    let posts = [
        ("sig_n", corner, Vec3::new(17.0, 0.15, 4.55), face_west, ped_south),
        ("sig_s", SignalSpec { button: true, countdown: true, ..SignalSpec::CORNER }, Vec3::new(23.0, 0.15, 15.45), std::f32::consts::FRAC_PI_2, ped_south),
        ("sig_mast", SignalSpec { mount: Mount::Mast { reach: 5.5 }, ..SignalSpec::for_arm(5.5) }, Vec3::new(16.2, 0.15, 15.5), -std::f32::consts::FRAC_PI_2, -std::f32::consts::FRAC_PI_2),
    ];
    for (name, spec, pos, yaw, pyaw) in posts {
        let m = at(pos.x, pos.y, pos.z, yaw);
        put(&mut out, &mut lib, name, &|| signals::signal_post(&spec, state, walk, pyaw), m);
        for mut l in signals::signal_post_lights(&spec, state, walk, pyaw) {
            l.pos = m.transform_point3(l.pos);
            if let Some(s) = &mut l.spot {
                s.dir = m.transform_vector3(s.dir);
            }
            lights.push(l);
        }
    }
    // Underground mouths in a small plaza patch further east.
    for (i, f) in [3u8, 0, 1, 4].iter().enumerate() {
        put(&mut out, &mut lib, &format!("ug_{f}"), &|| sw::underground_bin(*f), at(40.0 + i as f32 * 1.75, 0.15, 2.6, 0.0));
    }
    put(&mut out, &mut lib, "igloo_x", &|| sw::glass_igloo(1), at(47.5, 0.15, 2.6, 0.3));

    // --- Lineups on the far pavement band at x >= 100.
    let lx = 100.0;
    // One line along x at z = 17, fronts toward the road (-Z): the
    // kerbside generation with the igloo, the side loaders, the 1100 l
    // bins and the underground mouths.
    let mut cx = lx;
    for m in [ContainerModel::Kerbside, ContainerModel::SideLoader, ContainerModel::Bin1100] {
        for f in [3u8, 0, 1, 4, 2] {
            if f == 2 && m != ContainerModel::Kerbside {
                continue;
            }
            let w = (f + m as u8) % 4;
            let size = sw::container_size(m, f);
            cx += size.x * 0.5;
            put(&mut out, &mut lib, &format!("c_{m:?}_{f}_{w}"), &|| sw::container_model(m, f, w), at(cx, 0.15, 17.0, std::f32::consts::PI));
            cx += size.x * 0.5 + 0.35;
        }
        cx += 1.2;
    }
    println!("lineup containers end at x = {cx}");
    for (i, f) in (0..5u8).enumerate() {
        put(&mut out, &mut lib, &format!("ug_{f}"), &|| sw::underground_bin(f), at(cx + 0.8 + i as f32 * 1.8, 0.15, 17.0, std::f32::consts::PI));
    }
    let bins = [BinModel::PostDouble, BinModel::PostSingle, BinModel::Wall, BinModel::Clamp(9), BinModel::Frame, BinModel::Park, BinModel::Compactor(3), BinModel::Compactor(0)];
    for (i, b) in bins.iter().enumerate() {
        let b = *b;
        put(&mut out, &mut lib, &format!("bin_{b:?}"), &|| sw::papelera(b, i as u8), at(lx + 50.0 + i as f32 * 1.3, 0.15, 17.0, std::f32::consts::PI));
        if matches!(b, BinModel::Clamp(_)) {
            out.placements.push((lamp, at(lx + 50.0 + i as f32 * 1.3, 0.15, 17.0, 0.0)));
        }
    }
    let specs = [
        SignalSpec::CORNER,
        SignalSpec { pole: signals::POLE_ANTHRACITE, twin: true, low_repeater: true, ..SignalSpec::CORNER },
        SignalSpec { bike: true, button: true, countdown: true, ..SignalSpec::CORNER },
        SignalSpec::for_arm(4.5),
    ];
    for (i, s) in specs.iter().enumerate() {
        let s = *s;
        let st = [Aspect::Red, Aspect::Amber, Aspect::Green, Aspect::Green][i];
        let m = at(lx + 64.0 + i as f32 * 2.6, 0.15, 17.0, std::f32::consts::PI);
        put(&mut out, &mut lib, &format!("sigl_{i}"), &|| signals::signal_post(&s, st, i % 2 == 0, -std::f32::consts::FRAC_PI_2 * 0.5), m);
        for mut l in signals::signal_post_lights(&s, st, i % 2 == 0, -std::f32::consts::FRAC_PI_2 * 0.5) {
            l.pos = m.transform_point3(l.pos);
            lights.push(l);
        }
    }
    put(&mut out, &mut lib, "lamp_head", &|| signals::head_on_lamp_post(Aspect::Green, 0.08, 3.0), at(lx + 76.0, 0.15, 17.0, 0.0));
    out.placements.push((lamp, at(lx + 76.0, 0.15, 17.0, 0.0)));
    out.city_lights.extend(lights);

    let tris: usize = out.placements.iter().map(|(s, _)| out.scenes[*s].triangle_count()).sum();
    println!("{} scenes, {} placements, {} tris", out.scenes.len(), out.placements.len(), tris);
    let mut r = Renderer::new(&create_instance(), None).unwrap();
    if let Ok(img) = forja_assets::load_hdr(&dl.join("hdri_wide_street/wide_street_01_2k.hdr")) {
        r.set_environment(Some(&env::EnvMap::from_hdr(&img, 14.0)));
    }
    let handles: Vec<ModelHandle> = out.scenes.iter().map(|s| r.upload_scene(s)).collect();
    let mut frame = FrameDesc::default();
    for (s, m) in &out.placements {
        frame.instances.push(DrawInstance::new(handles[*s], *m));
    }
    frame.env.shadow_distance = 60.0;
    if night {
        frame.env.use_ibl = false;
        frame.env.sun_dir = Vec3::new(-0.35, 0.72, -0.55).normalize();
        frame.env.sun_illuminance = 0.025;
        frame.env.exposure = 0.55 + 3.0 * kit.night.dark;
        frame.env.bloom = 0.08;
    }
    let shots: Vec<(&str, Vec3, Vec3, f32)> = vec![
        ("street", Vec3::new(-22.0, 1.65, 3.2), Vec3::new(0.0, 1.6, 6.8), 60.0),
        ("island", Vec3::new(-6.5, 1.6, 2.2), Vec3::new(-10.5, 0.9, 6.2), 55.0),
        ("crossing", Vec3::new(8.0, 1.65, 2.5), Vec3::new(20.0, 3.0, 10.0), 60.0),
        ("plaza", Vec3::new(37.0, 1.6, 6.5), Vec3::new(43.5, 0.7, 2.6), 50.0),
        ("line_containers", Vec3::new(lx + 18.0, 2.4, 6.5), Vec3::new(lx + 18.0, 0.8, 17.0), 62.0),
        ("line_underground", Vec3::new(lx + 42.0, 1.7, 12.5), Vec3::new(lx + 40.0, 0.6, 17.0), 55.0),
        ("line_bins", Vec3::new(lx + 54.6, 1.5, 11.8), Vec3::new(lx + 54.6, 0.7, 17.0), 55.0),
        ("line_signals", Vec3::new(lx + 68.0, 2.2, 8.0), Vec3::new(lx + 68.0, 2.6, 17.0), 62.0),
        ("close_kerbside", Vec3::new(lx + 3.4, 1.55, 13.6), Vec3::new(lx + 2.0, 0.8, 17.0), 50.0),
        ("close_sideloader", Vec3::new(lx + 14.0, 1.6, 13.6), Vec3::new(lx + 13.7, 1.0, 17.0), 50.0),
        ("close_1100", Vec3::new(lx + 25.5, 1.6, 13.2), Vec3::new(lx + 24.6, 0.7, 17.0), 52.0),
        ("close_igloo", Vec3::new(lx + 9.2, 1.5, 14.0), Vec3::new(lx + 9.0, 0.8, 17.0), 50.0),
        ("close_bin_double", Vec3::new(lx + 50.0, 1.25, 15.3), Vec3::new(lx + 50.0, 0.75, 17.0), 50.0),
        ("close_signal", Vec3::new(lx + 66.0, 1.7, 12.0), Vec3::new(lx + 66.6, 2.6, 17.0), 45.0),
        ("close_mast", Vec3::new(lx + 70.0, 1.7, 9.0), Vec3::new(lx + 68.0, 4.0, 17.0), 55.0),
    ];
    for (name, eye, look, fov) in shots {
        if only.as_deref().is_some_and(|o| !o.split(',').any(|n| n.trim() == name)) {
            continue;
        }
        frame.camera = Camera::look_at(eye, look);
        frame.camera.fov_y = fov.to_radians();
        if night {
            frame.lights = forja_city::nightlight::select(&out.city_lights, eye, look - eye, MAX_LIGHTS)
                .into_iter()
                .map(|i| {
                    let (position, color, intensity, radius) = out.city_lights[i].as_point();
                    PointLight { position, radius, color, intensity }
                })
                .collect();
        }
        let img = r.render_to_image(&frame, 1280, 720);
        let path = format!("{prefix}_{name}.png");
        img.save(&path).unwrap();
        println!("{path}");
    }
}
