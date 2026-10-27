//! Precomputed BRDF integrals for image-based lighting.
//!
//! The split-sum approximation (Karis 2013, "Real Shading in Unreal Engine
//! 4") factors the environment specular into prefiltered radiance (see
//! [`crate::env`]) times the directional albedo of the specular BRDF,
//! `E(v) = F0 * A + B`. This module integrates `A` and `B` for GGX with the
//! height-correlated Smith visibility used by the shader, giving a small LUT
//! indexed by (n.v, perceptual roughness). `A + B` is also the single
//! scattering energy `Ess` that the multiple-scattering compensation
//! (Fdez-Aguera 2019, JCGT 8(1); Kulla & Conty 2017) restores.

use crate::env::{ggx_half, hammersley};

/// LUT resolution (n.v by roughness).
pub const DFG_SIZE: usize = 32;

fn v_smith(nv: f32, nl: f32, a: f32) -> f32 {
    let a2 = a * a;
    let gv = nl * (nv * nv * (1.0 - a2) + a2).sqrt();
    let gl = nv * (nl * nl * (1.0 - a2) + a2).sqrt();
    0.5 / (gv + gl).max(1e-7)
}

/// Split-sum terms (A, B) for `nv` and perceptual roughness `rough`.
pub fn integrate_dfg(nv: f32, rough: f32, samples: u32) -> (f32, f32) {
    let a = (rough * rough).max(1e-3);
    let nv = nv.max(1e-4);
    let v = forja_math::vec3((1.0 - nv * nv).sqrt(), 0.0, nv);
    let (mut sa, mut sb) = (0.0, 0.0);
    for i in 0..samples {
        let (u1, u2) = hammersley(i, samples);
        let h = ggx_half(u1, u2, a);
        let vh = v.dot(h);
        let l = h * (2.0 * vh) - v;
        let (nl, nh) = (l.z, h.z);
        if nl > 0.0 && vh > 0.0 {
            // pdf = D nh / (4 vh): f * nl / pdf = V * 4 nl vh / nh.
            let g = v_smith(nv, nl, a) * 4.0 * nl * vh / nh;
            let fc = (1.0 - vh).powi(5);
            sa += (1.0 - fc) * g;
            sb += fc * g;
        }
    }
    (sa / samples as f32, sb / samples as f32)
}

/// The LUT as RGBA16F bits: rows by roughness, columns by n.v (texel
/// centres), r = A, g = B.
pub fn dfg_lut() -> Vec<u16> {
    let n = DFG_SIZE;
    let mut out = Vec::with_capacity(n * n * 4);
    for y in 0..n {
        let rough = (y as f32 + 0.5) / n as f32;
        for x in 0..n {
            let nv = (x as f32 + 0.5) / n as f32;
            let (a, b) = integrate_dfg(nv, rough, 256);
            out.extend([half(a), half(b), half(0.0), half(1.0)]);
        }
    }
    out
}

fn half(v: f32) -> u16 {
    let v = v.clamp(0.0, 65504.0);
    if v < 6.1e-5 {
        return (v / 5.96e-8) as u16;
    }
    let bits = v.to_bits();
    let exp = ((bits >> 23) & 0xff) as i32 - 127 + 15;
    let mant = ((bits >> 13) & 0x3ff) + ((bits >> 12) & 1); // round half up
    (((exp as u32) << 10) + mant) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smooth_surfaces_reflect_all_energy() {
        // A mirror at normal incidence: A + B = 1 (no energy lost).
        let (a, b) = integrate_dfg(1.0, 0.05, 512);
        assert!((a + b - 1.0).abs() < 0.02, "{a} + {b}");
    }

    #[test]
    fn rough_surfaces_lose_single_scattering_energy() {
        // Rough GGX loses energy to multiple scattering, most at grazing
        // angles (Kulla & Conty 2017): this is what the compensation adds back.
        let (a, b) = integrate_dfg(0.5, 1.0, 512);
        let e_rough = a + b;
        assert!(e_rough > 0.4 && e_rough < 0.9, "Ess {e_rough}");
        let (a2, b2) = integrate_dfg(0.5, 0.3, 512);
        assert!(a2 + b2 > e_rough);
        // nv 0.5, rough 0.5 (alpha 0.25): little energy is lost
        // (Ess ~0.86 for height-correlated Smith; Karis' analytic fit, the
        // shader's `-ms` fallback, gives A ~0.70 with its separable G).
        let (a3, b3) = integrate_dfg(0.5, 0.5, 512);
        assert!(a3 > 0.7 && a3 < 0.92 && b3 > 0.005 && b3 < 0.06, "{a3} {b3}");
    }
}
