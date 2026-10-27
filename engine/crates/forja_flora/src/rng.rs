//! Deterministic random numbers and noise. Everything a tree is made of
//! derives from one seed, so the same `(species, seed, params)` always
//! produces the same asset on every platform (no `rand`, no float
//! transcendental differences in the integer paths).

use forja_math::Vec3;

/// SplitMix64: tiny, fast, good enough statistical quality for geometry.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        let mut r = Rng(seed ^ 0x9E37_79B9_7F4A_7C15);
        r.next_u64();
        r
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    pub fn f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    pub fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.f32()
    }

    /// Uniform in `[-1, 1)`.
    pub fn signed(&mut self) -> f32 {
        self.f32() * 2.0 - 1.0
    }

    /// `v ± var`, uniformly.
    pub fn vary(&mut self, v: f32, var: f32) -> f32 {
        v + var * self.signed()
    }

    pub fn chance(&mut self, p: f32) -> bool {
        self.f32() < p
    }

    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next_u64() % n as u64) as usize }
    }

    /// Approximately normal (sum of 4 uniforms), mean 0, sd ~1.
    pub fn normal(&mut self) -> f32 {
        (self.f32() + self.f32() + self.f32() + self.f32() - 2.0) * 1.732
    }

    /// Uniform direction on the unit sphere.
    pub fn unit(&mut self) -> Vec3 {
        let z = self.signed();
        let a = self.f32() * std::f32::consts::TAU;
        let r = (1.0 - z * z).max(0.0).sqrt();
        Vec3::new(r * a.cos(), z, r * a.sin())
    }

    /// Independent stream for a sub-part, so adding parts elsewhere does not
    /// reshuffle this one.
    pub fn fork(&self, tag: u64) -> Rng {
        Rng::new(self.0 ^ tag.wrapping_mul(0xD6E8_FEB8_6659_FD93))
    }
}

/// Stable 64-bit hash of a string (FNV-1a).
pub fn hash_str(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

#[inline]
fn hash3(x: i32, y: i32, z: i32, seed: u32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841) ^ (z as u32).wrapping_mul(0xcb1a_b31f) ^ seed.wrapping_mul(0x1656_67b1);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^ (h >> 15)
}

#[inline]
fn hf(h: u32) -> f32 {
    (h & 0x00ff_ffff) as f32 / 16_777_215.0
}

#[inline]
fn smooth(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// 3D value noise in `[0, 1]`.
pub fn value3(p: Vec3, seed: u32) -> f32 {
    let (fx, fy, fz) = (p.x.floor(), p.y.floor(), p.z.floor());
    let (ix, iy, iz) = (fx as i32, fy as i32, fz as i32);
    let (tx, ty, tz) = (smooth(p.x - fx), smooth(p.y - fy), smooth(p.z - fz));
    let l = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let c = |dx, dy, dz| hf(hash3(ix + dx, iy + dy, iz + dz, seed));
    l(
        l(l(c(0, 0, 0), c(1, 0, 0), tx), l(c(0, 1, 0), c(1, 1, 0), tx), ty),
        l(l(c(0, 0, 1), c(1, 0, 1), tx), l(c(0, 1, 1), c(1, 1, 1), tx), ty),
        tz,
    )
}

/// Fractal 3D noise in roughly `[-1, 1]`.
pub fn fbm3(p: Vec3, octaves: u32, seed: u32) -> f32 {
    let (mut sum, mut amp, mut f, mut norm) = (0.0, 1.0, 1.0, 0.0);
    for o in 0..octaves {
        sum += (value3(p * f, seed.wrapping_add(o * 31)) * 2.0 - 1.0) * amp;
        norm += amp;
        amp *= 0.5;
        f *= 2.03;
    }
    sum / norm
}

/// Smooth 3D vector noise (each component in `[-1, 1]`).
pub fn vec_noise(p: Vec3, seed: u32) -> Vec3 {
    Vec3::new(fbm3(p, 3, seed), fbm3(p, 3, seed.wrapping_add(101)), fbm3(p, 3, seed.wrapping_add(202)))
}

/// Tileable 2D noise on a `period`-cell lattice (texture space `[0,1)²`).
pub struct Tile2 {
    pub seed: u32,
}

impl Tile2 {
    /// Value noise in `[0, 1]`, periodic with `px` × `py` cells over the unit square.
    pub fn value(&self, u: f32, v: f32, px: i32, py: i32) -> f32 {
        let (x, y) = (u * px as f32, v * py as f32);
        let (fx, fy) = (x.floor(), y.floor());
        let (ix, iy) = (fx as i32, fy as i32);
        let (tx, ty) = (smooth(x - fx), smooth(y - fy));
        let c = |dx: i32, dy: i32| hf(hash3((ix + dx).rem_euclid(px), (iy + dy).rem_euclid(py), 0, self.seed));
        let a = c(0, 0) + (c(1, 0) - c(0, 0)) * tx;
        let b = c(0, 1) + (c(1, 1) - c(0, 1)) * tx;
        a + (b - a) * ty
    }

    /// Tileable fbm in roughly `[-1, 1]`; base frequency `px` × `py` cells.
    pub fn fbm(&self, u: f32, v: f32, px: i32, py: i32, octaves: u32) -> f32 {
        let (mut sum, mut amp, mut norm) = (0.0, 1.0, 0.0);
        let (mut fx, mut fy) = (px, py);
        for o in 0..octaves {
            let t = Tile2 { seed: self.seed.wrapping_add(o * 977) };
            sum += (t.value(u, v, fx, fy) * 2.0 - 1.0) * amp;
            norm += amp;
            amp *= 0.5;
            fx *= 2;
            fy *= 2;
        }
        sum / norm
    }

    /// Tileable Worley noise: `(F1, F2, cell id hash)` with `px` × `py`
    /// jittered points over the unit square (distances in cell units).
    pub fn worley(&self, u: f32, v: f32, px: i32, py: i32) -> (f32, f32, u32) {
        let (x, y) = (u * px as f32, v * py as f32);
        let (ix, iy) = (x.floor() as i32, y.floor() as i32);
        let (mut f1, mut f2, mut id) = (1e9f32, 1e9f32, 0u32);
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (cx, cy) = (ix + dx, iy + dy);
                let h = hash3(cx.rem_euclid(px), cy.rem_euclid(py), 7, self.seed);
                let jx = hf(h);
                let jy = hf(h.rotate_left(11) ^ 0x2545_f491);
                let (qx, qy) = (cx as f32 + jx, cy as f32 + jy);
                let d = ((qx - x).powi(2) + (qy - y).powi(2)).sqrt();
                if d < f1 {
                    f2 = f1;
                    f1 = d;
                    id = h;
                } else if d < f2 {
                    f2 = d;
                }
            }
        }
        (f1, f2, id)
    }
}

/// Random float in `[0,1)` from an id (cell colours etc.).
pub fn id_f(id: u32, k: u32) -> f32 {
    hf(hash3(id as i32, k as i32, 3, 0x51ed_270b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_is_deterministic_and_uniformish() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        let mut sum = 0.0;
        for _ in 0..10_000 {
            let x = a.f32();
            assert_eq!(x, b.f32());
            assert!((0.0..1.0).contains(&x));
            sum += x;
        }
        assert!((sum / 10_000.0 - 0.5).abs() < 0.02);
    }

    #[test]
    fn tile_noise_wraps() {
        let t = Tile2 { seed: 9 };
        for i in 0..20 {
            let v = i as f32 / 20.0;
            assert!((t.fbm(0.0, v, 4, 4, 4) - t.fbm(1.0, v, 4, 4, 4)).abs() < 1e-4);
            assert!((t.worley(v, 0.0, 5, 5).0 - t.worley(v, 1.0, 5, 5).0).abs() < 1e-4);
        }
    }
}
