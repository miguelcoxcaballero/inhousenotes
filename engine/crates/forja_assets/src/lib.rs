//! # forja_assets
//!
//! Loads glTF 2.0 (`.gltf`/`.glb`) and Wavefront OBJ into plain CPU-side
//! data ([`SceneData`]) that the renderer, the animation system and the
//! world importers consume. Nothing here touches the GPU, so it is fully
//! testable and can run on background threads for streaming.

pub mod bvh;
pub mod cache;
pub mod character;
pub mod fbx;
pub mod lod;
pub mod makehuman;
pub mod manifest;
mod obj;
pub mod pbr;
pub mod progress;

use std::path::Path;

use forja_math::{Aabb, Mat4, Quat, Vec2, Vec3, Vec4};

pub use obj::load_obj;
pub use pbr::{HdrImage, PbrSet, load_hdr, load_pbr_set};

#[derive(Debug)]
pub enum AssetError {
    Io(String),
    Gltf(String),
    Format(String),
}

impl std::fmt::Display for AssetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AssetError::Io(e) => write!(f, "io: {e}"),
            AssetError::Gltf(e) => write!(f, "gltf: {e}"),
            AssetError::Format(e) => write!(f, "format: {e}"),
        }
    }
}

impl std::error::Error for AssetError {}

/// One drawable primitive with a single material.
#[derive(Clone, Debug, Default)]
pub struct MeshData {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub uvs: Vec<Vec2>,
    /// xyz tangent, w handedness. Generated when missing.
    pub tangents: Vec<Vec4>,
    pub indices: Vec<u32>,
    pub joints: Vec<[u16; 4]>,
    pub weights: Vec<[f32; 4]>,
    pub material: usize,
    pub aabb: Aabb,
    /// Simplified index lists (LOD 1, 2, ...) over the same vertices.
    pub lods: Vec<Vec<u32>>,
    /// Switch LODs together with the other `lod_sync` primitives of the same
    /// mesh (a tree's trunk and crown): the renderer picks one level for all
    /// of them from their summed triangle counts and joint bounds, so a
    /// mid-detail trunk never stands under a billboard crown.
    pub lod_sync: bool,
    /// Which shadow cascades this primitive casts into (see [`ShadowRole`]).
    pub shadow: ShadowRole,
}

/// How a primitive takes part in the sun's shadow cascades.
///
/// Facades are assembled from thousands of small modules (windows,
/// shutters, balconies) over walls with real openings; far away they cast
/// the same shadow as a closed box. A city can mark that detail
/// [`ShadowRole::Near`] and add a [`ShadowRole::Proxy`] stand-in (the
/// building's mass): the renderer draws the detail in the camera and the
/// near cascades, and the proxy only in the far cascades.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ShadowRole {
    /// Camera and every shadow view (the default).
    #[default]
    All,
    /// Camera, near cascades and spot shadows; replaced by a proxy in the
    /// far cascades.
    Near,
    /// Far cascades only: never drawn in the camera.
    Proxy,
    /// Thin detail (iron grilles, railings): camera, the first cascade and
    /// spot shadows only. Bars a few centimetres wide are thinner than a
    /// texel of the other cascades, where they only alias (flicker) as
    /// the camera moves.
    Fine,
}

impl MeshData {
    pub fn is_skinned(&self) -> bool {
        !self.joints.is_empty()
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Fills missing normals, UVs and tangents so every mesh has a uniform layout.
    pub fn finalize(&mut self) {
        let n = self.positions.len();
        if self.indices.is_empty() {
            self.indices = (0..n as u32).collect();
        }
        if self.normals.len() != n {
            self.normals = vec![Vec3::ZERO; n];
            for t in self.indices.chunks_exact(3) {
                let (a, b, c) = (self.positions[t[0] as usize], self.positions[t[1] as usize], self.positions[t[2] as usize]);
                let fnrm = (b - a).cross(c - a);
                for i in t {
                    self.normals[*i as usize] += fnrm;
                }
            }
            for v in &mut self.normals {
                *v = v.normalize_or(Vec3::Y);
            }
        }
        if self.uvs.len() != n {
            self.uvs = vec![Vec2::ZERO; n];
        }
        if self.tangents.len() != n {
            self.tangents = generate_tangents(&self.positions, &self.normals, &self.uvs, &self.indices);
        }
        self.aabb = Aabb::from_points(self.positions.iter().copied());
    }
}

/// Per-vertex tangents from UV gradients (Lengyel's method), Gram-Schmidt
/// orthogonalised, with handedness in `w`.
pub fn generate_tangents(pos: &[Vec3], nrm: &[Vec3], uv: &[Vec2], idx: &[u32]) -> Vec<Vec4> {
    let n = pos.len();
    let mut tan = vec![Vec3::ZERO; n];
    let mut bit = vec![Vec3::ZERO; n];
    for t in idx.chunks_exact(3) {
        let (i0, i1, i2) = (t[0] as usize, t[1] as usize, t[2] as usize);
        let e1 = pos[i1] - pos[i0];
        let e2 = pos[i2] - pos[i0];
        let d1 = uv[i1] - uv[i0];
        let d2 = uv[i2] - uv[i0];
        let det = d1.x * d2.y - d2.x * d1.y;
        if det.abs() < 1e-12 {
            continue;
        }
        let r = 1.0 / det;
        let sdir = (e1 * d2.y - e2 * d1.y) * r;
        let tdir = (e2 * d1.x - e1 * d2.x) * r;
        for i in [i0, i1, i2] {
            tan[i] += sdir;
            bit[i] += tdir;
        }
    }
    (0..n)
        .map(|i| {
            let nv = nrm[i];
            let mut t = tan[i] - nv * nv.dot(tan[i]);
            if t.length_squared() < 1e-12 {
                t = forja_math::orthonormal_basis(nv).0;
            }
            let t = t.normalize();
            let w = if nv.cross(t).dot(bit[i]) < 0.0 { -1.0 } else { 1.0 };
            t.extend(w)
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlphaMode {
    Opaque,
    Mask(u8),
    Blend,
}

#[derive(Clone, Debug)]
pub struct MaterialData {
    pub name: String,
    pub base_color: Vec4,
    pub base_color_tex: Option<usize>,
    pub metallic: f32,
    pub roughness: f32,
    /// glTF layout: G = roughness, B = metallic.
    pub metallic_roughness_tex: Option<usize>,
    pub normal_tex: Option<usize>,
    pub normal_scale: f32,
    pub occlusion_tex: Option<usize>,
    pub emissive: Vec3,
    pub emissive_tex: Option<usize>,
    pub alpha: AlphaMode,
    pub double_sided: bool,
    /// KHR_materials_unlit: output base color without lighting.
    pub unlit: bool,
    /// KHR_materials_transmission (rendered as tinted glass).
    pub transmission: f32,
    /// KHR_materials_clearcoat layer strength and roughness (car paint).
    pub clearcoat: f32,
    pub clearcoat_roughness: f32,
    /// Skin-like subsurface scattering amount (0..1).
    pub subsurface: f32,
    /// Leaf / frond / impostor shading (see [`FoliageShading`]).
    pub foliage: Option<FoliageShading>,
    /// KHR_materials_specular factor: scales the Fresnel reflectance of
    /// dielectrics (1 = F0 0.04). Porous masonry, unglazed clay and lime
    /// render reflect less than their solid mineral (~0.65-0.8).
    pub specular: f32,
}

/// Foliage shading for alpha-tested cards: normals are not flipped on back
/// faces (cards carry bent crown normals), diffuse wraps around and light
/// passing through the leaves (`translucency`) brightens the shadowed side.
/// Back faces whose `v` texture coordinate is above `backface_cull_v` are
/// discarded, so one-sided impostor quads can share a double-sided
/// material with the leaf cards.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FoliageShading {
    pub translucency: f32,
    pub backface_cull_v: f32,
}

impl Default for FoliageShading {
    fn default() -> Self {
        Self { translucency: 0.5, backface_cull_v: 2.0 }
    }
}

impl Default for MaterialData {
    fn default() -> Self {
        Self {
            name: "default".into(),
            base_color: Vec4::ONE,
            base_color_tex: None,
            metallic: 0.0,
            roughness: 0.6,
            metallic_roughness_tex: None,
            normal_tex: None,
            normal_scale: 1.0,
            occlusion_tex: None,
            emissive: Vec3::ZERO,
            emissive_tex: None,
            alpha: AlphaMode::Opaque,
            double_sided: false,
            unlit: false,
            transmission: 0.0,
            clearcoat: 0.0,
            clearcoat_roughness: 0.03,
            subsurface: 0.0,
            foliage: None,
            specular: 1.0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct TextureData {
    pub name: String,
    pub width: u32,
    pub height: u32,
    /// Shared so tinted material variants and instanced kits reuse one copy
    /// (the renderer also uploads each shared buffer once).
    pub rgba8: std::sync::Arc<[u8]>,
    /// Color data (base color, emissive) is sRGB; data maps are linear.
    pub srgb: bool,
}

/// Translation, rotation, scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trs {
    pub t: Vec3,
    pub r: Quat,
    pub s: Vec3,
}

impl Default for Trs {
    fn default() -> Self {
        Self { t: Vec3::ZERO, r: Quat::IDENTITY, s: Vec3::ONE }
    }
}

impl Trs {
    pub fn to_mat4(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.s, self.r, self.t)
    }

    pub fn lerp(&self, o: &Trs, t: f32) -> Trs {
        Trs { t: self.t.lerp(o.t, t), r: nlerp(self.r, o.r, t), s: self.s.lerp(o.s, t) }
    }
}

/// Normalised lerp on the shortest arc; fast and good enough for blending.
pub fn nlerp(a: Quat, b: Quat, t: f32) -> Quat {
    let b = if a.dot(b) < 0.0 { -b } else { b };
    Quat::from_vec4(Vec4::from(a).lerp(Vec4::from(b), t)).normalize()
}

#[derive(Clone, Debug, Default)]
pub struct NodeData {
    pub name: String,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub local: Trs,
    /// Primitives drawn by this node (LOD 0).
    pub mesh: Option<usize>,
    pub skin: Option<usize>,
}

#[derive(Clone, Debug, Default)]
pub struct SkinData {
    pub name: String,
    /// Node index of each joint.
    pub joints: Vec<usize>,
    pub inverse_bind: Vec<Mat4>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interpolation {
    Step,
    Linear,
    CubicSpline,
}

#[derive(Clone, Debug)]
pub enum ChannelValues {
    Translation(Vec<Vec3>),
    Rotation(Vec<Quat>),
    Scale(Vec<Vec3>),
}

#[derive(Clone, Debug)]
pub struct ChannelData {
    pub node: usize,
    pub interpolation: Interpolation,
    pub times: Vec<f32>,
    /// For cubic spline: `[in_tangent, value, out_tangent]` per key.
    pub values: ChannelValues,
}

#[derive(Clone, Debug)]
pub struct ClipData {
    pub name: String,
    pub duration: f32,
    pub channels: Vec<ChannelData>,
}

/// Everything loaded from one file.
#[derive(Clone, Debug, Default)]
pub struct SceneData {
    pub name: String,
    pub nodes: Vec<NodeData>,
    pub roots: Vec<usize>,
    /// `meshes[i]` = primitives of glTF mesh `i`.
    pub meshes: Vec<Vec<MeshData>>,
    pub materials: Vec<MaterialData>,
    pub textures: Vec<TextureData>,
    pub skins: Vec<SkinData>,
    pub clips: Vec<ClipData>,
}

impl SceneData {
    /// World matrix of every node.
    pub fn world_matrices(&self) -> Vec<Mat4> {
        let mut out = vec![Mat4::IDENTITY; self.nodes.len()];
        let mut stack: Vec<(usize, Mat4)> = self.roots.iter().map(|r| (*r, Mat4::IDENTITY)).collect();
        while let Some((i, parent)) = stack.pop() {
            let m = parent * self.nodes[i].local.to_mat4();
            out[i] = m;
            for c in &self.nodes[i].children {
                stack.push((*c, m));
            }
        }
        out
    }

    pub fn find_node(&self, name: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.name == name)
    }

    /// Bounds of all meshes in scene space.
    pub fn bounds(&self) -> Aabb {
        let world = self.world_matrices();
        let mut b = Aabb::EMPTY;
        for (i, n) in self.nodes.iter().enumerate() {
            if let Some(m) = n.mesh {
                for p in &self.meshes[m] {
                    if p.aabb.is_empty() {
                        continue;
                    }
                    let c = world[i].transform_point3(p.aabb.center());
                    let h = p.aabb.half_extents();
                    let m3 = forja_math::Mat3::from_mat4(world[i]);
                    let e = m3.x_axis.abs() * h.x + m3.y_axis.abs() * h.y + m3.z_axis.abs() * h.z;
                    b = b.union(&Aabb::from_center_half(c, e));
                }
            }
        }
        b
    }

    /// All triangles in scene space (for static collision meshes).
    pub fn world_triangles(&self) -> (Vec<Vec3>, Vec<[u32; 3]>) {
        let world = self.world_matrices();
        let (mut verts, mut tris) = (Vec::new(), Vec::new());
        for (i, n) in self.nodes.iter().enumerate() {
            let Some(m) = n.mesh else { continue };
            for p in &self.meshes[m] {
                let base = verts.len() as u32;
                verts.extend(p.positions.iter().map(|v| world[i].transform_point3(*v)));
                tris.extend(p.indices.chunks_exact(3).map(|t| [base + t[0], base + t[1], base + t[2]]));
            }
        }
        (verts, tris)
    }

    pub fn triangle_count(&self) -> usize {
        self.meshes.iter().flatten().map(|m| m.triangle_count()).sum()
    }
}

/// Loads a glTF or GLB file with its buffers and images.
pub fn load_gltf(path: &Path) -> Result<SceneData, AssetError> {
    let (doc, buffers, images) = gltf::import(path).map_err(|e| AssetError::Gltf(format!("{}: {e}", path.display())))?;
    let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut scene = SceneData { name, ..Default::default() };
    let get_buf = |b: gltf::Buffer| Some(&buffers[b.index()].0[..]);

    // Textures: gltf "textures" reference images; we store per texture.
    let mut srgb = vec![false; doc.textures().len()];
    for m in doc.materials() {
        if let Some(t) = m.pbr_metallic_roughness().base_color_texture() {
            srgb[t.texture().index()] = true;
        }
        if let Some(t) = m.emissive_texture() {
            srgb[t.texture().index()] = true;
        }
    }
    for t in doc.textures() {
        let img = &images[t.source().index()];
        scene.textures.push(TextureData {
            name: t.name().unwrap_or("").to_string(),
            width: img.width,
            height: img.height,
            rgba8: to_rgba8(img).into(),
            srgb: srgb[t.index()],
        });
    }
    for m in doc.materials() {
        let pbr = m.pbr_metallic_roughness();
        let emissive_strength = m.emissive_strength().unwrap_or(1.0);
        scene.materials.push(MaterialData {
            name: m.name().unwrap_or("material").to_string(),
            base_color: Vec4::from(pbr.base_color_factor()),
            base_color_tex: pbr.base_color_texture().map(|t| t.texture().index()),
            metallic: pbr.metallic_factor(),
            roughness: pbr.roughness_factor(),
            metallic_roughness_tex: pbr.metallic_roughness_texture().map(|t| t.texture().index()),
            normal_tex: m.normal_texture().map(|t| t.texture().index()),
            normal_scale: m.normal_texture().map(|t| t.scale()).unwrap_or(1.0),
            occlusion_tex: m.occlusion_texture().map(|t| t.texture().index()),
            emissive: Vec3::from(m.emissive_factor()) * emissive_strength,
            emissive_tex: m.emissive_texture().map(|t| t.texture().index()),
            alpha: match m.alpha_mode() {
                gltf::material::AlphaMode::Opaque => AlphaMode::Opaque,
                gltf::material::AlphaMode::Mask => AlphaMode::Mask((m.alpha_cutoff().unwrap_or(0.5) * 255.0) as u8),
                gltf::material::AlphaMode::Blend => AlphaMode::Blend,
            },
            double_sided: m.double_sided(),
            unlit: m.unlit(),
            transmission: m.transmission().map(|t| t.transmission_factor()).unwrap_or(0.0),
            clearcoat: clearcoat(&doc, m.index()).0,
            clearcoat_roughness: clearcoat(&doc, m.index()).1,
            subsurface: 0.0,
            foliage: None,
            specular: m.specular().map(|s| s.specular_factor()).unwrap_or(1.0),
        });
    }
    let default_material = scene.materials.len();
    let mut needs_default = false;
    for mesh in doc.meshes() {
        let mut prims = Vec::new();
        for p in mesh.primitives() {
            if p.mode() != gltf::mesh::Mode::Triangles {
                continue;
            }
            let r = p.reader(get_buf);
            let mut md = MeshData {
                positions: r.read_positions().map(|i| i.map(Vec3::from).collect()).unwrap_or_default(),
                normals: r.read_normals().map(|i| i.map(Vec3::from).collect()).unwrap_or_default(),
                uvs: r.read_tex_coords(0).map(|i| i.into_f32().map(Vec2::from).collect()).unwrap_or_default(),
                tangents: r.read_tangents().map(|i| i.map(Vec4::from).collect()).unwrap_or_default(),
                indices: r.read_indices().map(|i| i.into_u32().collect()).unwrap_or_default(),
                joints: r.read_joints(0).map(|i| i.into_u16().collect()).unwrap_or_default(),
                weights: r.read_weights(0).map(|i| i.into_f32().collect()).unwrap_or_default(),
                material: p.material().index().unwrap_or_else(|| {
                    needs_default = true;
                    default_material
                }),
                aabb: Aabb::EMPTY,
                lods: Vec::new(),
                lod_sync: false,
                shadow: ShadowRole::All,
            };
            md.finalize();
            prims.push(md);
        }
        scene.meshes.push(prims);
    }
    if needs_default {
        scene.materials.push(MaterialData::default());
    }
    for n in doc.nodes() {
        let (t, r, s) = n.transform().decomposed();
        scene.nodes.push(NodeData {
            name: n.name().unwrap_or("").to_string(),
            parent: None,
            children: n.children().map(|c| c.index()).collect(),
            local: Trs { t: Vec3::from(t), r: Quat::from_array(r).normalize(), s: Vec3::from(s) },
            mesh: n.mesh().map(|m| m.index()),
            skin: n.skin().map(|s| s.index()),
        });
    }
    for i in 0..scene.nodes.len() {
        for c in scene.nodes[i].children.clone() {
            scene.nodes[c].parent = Some(i);
        }
    }
    scene.roots = match doc.default_scene().or_else(|| doc.scenes().next()) {
        Some(s) => s.nodes().map(|n| n.index()).collect(),
        None => (0..scene.nodes.len()).filter(|i| scene.nodes[*i].parent.is_none()).collect(),
    };
    for skin in doc.skins() {
        let r = skin.reader(get_buf);
        let joints: Vec<usize> = skin.joints().map(|j| j.index()).collect();
        let inverse_bind = r
            .read_inverse_bind_matrices()
            .map(|i| i.map(|m| Mat4::from_cols_array_2d(&m)).collect())
            .unwrap_or_else(|| vec![Mat4::IDENTITY; joints.len()]);
        scene.skins.push(SkinData { name: skin.name().unwrap_or("").to_string(), joints, inverse_bind });
    }
    for anim in doc.animations() {
        let mut channels = Vec::new();
        let mut duration: f32 = 0.0;
        for ch in anim.channels() {
            let r = ch.reader(get_buf);
            let Some(times) = r.read_inputs().map(|i| i.collect::<Vec<f32>>()) else { continue };
            let Some(outputs) = r.read_outputs() else { continue };
            let values = match outputs {
                gltf::animation::util::ReadOutputs::Translations(v) => ChannelValues::Translation(v.map(Vec3::from).collect()),
                gltf::animation::util::ReadOutputs::Rotations(v) => {
                    ChannelValues::Rotation(v.into_f32().map(|q| Quat::from_array(q).normalize()).collect())
                }
                gltf::animation::util::ReadOutputs::Scales(v) => ChannelValues::Scale(v.map(Vec3::from).collect()),
                gltf::animation::util::ReadOutputs::MorphTargetWeights(_) => continue,
            };
            duration = duration.max(times.last().copied().unwrap_or(0.0));
            channels.push(ChannelData {
                node: ch.target().node().index(),
                interpolation: match ch.sampler().interpolation() {
                    gltf::animation::Interpolation::Step => Interpolation::Step,
                    gltf::animation::Interpolation::Linear => Interpolation::Linear,
                    gltf::animation::Interpolation::CubicSpline => Interpolation::CubicSpline,
                },
                times,
                values,
            });
        }
        scene.clips.push(ClipData { name: anim.name().unwrap_or("clip").to_string(), duration, channels });
    }
    Ok(scene)
}

/// Reads KHR_materials_clearcoat from the raw JSON (the gltf crate does not
/// expose it): `(factor, roughness)`.
fn clearcoat(doc: &gltf::Document, index: Option<usize>) -> (f32, f32) {
    let Some(i) = index else { return (0.0, 0.03) };
    let json = doc.as_json();
    let Some(m) = json.materials.get(i) else { return (0.0, 0.03) };
    let Some(ext) = m.extensions.as_ref().and_then(|e| e.others.get("KHR_materials_clearcoat")) else { return (0.0, 0.03) };
    let f = ext.get("clearcoatFactor").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
    let r = ext.get("clearcoatRoughnessFactor").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
    (f, r.max(0.02))
}

fn to_rgba8(img: &gltf::image::Data) -> Vec<u8> {
    use gltf::image::Format;
    let p = &img.pixels;
    match img.format {
        Format::R8G8B8A8 => p.clone(),
        Format::R8G8B8 => p.chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        Format::R8G8 => p.chunks_exact(2).flat_map(|c| [c[0], c[1], 0, 255]).collect(),
        Format::R8 => p.iter().flat_map(|c| [*c, *c, *c, 255]).collect(),
        Format::R16G16B16A16 => p.chunks_exact(2).map(|c| c[1]).collect(),
        Format::R16G16B16 => p.chunks_exact(6).flat_map(|c| [c[1], c[3], c[5], 255]).collect(),
        Format::R16G16 => p.chunks_exact(4).flat_map(|c| [c[1], c[3], 0, 255]).collect(),
        Format::R16 => p.chunks_exact(2).flat_map(|c| [c[1], c[1], c[1], 255]).collect(),
        Format::R32G32B32FLOAT | Format::R32G32B32A32FLOAT => {
            let ch = if img.format == Format::R32G32B32FLOAT { 3 } else { 4 };
            let f: Vec<f32> = p.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
            f.chunks_exact(ch)
                .flat_map(|c| {
                    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                    [q(c[0]), q(c[1]), q(c[2]), if ch == 4 { q(c[3]) } else { 255 }]
                })
                .collect()
        }
    }
}

/// Loads any supported model by extension.
pub fn load_model(path: &Path) -> Result<SceneData, AssetError> {
    match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("gltf" | "glb") => load_gltf(path),
        Some("obj") => load_obj(path),
        Some("fbx") => fbx::load_fbx(path),
        other => Err(AssetError::Format(format!("unsupported model extension {other:?}"))),
    }
}

/// Decodes an image file into RGBA8.
pub fn load_texture(path: &Path, srgb: bool) -> Result<TextureData, AssetError> {
    let img = image::open(path).map_err(|e| AssetError::Io(format!("{}: {e}", path.display())))?.to_rgba8();
    Ok(TextureData {
        name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        width: img.width(),
        height: img.height(),
        rgba8: img.into_raw().into(),
        srgb,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tangents_follow_uv_u_direction() {
        let pos = vec![Vec3::ZERO, Vec3::X, Vec3::Z];
        let nrm = vec![Vec3::Y; 3];
        let uv = vec![Vec2::ZERO, Vec2::new(1.0, 0.0), Vec2::new(0.0, 1.0)];
        let t = generate_tangents(&pos, &nrm, &uv, &[0, 1, 2]);
        assert!((t[0].truncate() - Vec3::X).length() < 1e-5);
    }

    #[test]
    fn finalize_fills_missing_attributes() {
        let mut m = MeshData { positions: vec![Vec3::ZERO, Vec3::X, Vec3::Z], indices: vec![0, 2, 1], ..Default::default() };
        m.finalize();
        assert_eq!(m.normals.len(), 3);
        assert!(m.normals[0].y > 0.99);
        assert_eq!(m.tangents.len(), 3);
    }
}
