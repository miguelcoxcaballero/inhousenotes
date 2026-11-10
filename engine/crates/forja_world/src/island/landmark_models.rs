//! The landmarks' models for the island: built from their specs once,
//! prepared for an open world (smaller textures, coarser distance LODs, a
//! far stand-in, a welded collision mesh) and kept in the asset cache, an
//! entry per landmark and one per texture (landmarks share many, and a
//! changed landmark is the only one built again). This file is part of
//! the cache key: editing it rebuilds every landmark.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use forja_assets::cache::{Key, Reader, Writer};
use forja_assets::{SceneData, TextureData};
use forja_math::Vec3;

/// A landmark ready for the world.
pub struct Model {
    /// Its scene, with distance LODs down to a few hundred triangles.
    pub scene: SceneData,
    /// Collision mesh in the landmark's frame.
    pub col_verts: Vec<Vec3>,
    pub col_tris: Vec<[u32; 3]>,
    /// Landmark-local bounds.
    pub bounds: (Vec3, Vec3),
}

/// Welds a triangle soup (the framework's collider has three vertices per
/// triangle) and drops degenerate triangles.
fn weld(verts: &[Vec3], tris: &[[u32; 3]]) -> (Vec<Vec3>, Vec<[u32; 3]>) {
    let mut map: std::collections::HashMap<[i32; 3], u32> = std::collections::HashMap::with_capacity(verts.len() / 3);
    let mut out = Vec::new();
    let remap: Vec<u32> = verts
        .iter()
        .map(|v| {
            let key = [(v.x * 500.0).round() as i32, (v.y * 500.0).round() as i32, (v.z * 500.0).round() as i32];
            *map.entry(key).or_insert_with(|| {
                out.push(*v);
                out.len() as u32 - 1
            })
        })
        .collect();
    let tris = tris.iter().filter(|t| t.iter().all(|i| (*i as usize) < remap.len())).map(|t| [remap[t[0] as usize], remap[t[1] as usize], remap[t[2] as usize]]).filter(|t| t[0] != t[1] && t[1] != t[2] && t[0] != t[2]).collect();
    (out, tris)
}

/// Largest side of a landmark's colour and normal maps in a world; the
/// roughness/metal/occlusion maps are half of it. The framework makes them
/// 1024 to 2048 for close-up renders, and 63 landmarks have some 640
/// different textures between them (5 GB at full size); at this size a
/// texel is still about a centimetre of wall.
const TEXTURE_SIDE: usize = 512;

fn shrink_textures(scene: &mut SceneData) {
    let mut side = vec![TEXTURE_SIDE / 2; scene.textures.len()];
    for m in &scene.materials {
        for t in [m.base_color_tex, m.normal_tex].into_iter().flatten() {
            if let Some(s) = side.get_mut(t) {
                *s = TEXTURE_SIDE;
            }
        }
    }
    for (t, side) in scene.textures.iter_mut().zip(side) {
        loop {
            let (w, h) = (t.width as usize, t.height as usize);
            // Halve while the result is not below the wanted side.
            if w.max(h) < side * 2 || w.min(h) < 2 || t.rgba8.len() != w * h * 4 {
                break;
            }
            let (nw, nh) = (w / 2, h / 2);
            let mut out = vec![0u8; nw * nh * 4];
            for y in 0..nh {
                let (r0, r1) = (&t.rgba8[y * 2 * w * 4..][..w * 4], &t.rgba8[(y * 2 + 1) * w * 4..][..w * 4]);
                for x in 0..nw {
                    for c in 0..4 {
                        let i = x * 8 + c;
                        out[(y * nw + x) * 4 + c] = ((r0[i] as u32 + r0[i + 4] as u32 + r1[i] as u32 + r1[i + 4] as u32 + 2) / 4) as u8;
                    }
                }
            }
            t.width = nw as u32;
            t.height = nh as u32;
            t.rgba8 = Arc::from(out);
        }
    }
}

fn build_model(dir: &Path) -> Option<Model> {
    // A material library of its own: the shared one would keep every
    // landmark's full-size textures (gigabytes) for the life of the process.
    let lib = forja_city::landmark::Library::new(forja_city::landmark::shared_library(dir).kit);
    let mut lm = match forja_city::landmark::load_with(dir, &lib, Default::default()) {
        Ok(lm) => lm,
        Err(e) => {
            log::warn!("landmarks: {}: {e}", dir.display());
            return None;
        }
    };
    drop(lib);
    shrink_textures(&mut lm.scene);
    forja_city::landmark::far::extend_lods(&mut lm.scene);
    let (col_verts, col_tris) = weld(&lm.collider.verts, &lm.collider.tris);
    Some(Model { scene: lm.scene, col_verts, col_tris, bounds: lm.bounds })
}

/// A texture's identity in the cache: its pixels' hash and its shape.
type TexKey = (u64, u32, u32, bool);

fn tex_key(t: &TextureData) -> TexKey {
    (forja_assets::cache::fast_hash(&t.rgba8), t.width, t.height, t.srgb)
}

fn tex_cache_key(k: &TexKey) -> [u8; 32] {
    Key::new().str("isla landmark texture 1").u64(k.0).u64(k.1 as u64).u64(k.2 as u64).u64(k.3 as u64).finish()
}

/// Textures by content, shared by every landmark (the same stone on
/// twenty landmarks is one buffer, uploaded once) and kept in the cache
/// one entry each.
#[derive(Default)]
struct Textures {
    seen: Mutex<HashMap<TexKey, Option<Arc<[u8]>>>>,
}

impl Textures {
    /// The shared buffer of `t` (the first one seen with these pixels).
    fn share(&self, t: &mut TextureData) -> TexKey {
        let key = tex_key(t);
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        match seen.get(&key) {
            Some(Some(b)) if b.len() == t.rgba8.len() => t.rgba8 = b.clone(),
            _ => {
                seen.insert(key, Some(t.rgba8.clone()));
                let mut w = Writer::new();
                w.texture(t);
                forja_assets::cache::store_limited("isla_landmark_tex", &tex_cache_key(&key), &w.finish(), 4000);
            }
        }
        key
    }

    /// The pixels stored for `key` (`None`: not in the cache).
    fn fetch(&self, key: &TexKey) -> Option<Arc<[u8]>> {
        if let Some(b) = self.seen.lock().unwrap_or_else(|e| e.into_inner()).get(key) {
            return b.clone();
        }
        // Read outside the lock; two threads may read the same entry once.
        let read = forja_assets::cache::load("isla_landmark_tex", &tex_cache_key(key)).and_then(|b| Reader::new(&b)).and_then(|mut r| r.texture()).filter(|t| (t.width, t.height, t.srgb) == (key.1, key.2, key.3) && t.rgba8.len() == key.1 as usize * key.2 as usize * 4).map(|t| t.rgba8);
        self.seen.lock().unwrap_or_else(|e| e.into_inner()).entry(*key).or_insert(read).clone()
    }
}

/// `scene` for the cache: its textures by key, placeholders in their place.
fn write_scene(w: &mut Writer, scene: &SceneData, keys: &[TexKey], blank: &Arc<[u8]>) -> bool {
    let bare = SceneData { name: scene.name.clone(), nodes: scene.nodes.clone(), roots: scene.roots.clone(), meshes: scene.meshes.clone(), materials: scene.materials.clone(), textures: scene.textures.iter().map(|t| TextureData { name: t.name.clone(), width: 1, height: 1, rgba8: blank.clone(), srgb: t.srgb }).collect(), ..Default::default() };
    if !w.scene(&bare) {
        return false;
    }
    w.usize(keys.len());
    for k in keys {
        w.u64(k.0);
        w.u32(k.1);
        w.u32(k.2);
        w.bool(k.3);
    }
    true
}

fn read_scene(r: &mut Reader, textures: &Textures) -> Option<SceneData> {
    let mut scene = r.scene()?;
    if r.usize()? != scene.textures.len() {
        return None;
    }
    for t in &mut scene.textures {
        let key: TexKey = (r.u64()?, r.u32()?, r.u32()?, r.bool()?);
        t.rgba8 = textures.fetch(&key)?;
        (t.width, t.height, t.srgb) = (key.1, key.2, key.3);
    }
    Some(scene)
}

fn store_model(key: &[u8; 32], m: &Model, keys: &[TexKey]) {
    if forja_assets::cache::dir().is_none() {
        return;
    }
    let blank: Arc<[u8]> = Arc::from(vec![0u8; 4]);
    let mut w = Writer::new();
    if !write_scene(&mut w, &m.scene, keys, &blank) {
        return;
    }
    w.usize(m.col_verts.len());
    for v in &m.col_verts {
        w.vec3(*v);
    }
    w.u32s(&m.col_tris.iter().flatten().copied().collect::<Vec<u32>>());
    w.vec3(m.bounds.0);
    w.vec3(m.bounds.1);
    forja_assets::cache::store_limited("isla_landmark", key, &w.finish(), 160);
}

fn read_model(mut r: Reader, textures: &Textures) -> Option<Model> {
    let scene = read_scene(&mut r, textures)?;
    let n = r.usize()?;
    let mut col_verts = Vec::with_capacity(n.min(1 << 22));
    for _ in 0..n {
        col_verts.push(r.vec3()?);
    }
    let idx = r.u32s()?;
    if idx.len() % 3 != 0 || idx.iter().any(|i| *i as usize >= n) {
        return None;
    }
    let col_tris = idx.chunks_exact(3).map(|t| [t[0], t[1], t[2]]).collect();
    Some(Model { scene, col_verts, col_tris, bounds: (r.vec3()?, r.vec3()?) })
}

/// The models of the landmarks under a folder: read back from the cache
/// or built (and stored). Keep one for as long as landmarks are being
/// loaded, so that they share their textures; drop it afterwards (it
/// holds a reference to every texture).
pub struct Models {
    dir: PathBuf,
    code: [u8; 32],
    textures: Textures,
}

impl Models {
    /// `dir`: `data/landmarks/valencia`.
    pub fn new(dir: &Path) -> Self {
        let mut code = Key::new().str("isla landmark 5").str(include_str!("landmark_models.rs"));
        for s in forja_city::landmark::far::SOURCES {
            code = code.u64(forja_assets::cache::fast_hash(s.as_bytes()));
        }
        // The textures come from the kit's downloaded sets when they are there.
        let downloaded = std::env::var_os("FORJA_DOWNLOADED").map(PathBuf::from).or_else(|| forja_city::landmark::engine_root(dir).map(|r| r.join("assets/downloaded"))).filter(|p| p.exists());
        Models { dir: dir.to_path_buf(), code: code.str(if downloaded.is_some() { "kit" } else { "no kit" }).finish(), textures: Textures::default() }
    }

    fn key(&self, slug: &str) -> [u8; 32] {
        Key::new().bytes(&self.code).str(slug).dir_files(&self.dir.join(slug)).finish()
    }

    /// Whether `slug` is in the cache (it then loads in a fraction of a second).
    pub fn cached(&self, slug: &str) -> bool {
        forja_assets::cache::dir().is_some_and(|d| {
            let k = self.key(slug);
            d.join("isla_landmark").join(format!("{}.bin", k[..12].iter().map(|b| format!("{b:02x}")).collect::<String>())).exists()
        })
    }

    /// The model of `slug`; the second value says whether it was built
    /// (seconds) rather than read from the cache. `None`: its spec fails.
    pub fn load(&self, slug: &str) -> Option<(Model, bool)> {
        let key = self.key(slug);
        if let Some(m) = forja_assets::cache::load("isla_landmark", &key).and_then(|b| Reader::new(&b)).and_then(|r| read_model(r, &self.textures)) {
            return Some((m, false));
        }
        let mut m = build_model(&self.dir.join(slug))?;
        let keys: Vec<TexKey> = m.scene.textures.iter_mut().map(|t| self.textures.share(t)).collect();
        store_model(&key, &m, &keys);
        Some((m, true))
    }
}
