//! Anti-tiling, weathered "macro" versions of scanned PBR sets.
//!
//! A scan repeats every 1–5 m, which shows as a regular pattern on a 20 m
//! facade or a long street, and it is spotless. [`build`] turns one scan
//! into a larger texture covering `tiles × tiles` repeats at the same pixel
//! budget (detail drops to what a street-level camera resolves anyway):
//!
//! * **Remix**: stochastic materials (stucco, asphalt, granite) are
//!   re-assembled from randomly offset and rotated copies of the scan in
//!   a jittered cell grid, blended across cell borders with a
//!   variance-preserving blend, so distinctive features (cracks, pebbles)
//!   no longer repeat on a grid.
//! * **Macro variation**: low-frequency albedo mottling and warm/cool
//!   drift, vertical rain-wash streaks, repair patches with a different
//!   tone, roughness and smoother trowel finish, stains, oil spots,
//!   hairline or tar-sealed cracks and flattened chewing gum.
//! * **Calibration**: chroma and mean albedo can be pulled to physically
//!   plausible values, and roughness remapped, so materials do not read as
//!   uniformly glossy or uniformly matte.
//!
//! Everything wraps at the texture border, so the result still tiles.

use forja_assets::{PbrSet, TextureData};
use rayon::prelude::*;

use crate::geom::Rng;
use crate::noise::{fbm, fbm_aniso, linear_to_srgb, smoothstep, srgb_to_linear, wrap_delta};

/// How to turn a scan into a macro texture. Distances are in meters of the
/// final texture (`meters` = one repeat of the result).
#[derive(Clone, Debug)]
pub struct Recipe {
    /// Scan repeats per side in the result.
    pub tiles: u32,
    /// Output size in pixels (rounded down to a multiple of `tiles`).
    pub size: u32,
    /// World size of one repeat of the result.
    pub meters: f32,
    /// Re-assemble from offset copies (stochastic materials only: a remix
    /// would break the grid of bricks or paving).
    pub remix: bool,
    /// Allow 180° rotated copies in the remix.
    pub rotate: bool,
    pub seed: u32,
    /// Chroma multiplier around the luminance (1 = keep).
    pub saturation: f32,
    /// Target mean linear luminance of the color map.
    pub mean: Option<f32>,
    /// Low-frequency albedo variation (fraction) and its feature size.
    pub mottle: f32,
    pub mottle_m: f32,
    /// Warm/cool drift amplitude.
    pub hue: f32,
    /// Vertical rain-wash streak darkening.
    pub streaks: f32,
    /// Roughness = src * rough_mul + rough_add + variation.
    pub rough_mul: f32,
    pub rough_add: f32,
    pub rough_var: f32,
    /// Repair patches: count, albedo shift (±), roughness shift, size range.
    pub patches: u32,
    pub patch_value: f32,
    pub patch_rough: f32,
    pub patch_m: [f32; 2],
    /// Smooth (re-trowelled) patches instead of keeping the scan relief.
    pub patch_flat: f32,
    /// Dark irregular stains (damp, soot, spills).
    pub stains: u32,
    pub stain_dark: f32,
    pub stain_m: [f32; 2],
    /// Oil / grease spots: darker and glossier.
    pub oil: u32,
    /// Cracks: count, width (m), darkness, roughness inside the crack.
    pub cracks: u32,
    pub crack_w: f32,
    pub crack_dark: f32,
    pub crack_rough: f32,
    /// Flattened chewing gum spots.
    pub gum: u32,
    /// Normal map strength multiplier.
    pub normal: f32,
    /// Grime packed into joints and pores: albedo darkening where the
    /// scan's ambient occlusion is low (0 = off).
    pub cavity: f32,
    /// Per-unit tone variation of tiled materials (paving tiles, slabs,
    /// cobbles, bricks): each piece between joints gets its own value, and
    /// a few are replacements from another batch (0 = off).
    pub tile_tone: f32,
    /// Fraction of pixels (lowest AO) that are joints between pieces.
    pub joint_q: f32,
}

impl Default for Recipe {
    fn default() -> Self {
        Recipe {
            tiles: 1,
            size: 2048,
            meters: 1.0,
            remix: false,
            rotate: false,
            seed: 1,
            saturation: 1.0,
            mean: None,
            mottle: 0.0,
            mottle_m: 3.0,
            hue: 0.0,
            streaks: 0.0,
            rough_mul: 1.0,
            rough_add: 0.0,
            rough_var: 0.0,
            patches: 0,
            patch_value: 0.0,
            patch_rough: 0.0,
            patch_m: [0.5, 1.5],
            patch_flat: 0.0,
            stains: 0,
            stain_dark: 0.0,
            stain_m: [0.3, 1.2],
            oil: 0,
            cracks: 0,
            crack_w: 0.004,
            crack_dark: 0.4,
            crack_rough: 0.0,
            gum: 0,
            normal: 1.0,
            cavity: 0.0,
            tile_tone: 0.0,
            joint_q: 0.1,
        }
    }
}

/// Linear float maps of one material at a square size.
struct Maps {
    n: u32,
    color: Vec<[f32; 3]>,
    normal: Vec<[f32; 3]>,
    orm: Vec<[f32; 3]>,
}

/// Area-average resample of an RGBA8 texture to `n × n`, decoded by `dec`.
fn resample(t: &TextureData, n: u32, dec: impl Fn([u8; 3]) -> [f32; 3] + Sync) -> Vec<[f32; 3]> {
    let (sw, sh) = (t.width as usize, t.height as usize);
    let ranges = |s: usize| -> Vec<(usize, usize)> {
        (0..n as usize)
            .map(|i| {
                let a = i * s / n as usize;
                let b = ((i + 1) * s / n as usize).max(a + 1).min(s);
                (a.min(s - 1), b)
            })
            .collect()
    };
    let (rx, ry) = (ranges(sw), ranges(sh));
    let mut out = vec![[0.0f32; 3]; (n * n) as usize];
    out.par_chunks_mut(n as usize).enumerate().for_each(|(y, row)| {
        let (y0, y1) = ry[y];
        for (x, px) in row.iter_mut().enumerate() {
            let (x0, x1) = rx[x];
            let mut acc = [0.0f32; 3];
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let i = (sy * sw + sx) * 4;
                    let c = dec([t.rgba8[i], t.rgba8[i + 1], t.rgba8[i + 2]]);
                    acc[0] += c[0];
                    acc[1] += c[1];
                    acc[2] += c[2];
                }
            }
            let k = 1.0 / ((y1 - y0) * (x1 - x0)) as f32;
            *px = [acc[0] * k, acc[1] * k, acc[2] * k];
        }
    });
    out
}

fn decode_srgb(c: [u8; 3]) -> [f32; 3] {
    c.map(|v| srgb_to_linear(v as f32 / 255.0))
}

fn decode_unorm(c: [u8; 3]) -> [f32; 3] {
    c.map(|v| v as f32 / 255.0)
}

fn decode_normal(c: [u8; 3]) -> [f32; 3] {
    c.map(|v| v as f32 / 255.0 * 2.0 - 1.0)
}

fn lum(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn mean3(v: &[[f32; 3]]) -> [f32; 3] {
    let mut m = [0.0f64; 3];
    for c in v {
        for k in 0..3 {
            m[k] += c[k] as f64;
        }
    }
    let n = v.len().max(1) as f64;
    [(m[0] / n) as f32, (m[1] / n) as f32, (m[2] / n) as f32]
}

fn tile_maps(set: &PbrSet, t: u32) -> Maps {
    let color = resample(&set.color, t, decode_srgb);
    let normal = match &set.normal {
        Some(nm) => resample(nm, t, decode_normal),
        None => vec![[0.0, 0.0, 1.0]; (t * t) as usize],
    };
    let orm = match &set.orm {
        Some(o) => resample(o, t, decode_unorm),
        None => vec![[1.0, 0.85, 0.0]; (t * t) as usize],
    };
    Maps { n: t, color, normal, orm }
}

/// Jittered cell grid with a random source offset and rotation per cell.
struct Cells {
    n: i32,
    centers: Vec<[f32; 2]>,
    offsets: Vec<(u32, u32, bool)>,
}

impl Cells {
    fn new(n: i32, tile: u32, rotate: bool, rng: &mut Rng) -> Self {
        let mut centers = Vec::with_capacity((n * n) as usize);
        let mut offsets = Vec::with_capacity((n * n) as usize);
        for j in 0..n {
            for i in 0..n {
                centers.push([i as f32 + rng.range(0.15, 0.85), j as f32 + rng.range(0.15, 0.85)]);
                offsets.push((rng.below(tile as usize) as u32, rng.below(tile as usize) as u32, rotate && rng.chance(0.5)));
            }
        }
        Cells { n, centers, offsets }
    }

    /// Nearest two cells of a point in cell units: (index, distance).
    fn nearest2(&self, q: [f32; 2]) -> ((usize, f32), (usize, f32)) {
        let (ci, cj) = (q[0].floor() as i32, q[1].floor() as i32);
        let mut best = [(0usize, f32::MAX); 2];
        for dj in -1..=1 {
            for di in -1..=1 {
                let (i, j) = (ci + di, cj + dj);
                let idx = (j.rem_euclid(self.n) * self.n + i.rem_euclid(self.n)) as usize;
                let c = self.centers[idx];
                // Center in the unwrapped neighbourhood.
                let cx = c[0] + (i - i.rem_euclid(self.n)) as f32;
                let cy = c[1] + (j - j.rem_euclid(self.n)) as f32;
                let d = ((q[0] - cx).powi(2) + (q[1] - cy).powi(2)).sqrt();
                if d < best[0].1 {
                    best[1] = best[0];
                    best[0] = (idx, d);
                } else if d < best[1].1 {
                    best[1] = (idx, d);
                }
            }
        }
        (best[0], best[1])
    }
}

/// Re-assembles `src` (one scan repeat, `src.n` px) into `tiles × tiles`
/// repeats from randomly offset copies; plain tiling when `remix` is off.
fn assemble(src: &Maps, tiles: u32, remix: bool, rotate: bool, seed: u32) -> Maps {
    let t = src.n;
    let n = t * tiles;
    let mut color = vec![[0.0f32; 3]; (n * n) as usize];
    let mut normal = vec![[0.0f32; 3]; (n * n) as usize];
    let mut orm = vec![[0.0f32; 3]; (n * n) as usize];
    let mut rng = Rng::new(seed as u64 ^ 0x5eed);
    let cells = Cells::new((tiles * 2) as i32, t, rotate, &mut rng);
    let cell_px = n as f32 / cells.n as f32;
    let (mc, mo) = (mean3(&src.color), mean3(&src.orm));
    let fetch = |x: u32, y: u32, off: (u32, u32, bool)| -> usize {
        let (mut sx, mut sy) = ((x + off.0) % t, (y + off.1) % t);
        if off.2 {
            sx = t - 1 - sx;
            sy = t - 1 - sy;
        }
        (sy * t + sx) as usize
    };
    color
        .par_chunks_mut(n as usize)
        .zip(normal.par_chunks_mut(n as usize))
        .zip(orm.par_chunks_mut(n as usize))
        .enumerate()
        .for_each(|(y, ((crow, nrow), orow))| {
            let y = y as u32;
            for x in 0..n {
                let xi = x as usize;
                if !remix {
                    let i = ((y % t) * t + x % t) as usize;
                    crow[xi] = src.color[i];
                    nrow[xi] = src.normal[i];
                    orow[xi] = src.orm[i];
                    continue;
                }
                let q = [(x as f32 + 0.5) / cell_px, (y as f32 + 0.5) / cell_px];
                let ((a, da), (b, db)) = cells.nearest2(q);
                // Blend weight of the second cell: 0.5 on the border, 0 a
                // short way inside the first cell.
                let w = 0.5 * (1.0 - smoothstep(0.0, 0.22, db - da));
                let (oa, ob) = (cells.offsets[a], cells.offsets[b]);
                let (ia, ib) = (fetch(x, y, oa), fetch(x, y, ob));
                // Variance-preserving blend keeps the grain contrast.
                let k = 1.0 / ((1.0 - w).powi(2) + w * w).sqrt();
                let mix = |p: [f32; 3], q: [f32; 3], m: [f32; 3]| -> [f32; 3] {
                    [0, 1, 2].map(|c| (m[c] + ((1.0 - w) * p[c] + w * q[c] - m[c]) * k).clamp(0.0, 1.0))
                };
                crow[xi] = mix(src.color[ia], src.color[ib], mc);
                orow[xi] = mix(src.orm[ia], src.orm[ib], mo);
                let (na, nb) = (rot_normal(src.normal[ia], oa.2), rot_normal(src.normal[ib], ob.2));
                let nx = ((1.0 - w) * na[0] + w * nb[0]) * k;
                let ny = ((1.0 - w) * na[1] + w * nb[1]) * k;
                nrow[xi] = [nx, ny, (1.0 - nx * nx - ny * ny).max(0.0).sqrt()];
            }
        });
    Maps { n, color, normal, orm }
}

fn rot_normal(n: [f32; 3], rot: bool) -> [f32; 3] {
    if rot { [-n[0], -n[1], n[2]] } else { n }
}

/// A soft-edged rectangle or blob on the unit torus.
#[derive(Clone, Copy)]
struct Spot {
    c: [f32; 2],
    half: [f32; 2],
    /// 0 = ellipse, 1 = rectangle.
    rect: bool,
    value: f32,
    rough: f32,
    seed: u32,
}

impl Spot {
    /// Coverage in [0, 1] at `(u, v)`; edges are broken up by noise.
    fn cover(&self, u: f32, v: f32, edge: f32) -> f32 {
        let (dx, dy) = (wrap_delta(u - self.c[0]), wrap_delta(v - self.c[1]));
        if dx.abs() > self.half[0] * 1.6 || dy.abs() > self.half[1] * 1.6 {
            return 0.0;
        }
        let (nx, ny) = (dx / self.half[0], dy / self.half[1]);
        let d = if self.rect { nx.abs().max(ny.abs()) } else { (nx * nx + ny * ny).sqrt() };
        let jag = fbm(u, v, 24, 3, self.seed) - 0.5;
        1.0 - smoothstep(1.0 - edge, 1.0, d + jag * 0.5)
    }
}

/// Rasterizes a soft line segment into `mask` (n × n, wrapping).
fn stroke(mask: &mut [f32], n: u32, a: [f32; 2], b: [f32; 2], w: f32, strength: f32) {
    let (minx, maxx) = (a[0].min(b[0]) - w - 1.0, a[0].max(b[0]) + w + 1.0);
    let (miny, maxy) = (a[1].min(b[1]) - w - 1.0, a[1].max(b[1]) + w + 1.0);
    let (bx, by) = (b[0] - a[0], b[1] - a[1]);
    let len2 = (bx * bx + by * by).max(1e-6);
    for y in miny.floor() as i32..=maxy.ceil() as i32 {
        for x in minx.floor() as i32..=maxx.ceil() as i32 {
            let (px, py) = (x as f32 + 0.5 - a[0], y as f32 + 0.5 - a[1]);
            let h = ((px * bx + py * by) / len2).clamp(0.0, 1.0);
            let d = ((px - bx * h).powi(2) + (py - by * h).powi(2)).sqrt();
            let c = (w * 0.5 + 0.5 - d).clamp(0.0, 1.0) * strength;
            if c > 0.0 {
                let i = (y.rem_euclid(n as i32) as u32 * n + x.rem_euclid(n as i32) as u32) as usize;
                mask[i] = mask[i].max(c);
            }
        }
    }
}

/// Branching random-walk cracks into `mask`.
fn crack_mask(n: u32, count: u32, width_px: f32, px_per_m: f32, rng: &mut Rng) -> Vec<f32> {
    let mut mask = vec![0.0f32; (n * n) as usize];
    for _ in 0..count {
        let mut p = [rng.f() * n as f32, rng.f() * n as f32];
        let mut ang = rng.range(0.0, std::f32::consts::TAU);
        let len_m = rng.range(0.8, 3.5);
        let step = (0.06 * px_per_m).max(2.0);
        let steps = (len_m * px_per_m / step) as usize;
        let mut branches: Vec<([f32; 2], f32, usize)> = Vec::new();
        for s in 0..steps {
            ang += rng.range(-0.55, 0.55);
            let q = [p[0] + ang.cos() * step, p[1] + ang.sin() * step];
            let fade = 1.0 - (s as f32 / steps as f32).powi(3);
            stroke(&mut mask, n, p, q, width_px * (0.6 + 0.6 * fade), 0.6 + 0.4 * fade);
            if rng.chance(0.06) {
                branches.push((q, ang + rng.range(0.6, 1.2) * if rng.chance(0.5) { 1.0 } else { -1.0 }, steps / 3));
            }
            p = q;
        }
        for (mut p, mut a, k) in branches {
            for s in 0..k {
                a += rng.range(-0.5, 0.5);
                let q = [p[0] + a.cos() * step, p[1] + a.sin() * step];
                stroke(&mut mask, n, p, q, width_px * 0.7, 0.8 * (1.0 - s as f32 / k as f32));
                p = q;
            }
        }
    }
    mask
}

/// Flattened chewing gum: small dark discs, a few elongated by shoes.
fn gum_mask(n: u32, count: u32, px_per_m: f32, rng: &mut Rng) -> Vec<f32> {
    let mut mask = vec![0.0f32; (n * n) as usize];
    for _ in 0..count {
        let c = [rng.f() * n as f32, rng.f() * n as f32];
        let r = rng.range(0.006, 0.016) * px_per_m;
        let smear = if rng.chance(0.2) { rng.range(1.0, 2.2) } else { 1.0 };
        let a = rng.range(0.0, std::f32::consts::TAU);
        let s = rng.range(0.55, 1.0);
        let b = [c[0] + a.cos() * r * (smear - 1.0), c[1] + a.sin() * r * (smear - 1.0)];
        stroke(&mut mask, n, c, b, r * 2.0, s);
    }
    mask
}

/// Value at quantile `q` of `v` (from a strided sample, plenty for maps).
fn quantile(v: &[f32], q: f32) -> f32 {
    let step = (v.len() / 65536).max(1);
    let mut s: Vec<f32> = v.iter().step_by(step).copied().collect();
    s.sort_by(f32::total_cmp);
    s[((s.len() - 1) as f32 * q.clamp(0.0, 1.0)) as usize]
}

/// Labels the pieces of a tiled material: connected regions (wrapping at
/// the border) of pixels whose AO is at least `thr`. Joints, pores and
/// fragments too small to be a piece are filled from their neighbours;
/// returns the label per pixel and the number of labels.
fn label_pieces(ao: &[f32], n: u32, thr: f32) -> (Vec<u32>, usize) {
    let n = n as usize;
    let solid: Vec<bool> = ao.iter().map(|&a| a >= thr).collect();
    let mut parent: Vec<u32> = (0..(n * n) as u32).collect();
    fn find(p: &mut [u32], mut x: u32) -> u32 {
        while p[x as usize] != x {
            p[x as usize] = p[p[x as usize] as usize];
            x = p[x as usize];
        }
        x
    }
    for y in 0..n {
        for x in 0..n {
            let i = y * n + x;
            if !solid[i] {
                continue;
            }
            for j in [y * n + (x + 1) % n, ((y + 1) % n) * n + x] {
                if solid[j] {
                    let (a, b) = (find(&mut parent, i as u32), find(&mut parent, j as u32));
                    if a != b {
                        parent[a.max(b) as usize] = a.min(b);
                    }
                }
            }
        }
    }
    let mut size = vec![0u32; n * n];
    let mut root = vec![u32::MAX; n * n];
    for i in 0..n * n {
        if solid[i] {
            let r = find(&mut parent, i as u32);
            root[i] = r;
            size[r as usize] += 1;
        }
    }
    // A piece covers at least ~0.02 % of the texture.
    let min = ((n * n) as f32 * 2e-4) as u32;
    let mut id = vec![u32::MAX; n * n];
    let mut count = 0usize;
    let mut labels = vec![u32::MAX; n * n];
    for i in 0..n * n {
        let r = root[i];
        if r == u32::MAX || size[r as usize] < min {
            continue;
        }
        if id[r as usize] == u32::MAX {
            id[r as usize] = count as u32;
            count += 1;
        }
        labels[i] = id[r as usize];
    }
    // Grow pieces over joints and pores so every pixel has a tone.
    for _ in 0..6 {
        let prev = labels.clone();
        for y in 0..n {
            for x in 0..n {
                let i = y * n + x;
                if prev[i] != u32::MAX {
                    continue;
                }
                for j in [y * n + (x + 1) % n, y * n + (x + n - 1) % n, ((y + 1) % n) * n + x, ((y + n - 1) % n) * n + x] {
                    if prev[j] != u32::MAX {
                        labels[i] = prev[j];
                        break;
                    }
                }
            }
        }
    }
    (labels, count)
}

/// Builds the macro version of `set` following `r`.
pub fn build(set: &PbrSet, r: &Recipe) -> PbrSet {
    let tiles = r.tiles.max(1);
    let t = (r.size / tiles).max(8);
    let src = tile_maps(set, t);
    let mut m = assemble(&src, tiles, r.remix, r.rotate, r.seed);
    let n = m.n;
    let px_per_m = n as f32 / r.meters.max(0.01);
    let mut rng = Rng::new(r.seed as u64);

    // Global calibration: chroma and mean albedo.
    let mean_l = lum(mean3(&m.color)).max(1e-4);
    let gain = r.mean.map_or(1.0, |t| t / mean_l);
    let sat = r.saturation;

    let spot = |rng: &mut Rng, size: [f32; 2], rect: bool, value: f32, rough: f32| {
        let w = rng.range(size[0], size[1]) / r.meters;
        let h = if rect { rng.range(size[0], size[1]) / r.meters } else { w * rng.range(0.6, 1.4) };
        Spot { c: [rng.f(), rng.f()], half: [w * 0.5, h * 0.5], rect, value, rough, seed: rng.next_u32() }
    };
    let patches: Vec<Spot> = (0..r.patches)
        .map(|_| {
            let sgn = if rng.chance(0.5) { 1.0 } else { -1.0 };
            let v = sgn * r.patch_value * rng.range(0.5, 1.0);
            let pr = r.patch_rough * rng.range(0.5, 1.0);
            spot(&mut rng, r.patch_m, true, v, pr)
        })
        .collect();
    let stains: Vec<Spot> = (0..r.stains)
        .map(|_| {
            let d = r.stain_dark * rng.range(0.4, 1.0);
            spot(&mut rng, r.stain_m, false, d, 0.0)
        })
        .collect();
    let oils: Vec<Spot> = (0..r.oil)
        .map(|_| {
            let d = rng.range(0.25, 0.5);
            spot(&mut rng, [0.15, 0.7], false, d, 0.0)
        })
        .collect();
    // Joints and pores from the scan's AO; per-piece tones.
    let ao: Vec<f32> = m.orm.iter().map(|o| o[0]).collect();
    let (ao_lo, ao_hi) = (quantile(&ao, 0.02), quantile(&ao, 0.3));
    let pieces = (r.tile_tone > 0.0).then(|| {
        let (labels, count) = label_pieces(&ao, n, quantile(&ao, r.joint_q));
        let mut prng = Rng::new(r.seed as u64 ^ 0x711e);
        let tones: Vec<(f32, f32)> = (0..count)
            .map(|_| {
                let v = 1.0 + r.tile_tone * (2.0 * prng.f() - 1.0);
                // A few replacement pieces from another batch.
                if prng.chance(0.05) { (v * (1.0 + r.tile_tone * if prng.chance(0.5) { 2.5 } else { -2.5 }), prng.range(-0.06, 0.06)) } else { (v, 0.0) }
            })
            .collect();
        (labels, tones)
    });
    let cracks = (r.cracks > 0).then(|| crack_mask(n, r.cracks, r.crack_w * px_per_m, px_per_m, &mut rng));
    let gum = (r.gum > 0).then(|| gum_mask(n, r.gum, px_per_m, &mut rng));
    let cells = |m_feature: f32| ((r.meters / m_feature).round() as i32).max(1);
    let (mott_c, hue_c, rough_c) = (cells(r.mottle_m), cells(r.mottle_m * 1.7), cells(r.mottle_m * 0.6));
    let streak_cx = cells(0.35);
    let streak_cy = cells(5.0);
    let seed = r.seed;
    // Low-frequency fields (value, hue, roughness, streaks) on a quarter
    // resolution grid, sampled bilinearly: 16x cheaper than per pixel.
    let fnn = (n / 4).max(4);
    let mut fields = vec![[0.0f32; 4]; (fnn * fnn) as usize];
    fields.par_chunks_mut(fnn as usize).enumerate().for_each(|(y, row)| {
        let v = (y as f32 + 0.5) / fnn as f32;
        for (x, f) in row.iter_mut().enumerate() {
            let u = (x as f32 + 0.5) / fnn as f32;
            let mut val = 1.0;
            if r.mottle > 0.0 {
                val *= 1.0 + r.mottle * (2.0 * fbm(u, v, mott_c, 4, seed ^ 0x11) - 1.0) * 1.4;
            }
            if r.streaks > 0.0 {
                let s = fbm_aniso(u, v, streak_cx, streak_cy, 3, seed ^ 0x33);
                val *= 1.0 - r.streaks * smoothstep(0.5, 0.78, s);
            }
            let hue = if r.hue > 0.0 { (2.0 * fbm(u, v, hue_c, 3, seed ^ 0x22) - 1.0) * r.hue } else { 0.0 };
            let rough = if r.rough_var > 0.0 { r.rough_var * (2.0 * fbm(u, v, rough_c, 4, seed ^ 0x44) - 1.0) * 1.4 } else { 0.0 };
            *f = [val, hue, rough, 0.0];
        }
    });
    let field = |u: f32, v: f32| -> [f32; 4] {
        let (fx, fy) = (u * fnn as f32 - 0.5, v * fnn as f32 - 0.5);
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let at = |x: i32, y: i32| fields[(y.rem_euclid(fnn as i32) * fnn as i32 + x.rem_euclid(fnn as i32)) as usize];
        let (x0, y0) = (x0 as i32, y0 as i32);
        let (a, b, c, d) = (at(x0, y0), at(x0 + 1, y0), at(x0, y0 + 1), at(x0 + 1, y0 + 1));
        [0, 1, 2, 3].map(|k| (a[k] + (b[k] - a[k]) * tx) * (1.0 - ty) + (c[k] + (d[k] - c[k]) * tx) * ty)
    };

    // Height of crack grooves for the normal map.
    let groove = |x: i64, y: i64| -> f32 {
        match &cracks {
            Some(c) => c[((y.rem_euclid(n as i64)) * n as i64 + x.rem_euclid(n as i64)) as usize],
            None => 0.0,
        }
    };

    m.color
        .par_chunks_mut(n as usize)
        .zip(m.normal.par_chunks_mut(n as usize))
        .zip(m.orm.par_chunks_mut(n as usize))
        .enumerate()
        .for_each(|(y, ((crow, nrow), orow))| {
            let v = (y as f32 + 0.5) / n as f32;
            for x in 0..n as usize {
                let u = (x as f32 + 0.5) / n as f32;
                let i = y * n as usize + x;
                let mut c = crow[x];
                let mut nrm = nrow[x];
                let mut o = orow[x];
                // Chroma and level.
                let l = lum(c);
                for k in 0..3 {
                    c[k] = ((l + (c[k] - l) * sat) * gain).max(0.0);
                }
                // Low-frequency mottling, rain streaks and warm/cool drift.
                let fl = field(u, v);
                let mut val = fl[0];
                c[0] *= 1.0 + fl[1];
                c[2] *= 1.0 - fl[1];
                let mut rough = o[1] * r.rough_mul + r.rough_add + fl[2];
                if let Some((labels, tones)) = &pieces {
                    if let Some(&(f, dr)) = tones.get(labels[i] as usize) {
                        val *= f;
                        rough += dr;
                    }
                }
                if r.cavity > 0.0 && ao_hi > ao_lo {
                    let k = smoothstep(ao_hi, ao_lo, o[0]);
                    val *= 1.0 - r.cavity * k;
                    rough += 0.05 * r.cavity * k;
                }
                let mut flat = 0.0f32;
                for p in &patches {
                    let k = p.cover(u, v, 0.08);
                    if k > 0.0 {
                        val *= 1.0 + p.value * k;
                        rough += p.rough * k;
                        flat = flat.max(k * r.patch_flat);
                    }
                }
                for s in &stains {
                    let k = s.cover(u, v, 0.7);
                    if k > 0.0 {
                        val *= 1.0 - s.value * k;
                        rough += 0.04 * k;
                    }
                }
                for s in &oils {
                    let k = s.cover(u, v, 0.8);
                    if k > 0.0 {
                        val *= 1.0 - s.value * k;
                        rough += (0.32 - rough) * k * 0.8;
                    }
                }
                if let Some(cm) = &cracks {
                    let k = cm[i];
                    if k > 0.0 {
                        val *= 1.0 - r.crack_dark * k;
                        if r.crack_rough > 0.0 {
                            rough += (r.crack_rough - rough) * k;
                        }
                        o[0] *= 1.0 - 0.5 * k;
                    }
                }
                for ch in c.iter_mut() {
                    *ch = (*ch * val).clamp(0.0, 1.0);
                }
                if let Some(g) = &gum {
                    let k = g[i] * 0.85;
                    if k > 0.0 {
                        // Gum turns into grey-black discs, slightly glossy.
                        for ch in c.iter_mut() {
                            *ch += (0.035 - *ch) * k;
                        }
                        rough += (0.55 - rough) * k;
                    }
                }
                // Normals: strength, smoothed patches, crack grooves.
                let s = r.normal * (1.0 - flat);
                let (mut nx, mut ny) = (nrm[0] * s, nrm[1] * s);
                if cracks.is_some() {
                    let (xi, yi) = (x as i64, y as i64);
                    let gx = groove(xi + 1, yi) - groove(xi - 1, yi);
                    let gy = groove(xi, yi + 1) - groove(xi, yi - 1);
                    nx += gx * 0.6;
                    ny -= gy * 0.6;
                }
                let l2 = (nx * nx + ny * ny).min(0.95);
                let z = (1.0 - l2).sqrt();
                let len = (nx * nx + ny * ny + z * z).sqrt();
                nrm = [nx / len, ny / len, z / len];
                o[1] = rough.clamp(0.04, 1.0);
                crow[x] = c;
                nrow[x] = nrm;
                orow[x] = o;
            }
        });

    let enc = |v: &[[f32; 3]], f: &(dyn Fn(f32) -> f32 + Sync)| -> Vec<u8> {
        let mut out = vec![255u8; v.len() * 4];
        out.par_chunks_mut(4).zip(v.par_iter()).for_each(|(px, c)| {
            for k in 0..3 {
                px[k] = (f(c[k]).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
        });
        out
    };
    let tex = |name: String, rgba: Vec<u8>, srgb: bool| TextureData { name, width: n, height: n, rgba8: rgba.into(), srgb };
    PbrSet {
        name: format!("{}_macro", set.name),
        color: tex(format!("{}_macro_color", set.name), enc(&m.color, &linear_to_srgb), true),
        normal: Some(tex(format!("{}_macro_normal", set.name), enc(&m.normal, &|v| v * 0.5 + 0.5), false)),
        orm: Some(tex(format!("{}_macro_orm", set.name), enc(&m.orm, &|v| v), false)),
        emission: set.emission.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic(n: u32) -> PbrSet {
        let mut rng = Rng::new(4);
        let mut c = Vec::new();
        let mut o = Vec::new();
        let mut nm = Vec::new();
        for _ in 0..n * n {
            let g = 100 + rng.below(60) as u8;
            c.extend([g, g - 10, g - 20, 255]);
            o.extend([220, 150 + rng.below(40) as u8, 0, 255]);
            nm.extend([128 + rng.below(20) as u8 - 10, 128, 250, 255]);
        }
        let t = |rgba: Vec<u8>, srgb| TextureData { name: "t".into(), width: n, height: n, rgba8: rgba.into(), srgb };
        PbrSet { name: "syn".into(), color: t(c, true), normal: Some(t(nm, false)), orm: Some(t(o, false)), emission: None }
    }

    fn mean_lum(t: &TextureData) -> f32 {
        let v: Vec<[f32; 3]> = t.rgba8.chunks_exact(4).map(|p| decode_srgb([p[0], p[1], p[2]])).collect();
        lum(mean3(&v))
    }

    #[test]
    fn remix_keeps_size_mean_and_tiles() {
        let s = synthetic(64);
        let r = Recipe { tiles: 3, size: 192, meters: 6.0, remix: true, rotate: true, ..Default::default() };
        let m = build(&s, &r);
        assert_eq!((m.color.width, m.color.height), (192, 192));
        let (a, b) = (mean_lum(&s.color), mean_lum(&m.color));
        assert!((a - b).abs() / a < 0.05, "mean {a} -> {b}");
        assert_eq!(m.orm.as_ref().unwrap().width, 192);
    }

    #[test]
    fn calibration_and_wear_change_albedo_and_roughness() {
        let s = synthetic(32);
        let r = Recipe {
            tiles: 2,
            size: 128,
            meters: 4.0,
            saturation: 0.0,
            mean: Some(0.3),
            mottle: 0.1,
            stains: 4,
            stain_dark: 0.4,
            oil: 3,
            cracks: 3,
            gum: 40,
            rough_var: 0.1,
            ..Default::default()
        };
        let m = build(&s, &r);
        let l = mean_lum(&m.color);
        assert!(l > 0.2 && l < 0.33, "mean {l}");
        // Desaturated: channels close together on average.
        let px: Vec<&[u8]> = m.color.rgba8.chunks_exact(4).collect();
        let spread = px.iter().map(|p| (p[0] as i32 - p[2] as i32).abs()).sum::<i32>() as f32 / px.len() as f32;
        assert!(spread < 4.0, "chroma {spread}");
        let r: Vec<u8> = m.orm.as_ref().unwrap().rgba8.chunks_exact(4).map(|p| p[1]).collect();
        let (lo, hi) = (*r.iter().min().unwrap(), *r.iter().max().unwrap());
        assert!(hi - lo > 40, "roughness range {lo}..{hi}");
    }

    #[test]
    fn pieces_get_their_own_tone_and_joints_get_dirt() {
        // 4 x 4 square tiles separated by 2 px joints of low AO.
        let n = 64u32;
        let mut c = Vec::new();
        let mut o = Vec::new();
        for y in 0..n {
            for x in 0..n {
                let joint = x % 16 < 2 || y % 16 < 2;
                c.extend([150u8, 150, 150, 255]);
                o.extend([if joint { 90u8 } else { 250 }, 180, 0, 255]);
            }
        }
        let t = |rgba: Vec<u8>, srgb| TextureData { name: "t".into(), width: n, height: n, rgba8: rgba.into(), srgb };
        let s = PbrSet { name: "tiles".into(), color: t(c, true), normal: None, orm: Some(t(o, false)), emission: None };
        let (labels, count) = label_pieces(&tile_maps(&s, n).orm.iter().map(|o| o[0]).collect::<Vec<_>>(), n, 0.5);
        assert_eq!(count, 16, "one label per tile");
        assert!(labels.iter().all(|&l| l != u32::MAX));
        let r = Recipe { tiles: 1, size: 64, meters: 1.0, tile_tone: 0.1, joint_q: 0.3, cavity: 0.6, seed: 3, ..Default::default() };
        let m = build(&s, &r);
        let px = |x: u32, y: u32| m.color.rgba8[((y * n + x) * 4) as usize];
        // Joints darker than tile centres; tile centres differ.
        assert!((px(0, 8) as i32) < px(8, 8) as i32 - 20, "{} {}", px(0, 8), px(8, 8));
        let centres: Vec<u8> = (0..4).flat_map(|j| (0..4).map(move |i| (i * 16 + 8, j * 16 + 8))).map(|(x, y)| px(x, y)).collect();
        let (lo, hi) = (centres.iter().min().unwrap(), centres.iter().max().unwrap());
        assert!(hi - lo > 6, "tones {centres:?}");
    }

    #[test]
    fn spots_wrap_around_the_border() {
        let s = Spot { c: [0.99, 0.5], half: [0.05, 0.05], rect: true, value: 0.0, rough: 0.0, seed: 1 };
        assert!(s.cover(0.01, 0.5, 0.1) > 0.5);
        assert!(s.cover(0.5, 0.5, 0.1) == 0.0);
    }
}
