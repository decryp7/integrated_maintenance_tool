//! Windows: GPU adapters and their dedicated video memory through DXGI. Works for NVIDIA, AMD and
//! Intel with only the normal graphics driver installed.

use crate::hardware::Gpu;
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, DXGI_QUERY_VIDEO_MEMORY_INFO, IDXGIAdapter3,
    IDXGIFactory1,
};
use windows::core::Interface;

/// Free dedicated video memory of the adapter with the most dedicated memory (the discrete GPU of a laptop that
/// also has an integrated one; budget the system grants this process minus what
/// is in use now: the desktop and other programs count as used). Shared system memory is the NON_LOCAL segment
/// and is never counted. `None` when no adapter answers.
pub fn free_vram_gb() -> Option<f64> {
    let mut best: Option<(usize, f64)> = None;
    // SAFETY: plain COM calls; every failure skips the adapter.
    unsafe {
        let factory = CreateDXGIFactory1::<IDXGIFactory1>().ok()?;
        let mut i = 0;
        while let Ok(adapter) = factory.EnumAdapters1(i) {
            i += 1;
            let Ok(desc) = adapter.GetDesc1() else { continue };
            if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
                continue;
            }
            let Ok(a3) = adapter.cast::<IDXGIAdapter3>() else { continue };
            let mut info = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
            if a3.QueryVideoMemoryInfo(0, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, &mut info).is_err() {
                continue;
            }
            let free = info.Budget.saturating_sub(info.CurrentUsage) as f64 / (1024.0 * 1024.0 * 1024.0);
            if best.is_none_or(|(d, _)| desc.DedicatedVideoMemory > d) {
                best = Some((desc.DedicatedVideoMemory, free));
            }
        }
    }
    best.map(|(_, f)| f)
}

pub fn adapters() -> Vec<Gpu> {
    let mut out = Vec::new();
    // SAFETY: plain COM calls; every failure ends the enumeration.
    unsafe {
        let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else {
            return out;
        };
        let mut i = 0;
        while let Ok(adapter) = factory.EnumAdapters1(i) {
            i += 1;
            let Ok(desc) = adapter.GetDesc1() else { continue };
            if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
                continue;
            }
            let len = desc.Description.iter().position(|&c| c == 0).unwrap_or(desc.Description.len());
            let name = String::from_utf16_lossy(&desc.Description[..len]);
            out.push(Gpu {
                name,
                vram_gb: Some(desc.DedicatedVideoMemory as f64 / (1024.0 * 1024.0 * 1024.0)),
                unified: false,
            });
        }
    }
    out
}
