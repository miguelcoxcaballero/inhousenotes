//! Image-based lighting from an equirectangular HDR photo: a mipmapped
//! radiance map for reflections and the sky, 3rd-order spherical harmonics
//! for diffuse irradiance, and the sun extracted for shadowed direct light.

use forja_assets::HdrImage;
use forja_math::{Vec3, vec3};

pub struct EnvMap {
    /// Mip chain of RGBA f16 bits, largest first.
    pub mips: Vec<(u32, u32, Vec<u16>)>,
    /// SH9 coefficients of irradiance / pi (ready to multiply albedo).
    pub sh: [Vec3; 9],
    pub sun_dir: Vec3,
    /// Sun irradiance (already scaled).
    pub sun_color: Vec3,
    /// Uniform scale applied so the sun matches `target_sun` luminance.
    pub scale: f32,
    /// Reflection mips are GGX-prefiltered (false: the legacy box-filtered
    /// pyramid of `FORJA_FX=-ggx`, kept for before/after comparisons).
    pub prefiltered: bool,
}

/// Direction for equirect texel center (u right, v down, -Z at u = 0.5).
pub fn dir_from_uv(u: f32, v: f32) -> Vec3 {
    let phi = (u - 0.5) * std::f32::consts::TAU;
    let theta = v * std::f32::consts::PI;
    vec3(theta.sin() * phi.sin(), theta.cos(), -theta.sin() * phi.cos())
}

fn lum(c: Vec3) -> f32 {
    c.dot(vec3(0.2126, 0.7152, 0.0722))
}

fn f16(v: f32) -> u16 {
    // Minimal float -> half conversion (round toward zero, clamps to 65504).
    let v = v.clamp(0.0, 65504.0);
    if v < 6.1e-5 {
        return (v / 5.96e-8) as u16; // subnormals
    }
    let bits = v.to_bits();
    let exp = ((bits >> 23) & 0xff) as i32 - 127 + 15;
    let mant = (bits >> 13) & 0x3ff;
    ((exp as u32) << 10 | mant) as u16
}

/// Albedo of the synthetic ground that replaces the photo's lower
/// hemisphere in the diffuse light: warm Spanish paving, dry soil and
/// asphalt in partial shade. The renderer has no bounce light, so this one
/// bounce from an infinite Lambertian ground lit by the photo's own sun and
/// sky is what lifts shaded facades with the warm glow of a sunlit street
/// (the street in the photo reflects only ~6%).
pub const GROUND_ALBEDO: Vec3 = Vec3::new(0.17, 0.15, 0.125);

/// Effective albedo of the street walls around the viewer for the diffuse
/// light: the low band of the photo's sky (up to ~25 degrees, fading out by
/// ~35) is replaced by facades lit by the photo's sun and sky, so surfaces
/// turned away from the sun get the warm bounce of the sunlit facades
/// across the street. Cream and ochre plaster is ~0.5, about half of it in
/// sun, and walls fill only part of the band (gaps, cross streets, dark
/// windows): ~0.15 effective.
pub const WALL_ALBEDO: Vec3 = Vec3::new(0.16, 0.145, 0.12);

/// Smallest reflection mip (texels across the equirect width).
pub const REFLECTION_MIN_WIDTH: usize = 32;

/// Perceptual roughness stored in reflection mip `k` (1..=`levels`): the
/// shader picks `lod = 1 + r (2 - r) (levels - 1)` (Filament's mapping,
/// which spends more mips on the glossy range), this is its inverse.
pub fn mip_roughness(k: usize, levels: usize) -> f32 {
    if levels <= 1 {
        return 1.0;
    }
    let x = (k as f32 - 1.0) / (levels as f32 - 1.0);
    1.0 - (1.0 - x).max(0.0).sqrt()
}

/// Box-filtered pyramid of an equirect image down to 8x4 (source of the
/// filtered importance sampling).
fn box_chain(w: usize, h: usize, level0: Vec<Vec3>) -> Vec<(usize, usize, Vec<Vec3>)> {
    let mut chain = vec![(w, h, level0)];
    loop {
        let (cw, ch, cur) = chain.last().expect("non-empty");
        let (cw, ch) = (*cw, *ch);
        if cw <= 8 || ch <= 4 {
            break;
        }
        let (nw, nh) = (cw / 2, ch / 2);
        let mut next = vec![Vec3::ZERO; nw * nh];
        for y in 0..nh {
            for x in 0..nw {
                next[y * nw + x] = (cur[2 * y * cw + 2 * x] + cur[2 * y * cw + 2 * x + 1] + cur[(2 * y + 1) * cw + 2 * x] + cur[(2 * y + 1) * cw + 2 * x + 1]) * 0.25;
            }
        }
        chain.push((nw, nh, next));
    }
    chain
}

/// Bilinear equirect lookup (wraps in u, clamps in v).
fn sample_level(level: &(usize, usize, Vec<Vec3>), d: Vec3) -> Vec3 {
    let (w, h, ref px) = *level;
    let u = d.x.atan2(-d.z) / std::f32::consts::TAU + 0.5;
    let v = d.y.clamp(-1.0, 1.0).acos() / std::f32::consts::PI;
    let fx = u * w as f32 - 0.5;
    let fy = (v * h as f32 - 0.5).clamp(0.0, h as f32 - 1.0);
    let (x0, y0) = (fx.floor(), fy.floor());
    let (tx, ty) = (fx - x0, fy - y0);
    let xi = |x: f32| (x as i64).rem_euclid(w as i64) as usize;
    let (xa, xb) = (xi(x0), xi(x0 + 1.0));
    let (ya, yb) = (y0 as usize, (y0 as usize + 1).min(h - 1));
    let a = px[ya * w + xa] * (1.0 - tx) + px[ya * w + xb] * tx;
    let b = px[yb * w + xa] * (1.0 - tx) + px[yb * w + xb] * tx;
    a * (1.0 - ty) + b * ty
}

fn sample_lod(chain: &[(usize, usize, Vec<Vec3>)], d: Vec3, lod: f32) -> Vec3 {
    let lod = lod.clamp(0.0, (chain.len() - 1) as f32);
    let l0 = lod.floor() as usize;
    let t = lod - l0 as f32;
    let a = sample_level(&chain[l0], d);
    if t <= 0.0 || l0 + 1 >= chain.len() { a } else { a * (1.0 - t) + sample_level(&chain[l0 + 1], d) * t }
}

/// Hammersley point `i` of `n`.
pub(crate) fn hammersley(i: u32, n: u32) -> (f32, f32) {
    (i as f32 / n as f32, i.reverse_bits() as f32 * 2.328_306_4e-10)
}

/// GGX half vector around +Z for the uniform pair `(u1, u2)` (alpha = r^2).
pub(crate) fn ggx_half(u1: f32, u2: f32, a: f32) -> Vec3 {
    let phi = std::f32::consts::TAU * u1;
    let cos_t = ((1.0 - u2) / (1.0 + (a * a - 1.0) * u2)).max(0.0).sqrt();
    let sin_t = (1.0 - cos_t * cos_t).max(0.0).sqrt();
    vec3(sin_t * phi.cos(), sin_t * phi.sin(), cos_t)
}

/// Radiance convolved with the GGX lobe (N = V = R) at `rough`, into a
/// `w` x `h` equirect: importance sampling with lod picked from each
/// sample's pdf (Krivanek & Colbert 2008), so 48 samples are noise free.
/// Cost at load: ~0.3 s for a 2k HDRI on 4 cores.
fn prefilter_ggx(chain: &[(usize, usize, Vec<Vec3>)], w: usize, h: usize, rough: f32) -> Vec<Vec3> {
    let a = rough * rough;
    let n_samples: u32 = if w >= 512 { 32 } else { 64 };
    let (w0, h0) = (chain[0].0 as f32, chain[0].1 as f32);
    // Average texel solid angle of the source's level 0.
    let omega_p = 4.0 * std::f32::consts::PI / (w0 * h0);
    // Precomputed samples in the tangent frame: (L, NoL, lod).
    let samples: Vec<(Vec3, f32, f32)> = (0..n_samples)
        .filter_map(|i| {
            let (u1, u2) = hammersley(i, n_samples);
            let hv = ggx_half(u1, u2, a);
            let l = vec3(0.0, 0.0, 1.0) * (2.0 * hv.z * hv.z - 1.0) + vec3(hv.x, hv.y, 0.0) * (2.0 * hv.z);
            let nl = l.z;
            if nl <= 0.0 {
                return None;
            }
            // pdf(L) = D(h) NoH / (4 VoH) = D / 4 with N = V.
            let nh = hv.z;
            let a2 = a * a;
            let f = nh * nh * (a2 - 1.0) + 1.0;
            let d = a2 / (std::f32::consts::PI * f * f);
            let omega_s = 1.0 / (n_samples as f32 * d * 0.25 + 1e-6);
            let lod = (0.5 * (omega_s / omega_p).log2() + 1.0).max(0.0);
            Some((l, nl, lod))
        })
        .collect();
    let mut out = vec![Vec3::ZERO; w * h];
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(16);
    let rows_per = h.div_ceil(threads);
    std::thread::scope(|s| {
        for (ci, rows) in out.chunks_mut(rows_per * w).enumerate() {
            let samples = &samples;
            s.spawn(move || {
                for (ri, row) in rows.chunks_mut(w).enumerate() {
                    let y = ci * rows_per + ri;
                    for (x, o) in row.iter_mut().enumerate() {
                        let n = dir_from_uv((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
                        let up = if n.y.abs() < 0.999 { Vec3::Y } else { Vec3::X };
                        let tx = up.cross(n).normalize();
                        let ty = n.cross(tx);
                        let mut acc = Vec3::ZERO;
                        let mut wsum = 0.0;
                        for &(l, nl, lod) in samples {
                            let d = tx * l.x + ty * l.y + n * l.z;
                            acc += sample_lod(chain, d, lod) * nl;
                            wsum += nl;
                        }
                        *o = if wsum > 0.0 { acc / wsum } else { sample_lod(chain, n, 0.0) };
                    }
                }
            });
        }
    });
    out
}

impl EnvMap {
    /// Environment from an HDR photo whose sun is normalised to `target_sun`
    /// renderer units of illuminance at normal incidence (14 ~ 105 klx, see
    /// [`crate::exposure`]). Diffuse light from below the horizon comes from
    /// a ground of [`GROUND_ALBEDO`].
    pub fn from_hdr(img: &HdrImage, target_sun: f32) -> Self {
        Self::from_hdr_with(img, target_sun, Some(GROUND_ALBEDO), Some(WALL_ALBEDO))
    }

    /// Like [`EnvMap::from_hdr`] without the street walls.
    pub fn from_hdr_with_ground(img: &HdrImage, target_sun: f32, ground_albedo: Option<Vec3>) -> Self {
        Self::from_hdr_with(img, target_sun, ground_albedo, None)
    }

    /// Like [`EnvMap::from_hdr`]; `ground_albedo: None` keeps the photo's
    /// own lower hemisphere and `wall_albedo: None` its horizon band for the
    /// diffuse light (reflections and the visible sky always keep the photo).
    pub fn from_hdr_with(img: &HdrImage, target_sun: f32, ground_albedo: Option<Vec3>, wall_albedo: Option<Vec3>) -> Self {
        let (w, h) = (img.width as usize, img.height as usize);
        let px = |x: usize, y: usize| Vec3::from(img.rgb[y * w + x]);
        // Sun: brightest texel of a 4x downsample (robust to single hot pixels).
        let (mut best, mut best_l) = ((0, 0), 0.0);
        for y in (0..h / 2).step_by(4) {
            for x in (0..w).step_by(4) {
                let l = lum(px(x, y));
                if l > best_l {
                    best_l = l;
                    best = (x, y);
                }
            }
        }
        let sun_dir = dir_from_uv((best.0 as f32 + 0.5) / w as f32, (best.1 as f32 + 0.5) / h as f32);
        // Integrate the sun cone (3 deg) above the local sky level.
        let cos_cone = 3f32.to_radians().cos();
        let mut sky_ring = Vec3::ZERO;
        let mut ring_n = 0.0;
        let mut sun_e = Vec3::ZERO;
        // Solid angle of an equirect texel: (2pi / w) * (pi / h) * sin(theta).
        // (It used to be counted twice, which halved the sky and IBL next to
        // the sun: 13:1 instead of the photo's 6.5:1 on flat ground.)
        let dw = 2.0 * std::f32::consts::PI * std::f32::consts::PI / (w * h) as f32;
        let mut in_cone = vec![false; w * h];
        for y in 0..h / 2 {
            let theta = (y as f32 + 0.5) / h as f32 * std::f32::consts::PI;
            for x in 0..w {
                let d = dir_from_uv((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
                let c = d.dot(sun_dir);
                if c > cos_cone {
                    sun_e += px(x, y) * dw * theta.sin();
                    in_cone[y * w + x] = true;
                } else if c > (6f32).to_radians().cos() {
                    sky_ring += px(x, y);
                    ring_n += 1.0;
                }
            }
        }
        let ring = if ring_n > 0.0 { sky_ring / ring_n } else { Vec3::ZERO };
        // Remove the sky level counted inside the cone.
        let cone_solid = std::f32::consts::TAU * (1.0 - cos_cone);
        sun_e = (sun_e - ring * cone_solid).max(Vec3::ZERO);
        let scale = if lum(sun_e) > 1e-6 { target_sun / lum(sun_e) } else { 1.0 };
        // IBL radiance without the sun (it is drawn as shadowed direct light).
        let mut clamped: Vec<Vec3> = (0..w * h).map(|i| Vec3::from(img.rgb[i]) * scale).collect();
        for (i, c) in clamped.iter_mut().enumerate() {
            if in_cone[i] {
                *c = ring * scale;
            }
        }
        // SH9 projection of radiance (on a 128x64 subsample).
        let (sw, shh) = (128usize, 64usize);
        let sample = |u: f32, v: f32| clamped[((v * h as f32) as usize).min(h - 1) * w + ((u * w as f32) as usize).min(w - 1)];
        let mut wsum = 0.0;
        let mut sky_h = Vec3::ZERO;
        for y in 0..shh {
            let v = (y as f32 + 0.5) / shh as f32;
            let sin_t = (v * std::f32::consts::PI).sin();
            for x in 0..sw {
                let u = (x as f32 + 0.5) / sw as f32;
                sky_h += sample(u, v) * dir_from_uv(u, v).y.max(0.0) * sin_t;
                wsum += sin_t;
            }
        }
        let norm = 4.0 * std::f32::consts::PI / wsum;
        // Horizontal illuminance of sun plus sky lights the synthetic ground.
        let ground = ground_albedo.map(|a| (sun_e * scale * sun_dir.y.max(0.0) + sky_h * norm) * a / std::f32::consts::PI);
        // Facade seen in direction `d` (horizon band): it faces back toward
        // the viewer and is lit by the sun when it faces it, by half the sky
        // and by the ground.
        let sun_scaled = sun_e * scale;
        let sky_side = sky_h * norm * 0.5;
        let ground_e = ground.unwrap_or(Vec3::ZERO) * (std::f32::consts::PI * 0.5);
        let wall = |d: Vec3, albedo: Vec3| -> Vec3 {
            let n = -vec3(d.x, 0.0, d.z).normalize_or_zero();
            albedo * (sun_scaled * n.dot(sun_dir).max(0.0) + sky_side + ground_e) / std::f32::consts::PI
        };
        // Street width ~ building height: walls up to ~25 degrees, gone by ~35.
        let wall_weight = |dy: f32| -> f32 {
            if dy < 0.0 {
                return 0.0;
            }
            let t = ((dy - 0.42) / (0.57 - 0.42)).clamp(0.0, 1.0);
            1.0 - t * t * (3.0 - 2.0 * t)
        };
        let mut sh = [Vec3::ZERO; 9];
        for y in 0..shh {
            let v = (y as f32 + 0.5) / shh as f32;
            let sin_t = (v * std::f32::consts::PI).sin();
            for x in 0..sw {
                let u = (x as f32 + 0.5) / sw as f32;
                let d = dir_from_uv(u, v);
                let mut c = sample(u, v);
                if let Some(g) = ground {
                    c = c.lerp(g, (-d.y / 0.08).clamp(0.0, 1.0));
                }
                if let Some(a) = wall_albedo {
                    c = c.lerp(wall(d, a), wall_weight(d.y));
                }
                let basis = sh_basis(d);
                for k in 0..9 {
                    sh[k] += c * basis[k] * sin_t;
                }
            }
        }
        // Convolve with the clamped cosine lobe and divide by pi:
        // band factors pi, 2pi/3, pi/4 -> 1, 2/3, 1/4.
        let band = [1.0, 2.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0, 0.25, 0.25, 0.25, 0.25, 0.25];
        for k in 0..9 {
            sh[k] *= norm * band[k];
        }
        // Mips: level 0 keeps the sun for the visible sky; levels 1.. hold
        // the sun-less radiance prefiltered with the GGX lobe of increasing
        // roughness (split-sum, Karis 2013), so rough masonry reflects a
        // broad, dim lobe instead of a sharp mirror image of the sky.
        let mut mips = Vec::new();
        let level0: Vec<Vec3> = (0..w * h).map(|i| Vec3::from(img.rgb[i]) * scale).collect();
        let to_half = |v: &[Vec3]| v.iter().flat_map(|c| [f16(c.x), f16(c.y), f16(c.z), f16(1.0)]).collect::<Vec<u16>>();
        mips.push((w as u32, h as u32, to_half(&level0)));
        let chain = box_chain(w, h, clamped);
        let prefiltered = !std::env::var("FORJA_FX").is_ok_and(|v| v.split(',').any(|i| i.trim() == "-ggx"));
        // Reflection levels from half resolution down to 32 texels wide
        // (enough for a roughness-1 lobe, ~90 degrees wide).
        let levels = if prefiltered { chain.iter().skip(1).take_while(|(cw, _, _)| *cw >= REFLECTION_MIN_WIDTH).count().max(1) } else { 0 };
        if !prefiltered {
            for (cw, ch, data) in chain.iter().skip(1) {
                mips.push((*cw as u32, *ch as u32, to_half(data)));
            }
        }
        for k in 1..=levels {
            let rough = mip_roughness(k, levels);
            let (cw, ch, ref data) = chain[k.min(chain.len() - 1)];
            let out = if rough < 1e-3 { data.clone() } else { prefilter_ggx(&chain, cw, ch, rough) };
            mips.push((cw as u32, ch as u32, to_half(&out)));
        }
        EnvMap { mips, sh, sun_dir, sun_color: sun_e * scale, scale, prefiltered }
    }

    /// Irradiance / pi in direction `n` (what the shader computes).
    pub fn irradiance(&self, n: Vec3) -> Vec3 {
        let b = sh_basis(n);
        (0..9).map(|k| self.sh[k] * b[k]).sum::<Vec3>().max(Vec3::ZERO)
    }
}

pub fn sh_basis(d: Vec3) -> [f32; 9] {
    [
        0.282095,
        0.488603 * d.y,
        0.488603 * d.z,
        0.488603 * d.x,
        1.092548 * d.x * d.y,
        1.092548 * d.y * d.z,
        0.315392 * (3.0 * d.z * d.z - 1.0),
        1.092548 * d.x * d.z,
        0.546274 * (d.x * d.x - d.y * d.y),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_sky_irradiance_and_sun() {
        let (w, h) = (256u32, 128u32);
        let mut rgb = vec![[0.5f32; 3]; (w * h) as usize];
        // A bright sun at 45 degrees elevation.
        let sun = vec3(0.0, 1.0, -1.0).normalize();
        for y in 0..h {
            for x in 0..w {
                let d = dir_from_uv((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
                if d.dot(sun) > 0.9995 {
                    rgb[(y * w + x) as usize] = [5000.0; 3];
                }
            }
        }
        let env = EnvMap::from_hdr_with_ground(&HdrImage { width: w, height: h, rgb }, 10.0, None);
        assert!(env.sun_dir.dot(sun) > 0.99, "sun at {:?}", env.sun_dir);
        assert!((lum(env.sun_color) - 10.0).abs() < 0.5);
        // Uniform sky of radiance L gives irradiance/pi = L in every direction.
        let l = 0.5 * env.scale;
        for n in [Vec3::Y, Vec3::X, -Vec3::Y] {
            let e = env.irradiance(n);
            assert!((e.x - l).abs() / l < 0.08, "{n}: {e} vs {l}");
        }
    }

    /// The extracted sun must carry the disc's real irradiance, so that the
    /// sky keeps its measured ratio to the sun after normalisation.
    #[test]
    fn sun_irradiance_is_physical() {
        let (w, h) = (1024u32, 512u32);
        let (sky, disc, cos_disc) = (0.5f32, 10000.0f32, 0.9998f32);
        let sun = vec3(0.3, 0.8, -0.5).normalize();
        let mut rgb = vec![[sky; 3]; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                let d = dir_from_uv((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
                if d.dot(sun) > cos_disc {
                    rgb[(y * w + x) as usize] = [disc; 3];
                }
            }
        }
        let env = EnvMap::from_hdr_with(&HdrImage { width: w, height: h, rgb }, 14.0, Some(GROUND_ALBEDO), None);
        // Irradiance of the disc above the sky level: L * solid angle.
        let e_true = (disc - sky) * std::f32::consts::TAU * (1.0 - cos_disc);
        let scale_true = 14.0 / e_true;
        assert!((env.scale - scale_true).abs() / scale_true < 0.06, "scale {} vs {}", env.scale, scale_true);
        // Sky radiance relative to the sun survives: irradiance/pi of the
        // uniform sky is its radiance.
        let up = env.irradiance(Vec3::Y).x;
        assert!((up - sky * scale_true).abs() / (sky * scale_true) < 0.1, "{up} vs {}", sky * scale_true);
        // The synthetic ground reflects GROUND_ALBEDO of sun plus sky.
        let e_h = 14.0 * sun.y + std::f32::consts::PI * sky * scale_true;
        let down = env.irradiance(-Vec3::Y).x;
        let want = GROUND_ALBEDO.x * e_h / std::f32::consts::PI;
        assert!((down - want).abs() / want < 0.12, "{down} vs {want}");
    }

    /// The street walls of the horizon band bounce sunlight onto surfaces
    /// turned away from the sun, and barely change the light from above.
    #[test]
    fn street_walls_light_the_shaded_side() {
        let (w, h) = (512u32, 256u32);
        let sun = vec3(0.3, 0.8, -0.5).normalize();
        // Blue sky over a dark band of trees and parked cars (like the
        // street photo), a small bright sun.
        let mut rgb = vec![[0.3f32, 0.4, 0.6]; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                let d = dir_from_uv((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
                if d.dot(sun) > 0.999 {
                    rgb[(y * w + x) as usize] = [3000.0; 3];
                } else if d.y < 0.35 {
                    rgb[(y * w + x) as usize] = [0.08; 3];
                }
            }
        }
        let img = HdrImage { width: w, height: h, rgb };
        let (walls, open) = (EnvMap::from_hdr(&img, 14.0), EnvMap::from_hdr_with(&img, 14.0, Some(GROUND_ALBEDO), None));
        let away = -vec3(sun.x, 0.0, sun.z).normalize();
        let (wa, oa) = (walls.irradiance(away), open.irradiance(away));
        assert!(wa.x > oa.x * 1.2, "{wa} vs {oa}");
        // The bounce of sunlit plaster is warm.
        assert!(wa.x / wa.z > oa.x / oa.z, "{wa} vs {oa}");
        let (a, b) = (walls.irradiance(Vec3::Y).x, open.irradiance(Vec3::Y).x);
        assert!((a - b).abs() / b < 0.15, "{a} vs {b}");
    }

    #[test]
    fn ggx_prefilter_widens_and_dims_with_roughness() {
        // A bright 10-degree patch on the horizon of a dim sky.
        let (w, h) = (256usize, 128usize);
        let spot = vec3(0.0, 0.0, -1.0);
        let px: Vec<Vec3> = (0..w * h)
            .map(|i| {
                let d = dir_from_uv(((i % w) as f32 + 0.5) / w as f32, ((i / w) as f32 + 0.5) / h as f32);
                if d.dot(spot) > 10f32.to_radians().cos() { Vec3::splat(20.0) } else { Vec3::splat(0.2) }
            })
            .collect();
        let chain = box_chain(w, h, px);
        let at = |m: &[Vec3], mw: usize, mh: usize, d: Vec3| sample_level(&(mw, mh, m.to_vec()), d);
        let off = vec3(0.0, 35f32.to_radians().sin(), -35f32.to_radians().cos());
        let mut prev_peak = f32::MAX;
        let mut prev_ratio = 0.0;
        for rough in [0.2, 0.5, 0.9] {
            let m = prefilter_ggx(&chain, 64, 32, rough);
            let peak = at(&m, 64, 32, spot).x;
            let side = at(&m, 64, 32, off).x;
            // Rougher: lower peak and a broader lobe (more of the peak
            // reaches 35 degrees away). The absolute value 35 degrees off
            // need not grow: at r 0.9 the lobe spreads the patch's energy
            // well past 35 degrees.
            let ratio = side / peak;
            assert!(peak < prev_peak && ratio > prev_ratio, "r {rough}: peak {peak} side {side}");
            prev_peak = peak;
            prev_ratio = ratio;
            // Energy is conserved: the mean radiance stays that of the source.
            let mean = |v: &[Vec3], mw: usize, mh: usize| {
                let mut s = 0.0;
                let mut ws = 0.0;
                for (i, c) in v.iter().enumerate() {
                    let st = (((i / mw) as f32 + 0.5) / mh as f32 * std::f32::consts::PI).sin();
                    s += c.x * st;
                    ws += st;
                }
                s / ws
            };
            let m0 = mean(&chain[0].2, w, h);
            assert!((mean(&m, 64, 32) - m0).abs() / m0 < 0.12, "r {rough}: {} vs {m0}", mean(&m, 64, 32));
        }
        // A rough-0.9 lobe is broad: the spot reaches 35 degrees off at a
        // sizeable fraction of its peak (a box mip would be ~sky level).
        assert!(prev_ratio > 0.3, "side / peak {prev_ratio}");
        assert!(mip_roughness(1, 6) == 0.0 && (mip_roughness(6, 6) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn half_conversion() {
        assert_eq!(f16(1.0), 0x3c00);
        assert_eq!(f16(0.5), 0x3800);
        assert_eq!(f16(2.0), 0x4000);
    }
}
