//! Adapter and device selection.
//!
//! The renderer runs on any desktop GPU from the GTX 1060 / RX 570 /
//! Intel Iris Xe class up. Startup therefore asks for **no optional
//! feature** (timestamp queries are used only when present) and the WebGPU
//! default limits (8 storage buffers and 16 sampled textures per stage,
//! 8 KiB of color attachments, 2048³ 3D textures), which every DX12
//! feature level 11_0 and Vulkan 1.1 driver of that class exceeds; an
//! adapter below them still gets a device with its own limits and a
//! warning instead of a failed start. Every texture format used is core
//! WebGPU (RGBA8, RGBA16F render and storage targets, Depth32F with
//! comparison sampling, R8).
//!
//! Choice order ([`AdapterChoice::Auto`]): discrete GPU, integrated GPU,
//! other, virtual, then software rasterizers; at equal type DX12 before
//! Vulkan on Windows. Every candidate is tried in turn, so a driver that
//! fails to create a device falls back to the next adapter (DX12 -> Vulkan
//! on the same GPU first). `--adapter` / `FORJA_ADAPTER` override it:
//! `warp` (or `software`, `cpu`) picks the software rasterizer (D3D12 WARP
//! on Windows, lavapipe / llvmpipe on Vulkan), `dx12` / `vulkan` /
//! `metal` / `gl` restrict the backend and any other text picks the
//! adapter whose name contains it.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

/// Validation and out-of-memory errors reported by the device since start
/// (logged; the smoke test fails on any).
pub static GPU_ERRORS: AtomicU32 = AtomicU32::new(0);

/// Which adapter to run on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum AdapterChoice {
    #[default]
    Auto,
    /// The software rasterizer (D3D12 WARP, lavapipe).
    Software,
    /// Only these backends (best adapter among them).
    Backend(wgpu::Backends),
    /// First adapter whose name contains this text (any case).
    Name(String),
}

impl AdapterChoice {
    pub fn parse(s: &str) -> AdapterChoice {
        match s.trim().to_lowercase().as_str() {
            "" | "auto" => AdapterChoice::Auto,
            "warp" | "software" | "cpu" | "fallback" | "lavapipe" | "llvmpipe" => AdapterChoice::Software,
            "dx12" | "d3d12" => AdapterChoice::Backend(wgpu::Backends::DX12),
            "vulkan" | "vk" => AdapterChoice::Backend(wgpu::Backends::VULKAN),
            "metal" => AdapterChoice::Backend(wgpu::Backends::METAL),
            "gl" | "opengl" => AdapterChoice::Backend(wgpu::Backends::GL),
            other => AdapterChoice::Name(other.to_string()),
        }
    }

    /// `FORJA_ADAPTER`, or [`AdapterChoice::Auto`].
    pub fn from_env() -> AdapterChoice {
        std::env::var("FORJA_ADAPTER").map(|v| AdapterChoice::parse(&v)).unwrap_or_default()
    }
}

/// Lower is better.
fn type_rank(t: wgpu::DeviceType) -> u32 {
    match t {
        wgpu::DeviceType::DiscreteGpu => 0,
        wgpu::DeviceType::IntegratedGpu => 1,
        wgpu::DeviceType::Other => 2,
        wgpu::DeviceType::VirtualGpu => 3,
        wgpu::DeviceType::Cpu => 4,
    }
}

fn backend_rank(b: wgpu::Backend) -> u32 {
    match b {
        wgpu::Backend::Dx12 if cfg!(windows) => 0,
        wgpu::Backend::Metal => 0,
        wgpu::Backend::Vulkan => 1,
        wgpu::Backend::Dx12 => 2,
        wgpu::Backend::Gl => 3,
        _ => 4,
    }
}

/// Software rasterizers: WARP reports itself as `Cpu` and is named
/// "Microsoft Basic Render Driver"; lavapipe and llvmpipe also say `Cpu`.
pub fn is_software(info: &wgpu::AdapterInfo) -> bool {
    info.device_type == wgpu::DeviceType::Cpu || info.name.contains("Basic Render Driver") || info.name.to_lowercase().contains("llvmpipe")
}

/// Every usable adapter for `choice`, best first. Logs the list.
pub fn candidates(instance: &wgpu::Instance, surface: Option<&wgpu::Surface<'_>>, choice: &AdapterChoice) -> Vec<wgpu::Adapter> {
    let mut all = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()));
    for a in &all {
        let i = a.get_info();
        log::info!("adapter: {} ({:?}, {:?}, driver {} {})", i.name, i.backend, i.device_type, i.driver, i.driver_info);
    }
    if let Some(s) = surface {
        all.retain(|a| a.is_surface_supported(s));
    }
    all.retain(|a| match choice {
        AdapterChoice::Auto => true,
        AdapterChoice::Software => is_software(&a.get_info()),
        AdapterChoice::Backend(b) => b.contains(wgpu::Backends::from(a.get_info().backend)),
        AdapterChoice::Name(n) => a.get_info().name.to_lowercase().contains(n),
    });
    all.sort_by_key(|a| {
        let i = a.get_info();
        (type_rank(i.device_type), backend_rank(i.backend))
    });
    all
}

/// Limits to ask for: the WebGPU defaults (all the renderer needs) with the
/// adapter's texture size, or the adapter's own limits when it cannot meet
/// the defaults (logged; the renderer then degrades where a pass would not
/// fit).
pub fn required_limits(adapter: &wgpu::Adapter) -> wgpu::Limits {
    let have = adapter.limits();
    let want = wgpu::Limits::default().using_resolution(have.clone());
    if want.check_limits(&have) {
        return want;
    }
    want.check_limits_with_fail_fn(&have, false, |name, w, h| log::warn!("adapter limit {name}: {h} (renderer default {w})"));
    have
}

/// Opens a device on the first candidate that gives one. Returns the
/// adapter with it (surface capabilities, present modes).
pub fn open(instance: &wgpu::Instance, surface: Option<&wgpu::Surface<'_>>, choice: &AdapterChoice) -> Result<(wgpu::Adapter, wgpu::Device, wgpu::Queue), String> {
    let mut list = candidates(instance, surface, choice);
    if list.is_empty() && *choice != AdapterChoice::Auto {
        log::warn!("no adapter matches {choice:?}; using the best available");
        list = candidates(instance, surface, &AdapterChoice::Auto);
    }
    if list.is_empty() {
        // Platforms that enumerate nothing (some GL setups): let wgpu pick.
        let a = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: *choice == AdapterChoice::Software,
            compatible_surface: surface,
            ..Default::default()
        }))
        .map_err(|e| format!("no GPU adapter: {e}"))?;
        list.push(a);
    }
    let mut last_err = String::new();
    for adapter in list {
        let info = adapter.get_info();
        let desc = wgpu::DeviceDescriptor {
            label: Some("forja"),
            required_features: crate::profiler::GpuProfiler::wanted_features(&adapter),
            required_limits: required_limits(&adapter),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        };
        match pollster::block_on(adapter.request_device(&desc)) {
            Ok((device, queue)) => {
                log::info!("GPU: {} ({:?}, {:?}){}", info.name, info.backend, info.device_type, if is_software(&info) { " [software rasterizer]" } else { "" });
                install_error_handlers(&device);
                return Ok((adapter, device, queue));
            }
            Err(e) => {
                log::warn!("{} ({:?}): no device: {e}; trying the next adapter", info.name, info.backend);
                last_err = e.to_string();
            }
        }
    }
    Err(format!("no adapter could create a device: {last_err}"))
}

/// Logs validation / out-of-memory errors (counted in [`GPU_ERRORS`])
/// instead of wgpu's default panic, and a lost device.
pub fn install_error_handlers(device: &wgpu::Device) {
    device.on_uncaptured_error(Arc::new(|e: wgpu::Error| {
        GPU_ERRORS.fetch_add(1, Ordering::Relaxed);
        log::error!("wgpu error: {e}");
    }));
    device.set_device_lost_callback(|reason, msg| {
        if reason != wgpu::DeviceLostReason::Destroyed {
            GPU_ERRORS.fetch_add(1, Ordering::Relaxed);
            log::error!("GPU device lost ({reason:?}): {msg}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_adapter_choices() {
        assert_eq!(AdapterChoice::parse("WARP"), AdapterChoice::Software);
        assert_eq!(AdapterChoice::parse("auto"), AdapterChoice::Auto);
        assert_eq!(AdapterChoice::parse("dx12"), AdapterChoice::Backend(wgpu::Backends::DX12));
        assert_eq!(AdapterChoice::parse("GeForce"), AdapterChoice::Name("geforce".into()));
    }
}
