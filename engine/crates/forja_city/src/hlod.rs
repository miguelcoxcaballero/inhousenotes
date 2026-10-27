//! Far-distance stand-ins (HLOD) of a city tile: every building of the
//! tile as one textured box (footprint prism to the eave, plus its ático)
//! in one mesh, one material and one small baked atlas. Each street facade
//! gets an atlas cell painted with the building's wall colour and its real
//! grid of windows (storeys x bays), each roof a cell of its roof colour;
//! at night some windows glow in an emissive copy of the atlas.
//!
//! The renderer draws a tile's HLOD instead of its detail (blocks, window
//! modules, shops, its shadow masses) once the camera is far from the
//! tile (`GraphicsConfig::hlod_distance`): one draw and a few hundred
//! triangles instead of thousands of draws and tens of thousands of
//! triangles, in the camera and in every shadow cascade.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use forja_assets::{MaterialData, MeshData, NodeData, SceneData, TextureData};
use forja_math::{Vec2, Vec3};

use crate::building::ShadowMass;
use crate::kit::{Assembler, Kit, Mat};
use crate::lot::EdgeKind;

const ATLAS: u32 = 512;
const CELL: u32 = 32;
const PER_ROW: u32 = ATLAS / CELL;

/// A tile's detail and its HLOD, as indices into `CityOut::placements`.
#[derive(Clone, Debug)]
pub struct HlodTile {
    /// Placements of the tile's detail (blocks, modules, shadow masses).
    pub detail: std::ops::Range<usize>,
    /// The placement of the tile's HLOD scene.
    pub hlod: usize,
}

/// Mean linear colour of a kit material (texture mean x tint), cached.
fn mat_color(kit: &Kit, m: Mat) -> [f32; 3] {
    static CACHE: OnceLock<Mutex<HashMap<(usize, Mat), [f32; 3]>>> = OnceLock::new();
    let key = (kit as *const Kit as usize, m);
    if let Some(c) = CACHE.get_or_init(Default::default).lock().ok().and_then(|c| c.get(&key).copied()) {
        return c;
    }
    let mut a = Assembler::new(kit, "probe");
    let i = a.material(m);
    let md = &a.scene.materials[i];
    let mut c = [md.base_color.x, md.base_color.y, md.base_color.z];
    if let Some(t) = md.base_color_tex.and_then(|t| a.scene.textures.get(t)) {
        let px = t.rgba8.len() / 4;
        let step = (px / 4096).max(1);
        let (mut acc, mut n) = ([0.0f32; 3], 0.0f32);
        for p in (0..px).step_by(step) {
            for (k, v) in acc.iter_mut().enumerate() {
                let s = t.rgba8[p * 4 + k] as f32 / 255.0;
                *v += if t.srgb { srgb_to_linear(s) } else { s };
            }
            n += 1.0;
        }
        for k in 0..3 {
            c[k] *= acc[k] / n.max(1.0);
        }
    }
    if let Ok(mut cache) = CACHE.get_or_init(Default::default).lock() {
        cache.insert(key, c);
    }
    c
}

/// Stable 0..1 hash.
fn hash(mut x: u64) -> f32 {
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    (x >> 40) as f32 / (1u64 << 24) as f32
}

fn srgb_to_linear(s: f32) -> f32 {
    if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
}

fn linear_to_srgb8(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let s = if v <= 0.0031308 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0 + 0.5) as u8
}

struct Atlas {
    color: Vec<u8>,
    glow: Vec<u8>,
    used: u32,
    plain: HashMap<[u8; 3], u32>,
}

impl Atlas {
    fn new() -> Self {
        Atlas { color: vec![0; (ATLAS * ATLAS * 4) as usize], glow: vec![0; (ATLAS * ATLAS * 4) as usize], used: 0, plain: HashMap::new() }
    }

    fn origin(cell: u32) -> (u32, u32) {
        ((cell % PER_ROW) * CELL, (cell / PER_ROW) * CELL)
    }

    /// UV rectangle of a cell, inset half a texel: `(u0, v0, u1, v1)`.
    fn uv(cell: u32) -> (f32, f32, f32, f32) {
        let (x, y) = Self::origin(cell);
        let a = ATLAS as f32;
        ((x as f32 + 0.5) / a, (y as f32 + 0.5) / a, ((x + CELL) as f32 - 0.5) / a, ((y + CELL) as f32 - 0.5) / a)
    }

    fn put(buf: &mut [u8], x: u32, y: u32, c: [u8; 4]) {
        let i = ((y * ATLAS + x) * 4) as usize;
        buf[i..i + 4].copy_from_slice(&c);
    }

    fn alloc(&mut self) -> Option<u32> {
        (self.used < PER_ROW * PER_ROW).then(|| {
            self.used += 1;
            self.used - 1
        })
    }

    /// A cell of one colour (walls without windows, roofs), shared.
    fn plain(&mut self, c: [u8; 3]) -> u32 {
        if let Some(&i) = self.plain.get(&c) {
            return i;
        }
        let i = self.alloc().unwrap_or(0);
        let (x0, y0) = Self::origin(i);
        for y in y0..y0 + CELL {
            for x in x0..x0 + CELL {
                Self::put(&mut self.color, x, y, [c[0], c[1], c[2], 255]);
            }
        }
        self.plain.insert(c, i);
        i
    }

    /// A facade cell: wall colour, `rows` storeys (the first `ground`
    /// fraction of the height is the ground floor) of `cols` windows;
    /// `lit(row, col)` windows glow at night.
    fn facade(&mut self, wall: [u8; 3], rows: u32, cols: u32, ground: f32, lit: &dyn Fn(u32, u32) -> bool) -> u32 {
        let Some(i) = self.alloc() else { return self.plain(wall) };
        let (x0, y0) = Self::origin(i);
        let glass = [38u8, 42, 50, 255];
        let shop = [30u8, 30, 34, 255];
        let warm = [255u8, 196, 120, 255];
        // Facade rows from the top of the cell (v = 0 at the top).
        let gh = (CELL as f32 * ground).round().clamp(4.0, CELL as f32 - 4.0) as u32;
        let upper = CELL - gh;
        let floors = rows.saturating_sub(1).max(1);
        for y in 0..CELL {
            for x in 0..CELL {
                let mut c = [wall[0], wall[1], wall[2], 255];
                let mut g = [0u8, 0, 0, 255];
                let bay = CELL as f32 / cols as f32;
                let bx = x as f32 % bay / bay;
                let col = (x as f32 / bay) as u32;
                if y < upper {
                    let fl = upper as f32 / floors as f32;
                    let fy = y as f32 % fl / fl;
                    let row = (y as f32 / fl) as u32;
                    if (0.3..0.7).contains(&bx) && (0.25..0.8).contains(&fy) {
                        c = glass;
                        if lit(row, col) {
                            g = warm;
                        }
                    }
                } else {
                    let fy = (y - upper) as f32 / gh as f32;
                    if (0.15..0.85).contains(&bx) && fy > 0.2 {
                        c = shop;
                    }
                }
                Self::put(&mut self.color, x0 + x, y0 + y, c);
                Self::put(&mut self.glow, x0 + x, y0 + y, g);
            }
        }
        i
    }
}

/// The HLOD scene of the buildings in `masses` (`None` without any).
pub fn tile_scene(kit: &Kit, masses: &[ShadowMass]) -> Option<SceneData> {
    let masses: Vec<&ShadowMass> = masses.iter().filter(|m| m.facade.is_some()).collect();
    if masses.is_empty() {
        return None;
    }
    let night = kit.night;
    let mut atlas = Atlas::new();
    let mut mesh = MeshData::default();
    let quad = |mesh: &mut MeshData, p: [Vec3; 4], uv: [Vec2; 4]| {
        let n = (p[1] - p[0]).cross(p[3] - p[0]).normalize_or(Vec3::Y);
        let b = mesh.positions.len() as u32;
        for k in 0..4 {
            mesh.positions.push(p[k]);
            mesh.normals.push(n);
            mesh.uvs.push(uv[k]);
        }
        mesh.indices.extend([b, b + 1, b + 2, b, b + 2, b + 3]);
    };
    for (mi, m) in masses.iter().enumerate() {
        let f = m.facade.as_ref().expect("filtered");
        let wall = mat_color(kit, f.wall).map(linear_to_srgb8);
        let roof = mat_color(kit, f.roof).map(linear_to_srgb8);
        let n = m.ring.len();
        // The ring runs clockwise seen from above for lots; pick the
        // outward normal from the signed area.
        let area: f32 = (0..n).map(|i| m.ring[i].perp_dot(m.ring[(i + 1) % n])).sum();
        let bottom = f.base - 0.3;
        let height = (m.top - f.base).max(1.0);
        for i in 0..n {
            let (a, b) = (m.ring[i], m.ring[(i + 1) % n]);
            let len = a.distance(b);
            if len < 0.05 {
                continue;
            }
            let party = f.edges.get(i).is_some_and(|e| *e == EdgeKind::Party);
            let cell = if party || len < 2.0 {
                atlas.plain(wall)
            } else {
                let cols = ((len / 3.2).round() as u32).clamp(1, 8);
                let rows = f.storeys.clamp(1, 9);
                let ground = (f.ground_height / height).clamp(0.12, 0.5);
                let seed = f.seed ^ (i as u64) << 20 ^ mi as u64;
                let p = night.lit_flats() * night.dark.sqrt();
                atlas.facade(wall, rows, cols, ground, &|r, c| night.is_night() && hash(seed ^ ((r * 16 + c) as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)) < p)
            };
            let (u0, v0, u1, v1) = Atlas::uv(cell);
            let (pa, pb) = (Vec3::new(a.x, bottom, a.y), Vec3::new(b.x, bottom, b.y));
            let (ta, tb) = (Vec3::new(a.x, m.top, a.y), Vec3::new(b.x, m.top, b.y));
            // Facing out of the lot.
            let out = if area < 0.0 { Vec3::new(b.y - a.y, 0.0, a.x - b.x) } else { Vec3::new(a.y - b.y, 0.0, b.x - a.x) };
            let p = [pa, pb, tb, ta];
            let nrm = (p[1] - p[0]).cross(p[3] - p[0]);
            if nrm.dot(out) >= 0.0 {
                quad(&mut mesh, p, [Vec2::new(u0, v1), Vec2::new(u1, v1), Vec2::new(u1, v0), Vec2::new(u0, v0)]);
            } else {
                quad(&mut mesh, [pb, pa, ta, tb], [Vec2::new(u1, v1), Vec2::new(u0, v1), Vec2::new(u0, v0), Vec2::new(u1, v0)]);
            }
        }
        // Roof: the footprint at the eave in the roof colour.
        if let Some(tris) = crate::geom::triangulate(&m.ring) {
            let rc = atlas.plain(roof);
            let (u0, v0, u1, v1) = Atlas::uv(rc);
            let uv = Vec2::new((u0 + u1) * 0.5, (v0 + v1) * 0.5);
            let b = mesh.positions.len() as u32;
            for p in &m.ring {
                mesh.positions.push(Vec3::new(p.x, m.top + f.roof_rise, p.y));
                mesh.normals.push(Vec3::Y);
                mesh.uvs.push(uv);
            }
            for t in tris {
                let (pa, pb, pc) = (m.ring[t[0] as usize], m.ring[t[1] as usize], m.ring[t[2] as usize]);
                if (pb - pa).perp_dot(pc - pa) < 0.0 {
                    mesh.indices.extend([b + t[0], b + t[1], b + t[2]]);
                } else {
                    mesh.indices.extend([b + t[0], b + t[2], b + t[1]]);
                }
            }
        }
    }
    if mesh.indices.is_empty() {
        return None;
    }
    mesh.finalize();
    let mut textures = vec![TextureData { name: "hlod_atlas".into(), width: ATLAS, height: ATLAS, rgba8: Arc::from(atlas.color), srgb: true }];
    let glow = night.is_night().then(|| {
        textures.push(TextureData { name: "hlod_glow".into(), width: ATLAS, height: ATLAS, rgba8: Arc::from(atlas.glow), srgb: true });
        1
    });
    let material = MaterialData {
        name: "hlod".into(),
        base_color_tex: Some(0),
        roughness: 0.9,
        specular: 0.7,
        emissive: if glow.is_some() { Vec3::splat(0.35 * night.dark.max(0.05)) } else { Vec3::ZERO },
        emissive_tex: glow,
        ..Default::default()
    };
    Some(SceneData {
        name: "hlod".into(),
        nodes: vec![NodeData { name: "hlod".into(), mesh: Some(0), ..Default::default() }],
        roots: vec![0],
        meshes: vec![vec![mesh]],
        materials: vec![material],
        textures,
        ..Default::default()
    })
}
