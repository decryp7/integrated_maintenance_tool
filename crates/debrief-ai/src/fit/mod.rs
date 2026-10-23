//! The one place that decides whether a model fits this machine, before it is downloaded and when it is loaded.
//!
//! Need = weights (the real file size) + KV cache (from the model's architecture and the context Debrief will
//! really use) + a compute-buffer allowance. When the architecture is not known yet (before the download, no
//! header read) the KV cache is a rough estimate from the parameter count with a wider margin and the result is
//! labelled as such. The need is compared with two capacities: what the machine can hold at all, and what is
//! free right now (other programs use memory, and on Windows the desktop uses video memory).

use crate::gguf::GgufInfo;
use crate::hardware::Hardware;
use crate::recommend::{MemoryClass, Speed, budget};
use serde::Serialize;

const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

/// Parallel request slots Debrief starts llama-server with (`--parallel 1`).
pub const SERVER_SLOTS: u32 = 1;
/// The cache type Debrief starts llama-server with (llama.cpp's default: f16 keys and values).
pub const KV_TYPE: KvType = KvType::F16;
/// llama.cpp keeps `window + n_ubatch` positions in a sliding-window cache.
const SWA_EXTRA_POSITIONS: u64 = 512;
/// Extra margin on the rough (header-less) KV estimate.
const ROUGH_MARGIN: f64 = 1.25;
/// A model is "comfortable" when it needs at most this share of the capacity.
const COMFORT: f64 = 0.88;
/// Share of free system RAM that spill-over layers may use (discrete GPU).
const SPILL_SHARE: f64 = 0.4;
const MOE_RAM_SHARE: f64 = 0.8;
/// Video memory the desktop is assumed to use when the real figure cannot be read.
const ASSUMED_DESKTOP_VRAM_GB: f64 = 1.0;
/// Kept free on the GPU on top of the model.
const VRAM_SAFETY_GB: f64 = 0.25;
/// Contexts offered as advice, largest first, never below the smallest.
const MIN_ADVISED_CONTEXT: usize = 2048;

/// Type of the cached keys or values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum KvType {
    F16,
    Q8_0,
}

impl KvType {
    fn bytes_per_element(self) -> f64 {
        match self {
            KvType::F16 => 2.0,
            KvType::Q8_0 => 34.0 / 32.0,
        }
    }
}

/// How the KV cache of a model grows with the context.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum KvShape {
    /// Read from the model's own header.
    Exact {
        /// Layers that cache the whole context.
        full_layers: u32,
        /// Sliding-window layers (they cache only `window` positions).
        window_layers: u32,
        window: u32,
        kv_heads: u32,
        key_len: u32,
        value_len: u32,
    },
    /// Estimated from the parameter count only.
    Rough { bytes_per_token: u64 },
}

/// Sliding-window pattern: one full-attention layer in `n` (llama.cpp's Gemma implementations).
fn window_pattern(architecture: &str) -> Option<u32> {
    match architecture {
        "gemma2" => Some(2),
        "gemma3" => Some(6),
        "gemma3n" => Some(5),
        _ => None,
    }
}

impl KvShape {
    /// The shape in a model's header. Missing numbers fall back to a typical 7-9B layout.
    pub fn from_gguf(i: &GgufInfo) -> KvShape {
        match (i.block_count, i.embedding_length, i.head_count) {
            (Some(l), Some(e), Some(h)) if h > 0 && l > 0 => {
                let kv_heads = i.head_count_kv.unwrap_or(h).max(1);
                let head_dim = (e / h).max(1);
                let key_len = i.key_length.unwrap_or(head_dim);
                let value_len = i.value_length.unwrap_or(key_len);
                let mut full = l;
                let mut window_layers = 0;
                let mut window = 0;
                if let (Some(n), Some(w)) = (window_pattern(&i.architecture), i.sliding_window.filter(|w| *w > 0)) {
                    full = (l / n).max(1);
                    window_layers = l - full;
                    window = w;
                } else if let Some(n) = i.full_attention_interval.filter(|n| *n > 1) {
                    full = (l / n).max(1);
                }
                KvShape::Exact {
                    full_layers: full,
                    window_layers,
                    window,
                    kv_heads,
                    key_len,
                    value_len,
                }
            }
            _ => KvShape::Rough {
                bytes_per_token: 128 * 1024,
            },
        }
    }

    /// A guess from the parameter count: about 160 KB per token at 8B, growing with the square root (never below 60% of that).
    pub fn from_params(params_b: f64) -> KvShape {
        let scale = (params_b.max(1.0) / 8.0).sqrt().max(0.6);
        KvShape::Rough {
            bytes_per_token: (160.0 * 1024.0 * scale) as u64,
        }
    }

    pub fn is_rough(&self) -> bool {
        matches!(self, KvShape::Rough { .. })
    }

    /// Bytes per token if every layer cached the whole context (f16).
    pub fn bytes_per_token_full(&self) -> u64 {
        match *self {
            KvShape::Exact {
                full_layers,
                window_layers,
                kv_heads,
                key_len,
                value_len,
                ..
            } => ((full_layers + window_layers) as f64 * kv_heads as f64 * (key_len as f64 + value_len as f64) * 2.0) as u64,
            KvShape::Rough { bytes_per_token } => bytes_per_token,
        }
    }

    /// Bytes of KV cache for `context` tokens (the server's total context) split over `slots` request slots.
    pub fn bytes(&self, context: usize, slots: u32, kt: KvType, vt: KvType) -> u64 {
        match *self {
            KvShape::Rough { bytes_per_token } => (bytes_per_token as f64 * context as f64 * ROUGH_MARGIN) as u64,
            KvShape::Exact {
                full_layers,
                window_layers,
                window,
                kv_heads,
                key_len,
                value_len,
            } => {
                let per_pos = kv_heads as f64 * (key_len as f64 * kt.bytes_per_element() + value_len as f64 * vt.bytes_per_element());
                let full = full_layers as f64 * per_pos * context as f64;
                // Each slot keeps its own window.
                let cached = (window as u64 + SWA_EXTRA_POSITIONS).min(context as u64) as f64;
                let win = window_layers as f64 * per_pos * cached * slots.max(1) as f64;
                (full + win).ceil() as u64
            }
        }
    }
}

/// Memory for the compute buffers and the runtime itself, which grows a little with the context.
pub fn compute_buffer_gb(context: usize) -> f64 {
    0.6 + 0.1 * (context as f64 / 8192.0)
}

/// What is to be loaded.
#[derive(Debug, Clone, PartialEq)]
pub struct Subject {
    pub weights_bytes: u64,
    /// Image projector loaded with the model.
    pub projector_bytes: u64,
    pub shape: KvShape,
    pub params_b: f64,
    pub moe: bool,
    pub min_ram_gb: f64,
    /// Context the server will be started with.
    pub context: usize,
}

impl Subject {
    /// A model whose header is known.
    pub fn from_gguf(info: &GgufInfo, projector_bytes: u64, context: usize) -> Subject {
        Subject {
            weights_bytes: info.file_size,
            projector_bytes,
            shape: KvShape::from_gguf(info),
            params_b: info.params_b,
            moe: info.is_moe(),
            min_ram_gb: 0.0,
            context,
        }
    }

    pub fn with_context(&self, context: usize) -> Subject {
        Subject { context, ..self.clone() }
    }
}

/// The numbers behind a verdict.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Need {
    pub weights_gb: f64,
    pub kv_gb: f64,
    pub compute_gb: f64,
    pub total_gb: f64,
    /// The KV cache is a guess because the model's header has not been read.
    pub rough: bool,
}

pub fn need(s: &Subject) -> Need {
    let weights_gb = (s.weights_bytes + s.projector_bytes) as f64 / GIB;
    let kv_gb = s.shape.bytes(s.context, SERVER_SLOTS, KV_TYPE, KV_TYPE) as f64 / GIB;
    let compute_gb = compute_buffer_gb(s.context);
    Need {
        weights_gb,
        kv_gb,
        compute_gb,
        total_gb: weights_gb + kv_gb + compute_gb,
        rough: s.shape.is_rough(),
    }
}

/// What the machine looks like right now, beyond the static [`Hardware`].
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Live {
    /// Free dedicated video memory (NVML or DXGI); `None` = unknown.
    pub free_vram_gb: Option<f64>,
    /// Memory that Debrief's own loaded model would give back if it were unloaded.
    pub reclaim_gb: f64,
    /// The GPU working-set limit when the system reports it (Metal's `recommendedMaxWorkingSetSize`).
    pub gpu_working_set_gb: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Fit {
    /// Fits with room to spare.
    Comfortable,
    /// Fits, or runs with part of the model in system RAM (slower), or only if other programs close.
    Tight,
    TooLarge,
}

fn worse(a: Fit, b: Fit) -> Fit {
    use Fit::*;
    match (a, b) {
        (TooLarge, _) | (_, TooLarge) => TooLarge,
        (Tight, _) | (_, Tight) => Tight,
        _ => Comfortable,
    }
}

/// What to change when the model does not fit comfortably.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Advice {
    /// Use this context (tokens) instead.
    ShorterContext { tokens: usize },
    /// Choose a smaller quantisation or model.
    SmallerQuant,
    /// Close other programs: about this many GB must come free.
    CloseApps { gb: f64 },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Assessment {
    /// Total need (weights + context).
    pub need_gb: f64,
    pub weights_gb: f64,
    /// KV cache plus compute buffers.
    pub context_gb: f64,
    /// Context length the need was computed for.
    pub context: usize,
    pub rough: bool,
    /// The verdict: never better than either of the two below.
    pub fit: Fit,
    pub speed: Speed,
    /// Does it fit this machine at all?
    pub fits_machine: Fit,
    /// Does it fit with what is free right now?
    pub fits_now: Fit,
    /// What this machine can hold for a model (GB).
    pub capacity_gb: f64,
    /// What is free for a model right now (GB).
    pub available_now_gb: f64,
    /// Memory other programs are using (GB): the part of the capacity that is not free.
    pub in_use_gb: f64,
    /// Part of the model runs from system RAM (discrete GPU).
    pub spills: bool,
    pub advice: Vec<Advice>,
}

struct Pool {
    cap: f64,
    /// Extra room beyond `cap` that costs speed (spill-over layers, MoE experts).
    extra: f64,
}

fn classify(need: f64, p: &Pool) -> (Fit, bool) {
    let total = p.cap + p.extra;
    if need <= p.cap * COMFORT {
        (Fit::Comfortable, false)
    } else if need <= p.cap {
        (Fit::Tight, false)
    } else if need <= total {
        (Fit::Tight, true)
    } else {
        (Fit::TooLarge, false)
    }
}

/// The capacity for the machine as a whole and for the moment, for the memory class of `hw`.
fn pools(hw: &Hardware, live: &Live, moe: bool) -> (Pool, Pool, f64, f64, MemoryClass) {
    let b = budget(hw);
    let capacity = live
        .gpu_working_set_gb
        .filter(|_| b.class == MemoryClass::Unified)
        .unwrap_or(b.usable_gb);
    let (machine_cap, now_cap) = match b.class {
        MemoryClass::Unified => {
            let margin = (hw.total_ram_gb * 0.05).max(2.0);
            let free = hw.available_ram_gb + live.reclaim_gb - margin;
            (capacity, capacity.min(free).max(0.0))
        }
        MemoryClass::DiscreteGpu => {
            let free = live
                .free_vram_gb
                .map(|f| f + live.reclaim_gb)
                .unwrap_or(capacity - ASSUMED_DESKTOP_VRAM_GB);
            (capacity - VRAM_SAFETY_GB, (capacity.min(free) - VRAM_SAFETY_GB).max(0.0))
        }
        MemoryClass::CpuOnly => (capacity, capacity),
    };
    let ram = hw.available_ram_gb + live.reclaim_gb;
    let extra = |cap: f64| match (b.class, moe) {
        (MemoryClass::Unified, _) => 0.0,
        (MemoryClass::DiscreteGpu, true) => MOE_RAM_SHARE * ram,
        (MemoryClass::DiscreteGpu, false) => SPILL_SHARE * ram,
        (MemoryClass::CpuOnly, true) => MOE_RAM_SHARE * ram - cap,
        (MemoryClass::CpuOnly, false) => 0.0,
    };
    // Expert tensors of a mixture of experts live in system RAM by design: that is its normal pool, not a spill.
    let pool = |cap: f64| {
        let e = extra(cap).max(0.0);
        if moe {
            Pool { cap: cap + e, extra: 0.0 }
        } else {
            Pool { cap, extra: e }
        }
    };
    let machine = pool(machine_cap);
    let now = pool(now_cap);
    let in_use = (machine_cap - now_cap).max(0.0);
    let shown = now.cap;
    (machine, now, shown, in_use, b.class)
}

/// Does `s` fit this machine, and does it fit right now? The single decision used before a download
/// (rough or from the header) and when the model is loaded.
pub fn assess(hw: &Hardware, live: &Live, s: &Subject) -> Assessment {
    let n = need(s);
    let (machine, now, now_cap, in_use, class) = pools(hw, live, s.moe);
    let ram_ok = hw.total_ram_gb + 0.5 >= s.min_ram_gb;
    let (mut fits_machine, spills) = classify(n.total_gb, &machine);
    let (fits_now, _) = classify(n.total_gb, &now);
    if !ram_ok {
        fits_machine = Fit::TooLarge;
    }
    // Memory that other programs hold can be freed: that alone never makes the model "too large".
    let fit = match (fits_machine, fits_now) {
        (Fit::TooLarge, _) => Fit::TooLarge,
        (m, Fit::TooLarge) => worse(m, Fit::Tight),
        (m, n) => worse(m, n),
    };
    let gpu = class != MemoryClass::CpuOnly;
    let small = s.params_b <= 4.5;
    let speed = match (fit, class) {
        (Fit::TooLarge, _) => Speed::Slow,
        _ if s.moe => {
            if gpu {
                Speed::Moderate
            } else {
                Speed::Slow
            }
        }
        _ if spills => Speed::Moderate,
        (_, MemoryClass::CpuOnly) => {
            if small && fit == Fit::Comfortable {
                Speed::Moderate
            } else {
                Speed::Slow
            }
        }
        _ => Speed::Fast,
    };
    let mut a = Assessment {
        need_gb: n.total_gb,
        weights_gb: n.weights_gb,
        context_gb: n.kv_gb + n.compute_gb,
        context: s.context,
        rough: n.rough,
        fit,
        speed,
        fits_machine,
        fits_now,
        capacity_gb: machine.cap,
        available_now_gb: now_cap,
        in_use_gb: in_use,
        spills,
        advice: Vec::new(),
    };
    if fit != Fit::Comfortable {
        a.advice = advise(hw, live, s, &a);
    }
    a
}

fn advise(hw: &Hardware, live: &Live, s: &Subject, a: &Assessment) -> Vec<Advice> {
    let mut out = Vec::new();
    // The largest halving of the context that makes it comfortable.
    let mut c = s.context / 2;
    while c >= MIN_ADVISED_CONTEXT {
        if assess_no_advice(hw, live, &s.with_context(c)) == Fit::Comfortable {
            out.push(Advice::ShorterContext { tokens: c });
            break;
        }
        c /= 2;
    }
    if a.fits_machine == Fit::TooLarge || out.is_empty() {
        out.push(Advice::SmallerQuant);
    }
    if a.fits_machine != Fit::TooLarge && a.fits_now != Fit::Comfortable {
        out.push(Advice::CloseApps {
            gb: (a.need_gb - a.available_now_gb).max(0.0),
        });
    }
    out
}

fn assess_no_advice(hw: &Hardware, live: &Live, s: &Subject) -> Fit {
    let n = need(s);
    let (machine, now, ..) = pools(hw, live, s.moe);
    let (m, _) = classify(n.total_gb, &machine);
    let (nw, _) = classify(n.total_gb, &now);
    worse(m, nw)
}

/// KV shapes read from model headers, by catalog file name: filled once per model and shared by every verdict.
#[derive(Debug, Default)]
pub struct ShapeCache(std::sync::Mutex<std::collections::HashMap<String, KvShape>>);

impl ShapeCache {
    fn map(&self) -> std::sync::MutexGuard<'_, std::collections::HashMap<String, KvShape>> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn insert(&self, file: &str, shape: KvShape) {
        self.map().insert(file.to_string(), shape);
    }

    pub fn contains(&self, file: &str) -> bool {
        self.map().contains_key(file)
    }

    pub fn snapshot(&self) -> std::collections::HashMap<String, KvShape> {
        self.map().clone()
    }
}

#[cfg(test)]
mod tests;
