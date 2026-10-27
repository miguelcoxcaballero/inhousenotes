//! Versioned on-disk cache for assets that are expensive to generate
//! (processed scanned materials, procedural trees).
//!
//! An entry is keyed by a SHA-256 of everything that determines it: the
//! producer's own source code (pass `include_str!` of the files that
//! generate it, so any code change invalidates the entry by itself), its
//! parameters and the name, size and modification time of every input
//! file. Entries are written to a temporary file and renamed into place,
//! carry the format version, their key and a checksum of the payload, and
//! anything that does not match on read is ignored and regenerated, so a
//! stale, truncated or foreign file can never be loaded.
//!
//! Location: `FORJA_CACHE_DIR`, else `<engine root>/cache`; `FORJA_NO_CACHE=1`
//! turns it off. Each kind keeps its most recent [`MAX_ENTRIES`] entries
//! (or the limit given to [`store_limited`]).
//!
//! Pixel buffers are stored once per shared buffer (scenes that share a
//! texture share it again after loading), delta-filtered and deflated, and
//! (de)compressed in parallel.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use forja_math::{Aabb, Quat, Vec2, Vec3, Vec4};
use rayon::prelude::*;
use sha2::{Digest, Sha256};

use crate::{AlphaMode, FoliageShading, MaterialData, MeshData, NodeData, SceneData, ShadowRole, TextureData, Trs};

/// Bump when the file layout or the meaning of a serialized field changes.
pub const FORMAT: u32 = 1;
/// Entries kept per kind (older ones are deleted when a new one is stored).
pub const MAX_ENTRIES: usize = 48;
const MAGIC: &[u8; 8] = b"FORJAC\0\x01";

/// Cache directory, or `None` when caching is off.
pub fn dir() -> Option<PathBuf> {
    if std::env::var_os("FORJA_NO_CACHE").is_some_and(|v| v != "0") {
        return None;
    }
    if let Some(d) = std::env::var_os("FORJA_CACHE_DIR") {
        return Some(PathBuf::from(d));
    }
    crate::manifest::find_engine_root().map(|r| r.join("cache"))
}

/// Builds a cache key.
pub struct Key(Sha256);

impl Default for Key {
    fn default() -> Self {
        Self::new()
    }
}

impl Key {
    pub fn new() -> Self {
        let mut h = Sha256::new();
        h.update(FORMAT.to_le_bytes());
        Key(h)
    }

    pub fn str(mut self, s: &str) -> Self {
        self.0.update((s.len() as u64).to_le_bytes());
        self.0.update(s.as_bytes());
        self
    }

    pub fn u64(mut self, v: u64) -> Self {
        self.0.update(v.to_le_bytes());
        self
    }

    pub fn f32(self, v: f32) -> Self {
        self.u64(v.to_bits() as u64)
    }

    pub fn bytes(mut self, b: &[u8]) -> Self {
        self.0.update((b.len() as u64).to_le_bytes());
        self.0.update(b);
        self
    }

    /// Name, size and modification time of a file (or "missing").
    pub fn file(self, path: &Path) -> Self {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        match std::fs::metadata(path) {
            Ok(m) => {
                let mtime = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos() as u64);
                self.str(&name).u64(m.len()).u64(mtime)
            }
            Err(_) => self.str(&name).str("missing"),
        }
    }

    /// Every file directly inside `dir` (sorted), as [`Key::file`].
    pub fn dir_files(mut self, dir: &Path) -> Self {
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir).map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect()).unwrap_or_default();
        files.sort();
        self = self.u64(files.len() as u64);
        for f in &files {
            self = self.file(f);
        }
        self
    }

    pub fn finish(self) -> [u8; 32] {
        self.0.finalize().into()
    }
}

fn hex(k: &[u8; 32]) -> String {
    k[..12].iter().map(|b| format!("{b:02x}")).collect()
}

/// Fast 64-bit hash of a byte buffer (cache keys of large pixel buffers,
/// payload checksums).
pub fn fast_hash(b: &[u8]) -> u64 {
    checksum(b)
}

fn checksum(b: &[u8]) -> u64 {
    // FNV-1a over 8-byte words with a final mix: cheap, catches truncation
    // and corruption (the key is checked separately).
    let mut h = 0xcbf2_9ce4_8422_2325u64 ^ b.len() as u64;
    let mut chunks = b.chunks_exact(8);
    for c in &mut chunks {
        h = (h ^ u64::from_le_bytes(c.try_into().unwrap())).wrapping_mul(0x0100_0000_01b3).rotate_left(29);
    }
    for &x in chunks.remainder() {
        h = (h ^ x as u64).wrapping_mul(0x0100_0000_01b3);
    }
    h ^ (h >> 31)
}

fn entry_path(kind: &str, key: &[u8; 32]) -> Option<PathBuf> {
    Some(dir()?.join(kind).join(format!("{}.bin", hex(key))))
}

/// The payload stored for `key`, if a valid entry exists.
pub fn load(kind: &str, key: &[u8; 32]) -> Option<Vec<u8>> {
    let path = entry_path(kind, key)?;
    let mut f = std::fs::File::open(&path).ok()?;
    let mut head = [0u8; 8 + 4 + 32 + 8 + 8];
    f.read_exact(&mut head).ok()?;
    if &head[..8] != MAGIC || u32::from_le_bytes(head[8..12].try_into().unwrap()) != FORMAT || head[12..44] != key[..] {
        return None;
    }
    let len = u64::from_le_bytes(head[44..52].try_into().unwrap()) as usize;
    let sum = u64::from_le_bytes(head[52..60].try_into().unwrap());
    let mut payload = Vec::new();
    payload.try_reserve_exact(len).ok()?;
    f.take(len as u64 + 1).read_to_end(&mut payload).ok()?;
    if payload.len() != len || checksum(&payload) != sum {
        log::warn!("cache: {} is damaged; regenerating", path.display());
        let _ = std::fs::remove_file(&path);
        return None;
    }
    // Touch it so pruning keeps recently used entries.
    let _ = std::fs::File::options().append(true).open(&path).and_then(|f| f.set_modified(std::time::SystemTime::now()));
    Some(payload)
}

/// Stores `payload` for `key` (atomically: temporary file, then rename).
/// Failures only log: the cache is an optimisation.
pub fn store(kind: &str, key: &[u8; 32], payload: &[u8]) {
    store_limited(kind, key, payload, MAX_ENTRIES);
}

/// [`store`], keeping at most `max_entries` entries of `kind`.
pub fn store_limited(kind: &str, key: &[u8; 32], payload: &[u8], max_entries: usize) {
    let Some(path) = entry_path(kind, key) else { return };
    let Some(dir) = path.parent() else { return };
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join(format!(".{}.{}.tmp", hex(key), std::process::id()));
        {
            let mut f = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
            f.write_all(MAGIC)?;
            f.write_all(&FORMAT.to_le_bytes())?;
            f.write_all(key)?;
            f.write_all(&(payload.len() as u64).to_le_bytes())?;
            f.write_all(&checksum(payload).to_le_bytes())?;
            f.write_all(payload)?;
            f.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        }
        std::fs::rename(&tmp, &path)?;
        Ok(())
    };
    match write() {
        Ok(()) => prune(dir, max_entries),
        Err(e) => log::warn!("cache: could not write {}: {e}", path.display()),
    }
}

fn prune(dir: &Path, max_entries: usize) {
    let mut entries: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path()))).filter(|(_, p)| p.extension().is_some_and(|x| x == "bin")).collect())
        .unwrap_or_default();
    if entries.len() <= max_entries {
        return;
    }
    entries.sort();
    for (_, p) in &entries[..entries.len() - max_entries] {
        let _ = std::fs::remove_file(p);
    }
}

// ------------------------------------------------------------------ encoding

/// Serializer. Pixel buffers go to a side table (one per shared buffer).
#[derive(Default)]
pub struct Writer {
    buf: Vec<u8>,
    blobs: Vec<Arc<[u8]>>,
    blob_of: HashMap<usize, u32>,
}

impl Writer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    pub fn bool(&mut self, v: bool) {
        self.u8(v as u8);
    }
    pub fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub fn usize(&mut self, v: usize) {
        self.u64(v as u64);
    }
    pub fn f32(&mut self, v: f32) {
        self.u32(v.to_bits());
    }
    pub fn str(&mut self, s: &str) {
        self.usize(s.len());
        self.buf.extend_from_slice(s.as_bytes());
    }
    pub fn opt_usize(&mut self, v: Option<usize>) {
        self.u64(v.map_or(u64::MAX, |v| v as u64));
    }
    pub fn vec2(&mut self, v: Vec2) {
        self.f32s(&v.to_array());
    }
    pub fn vec3(&mut self, v: Vec3) {
        self.f32s(&v.to_array());
    }
    pub fn vec4(&mut self, v: Vec4) {
        self.f32s(&v.to_array());
    }
    pub fn f32s(&mut self, v: &[f32]) {
        self.buf.reserve(v.len() * 4);
        for x in v {
            self.buf.extend_from_slice(&x.to_le_bytes());
        }
    }
    pub fn u32s(&mut self, v: &[u32]) {
        self.usize(v.len());
        self.buf.reserve(v.len() * 4);
        for x in v {
            self.buf.extend_from_slice(&x.to_le_bytes());
        }
    }

    pub fn texture(&mut self, t: &TextureData) {
        self.str(&t.name);
        self.u32(t.width);
        self.u32(t.height);
        self.bool(t.srgb);
        let n = self.blobs.len() as u32;
        let i = *self.blob_of.entry(t.rgba8.as_ptr() as usize).or_insert(n);
        if i == n {
            self.blobs.push(t.rgba8.clone());
        }
        self.u32(i);
    }

    pub fn opt_texture(&mut self, t: Option<&TextureData>) {
        self.bool(t.is_some());
        if let Some(t) = t {
            self.texture(t);
        }
    }

    pub fn material(&mut self, m: &MaterialData) {
        self.str(&m.name);
        self.vec4(m.base_color);
        self.opt_usize(m.base_color_tex);
        self.f32(m.metallic);
        self.f32(m.roughness);
        self.opt_usize(m.metallic_roughness_tex);
        self.opt_usize(m.normal_tex);
        self.f32(m.normal_scale);
        self.opt_usize(m.occlusion_tex);
        self.vec3(m.emissive);
        self.opt_usize(m.emissive_tex);
        match m.alpha {
            AlphaMode::Opaque => self.u8(0),
            AlphaMode::Mask(c) => {
                self.u8(1);
                self.u8(c);
            }
            AlphaMode::Blend => self.u8(2),
        }
        self.bool(m.double_sided);
        self.bool(m.unlit);
        self.f32(m.transmission);
        self.f32(m.clearcoat);
        self.f32(m.clearcoat_roughness);
        self.f32(m.subsurface);
        self.bool(m.foliage.is_some());
        if let Some(f) = m.foliage {
            self.f32(f.translucency);
            self.f32(f.backface_cull_v);
        }
        self.f32(m.specular);
    }

    pub fn mesh(&mut self, m: &MeshData) {
        self.usize(m.positions.len());
        for (i, p) in m.positions.iter().enumerate() {
            self.vec3(*p);
            self.vec3(m.normals.get(i).copied().unwrap_or(Vec3::Y));
            self.vec2(m.uvs.get(i).copied().unwrap_or(Vec2::ZERO));
            self.vec4(m.tangents.get(i).copied().unwrap_or(Vec4::X));
        }
        self.bool(!m.uvs.is_empty());
        self.bool(!m.tangents.is_empty());
        self.u32s(&m.indices);
        self.usize(m.joints.len());
        for (j, w) in m.joints.iter().zip(&m.weights) {
            for x in j {
                self.buf.extend_from_slice(&x.to_le_bytes());
            }
            self.f32s(w);
        }
        self.usize(m.material);
        self.vec3(m.aabb.min);
        self.vec3(m.aabb.max);
        self.usize(m.lods.len());
        for l in &m.lods {
            self.u32s(l);
        }
        self.bool(m.lod_sync);
        self.u8(match m.shadow {
            ShadowRole::All => 0,
            ShadowRole::Near => 1,
            ShadowRole::Proxy => 2,
            ShadowRole::Fine => 3,
        });
    }

    /// A scene without skins or animation clips (`false`: not cacheable).
    pub fn scene(&mut self, s: &SceneData) -> bool {
        if !s.skins.is_empty() || !s.clips.is_empty() {
            return false;
        }
        self.str(&s.name);
        self.usize(s.nodes.len());
        for n in &s.nodes {
            self.str(&n.name);
            self.opt_usize(n.parent);
            self.usize(n.children.len());
            for c in &n.children {
                self.usize(*c);
            }
            self.vec3(n.local.t);
            self.vec4(Vec4::from(n.local.r));
            self.vec3(n.local.s);
            self.opt_usize(n.mesh);
            self.opt_usize(n.skin);
        }
        self.usize(s.roots.len());
        for r in &s.roots {
            self.usize(*r);
        }
        self.usize(s.meshes.len());
        for prims in &s.meshes {
            self.usize(prims.len());
            for m in prims {
                self.mesh(m);
            }
        }
        self.usize(s.materials.len());
        for m in &s.materials {
            self.material(m);
        }
        self.usize(s.textures.len());
        for t in &s.textures {
            self.texture(t);
        }
        true
    }

    /// The payload: the blob table (compressed in parallel), then the
    /// serialized fields.
    pub fn finish(self) -> Vec<u8> {
        let packed: Vec<Vec<u8>> = self.blobs.par_iter().map(|b| pack(b)).collect();
        let mut out = Vec::with_capacity(packed.iter().map(|p| p.len() + 16).sum::<usize>() + self.buf.len() + 16);
        out.extend_from_slice(&(packed.len() as u64).to_le_bytes());
        for (p, b) in packed.iter().zip(&self.blobs) {
            out.extend_from_slice(&(b.len() as u64).to_le_bytes());
            out.extend_from_slice(&(p.len() as u64).to_le_bytes());
        }
        for p in &packed {
            out.extend_from_slice(p);
        }
        out.extend_from_slice(&self.buf);
        out
    }
}

/// Delta filter (per RGBA channel) and fast deflate.
fn pack(b: &[u8]) -> Vec<u8> {
    let mut f = Vec::with_capacity(b.len());
    if b.len() % 4 == 0 && b.len() >= 8 {
        f.extend_from_slice(&b[..4]);
        f.extend(b[4..].iter().zip(b.iter()).map(|(x, p)| x.wrapping_sub(*p)));
    } else {
        f.extend_from_slice(b);
    }
    let mut e = flate2::write::DeflateEncoder::new(Vec::with_capacity(b.len() / 2), flate2::Compression::fast());
    e.write_all(&f).expect("in-memory deflate");
    e.finish().expect("in-memory deflate")
}

fn unpack(p: &[u8], len: usize) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    out.try_reserve_exact(len).ok()?;
    flate2::read::DeflateDecoder::new(p).take(len as u64 + 1).read_to_end(&mut out).ok()?;
    if out.len() != len {
        return None;
    }
    if len % 4 == 0 && len >= 8 {
        for i in 4..len {
            out[i] = out[i].wrapping_add(out[i - 4]);
        }
    }
    Some(out)
}

/// Deserializer; every read fails (`None`) on malformed data. It owns
/// the pixel buffers and a copy of the (small) field section, so the
/// payload can be dropped as soon as it is opened.
pub struct Reader {
    buf: Vec<u8>,
    pos: usize,
    blobs: Vec<Arc<[u8]>>,
}

impl Reader {
    /// Opens a [`Writer::finish`] payload (decompresses its pixel buffers
    /// in parallel).
    pub fn new(payload: &[u8]) -> Option<Self> {
        let mut pos = 0usize;
        let mut take = |n: usize| -> Option<&[u8]> {
            let end = pos.checked_add(n)?;
            let s = payload.get(pos..end)?;
            pos = end;
            Some(s)
        };
        let word = |b: &[u8]| usize::try_from(u64::from_le_bytes(b.try_into().ok()?)).ok();
        let n = word(take(8)?)?;
        if n > 1 << 24 {
            return None;
        }
        let mut sizes = Vec::with_capacity(n);
        for _ in 0..n {
            sizes.push((word(take(8)?)?, word(take(8)?)?));
        }
        let mut spans = Vec::with_capacity(n);
        for (raw, packed) in sizes {
            spans.push((take(packed)?, raw));
        }
        let blobs: Option<Vec<Arc<[u8]>>> = spans.into_par_iter().map(|(p, raw)| unpack(p, raw).map(Arc::from)).collect();
        Some(Reader { buf: payload[pos..].to_vec(), pos: 0, blobs: blobs? })
    }

    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let end = self.pos.checked_add(n)?;
        let s = self.buf.get(self.pos..end)?;
        self.pos = end;
        Some(s)
    }

    pub fn at_end(&self) -> bool {
        self.pos == self.buf.len()
    }
    pub fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    pub fn bool(&mut self) -> Option<bool> {
        match self.u8()? {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    }
    pub fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    pub fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    pub fn usize(&mut self) -> Option<usize> {
        usize::try_from(self.u64()?).ok()
    }
    /// A length that must fit in the remaining bytes at `unit` bytes each.
    fn len(&mut self, unit: usize) -> Option<usize> {
        let n = self.usize()?;
        (n.checked_mul(unit)? <= self.buf.len() - self.pos).then_some(n)
    }
    pub fn f32(&mut self) -> Option<f32> {
        Some(f32::from_bits(self.u32()?))
    }
    pub fn str(&mut self) -> Option<String> {
        let n = self.len(1)?;
        String::from_utf8(self.take(n)?.to_vec()).ok()
    }
    pub fn opt_usize(&mut self) -> Option<Option<usize>> {
        let v = self.u64()?;
        Some(if v == u64::MAX { None } else { Some(usize::try_from(v).ok()?) })
    }
    pub fn vec2(&mut self) -> Option<Vec2> {
        Some(Vec2::new(self.f32()?, self.f32()?))
    }
    pub fn vec3(&mut self) -> Option<Vec3> {
        Some(Vec3::new(self.f32()?, self.f32()?, self.f32()?))
    }
    pub fn vec4(&mut self) -> Option<Vec4> {
        Some(Vec4::new(self.f32()?, self.f32()?, self.f32()?, self.f32()?))
    }
    pub fn u32s(&mut self) -> Option<Vec<u32>> {
        let n = self.len(4)?;
        let b = self.take(n * 4)?;
        Some(b.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect())
    }

    pub fn texture(&mut self) -> Option<TextureData> {
        let name = self.str()?;
        let (width, height, srgb) = (self.u32()?, self.u32()?, self.bool()?);
        let i = self.u32()? as usize;
        let rgba8 = self.blobs.get(i)?.clone();
        Some(TextureData { name, width, height, rgba8, srgb })
    }

    pub fn opt_texture(&mut self) -> Option<Option<TextureData>> {
        Some(if self.bool()? { Some(self.texture()?) } else { None })
    }

    pub fn material(&mut self) -> Option<MaterialData> {
        Some(MaterialData {
            name: self.str()?,
            base_color: self.vec4()?,
            base_color_tex: self.opt_usize()?,
            metallic: self.f32()?,
            roughness: self.f32()?,
            metallic_roughness_tex: self.opt_usize()?,
            normal_tex: self.opt_usize()?,
            normal_scale: self.f32()?,
            occlusion_tex: self.opt_usize()?,
            emissive: self.vec3()?,
            emissive_tex: self.opt_usize()?,
            alpha: match self.u8()? {
                0 => AlphaMode::Opaque,
                1 => AlphaMode::Mask(self.u8()?),
                2 => AlphaMode::Blend,
                _ => return None,
            },
            double_sided: self.bool()?,
            unlit: self.bool()?,
            transmission: self.f32()?,
            clearcoat: self.f32()?,
            clearcoat_roughness: self.f32()?,
            subsurface: self.f32()?,
            foliage: if self.bool()? { Some(FoliageShading { translucency: self.f32()?, backface_cull_v: self.f32()? }) } else { None },
            specular: self.f32()?,
        })
    }

    pub fn mesh(&mut self) -> Option<MeshData> {
        let n = self.len(48)?;
        let mut m = MeshData { positions: Vec::with_capacity(n), normals: Vec::with_capacity(n), uvs: Vec::with_capacity(n), tangents: Vec::with_capacity(n), ..Default::default() };
        for _ in 0..n {
            m.positions.push(self.vec3()?);
            m.normals.push(self.vec3()?);
            m.uvs.push(self.vec2()?);
            m.tangents.push(self.vec4()?);
        }
        if !self.bool()? {
            m.uvs.clear();
        }
        if !self.bool()? {
            m.tangents.clear();
        }
        m.indices = self.u32s()?;
        let nj = self.len(24)?;
        for _ in 0..nj {
            let mut j = [0u16; 4];
            for x in &mut j {
                *x = u16::from_le_bytes(self.take(2)?.try_into().ok()?);
            }
            m.joints.push(j);
            m.weights.push([self.f32()?, self.f32()?, self.f32()?, self.f32()?]);
        }
        m.material = self.usize()?;
        m.aabb = Aabb { min: self.vec3()?, max: self.vec3()? };
        let nl = self.len(8)?;
        for _ in 0..nl {
            m.lods.push(self.u32s()?);
        }
        m.lod_sync = self.bool()?;
        m.shadow = match self.u8()? {
            0 => ShadowRole::All,
            1 => ShadowRole::Near,
            2 => ShadowRole::Proxy,
            3 => ShadowRole::Fine,
            _ => return None,
        };
        let nv = m.positions.len() as u32;
        if m.indices.iter().chain(m.lods.iter().flatten()).any(|&i| i >= nv) {
            return None;
        }
        Some(m)
    }

    pub fn scene(&mut self) -> Option<SceneData> {
        let name = self.str()?;
        let nn = self.len(8)?;
        let mut nodes = Vec::with_capacity(nn);
        for _ in 0..nn {
            let name = self.str()?;
            let parent = self.opt_usize()?;
            let nc = self.len(8)?;
            let mut children = Vec::with_capacity(nc);
            for _ in 0..nc {
                children.push(self.usize()?);
            }
            let t = self.vec3()?;
            let r = Quat::from_vec4(self.vec4()?);
            let s = self.vec3()?;
            nodes.push(NodeData { name, parent, children, local: Trs { t, r, s }, mesh: self.opt_usize()?, skin: self.opt_usize()? });
        }
        let nr = self.len(8)?;
        let mut roots = Vec::with_capacity(nr);
        for _ in 0..nr {
            roots.push(self.usize()?);
        }
        let nm = self.len(8)?;
        let mut meshes = Vec::with_capacity(nm);
        for _ in 0..nm {
            let np = self.len(8)?;
            let mut prims = Vec::with_capacity(np);
            for _ in 0..np {
                prims.push(self.mesh()?);
            }
            meshes.push(prims);
        }
        let nmat = self.len(8)?;
        let mut materials = Vec::with_capacity(nmat);
        for _ in 0..nmat {
            materials.push(self.material()?);
        }
        let nt = self.len(8)?;
        let mut textures = Vec::with_capacity(nt);
        for _ in 0..nt {
            textures.push(self.texture()?);
        }
        let s = SceneData { name, nodes, roots, meshes, materials, textures, skins: Vec::new(), clips: Vec::new() };
        // Indices must stay in range (a valid entry always is).
        let ok = s.roots.iter().all(|&r| r < s.nodes.len())
            && s.nodes.iter().all(|n| n.parent.is_none_or(|p| p < s.nodes.len()) && n.children.iter().all(|&c| c < s.nodes.len()) && n.mesh.is_none_or(|m| m < s.meshes.len()) && n.skin.is_none())
            && s.meshes.iter().flatten().all(|m| m.material < s.materials.len().max(1))
            && s.materials.iter().all(|m| [m.base_color_tex, m.metallic_roughness_tex, m.normal_tex, m.occlusion_tex, m.emissive_tex].iter().flatten().all(|&t| t < s.textures.len()));
        ok.then_some(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene() -> SceneData {
        let tex = TextureData { name: "t".into(), width: 2, height: 2, rgba8: Arc::from(vec![1u8, 2, 3, 4, 5, 6, 7, 8, 200, 100, 0, 255, 9, 9, 9, 9]), srgb: true };
        let mut mesh = MeshData { positions: vec![Vec3::ZERO, Vec3::X, Vec3::Y], normals: vec![Vec3::Z; 3], uvs: vec![Vec2::ZERO; 3], tangents: vec![Vec4::X; 3], indices: vec![0, 1, 2], lods: vec![vec![0, 1, 2]], shadow: ShadowRole::Near, ..Default::default() };
        mesh.finalize();
        SceneData {
            name: "s".into(),
            nodes: vec![NodeData { name: "n".into(), mesh: Some(0), ..Default::default() }],
            roots: vec![0],
            meshes: vec![vec![mesh]],
            materials: vec![MaterialData { base_color_tex: Some(0), normal_tex: Some(1), alpha: AlphaMode::Mask(107), foliage: Some(FoliageShading::default()), ..Default::default() }],
            textures: vec![tex.clone(), tex],
            ..Default::default()
        }
    }

    #[test]
    fn scenes_round_trip_and_share_buffers() {
        let s = scene();
        let mut w = Writer::new();
        assert!(w.scene(&s));
        let bytes = w.finish();
        let mut r = Reader::new(&bytes).unwrap();
        let back = r.scene().unwrap();
        assert!(r.at_end());
        assert_eq!(back.meshes[0][0].positions, s.meshes[0][0].positions);
        assert_eq!(back.meshes[0][0].indices, s.meshes[0][0].indices);
        assert_eq!(back.meshes[0][0].shadow, ShadowRole::Near);
        assert_eq!(back.materials[0].alpha, AlphaMode::Mask(107));
        assert_eq!(&back.textures[0].rgba8[..], &s.textures[0].rgba8[..]);
        assert!(Arc::ptr_eq(&back.textures[0].rgba8, &back.textures[1].rgba8));
    }

    #[test]
    fn malformed_payloads_are_rejected() {
        let mut w = Writer::new();
        w.scene(&scene());
        let bytes = w.finish();
        for cut in [0, 8, bytes.len() / 2, bytes.len() - 1] {
            let ok = Reader::new(&bytes[..cut]).and_then(|mut r| r.scene().filter(|_| r.at_end()));
            assert!(ok.is_none(), "cut at {cut}");
        }
    }

    #[test]
    fn entries_are_keyed_and_checked() {
        let d = std::env::temp_dir().join(format!("forja_cache_test_{}", std::process::id()));
        // SAFETY: tests in this module are the only users of the variable.
        unsafe { std::env::set_var("FORJA_CACHE_DIR", &d) };
        let k = Key::new().str("a").finish();
        let k2 = Key::new().str("b").finish();
        store("test", &k, b"hello");
        assert_eq!(load("test", &k).as_deref(), Some(&b"hello"[..]));
        assert!(load("test", &k2).is_none());
        // Corrupt the payload: rejected and removed.
        let p = entry_path("test", &k).unwrap();
        let mut b = std::fs::read(&p).unwrap();
        let n = b.len();
        b[n - 1] ^= 0xff;
        std::fs::write(&p, b).unwrap();
        assert!(load("test", &k).is_none());
        assert!(!p.exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}
