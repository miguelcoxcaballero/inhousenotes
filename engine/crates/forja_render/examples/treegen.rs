//! Procedural tree tool (forja_flora): generates species, renders them on
//! grass under the HDRI sky and exports glTF.
//!
//! ```text
//! treegen <species|all> [--seed N] [--out DIR] [--views tree,crown,bark,lod]
//!         [--season summer|spring|autumn|winter] [--age A] [--health H]
//!         [--pruning natural|pollard|lollipop|lifted|trimmed] [--width W]
//!         [--impostor gpu|cpu] [--glb] [--no-lineup] [--stats]
//!         [--compare DIR]   side-by-side sheets with reference photos
//!                           (DIR/<species>_*.jpg, see tools/flora_photos.py)
//! ```
//! Build: `cargo build --release -p forja_render --example treegen`.
//! Each species gets `<id>_tree.png` (whole tree with a 1.75 m figure),
//! `<id>_crown.png`, `<id>_bark.png` (and `<id>_lod.png`: LOD 0/1/2 side
//! by side); `all` also renders `lineup.png`. With several species each
//! one is rendered by a child process (peak ~1.5 GB each on llvmpipe; the
//! lineup ~2.8 GB) instead of holding every tree in one process.

use std::path::{Path, PathBuf};
use std::time::Instant;

use forja_assets::{AlphaMode, MaterialData, MeshData, NodeData, SceneData};
use forja_flora::{Params, Pruning, Season, SpeciesLibrary, TreeAsset, generate, impostor};
use forja_math::{Mat4, Quat, Vec2, Vec3, Vec4};
use forja_render::capture::{CutoutView, capture_cutouts};
use forja_render::*;

struct Args {
    target: String,
    seed: u64,
    out: PathBuf,
    views: Vec<String>,
    params: Params,
    width: u32,
    gpu_impostor: bool,
    glb: bool,
    lineup: bool,
    stats: bool,
    compare: Option<PathBuf>,
}

fn engine_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn parse() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let target = it.next().ok_or_else(usage)?;
    let mut a = Args {
        target,
        seed: 1,
        out: PathBuf::from("treegen_out"),
        views: vec!["tree".into(), "crown".into(), "bark".into()],
        params: Params { bake_impostor: false, ..Default::default() },
        width: 1280,
        gpu_impostor: true,
        glb: false,
        lineup: true,
        stats: false,
        compare: None,
    };
    while let Some(x) = it.next() {
        let mut val = || it.next().ok_or(format!("{x} needs a value"));
        match x.as_str() {
            "--seed" => a.seed = val()?.parse().map_err(|_| "bad --seed")?,
            "--out" => a.out = PathBuf::from(val()?),
            "--views" => a.views = val()?.split(',').map(|s| s.trim().to_string()).collect(),
            "--season" => a.params.season = Season::parse(&val()?).ok_or("bad --season")?,
            "--age" => a.params.age = val()?.parse().map_err(|_| "bad --age")?,
            "--health" => a.params.health = val()?.parse().map_err(|_| "bad --health")?,
            "--pruning" => a.params.pruning = Some(Pruning::parse(&val()?).ok_or("bad --pruning")?),
            "--width" => a.width = val()?.parse().map_err(|_| "bad --width")?,
            "--impostor" => a.gpu_impostor = val()? != "cpu",
            "--glb" => a.glb = true,
            "--no-lineup" => a.lineup = false,
            "--stats" => a.stats = true,
            "--compare" => a.compare = Some(PathBuf::from(val()?)),
            o => return Err(format!("unknown option {o}\n{}", usage())),
        }
    }
    a.width = a.width.clamp(320, 3840);
    Ok(a)
}

fn usage() -> String {
    "usage: treegen <species|all> [--seed N] [--out DIR] [--views tree,crown,bark,lod] [--season S] [--age A] [--health H] [--pruning P] [--width W] [--impostor gpu|cpu] [--glb] [--no-lineup] [--stats] [--compare DIR]".into()
}

fn main() {
    let args = match parse() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let root = engine_root();
    let lib = SpeciesLibrary::load(Some(&root.join("data/flora/species.toml")));
    let ids: Vec<String> = if args.target == "all" { lib.ids().iter().map(|s| s.to_string()).collect() } else { args.target.split(',').map(|s| s.to_string()).collect() };
    for id in &ids {
        if lib.get(id).is_none() {
            eprintln!("unknown species {id}; known: {}", lib.ids().join(", "));
            std::process::exit(2);
        }
    }
    std::fs::create_dir_all(&args.out).expect("create --out");
    // Several species: render each in its own process so its geometry,
    // textures and GPU buffers are freed before the next one (one process
    // for all of them ran out of memory on a software adapter).
    if ids.len() > 1 && !args.stats {
        let exe = std::env::current_exe().expect("current exe");
        let rest: Vec<String> = std::env::args().skip(2).collect();
        let mut failed = Vec::new();
        for id in &ids {
            let ok = std::process::Command::new(&exe).arg(id).args(&rest).arg("--no-lineup").status().map(|s| s.success()).unwrap_or(false);
            if !ok {
                failed.push(id.clone());
            }
        }
        if args.lineup {
            lineup(&lib, &ids, &args, &root.join("assets/downloaded"));
        }
        if !failed.is_empty() {
            eprintln!("failed: {}", failed.join(", "));
            std::process::exit(1);
        }
        return;
    }
    // Generate.
    let mut trees: Vec<TreeAsset> = Vec::new();
    for id in &ids {
        let t0 = Instant::now();
        let t = generate(lib.get(id).unwrap(), args.seed, &args.params);
        println!(
            "{:14} h {:5.1} m  LOD tris {:>6} / {:>5} / {:>4}  stems {:5} cards {:5} fruit {:4} fronds {:3}  {:.2}s",
            id,
            t.height,
            t.lod_triangles[0],
            t.lod_triangles[1],
            t.lod_triangles[2],
            t.stats.stems,
            t.stats.cards,
            t.stats.fruits,
            t.stats.fronds,
            t0.elapsed().as_secs_f32()
        );
        if std::env::var("TREEGEN_DUMP").is_ok() {
            for (i, tex) in t.scene.textures.iter().enumerate() {
                if let Some(img) = image::RgbaImage::from_raw(tex.width, tex.height, tex.rgba8.to_vec()) {
                    let _ = img.save(args.out.join(format!("{}_tex{i}.png", id)));
                }
            }
        }
        if !args.stats {
            trees.push(t);
        }
    }
    if args.stats {
        return;
    }
    let dl = root.join("assets/downloaded");
    // The renderer keeps every uploaded scene until it is dropped, so each
    // species gets its own and the lineup a fresh one: rendering all of them
    // with one renderer runs out of memory on a software adapter.
    let new_renderer = || {
        let mut r = Renderer::new(&create_instance(), None).expect("renderer");
        if let Ok(img) = forja_assets::load_hdr(&dl.join("polyhaven_sky_kloofendal/kloofendal_48d_partly_cloudy_puresky_2k.hdr")).or_else(|_| forja_assets::load_hdr(&dl.join("hdri_wide_street/wide_street_01_2k.hdr"))) {
            r.set_environment(Some(&env::EnvMap::from_hdr(&img, 14.0)));
        }
        r
    };
    let (w, h) = (args.width, args.width * 9 / 16);
    let mut first = true;
    for t in &mut trees {
        let mut r = new_renderer();
        if first {
            println!("adapter: {:?}", r.adapter_info.name);
            first = false;
        }
        let t0 = Instant::now();
        if args.gpu_impostor {
            let scene = t.lod_scene(0);
            let views: Vec<CutoutView> = impostor::views(&t.billboard).iter().map(|v| CutoutView { center: v.center, dir: v.dir, half: v.half }).collect();
            let shots = capture_cutouts(&mut r, &scene, &views, impostor::TILE as u32);
            let mut canvases: Vec<forja_flora::texture::Canvas> = shots.into_iter().map(|px| forja_flora::texture::Canvas { w: impostor::TILE, h: impostor::TILE, px }).collect();
            // The unlit captures carry albedo and coverage; add the crown's
            // self-shadowing so the billboard matches the lit tree.
            impostor::shade(t, &mut canvases);
            impostor::apply(t, &canvases);
        } else {
            impostor::bake_cpu(t);
        }
        println!("{:14} impostor ({}) {:.1}s", t.species, if args.gpu_impostor { "gpu" } else { "cpu" }, t0.elapsed().as_secs_f32());
        if args.glb {
            let p = args.out.join(format!("{}.glb", t.species));
            forja_flora::gltf::write_glb(t, &p).expect("write glb");
            println!("  {}", p.display());
        }
        let ground = r.upload_scene(&grass(&dl));
        let figure = r.upload_scene(&scale_figure(&dl));
        let mut frame = FrameDesc::default();
        frame.env.shadow_distance = 60.0;
        let model = r.upload_scene(&t.scene);
        let rr = t.crown_radii.x.max(t.crown_radii.z);
        let fig_pos = Vec3::new(t.capsule.radius + 1.6, 0.0, 1.2);
        let base = vec![ground_instance(ground), DrawInstance::new(model, Mat4::IDENTITY)];
        for view in &args.views {
            let mut inst = base.clone();
            let (eye, at, fov) = match view.as_str() {
                "tree" => {
                    inst.push(DrawInstance::new(figure, Mat4::from_rotation_translation(Quat::from_rotation_y(0.3), fig_pos)));
                    let fov = 50f32;
                    let fit_h = t.height * 0.62;
                    let fit_w = rr * 1.25 / (16.0 / 9.0);
                    let d = fit_h.max(fit_w) / (fov * 0.5).to_radians().tan() + rr * 0.5;
                    (Vec3::new(d * 0.35, 1.6, d), Vec3::new(0.0, t.height * 0.47, 0.0), fov)
                }
                "crown" => {
                    let cy = t.crown_center.y;
                    let lo = (cy - t.crown_radii.y).max(1.5);
                    (Vec3::new(rr * 1.0 + 2.2, (lo * 0.75).clamp(1.6, 6.0), rr * 0.35), Vec3::new(rr * 0.35, cy * 0.95, 0.0), 58.0)
                }
                "bark" => {
                    // Stand clear of flares and buttresses.
                    let base = t.scene.meshes[0][0].positions.iter().filter(|p| p.y < 1.2 && p.y > 0.2).map(|p| Vec2::new(p.x, p.z).length()).fold(t.capsule.radius, f32::max).min(t.capsule.radius * 4.0);
                    (Vec3::new(base + 0.9, 1.3, base * 0.4 + 0.5), Vec3::new(0.0, 0.85, 0.0), 52.0)
                }
                "lod" => {
                    // LOD 0, 1 and the billboard side by side.
                    inst.truncate(1);
                    let sp = t.billboard.half * 2.3;
                    for (k, lod) in [0usize, 1, 2].iter().enumerate() {
                        let m = r.upload_scene(&t.lod_scene(*lod));
                        inst.push(DrawInstance::new(m, Mat4::from_translation(Vec3::new((k as f32 - 1.0) * sp, 0.0, 0.0))));
                    }
                    let d = sp * 1.9 / (25f32).to_radians().tan();
                    (Vec3::new(0.0, 1.7, d), Vec3::new(0.0, t.height * 0.45, 0.0), 50.0)
                }
                other => {
                    eprintln!("unknown view {other}");
                    continue;
                }
            };
            frame.instances = inst;
            frame.camera = Camera::look_at(eye, at);
            frame.camera.fov_y = fov.to_radians();
            let t0 = Instant::now();
            let img = r.render_to_image(&frame, w, h);
            let p = args.out.join(format!("{}_{view}.png", t.species));
            img.save(&p).expect("save");
            println!("  {} ({:.1}s, {} tris)", p.display(), t0.elapsed().as_secs_f32(), r.stats.triangles);
        }
    }
    if let Some(dir) = &args.compare {
        for t in &trees {
            compare(&args.out, dir, &t.species);
        }
    }
}

/// All species side by side. Trees are generated one at a time and only
/// their full-detail LOD is uploaded, so just one CPU-side tree is alive.
fn lineup(lib: &SpeciesLibrary, ids: &[String], args: &Args, dl: &Path) {
    let mut r = Renderer::new(&create_instance(), None).expect("renderer");
    if let Ok(img) = forja_assets::load_hdr(&dl.join("polyhaven_sky_kloofendal/kloofendal_48d_partly_cloudy_puresky_2k.hdr")).or_else(|_| forja_assets::load_hdr(&dl.join("hdri_wide_street/wide_street_01_2k.hdr"))) {
        r.set_environment(Some(&env::EnvMap::from_hdr(&img, 14.0)));
    }
    let ground = r.upload_scene(&grass(dl));
    let figure = r.upload_scene(&scale_figure(dl));
    let mut models = Vec::new();
    for id in ids {
        let t = generate(lib.get(id).unwrap(), args.seed, &args.params);
        let wd = t.crown_radii.x.max(t.crown_radii.z).max(1.2) * 1.6 + 1.0;
        models.push((r.upload_scene(&t.lod_scene(0)), wd, t.height));
    }
    let w = args.width;
    let mut frame = FrameDesc::default();
    frame.env.shadow_distance = 160.0;
    frame.instances.push(ground_instance(ground));
    frame.env.fog_density *= 0.35;
    let total: f32 = models.iter().map(|m| m.1).sum();
    let mut x = -total * 0.5;
    let mut hmax: f32 = 0.0;
    for &(m, wd, ht) in &models {
        frame.instances.push(DrawInstance::new(m, Mat4::from_translation(Vec3::new(x + wd * 0.5, 0.0, 0.0))));
        x += wd;
        hmax = hmax.max(ht);
    }
    frame.instances.push(DrawInstance::new(figure, Mat4::from_translation(Vec3::new(-total * 0.5 - 1.0, 0.0, 3.0))));
    let fov = 30f32;
    let d = (total * 0.55 / (16.0 / 9.0)).max(hmax * 0.6) / (fov * 0.5).to_radians().tan();
    frame.camera = Camera::look_at(Vec3::new(0.0, 2.0, d), Vec3::new(0.0, hmax * 0.42, 0.0));
    frame.camera.fov_y = fov.to_radians();
    let img = r.render_to_image(&frame, w * 2, w * 9 / 16);
    let p = args.out.join("lineup.png");
    img.save(&p).expect("save");
    println!("  {}", p.display());
}

/// Side-by-side sheet: our `<id>_tree.png` next to reference photos.
fn compare(out: &Path, photos: &Path, id: &str) {
    let Ok(ours) = image::open(out.join(format!("{id}_tree.png"))) else {
        return;
    };
    let ours = ours.to_rgba8();
    let mut refs: Vec<image::RgbaImage> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(photos) {
        let mut names: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(&format!("{id}_")))).collect();
        names.sort();
        for n in names.iter().take(2) {
            if let Ok(img) = image::open(n) {
                refs.push(img.to_rgba8());
            }
        }
    }
    if refs.is_empty() {
        return;
    }
    let hgt = 540;
    let scale = |img: &image::RgbaImage| {
        let wd = (img.width() as f32 * hgt as f32 / img.height() as f32) as u32;
        image::imageops::resize(img, wd.max(1), hgt, image::imageops::FilterType::Triangle)
    };
    let mut parts = vec![scale(&ours)];
    parts.extend(refs.iter().map(scale));
    let total: u32 = parts.iter().map(|p| p.width() + 8).sum();
    let mut sheet = image::RgbaImage::from_pixel(total, hgt, image::Rgba([20, 20, 20, 255]));
    let mut x = 0;
    for p in &parts {
        image::imageops::overlay(&mut sheet, p, x as i64, 0);
        x += p.width() + 8;
    }
    let path = out.join(format!("{id}_compare.png"));
    sheet.save(&path).expect("save");
    println!("  {}", path.display());
}

/// The flat ground receives shadows but casts none (it can only shadow itself).
fn ground_instance(ground: ModelHandle) -> DrawInstance {
    let mut g = DrawInstance::new(ground, Mat4::IDENTITY);
    g.cast_shadows = false;
    g
}

fn grass(dl: &Path) -> SceneData {
    let mut s = SceneData { name: "grass".into(), ..Default::default() };
    let tex = |f: &str, srgb: bool| forja_assets::load_texture(&dl.join("pbr_grass").join(f), srgb).ok();
    let mut mat = MaterialData { name: "grass".into(), base_color: Vec4::new(0.6, 0.82, 0.48, 1.0), roughness: 1.0, ..Default::default() };
    if let (Some(c), Some(n), Some(a)) = (tex("aerial_grass_rock_color.jpg", true), tex("aerial_grass_rock_normal.jpg", false), tex("aerial_grass_rock_arm.jpg", false)) {
        s.textures = vec![c, n, a];
        mat.base_color_tex = Some(0);
        mat.normal_tex = Some(1);
        mat.metallic_roughness_tex = Some(2);
        mat.occlusion_tex = Some(2);
    } else {
        mat.base_color = Vec4::new(0.12, 0.2, 0.07, 1.0);
    }
    s.materials.push(mat);
    // A big grid (several quads keep the culling and shadows happy).
    let n = 16;
    let size = 400.0;
    let tile = 5.0;
    let mut m = MeshData::default();
    for j in 0..=n {
        for i in 0..=n {
            let x = -size * 0.5 + size * i as f32 / n as f32;
            let z = -size * 0.5 + size * j as f32 / n as f32;
            m.positions.push(Vec3::new(x, 0.0, z));
            m.normals.push(Vec3::Y);
            m.uvs.push(Vec2::new(x / tile, z / tile));
        }
    }
    for j in 0..n as u32 {
        for i in 0..n as u32 {
            let a = j * (n as u32 + 1) + i;
            let b = a + 1;
            let c = a + n as u32 + 1;
            let d = c + 1;
            m.indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    m.finalize();
    s.meshes = vec![vec![m]];
    s.nodes = vec![NodeData { name: "grass".into(), mesh: Some(0), ..Default::default() }];
    s.roots = vec![0];
    s
}

/// 1.75 m scale figure: a MakeHuman character when available, else a
/// neutral grey mannequin.
fn scale_figure(dl: &Path) -> SceneData {
    if let Some(spec) = forja_assets::makehuman::HumanSpec::presets().into_iter().find(|s| s.name == "casual_male")
        && let Ok(mut s) = forja_assets::makehuman::build_human(&dl.join("makehuman_base"), &dl.join("makehuman_assets"), &spec)
    {
        let b = s.bounds();
        let k = 1.75 / (b.max.y - b.min.y).max(0.1);
        for n in &mut s.nodes {
            if n.parent.is_none() {
                n.local.s *= k;
                n.local.t = n.local.t * k - Vec3::Y * b.min.y * k;
            }
        }
        return s;
    }
    let mut s = SceneData { name: "mannequin".into(), ..Default::default() };
    s.materials.push(MaterialData { name: "mannequin".into(), base_color: Vec4::new(0.55, 0.55, 0.55, 1.0), roughness: 0.6, alpha: AlphaMode::Opaque, ..Default::default() });
    let mut prims = Vec::new();
    for (c, r) in [(Vec3::new(-0.1, 0.45, 0.0), Vec3::new(0.08, 0.45, 0.08)), (Vec3::new(0.1, 0.45, 0.0), Vec3::new(0.08, 0.45, 0.08)), (Vec3::new(0.0, 1.2, 0.0), Vec3::new(0.2, 0.32, 0.12)), (Vec3::new(0.0, 1.64, 0.0), Vec3::new(0.1, 0.12, 0.1))] {
        let mut m = MeshData::default();
        let (rings, segs) = (8, 12);
        for i in 0..=rings {
            let phi = i as f32 / rings as f32 * std::f32::consts::PI;
            for j in 0..=segs {
                let th = j as f32 / segs as f32 * std::f32::consts::TAU;
                let d = Vec3::new(phi.sin() * th.cos(), phi.cos(), phi.sin() * th.sin());
                m.positions.push(c + d * r);
            }
        }
        for i in 0..rings as u32 {
            for j in 0..segs as u32 {
                let a = i * (segs as u32 + 1) + j;
                m.indices.extend_from_slice(&[a, a + 1, a + segs as u32 + 1, a + 1, a + segs as u32 + 2, a + segs as u32 + 1]);
            }
        }
        m.finalize();
        prims.push(m);
    }
    s.meshes = vec![prims];
    s.nodes = vec![NodeData { name: "mannequin".into(), mesh: Some(0), ..Default::default() }];
    s.roots = vec![0];
    s
}
