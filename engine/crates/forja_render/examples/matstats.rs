//! Material audit of the city kit: effective roughness / metallic / albedo
//! and normal-map strength of every facade and street material, as the
//! shader sees them (factor x texture). `cargo run --release -p forja_render
//! --example matstats`.

use std::path::PathBuf;

use forja_city::kit::{Assembler, Kit, Mat};

fn srgb_to_lin(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn pct(v: &mut [f32], p: f32) -> f32 {
    v.sort_by(|a, b| a.total_cmp(b));
    v[((v.len() - 1) as f32 * p) as usize]
}

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let kit = Kit::load(Some(&root.join("assets/downloaded")));
    let mut asm = Assembler::new(&kit, "audit");
    let c = [230, 222, 210];
    let mats = [
        ("stucco", Mat::Stucco(c)),
        ("plaster", Mat::Plaster(c)),
        ("brick", Mat::Brick([255; 3])),
        ("ashlar", Mat::Ashlar([255; 3])),
        ("moulding", Mat::Moulding(c)),
        ("granite", Mat::Granite),
        ("roof_clay", Mat::RoofClay),
        ("painted_wood", Mat::PaintedWood([90, 110, 80])),
        ("wood", Mat::Wood([160, 120, 90])),
        ("shutter", Mat::MetalShutter(0)),
        ("iron", Mat::Iron),
        ("zinc", Mat::Zinc),
        ("glass", Mat::Glass),
        ("asphalt", Mat::Asphalt),
        ("sidewalk_hex", Mat::SidewalkHex),
        ("curb", Mat::Curb),
        ("terracotta", Mat::Terracotta),
    ];
    println!("{:<14} {:>6} {:>6} {:>5} | rough p10 p50 p90 | metal p50 p90 | albedo p50 | normal xy p50 p90 (scale)", "material", "rf", "mf", "spec");
    for (name, m) in mats {
        let i = asm.material(m);
        let d = asm.scene.materials[i].clone();
        let tex = |k: Option<usize>| k.map(|k| asm.scene.textures[k].clone());
        let (mut r, mut mt) = (Vec::new(), Vec::new());
        if let Some(t) = tex(d.metallic_roughness_tex) {
            for p in t.rgba8.chunks_exact(4).step_by(7) {
                r.push(d.roughness * p[1] as f32 / 255.0);
                mt.push(d.metallic * p[2] as f32 / 255.0);
            }
        } else {
            r.push(d.roughness);
            mt.push(d.metallic);
        }
        let mut alb = Vec::new();
        if let Some(t) = tex(d.base_color_tex) {
            for p in t.rgba8.chunks_exact(4).step_by(7) {
                let l = 0.2126 * srgb_to_lin(p[0]) + 0.7152 * srgb_to_lin(p[1]) + 0.0722 * srgb_to_lin(p[2]);
                alb.push(l * (0.2126 * d.base_color.x + 0.7152 * d.base_color.y + 0.0722 * d.base_color.z));
            }
        } else {
            alb.push(0.2126 * d.base_color.x + 0.7152 * d.base_color.y + 0.0722 * d.base_color.z);
        }
        let mut nxy = vec![0.0];
        if let Some(t) = tex(d.normal_tex) {
            nxy = t.rgba8.chunks_exact(4).step_by(7).map(|p| {
                let (x, y) = (p[0] as f32 / 127.5 - 1.0, p[1] as f32 / 127.5 - 1.0);
                (x * x + y * y).sqrt() * d.normal_scale
            }).collect();
        }
        println!(
            "{name:<14} {:>6.2} {:>6.2} {:>5.2} | {:.2} {:.2} {:.2} | {:.2} {:.2} | {:.3} | {:.3} {:.3} ({:.2})",
            d.roughness,
            d.metallic,
            d.specular,
            pct(&mut r, 0.1),
            pct(&mut r, 0.5),
            pct(&mut r, 0.9),
            pct(&mut mt, 0.5),
            pct(&mut mt, 0.9),
            pct(&mut alb, 0.5),
            pct(&mut nxy, 0.5),
            pct(&mut nxy, 0.9),
            d.normal_scale
        );
    }
}
