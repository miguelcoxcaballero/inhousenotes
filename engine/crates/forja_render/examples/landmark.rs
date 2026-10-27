//! Landmark tool: check, render and compare the `landmark.toml` specs.
//!
//! ```text
//! landmark check   <slug-or-dir>                       JSON metrics (fast, no GPU)
//! landmark render  <slug-or-dir> [--out DIR] [--views std|photo|all]
//!                                [--view NAME|INDEX] [--width N]
//!                                [--context on|off] [--clay] [--edges-only]
//! landmark gallery [DIR...] [--out DIR] [--width N] [--samples]
//! landmark init    <slug-or-dir> [--force]             draft landmark.toml from footprint.json
//! ```
//! `--clay` renders untextured (no texture generation, a few seconds);
//! `--edges-only` skips the GPU and draws the model's edges over each
//! photo (about a second): the quick loop for aligning a `[[view]]`.
//! `--view` also takes a standard view (`ne`, `se`, `sw`, `nw`, `street`,
//! `plan`) to render only that one.
//! Build: `cargo build --release -p forja_render --example landmark`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use forja_assets::SceneData;
use forja_city::kit::Kit;
use forja_city::landmark::camera::{self as lcam, Cam};
use forja_city::landmark::spec::{CtxRef, Focal, ShiftSpec, ViewSpec};
use forja_city::landmark::{self, AssembleOptions, Library, Model, Source, footprint::FpPoly, mesh, palette::Palette, poly, raster, spec::EdgeSel};
use forja_city::texgen::Canvas;
use forja_math::{Mat4, Quat, Vec2, Vec3};
use forja_render::*;
use image::{Rgba, RgbaImage};

fn engine_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

struct Args {
    cmd: String,
    targets: Vec<String>,
    out: Option<PathBuf>,
    views: String,
    view: Option<String>,
    width: u32,
    context: bool,
    force: bool,
    samples: bool,
    clay: bool,
    edges_only: bool,
}

impl Args {
    fn target(&self) -> Option<&str> {
        self.targets.first().map(|s| s.as_str())
    }
}

fn parse_args() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let cmd = it.next().ok_or_else(usage)?;
    if !["check", "render", "gallery", "init"].contains(&cmd.as_str()) {
        return Err(format!("unknown command `{cmd}`\n{}", usage()));
    }
    let mut a = Args { cmd, targets: Vec::new(), out: None, views: "all".into(), view: None, width: 1280, context: true, force: false, samples: false, clay: false, edges_only: false };
    while let Some(x) = it.next() {
        match x.as_str() {
            "--out" => a.out = Some(PathBuf::from(it.next().ok_or("--out needs a folder")?)),
            "--views" => a.views = it.next().ok_or("--views needs std|photo|all")?,
            "--view" => a.view = Some(it.next().ok_or("--view needs a view name or index")?),
            "--width" => a.width = it.next().ok_or("--width needs a number")?.parse().map_err(|_| "bad --width")?,
            "--context" => {
                let v = it.next().ok_or("--context needs on|off")?;
                a.context = match v.as_str() {
                    "on" => true,
                    "off" => false,
                    o => return Err(format!("--context takes on or off, not `{o}`")),
                };
            }
            "--force" => a.force = true,
            "--samples" => a.samples = true,
            "--clay" | "--fast" => a.clay = true,
            "--edges-only" | "--edges" => a.edges_only = true,
            o if o.starts_with("--") => return Err(format!("unknown option {o}\n{}", usage())),
            o => a.targets.push(o.to_string()),
        }
    }
    if !["std", "photo", "all"].contains(&a.views.as_str()) {
        return Err("--views must be std, photo or all".into());
    }
    if a.cmd != "gallery" && a.targets.len() > 1 {
        return Err(format!("`{}` takes one landmark, got {}: {}", a.cmd, a.targets.len(), a.targets.join(" ")));
    }
    if a.view.is_some() || a.edges_only {
        a.views = "photo".into();
    }
    a.width = a.width.clamp(256, 4096);
    Ok(a)
}

fn usage() -> String {
    "usage: landmark check <slug|dir> | render <slug|dir> [--out DIR] [--views std|photo|all] [--view NAME|INDEX] [--width N] [--context on|off] [--clay] [--edges-only] | gallery [DIR...] [--out DIR] [--samples] | init <slug|dir> [--force]".into()
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let root = engine_root();
    let res = match args.cmd.as_str() {
        "check" => check(&args, &root),
        "render" => render(&args, &root),
        "gallery" => gallery(&args, &root),
        "init" => init(&args, &root),
        _ => Err(usage()),
    };
    if let Err(e) = res {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn locate(args: &Args, root: &Path) -> Result<PathBuf, String> {
    let t = args.target().ok_or("missing <slug-or-dir>")?;
    landmark::locate(t, root).ok_or_else(|| format!("no landmark `{t}` (looked for a folder, data/landmarks/valencia/{t}, data/landmarks/_samples/{t})"))
}

// ------------------------------------------------------------------ check

fn check(args: &Args, root: &Path) -> Result<(), String> {
    let dir = locate(args, root)?;
    let t0 = Instant::now();
    let src = match landmark::read(&dir) {
        Ok(s) => s,
        Err(e) => {
            println!("{}", serde_json::to_string_pretty(&serde_json::json!({ "ok": false, "error": e.to_string() })).unwrap());
            return Err("spec does not parse".into());
        }
    };
    let model = match landmark::build_model(&src) {
        Ok(m) => m,
        Err(e) => {
            println!("{}", serde_json::to_string_pretty(&serde_json::json!({ "ok": false, "error": e.to_string() })).unwrap());
            return Err("spec does not build".into());
        }
    };
    let ms = t0.elapsed().as_secs_f32() * 1000.0;
    let mut v = landmark::metrics::compute(&model, src.footprint.as_ref(), &src.slug(), ms);
    v["name"] = serde_json::json!(src.spec.landmark.name);
    v["views"] = serde_json::json!(src.spec.view.len());
    // Cameras solved from point matches, ready to paste, and problems
    // with the view cameras.
    let (hidden, hide_warn) = hidden_context(&src);
    let mut extra = hide_warn;
    let mut solved: Vec<serde_json::Value> = Vec::new();
    for (k, vs) in src.spec.view.iter().enumerate() {
        let vc = view_camera(&src, vs, k);
        let label = format!("view {k} ({})", view_name(vs, k));
        if let Some(vc) = &vc {
            if let Some((ci, h)) = camera_in_context(&src, &hidden, vc.cam.eye)
                && vs.context.unwrap_or(true)
            {
                extra.push(inside_context_warning(&label, ci, h));
            }
            for n in &vc.notes {
                extra.push(format!("{label}: {n}"));
            }
            if let Some(s) = &vc.solved {
                for w in &s.warnings {
                    extra.push(format!("{label}: {w}"));
                }
            }
        }
        if vs.matches.is_empty() {
            continue;
        }
        solved.push(match &vc {
            Some(vc) if vc.rms.is_some() => solved_json(&src, k, vc),
            _ => serde_json::json!({ "view": k, "error": "needs at least 4 matches and the photo size" }),
        });
    }
    if !solved.is_empty() {
        v["solved_views"] = serde_json::json!(solved);
    }
    if !extra.is_empty()
        && let Some(w) = v["warnings"].as_array_mut()
    {
        w.extend(extra.into_iter().map(serde_json::Value::String));
    }
    v["check_ms"] = serde_json::json!((t0.elapsed().as_secs_f32() * 1000.0).round());
    println!("{}", serde_json::to_string_pretty(&v).unwrap());
    Ok(())
}

// ------------------------------------------------------------------ init

fn init(args: &Args, root: &Path) -> Result<(), String> {
    let t = args.target().ok_or("missing <slug-or-dir>")?;
    let dir = landmark::locate(t, root).unwrap_or_else(|| root.join("data/landmarks/valencia").join(t));
    let fp_path = dir.join("footprint.json");
    let fp = if fp_path.exists() { Some(landmark::Footprint::load(&fp_path).map_err(|e| e.to_string())?) } else { None };
    let src = dir.join("sources.toml");
    let sources = if src.exists() { landmark::Sources::load(&src).ok() } else { None };
    let slug = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let text = landmark::scaffold::draft(&slug, fp.as_ref(), sources.as_ref());
    let out = dir.join("landmark.toml");
    if out.exists() && !args.force {
        println!("{text}");
        eprintln!("{} exists: draft printed instead (use --force to overwrite)", out.display());
    } else {
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        std::fs::write(&out, text).map_err(|e| e.to_string())?;
        eprintln!("wrote {}", out.display());
    }
    Ok(())
}

// --------------------------------------------------------------- scenes

/// How scenes get their materials: the texture library or flat clay.
#[derive(Clone, Copy)]
enum Look<'a> {
    Lib(&'a Library<'a>),
    Clay,
}

fn assemble_with(look: Look, model: &Model, name: &str, lods: bool) -> SceneData {
    match look {
        Look::Lib(lib) => landmark::assemble(model, lib, name, AssembleOptions { lods }),
        Look::Clay => landmark::assemble_clay(model, name),
    }
}

/// Assembles plain geometry with a few built-in materials.
fn quick_scene(look: Look, name: &str, parts: &[(&str, forja_city::geom::Prim)]) -> SceneData {
    let mut pal = Palette::new(&BTreeMap::new()).unwrap();
    let mut geo = mesh::Geo::new();
    for (m, p) in parts {
        let id = pal.id(m).unwrap();
        geo.get(id).append(p, &Mat4::IDENTITY);
    }
    let model = Model { geo, collider: Vec::new(), col_groups: Vec::new(), palette: pal, stats: Vec::new(), warnings: Vec::new(), ground_cuts: Vec::new(), reference_heights: Vec::new() };
    assemble_with(look, &model, name, false)
}

/// Height of a context building's massing (floors x 3.2 m).
fn context_height(c: &FpPoly) -> f32 {
    c.floors_above.unwrap_or(4.0).max(1.0) * 3.2 + 0.6
}

/// Which context buildings `[render] hide_context` hides, and problems
/// with the list.
fn hidden_context(src: &Source) -> (Vec<bool>, Vec<String>) {
    let Some(fp) = &src.footprint else { return (Vec::new(), Vec::new()) };
    let mut hide = vec![false; fp.context.len()];
    let mut warn = Vec::new();
    for r in &src.spec.render.hide_context {
        match r {
            CtxRef::Index(i) if *i < hide.len() => hide[*i] = true,
            CtxRef::Index(i) => warn.push(format!("[render] hide_context: there is no context:{i} (the footprint has {} context buildings)", hide.len())),
            CtxRef::Ref(t) => {
                let arg = t.trim().strip_prefix("context:").unwrap_or(t.trim());
                match fp.resolve(&format!("context:{arg}")) {
                    Ok(list) => {
                        for p in list {
                            if let Some(k) = fp.context.iter().position(|c| std::ptr::eq(c, p)) {
                                hide[k] = true;
                            }
                        }
                    }
                    Err(e) => warn.push(format!("[render] hide_context: {e}")),
                }
            }
        }
    }
    (hide, warn)
}

/// The visible context building (index, height) a camera stands inside.
fn camera_in_context(src: &Source, hidden: &[bool], eye: Vec3) -> Option<(usize, f32)> {
    let fp = src.footprint.as_ref()?;
    fp.context.iter().enumerate().filter(|(k, _)| !hidden.get(*k).copied().unwrap_or(false)).find(|(_, c)| eye.y < context_height(c) && c.shape.contains(Vec2::new(eye.x, eye.z))).map(|(k, c)| (k, context_height(c)))
}

fn inside_context_warning(what: &str, k: usize, h: f32) -> String {
    format!("{what}: the camera is inside context building context:{k} ({h:.1} m high): hide it with [render] hide_context = [{k}] or move the eye")
}

/// Neighbouring buildings as light massing (floors x 3.2 m).
fn context_scene(look: Look, ctx: &[FpPoly], hidden: &[bool], shift: Vec2) -> Option<SceneData> {
    if ctx.iter().enumerate().all(|(k, _)| hidden.get(k).copied().unwrap_or(false)) {
        return None;
    }
    let mut walls = forja_city::geom::Prim::default();
    let mut roofs = forja_city::geom::Prim::default();
    for (k, c) in ctx.iter().enumerate() {
        if hidden.get(k).copied().unwrap_or(false) {
            continue;
        }
        let h = context_height(c);
        let s = c.shape.map(|p| p + shift);
        mesh::shape_walls(&mut walls, &s, 0.0, h);
        mesh::horizontal_shape(&mut roofs, &s, h, true);
    }
    Some(quick_scene(look, "context", &[("context", walls), ("concrete", roofs)]))
}

/// Paving around the building and asphalt beyond, open over the
/// model's ground cuts (moats, sunken plazas).
fn ground_scene(look: Look, center: Vec2, radius: f32, cuts: &[poly::Shape]) -> SceneData {
    let mut pave = forja_city::geom::Prim::default();
    let r = radius.max(20.0);
    let sq = poly::Shape::new(poly::rect(center, Vec2::splat(r * 2.0), 0.0));
    if cuts.is_empty() {
        mesh::horizontal_shape(&mut pave, &sq, 0.0, true);
    } else {
        for piece in poly::boolean(std::slice::from_ref(&sq), cuts, poly::BoolOp::Difference) {
            mesh::horizontal_shape(&mut pave, &piece, 0.0, true);
        }
    }
    let mut asphalt = forja_city::geom::Prim::default();
    let big = poly::Shape { outer: poly::rect(center, Vec2::splat((r * 8.0).max(4000.0)), 0.0), holes: vec![poly::rect(center, Vec2::splat(r * 2.0), 0.0).into_iter().rev().collect()] };
    mesh::horizontal_shape(&mut asphalt, &big, -0.02, true);
    quick_scene(look, "ground", &[("granite", pave), ("asphalt", asphalt)])
}

// ------------------------------------------------------------- rendering

/// Sky and light of a render.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Light {
    /// HDRI image lighting (world rotated so the HDRI sun lands on the
    /// requested azimuth).
    Hdri,
    /// Analytic sky following the sun (dusk, night, low sun).
    Atmosphere,
}

struct Studio {
    r: Renderer,
    env: Option<env::EnvMap>,
    /// HDRI sun as authored.
    sun0: Vec3,
    elevation: Option<f32>,
    lights: Vec<PointLight>,
}

/// HDRI file for a `[render] sky` choice.
fn sky_file(dl: &Path, sky: &str) -> Option<PathBuf> {
    let clear = dl.join("polyhaven_sky_kloofendal/kloofendal_48d_partly_cloudy_puresky_2k.hdr");
    let street = dl.join("hdri_wide_street/wide_street_01_2k.hdr");
    match sky {
        "atmosphere" => None,
        "street" => Some(street).filter(|p| p.exists()),
        _ => Some(clear).filter(|p| p.exists()).or(Some(street).filter(|p| p.exists())),
    }
}

impl Studio {
    fn new(dl: &Path, sky: &str, lights: Vec<PointLight>) -> Result<Studio, String> {
        let mut r = Renderer::new(&create_instance(), None)?;
        let env = sky_file(dl, sky).and_then(|p| forja_assets::load_hdr(&p).ok()).map(|img| env::EnvMap::from_hdr(&img, 14.0));
        if let Some(e) = &env {
            r.set_environment(Some(e));
        }
        let sun0 = env.as_ref().map(|e| e.sun_dir).unwrap_or(Environment::default().sun_dir);
        Ok(Studio { r, env, sun0, elevation: None, lights })
    }

    /// World rotation that puts the HDRI sun at compass azimuth `az` (and
    /// elevation `el` when given).
    fn sun(&mut self, az: f32, el: Option<f32>) -> Mat4 {
        let az0 = poly::azimuth(Vec2::new(self.sun0.x, self.sun0.z));
        let el0 = self.sun0.y.asin().to_degrees();
        let want_el = el.unwrap_or(el0).clamp(3.0, 88.0);
        if self.elevation.is_none_or(|e| (e - want_el).abs() > 0.1) {
            if let Some(e) = &mut self.env {
                let h = Vec2::new(self.sun0.x, self.sun0.z).normalize_or(Vec2::X) * want_el.to_radians().cos();
                e.sun_dir = Vec3::new(h.x, want_el.to_radians().sin(), h.y).normalize();
                self.r.set_environment(Some(e));
            }
            self.elevation = Some(want_el);
        }
        // rotY(t) maps azimuth a to a - t: we need world azimuth `az` to
        // land on the HDRI's az0.
        Mat4::from_rotation_y((az - az0).to_radians())
    }
}

/// Sun elevation and exposure of a time of day.
fn time_of_day(t: &str) -> Option<(f32, f32)> {
    match t {
        "morning" => Some((14.0, 0.6)),
        "evening" => Some((9.0, 0.7)),
        "dusk" => Some((2.5, 1.6)),
        "night" => Some((-8.0, 3.5)),
        _ => None,
    }
}

#[derive(Clone)]
struct Shot {
    #[allow(dead_code)]
    name: String,
    eye: Vec3,
    target: Vec3,
    fov_y: f32,
    roll: f32,
    w: u32,
    h: u32,
    sun: (f32, Option<f32>),
    plan: bool,
    light: Light,
    exposure: Option<f32>,
    /// Lens shift (fractions of the image; see `Cam::shift`).
    shift: Vec2,
}

fn camera(s: &Shot, world: &Mat4) -> Camera {
    let eye = world.transform_point3(s.eye);
    let target = world.transform_point3(s.target);
    let mut c = Camera::look_at(eye, target);
    if s.roll != 0.0 {
        c.rotation *= Quat::from_rotation_z(s.roll.to_radians());
    }
    c.fov_y = s.fov_y.to_radians();
    c.aspect = s.w as f32 / s.h as f32;
    if s.plan {
        let d = (eye - target).length();
        c.near = (d - 600.0).max(1.0);
    }
    c
}

fn render_shot(st: &mut Studio, handles: &[(ModelHandle, Mat4)], s: &Shot) -> (RgbaImage, Mat4) {
    if s.shift != Vec2::ZERO {
        // Lens shift: render a larger centred frame and crop the shifted
        // window out of it.
        let (fx, fy) = (1.0 + 2.0 * s.shift.x.abs(), 1.0 + 2.0 * s.shift.y.abs());
        let (bw, bh) = (((s.w as f32 * fx).round() as u32).clamp(s.w, 8192), ((s.h as f32 * fy).round() as u32).clamp(s.h, 8192));
        let fov_big = 2.0 * ((s.fov_y.to_radians() * 0.5).tan() * bh as f32 / s.h as f32).atan().to_degrees();
        let big = Shot { w: bw, h: bh, fov_y: fov_big, shift: Vec2::ZERO, ..s.clone() };
        let (img, vp_big) = render_shot(st, handles, &big);
        let x0 = (bw as f32 * 0.5 + s.shift.x * s.w as f32 - s.w as f32 * 0.5).round().clamp(0.0, (bw - s.w) as f32) as u32;
        let y0 = (bh as f32 * 0.5 - s.shift.y * s.h as f32 - s.h as f32 * 0.5).round().clamp(0.0, (bh - s.h) as f32) as u32;
        let crop = image::imageops::crop_imm(&img, x0, y0, s.w, s.h).to_image();
        let (w, h) = (s.w as f32, s.h as f32);
        let to_crop = Mat4::from_translation(Vec3::new((bw as f32 - 2.0 * x0 as f32 - w) / w, 1.0 - bh as f32 / h + 2.0 * y0 as f32 / h, 0.0)) * Mat4::from_scale(Vec3::new(bw as f32 / w, bh as f32 / h, 1.0));
        return (crop, to_crop * vp_big);
    }
    let world = match s.light {
        Light::Hdri if st.env.is_some() => st.sun(s.sun.0, s.sun.1),
        _ => Mat4::IDENTITY,
    };
    let cam = camera(s, &world);
    let mut frame = FrameDesc { camera: cam, ..Default::default() };
    for (h, m) in handles {
        frame.instances.push(DrawInstance::new(*h, world * *m));
    }
    if s.light == Light::Atmosphere || st.env.is_none() {
        let el = s.sun.1.unwrap_or(40.0).to_radians();
        let d = poly::from_azimuth(s.sun.0);
        frame.env.use_ibl = false;
        frame.env.sun_dir = Vec3::new(d.x * el.cos(), el.sin(), d.y * el.cos()).normalize();
        frame.env.sun_illuminance *= el.sin().clamp(0.0, 1.0).powf(0.6);
    }
    // Specs give a linear exposure relative to the old fixed camera (0.55).
    // In daylight the physical camera meters itself and takes it as
    // compensation; a time of day keeps its hand-tuned exposure.
    if let Some(e) = s.exposure {
        if s.sun.1.is_some() && s.light == Light::Atmosphere {
            frame.env.set_linear_exposure(e);
        } else {
            frame.env.exposure_compensation += (e.max(1e-3) / 0.55).log2();
        }
    }
    for l in &st.lights {
        frame.lights.push(PointLight { position: world.transform_point3(l.position), ..*l });
    }
    if s.plan {
        frame.env.fog_density = 0.0;
        frame.env.vignette = 0.0;
        frame.env.shadow_distance = (s.eye - s.target).length() + 600.0;
    } else {
        frame.env.shadow_distance = ((s.eye - s.target).length() * 2.5).clamp(80.0, 600.0);
    }
    let img = st.r.render_to_image(&frame, s.w, s.h);
    (img, cam.view_proj() * world)
}

// ---------------------------------------------------------- photo views

/// A view's camera: as written, from sources.toml, or solved from matches.
struct ViewCam {
    cam: Cam,
    /// Photo size used for the aspect (and the match pixels).
    size: (u32, u32),
    /// RMS reprojection error when solved from matches.
    rms: Option<f32>,
    /// Solver diagnostics (per-point errors, warnings).
    solved: Option<lcam::Solved>,
    /// Parameters held fixed while solving.
    fixed: Vec<&'static str>,
    /// Problems with the view's camera settings.
    notes: Vec<String>,
}

/// Photo size from the image file or sources.toml.
fn photo_size(src: &Source, v: &ViewSpec) -> Option<(u32, u32)> {
    let p = v.photo.as_ref()?;
    if let Ok(d) = image::image_dimensions(src.photo_path(p)) {
        return Some(d);
    }
    let m = src.sources.as_ref()?.photo(p)?;
    Some((m.width?, m.height?))
}

/// 35 mm focal length from the photo's EXIF.
fn photo_exif_focal(src: &Source, v: &ViewSpec) -> Option<f32> {
    let p = v.photo.as_ref()?;
    let mut f = std::fs::File::open(src.photo_path(p)).ok()?;
    let mut buf = Vec::new();
    std::io::Read::read_to_end(&mut std::io::Read::take(&mut f, 512 * 1024), &mut buf).ok()?;
    lcam::exif_focal_35mm(&buf)
}

fn view_camera(src: &Source, v: &ViewSpec, k: usize) -> Option<ViewCam> {
    let size = photo_size(src, v).unwrap_or((4, 3));
    let mut notes = Vec::new();
    let aspect = size.0 as f32 / size.1.max(1) as f32;
    let cam_src = v.photo.as_ref().and_then(|p| src.sources.as_ref().and_then(|s| s.photo(p))).and_then(|m| m.camera.map(|c| (c, m.heading)));
    let eye = match (v.eye, cam_src) {
        (Some(e), _) => Some(Vec3::from_array(e)),
        (None, Some((c, _))) => Some(Vec3::new(c.x, 1.6, c.y)),
        _ => None,
    };
    let target = match (v.target, cam_src.and_then(|c| c.1), eye) {
        (Some(t), _, _) => Some(Vec3::from_array(t)),
        (None, Some(hd), Some(e)) => {
            let d = poly::from_azimuth(hd);
            Some(e + Vec3::new(d.x, 0.0, d.y) * 50.0 + Vec3::Y * 8.0)
        }
        _ => None,
    };
    let exif = match v.focal_35mm {
        Some(Focal::Exif) => {
            let f = photo_exif_focal(src, v);
            if f.is_none() {
                notes.push("focal_35mm = \"exif\": the photo has no 35 mm focal length in its EXIF; the field of view is solved / defaulted".to_string());
            }
            f
        }
        _ => None,
    };
    let given_fov = match (v.fov_y, v.focal_35mm, exif) {
        (Some(f), _, _) => Some(f),
        (None, Some(Focal::Mm(f)), _) => Some(lcam::fov_from_focal(f, aspect)),
        (None, Some(Focal::Exif), Some(f)) => Some(lcam::fov_from_focal(f, aspect)),
        _ => None,
    };
    let fov_y = given_fov.unwrap_or(55.0);
    // Constraints for `match` (also imposed on a written camera).
    let has = |n: &str| v.fix.iter().any(|f| f == n);
    let mut k_ = lcam::Constraints::default();
    let mut fixed: Vec<&'static str> = Vec::new();
    if has("fov") || exif.is_some() {
        match given_fov {
            Some(f) => {
                k_.fov = Some(f);
                fixed.push("fov");
            }
            None if has("fov") => notes.push("fix = [\"fov\"] needs fov_y or focal_35mm".into()),
            None => {}
        }
    }
    if has("eye") {
        match eye {
            Some(e) => {
                k_.eye = Some(e);
                fixed.push("eye");
            }
            None => notes.push("fix = [\"eye\"] needs `eye` (or a camera in sources.toml)".into()),
        }
    }
    if has("height") && k_.eye.is_none() {
        k_.eye_y = Some(v.eye_height.or(eye.map(|e| e.y)).unwrap_or(1.6));
        fixed.push("height");
    } else if let Some(hh) = v.eye_height {
        k_.eye_height = Some((hh, v.eye_height_sigma.unwrap_or(0.3)));
    }
    if v.level || v.upright || has("roll") {
        k_.roll = Some(if v.level || v.upright { 0.0 } else { v.roll });
        fixed.push("roll");
    }
    if v.upright || has("pitch") {
        let written_pitch = match (eye, target) {
            (Some(e), Some(t)) => Cam::new(e, t, fov_y, 0.0).pitch(),
            _ => 0.0,
        };
        k_.pitch = Some(if v.upright { 0.0 } else { written_pitch });
        fixed.push("pitch");
    }
    match v.shift {
        Some(ShiftSpec::Fixed(sh)) => k_.shift = Vec2::from_array(sh),
        Some(ShiftSpec::Solve) => k_.solve_shift = [true, true],
        None if v.upright => k_.solve_shift = [false, true],
        None => {}
    }
    let written = eye.map(|e| k_.apply(Cam::new(e, target.unwrap_or(Vec3::new(0.0, 8.0, 0.0)), fov_y, v.roll)));
    let min_pts = if k_ == lcam::Constraints::default() { 4 } else { 2 };
    if v.matches.len() >= min_pts && size != (4, 3) {
        let m: Vec<(Vec2, Vec3)> = v.matches.iter().map(|p| (Vec2::from_array(p.px), Vec3::from_array(p.world))).collect();
        let hints: Vec<Cam> = written.into_iter().collect();
        if let Some(sol) = lcam::solve_with(&m, size.0 as f32, size.1 as f32, &hints, &k_) {
            return Some(ViewCam { cam: sol.cam, size, rms: Some(sol.rms), solved: Some(sol), fixed, notes });
        }
        notes.push(format!("`match` has too few points ({}) for the free camera parameters; using eye/target", v.matches.len()));
    } else if !v.matches.is_empty() {
        eprintln!("view {k}: `match` needs at least 4 points (and the photo size); using eye/target");
    }
    written.map(|cam| ViewCam { cam, size, rms: None, solved: None, fixed, notes })
}

/// JSON of a solved view: the camera ready to paste plus diagnostics.
fn solved_json(src: &Source, k: usize, vc: &ViewCam) -> serde_json::Value {
    let v = &src.spec.view[k];
    let c = vc.cam;
    let mut j = serde_json::json!({ "view": k, "rms_px": vc.rms, "eye": c.eye.to_array(), "target": c.target.to_array(), "fov_y": c.fov_y, "roll": c.roll });
    if c.shift != Vec2::ZERO {
        j["shift"] = serde_json::json!(c.shift.to_array());
    }
    if !vc.fixed.is_empty() {
        j["fixed"] = serde_json::json!(vc.fixed);
    }
    if let Some(s) = &vc.solved {
        j["planarity"] = serde_json::json!((s.planarity * 1000.0).round() / 1000.0);
        let pts: Vec<serde_json::Value> = v
            .matches
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let mut p = serde_json::json!({ "i": i, "px": m.px, "world": m.world, "error_px": (s.errors[i] * 10.0).round() / 10.0 });
                if let Some(q) = s.reprojected[i] {
                    p["reprojected"] = serde_json::json!([(q.x * 10.0).round() / 10.0, (q.y * 10.0).round() / 10.0]);
                }
                if let Some(n) = &m.note {
                    p["note"] = serde_json::json!(n);
                }
                p
            })
            .collect();
        j["points"] = serde_json::json!(pts);
        if !s.warnings.is_empty() {
            j["warnings"] = serde_json::json!(s.warnings);
        }
    }
    if !vc.notes.is_empty() {
        j["notes"] = serde_json::json!(vc.notes);
    }
    j
}

fn view_name(v: &ViewSpec, k: usize) -> String {
    v.name.clone().unwrap_or_else(|| format!("view{k}"))
}

/// Edges of the model seen from a camera (CPU rasterizer).
fn model_edges(tris: &[([Vec3; 3], u32)], vp: Mat4, w: u32, h: u32) -> Vec<f32> {
    let ras = raster::rasterize(tris.iter().copied(), vp, w as usize, h as usize);
    thicken(&raster::edges(&ras, 32.0), w as usize, h as usize)
}

/// White-on-dark drawing of the edges (the "render" panel of a fast compare).
fn edges_image(e: &[f32], w: u32, h: u32) -> RgbaImage {
    let mut img = RgbaImage::from_pixel(w, h, Rgba([28, 30, 34, 255]));
    for (i, p) in img.pixels_mut().enumerate() {
        if e[i] > 0.0 {
            let v = if e[i] >= 0.85 { [255, 230, 60] } else { [150, 220, 255] };
            *p = Rgba([v[0], v[1], v[2], 255]);
        }
    }
    img
}

// ----------------------------------------------------------- street view

/// Street-level camera for the main facade: far enough to frame the
/// facade, never inside or behind the buildings across the street; in a
/// narrow street a view along it, as people photograph alleys.
fn street_shot(src: &Source, face: &landmark::runs::Bend, height: f32, w: u32, h: u32, hidden: &[bool]) -> (Vec3, Vec3, f32, String) {
    let r = &src.spec.render;
    if let (Some(e), Some(t)) = (r.street_eye, r.street_target) {
        return (Vec3::from_array(e), Vec3::from_array(t), r.street_fov.unwrap_or(62.0), "street_eye / street_target".into());
    }
    let fp = src.footprint.as_ref();
    let obstacles: Vec<&poly::Shape> = fp.map(|f| f.context.iter().enumerate().filter(|(k, _)| !hidden.get(*k).copied().unwrap_or(false)).map(|(_, c)| c).chain(f.outlines.iter()).map(|c| &c.shape).collect()).unwrap_or_default();
    let free_along = |o: Vec2, d: Vec2| -> f32 { obstacles.iter().filter_map(|s| landmark::runs::ray_hit(s, o, d, 0.05).map(|hh| hh.0)).fold(250.0, f32::min) };
    let inside = |p: Vec2| obstacles.iter().any(|s| s.contains(p));
    let len = face.len();
    let (fc, fnrm) = face.frame(len * 0.5);
    let dfree = free_along(fc + fnrm * 0.05, fnrm);
    let aspect = w as f32 / h as f32;
    let fov = r.street_fov.unwrap_or(60.0);
    let hfov = 2.0 * ((fov.to_radians() * 0.5).tan() * aspect).atan();
    let chord = (face.pts[face.pts.len() - 1] - face.pts[0]).length().max(len * 0.6);
    let fit_w = chord * 0.55 / (hfov * 0.5).tan();
    let fit_h = height.min(80.0) * 0.95;
    let fit = r.street_distance.unwrap_or(fit_w.max(fit_h).clamp(12.0, 120.0));
    let look = (height.min(60.0) * 0.42).clamp(3.0, 25.0);
    // Enough room to stand back (a square, a wide street): as far as the
    // facade needs or the space allows, widening the lens for tall
    // buildings rather than giving up on the frontal view.
    if dfree - 0.5 >= (fit * 0.8).min(9.0) || r.street_distance.is_some() {
        let d = fit.min(dfree - 0.5).max(1.0);
        let eye = fc + fnrm * d;
        let mut fov = fov;
        let mut look = look;
        if r.street_fov.is_none() {
            // Aim at the middle of the angular span from the foot of the
            // facade to the top, and open the lens just enough for it.
            let top = (height.min(80.0) - 1.6).max(0.0).atan2(d);
            let bot = (-1.6f32).atan2(d);
            let mid = (top + bot) * 0.5;
            let need_v = (top - bot).to_degrees() * 1.08;
            let need_h = 2.0 * ((chord * 0.55).atan2(d)).to_degrees() * 1.05;
            let need_h_as_v = 2.0 * ((need_h.to_radians() * 0.5).tan() / aspect).atan().to_degrees();
            fov = fov.max(need_v).max(need_h_as_v).min(100.0);
            if need_v > 60.0 {
                look = 1.6 + d * mid.tan();
            }
        }
        return (Vec3::new(eye.x, 1.6, eye.y), Vec3::new(fc.x, look, fc.y), fov, format!("{d:.1} m in front of the main facade, fov {fov:.0}"));
    }
    // Narrow street: stand in the street towards one end of the facade, at
    // most 2 m out, and look along the street at the other half.
    for (fe, ft) in [(0.12f32, 0.62f32), (0.88, 0.38), (0.25, 0.7), (0.75, 0.3)] {
        let (pe, ne) = face.frame(len * fe);
        let free = free_along(pe + ne * 0.05, ne);
        let eye = pe + ne * (free * 0.55).clamp(0.6, 2.0);
        if inside(eye) {
            continue;
        }
        let (pt, _) = face.frame(len * ft);
        let tgt = Vec3::new(pt.x, look.min(height * 0.5), pt.y);
        return (Vec3::new(eye.x, 1.6, eye.y), tgt, r.street_fov.unwrap_or(74.0), format!("narrow street ({dfree:.1} m free): view along the street"));
    }
    let d = (dfree - 0.5).clamp(0.8, fit);
    let eye = fc + fnrm * d;
    (Vec3::new(eye.x, 1.6, eye.y), Vec3::new(fc.x, look, fc.y), r.street_fov.unwrap_or(90.0), format!("narrow street ({dfree:.1} m free)"))
}

/// Distance along `-fwd` from `target` at which all points fit the frame.
fn fit_distance(points: &[Vec3], target: Vec3, fwd: Vec3, fov_y: f32, aspect: f32) -> f32 {
    let f = fwd.normalize();
    let r = f.cross(Vec3::Y).normalize_or(Vec3::X);
    let u = r.cross(f);
    let ty = (fov_y.to_radians() * 0.5).tan() * 0.92;
    let tx = ty * aspect;
    let mut d: f32 = 1.0;
    for p in points {
        let rel = *p - target;
        let need = (rel.dot(r).abs() / tx).max(rel.dot(u).abs() / ty) - rel.dot(f);
        d = d.max(need);
    }
    d
}

/// Up to `max` vertices of the model, evenly subsampled.
fn sample_points(model: &Model, max: usize) -> Vec<Vec3> {
    let total: usize = model.geo.prims.values().map(|p| p.positions.len()).sum();
    let step = (total / max.max(1)).max(1);
    model.geo.prims.values().flat_map(|p| p.positions.iter().step_by(step).copied()).collect()
}

fn project(vp: &Mat4, p: Vec3, w: u32, h: u32) -> Option<Vec2> {
    let c = *vp * p.extend(1.0);
    if c.w <= 0.01 {
        return None;
    }
    Some(Vec2::new((c.x / c.w * 0.5 + 0.5) * w as f32, (1.0 - (c.y / c.w * 0.5 + 0.5)) * h as f32))
}

fn to_canvas(img: &RgbaImage) -> Canvas {
    let mut c = Canvas::new(img.width(), img.height(), [0.0, 0.0, 0.0, 1.0]);
    for (i, p) in img.pixels().enumerate() {
        c.px[i] = [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0, 1.0];
    }
    c
}

fn from_canvas(c: &Canvas) -> RgbaImage {
    let mut img = RgbaImage::new(c.w, c.h);
    for (i, p) in img.pixels_mut().enumerate() {
        let q = c.px[i];
        *p = Rgba([(q[0].clamp(0.0, 1.0) * 255.0) as u8, (q[1].clamp(0.0, 1.0) * 255.0) as u8, (q[2].clamp(0.0, 1.0) * 255.0) as u8, 255]);
    }
    img
}

/// Plan overlay: OSM outlines (red), parts (green), Catastro (blue), edge
/// numbers of outline 0, scale bar and north arrow.
fn draw_plan(img: &RgbaImage, vp: &Mat4, src: &Source) -> RgbaImage {
    let (w, h) = (img.width(), img.height());
    let mut c = to_canvas(img);
    let Some(fp) = &src.footprint else { return img.clone() };
    let ring = |c: &mut Canvas, r: &[Vec2], col: [f32; 4], width: f32| {
        for i in 0..r.len() {
            let (a, b) = (r[i], r[(i + 1) % r.len()]);
            if let (Some(p), Some(q)) = (project(vp, Vec3::new(a.x, 0.3, a.y), w, h), project(vp, Vec3::new(b.x, 0.3, b.y), w, h)) {
                c.line(p, q, width, col);
            }
        }
    };
    for p in &fp.catastro {
        for (r, _) in p.shape.rings() {
            ring(&mut c, r, [0.15, 0.35, 1.0, 0.9], 2.0);
        }
    }
    for p in &fp.parts {
        for (r, _) in p.shape.rings() {
            ring(&mut c, r, [0.1, 0.85, 0.2, 0.9], 2.0);
        }
    }
    for p in &fp.outlines {
        for (r, _) in p.shape.rings() {
            ring(&mut c, r, [1.0, 0.12, 0.1, 1.0], 3.0);
        }
    }
    if let Some(o) = fp.outlines.first() {
        let r = &o.shape.outer;
        for i in 0..r.len() {
            let (a, b) = (r[i], r[(i + 1) % r.len()]);
            if (b - a).length() < 2.0 {
                continue;
            }
            let mid = (a + b) * 0.5 + poly::edge_normal(r, i) * 1.5;
            if let Some(p) = project(vp, Vec3::new(mid.x, 0.3, mid.y), w, h) {
                c.circle(p, 9.0, [0.0, 0.0, 0.0, 0.65]);
                c.text_centered(&format!("{i}"), p, 10.0, 1.2, [1.0, 1.0, 1.0, 1.0]);
            }
        }
    }
    // Scale bar (10 m) and north arrow.
    if let (Some(a), Some(b)) = (project(vp, Vec3::ZERO, w, h), project(vp, Vec3::new(10.0, 0.0, 0.0), w, h)) {
        let len = (b - a).length();
        let o = Vec2::new(30.0, h as f32 - 30.0);
        c.rect(o - Vec2::new(6.0, 22.0), o + Vec2::new(len + 6.0, 8.0), [0.0, 0.0, 0.0, 0.55]);
        c.line(o, o + Vec2::new(len, 0.0), 4.0, [1.0, 1.0, 1.0, 1.0]);
        c.text("10 M", o + Vec2::new(0.0, -6.0), 11.0, 1.1, [1.0, 1.0, 1.0, 1.0]);
    }
    if let (Some(a), Some(b)) = (project(vp, Vec3::ZERO, w, h), project(vp, Vec3::new(0.0, 0.0, -10.0), w, h)) {
        let d = (b - a).normalize_or(Vec2::new(0.0, -1.0));
        let o = Vec2::new(w as f32 - 40.0, 60.0);
        c.line(o - d * 25.0, o + d * 25.0, 3.0, [1.0, 1.0, 1.0, 1.0]);
        c.line(o + d * 25.0, o + d * 12.0 + Vec2::new(-d.y, d.x) * 8.0, 3.0, [1.0, 1.0, 1.0, 1.0]);
        c.line(o + d * 25.0, o + d * 12.0 - Vec2::new(-d.y, d.x) * 8.0, 3.0, [1.0, 1.0, 1.0, 1.0]);
        c.text_centered("N", o + d * 40.0, 14.0, 1.3, [1.0, 1.0, 1.0, 1.0]);
    }
    c.text("OSM OUTLINE RED  PARTS GREEN  CATASTRO BLUE", Vec2::new(16.0, 24.0), 12.0, 1.1, [1.0, 1.0, 1.0, 1.0]);
    from_canvas(&c)
}

fn model_tris(model: &Model) -> Vec<([Vec3; 3], u32)> {
    let mut v = Vec::with_capacity(model.geo.tri_count());
    for (k, p) in &model.geo.prims {
        for t in p.indices.chunks_exact(3) {
            v.push(([p.positions[t[0] as usize], p.positions[t[1] as usize], p.positions[t[2] as usize]], *k));
        }
    }
    v
}

/// [photo | render | edges over the photo], all at the render's height.
fn compare(photo: &RgbaImage, render: &RgbaImage, edges: &[f32]) -> RgbaImage {
    let (w, h) = (render.width(), render.height());
    let ph = image::imageops::resize(photo, w, h, image::imageops::FilterType::Triangle);
    let gap = 6;
    let mut out = RgbaImage::from_pixel(w * 3 + gap * 2, h, Rgba([20, 20, 20, 255]));
    image::imageops::overlay(&mut out, &ph, 0, 0);
    image::imageops::overlay(&mut out, render, (w + gap) as i64, 0);
    let mut ov = ph.clone();
    for (i, p) in ov.pixels_mut().enumerate() {
        let l = (0.3 * p[0] as f32 + 0.59 * p[1] as f32 + 0.11 * p[2] as f32) * 0.55;
        let e = edges[i];
        let (r, g, b) = if e >= 0.85 { (255.0, 230.0, 40.0) } else if e > 0.0 { (60.0, 230.0, 255.0) } else { (l, l, l) };
        let k = if e > 0.0 { 0.9 } else { 1.0 };
        *p = Rgba([(l * (1.0 - k) + r * k).min(255.0) as u8, (l * (1.0 - k) + g * k).min(255.0) as u8, (l * (1.0 - k) + b * k).min(255.0) as u8, 255]);
    }
    image::imageops::overlay(&mut out, &ov, (2 * (w + gap)) as i64, 0);
    let mut c = to_canvas(&out);
    for (k, label) in ["PHOTO", "RENDER", "MODEL EDGES ON PHOTO"].iter().enumerate() {
        let x = k as f32 * (w + gap) as f32 + 10.0;
        c.rect(Vec2::new(x - 4.0, 6.0), Vec2::new(x + Canvas::text_width(label, 13.0) + 6.0, 26.0), [0.0, 0.0, 0.0, 0.6]);
        c.text(label, Vec2::new(x, 22.0), 13.0, 1.2, [1.0, 1.0, 1.0, 1.0]);
    }
    from_canvas(&c)
}

fn thicken(e: &[f32], w: usize, h: usize) -> Vec<f32> {
    let mut o = e.to_vec();
    for y in 0..h {
        for x in 0..w {
            let v = e[y * w + x];
            if v <= 0.0 {
                continue;
            }
            for (dx, dy) in [(1i32, 0i32), (0, 1), (-1, 0), (0, -1)] {
                let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
                    let k = ny as usize * w + nx as usize;
                    o[k] = o[k].max(v * 0.95);
                }
            }
        }
    }
    o
}

/// Main facade of outline 0 as a developed run (a curved street front is
/// one facade): the run holding the selected edges, the longest if several.
fn main_facade(src: &Source, model_min: Vec3, model_max: Vec3) -> landmark::runs::Bend {
    if let Some(o) = src.footprint.as_ref().and_then(|f| f.outlines.first()) {
        let shape = &o.shape;
        let sel = src.spec.landmark.main_facade.clone().unwrap_or(EdgeSel(vec![forja_city::landmark::spec::EdgeTok::Longest(1)]));
        let edges = landmark::resolve::select(shape, &sel);
        let runs = landmark::facade::runs_of(shape, 0.0, 1.0, 0, landmark::facade::RUN_ANGLE, false, &[]);
        let best = runs
            .iter()
            .filter_map(|r| {
                // Length of the selected part of this run.
                let l: f32 = r.edges.iter().filter(|(e, _, _)| edges.contains(e)).map(|(_, a, b)| b - a).sum();
                (l > 0.0).then_some((r, l))
            })
            .max_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((r, _)) = best {
            // The selected stretch of the run, in facade order.
            let sel_edges: Vec<&(landmark::resolve::EdgeRef, f32, f32)> = r.edges.iter().filter(|(e, _, _)| edges.contains(e)).collect();
            let (u0, u1) = (sel_edges.iter().map(|e| e.1).fold(f32::MAX, f32::min), sel_edges.iter().map(|e| e.2).fold(f32::MIN, f32::max));
            if let Some(b) = &r.bend {
                let n = ((u1 - u0) / 0.5).ceil().max(1.0) as usize;
                let pts: Vec<Vec2> = (0..=n).map(|k| b.frame(u0 + (u1 - u0) * k as f32 / n as f32).0).collect();
                if let Some(bend) = landmark::runs::Bend::new(&pts, false, 20.0) {
                    return bend;
                }
            }
        }
    }
    // No footprint: the south side of the bounds.
    let (a, b) = (Vec2::new(model_max.x, model_max.z), Vec2::new(model_min.x, model_max.z));
    landmark::runs::Bend::new(&[b, a], false, 20.0).unwrap_or_else(|| landmark::runs::Bend::new(&[Vec2::ZERO, Vec2::X], false, 20.0).unwrap())
}

fn save_png(img: &RgbaImage, path: &Path) -> Result<(), String> {
    img.save(path).map_err(|e| format!("{}: {e}", path.display()))
}

fn save_jpg(img: &RgbaImage, path: &Path) -> Result<(), String> {
    let rgb = image::DynamicImage::ImageRgba8(img.clone()).to_rgb8();
    let f = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(std::io::BufWriter::new(f), 88);
    enc.encode_image(&rgb).map_err(|e| format!("{}: {e}", path.display()))
}

fn contact_sheet(images: &[(String, RgbaImage)], cols: u32, tile_w: u32) -> RgbaImage {
    let tile_h = images.first().map(|(_, i)| (i.height() as f32 * tile_w as f32 / i.width() as f32) as u32).unwrap_or(tile_w * 9 / 16);
    let rows = (images.len() as u32).div_ceil(cols);
    let mut out = RgbaImage::from_pixel(cols * tile_w, rows * tile_h, Rgba([16, 16, 16, 255]));
    for (k, (name, img)) in images.iter().enumerate() {
        let t = image::imageops::resize(img, tile_w, tile_h, image::imageops::FilterType::Triangle);
        let (x, y) = ((k as u32 % cols) * tile_w, (k as u32 / cols) * tile_h);
        image::imageops::overlay(&mut out, &t, x as i64, y as i64);
        let mut c = to_canvas(&out);
        c.rect(Vec2::new(x as f32 + 4.0, y as f32 + 4.0), Vec2::new(x as f32 + Canvas::text_width(&name.to_uppercase(), 12.0) + 14.0, y as f32 + 22.0), [0.0, 0.0, 0.0, 0.6]);
        c.text(&name.to_uppercase(), Vec2::new(x as f32 + 9.0, y as f32 + 18.0), 12.0, 1.1, [1.0, 1.0, 1.0, 1.0]);
        out = from_canvas(&c);
    }
    out
}

/// A standard view name for `--view` (`ne`, `view_ne`, `street`, `plan`).
fn std_view_name(v: &str) -> Option<&'static str> {
    match v.trim().to_ascii_lowercase().as_str() {
        "ne" | "view_ne" => Some("view_ne"),
        "se" | "view_se" => Some("view_se"),
        "sw" | "view_sw" => Some("view_sw"),
        "nw" | "view_nw" => Some("view_nw"),
        "street" => Some("street"),
        "plan" => Some("plan"),
        _ => None,
    }
}

/// Index of the view chosen with `--view` (name or index).
fn pick_view(src: &Source, sel: &str) -> Result<usize, String> {
    if let Ok(k) = sel.parse::<usize>() {
        if k < src.spec.view.len() {
            return Ok(k);
        }
    }
    let names: Vec<String> = src.spec.view.iter().enumerate().map(|(k, v)| view_name(v, k)).collect();
    names.iter().position(|n| n == sel).ok_or_else(|| {
        let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        format!("no [[view]] `{sel}`{}; views: {} (or a standard view: ne, se, sw, nw, street, plan)", landmark::parse::did_you_mean(sel, &refs), names.join(", "))
    })
}

fn point_lights(src: &Source) -> Vec<PointLight> {
    src.spec
        .light
        .iter()
        .map(|l| {
            let c = l.color.map(|c| c.0).unwrap_or([1.0, 0.78, 0.5]);
            PointLight { position: Vec3::from_array(l.at), radius: l.radius.max(0.5), color: Vec3::from_array(c), intensity: l.intensity * 20.0 }
        })
        .collect()
}

fn render(args: &Args, root: &Path) -> Result<(), String> {
    let t0 = Instant::now();
    let dir = locate(args, root)?;
    let src = landmark::read(&dir).map_err(|e| e.to_string())?;
    let model = landmark::build_model(&src).map_err(|e| e.to_string())?;
    let slug = src.slug();
    let metrics = landmark::metrics::compute(&model, src.footprint.as_ref(), &slug, t0.elapsed().as_secs_f32() * 1000.0);
    let out_dir = args.out.clone().unwrap_or_else(|| root.join("screenshots/landmarks").join(&slug));
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    std::fs::write(out_dir.join("metrics.json"), serde_json::to_string_pretty(&metrics).unwrap()).map_err(|e| e.to_string())?;
    eprintln!(
        "{slug}: {} triangles, {} materials, height {:.1} m, IoU {} (built in {:.2}s)",
        metrics["triangles"],
        metrics["draw_calls"],
        metrics["max_height"].as_f64().unwrap_or(0.0),
        metrics["footprint"]["osm"]["iou"],
        t0.elapsed().as_secs_f32()
    );
    for w in metrics["warnings"].as_array().into_iter().flatten() {
        eprintln!("  warning: {}", w.as_str().unwrap_or(""));
    }
    // `--view` names a photo view, else a standard one (render only it).
    let std_only: Option<&'static str> = match args.view.as_deref() {
        Some(v) if pick_view(&src, v).is_err() => std_view_name(v),
        _ => None,
    };
    if std_only.is_some() && args.edges_only {
        return Err("--edges-only draws photo views; a standard view needs a GPU render (drop --edges-only)".into());
    }
    let only = if std_only.is_some() { None } else { args.view.as_deref().map(|v| pick_view(&src, v)).transpose()? };
    let views: Vec<usize> = match (only, std_only) {
        (_, Some(_)) => Vec::new(),
        (Some(k), _) => vec![k],
        (None, None) => (0..src.spec.view.len()).collect(),
    };
    let (hidden, hide_warn) = hidden_context(&src);
    for w in &hide_warn {
        eprintln!("  warning: {w}");
    }
    let w = args.width;
    let tris = model_tris(&model);
    // Cameras first: solved ones are printed ready to paste.
    let mut cams: Vec<(usize, ViewCam)> = Vec::new();
    if args.views != "std" || std_only.is_some() {
        for &k in &views {
            let v = &src.spec.view[k];
            match view_camera(&src, v, k) {
                Some(vc) => {
                    if let Some(rms) = vc.rms {
                        let c = vc.cam;
                        eprintln!(
                            "view {k} ({}): solved from {} matches, rms {rms:.1} px\n  eye = [{:.2}, {:.2}, {:.2}]\n  target = [{:.2}, {:.2}, {:.2}]\n  fov_y = {:.1}\n  roll = {:.1}",
                            view_name(v, k),
                            v.matches.len(),
                            c.eye.x,
                            c.eye.y,
                            c.eye.z,
                            c.target.x,
                            c.target.y,
                            c.target.z,
                            c.fov_y,
                            c.roll
                        );
                        if c.shift != Vec2::ZERO {
                            eprintln!("  shift = [{:.4}, {:.4}]", c.shift.x, c.shift.y);
                        }
                        if !vc.fixed.is_empty() {
                            eprintln!("  (held fixed: {})", vc.fixed.join(", "));
                        }
                        if let Some(sol) = &vc.solved {
                            let errs: Vec<String> = sol.errors.iter().enumerate().map(|(i, e)| format!("{i}: {e:.1}")).collect();
                            eprintln!("  per-point error (px) {}", errs.join(", "));
                            for w in &sol.warnings {
                                eprintln!("  warning: {w}");
                            }
                        }
                    }
                    for n in &vc.notes {
                        eprintln!("  warning: {n}");
                    }
                    if let Some((ci, h)) = camera_in_context(&src, &hidden, vc.cam.eye)
                        && v.context.unwrap_or(true)
                    {
                        eprintln!("  warning: {}", inside_context_warning(&format!("view {k} ({})", view_name(v, k)), ci, h));
                    }
                    cams.push((k, vc));
                }
                None => eprintln!("view {k} ({}): no eye and no camera in sources.toml; skipped", view_name(v, k)),
            }
        }
    }
    if args.edges_only {
        // No GPU: the model's edges over each photo.
        for (k, vc) in &cams {
            let v = &src.spec.view[*k];
            let aspect = vc.size.0 as f32 / vc.size.1.max(1) as f32;
            let rh = (w as f32 / aspect).round().clamp(128.0, 4096.0) as u32;
            let e = model_edges(&tris, vc.cam.view_proj(w as f32 / rh as f32), w, rh);
            let edges = edges_image(&e, w, rh);
            let photo = v.photo.as_ref().and_then(|p| image::open(src.photo_path(p)).ok()).map(|i| i.to_rgba8()).unwrap_or_else(|| RgbaImage::from_pixel(w, rh, Rgba([60, 60, 60, 255])));
            let cmp = compare(&photo, &edges, &e);
            save_jpg(&cmp, &out_dir.join(format!("compare_{k}.jpg")))?;
            eprintln!("view {k} ({}): compare_{k}.jpg (edges only) in {:.2}s", view_name(v, *k), t0.elapsed().as_secs_f32());
        }
        println!("{}", out_dir.display());
        return Ok(());
    }
    let dl = root.join("assets/downloaded");
    let kit;
    let lib;
    let look = if args.clay {
        Look::Clay
    } else {
        kit = Kit::load(Some(&dl));
        lib = Library::new(&kit);
        Look::Lib(&lib)
    };
    let scene = assemble_with(look, &model, &slug, false);
    eprintln!("materials and textures ready in {:.2}s", t0.elapsed().as_secs_f32());
    let sky = src.spec.render.sky.clone().unwrap_or_else(|| "clear".into());
    let mut st = Studio::new(&dl, &sky, point_lights(&src))?;
    let (mn, mx) = mesh::bounds(&model.geo).unwrap_or((Vec3::ZERO, Vec3::ONE));
    let center = (mn + mx) * 0.5;
    let radius = ((mx - mn) * 0.5).length().max(5.0);
    let mut handles: Vec<(ModelHandle, Mat4)> = vec![(st.r.upload_scene(&scene), Mat4::IDENTITY)];
    let pav = src.spec.render.paving.unwrap_or(radius * 1.6 + 25.0);
    handles.push((st.r.upload_scene(&ground_scene(look, Vec2::new(center.x, center.z), pav, &model.ground_cuts)), Mat4::IDENTITY));
    let context = if args.context { src.footprint.as_ref().and_then(|f| context_scene(look, &f.context, &hidden, Vec2::ZERO)) } else { None };
    let ctx_handle = context.as_ref().map(|c| st.r.upload_scene(c));
    eprintln!("uploaded in {:.2}s", t0.elapsed().as_secs_f32());
    let face = main_facade(&src, mn, mx);
    let (_, fnrm) = face.frame(face.len() * 0.5);
    let time = src.spec.render.time.clone().unwrap_or_else(|| "day".into());
    let (light0, sun_el0, exposure0) = match time_of_day(&time) {
        Some((el, ex)) => (Light::Atmosphere, Some(el), Some(src.spec.render.exposure.unwrap_or(0.55) * ex)),
        None => (if sky == "atmosphere" { Light::Atmosphere } else { Light::Hdri }, None, src.spec.render.exposure),
    };
    let default_sun = src.spec.render.sun.map(|s| (s[0], Some(s[1]))).unwrap_or((poly::azimuth(fnrm) - 35.0, sun_el0));
    let hh = (w as f32 * 9.0 / 16.0) as u32;
    let shot = |name: &str, eye: Vec3, target: Vec3, fov_y: f32, w: u32, h: u32, sun: (f32, Option<f32>), plan: bool| Shot { name: name.into(), eye, target, fov_y, roll: 0.0, w, h, sun, plan, light: light0, exposure: exposure0, shift: Vec2::ZERO };
    let mut sheet: Vec<(String, RgbaImage)> = Vec::new();
    let want = |name: &str| std_only.is_none_or(|o| o == name);
    if args.views != "photo" || std_only.is_some() {
        // Obliques frame the landmark alone (context hides it); the street,
        // plan and photo views show the neighbours.
        let fov = 36.0f32;
        let pts = sample_points(&model, 20_000);
        // Aim at the centre of the geometry's mass, not of its bounds.
        let mut target = pts.iter().copied().sum::<Vec3>() / pts.len().max(1) as f32;
        target.y = (mx.y * 0.35).max(1.0);
        for (name, az) in [("view_ne", 45.0f32), ("view_se", 135.0), ("view_sw", 225.0), ("view_nw", 315.0)] {
            if !want(name) {
                continue;
            }
            let d = poly::from_azimuth(az);
            let el = 26f32.to_radians();
            let dir = Vec3::new(d.x * el.cos(), el.sin(), d.y * el.cos());
            let dist = fit_distance(&pts, target, -dir, fov, w as f32 / hh as f32);
            let s = shot(name, target + dir * dist, target, fov, w, hh, default_sun, false);
            let (img, _) = render_shot(&mut st, &handles, &s);
            save_png(&img, &out_dir.join(format!("{name}.png")))?;
            sheet.push((name.into(), img));
            eprintln!("{name} in {:.2}s", t0.elapsed().as_secs_f32());
        }
        // Street-level view of the main facade.
        if want("street") {
            let (eye, tgt, fov, why) = street_shot(&src, &face, mx.y, w, hh, &hidden);
            eprintln!("street: {why}");
            if let Some((ci, h)) = camera_in_context(&src, &hidden, eye) {
                eprintln!("  warning: {}", inside_context_warning("street view", ci, h));
            }
            let s = shot("street", eye, tgt, fov, w, hh, default_sun, false);
            let mut hs = handles.clone();
            if let Some(c) = ctx_handle {
                hs.push((c, Mat4::IDENTITY));
            }
            let (img, _) = render_shot(&mut st, &hs, &s);
            save_png(&img, &out_dir.join("street.png"))?;
            sheet.push(("street".into(), img));
        }
        // Top-down plan, near orthographic.
        if !want("plan") {
            eprintln!("{} done in {:.2}s", std_only.unwrap_or(""), t0.elapsed().as_secs_f32());
            println!("{}", out_dir.display());
            return Ok(());
        }
        let mut pmin = Vec2::new(mn.x, mn.z);
        let mut pmax = Vec2::new(mx.x, mx.z);
        if let Some((a, b)) = src.footprint.as_ref().and_then(|f| f.bounds()) {
            pmin = pmin.min(a);
            pmax = pmax.max(b);
        }
        let pc = (pmin + pmax) * 0.5;
        let ext = (pmax - pmin).max(Vec2::splat(10.0)) * 1.15;
        let height = 3000.0;
        let half = ext.x.max(ext.y) * 0.5;
        let pfov = 2.0 * (half / height).atan().to_degrees();
        let mut s = shot("plan", Vec3::new(pc.x, height, pc.y + height * 1e-4), Vec3::new(pc.x, 0.0, pc.y), pfov, w, w, (default_sun.0, Some(55.0)), true);
        s.light = if light0 == Light::Atmosphere && st.env.is_some() { Light::Hdri } else { light0 };
        s.exposure = src.spec.render.exposure;
        let mut hs = handles.clone();
        if let Some(c) = ctx_handle {
            hs.push((c, Mat4::IDENTITY));
        }
        let (img, vp) = render_shot(&mut st, &hs, &s);
        let img = draw_plan(&img, &vp, &src);
        save_png(&img, &out_dir.join("plan.png"))?;
        let sq = image::imageops::resize(&img, w, hh, image::imageops::FilterType::Triangle);
        sheet.push(("plan".into(), sq));
        if std_only.is_some() {
            eprintln!("plan done in {:.2}s", t0.elapsed().as_secs_f32());
            println!("{}", out_dir.display());
            return Ok(());
        }
        let sheet_img = contact_sheet(&sheet, 3, 640);
        save_jpg(&sheet_img, &out_dir.join("sheet.jpg"))?;
        eprintln!("standard views done in {:.2}s", t0.elapsed().as_secs_f32());
    }
    if args.views != "std" {
        for (k, vc) in &cams {
            let v = &src.spec.view[*k];
            let name = view_name(v, *k);
            let photo = v.photo.as_ref().and_then(|p| image::open(src.photo_path(p)).ok()).map(|i| i.to_rgba8());
            let aspect = vc.size.0 as f32 / vc.size.1.max(1) as f32;
            let rh = (w as f32 / aspect).round().clamp(128.0, 4096.0) as u32;
            let vt = v.time.as_deref().and_then(time_of_day);
            let sun = match (v.sun, vt) {
                (Some(s), _) => (s[0], Some(s[1])),
                (None, Some((el, _))) => (default_sun.0, Some(el)),
                _ => default_sun,
            };
            let mut s = Shot { name: name.clone(), eye: vc.cam.eye, target: vc.cam.target, fov_y: vc.cam.fov_y, roll: vc.cam.roll, w, h: rh, sun, plan: false, light: light0, exposure: exposure0, shift: vc.cam.shift };
            // The sky follows a low sun (dusk photos) or an explicit time.
            if vt.is_some() || sun.1.is_some_and(|e| e < 12.0) {
                s.light = Light::Atmosphere;
                if let Some((_, ex)) = vt.or(time_of_day("evening")) {
                    s.exposure = Some(src.spec.render.exposure.unwrap_or(0.55) * ex);
                }
            }
            let mut hs = handles.clone();
            if let Some(c) = ctx_handle
                && v.context.unwrap_or(true)
            {
                hs.push((c, Mat4::IDENTITY));
            }
            let (img, vp) = render_shot(&mut st, &hs, &s);
            save_png(&img, &out_dir.join(format!("photo_{k}.png")))?;
            if let Some(p) = &photo {
                let e = model_edges(&tris, vp, w, rh);
                let cmp = compare(p, &img, &e);
                save_jpg(&cmp, &out_dir.join(format!("compare_{k}.jpg")))?;
                eprintln!("view {k} ({name}): compare_{k}.jpg");
            } else {
                eprintln!("view {k} ({name}): photo {:?} not found; rendered photo_{k}.png only", v.photo);
            }
        }
    }
    eprintln!("{} written in {:.1}s", out_dir.display(), t0.elapsed().as_secs_f32());
    println!("{}", out_dir.display());
    Ok(())
}

// ---------------------------------------------------------------- gallery

fn gallery(args: &Args, root: &Path) -> Result<(), String> {
    let t0 = Instant::now();
    let base = root.join("data/landmarks");
    let mut dirs: Vec<PathBuf> = Vec::new();
    if args.targets.is_empty() {
        dirs = landmark::find_all(&base).into_iter().filter(|d| args.samples || !d.components().any(|c| c.as_os_str() == "_samples")).collect();
        if dirs.is_empty() {
            dirs = landmark::find_all(&base.join("_samples"));
        }
    } else {
        for t in &args.targets {
            let p = landmark::locate(t, root).unwrap_or_else(|| PathBuf::from(t));
            if !p.is_dir() {
                return Err(format!("gallery: `{t}` is not a folder or landmark slug"));
            }
            let found = landmark::find_all(&p);
            if found.is_empty() {
                return Err(format!("gallery: no landmark.toml under {}", p.display()));
            }
            dirs.extend(found);
        }
    }
    if dirs.is_empty() {
        return Err("no landmark.toml found under data/landmarks".into());
    }
    let dl = root.join("assets/downloaded");
    let kit = Kit::load(Some(&dl));
    let lib = Library::new(&kit);
    let mut st = Studio::new(&dl, "clear", Vec::new())?;
    let mut handles = Vec::new();
    let mut lo = Vec2::splat(f32::MAX);
    let mut hi = Vec2::splat(f32::MIN);
    let mut top = 10.0f32;
    let mut n = 0;
    let mut each: Vec<(String, Vec3, f32)> = Vec::new();
    let mut cuts: Vec<poly::Shape> = Vec::new();
    for d in &dirs {
        match landmark::load_with(d, &lib, AssembleOptions { lods: true }) {
            Ok(l) => {
                let h = st.r.upload_scene(&l.scene);
                let m = l.transform();
                let (a, b) = l.bounds;
                lo = lo.min(Vec2::new(a.x, a.z) + l.anchor);
                hi = hi.max(Vec2::new(b.x, b.z) + l.anchor);
                top = top.max(b.y);
                let c3 = (a + b) * 0.5 + Vec3::new(l.anchor.x, 0.0, l.anchor.y);
                each.push((l.slug.clone(), c3, ((b - a) * 0.5).length()));
                cuts.extend(l.ground_cuts.iter().map(|g| g.map(|p| p + l.anchor)));
                handles.push((h, m));
                n += 1;
                eprintln!("{}: {} triangles at [{:.1}, {:.1}]", l.slug, l.scene.triangle_count(), l.anchor.x, l.anchor.y);
            }
            Err(e) => eprintln!("{}: {e}", d.display()),
        }
    }
    if n == 0 {
        return Err("no landmark could be built".into());
    }
    let c = (lo + hi) * 0.5;
    let ext = (hi - lo).max(Vec2::splat(60.0));
    handles.push((st.r.upload_scene(&ground_scene(Look::Lib(&lib), c, ext.max_element() * 0.8 + 60.0, &cuts)), Mat4::IDENTITY));
    let out_dir = args.out.clone().unwrap_or_else(|| root.join("screenshots/landmarks/_gallery"));
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    let w = args.width;
    let h = w * 9 / 16;
    let r = ext.length() * 0.5 + top;
    let ctr = Vec3::new(c.x, 0.0, c.y);
    let shots = [
        ("aerial_s", ctr + Vec3::new(0.0, r * 0.9, r * 1.5), 45.0),
        ("aerial_se", ctr + Vec3::new(r * 1.1, r * 0.7, r * 1.1), 45.0),
        ("aerial_nw", ctr + Vec3::new(-r * 1.1, r * 0.8, -r * 1.1), 45.0),
        ("top", ctr + Vec3::new(0.0, r * 2.6, r * 0.01), 45.0),
    ];
    let mk = |name: &str, eye: Vec3, target: Vec3, fov_y: f32| Shot { name: name.into(), eye, target, fov_y, roll: 0.0, w, h, sun: (200.0, Some(50.0)), plan: false, light: Light::Hdri, exposure: None, shift: Vec2::ZERO };
    let mut sheet = Vec::new();
    for (name, eye, fov) in shots {
        let (img, _) = render_shot(&mut st, &handles, &mk(name, eye, ctr, fov));
        save_png(&img, &out_dir.join(format!("{name}.png")))?;
        sheet.push((name.to_string(), img));
    }
    // A closer aerial of each landmark in its place.
    for (slug, c3, rad) in each.iter().take(16) {
        let dir = Vec3::new(0.55, 0.6, 0.58).normalize();
        let (img, _) = render_shot(&mut st, &handles, &mk(slug, *c3 + dir * (rad * 2.6 + 20.0), *c3 - Vec3::Y * (c3.y * 0.5), 40.0));
        save_png(&img, &out_dir.join(format!("aerial_{slug}.png")))?;
        sheet.push((slug.clone(), img));
    }
    save_jpg(&contact_sheet(&sheet, 3, 640), &out_dir.join("sheet.jpg"))?;
    eprintln!("gallery of {n} landmarks in {:.1}s", t0.elapsed().as_secs_f32());
    println!("{}", out_dir.display());
    Ok(())
}
