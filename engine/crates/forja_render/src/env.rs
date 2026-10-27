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
        // Mips: level 0 keeps the sun for the visible sky, others use the
        // clamped map (blurred reflections).
        let mut mips = Vec::new();
        let level0: Vec<Vec3> = (0..w * h).map(|i| Vec3::from(img.rgb[i]) * scale).collect();
        let to_half = |v: &[Vec3]| v.iter().flat_map(|c| [f16(c.x), f16(c.y), f16(c.z), f16(1.0)]).collect::<Vec<u16>>();
        mips.push((w as u32, h as u32, to_half(&level0)));
        let (mut cw, mut ch, mut cur) = (w, h, clamped);
        while cw > 8 && ch > 4 {
            let (nw, nh) = (cw / 2, ch / 2);
            let mut next = vec![Vec3::ZERO; nw * nh];
            for y in 0..nh {
                for x in 0..nw {
                    next[y * nw + x] = (cur[2 * y * cw + 2 * x] + cur[2 * y * cw + 2 * x + 1] + cur[(2 * y + 1) * cw + 2 * x] + cur[(2 * y + 1) * cw + 2 * x + 1]) * 0.25;
                }
            }
            mips.push((nw as u32, nh as u32, to_half(&next)));
            cw = nw;
            ch = nh;
            cur = next;
        }
        EnvMap { mips, sh, sun_dir, sun_color: sun_e * scale, scale }
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
    fn half_conversion() {
        assert_eq!(f16(1.0), 0x3c00);
        assert_eq!(f16(0.5), 0x3800);
        assert_eq!(f16(2.0), 0x4000);
    }
}
