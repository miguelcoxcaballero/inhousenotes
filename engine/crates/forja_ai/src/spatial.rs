//! Uniform spatial hash rebuilt every step: sorted `(cell, id)` pairs, so
//! queries are allocation-free and iteration order is deterministic.

use forja_math::Vec2;

#[derive(Clone, Debug)]
pub struct SpatialHash {
    cell: f32,
    entries: Vec<(u64, u32)>,
}

fn key(x: i32, y: i32) -> u64 {
    ((x as u32 as u64) << 32) | (y as u32 as u64)
}

impl SpatialHash {
    pub fn new(cell: f32) -> Self {
        Self { cell, entries: Vec::new() }
    }

    fn cell_of(&self, p: Vec2) -> (i32, i32) {
        ((p.x / self.cell).floor() as i32, (p.y / self.cell).floor() as i32)
    }

    pub fn rebuild(&mut self, items: impl Iterator<Item = (u32, Vec2)>) {
        self.entries.clear();
        for (id, p) in items {
            let (x, y) = self.cell_of(p);
            self.entries.push((key(x, y), id));
        }
        self.entries.sort_unstable();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Calls `f` with every id whose cell touches the box `lo..hi`.
    pub fn query_box(&self, lo: Vec2, hi: Vec2, mut f: impl FnMut(u32)) {
        let (x0, y0) = self.cell_of(lo);
        let (x1, y1) = self.cell_of(hi);
        for x in x0..=x1 {
            let k0 = key(x, y0);
            let k1 = key(x, y1);
            let start = self.entries.partition_point(|e| e.0 < k0);
            for e in &self.entries[start..] {
                if e.0 > k1 {
                    break;
                }
                f(e.1);
            }
        }
    }

    /// Calls `f` with every id whose cell touches the disc `(c, r)`.
    pub fn query(&self, c: Vec2, r: f32, mut f: impl FnMut(u32)) {
        let (x0, y0) = self.cell_of(c - Vec2::splat(r));
        let (x1, y1) = self.cell_of(c + Vec2::splat(r));
        for x in x0..=x1 {
            for y in y0..=y1 {
                let k = key(x, y);
                let start = self.entries.partition_point(|e| e.0 < k);
                for e in &self.entries[start..] {
                    if e.0 != k {
                        break;
                    }
                    f(e.1);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_neighbours_across_cells() {
        let mut h = SpatialHash::new(5.0);
        h.rebuild([(0, Vec2::new(0.0, 0.0)), (1, Vec2::new(4.9, 0.0)), (2, Vec2::new(5.1, 0.0)), (3, Vec2::new(-30.0, 2.0))].into_iter());
        let mut got = Vec::new();
        h.query(Vec2::new(5.0, 0.0), 1.0, |i| got.push(i));
        got.sort();
        assert_eq!(got, vec![0, 1, 2]);
    }
}
