//! Character self-check: renders every mocap clip on a MakeHuman character
//! at several phases (side and front) and prints objective metrics:
//! ground contact, foot sliding, loop seam and joint bend direction.
//! `cargo run --release -p forja_render --example charcheck -- out.png [preset]`

use std::path::PathBuf;

use forja_anim::{Clip, Skeleton, mocap};
use forja_assets::makehuman::{HumanSpec, build_human};
use forja_math::{Mat4, Vec3};
use forja_render::*;

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "charcheck.png".into());
    let preset = std::env::args().nth(2).unwrap_or_else(|| "casual_male".into());
    let only: Option<String> = std::env::args().nth(3);
    let dl = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/downloaded");
    let spec = HumanSpec::presets().into_iter().find(|s| s.name == preset).expect("preset");
    let scene = build_human(&dl.join("makehuman_base"), &dl.join("makehuman_assets"), &spec).unwrap();
    let skel = Skeleton::from_scene(&scene, 0).unwrap();
    let t0 = std::time::Instant::now();
    let clips: Vec<Clip> = if only.as_deref() == Some("portrait") { Vec::new() } else { mocap::load(&dl.join("cmu_mocap"), &mocap::cmu_set(), &skel, Vec3::Z, 30.0)
        .into_iter()
        .filter(|c| only.as_deref().is_none_or(|o| c.name == o))
        .collect() };
    println!("{} clips in {:.2}s", clips.len(), t0.elapsed().as_secs_f32());

    // Metrics on CPU-skinned feet and joints.
    let find = |n: &str| skel.find(n);
    let (lf, rf) = (find("foot.L").or(find("toe1-1.L")), find("foot.R"));
    let (lk, rk) = (find("lowerleg01.L"), find("lowerleg01.R"));
    let (lh, rh) = (find("upperleg01.L"), find("upperleg01.R"));
    let rest_min = {
        let mut m = Vec::new();
        skel.model_matrices(&skel.rest_pose(), &mut m);
        [lf, rf].iter().flatten().map(|j| m[*j].w_axis.y).fold(f32::MAX, f32::min)
    };
    for c in &clips {
        let n = 24;
        let mut lowest = f32::MAX;
        let mut highest_low = f32::MIN;
        let mut slide = 0.0f32;
        let mut prev: Option<(Vec3, Vec3)> = None;
        let mut knee_bad = 0;
        for k in 0..n {
            let t = c.duration * k as f32 / n as f32;
            let mut pose = skel.rest_pose();
            c.sample(t, true, &mut pose);
            let mut m = Vec::new();
            skel.model_matrices(&pose, &mut m);
            let p = |j: Option<usize>| j.map(|j| m[j].w_axis.truncate()).unwrap_or(Vec3::ZERO);
            let (a, b) = (p(lf), p(rf));
            let low = a.y.min(b.y);
            lowest = lowest.min(low);
            highest_low = highest_low.max(low);
            // Sliding: horizontal motion of the planted (lower) foot.
            if let Some((pa, pb)) = prev {
                let planted = if a.y < b.y { (a - pa) } else { (b - pb) };
                slide += Vec3::new(planted.x, 0.0, planted.z).length();
            }
            prev = Some((a, b));
            // Knees must bend forward: knee ahead of the hip-ankle line.
            for (h, kj, f) in [(lh, lk, lf), (rh, rk, rf)] {
                let (hp, kp, fp) = (p(h), p(kj), p(f));
                let line = (fp - hp).normalize_or_zero();
                let off = (kp - hp) - line * (kp - hp).dot(line);
                if off.length() > 0.03 && off.dot(Vec3::Z) < -0.01 {
                    knee_bad += 1;
                }
            }
        }
        // Loop seam: pose difference between end and start.
        let (mut p0, mut p1) = (skel.rest_pose(), skel.rest_pose());
        c.sample(0.0, false, &mut p0);
        c.sample(c.duration - 1e-4, false, &mut p1);
        let seam: f32 = p0.local.iter().zip(&p1.local).map(|(a, b)| 1.0 - a.r.dot(b.r).abs()).sum();
        println!(
            "{:8} dur {:5.2}s | feet lowest {:+.3} m (rest {:+.3}), planted max {:+.3} | slide {:.2} m/cycle | seam {:.4} | knees backwards {}",
            c.name, c.duration, lowest, rest_min, highest_low, slide, seam, knee_bad
        );
    }

    // Portrait mode: face and hands close up in the idle pose.
    if only.as_deref() == Some("portrait") {
        let mut r = Renderer::new(&create_instance(), None).unwrap();
        if let Ok(img) = forja_assets::load_hdr(&dl.join("hdri_wide_street/wide_street_01_2k.hdr")) {
            r.set_environment(Some(&env::EnvMap::from_hdr(&img, 14.0)));
        }
        let h = r.upload_scene(&scene);
        let mut m = Vec::new();
        skel.model_matrices(&skel.rest_pose(), &mut m);
        let head = m[skel.find("head").unwrap()].w_axis.truncate();
        let hand = m[skel.find("wrist.R").unwrap()].w_axis.truncate();
        let mut sheet = image::RgbaImage::new(1500, 500);
        for (k, (at, eye)) in [(head + Vec3::Y * 0.08, head + Vec3::new(0.0, 0.1, 0.75)), (head + Vec3::Y * 0.08, head + Vec3::new(0.55, 0.12, 0.5)), (hand, hand + Vec3::new(-0.15, 0.1, 0.55))].iter().enumerate() {
            let mut frame = FrameDesc::default();
            frame.instances.push(DrawInstance::new(h, Mat4::IDENTITY));
            frame.camera = Camera::look_at(*eye, *at);
            frame.camera.fov_y = 30f32.to_radians();
            frame.camera.near = 0.02;
            frame.env.shadow_distance = 4.0;
            let img = r.render_to_image(&frame, 500, 500);
            image::imageops::overlay(&mut sheet, &img, k as i64 * 500, 0);
        }
        sheet.save(&out).unwrap();
        println!("{out}");
        return;
    }

    // Contact sheet.
    let mut r = Renderer::new(&create_instance(), None).unwrap();
    if let Ok(img) = forja_assets::load_hdr(&dl.join("hdri_wide_street/wide_street_01_2k.hdr")) {
        r.set_environment(Some(&env::EnvMap::from_hdr(&img, 14.0)));
    }
    let h = r.upload_scene(&scene);
    let ground = {
        let mut g = forja_assets::SceneData::default();
        let mut m = forja_assets::MeshData {
            positions: vec![Vec3::new(-30.0, 0.0, -30.0), Vec3::new(30.0, 0.0, -30.0), Vec3::new(30.0, 0.0, 30.0), Vec3::new(-30.0, 0.0, 30.0)],
            indices: vec![0, 2, 1, 0, 3, 2],
            ..Default::default()
        };
        m.finalize();
        g.meshes.push(vec![m]);
        g.materials.push(forja_assets::MaterialData { base_color: forja_math::Vec4::new(0.3, 0.3, 0.28, 1.0), roughness: 0.9, ..Default::default() });
        g.nodes.push(forja_assets::NodeData { mesh: Some(0), ..Default::default() });
        g.roots.push(0);
        r.upload_scene(&g)
    };
    let b = scene.bounds();
    let foot_y = b.min.y;
    let cell = 300u32;
    let cols = 5usize;
    let rows = clips.len() * 2;
    let mut sheet = image::RgbaImage::new(cell * cols as u32, cell * rows as u32);
    for (ci, c) in clips.iter().enumerate() {
        for view in 0..2 {
            for k in 0..cols {
                let t = c.duration * k as f32 / cols as f32;
                let mut pose = skel.rest_pose();
                c.sample(t, true, &mut pose);
                let mut mats = Vec::new();
                skel.skinning_matrices(&pose, &mut mats);
                let mut frame = FrameDesc::default();
                frame.instances.push(DrawInstance::new(ground, Mat4::from_translation(Vec3::new(0.0, foot_y, 0.0))));
                let mut inst = DrawInstance::new(h, Mat4::IDENTITY);
                inst.joints = Some(std::sync::Arc::new(mats));
                frame.instances.push(inst);
                let ctr = Vec3::new(0.0, b.center().y, 0.0);
                let eye = if view == 0 { ctr + Vec3::new(4.2, 0.2, 0.0) } else { ctr + Vec3::new(0.6, 0.3, 4.2) };
                frame.camera = Camera::look_at(eye, ctr);
                frame.camera.fov_y = 32f32.to_radians();
                frame.env.shadow_distance = 12.0;
                let img = r.render_to_image(&frame, cell, cell);
                image::imageops::overlay(&mut sheet, &img, (k as u32 * cell) as i64, ((ci * 2 + view) as u32 * cell) as i64);
            }
        }
    }
    sheet.save(&out).unwrap();
    println!("{out}");
}
