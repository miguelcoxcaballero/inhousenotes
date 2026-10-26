//! Cutout captures: a model's unlit albedo and coverage seen from a far,
//! near-orthographic camera, rendered by the engine itself (impostors,
//! icons). The tonemapper is undone with a measured grey ramp, each colour
//! channel is rendered as grey (so the inversion is exact), and coverage
//! comes from matting against a black and a white backdrop. TAA gives the
//! edges real partial coverage.

use std::sync::Arc;

use forja_assets::{MaterialData, MeshData, NodeData, SceneData, TextureData};
use forja_math::{Mat4, Quat, Vec2, Vec3, Vec4};

use crate::{Camera, DrawInstance, FrameDesc, ModelHandle, Renderer};

/// One view: the camera sits along `dir` from `center` and sees a square
/// of half size `half` metres.
#[derive(Clone, Copy, Debug)]
pub struct CutoutView {
    pub center: Vec3,
    pub dir: Vec3,
    pub half: f32,
}

/// Straight-alpha linear RGBA pixels of a `size`² capture, row-major from the top.
pub type Cutout = Vec<[f32; 4]>;

fn quad_scene(name: &str, color: Vec4, quads: &[(Vec2, f32, f32)]) -> SceneData {
    // Quads in the XY plane facing +Z: (centre, half size, grey).
    let mut scene = SceneData { name: name.into(), ..Default::default() };
    let mut prims = Vec::new();
    for (i, (c, h, g)) in quads.iter().enumerate() {
        scene.materials.push(MaterialData { name: format!("{name}{i}"), base_color: if quads.len() > 1 { Vec4::new(*g, *g, *g, 1.0) } else { color }, unlit: true, ..Default::default() });
        let p = |x: f32, y: f32| Vec3::new(c.x + x * h, c.y + y * h, 0.0);
        let mut m = MeshData { positions: vec![p(-1.0, -1.0), p(1.0, -1.0), p(1.0, 1.0), p(-1.0, 1.0)], normals: vec![Vec3::Z; 4], indices: vec![0, 1, 2, 0, 2, 3], material: i, ..Default::default() };
        m.finalize();
        prims.push(m);
    }
    scene.meshes = vec![prims];
    scene.nodes = vec![NodeData { name: name.into(), mesh: Some(0), ..Default::default() }];
    scene.roots = vec![0];
    scene
}

/// The scene with every base colour reduced to channel `c` as grey, unlit.
fn channel_scene(scene: &SceneData, c: usize) -> SceneData {
    let mut s = scene.clone();
    let used: Vec<bool> = (0..s.textures.len()).map(|i| s.materials.iter().any(|m| m.base_color_tex == Some(i))).collect();
    for (i, t) in s.textures.iter_mut().enumerate() {
        if !used[i] {
            continue;
        }
        let data: Vec<u8> = t.rgba8.chunks_exact(4).flat_map(|p| [p[c], p[c], p[c], p[3]]).collect();
        *t = TextureData { name: format!("{}_ch{c}", t.name), width: t.width, height: t.height, rgba8: Arc::from(data), srgb: t.srgb };
    }
    for m in &mut s.materials {
        let g = m.base_color[c];
        m.base_color = Vec4::new(g, g, g, m.base_color.w);
        m.unlit = true;
        m.foliage = None;
        m.emissive = Vec3::ZERO;
        m.transmission = 0.0;
    }
    s
}

fn face(dir: Vec3) -> Quat {
    Quat::from_rotation_arc(Vec3::Z, dir.normalize())
}

/// Renders `views` of `scene` at `size`² pixels.
pub fn capture_cutouts(r: &mut Renderer, scene: &SceneData, views: &[CutoutView], size: u32) -> Vec<Cutout> {
    let saved = r.settings.clone();
    r.settings.ssr = false;
    r.settings.ssao = false;
    r.settings.contact_shadows = false;
    r.settings.sharpen = 0.0;
    r.settings.taa = true;
    r.settings.capture_frames = r.settings.capture_frames.clamp(2, 4);
    let mut frame = FrameDesc::default();
    frame.env.fog_density = 0.0;
    frame.env.bloom = 0.0;
    frame.env.vignette = 0.0;
    frame.env.exposure = 0.35;
    let dist = 400.0;
    let look = |frame: &mut FrameDesc, center: Vec3, dir: Vec3, half: f32| {
        frame.camera = Camera::look_at(center + dir * dist, center);
        frame.camera.fov_y = 2.0 * (half / dist).atan();
        frame.camera.near = dist - half * 4.0;
    };
    // Grey ramp: display value -> albedo.
    let steps = 32usize;
    let ramp: Vec<(Vec2, f32, f32)> = (0..steps).map(|i| (Vec2::new(-1.0 + (i as f32 + 0.5) * 2.0 / steps as f32, 0.0), 1.0 / steps as f32 * 0.9, i as f32 / (steps - 1) as f32)).collect();
    let ramp_h = r.upload_scene(&quad_scene("cutout_ramp", Vec4::ONE, &ramp));
    let mut lut = [0.0f32; 256];
    for _ in 0..4 {
        look(&mut frame, Vec3::ZERO, Vec3::Z, 1.0);
        frame.instances = vec![DrawInstance::new(ramp_h, Mat4::IDENTITY)];
        let img = r.render_to_image(&frame, 256, 256);
        let vals: Vec<f32> = (0..steps).map(|i| img.get_pixel(((i as f32 + 0.5) * 256.0 / steps as f32) as u32, 128)[0] as f32).collect();
        if vals[steps - 1] > 245.0 {
            frame.env.exposure *= 0.7;
            continue;
        }
        // Monotone inverse by linear interpolation.
        for (d, out) in lut.iter_mut().enumerate() {
            let d = d as f32;
            let mut g = 1.0;
            for i in 0..steps - 1 {
                let (a, b) = (vals[i], vals[i + 1].max(vals[i] + 1e-3));
                if d <= a {
                    g = i as f32 / (steps - 1) as f32;
                    break;
                }
                if d <= b {
                    g = (i as f32 + (d - a) / (b - a)) / (steps - 1) as f32;
                    break;
                }
            }
            *out = g;
        }
        break;
    }
    let chans: Vec<ModelHandle> = (0..3).map(|c| r.upload_scene(&channel_scene(scene, c))).collect();
    let black = r.upload_scene(&quad_scene("cutout_black", Vec4::new(0.0, 0.0, 0.0, 1.0), &[(Vec2::ZERO, 1.0, 0.0)]));
    let white = r.upload_scene(&quad_scene("cutout_white", Vec4::ONE, &[(Vec2::ZERO, 1.0, 1.0)]));
    let mut out = Vec::new();
    for v in views {
        let bg = |h: ModelHandle| DrawInstance { cast_shadows: false, ..DrawInstance::new(h, Mat4::from_scale_rotation_translation(Vec3::splat(v.half * 3.0), face(v.dir), v.center - v.dir * v.half * 2.5)) };
        let mut shot = |model: ModelHandle, back: ModelHandle| {
            look(&mut frame, v.center, v.dir, v.half);
            frame.instances = vec![DrawInstance::new(model, Mat4::IDENTITY), bg(back)];
            r.render_to_image(&frame, size, size)
        };
        let rb = shot(chans[0], black);
        let rw = shot(chans[0], white);
        let gb = shot(chans[1], black);
        let bb = shot(chans[2], black);
        let px: Cutout = (0..(size * size) as usize)
            .map(|i| {
                let (x, y) = ((i as u32) % size, (i as u32) / size);
                let k = |img: &image::RgbaImage| lut[img.get_pixel(x, y)[0] as usize];
                let (b, w) = (k(&rb), k(&rw));
                let a = (1.0 - (w - b)).clamp(0.0, 1.0);
                if a < 0.02 {
                    return [0.0; 4];
                }
                [(b / a).min(1.0), (k(&gb) / a).min(1.0), (k(&bb) / a).min(1.0), a]
            })
            .collect();
        out.push(px);
    }
    r.settings = saved;
    out
}
