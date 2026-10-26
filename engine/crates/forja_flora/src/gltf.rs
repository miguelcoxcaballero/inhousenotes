//! Binary glTF (`.glb`) export of a [`TreeAsset`].
//!
//! LOD 0 is the scene's visible node; LOD 1 and the billboard LOD 2 are
//! attached with `MSFT_lod` (viewers without it just show LOD 0). The wind
//! weights travel as the custom `_WIND` vertex attribute. Textures are
//! embedded as PNG; normal maps are converted to the glTF (+Y up)
//! convention and tangents are left to the importer (MikkTSpace).

use std::path::Path;

use serde_json::{Value, json};

use crate::TreeAsset;

struct Builder {
    bin: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
}

impl Builder {
    fn align(&mut self) {
        while self.bin.len() % 4 != 0 {
            self.bin.push(0);
        }
    }

    fn view(&mut self, bytes: &[u8], target: Option<u32>) -> usize {
        self.align();
        let off = self.bin.len();
        self.bin.extend_from_slice(bytes);
        let mut v = json!({ "buffer": 0, "byteOffset": off, "byteLength": bytes.len() });
        if let Some(t) = target {
            v["target"] = json!(t);
        }
        self.views.push(v);
        self.views.len() - 1
    }

    fn floats(&mut self, data: &[f32], comps: usize, kind: &str, minmax: bool) -> usize {
        let bytes: Vec<u8> = data.iter().flat_map(|f| f.to_le_bytes()).collect();
        let view = self.view(&bytes, Some(34962));
        let count = data.len() / comps;
        let mut a = json!({ "bufferView": view, "componentType": 5126, "count": count, "type": kind });
        if minmax && count > 0 {
            let mut lo = vec![f32::MAX; comps];
            let mut hi = vec![f32::MIN; comps];
            for c in data.chunks_exact(comps) {
                for k in 0..comps {
                    lo[k] = lo[k].min(c[k]);
                    hi[k] = hi[k].max(c[k]);
                }
            }
            a["min"] = json!(lo);
            a["max"] = json!(hi);
        }
        self.accessors.push(a);
        self.accessors.len() - 1
    }

    fn indices(&mut self, idx: &[u32]) -> usize {
        let bytes: Vec<u8> = idx.iter().flat_map(|i| i.to_le_bytes()).collect();
        let view = self.view(&bytes, Some(34963));
        self.accessors.push(json!({ "bufferView": view, "componentType": 5125, "count": idx.len(), "type": "SCALAR" }));
        self.accessors.len() - 1
    }
}

fn png(tex: &forja_assets::TextureData, flip_green: bool) -> Vec<u8> {
    let mut data = tex.rgba8.to_vec();
    if flip_green {
        for p in data.chunks_exact_mut(4) {
            p[1] = 255 - p[1];
        }
    }
    let img = image::RgbaImage::from_raw(tex.width, tex.height, data).expect("rgba8 size");
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).expect("png encode");
    out.into_inner()
}

/// Encodes the asset as GLB bytes.
pub fn to_glb(asset: &TreeAsset) -> Vec<u8> {
    let sc = &asset.scene;
    let mut b = Builder { bin: Vec::new(), views: Vec::new(), accessors: Vec::new() };
    let normal_tex: Vec<bool> = (0..sc.textures.len()).map(|i| sc.materials.iter().any(|m| m.normal_tex == Some(i))).collect();
    let mut images = Vec::new();
    for (i, t) in sc.textures.iter().enumerate() {
        let bytes = png(t, normal_tex[i]);
        let v = b.view(&bytes, None);
        images.push(json!({ "bufferView": v, "mimeType": "image/png", "name": t.name }));
    }
    let textures: Vec<Value> = (0..sc.textures.len()).map(|i| json!({ "source": i, "sampler": 0 })).collect();
    let materials: Vec<Value> = sc
        .materials
        .iter()
        .map(|m| {
            let mut pbr = json!({ "baseColorFactor": m.base_color.to_array(), "metallicFactor": m.metallic, "roughnessFactor": m.roughness });
            if let Some(t) = m.base_color_tex {
                pbr["baseColorTexture"] = json!({ "index": t });
            }
            if let Some(t) = m.metallic_roughness_tex {
                pbr["metallicRoughnessTexture"] = json!({ "index": t });
            }
            let mut v = json!({ "name": m.name, "pbrMetallicRoughness": pbr, "doubleSided": m.double_sided });
            if let Some(t) = m.normal_tex {
                v["normalTexture"] = json!({ "index": t, "scale": m.normal_scale });
            }
            if let Some(t) = m.occlusion_tex {
                v["occlusionTexture"] = json!({ "index": t });
            }
            if let forja_assets::AlphaMode::Mask(c) = m.alpha {
                v["alphaMode"] = json!("MASK");
                v["alphaCutoff"] = json!(c as f32 / 255.0);
            }
            if let Some(f) = m.foliage {
                v["extras"] = json!({ "foliage_translucency": f.translucency });
            }
            v
        })
        .collect();
    let mut meshes = Vec::new();
    for lod in 0..3 {
        let mut prims = Vec::new();
        for (pi, p) in sc.meshes[0].iter().enumerate() {
            let idx: &[u32] = if lod == 0 { &p.indices } else { p.lods.get(lod - 1).map(|v| v.as_slice()).unwrap_or(&[]) };
            if idx.is_empty() {
                continue;
            }
            // Compact the shared vertex buffer to this level's vertices.
            let mut remap = vec![u32::MAX; p.positions.len()];
            let mut used = Vec::new();
            let local: Vec<u32> = idx
                .iter()
                .map(|&i| {
                    if remap[i as usize] == u32::MAX {
                        remap[i as usize] = used.len() as u32;
                        used.push(i as usize);
                    }
                    remap[i as usize]
                })
                .collect();
            let pos: Vec<f32> = used.iter().flat_map(|&i| p.positions[i].to_array()).collect();
            let nrm: Vec<f32> = used.iter().flat_map(|&i| p.normals[i].to_array()).collect();
            let uv: Vec<f32> = used.iter().flat_map(|&i| p.uvs[i].to_array()).collect();
            let wind: Vec<f32> = used.iter().map(|&i| asset.wind[pi].get(i).copied().unwrap_or(0.0)).collect();
            let a_pos = b.floats(&pos, 3, "VEC3", true);
            let a_nrm = b.floats(&nrm, 3, "VEC3", false);
            let a_uv = b.floats(&uv, 2, "VEC2", false);
            let a_wind = b.floats(&wind, 1, "SCALAR", false);
            let a_idx = b.indices(&local);
            prims.push(json!({ "attributes": { "POSITION": a_pos, "NORMAL": a_nrm, "TEXCOORD_0": a_uv, "_WIND": a_wind }, "indices": a_idx, "material": p.material }));
        }
        meshes.push(json!({ "name": format!("{}_lod{lod}", sc.name), "primitives": prims }));
    }
    let c = &asset.capsule;
    let nodes = json!([
        {
            "name": format!("{}_LOD0", sc.name),
            "mesh": 0,
            "extensions": { "MSFT_lod": { "ids": [1, 2] } },
            "extras": {
                "MSFT_screencoverage": [0.25, 0.06, 0.0],
                "species": asset.species,
                "seed": asset.seed,
                "height": asset.height,
                "capsule": { "a": c.a.to_array(), "b": c.b.to_array(), "radius": c.radius },
                "lod_triangles": asset.lod_triangles,
            }
        },
        { "name": format!("{}_LOD1", sc.name), "mesh": 1 },
        { "name": format!("{}_LOD2", sc.name), "mesh": 2 }
    ]);
    b.align();
    let doc = json!({
        "asset": { "version": "2.0", "generator": "forja_flora" },
        "extensionsUsed": ["MSFT_lod"],
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": nodes,
        "meshes": meshes,
        "materials": materials,
        "textures": textures,
        "images": images,
        "samplers": [{ "magFilter": 9729, "minFilter": 9987, "wrapS": 10497, "wrapT": 10497 }],
        "accessors": b.accessors,
        "bufferViews": b.views,
        "buffers": [{ "byteLength": b.bin.len() }],
    });
    let mut js = serde_json::to_vec(&doc).expect("json");
    while js.len() % 4 != 0 {
        js.push(b' ');
    }
    let total = 12 + 8 + js.len() + 8 + b.bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(js.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&js);
    out.extend_from_slice(&(b.bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&b.bin);
    out
}

pub fn write_glb(asset: &TreeAsset, path: &Path) -> std::io::Result<()> {
    std::fs::write(path, to_glb(asset))
}
