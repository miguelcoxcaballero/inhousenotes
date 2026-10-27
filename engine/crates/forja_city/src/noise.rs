//! Small, fast, tileable noise used to generate weathering: integer hashes,
//! periodic value noise and fractal sums. Everything is deterministic and
//! periodic so generated textures repeat seamlessly.

/// 32-bit integer hash of a lattice point (PCG-style output permutation).
#[inline]
pub fn hash(x: i32, y: i32, seed: u32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841) ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^= h >> 15;
    h
}

/// Uniform in [0, 1) for a lattice point.
#[inline]
pub fn hashf(x: i32, y: i32, seed: u32) -> f32 {
    (hash(x, y, seed) >> 8) as f32 / (1u32 << 24) as f32
}

#[inline]
fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// Value noise in [0, 1) at `(x, y)` (lattice units), repeating every
/// `period` cells in both axes.
#[inline]
pub fn value(x: f32, y: f32, period: i32, seed: u32) -> f32 {
    let (fx, fy) = (x.floor(), y.floor());
    let (tx, ty) = (fade(x - fx), fade(y - fy));
    let p = period.max(1);
    let (x0, y0) = ((fx as i32).rem_euclid(p), (fy as i32).rem_euclid(p));
    let (x1, y1) = ((x0 + 1) % p, (y0 + 1) % p);
    let a = hashf(x0, y0, seed);
    let b = hashf(x1, y0, seed);
    let c = hashf(x0, y1, seed);
    let d = hashf(x1, y1, seed);
    let top = a + (b - a) * tx;
    let bot = c + (d - c) * tx;
    top + (bot - top) * ty
}

/// Fractal value noise in roughly [0, 1] on a unit square `(u, v)` that
/// tiles with period 1. `cells` is the lattice count of the first octave.
#[inline]
pub fn fbm(u: f32, v: f32, cells: i32, octaves: u32, seed: u32) -> f32 {
    let (mut sum, mut amp, mut norm, mut c) = (0.0, 1.0, 0.0, cells.max(1));
    for o in 0..octaves {
        sum += value(u * c as f32, v * c as f32, c, seed.wrapping_add(o * 7919)) * amp;
        norm += amp;
        amp *= 0.5;
        c *= 2;
    }
    sum / norm
}

/// Like [`fbm`] but stretched: `cx` cells across and `cy` cells down, so
/// features are elongated (rain streaks, wood grain). Tiles with period 1.
#[inline]
pub fn fbm_aniso(u: f32, v: f32, cx: i32, cy: i32, octaves: u32, seed: u32) -> f32 {
    let (mut sum, mut amp, mut norm) = (0.0, 1.0, 0.0);
    let (mut a, mut b) = (cx.max(1), cy.max(1));
    for o in 0..octaves {
        let s = seed.wrapping_add(o * 7919);
        let (x, y) = (u * a as f32, v * b as f32);
        // Separate periods per axis: wrap each lattice coordinate itself.
        let (fx, fy) = (x.floor(), y.floor());
        let (tx, ty) = (fade(x - fx), fade(y - fy));
        let (x0, y0) = ((fx as i32).rem_euclid(a), (fy as i32).rem_euclid(b));
        let (x1, y1) = ((x0 + 1) % a, (y0 + 1) % b);
        let top = hashf(x0, y0, s) + (hashf(x1, y0, s) - hashf(x0, y0, s)) * tx;
        let bot = hashf(x0, y1, s) + (hashf(x1, y1, s) - hashf(x0, y1, s)) * tx;
        sum += (top + (bot - top) * ty) * amp;
        norm += amp;
        amp *= 0.5;
        a *= 2;
        b *= 2;
    }
    sum / norm
}

#[inline]
pub fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[inline]
pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

#[inline]
pub fn linear_to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/// Wrapped difference on a unit torus (shortest signed offset).
#[inline]
pub fn wrap_delta(d: f32) -> f32 {
    d - d.round()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_noise_is_periodic_and_bounded() {
        for i in 0..50 {
            let (x, y) = (i as f32 * 0.37, i as f32 * 0.91);
            let a = value(x, y, 8, 3);
            let b = value(x + 8.0, y - 16.0, 8, 3);
            assert!((a - b).abs() < 1e-5);
            assert!((0.0..1.0).contains(&a));
        }
        let a = fbm(0.0, 0.3, 4, 5, 9);
        let b = fbm(1.0, 1.3, 4, 5, 9);
        assert!((a - b).abs() < 1e-4, "{a} {b}");
        let a = fbm_aniso(0.2, 0.0, 3, 9, 4, 2);
        let b = fbm_aniso(1.2, 1.0, 3, 9, 4, 2);
        assert!((a - b).abs() < 1e-4, "{a} {b}");
    }

    #[test]
    fn srgb_round_trip() {
        for i in 0..=20 {
            let c = i as f32 / 20.0;
            assert!((linear_to_srgb(srgb_to_linear(c)) - c).abs() < 1e-4);
        }
    }
}
