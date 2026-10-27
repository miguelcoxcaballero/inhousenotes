//! Block-compressed textures (BC1 / BC3) with an on-disk cache.
//!
//! Colour maps (sRGB) and packed data maps (ORM, occlusion) are encoded to
//! BC1 (4 bits per texel) or, when they carry alpha, BC3 (8 bits); normal
//! maps stay RGBA8 (the shader reads the unnormalised length of the
//! filtered normal for specular anti-aliasing, which a two-channel format
//! would lose). Encoding uses `texpresso` (pure Rust, range fit), once per
//! texture: the mip chain is stored in the asset cache
//! ([`forja_assets::cache`], kind `textures`) keyed by the pixels, the
//! format, the size cap and this encoder's source, so later loads skip
//! both the mip generation and the encoding. Adapters without
//! `TEXTURE_COMPRESSION_BC` keep RGBA8.

use forja_assets::TextureData;
use forja_assets::cache;

/// Bump when the encoded data changes meaning.
const VERSION: &str = "texcomp 1";
/// Texture entries kept on disk (a city uses ~200).
const MAX_ENTRIES: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Bc1,
    Bc3,
}

impl Kind {
    fn texpresso(self) -> texpresso::Format {
        match self {
            Kind::Bc1 => texpresso::Format::Bc1,
            Kind::Bc3 => texpresso::Format::Bc3,
        }
    }

    pub fn block_bytes(self) -> u32 {
        match self {
            Kind::Bc1 => 8,
            Kind::Bc3 => 16,
        }
    }

    pub fn format(self, srgb: bool) -> wgpu::TextureFormat {
        match (self, srgb) {
            (Kind::Bc1, true) => wgpu::TextureFormat::Bc1RgbaUnormSrgb,
            (Kind::Bc1, false) => wgpu::TextureFormat::Bc1RgbaUnorm,
            (Kind::Bc3, true) => wgpu::TextureFormat::Bc3RgbaUnormSrgb,
            (Kind::Bc3, false) => wgpu::TextureFormat::Bc3RgbaUnorm,
        }
    }

    fn code(self) -> u8 {
        match self {
            Kind::Bc1 => 1,
            Kind::Bc3 => 3,
        }
    }
}

/// The BC format for `t`, if it should be compressed: not a normal map,
/// and a first level that is a whole number of 4x4 blocks.
pub(crate) fn kind_for(t: &TextureData, normal_map: bool, first: (u32, u32)) -> Option<Kind> {
    if normal_map || first.0 % 4 != 0 || first.1 % 4 != 0 || t.rgba8.len() != (t.width * t.height * 4) as usize {
        return None;
    }
    let opaque = t.rgba8.chunks_exact(4).all(|p| p[3] == 255);
    Some(if opaque { Kind::Bc1 } else { Kind::Bc3 })
}

/// A ready mip chain: `(width, height, data)` per level.
pub(crate) type Mips = Vec<(u32, u32, Vec<u8>)>;

fn key(t: &TextureData, kind: Kind, alpha_cutoff: Option<f32>, max_size: u32) -> [u8; 32] {
    cache::Key::new()
        .str(VERSION)
        .str(include_str!("texcomp.rs"))
        .u64(kind.code() as u64)
        .u64(t.width as u64)
        .u64(t.height as u64)
        .u64(t.srgb as u64)
        .u64(alpha_cutoff.map_or(u64::MAX, |c| c.to_bits() as u64))
        .u64(max_size as u64)
        .u64(cache::fast_hash(&t.rgba8))
        .u64(cache::fast_hash(&t.rgba8[t.rgba8.len() / 2..]))
        .finish()
}

/// Cached compressed chain of `t` (levels from the first not larger than
/// `max_size`).
pub(crate) fn load(t: &TextureData, kind: Kind, alpha_cutoff: Option<f32>, max_size: u32) -> Option<Mips> {
    let bytes = cache::load("textures", &key(t, kind, alpha_cutoff, max_size))?;
    let mut pos = 0usize;
    let mut take = |n: usize| -> Option<&[u8]> {
        let s = bytes.get(pos..pos + n)?;
        pos += n;
        Some(s)
    };
    let u32_at = |b: &[u8]| u32::from_le_bytes(b.try_into().unwrap());
    let levels = u32_at(take(4)?) as usize;
    if levels == 0 || levels > 16 {
        return None;
    }
    let mut dims = Vec::with_capacity(levels);
    for _ in 0..levels {
        let (w, h) = (u32_at(take(4)?), u32_at(take(4)?));
        dims.push((w, h));
    }
    let mut out = Vec::with_capacity(levels);
    for (w, h) in dims {
        let n = (w.div_ceil(4) * h.div_ceil(4) * kind.block_bytes()) as usize;
        out.push((w, h, take(n)?.to_vec()));
    }
    (pos == bytes.len()).then_some(out)
}

/// Encodes an RGBA8 chain and stores it.
pub(crate) fn encode(t: &TextureData, kind: Kind, alpha_cutoff: Option<f32>, max_size: u32, rgba: &Mips) -> Mips {
    let params = texpresso::Params { algorithm: texpresso::Algorithm::RangeFit, ..Default::default() };
    let f = kind.texpresso();
    let out: Mips = rgba
        .iter()
        .map(|(w, h, data)| {
            let mut block = vec![0u8; f.compressed_size(*w as usize, *h as usize)];
            f.compress(data, *w as usize, *h as usize, params, &mut block);
            (*w, *h, block)
        })
        .collect();
    let mut payload = Vec::with_capacity(4 + out.len() * 8 + out.iter().map(|m| m.2.len()).sum::<usize>());
    payload.extend_from_slice(&(out.len() as u32).to_le_bytes());
    for (w, h, _) in &out {
        payload.extend_from_slice(&w.to_le_bytes());
        payload.extend_from_slice(&h.to_le_bytes());
    }
    for (_, _, d) in &out {
        payload.extend_from_slice(d);
    }
    cache::store_limited("textures", &key(t, kind, alpha_cutoff, max_size), &payload, MAX_ENTRIES);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn picks_formats() {
        let translucent = TextureData { name: "a".into(), width: 8, height: 8, rgba8: Arc::from(vec![200u8; 256]), srgb: true };
        assert_eq!(kind_for(&translucent, false, (8, 8)), Some(Kind::Bc3), "alpha 200 needs BC3");
        let mut px = vec![90u8; 256];
        for p in px.chunks_exact_mut(4) {
            p[3] = 255;
        }
        let t = TextureData { rgba8: Arc::from(px), ..translucent.clone() };
        assert_eq!(kind_for(&t, false, (8, 8)), Some(Kind::Bc1));
        assert_eq!(kind_for(&t, true, (8, 8)), None, "normal maps stay RGBA8");
        assert_eq!(kind_for(&t, false, (6, 8)), None, "partial blocks");
    }

    #[test]
    fn encodes_every_level_to_whole_blocks() {
        let mut px = vec![0u8; 8 * 8 * 4];
        for (i, p) in px.chunks_exact_mut(4).enumerate() {
            p.copy_from_slice(&[(i * 4) as u8, 128, 255 - (i * 4) as u8, 255]);
        }
        let t = TextureData { name: "g".into(), width: 8, height: 8, rgba8: Arc::from(px.clone()), srgb: true };
        let chain: Mips = vec![(8, 8, px), (4, 4, vec![100; 64]), (2, 2, vec![100; 16]), (1, 1, vec![100; 4])];
        // SAFETY: only this test sets the variable in this process.
        unsafe { std::env::set_var("FORJA_NO_CACHE", "1") };
        let out = encode(&t, Kind::Bc1, None, 4096, &chain);
        assert_eq!(out.iter().map(|m| m.2.len()).collect::<Vec<_>>(), vec![32, 8, 8, 8]);
    }
}
