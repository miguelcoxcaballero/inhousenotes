//! Scanned PBR material sets and HDR environment maps.

use std::path::{Path, PathBuf};

use forja_math::Vec4;

use crate::{AssetError, MaterialData, SceneData, TextureData, load_texture};

/// Color, normal and packed occlusion/roughness/metal (glTF ORM) textures.
#[derive(Clone, Debug)]
pub struct PbrSet {
    pub name: String,
    pub color: TextureData,
    pub normal: Option<TextureData>,
    pub orm: Option<TextureData>,
    pub emission: Option<TextureData>,
}

fn find(dir: &Path, keys: &[&str]) -> Option<PathBuf> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).collect();
    entries.sort();
    entries.into_iter().find(|p| {
        let n = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        (n.ends_with(".jpg") || n.ends_with(".png")) && keys.iter().any(|k| n.contains(&k.to_lowercase()))
    })
}

fn gray(path: &Path) -> Result<Vec<u8>, AssetError> {
    Ok(image::open(path).map_err(|e| AssetError::Io(format!("{}: {e}", path.display())))?.to_luma8().into_raw())
}

/// Loads a material folder written by forja_fetch (Poly Haven `*_color /
/// *_normal / *_arm` or ambientCG `*_Color / *_NormalGL / *_Roughness /
/// *_AmbientOcclusion / *_Metalness / *_Emission`).
pub fn load_pbr_set(dir: &Path) -> Result<PbrSet, AssetError> {
    let color_path = find(dir, &["_color", "_diff"]).ok_or_else(|| AssetError::Io(format!("{}: no color map", dir.display())))?;
    let color = load_texture(&color_path, true)?;
    let normal = find(dir, &["_normalgl", "_normal", "_nor_gl"]).map(|p| load_texture(&p, false)).transpose()?;
    let orm = if let Some(p) = find(dir, &["_arm"]) {
        Some(load_texture(&p, false)?)
    } else if let Some(rough) = find(dir, &["_roughness"]) {
        let r = gray(&rough)?;
        let ao = find(dir, &["_ambientocclusion"]).map(|p| gray(&p)).transpose()?;
        let metal = find(dir, &["_metalness"]).map(|p| gray(&p)).transpose()?;
        let n = color.width as usize * color.height as usize;
        if r.len() != n {
            return Err(AssetError::Format(format!("{}: map sizes differ", dir.display())));
        }
        let mut rgba = Vec::with_capacity(n * 4);
        for i in 0..n {
            rgba.extend([ao.as_ref().map_or(255, |a| a[i]), r[i], metal.as_ref().map_or(0, |m| m[i]), 255]);
        }
        Some(TextureData { name: "orm".into(), width: color.width, height: color.height, rgba8: rgba, srgb: false })
    } else {
        None
    };
    let emission = find(dir, &["_emission"]).map(|p| load_texture(&p, true)).transpose()?;
    Ok(PbrSet { name: dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), color, normal, orm, emission })
}

impl PbrSet {
    /// Adds the textures and a material to `scene`; returns the material index.
    pub fn add_to(&self, scene: &mut SceneData, tint: Vec4, emissive: bool) -> usize {
        let mut push = |t: &TextureData| {
            scene.textures.push(t.clone());
            scene.textures.len() - 1
        };
        let base = push(&self.color);
        let normal = self.normal.as_ref().map(&mut push);
        let orm = self.orm.as_ref().map(&mut push);
        let emission = if emissive { self.emission.as_ref().map(&mut push) } else { None };
        scene.materials.push(MaterialData {
            name: self.name.clone(),
            base_color: tint,
            base_color_tex: Some(base),
            metallic: if orm.is_some() { 1.0 } else { 0.0 },
            roughness: 1.0,
            metallic_roughness_tex: orm,
            normal_tex: normal,
            occlusion_tex: orm,
            emissive: if emission.is_some() { forja_math::Vec3::splat(1.0) } else { forja_math::Vec3::ZERO },
            emissive_tex: emission,
            ..Default::default()
        });
        scene.materials.len() - 1
    }
}

/// Linear HDR image (equirectangular environment).
#[derive(Clone, Debug)]
pub struct HdrImage {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<[f32; 3]>,
}

pub fn load_hdr(path: &Path) -> Result<HdrImage, AssetError> {
    let img = image::open(path).map_err(|e| AssetError::Io(format!("{}: {e}", path.display())))?.to_rgb32f();
    let (width, height) = img.dimensions();
    let rgb = img.into_raw().chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
    Ok(HdrImage { width, height, rgb })
}
