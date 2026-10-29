//! Shared vertex and index buffers for meshes.
//!
//! A city has tens of thousands of mesh primitives. With one vertex and
//! one index buffer (plus one per LOD) each, every draw rebinds its
//! buffers, and the backend validates, tracks and records each binding:
//! with ~3k draws per frame that was most of the command encoding CPU
//! time. Here primitives are packed into a few large pages: a page is
//! bound once and draws select their range with `first_index` and
//! `base_vertex`. Streamed tiles give their ranges back
//! ([`MeshPool::free`]) and re-upload into free space with
//! `queue.write_buffer`, so streaming creates no GPU resources.
//!
//! Allocation is first-fit over a sorted free list per page, in *units*
//! (one vertex or one index), so offsets are always aligned to the unit.

/// Location of an allocation: page and first unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Alloc {
    pub page: u32,
    pub first: u32,
    pub units: u32,
}

struct Page {
    buf: Option<wgpu::Buffer>,
    units: u32,
    /// Free ranges `(first, units)`, sorted and coalesced.
    free: Vec<(u32, u32)>,
    /// A page holding exactly one oversized allocation (dropped when it
    /// is freed).
    dedicated: bool,
}

pub(crate) struct MeshPool {
    label: &'static str,
    usage: wgpu::BufferUsages,
    unit: u32,
    page_units: u32,
    pages: Vec<Page>,
    /// Units in use (statistics).
    pub used_units: u64,
}

impl MeshPool {
    /// Pages of `page_bytes` (rounded down to whole units of `unit`
    /// bytes, which must be a multiple of 4).
    pub fn new(label: &'static str, usage: wgpu::BufferUsages, unit: u32, page_bytes: u64) -> Self {
        assert!(unit % 4 == 0, "copy alignment");
        Self { label, usage: usage | wgpu::BufferUsages::COPY_DST, unit, page_units: (page_bytes / unit as u64) as u32, pages: Vec::new(), used_units: 0 }
    }

    /// Bytes per unit.
    pub fn unit(&self) -> u32 {
        self.unit
    }

    /// The buffer of page `page`.
    pub fn buffer(&self, page: u32) -> &wgpu::Buffer {
        self.pages[page as usize].buf.as_ref().expect("live page")
    }

    /// Reserves `units` units (at least 1); `dedicated`: in a page of its
    /// own, starting at unit 0 (skinned meshes, whose skin stream is a
    /// separate buffer indexed like the vertices).
    pub fn alloc(&mut self, device: &wgpu::Device, units: u32, dedicated: bool) -> Alloc {
        let units = units.max(1);
        self.used_units += units as u64;
        if dedicated || units > self.page_units / 2 {
            // Oversized: a page of its own (reusing a dead slot).
            let buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some(self.label), size: units as u64 * self.unit as u64, usage: self.usage, mapped_at_creation: false });
            let page = Page { buf: Some(buf), units, free: Vec::new(), dedicated: true };
            let idx = match self.pages.iter().position(|p| p.buf.is_none()) {
                Some(i) => {
                    self.pages[i] = page;
                    i
                }
                None => {
                    self.pages.push(page);
                    self.pages.len() - 1
                }
            };
            return Alloc { page: idx as u32, first: 0, units };
        }
        for (pi, p) in self.pages.iter_mut().enumerate() {
            if p.dedicated || p.buf.is_none() {
                continue;
            }
            if let Some(k) = p.free.iter().position(|&(_, n)| n >= units) {
                let (first, n) = p.free[k];
                if n == units {
                    p.free.remove(k);
                } else {
                    p.free[k] = (first + units, n - units);
                }
                return Alloc { page: pi as u32, first, units };
            }
        }
        let buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some(self.label), size: self.page_units as u64 * self.unit as u64, usage: self.usage, mapped_at_creation: false });
        let page = Page { buf: Some(buf), units: self.page_units, free: vec![(units, self.page_units - units)], dedicated: false };
        let idx = match self.pages.iter().position(|p| p.buf.is_none()) {
            Some(i) => {
                self.pages[i] = page;
                i
            }
            None => {
                self.pages.push(page);
                self.pages.len() - 1
            }
        };
        Alloc { page: idx as u32, first: 0, units }
    }

    /// Gives an allocation back.
    pub fn free(&mut self, a: Alloc) {
        self.used_units = self.used_units.saturating_sub(a.units as u64);
        let p = &mut self.pages[a.page as usize];
        if p.dedicated {
            p.buf = None;
            p.free.clear();
            return;
        }
        let k = p.free.partition_point(|&(f, _)| f < a.first);
        p.free.insert(k, (a.first, a.units));
        // Coalesce with the neighbours.
        if k + 1 < p.free.len() && p.free[k].0 + p.free[k].1 == p.free[k + 1].0 {
            p.free[k].1 += p.free[k + 1].1;
            p.free.remove(k + 1);
        }
        if k > 0 && p.free[k - 1].0 + p.free[k - 1].1 == p.free[k].0 {
            p.free[k - 1].1 += p.free[k].1;
            p.free.remove(k);
        }
        let _ = p.units;
    }

    /// Writes `data` at the start of `a` (staged until the next submit).
    pub fn write(&self, queue: &wgpu::Queue, a: Alloc, data: &[u8]) {
        debug_assert!(data.len() as u64 <= a.units as u64 * self.unit as u64);
        if !data.is_empty() {
            queue.write_buffer(self.buffer(a.page), a.first as u64 * self.unit as u64, data);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool_free_list(p: &MeshPool, page: usize) -> Vec<(u32, u32)> {
        p.pages[page].free.clone()
    }

    #[test]
    fn free_list_coalesces() {
        // Exercise the free list without a device: build a page by hand.
        let mut p = MeshPool::new("t", wgpu::BufferUsages::VERTEX, 4, 400);
        p.pages.push(Page { buf: None, units: 100, free: vec![], dedicated: false });
        p.used_units = 30;
        p.free(Alloc { page: 0, first: 10, units: 10 });
        p.free(Alloc { page: 0, first: 30, units: 10 });
        assert_eq!(pool_free_list(&p, 0), vec![(10, 10), (30, 10)]);
        p.free(Alloc { page: 0, first: 20, units: 10 });
        assert_eq!(pool_free_list(&p, 0), vec![(10, 30)]);
        assert_eq!(p.used_units, 0);
    }
}
