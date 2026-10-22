//! Windows: GPU adapters and their dedicated video memory through DXGI. Works for NVIDIA, AMD and
//! Intel with only the normal graphics driver installed.

use crate::hardware::Gpu;
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE};

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
