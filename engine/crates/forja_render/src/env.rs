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

impl EnvMap {
    pub fn from_hdr(img: &HdrImage, target_sun: f32) -> Self {
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
        let dw = 4.0 * std::f32::consts::PI * std::f32::consts::PI / (w * h) as f32 / 2.0;
        let mut in_cone = vec![false; w * h];
        for y in 0..h / 2 {
            let theta = (y as f32 + 0.5) / h as f32 * std::f32::consts::PI;
            for x in 0..w {
                let d = dir_from_uv((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
                let c = d.dot(sun_dir);
                if c > cos_cone {
                    sun_e += px(x, y) * dw * theta.sin() * 2.0;
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
        let mut sh = [Vec3::ZERO; 9];
        let (sw, shh) = (128usize, 64usize);
        let mut wsum = 0.0;
        for y in 0..shh {
            let v = (y as f32 + 0.5) / shh as f32;
            let sin_t = (v * std::f32::consts::PI).sin();
            for x in 0..sw {
                let u = (x as f32 + 0.5) / sw as f32;
                let d = dir_from_uv(u, v);
                let c = clamped[((v * h as f32) as usize).min(h - 1) * w + ((u * w as f32) as usize).min(w - 1)];
                let basis = sh_basis(d);
                for k in 0..9 {
                    sh[k] += c * basis[k] * sin_t;
                }
                wsum += sin_t;
            }
        }
        let norm = 4.0 * std::f32::consts::PI / wsum;
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
        let env = EnvMap::from_hdr(&HdrImage { width: w, height: h, rgb }, 10.0);
        assert!(env.sun_dir.dot(sun) > 0.99, "sun at {:?}", env.sun_dir);
        assert!((lum(env.sun_color) - 10.0).abs() < 0.5);
        // Uniform sky of radiance L gives irradiance/pi = L in every direction.
        let l = 0.5 * env.scale;
        for n in [Vec3::Y, Vec3::X, -Vec3::Y] {
            let e = env.irradiance(n);
            assert!((e.x - l).abs() / l < 0.08, "{n}: {e} vs {l}");
        }
    }

    #[test]
    fn half_conversion() {
        assert_eq!(f16(1.0), 0x3c00);
        assert_eq!(f16(0.5), 0x3800);
        assert_eq!(f16(2.0), 0x4000);
    }
}
