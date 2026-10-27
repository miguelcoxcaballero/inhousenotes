//! Frame profiler: GPU timestamp queries per render pass (when the adapter
//! supports `TIMESTAMP_QUERY`) and named CPU scopes of the renderer.
//!
//! GPU results arrive a few frames late (read back through a small ring of
//! buffers, never stalling the GPU); [`GpuProfiler::last`] always holds the
//! most recent complete frame. Passes with the same name are summed (the 4
//! shadow cascades report one line each, the 11 bloom passes one line).

use std::cell::{Cell, RefCell};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

const MAX_PASSES: u32 = 48;
const RING: usize = 3;

/// Slot states of the readback ring.
const FREE: u8 = 0;
const PENDING: u8 = 1;
const READY: u8 = 2;
const FAILED: u8 = 3;

struct Slot {
    buf: wgpu::Buffer,
    state: Arc<AtomicU8>,
    names: Vec<&'static str>,
}

pub struct GpuProfiler {
    query_set: Option<wgpu::QuerySet>,
    resolve: Option<wgpu::Buffer>,
    ring: Vec<Slot>,
    /// Ring slot recording this frame (None: profiling skipped this frame).
    current: Option<usize>,
    next: Cell<u32>,
    names: RefCell<Vec<&'static str>>,
    period_ns: f32,
    /// Most recent complete frame: `(pass, ms)`, in submission order.
    pub last: Vec<(&'static str, f32)>,
    /// Sum of [`GpuProfiler::last`].
    pub total_ms: f32,
    pub enabled: bool,
}

impl GpuProfiler {
    /// Features to request so the profiler works (empty when unsupported).
    pub fn wanted_features(adapter: &wgpu::Adapter) -> wgpu::Features {
        adapter.features() & wgpu::Features::TIMESTAMP_QUERY
    }

    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let supported = device.features().contains(wgpu::Features::TIMESTAMP_QUERY);
        let count = MAX_PASSES * 2;
        let bytes = count as u64 * 8;
        let (query_set, resolve, ring) = if supported {
            let qs = device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("gpu-profiler"), ty: wgpu::QueryType::Timestamp, count });
            let resolve = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gpu-profiler-resolve"),
                size: bytes,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let ring = (0..RING)
                .map(|_| Slot {
                    buf: device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("gpu-profiler-readback"),
                        size: bytes,
                        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                        mapped_at_creation: false,
                    }),
                    state: Arc::new(AtomicU8::new(FREE)),
                    names: Vec::new(),
                })
                .collect();
            (Some(qs), Some(resolve), ring)
        } else {
            (None, None, Vec::new())
        };
        Self {
            query_set,
            resolve,
            ring,
            current: None,
            next: Cell::new(0),
            names: RefCell::new(Vec::new()),
            period_ns: queue.get_timestamp_period(),
            last: Vec::new(),
            total_ms: 0.0,
            enabled: true,
        }
    }

    /// GPU timings are available on this device.
    pub fn supported(&self) -> bool {
        self.query_set.is_some()
    }

    /// Starts recording a frame: collects finished readbacks and picks a
    /// free slot (none free: this frame is not timed).
    pub fn begin_frame(&mut self, device: &wgpu::Device) {
        self.current = None;
        self.next.set(0);
        self.names.borrow_mut().clear();
        if self.query_set.is_none() {
            return;
        }
        let _ = device.poll(wgpu::PollType::Poll);
        let mut newest: Option<Vec<(&'static str, f32)>> = None;
        for slot in &mut self.ring {
            match slot.state.load(Ordering::Acquire) {
                READY => {
                    let mut out: Vec<(&'static str, f32)> = Vec::new();
                    if let Ok(data) = slot.buf.slice(..).get_mapped_range() {
                        let ts: &[u64] = bytemuck::cast_slice(&data[..slot.names.len() * 16]);
                        for (i, name) in slot.names.iter().enumerate() {
                            let (a, b) = (ts[i * 2], ts[i * 2 + 1]);
                            let ms = b.saturating_sub(a) as f32 * self.period_ns * 1e-6;
                            match out.iter_mut().find(|e| e.0 == *name) {
                                Some(e) => e.1 += ms,
                                None => out.push((name, ms)),
                            }
                        }
                    }
                    slot.buf.unmap();
                    slot.state.store(FREE, Ordering::Release);
                    newest = Some(out);
                }
                FAILED => {
                    slot.buf.unmap();
                    slot.state.store(FREE, Ordering::Release);
                }
                _ => {}
            }
        }
        if let Some(out) = newest {
            self.total_ms = out.iter().map(|e| e.1).sum();
            self.last = out;
        }
        if self.enabled {
            self.current = self.ring.iter().position(|s| s.state.load(Ordering::Acquire) == FREE);
        }
    }

    /// Timestamp writes for a render pass named `name` (None when not timing).
    pub fn pass(&self, name: &'static str) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        self.current?;
        let qs = self.query_set.as_ref()?;
        let i = self.next.get();
        if i >= MAX_PASSES {
            return None;
        }
        self.next.set(i + 1);
        self.names.borrow_mut().push(name);
        Some(wgpu::RenderPassTimestampWrites { query_set: qs, beginning_of_pass_write_index: Some(i * 2), end_of_pass_write_index: Some(i * 2 + 1) })
    }

    /// Resolves this frame's queries into its readback slot (record before
    /// finishing the encoder).
    pub fn resolve(&mut self, enc: &mut wgpu::CommandEncoder) {
        let (Some(slot), Some(qs), Some(resolve)) = (self.current, &self.query_set, &self.resolve) else { return };
        let n = self.next.get();
        if n == 0 {
            self.current = None;
            return;
        }
        enc.resolve_query_set(qs, 0..n * 2, resolve, 0);
        enc.copy_buffer_to_buffer(resolve, 0, &self.ring[slot].buf, 0, n as u64 * 16);
        self.ring[slot].names = self.names.borrow().clone();
    }

    /// Maps this frame's slot after the submit that contains it.
    pub fn after_submit(&mut self) {
        let Some(slot) = self.current.take() else { return };
        let s = &self.ring[slot];
        s.state.store(PENDING, Ordering::Release);
        let state = s.state.clone();
        s.buf.slice(..).map_async(wgpu::MapMode::Read, move |r| state.store(if r.is_ok() { READY } else { FAILED }, Ordering::Release));
    }
}

/// Named CPU scopes of one frame, in ms.
#[derive(Clone, Debug, Default)]
pub struct CpuScopes {
    pub scopes: Vec<(&'static str, f32)>,
}

impl CpuScopes {
    pub fn clear(&mut self) {
        self.scopes.clear();
    }

    pub fn add(&mut self, name: &'static str, ms: f32) {
        match self.scopes.iter_mut().find(|e| e.0 == name) {
            Some(e) => e.1 += ms,
            None => self.scopes.push((name, ms)),
        }
    }

    /// Times `f` under `name`.
    pub fn time<R>(&mut self, name: &'static str, f: impl FnOnce() -> R) -> R {
        let t = std::time::Instant::now();
        let r = f();
        self.add(name, t.elapsed().as_secs_f32() * 1000.0);
        r
    }

    pub fn get(&self, name: &str) -> f32 {
        self.scopes.iter().find(|e| e.0 == name).map_or(0.0, |e| e.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_scopes_sum_by_name() {
        let mut s = CpuScopes::default();
        s.add("cull", 1.0);
        s.add("sort", 0.5);
        s.add("cull", 2.0);
        assert_eq!(s.scopes.len(), 2);
        assert_eq!(s.get("cull"), 3.0);
        let v = s.time("work", || 7);
        assert_eq!(v, 7);
        assert!(s.get("work") >= 0.0);
    }
}
