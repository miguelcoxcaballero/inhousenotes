//! Static instances (buildings, facade modules, street furniture, trees)
//! uploaded once and culled per grid cell instead of per instance.
//!
//! The procedural city is made of tens of thousands of small instanced
//! modules (windows, shutters, balconies, bollards). Testing, LOD-selecting
//! and sorting every one of them five times per frame (camera + 4 shadow
//! cascades) cost more CPU than everything else in the frame. Here they are
//! bucketed once into a uniform XZ grid; inside a cell they are grouped into
//! *runs* of the same draw key (pipeline, material, mesh), each run sorted
//! by bounding radius, largest first. Per frame and per view:
//!
//! * a cell outside the view is skipped whole;
//! * a cell fully inside is copied run by run (no per-instance work), a cell
//!   crossing the view border tests each instance's bounding sphere;
//! * small-detail culling (sub-pixel in the camera, under ~2.5 shadow texels
//!   in a cascade) is a binary search in the radius-sorted run;
//! * LOD is chosen per run from the cell's nearest distance (conservative:
//!   never coarser than the per-instance choice);
//! * the surviving segments are sorted by draw key, so each key becomes one
//!   instanced draw for the whole view.
//!
//! The work is proportional to the visible cells and runs, not to the city.

use forja_math::{Aabb, Vec3, Vec4};

use crate::InstanceRaw;

/// Draw key of a static mesh part: everything that selects a pipeline and
/// its bindings. Sorted order = submission order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct DrawKey {
    /// 0 opaque, 1 double-sided, 3 blended (static parts are never skinned).
    pub pipe: u8,
    pub material: u32,
    pub mesh: u32,
}

/// A batch: `count` instances from `first` in the frame instance buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Batch {
    pub pipe: u8,
    pub material: u32,
    pub mesh: u32,
    pub lod: u32,
    pub skinned: bool,
    pub first: u32,
    pub count: u32,
}

impl Batch {
    pub fn sort_key(&self) -> (u8, bool, u32, u32, u32) {
        (self.pipe, self.skinned, self.material, self.mesh, self.lod)
    }
}

/// One static mesh part in world space.
#[derive(Clone, Copy)]
pub(crate) struct StaticItem {
    pub key: DrawKey,
    pub raw: InstanceRaw,
    pub bounds: Aabb,
    /// Not drawn into shadow maps (blended, or `cast_shadows: false`).
    pub no_shadow: bool,
}

struct Run {
    key: u32,
    start: u32,
    count: u32,
    /// Largest bounding radius in the run (items sorted descending).
    max_radius: f32,
    casts: bool,
}

struct Cell {
    bounds: Aabb,
    runs: std::ops::Range<u32>,
}

/// View of one pass for culling.
pub(crate) struct View<'a> {
    pub planes: &'a [Vec4],
    pub eye: Vec3,
    /// Minimum bounding radius per metre of distance (sub-pixel culling in
    /// the camera; 0 for shadow cascades).
    pub min_radius_per_m: f32,
    /// Absolute minimum radius (shadow texel threshold; 0 for the camera).
    pub min_radius: f32,
    /// Camera: pixels covered per (radius/distance)^2, for LOD selection.
    pub lod_scale: f32,
    pub shadow: bool,
}

enum Source {
    Range(u32, u32),
    Picked(u32, u32),
}

struct Segment {
    key: u32,
    lod: u32,
    src: Source,
}

#[derive(Default)]
pub(crate) struct StaticScene {
    keys: Vec<DrawKey>,
    cells: Vec<Cell>,
    runs: Vec<Run>,
    raws: Vec<InstanceRaw>,
    /// center xyz, radius (same order as `raws`).
    spheres: Vec<Vec4>,
    pub items: usize,
    segments: Vec<Segment>,
    picked: Vec<u32>,
}

/// Grid cell edge (m): about a city block side, so a street view touches
/// a few dozen cells.
pub(crate) const CELL_SIZE: f32 = 32.0;

fn sphere_in(planes: &[Vec4], c: Vec3, r: f32) -> bool {
    planes.iter().all(|p| p.truncate().dot(c) + p.w >= -r)
}

/// 0 outside, 1 crossing, 2 fully inside.
fn classify(planes: &[Vec4], b: &Aabb) -> u8 {
    let c = b.center();
    let h = b.half_extents();
    let mut inside = true;
    for p in planes {
        let n = p.truncate();
        let d = n.dot(c) + p.w;
        let e = n.abs().dot(h);
        if d + e < 0.0 {
            return 0;
        }
        if d - e < 0.0 {
            inside = false;
        }
    }
    if inside { 2 } else { 1 }
}

fn dist_to_aabb(p: Vec3, b: &Aabb) -> f32 {
    (b.min - p).max(p - b.max).max(Vec3::ZERO).length()
}

impl StaticScene {
    pub fn build(items: Vec<StaticItem>) -> Self {
        let mut keys: Vec<DrawKey> = items.iter().map(|i| i.key).collect();
        keys.sort_unstable();
        keys.dedup();
        let key_of = |k: &DrawKey| keys.binary_search(k).expect("key") as u32;
        // Cell of each item: grid by center; items larger than a cell get
        // a cell of their own (a ground plane must not make a whole block
        // of modules always visible).
        let mut tagged: Vec<(u64, u32, f32, usize)> = items
            .iter()
            .enumerate()
            .map(|(i, it)| {
                let c = it.bounds.center();
                let r = it.bounds.half_extents().length();
                let cell = if r > CELL_SIZE {
                    (1u64 << 63) | i as u64
                } else {
                    let gx = ((c.x / CELL_SIZE).floor() as i64 + (1 << 20)) as u64;
                    let gz = ((c.z / CELL_SIZE).floor() as i64 + (1 << 20)) as u64;
                    (gx << 21) | gz
                };
                (cell, key_of(&it.key), -r, i)
            })
            .collect();
        tagged.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)).then(a.2.total_cmp(&b.2)));
        let mut s = StaticScene { items: items.len(), ..Default::default() };
        let mut i = 0;
        while i < tagged.len() {
            let cell_id = tagged[i].0;
            let run_start = s.runs.len() as u32;
            let mut bounds = Aabb::EMPTY;
            while i < tagged.len() && tagged[i].0 == cell_id {
                let key = tagged[i].1;
                let start = s.raws.len() as u32;
                let mut casts = false;
                let mut max_radius = 0.0f32;
                while i < tagged.len() && tagged[i].0 == cell_id && tagged[i].1 == key {
                    let it = &items[tagged[i].3];
                    let r = it.bounds.half_extents().length();
                    max_radius = max_radius.max(r);
                    bounds = bounds.union(&it.bounds);
                    // Shadow casting is per item; a run casts if any does
                    // (non-casting items of a casting run are skipped below).
                    casts |= !it.no_shadow;
                    s.raws.push(it.raw);
                    s.spheres.push(it.bounds.center().extend(if it.no_shadow { -r } else { r }));
                    i += 1;
                }
                s.runs.push(Run { key, start, count: s.raws.len() as u32 - start, max_radius, casts });
            }
            s.cells.push(Cell { bounds, runs: run_start..s.runs.len() as u32 });
        }
        s.keys = keys;
        s
    }

    pub fn cell_count(&self) -> usize {
        self.cells.len()
    }

    /// Lower bound of the distance from `eye` to visible static geometry
    /// (cell bounds); stops early once under `stop`.
    pub fn nearest_visible(&self, planes: &[Vec4], eye: Vec3, stop: f32) -> f32 {
        let mut nearest = f32::INFINITY;
        for cell in &self.cells {
            let d = dist_to_aabb(eye, &cell.bounds);
            if d < nearest && classify(planes, &cell.bounds) != 0 {
                nearest = d;
                if nearest <= stop {
                    break;
                }
            }
        }
        nearest
    }

    /// Culls the static set for one view and appends its instances to `out`
    /// and its draws to `batches`. `lod_for(mesh, pixels)` picks a LOD from
    /// the covered pixel area (infinite for shadow views; the closure
    /// applies any bias and clamps to the mesh's levels). Returns the number of instances emitted.
    pub fn gather(&mut self, view: &View, lod_for: &dyn Fn(u32, f32) -> u32, out: &mut Vec<InstanceRaw>, batches: &mut Vec<Batch>) -> u32 {
        self.segments.clear();
        self.picked.clear();
        for cell in &self.cells {
            let class = classify(view.planes, &cell.bounds);
            if class == 0 {
                continue;
            }
            let d = dist_to_aabb(view.eye, &cell.bounds).max(0.1);
            let min_r = (view.min_radius_per_m * d).max(view.min_radius);
            for run in &self.runs[cell.runs.start as usize..cell.runs.end as usize] {
                if run.max_radius < min_r || (view.shadow && !run.casts) {
                    continue;
                }
                let lod = if view.lod_scale > 0.0 {
                    let px = run.max_radius / d;
                    lod_for(self.keys[run.key as usize].mesh, view.lod_scale * px * px)
                } else {
                    lod_for(self.keys[run.key as usize].mesh, f32::INFINITY)
                };
                let range = &self.spheres[run.start as usize..(run.start + run.count) as usize];
                // Radius-sorted: keep the prefix that is large enough.
                let n = range.partition_point(|s| s.w.abs() >= min_r) as u32;
                if n == 0 {
                    continue;
                }
                if class == 2 && (!view.shadow || range[..n as usize].iter().all(|s| s.w >= 0.0)) {
                    self.segments.push(Segment { key: run.key, lod, src: Source::Range(run.start, n) });
                } else {
                    let p0 = self.picked.len() as u32;
                    for (k, s) in range[..n as usize].iter().enumerate() {
                        if view.shadow && s.w < 0.0 {
                            continue;
                        }
                        if class == 2 || sphere_in(view.planes, s.truncate(), s.w.abs()) {
                            self.picked.push(run.start + k as u32);
                        }
                    }
                    let pn = self.picked.len() as u32 - p0;
                    if pn > 0 {
                        self.segments.push(Segment { key: run.key, lod, src: Source::Picked(p0, pn) });
                    }
                }
            }
        }
        self.segments.sort_unstable_by_key(|s| (s.key, s.lod));
        let mut emitted = 0;
        let mut i = 0;
        while i < self.segments.len() {
            let (key, lod) = (self.segments[i].key, self.segments[i].lod);
            let first = out.len() as u32;
            while i < self.segments.len() && self.segments[i].key == key && self.segments[i].lod == lod {
                match self.segments[i].src {
                    Source::Range(s, n) => out.extend_from_slice(&self.raws[s as usize..(s + n) as usize]),
                    Source::Picked(s, n) => out.extend(self.picked[s as usize..(s + n) as usize].iter().map(|&k| self.raws[k as usize])),
                }
                i += 1;
            }
            let count = out.len() as u32 - first;
            emitted += count;
            let k = self.keys[key as usize];
            batches.push(Batch { pipe: k.pipe, material: k.material, mesh: k.mesh, lod, skinned: false, first, count });
        }
        emitted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::Zeroable;
    use forja_math::Mat4;

    fn item(mesh: u32, x: f32, z: f32, r: f32) -> StaticItem {
        let mut raw = InstanceRaw::zeroed();
        raw.model = Mat4::from_translation(Vec3::new(x, 0.0, z)).to_cols_array_2d();
        StaticItem { key: DrawKey { pipe: 0, material: 0, mesh }, raw, bounds: Aabb::from_center_half(Vec3::new(x, 0.0, z), Vec3::splat(r / 3f32.sqrt())), no_shadow: false }
    }

    /// Planes of an axis-aligned box region x in [x0, x1] (all y, z < 1000).
    fn slab(x0: f32, x1: f32) -> [Vec4; 6] {
        [
            Vec4::new(1.0, 0.0, 0.0, -x0),
            Vec4::new(-1.0, 0.0, 0.0, x1),
            Vec4::new(0.0, 1.0, 0.0, 1000.0),
            Vec4::new(0.0, -1.0, 0.0, 1000.0),
            Vec4::new(0.0, 0.0, 1.0, 1000.0),
            Vec4::new(0.0, 0.0, -1.0, 1000.0),
        ]
    }

    #[test]
    fn culls_cells_and_batches_by_key() {
        let mut items = Vec::new();
        for i in 0..200 {
            items.push(item(i % 3, i as f32 * 2.0, 5.0, 0.5));
        }
        let mut s = StaticScene::build(items);
        assert!(s.cell_count() >= 12);
        let planes = slab(-1.0, 99.0);
        let view = View { planes: &planes, eye: Vec3::ZERO, min_radius_per_m: 0.0, min_radius: 0.0, lod_scale: 0.0, shadow: false };
        let mut out = Vec::new();
        let mut batches = Vec::new();
        let n = s.gather(&view, &|_, _| 0, &mut out, &mut batches);
        // x = 0..98 step 2 -> 50 items, in exactly one batch per mesh.
        assert_eq!(n, 50);
        assert_eq!(batches.len(), 3);
        assert_eq!(batches.iter().map(|b| b.count).sum::<u32>(), 50);
        for b in &batches {
            for raw in &out[b.first as usize..(b.first + b.count) as usize] {
                let x = raw.model[3][0];
                assert!((0.0..99.0).contains(&x));
                assert_eq!((x as u32 / 2) % 3, b.mesh);
            }
        }
    }

    #[test]
    fn small_detail_and_shadow_flags() {
        let mut items = vec![item(0, 1.0, 1.0, 2.0), item(0, 2.0, 1.0, 0.1), item(1, 3.0, 1.0, 1.0)];
        items[2].no_shadow = true;
        let mut s = StaticScene::build(items);
        let planes = slab(-100.0, 100.0);
        let mut out = Vec::new();
        let mut batches = Vec::new();
        let view = View { planes: &planes, eye: Vec3::ZERO, min_radius_per_m: 0.0, min_radius: 0.5, lod_scale: 0.0, shadow: true };
        let n = s.gather(&view, &|_, _| 1, &mut out, &mut batches);
        // Only the large caster survives: the small one is under the
        // threshold, the third does not cast.
        assert_eq!(n, 1);
        assert_eq!(batches[0].lod, 1);
    }
}
