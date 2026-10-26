//! Minimal Wavefront OBJ loader: positions, UVs, normals, polygon faces
//! (fan-triangulated) and `usemtl` groups. A texture next to the file with
//! the same stem (`bike.png` for `bike.obj`) becomes the base color.

use std::collections::HashMap;
use std::path::Path;

use forja_math::{Vec2, Vec3};

use crate::{AssetError, MaterialData, MeshData, NodeData, SceneData, load_texture};

pub fn load_obj(path: &Path) -> Result<SceneData, AssetError> {
    let text = std::fs::read_to_string(path).map_err(|e| AssetError::Io(format!("{}: {e}", path.display())))?;
    let mut scene = parse_obj(&text)?;
    scene.name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    for ext in ["png", "jpg", "jpeg"] {
        let tex = path.with_extension(ext);
        if tex.is_file() {
            scene.textures.push(load_texture(&tex, true)?);
            for m in &mut scene.materials {
                m.base_color_tex = Some(0);
            }
            break;
        }
    }
    Ok(scene)
}

pub fn parse_obj(text: &str) -> Result<SceneData, AssetError> {
    let (mut v, mut vt, mut vn) = (Vec::new(), Vec::new(), Vec::new());
    let mut groups: Vec<(String, MeshData, HashMap<(i64, i64, i64), u32>)> = vec![("default".into(), MeshData::default(), HashMap::new())];
    let parse_f = |s: Option<&str>, line: usize| -> Result<f32, AssetError> {
        s.and_then(|x| x.parse().ok()).ok_or_else(|| AssetError::Format(format!("obj line {line}: bad number")))
    };
    for (ln, line) in text.lines().enumerate() {
        let mut it = line.split_whitespace();
        match it.next() {
            Some("v") => v.push(Vec3::new(parse_f(it.next(), ln)?, parse_f(it.next(), ln)?, parse_f(it.next(), ln)?)),
            Some("vt") => vt.push(Vec2::new(parse_f(it.next(), ln)?, 1.0 - parse_f(it.next(), ln).unwrap_or(0.0))),
            Some("vn") => vn.push(Vec3::new(parse_f(it.next(), ln)?, parse_f(it.next(), ln)?, parse_f(it.next(), ln)?)),
            Some("usemtl") => {
                let name = it.next().unwrap_or("material").to_string();
                if !groups.iter().any(|g| g.0 == name) {
                    groups.push((name.clone(), MeshData::default(), HashMap::new()));
                }
                let i = groups.iter().position(|g| g.0 == name).expect("just inserted");
                let g = groups.remove(i);
                groups.push(g);
            }
            Some("f") => {
                let g = groups.last_mut().expect("default group");
                let mut face = Vec::new();
                for tok in it {
                    let mut p = tok.split('/');
                    let idx = |s: Option<&str>, len: usize| -> i64 {
                        match s.and_then(|x| x.parse::<i64>().ok()) {
                            Some(i) if i < 0 => len as i64 + i,
                            Some(i) => i - 1,
                            None => -1,
                        }
                    };
                    let key = (idx(p.next(), v.len()), idx(p.next(), vt.len()), idx(p.next(), vn.len()));
                    if key.0 < 0 || key.0 as usize >= v.len() {
                        return Err(AssetError::Format(format!("obj line {ln}: vertex index out of range")));
                    }
                    let mesh = &mut g.1;
                    let id = *g.2.entry(key).or_insert_with(|| {
                        mesh.positions.push(v[key.0 as usize]);
                        if key.1 >= 0 && (key.1 as usize) < vt.len() {
                            mesh.uvs.push(vt[key.1 as usize]);
                        }
                        if key.2 >= 0 && (key.2 as usize) < vn.len() {
                            mesh.normals.push(vn[key.2 as usize]);
                        }
                        mesh.positions.len() as u32 - 1
                    });
                    face.push(id);
                }
                for k in 1..face.len().saturating_sub(1) {
                    g.1.indices.extend([face[0], face[k], face[k + 1]]);
                }
            }
            _ => {}
        }
    }
    let mut scene = SceneData::default();
    let mut prims = Vec::new();
    for (name, mut mesh, _) in groups {
        if mesh.indices.is_empty() {
            continue;
        }
        // Mixed faces with and without UVs/normals: drop partial attributes.
        if mesh.uvs.len() != mesh.positions.len() {
            mesh.uvs.clear();
        }
        if mesh.normals.len() != mesh.positions.len() {
            mesh.normals.clear();
        }
        mesh.material = scene.materials.len();
        mesh.finalize();
        scene.materials.push(MaterialData { name, ..Default::default() });
        prims.push(mesh);
    }
    if prims.is_empty() {
        return Err(AssetError::Format("obj has no faces".into()));
    }
    scene.meshes.push(prims);
    scene.nodes.push(NodeData { name: "obj".into(), mesh: Some(0), ..Default::default() });
    scene.roots.push(0);
    Ok(scene)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quads_and_negative_indices() {
        let s = parse_obj("v 0 0 0\nv 1 0 0\nv 1 0 1\nv 0 0 1\nvt 0 0\nf -4/1 -3/1 -2/1 -1/1\n").unwrap();
        assert_eq!(s.meshes[0][0].indices.len(), 6);
        assert_eq!(s.meshes[0][0].positions.len(), 4);
    }

    #[test]
    fn rejects_bad_indices() {
        assert!(parse_obj("v 0 0 0\nf 1 2 3\n").is_err());
    }
}
