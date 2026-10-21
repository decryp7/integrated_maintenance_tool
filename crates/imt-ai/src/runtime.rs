//! The port to the inference engine: turn a validated model file into a provider. The real adapter
//! (`engine::LlamaRuntime`) wraps llama.cpp; tests use fakes.

use crate::error::{AiError, Result};
use crate::gguf::GgufInfo;
use crate::hardware::Hardware;
use crate::llm::{LlmProvider, ModelLoader};
use crate::models::ModelStore;
use crate::recommend::{assess, Fit};
use crate::settings::{AiSettings, GpuOffload};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuMode {
    /// Decided from the free GPU memory.
    Auto,
    Cpu,
    Layers(u32),
}

impl From<GpuOffload> for GpuMode {
    fn from(g: GpuOffload) -> Self {
        match g {
            GpuOffload::Auto => GpuMode::Auto,
            GpuOffload::Layers(0) => GpuMode::Cpu,
            GpuOffload::Layers(n) => GpuMode::Layers(n),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LoadRequest {
    pub path: PathBuf,
    /// The model's image projector (`mmproj`), loaded into the same engine so the one model reads images too
    /// (ADR 0025).
    pub projector: Option<PathBuf>,
    pub info: GgufInfo,
    pub context: usize,
    pub gpu: GpuMode,
    /// 0 = automatic.
    pub threads: u32,
}

pub struct Loaded {
    pub provider: Arc<dyn LlmProvider>,
    /// "Metal", "Vulkan", "CPU", ...
    pub backend: String,
}

/// Contract: `load` validates what it can, never panics, and returns an error (not a half-working
/// provider) when the GPU or the file cannot be used, so callers can retry on the CPU.
pub trait ModelRuntime: Send + Sync {
    fn load(&self, req: &LoadRequest) -> Result<Loaded>;
}

/// Context length to use: the setting, but never above what the model was trained for.
pub fn effective_context(settings: &AiSettings, info: &GgufInfo) -> usize {
    match info.context_length {
        Some(c) if c >= 512 => settings.context_length.min(c as usize),
        _ => settings.context_length,
    }
}

/// Size of a projector file (0 for none or an unreadable one): it is loaded with the model and counts toward
/// its memory.
pub fn projector_bytes(projector: Option<&Path>) -> u64 {
    projector.and_then(|p| std::fs::metadata(p).ok()).map_or(0, |m| m.len())
}

/// Refuses a model (with `extra_bytes` loaded alongside it, its projector) that cannot fit this machine, with
/// advice.
pub fn check_memory(hw: &Hardware, info: &GgufInfo, extra_bytes: u64) -> Result<()> {
    let a = assess(hw, info.file_size.saturating_add(extra_bytes), info.params_b, info.is_moe(), 0.0);
    if a.fit == Fit::TooLarge {
        return Err(AiError::Memory(format!(
            "this model needs about {:.1} GB but this machine can offer less ({}). Choose a smaller model or a lower quantisation.",
            a.need_gb,
            hw.summary()
        )));
    }
    Ok(())
}

/// Loads with the requested GPU mode; if that fails, retries on the CPU. Returns the loaded model and a
/// note when it had to fall back.
pub fn load_with_fallback(runtime: &dyn ModelRuntime, mut req: LoadRequest) -> Result<(Loaded, Option<String>)> {
    match runtime.load(&req) {
        Ok(l) => Ok((l, None)),
        Err(first) if req.gpu != GpuMode::Cpu => {
            req.gpu = GpuMode::Cpu;
            match runtime.load(&req) {
                Ok(l) => Ok((l, Some(format!("The GPU could not be used ({first}); running on the CPU instead.")))),
                Err(_) => Err(first),
            }
        }
        Err(e) => Err(e),
    }
}

/// Loads the installed active model on demand (for `LazyProvider`).
pub struct InstalledModelLoader {
    pub runtime: Arc<dyn ModelRuntime>,
    pub store: ModelStore,
    pub file_name: String,
    pub settings: AiSettings,
    pub hardware: Hardware,
}

impl ModelLoader for InstalledModelLoader {
    fn load(&self) -> Result<Arc<dyn LlmProvider>> {
        let path = self.store.path_of(&self.file_name)?;
        let info = crate::gguf::inspect(&path)?;
        let projector = self.store.projector_path(&self.file_name)?;
        check_memory(&self.hardware, &info, projector_bytes(projector.as_deref()))?;
        let req = LoadRequest {
            context: effective_context(&self.settings, &info),
            path,
            projector,
            info,
            gpu: self.settings.gpu_offload.into(),
            threads: self.settings.threads,
        };
        load_with_fallback(self.runtime.as_ref(), req).map(|(l, _)| l.provider)
    }

    fn context_hint(&self) -> usize {
        self.settings.context_length
    }

    fn vision_hint(&self) -> bool {
        self.store.projector_path(&self.file_name).is_ok_and(|p| p.is_some())
    }
}
