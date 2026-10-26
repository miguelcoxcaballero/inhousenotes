//! Realistic CC0 humans assembled from MakeHuman data: the base body mesh,
//! the default 163-bone skeleton with its weights, a skin, and proxies
//! (clothes, shoes, hair, eyebrows, eyelashes, eyes) fitted to the body
//! through their `.mhclo` vertex references.
//!
//! Output is an ordinary skinned [`SceneData`] in meters, +Y up, facing +Z,
//! so the renderer and `forja_anim` treat it like any glTF character.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use forja_math::{Mat4, Vec2, Vec3, Vec4};

use crate::{AlphaMode, AssetError, MaterialData, MeshData, NodeData, SceneData, SkinData, TextureData, Trs, load_texture};

/// MakeHuman meshes are in decimeters.
const SCALE: f32 = 0.1;
const MAX_INFLUENCES: usize = 4;

/// Which assets to dress the body with (folder names inside the pack).
#[derive(Clone, Debug)]
pub struct HumanSpec {
    pub name: String,
    pub skin: String,
    pub proxies: Vec<String>,
    pub eye_color: String,
}

impl HumanSpec {
    pub fn casual_male() -> Self {
        Self {
            name: "casual_male".into(),
            skin: "skins/young_caucasian_male".into(),
            proxies: vec![
                "clothes/male_casualsuit04".into(),
                "clothes/shoes01".into(),
                "hair/short02".into(),
                "eyebrows/eyebrow001".into(),
                "eyelashes/eyelashes01".into(),
                "eyes/high-poly".into(),
            ],
            eye_color: "brown".into(),
        }
    }

    /// Ready-made outfits: casual_male, worker, tourist, local, commuter.
    pub fn presets() -> Vec<HumanSpec> {
        let base = Self::casual_male();
        let face = |hair: &str| vec![hair.to_string(), "eyebrows/eyebrow001".into(), "eyelashes/eyelashes01".into(), "eyes/high-poly".into()];
        let spec = |name: &str, skin: &str, clothes: &[&str], hair: &str, eyes: &str| HumanSpec {
            name: name.into(),
            skin: skin.into(),
            proxies: clothes.iter().map(|c| c.to_string()).chain(face(hair)).collect(),
            eye_color: eyes.into(),
        };
        vec![
            base.clone(),
            spec("worker", "skins/young_african_male", &["clothes/male_worksuit01", "clothes/shoes06"], "hair/short04", "brown"),
            spec("tourist", "skins/young_caucasian_female", &["clothes/female_casualsuit01", "clothes/shoes02"], "hair/ponytail01", "blue"),
            spec("local", "skins/middleage_asian_female", &["clothes/female_casualsuit01", "clothes/shoes02"], "hair/bob02", "brownlight"),
            HumanSpec { name: "commuter".into(), proxies: { let mut p = base.proxies.clone(); p[0] = "clothes/male_casualsuit02".into(); p }, ..base },
        ]
    }
}

struct ObjMesh {
    positions: Vec<Vec3>,
    uvs: Vec<Vec2>,
    /// Face corners `(position index, uv index)`, polygons as written.
    faces: Vec<Vec<(usize, usize)>>,
    /// Group of each face.
    groups: Vec<String>,
}

fn parse_obj(text: &str) -> ObjMesh {
    let mut m = ObjMesh { positions: Vec::new(), uvs: Vec::new(), faces: Vec::new(), groups: Vec::new() };
    let mut group = String::new();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        match it.next() {
            Some("v") => {
                let v: Vec<f32> = it.take(3).filter_map(|x| x.parse().ok()).collect();
                if v.len() == 3 {
                    m.positions.push(Vec3::new(v[0], v[1], v[2]));
                }
            }
            Some("vt") => {
                let v: Vec<f32> = it.take(2).filter_map(|x| x.parse().ok()).collect();
                if v.len() == 2 {
                    m.uvs.push(Vec2::new(v[0], 1.0 - v[1]));
                }
            }
            Some("g") => group = it.next().unwrap_or("").to_string(),
            Some("f") => {
                let face: Vec<(usize, usize)> = it
                    .filter_map(|c| {
                        let mut p = c.split('/');
                        let v = p.next()?.parse::<usize>().ok()?.checked_sub(1)?;
                        let t = p.next().and_then(|t| t.parse::<usize>().ok()).and_then(|t| t.checked_sub(1)).unwrap_or(usize::MAX);
                        Some((v, t))
                    })
                    .collect();
                if face.len() >= 3 {
                    m.faces.push(face);
                    m.groups.push(group.clone());
                }
            }
            _ => {}
        }
    }
    m
}

/// `.mhclo` proxy: per proxy vertex, three body references, weights and an offset.
struct Proxy {
    obj_file: String,
    material: Option<String>,
    weights_file: Option<String>,
    scales: [(usize, usize, f32); 3],
    refs: Vec<([usize; 3], [f32; 3], [f32; 3])>,
    delete: Vec<usize>,
}

fn parse_mhclo(text: &str) -> Result<Proxy, AssetError> {
    let mut p = Proxy { obj_file: String::new(), material: None, weights_file: None, scales: [(0, 0, 1.0); 3], refs: Vec::new(), delete: Vec::new() };
    let mut section = "";
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let toks: Vec<&str> = line.split_whitespace().collect();
        match toks[0] {
            "verts" => {
                section = "verts";
                continue;
            }
            "delete_verts" => {
                section = "delete";
                continue;
            }
            "obj_file" => p.obj_file = toks.get(1).unwrap_or(&"").to_string(),
            "material" => p.material = toks.get(1).map(|s| s.to_string()),
            "vertexboneweights_file" => p.weights_file = toks.get(1).map(|s| s.to_string()),
            k @ ("x_scale" | "y_scale" | "z_scale") if toks.len() >= 4 => {
                let i = match k {
                    "x_scale" => 0,
                    "y_scale" => 1,
                    _ => 2,
                };
                p.scales[i] = (toks[1].parse().unwrap_or(0), toks[2].parse().unwrap_or(0), toks[3].parse().unwrap_or(1.0));
            }
            _ if section == "verts" => {
                let n: Vec<f32> = toks.iter().filter_map(|t| t.parse::<f32>().ok()).collect();
                if n.len() >= 9 {
                    p.refs.push(([n[0] as usize, n[1] as usize, n[2] as usize], [n[3], n[4], n[5]], [n[6], n[7], n[8]]));
                } else if n.len() == 1 {
                    p.refs.push(([n[0] as usize; 3], [1.0, 0.0, 0.0], [0.0; 3]));
                }
            }
            _ if section == "delete" => {
                // "a - b" ranges and single indices.
                let mut i = 0;
                while i < toks.len() {
                    if i + 2 < toks.len() && toks[i + 1] == "-" {
                        if let (Ok(a), Ok(b)) = (toks[i].parse::<usize>(), toks[i + 2].parse::<usize>()) {
                            p.delete.extend(a..=b);
                        }
                        i += 3;
                    } else {
                        if let Ok(a) = toks[i].parse::<usize>() {
                            p.delete.push(a);
                        }
                        i += 1;
                    }
                }
            }
            _ => {}
        }
    }
    if p.obj_file.is_empty() {
        return Err(AssetError::Format("mhclo without obj_file".into()));
    }
    Ok(p)
}

/// Bone name -> list of (vertex, weight).
fn parse_weights(text: &str) -> Result<HashMap<String, Vec<(usize, f32)>>, AssetError> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| AssetError::Format(format!("mhw: {e}")))?;
    let mut out = HashMap::new();
    if let Some(w) = v.get("weights").and_then(|w| w.as_object()) {
        for (bone, list) in w {
            let pairs = list
                .as_array()
                .map(|a| a.iter().filter_map(|p| Some((p.get(0)?.as_u64()? as usize, p.get(1)?.as_f64()? as f32))).collect())
                .unwrap_or_default();
            out.insert(bone.clone(), pairs);
        }
    }
    Ok(out)
}

struct MhMaterial {
    diffuse: Option<PathBuf>,
    normal: Option<PathBuf>,
    ao: Option<PathBuf>,
    transparent: bool,
}

fn parse_mhmat(path: &Path) -> MhMaterial {
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut m = MhMaterial { diffuse: None, normal: None, ao: None, transparent: false };
    let Ok(text) = std::fs::read_to_string(path) else { return m };
    for line in text.lines() {
        let toks: Vec<&str> = line.split_whitespace().collect();
        match toks.as_slice() {
            ["diffuseTexture", f, ..] => m.diffuse = Some(dir.join(f)),
            ["normalmapTexture", f, ..] => m.normal = Some(dir.join(f)),
            ["aomapTexture", f, ..] => m.ao = Some(dir.join(f)),
            ["transparent", v, ..] => m.transparent = v.eq_ignore_ascii_case("true"),
            _ => {}
        }
    }
    m
}

fn add_texture(scene: &mut SceneData, cache: &mut HashMap<PathBuf, usize>, path: &Path, srgb: bool) -> Option<usize> {
    if let Some(i) = cache.get(path) {
        return Some(*i);
    }
    let t: TextureData = load_texture(path, srgb).map_err(|e| log::warn!("{e}")).ok()?;
    scene.textures.push(t);
    cache.insert(path.to_path_buf(), scene.textures.len() - 1);
    Some(scene.textures.len() - 1)
}

/// Bone rest frames: Y along head -> tail, X from the bone's roll plane.
fn bone_frames(skel: &serde_json::Value, base: &[Vec3]) -> Result<(Vec<String>, Vec<Option<usize>>, Vec<Mat4>), AssetError> {
    let joints = skel.get("joints").and_then(|j| j.as_object()).ok_or_else(|| AssetError::Format("mhskel: no joints".into()))?;
    let planes = skel.get("planes").and_then(|j| j.as_object());
    let bones = skel.get("bones").and_then(|j| j.as_object()).ok_or_else(|| AssetError::Format("mhskel: no bones".into()))?;
    let joint_pos = |name: &str| -> Vec3 {
        let ids = joints.get(name).and_then(|v| v.as_array()).cloned().unwrap_or_default();
        let pts: Vec<Vec3> = ids.iter().filter_map(|i| i.as_u64()).filter_map(|i| base.get(i as usize).copied()).collect();
        if pts.is_empty() { Vec3::ZERO } else { pts.iter().copied().sum::<Vec3>() / pts.len() as f32 }
    };
    // Parents before children.
    let mut names: Vec<String> = Vec::new();
    let mut pending: Vec<&String> = bones.keys().collect();
    pending.sort();
    while !pending.is_empty() {
        let before = names.len();
        pending.retain(|n| {
            let parent = bones[n.as_str()].get("parent").and_then(|p| p.as_str());
            if parent.is_none_or(|p| names.iter().any(|x| x == p)) {
                names.push((*n).clone());
                false
            } else {
                true
            }
        });
        if names.len() == before {
            return Err(AssetError::Format("mhskel: cyclic bones".into()));
        }
    }
    let index: HashMap<&str, usize> = names.iter().enumerate().map(|(i, n)| (n.as_str(), i)).collect();
    let mut parents = Vec::new();
    let mut world = Vec::new();
    for n in &names {
        let b = &bones[n.as_str()];
        parents.push(b.get("parent").and_then(|p| p.as_str()).and_then(|p| index.get(p).copied()));
        let head = joint_pos(b.get("head").and_then(|v| v.as_str()).unwrap_or("")) * SCALE;
        let tail = joint_pos(b.get("tail").and_then(|v| v.as_str()).unwrap_or("")) * SCALE;
        let y = (tail - head).normalize_or(Vec3::Y);
        let plane_normal = b
            .get("rotation_plane")
            .and_then(|p| p.as_str())
            .and_then(|p| planes?.get(p)?.as_array().cloned())
            .and_then(|pts| {
                let q: Vec<Vec3> = pts.iter().filter_map(|j| j.as_str()).map(|j| joint_pos(j) * SCALE).collect();
                (q.len() == 3).then(|| (q[1] - q[0]).cross(q[2] - q[0]).normalize_or_zero())
            })
            .filter(|n| n.length_squared() > 0.5 && n.cross(y).length_squared() > 1e-4)
            .unwrap_or_else(|| forja_math::orthonormal_basis(y).0);
        let z = plane_normal.cross(y).normalize();
        let x = y.cross(z).normalize();
        world.push(Mat4::from_cols(x.extend(0.0), y.extend(0.0), z.extend(0.0), head.extend(1.0)));
    }
    Ok((names, parents, world))
}

/// Builds a dressed, rigged human. `base_dir` holds `base.obj`,
/// `default.mhskel` and `default_weights.mhw`; `pack_dir` the CC0 system
/// assets (skins, clothes, hair, ...).
pub fn build_human(base_dir: &Path, pack_dir: &Path, spec: &HumanSpec) -> Result<SceneData, AssetError> {
    let read = |p: PathBuf| std::fs::read_to_string(&p).map_err(|e| AssetError::Io(format!("{}: {e}", p.display())));
    let base = parse_obj(&read(base_dir.join("base.obj"))?);
    let skel: serde_json::Value = serde_json::from_str(&read(base_dir.join("default.mhskel"))?).map_err(|e| AssetError::Format(format!("mhskel: {e}")))?;
    let body_weights = parse_weights(&read(base_dir.join("default_weights.mhw"))?)?;
    let (bone_names, parents, bone_world) = bone_frames(&skel, &base.positions)?;
    let bone_index: HashMap<&str, usize> = bone_names.iter().enumerate().map(|(i, n)| (n.as_str(), i)).collect();
    // Per base vertex: bone influences.
    let mut vw: Vec<Vec<(usize, f32)>> = vec![Vec::new(); base.positions.len()];
    for (bone, list) in &body_weights {
        if let Some(&b) = bone_index.get(bone.as_str()) {
            for &(v, w) in list {
                if v < vw.len() {
                    vw[v].push((b, w));
                }
            }
        }
    }
    let base_pos: Vec<Vec3> = base.positions.iter().map(|p| *p * SCALE).collect();

    let mut scene = SceneData { name: spec.name.clone(), ..Default::default() };
    let mut tex_cache = HashMap::new();
    let mut prims: Vec<MeshData> = Vec::new();
    let mut deleted = vec![false; base.positions.len()];

    // Proxies first: they decide which body faces are hidden.
    for proxy_dir in &spec.proxies {
        let dir = pack_dir.join(proxy_dir);
        let Some(clo_path) = std::fs::read_dir(&dir).ok().and_then(|rd| rd.flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|e| e == "mhclo"))) else {
            log::warn!("{}: no .mhclo", dir.display());
            continue;
        };
        let proxy = parse_mhclo(&read(clo_path.clone())?)?;
        for d in &proxy.delete {
            if *d < deleted.len() {
                deleted[*d] = true;
            }
        }
        let obj = parse_obj(&read(dir.join(&proxy.obj_file))?);
        // Offset scale factors from reference vertex distances.
        let mut s = [1.0f32; 3];
        for (axis, (a, b, d)) in proxy.scales.iter().enumerate() {
            if let (Some(pa), Some(pb)) = (base.positions.get(*a), base.positions.get(*b)) {
                s[axis] = (pa[axis] - pb[axis]).abs() / d.max(1e-6);
            }
        }
        let fitted: Vec<Vec3> = proxy
            .refs
            .iter()
            .map(|(v, w, off)| {
                let p = |i: usize| base.positions.get(i).copied().unwrap_or(Vec3::ZERO);
                (p(v[0]) * w[0] + p(v[1]) * w[1] + p(v[2]) * w[2] + Vec3::new(off[0] * s[0], off[1] * s[1], off[2] * s[2])) * SCALE
            })
            .collect();
        // Weights: the proxy's own file, else interpolated from the body.
        let own = proxy.weights_file.as_ref().and_then(|f| read(dir.join(f)).ok()).and_then(|t| parse_weights(&t).ok());
        let mut pw: Vec<Vec<(usize, f32)>> = vec![Vec::new(); fitted.len()];
        match &own {
            Some(map) => {
                for (bone, list) in map {
                    if let Some(&b) = bone_index.get(bone.as_str()) {
                        for &(v, w) in list {
                            if v < pw.len() {
                                pw[v].push((b, w));
                            }
                        }
                    }
                }
            }
            None => {
                for (i, (v, w, _)) in proxy.refs.iter().enumerate() {
                    let mut acc: HashMap<usize, f32> = HashMap::new();
                    for k in 0..3 {
                        if let Some(list) = vw.get(v[k]) {
                            for &(b, bw) in list {
                                *acc.entry(b).or_default() += bw * w[k];
                            }
                        }
                    }
                    pw[i] = acc.into_iter().collect();
                }
            }
        }
        let mat_path = match &proxy.material {
            Some(m) if proxy_dir.starts_with("eyes") => dir.join(m).parent().unwrap_or(&dir).join(format!("{}.mhmat", spec.eye_color)),
            Some(m) => dir.join(m),
            None => dir.join(format!("{}.mhmat", dir.file_name().unwrap_or_default().to_string_lossy())),
        };
        let mhm = parse_mhmat(&mat_path);
        let material = push_material(&mut scene, &mut tex_cache, &mhm, proxy_dir);
        prims.push(build_mesh(&obj, &fitted, &pw, material, None));
    }

    // Body with the skin, minus faces hidden under clothes and helpers.
    let skin_mat = parse_mhmat(&std::fs::read_dir(pack_dir.join(&spec.skin)).ok().and_then(|rd| rd.flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|e| e == "mhmat"))).unwrap_or_default());
    let body_mat = push_material(&mut scene, &mut tex_cache, &skin_mat, "skin");
    let keep = |fi: usize| base.groups[fi] == "body" && !base.faces[fi].iter().any(|(v, _)| deleted.get(*v).copied().unwrap_or(false));
    prims.push(build_mesh(&base, &base_pos, &vw, body_mat, Some(&keep)));

    // Skeleton nodes: node 0 is the mesh holder, bones follow.
    let first_bone = 1;
    scene.nodes.push(NodeData { name: spec.name.clone(), mesh: Some(0), skin: Some(0), ..Default::default() });
    for (i, name) in bone_names.iter().enumerate() {
        let local = match parents[i] {
            Some(p) => bone_world[p].inverse() * bone_world[i],
            None => bone_world[i],
        };
        let (_, r, t) = local.to_scale_rotation_translation();
        scene.nodes.push(NodeData {
            name: name.clone(),
            parent: parents[i].map(|p| p + first_bone),
            children: Vec::new(),
            local: Trs { t, r: r.normalize(), s: Vec3::ONE },
            mesh: None,
            skin: None,
        });
    }
    for i in 0..bone_names.len() {
        if let Some(p) = parents[i] {
            scene.nodes[p + first_bone].children.push(i + first_bone);
        }
    }
    scene.roots = vec![0];
    for (i, _) in bone_names.iter().enumerate() {
        if parents[i].is_none() {
            scene.roots.push(i + first_bone);
        }
    }
    scene.skins.push(SkinData {
        name: "makehuman_default".into(),
        joints: (0..bone_names.len()).map(|i| i + first_bone).collect(),
        inverse_bind: bone_world.iter().map(|m| m.inverse()).collect(),
    });
    for p in &mut prims {
        p.finalize();
    }
    scene.meshes.push(prims);
    Ok(scene)
}

fn push_material(scene: &mut SceneData, cache: &mut HashMap<PathBuf, usize>, m: &MhMaterial, kind: &str) -> usize {
    let base_color_tex = m.diffuse.as_ref().and_then(|p| add_texture(scene, cache, p, true));
    let normal_tex = m.normal.as_ref().and_then(|p| add_texture(scene, cache, p, false));
    let occlusion_tex = m.ao.as_ref().and_then(|p| add_texture(scene, cache, p, false));
    let hairy = kind.starts_with("hair") || kind.starts_with("eyebrows") || kind.starts_with("eyelashes");
    let (roughness, metallic) = match kind {
        k if k.starts_with("eyes") => (0.05, 0.0),
        k if k.starts_with("hair") => (0.45, 0.0),
        "skin" => (0.55, 0.0),
        k if k.contains("shoes") => (0.4, 0.0),
        _ => (0.8, 0.0),
    };
    scene.materials.push(MaterialData {
        name: kind.to_string(),
        base_color: Vec4::ONE,
        base_color_tex,
        metallic,
        roughness,
        normal_tex,
        occlusion_tex,
        alpha: if hairy || m.transparent && kind.starts_with("eyes") { AlphaMode::Mask(110) } else { AlphaMode::Opaque },
        double_sided: hairy,
        ..Default::default()
    });
    scene.materials.len() - 1
}

/// Triangulates faces, splits vertices per UV and keeps the top 4 weights.
fn build_mesh(obj: &ObjMesh, pos: &[Vec3], weights: &[Vec<(usize, f32)>], material: usize, keep: Option<&dyn Fn(usize) -> bool>) -> MeshData {
    let mut mesh = MeshData { material, ..Default::default() };
    let mut map: HashMap<(usize, usize), u32> = HashMap::new();
    for (fi, face) in obj.faces.iter().enumerate() {
        if keep.is_some_and(|k| !k(fi)) {
            continue;
        }
        let mut ids = Vec::with_capacity(face.len());
        for &(v, t) in face {
            if v >= pos.len() {
                continue;
            }
            let id = *map.entry((v, t)).or_insert_with(|| {
                mesh.positions.push(pos[v]);
                mesh.uvs.push(obj.uvs.get(t).copied().unwrap_or(Vec2::ZERO));
                let mut w = weights.get(v).cloned().unwrap_or_default();
                w.sort_by(|a, b| b.1.total_cmp(&a.1));
                w.truncate(MAX_INFLUENCES);
                let sum: f32 = w.iter().map(|x| x.1).sum::<f32>().max(1e-6);
                let mut j = [0u16; 4];
                let mut ww = [0.0f32; 4];
                for (k, (b, x)) in w.iter().enumerate() {
                    j[k] = *b as u16;
                    ww[k] = x / sum;
                }
                if w.is_empty() {
                    ww[0] = 1.0;
                }
                mesh.joints.push(j);
                mesh.weights.push(ww);
                mesh.positions.len() as u32 - 1
            });
            ids.push(id);
        }
        // MakeHuman faces wind counter-clockwise seen from outside.
        for k in 1..ids.len().saturating_sub(1) {
            mesh.indices.extend([ids[0], ids[k], ids[k + 1]]);
        }
    }
    mesh
}

/// Simple sanity data for tests and tools.
pub fn human_height(scene: &SceneData) -> f32 {
    let b = scene.bounds();
    b.max.y - b.min.y
}
