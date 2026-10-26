//! Binary FBX (7.x) loader: node tree with zlib-compressed arrays, object
//! connections, model transforms (pre/post rotation, rotation order, bind
//! pose), polygon meshes split per material, skin clusters and textures.
//! Produces the same [`SceneData`] as the glTF loader, in meters, +Y up.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use forja_math::{Mat4, Quat, Vec2, Vec3, Vec4};

use crate::{AlphaMode, AssetError, MaterialData, MeshData, NodeData, SceneData, SkinData, TextureData, Trs, load_texture};

#[derive(Clone, Debug)]
pub enum Prop {
    Bool(bool),
    I16(i16),
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
    Str(String),
    Raw(Vec<u8>),
    ArrF32(Vec<f32>),
    ArrF64(Vec<f64>),
    ArrI32(Vec<i32>),
    ArrI64(Vec<i64>),
    ArrBool(Vec<bool>),
}

impl Prop {
    pub fn as_i64(&self) -> Option<i64> {
        Some(match self {
            Prop::I16(v) => *v as i64,
            Prop::I32(v) => *v as i64,
            Prop::I64(v) => *v,
            Prop::Bool(v) => *v as i64,
            Prop::F32(v) => *v as i64,
            Prop::F64(v) => *v as i64,
            _ => return None,
        })
    }

    pub fn as_f64(&self) -> Option<f64> {
        Some(match self {
            Prop::F32(v) => *v as f64,
            Prop::F64(v) => *v,
            Prop::I16(v) => *v as f64,
            Prop::I32(v) => *v as f64,
            Prop::I64(v) => *v as f64,
            Prop::Bool(v) => *v as i64 as f64,
            _ => return None,
        })
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Prop::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn f64s(&self) -> Option<Vec<f64>> {
        match self {
            Prop::ArrF64(v) => Some(v.clone()),
            Prop::ArrF32(v) => Some(v.iter().map(|x| *x as f64).collect()),
            _ => None,
        }
    }

    pub fn i32s(&self) -> Option<Vec<i32>> {
        match self {
            Prop::ArrI32(v) => Some(v.clone()),
            Prop::ArrI64(v) => Some(v.iter().map(|x| *x as i32).collect()),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Node {
    pub name: String,
    pub props: Vec<Prop>,
    pub children: Vec<Node>,
}

impl Node {
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }

    pub fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }

    fn id(&self) -> i64 {
        self.props.first().and_then(Prop::as_i64).unwrap_or(0)
    }

    /// Object name without the `\0\x01Class` suffix.
    fn object_name(&self) -> String {
        let raw = self.props.get(1).and_then(Prop::as_str).unwrap_or("");
        raw.split('\0').next().unwrap_or(raw).trim_start_matches("Model::").to_string()
    }

    fn subclass(&self) -> &str {
        self.props.get(2).and_then(Prop::as_str).unwrap_or("")
    }

    /// `Properties70` entries: name → numeric values.
    fn p70(&self) -> HashMap<String, Vec<f64>> {
        let mut out = HashMap::new();
        if let Some(p) = self.child("Properties70") {
            for e in p.all("P") {
                if let Some(name) = e.props.first().and_then(Prop::as_str) {
                    let vals: Vec<f64> = e.props.iter().skip(4).filter_map(Prop::as_f64).collect();
                    out.insert(name.to_string(), vals);
                }
            }
        }
        out
    }

    fn array_f64(&self, name: &str) -> Vec<f64> {
        self.child(name).and_then(|n| n.props.first()).and_then(Prop::f64s).unwrap_or_default()
    }

    fn array_i32(&self, name: &str) -> Vec<i32> {
        self.child(name).and_then(|n| n.props.first()).and_then(Prop::i32s).unwrap_or_default()
    }

    fn string(&self, name: &str) -> Option<String> {
        self.child(name).and_then(|n| n.props.first()).and_then(Prop::as_str).map(str::to_string)
    }
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    wide: bool,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], AssetError> {
        let s = self.data.get(self.pos..self.pos + n).ok_or_else(|| AssetError::Format("fbx: unexpected end".into()))?;
        self.pos += n;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8, AssetError> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, AssetError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("4")))
    }

    fn u64(&mut self) -> Result<u64, AssetError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().expect("8")))
    }

    fn word(&mut self) -> Result<u64, AssetError> {
        if self.wide { self.u64() } else { self.u32().map(|v| v as u64) }
    }

    fn array<T>(&mut self, size: usize, conv: impl Fn(&[u8]) -> T) -> Result<Vec<T>, AssetError> {
        let len = self.u32()? as usize;
        let enc = self.u32()?;
        let clen = self.u32()? as usize;
        let raw = self.take(clen)?;
        let bytes: Vec<u8> = if enc == 1 {
            let mut out = Vec::with_capacity(len * size);
            flate2::read::ZlibDecoder::new(raw).read_to_end(&mut out).map_err(|e| AssetError::Format(format!("fbx: zlib: {e}")))?;
            out
        } else {
            raw.to_vec()
        };
        if bytes.len() < len * size {
            return Err(AssetError::Format("fbx: short array".into()));
        }
        Ok(bytes[..len * size].chunks_exact(size).map(conv).collect())
    }

    fn prop(&mut self) -> Result<Prop, AssetError> {
        let t = self.u8()?;
        Ok(match t {
            b'C' => Prop::Bool(self.u8()? != 0),
            b'Y' => Prop::I16(i16::from_le_bytes(self.take(2)?.try_into().expect("2"))),
            b'I' => Prop::I32(self.u32()? as i32),
            b'L' => Prop::I64(self.u64()? as i64),
            b'F' => Prop::F32(f32::from_le_bytes(self.take(4)?.try_into().expect("4"))),
            b'D' => Prop::F64(f64::from_le_bytes(self.take(8)?.try_into().expect("8"))),
            b'S' => {
                let n = self.u32()? as usize;
                Prop::Str(String::from_utf8_lossy(self.take(n)?).into_owned())
            }
            b'R' => {
                let n = self.u32()? as usize;
                Prop::Raw(self.take(n)?.to_vec())
            }
            b'f' => Prop::ArrF32(self.array(4, |b| f32::from_le_bytes(b.try_into().expect("4")))?),
            b'd' => Prop::ArrF64(self.array(8, |b| f64::from_le_bytes(b.try_into().expect("8")))?),
            b'i' => Prop::ArrI32(self.array(4, |b| i32::from_le_bytes(b.try_into().expect("4")))?),
            b'l' => Prop::ArrI64(self.array(8, |b| i64::from_le_bytes(b.try_into().expect("8")))?),
            b'b' => Prop::ArrBool(self.array(1, |b| b[0] != 0)?),
            other => return Err(AssetError::Format(format!("fbx: property type {:?}", other as char))),
        })
    }

    fn node(&mut self) -> Result<Option<Node>, AssetError> {
        let end = self.word()? as usize;
        let nprops = self.word()?;
        let _plen = self.word()?;
        let nlen = self.u8()? as usize;
        if end == 0 {
            return Ok(None);
        }
        let name = String::from_utf8_lossy(self.take(nlen)?).into_owned();
        let mut props = Vec::with_capacity(nprops as usize);
        for _ in 0..nprops {
            props.push(self.prop()?);
        }
        let mut children = Vec::new();
        while self.pos < end {
            match self.node()? {
                Some(c) => children.push(c),
                None => break,
            }
        }
        self.pos = end;
        Ok(Some(Node { name, props, children }))
    }
}

/// Parses a binary FBX file into its top-level nodes.
pub fn parse(data: &[u8]) -> Result<Vec<Node>, AssetError> {
    if data.len() < 27 || &data[..20] != b"Kaydara FBX Binary  " {
        return Err(AssetError::Format("fbx: not a binary FBX (ASCII FBX is not supported)".into()));
    }
    let version = u32::from_le_bytes(data[23..27].try_into().expect("4"));
    let mut r = Reader { data, pos: 27, wide: version >= 7500 };
    let mut nodes = Vec::new();
    while r.pos + 13 < data.len() {
        match r.node()? {
            Some(n) => nodes.push(n),
            None => break,
        }
    }
    Ok(nodes)
}

fn euler(deg: [f64; 3], order: i64) -> Quat {
    let (x, y, z) = (Quat::from_rotation_x((deg[0] as f32).to_radians()), Quat::from_rotation_y((deg[1] as f32).to_radians()), Quat::from_rotation_z((deg[2] as f32).to_radians()));
    // FBX order names the first axis applied: XYZ = X, then Y, then Z.
    match order {
        1 => y * z * x,
        2 => x * z * y,
        3 => z * x * y,
        4 => y * x * z,
        5 => x * y * z,
        _ => z * y * x,
    }
}

fn v3(p: &HashMap<String, Vec<f64>>, key: &str, default: f64) -> [f64; 3] {
    match p.get(key) {
        Some(v) if v.len() >= 3 => [v[0], v[1], v[2]],
        _ => [default; 3],
    }
}

fn mat_from(v: &[f64]) -> Option<Mat4> {
    (v.len() >= 16).then(|| Mat4::from_cols_array(&std::array::from_fn(|i| v[i] as f32)))
}

/// Converts a centimetre-space matrix to meters (scales the translation).
fn to_m(m: Mat4, s: f32) -> Mat4 {
    let mut m = m;
    m.w_axis = Vec4::new(m.w_axis.x * s, m.w_axis.y * s, m.w_axis.z * s, m.w_axis.w);
    m
}

pub fn load_fbx(path: &Path) -> Result<SceneData, AssetError> {
    let data = std::fs::read(path).map_err(|e| AssetError::Io(format!("{}: {e}", path.display())))?;
    let roots = parse(&data)?;
    let top = |name: &str| roots.iter().find(|n| n.name == name);
    let gs = top("GlobalSettings").map(Node::p70).unwrap_or_default();
    let unit = gs.get("UnitScaleFactor").and_then(|v| v.first()).copied().unwrap_or(1.0) as f32 * 0.01;
    let up_axis = gs.get("UpAxis").and_then(|v| v.first()).copied().unwrap_or(1.0) as i64;
    let objects = top("Objects").ok_or_else(|| AssetError::Format("fbx: no Objects".into()))?;
    let by_id: HashMap<i64, &Node> = objects.children.iter().map(|n| (n.id(), n)).collect();
    // Connections: child → parents (with property name for OP links).
    let mut parents: HashMap<i64, Vec<(i64, String)>> = HashMap::new();
    let mut kids: HashMap<i64, Vec<(i64, String)>> = HashMap::new();
    if let Some(c) = top("Connections") {
        for e in c.all("C") {
            let (Some(child), Some(parent)) = (e.props.get(1).and_then(Prop::as_i64), e.props.get(2).and_then(Prop::as_i64)) else { continue };
            let prop = e.props.get(3).and_then(Prop::as_str).unwrap_or("").to_string();
            parents.entry(child).or_default().push((parent, prop.clone()));
            kids.entry(parent).or_default().push((child, prop));
        }
    }
    let children_of = |id: i64, class: &str| -> Vec<&Node> {
        kids.get(&id).map(|v| v.iter().filter_map(|(c, _)| by_id.get(c).copied()).filter(|n| n.name == class).collect()).unwrap_or_default()
    };

    let mut scene = SceneData { name: path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(), ..Default::default() };
    // Models become nodes.
    let models: Vec<&Node> = objects.all("Model").collect();
    let index: HashMap<i64, usize> = models.iter().enumerate().map(|(i, m)| (m.id(), i)).collect();
    let mut local_cm: Vec<Mat4> = Vec::with_capacity(models.len());
    for m in &models {
        let p = m.p70();
        let t = v3(&p, "Lcl Translation", 0.0);
        let r = v3(&p, "Lcl Rotation", 0.0);
        let s = v3(&p, "Lcl Scaling", 1.0);
        let order = p.get("RotationOrder").and_then(|v| v.first()).copied().unwrap_or(0.0) as i64;
        let pre = euler(v3(&p, "PreRotation", 0.0), 0);
        let post = euler(v3(&p, "PostRotation", 0.0), 0);
        let rot = pre * euler(r, order) * post.inverse();
        local_cm.push(Mat4::from_scale_rotation_translation(Vec3::new(s[0] as f32, s[1] as f32, s[2] as f32), rot, Vec3::new(t[0] as f32, t[1] as f32, t[2] as f32)));
        scene.nodes.push(NodeData { name: m.object_name(), parent: None, children: Vec::new(), local: Trs::default(), mesh: None, skin: None });
    }
    for (i, m) in models.iter().enumerate() {
        let parent = parents.get(&m.id()).and_then(|ps| ps.iter().find_map(|(p, _)| index.get(p).copied()));
        scene.nodes[i].parent = parent;
        match parent {
            Some(p) => scene.nodes[p].children.push(i),
            None => scene.roots.push(i),
        }
    }
    // Globals from local transforms, then replaced by the bind pose where
    // the file has one (skinned characters are bound in that pose).
    let mut global: Vec<Mat4> = vec![Mat4::IDENTITY; models.len()];
    let mut order_v: Vec<usize> = Vec::new();
    let mut stack: Vec<usize> = scene.roots.clone();
    while let Some(i) = stack.pop() {
        order_v.push(i);
        stack.extend(scene.nodes[i].children.iter().copied());
    }
    let axis = if up_axis == 2 { Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2) } else { Mat4::IDENTITY };
    for &i in &order_v {
        let pg = scene.nodes[i].parent.map(|p| global[p]).unwrap_or(axis);
        global[i] = pg * local_cm[i];
    }
    for pose in objects.all("Pose").filter(|p| p.subclass() == "BindPose") {
        for pn in pose.all("PoseNode") {
            let id = pn.child("Node").and_then(|n| n.props.first()).and_then(Prop::as_i64).unwrap_or(0);
            if let (Some(&i), Some(m)) = (index.get(&id), mat_from(&pn.array_f64("Matrix"))) {
                global[i] = axis * m;
            }
        }
    }
    for &i in &order_v {
        let pg = scene.nodes[i].parent.map(|p| global[p]).unwrap_or(Mat4::IDENTITY);
        let local = to_m(pg.inverse() * global[i], unit);
        let (s, r, t) = local.to_scale_rotation_translation();
        scene.nodes[i].local = Trs { t, r: r.normalize(), s };
    }

    // Materials and their textures.
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut tex_cache: HashMap<PathBuf, usize> = HashMap::new();
    let mut material_index: HashMap<i64, usize> = HashMap::new();
    for mat in objects.all("Material") {
        let name = mat.object_name().split("::").next().unwrap_or("").to_string();
        let mut files: HashMap<String, String> = HashMap::new();
        for (tid, prop) in kids.get(&mat.id()).into_iter().flatten() {
            let Some(tex) = by_id.get(tid).filter(|n| n.name == "Texture") else { continue };
            let file = tex.string("RelativeFilename").or_else(|| tex.string("FileName")).unwrap_or_default();
            files.insert(prop.clone(), file);
        }
        let mut load = |slot: &[&str], suffix: &str, srgb: bool| -> Option<usize> {
            let named = slot.iter().find_map(|s| files.get(*s)).and_then(|f| resolve_texture(dir, f));
            let found = named.or_else(|| resolve_texture(dir, &format!("{name}_{suffix}.tga")))?;
            if let Some(&i) = tex_cache.get(&found) {
                return Some(i);
            }
            let t = load_texture(&found, srgb).map_err(|e| log::warn!("{e}")).ok()?;
            scene.textures.push(t);
            tex_cache.insert(found, scene.textures.len() - 1);
            Some(scene.textures.len() - 1)
        };
        let base = load(&["DiffuseColor"], "color", true);
        let normal = load(&["NormalMap", "Bump"], "normal", false);
        let spec = load(&["SpecularColor", "SpecularFactor"], "specular", false);
        let opacity = name.contains("opacity") || files.contains_key("TransparentColor");
        // Specular intensity map → glTF ORM (G = roughness).
        let orm = spec.map(|i| {
            let s = &scene.textures[i];
            let mut rgba = Vec::with_capacity(s.rgba8.len());
            for px in s.rgba8.chunks_exact(4) {
                let k = px[0] as f32 / 255.0;
                let rough = (0.92 - 0.55 * k.sqrt()).clamp(0.3, 1.0);
                rgba.extend([255u8, (rough * 255.0) as u8, 0, 255]);
            }
            scene.textures.push(TextureData { name: format!("{name}_orm"), width: s.width, height: s.height, rgba8: rgba.into(), srgb: false });
            scene.textures.len() - 1
        });
        scene.materials.push(MaterialData {
            name: name.clone(),
            base_color: Vec4::ONE,
            base_color_tex: base,
            metallic: 0.0,
            roughness: if orm.is_some() { 1.0 } else { 0.6 },
            metallic_roughness_tex: orm,
            normal_tex: normal,
            alpha: if opacity { AlphaMode::Mask(110) } else { AlphaMode::Opaque },
            double_sided: opacity,
            ..Default::default()
        });
        material_index.insert(mat.id(), scene.materials.len() - 1);
    }

    // Geometry.
    for geo in objects.all("Geometry").filter(|g| g.subclass() == "Mesh") {
        let Some(&model) = parents.get(&geo.id()).and_then(|ps| ps.iter().find_map(|(p, _)| index.get(p))) else { continue };
        let mats: Vec<usize> = kids.get(&models[model].id()).into_iter().flatten().filter_map(|(c, _)| material_index.get(c).copied()).collect();
        let cps: Vec<Vec3> = geo.array_f64("Vertices").chunks_exact(3).map(|v| Vec3::new(v[0] as f32, v[1] as f32, v[2] as f32) * unit).collect();
        let pvi = geo.array_i32("PolygonVertexIndex");
        // Per polygon-vertex control point and polygon index.
        let mut corner_cp = Vec::with_capacity(pvi.len());
        let mut corner_poly = Vec::with_capacity(pvi.len());
        let mut polys: Vec<(usize, usize)> = Vec::new(); // (first corner, count)
        let mut start = 0;
        for (k, &v) in pvi.iter().enumerate() {
            let cp = if v < 0 { (-v - 1) as usize } else { v as usize };
            corner_cp.push(cp);
            corner_poly.push(polys.len());
            if v < 0 {
                polys.push((start, k + 1 - start));
                start = k + 1;
            }
        }
        let layer = |name: &str| geo.child(name);
        let fetch3 = |el: Option<&Node>, values: &str, idx: &str| -> Option<Vec<Vec3>> {
            let el = el?;
            let raw: Vec<Vec3> = el.array_f64(values).chunks_exact(3).map(|v| Vec3::new(v[0] as f32, v[1] as f32, v[2] as f32)).collect();
            Some(map_layer(el, &raw, idx, &corner_cp, &corner_poly))
        };
        let normals = fetch3(layer("LayerElementNormal"), "Normals", "NormalsIndex");
        let uvs: Option<Vec<Vec2>> = layer("LayerElementUV").map(|el| {
            let raw: Vec<Vec2> = el.array_f64("UV").chunks_exact(2).map(|v| Vec2::new(v[0] as f32, 1.0 - v[1] as f32)).collect();
            map_layer(el, &raw, "UVIndex", &corner_cp, &corner_poly)
        });
        let poly_mat: Vec<usize> = match layer("LayerElementMaterial") {
            Some(el) => {
                let m = el.array_i32("Materials");
                let all_same = el.string("MappingInformationType").as_deref() == Some("AllSame");
                (0..polys.len()).map(|p| if all_same { m.first().copied().unwrap_or(0) } else { m.get(p).copied().unwrap_or(0) } as usize).collect()
            }
            None => vec![0; polys.len()],
        };
        // Skin clusters.
        let mut influences: Vec<Vec<(u16, f32)>> = vec![Vec::new(); cps.len()];
        let mut skin = SkinData { name: format!("{}_skin", scene.nodes[model].name), ..Default::default() };
        for sk in children_of(geo.id(), "Deformer").into_iter().filter(|d| d.subclass() == "Skin") {
            for cl in children_of(sk.id(), "Deformer").into_iter().filter(|d| d.subclass() == "Cluster") {
                let Some(&bone) = kids.get(&cl.id()).and_then(|v| v.iter().find_map(|(c, _)| index.get(c))) else { continue };
                let Some(link) = mat_from(&cl.array_f64("TransformLink")) else { continue };
                let j = skin.joints.len() as u16;
                skin.joints.push(bone);
                // Exporters disagree on what the cluster's `Transform` holds
                // (mesh global vs. already bone-relative); the mesh's own
                // bind-pose global is unambiguous.
                skin.inverse_bind.push(to_m(axis * link, unit).inverse() * to_m(global[model], unit));
                let idx = cl.array_i32("Indexes");
                let w = cl.array_f64("Weights");
                for (i, wt) in idx.iter().zip(w) {
                    if let Some(list) = influences.get_mut(*i as usize) {
                        list.push((j, wt as f32));
                    }
                }
            }
        }
        // Every other bone of the hierarchy becomes a joint too (bind =
        // its bind-pose global), so animation reaches through helpers like
        // the Biped root that carry no weights.
        if !skin.joints.is_empty() {
            let mesh_bind = to_m(global[model], unit);
            let mut in_skin: std::collections::HashSet<usize> = skin.joints.iter().copied().collect();
            for (i, m) in models.iter().enumerate() {
                let bone = matches!(m.subclass(), "LimbNode" | "Root" | "Limb" | "Null") && scene.nodes[i].mesh.is_none() && i != model;
                if bone && in_skin.insert(i) {
                    let g = to_m(global[i], unit);
                    skin.joints.push(i);
                    skin.inverse_bind.push(g.inverse() * mesh_bind);
                }
            }
        }
        let skinned = !skin.joints.is_empty();
        // Primitives per material, vertices deduplicated.
        let nmat = poly_mat.iter().copied().max().unwrap_or(0) + 1;
        let mut prims: Vec<MeshData> = (0..nmat).map(|m| MeshData { material: mats.get(m).copied().unwrap_or(0), ..Default::default() }).collect();
        let mut dedup: Vec<HashMap<(usize, [u32; 5]), u32>> = vec![HashMap::new(); nmat];
        for (p, &(first, count)) in polys.iter().enumerate() {
            let m = poly_mat[p].min(nmat - 1);
            let mut ids = Vec::with_capacity(count);
            for c in first..first + count {
                let cp = corner_cp[c];
                let n = normals.as_ref().map(|v| v[c]).unwrap_or(Vec3::Y);
                let uv = uvs.as_ref().map(|v| v[c]).unwrap_or(Vec2::ZERO);
                let key = (cp, [n.x.to_bits(), n.y.to_bits(), n.z.to_bits(), uv.x.to_bits(), uv.y.to_bits()]);
                let prim = &mut prims[m];
                let id = *dedup[m].entry(key).or_insert_with(|| {
                    prim.positions.push(cps[cp]);
                    prim.normals.push(n.normalize_or(Vec3::Y));
                    prim.uvs.push(uv);
                    if skinned {
                        let mut list = influences[cp].clone();
                        list.sort_by(|a, b| b.1.total_cmp(&a.1));
                        list.truncate(4);
                        let sum: f32 = list.iter().map(|x| x.1).sum::<f32>().max(1e-6);
                        let mut js = [0u16; 4];
                        let mut ws = [0f32; 4];
                        for (k, (j, w)) in list.iter().enumerate() {
                            js[k] = *j;
                            ws[k] = w / sum;
                        }
                        if list.is_empty() {
                            ws[0] = 1.0;
                        }
                        prim.joints.push(js);
                        prim.weights.push(ws);
                    }
                    prim.positions.len() as u32 - 1
                });
                ids.push(id);
            }
            for k in 1..ids.len().saturating_sub(1) {
                prims[m].indices.extend([ids[0], ids[k], ids[k + 1]]);
            }
        }
        let prims: Vec<MeshData> = prims
            .into_iter()
            .filter(|p| !p.indices.is_empty())
            .map(|mut p| {
                p.finalize();
                p
            })
            .collect();
        if prims.is_empty() {
            continue;
        }
        scene.meshes.push(prims);
        scene.nodes[model].mesh = Some(scene.meshes.len() - 1);
        if skinned {
            scene.skins.push(skin);
            scene.nodes[model].skin = Some(scene.skins.len() - 1);
        }
    }
    Ok(scene)
}

/// Expands a layer element to one value per polygon corner.
fn map_layer<T: Copy + Default>(el: &Node, raw: &[T], index_name: &str, corner_cp: &[usize], corner_poly: &[usize]) -> Vec<T> {
    let mapping = el.string("MappingInformationType").unwrap_or_default();
    let reference = el.string("ReferenceInformationType").unwrap_or_default();
    let idx = el.array_i32(index_name);
    let get = |i: usize| -> T {
        let j = if reference == "IndexToDirect" { idx.get(i).copied().unwrap_or(0).max(0) as usize } else { i };
        raw.get(j).copied().unwrap_or_default()
    };
    (0..corner_cp.len())
        .map(|c| match mapping.as_str() {
            "ByVertice" | "ByVertex" | "ByControlPoint" => get(corner_cp[c]),
            "ByPolygon" => get(corner_poly[c]),
            "AllSame" => get(0),
            _ => get(c),
        })
        .collect()
}

/// Finds a texture referenced with an absolute or foreign path: by file
/// name next to the model, in `Textures/` beside or above it.
fn resolve_texture(dir: &Path, file: &str) -> Option<PathBuf> {
    let base = file.rsplit(['/', '\\']).next()?.to_string();
    if base.is_empty() {
        return None;
    }
    let candidates = [dir.join(&base), dir.join("Textures").join(&base), dir.join("textures").join(&base), dir.join("../Textures").join(&base), dir.join("../textures").join(&base)];
    candidates.into_iter().find(|p| p.exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn euler_xyz_applies_x_first() {
        let q = euler([90.0, 0.0, 90.0], 0);
        // X first: +Y → +Z, then Z: +Z stays +Z.
        assert!((q * Vec3::Y - Vec3::Z).length() < 1e-5);
    }

    #[test]
    fn rejects_ascii_fbx() {
        assert!(parse(b"; FBX 7.4.0 project file\n").is_err());
    }
}
