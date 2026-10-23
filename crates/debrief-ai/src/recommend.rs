//! Pure recommendation logic: how much memory a model needs, whether it fits this machine, how
//! fast it will run, which catalog model to suggest, and how many layers to put on the GPU.

use crate::catalog::{Catalog, CatalogModel};
use crate::fit::{KvShape, Live, Subject, assess};
use crate::hardware::Hardware;
use crate::settings::AiSettings;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Quant {
    Q2,
    Q3,
    Q4,
    Q5,
    Q6,
    Q8,
    F16,
    F32,
}

impl Quant {
    /// GB of weights per billion parameters.
    pub fn gb_per_b(self) -> f64 {
        match self {
            Quant::Q2 => 0.4,
            Quant::Q3 => 0.5,
            Quant::Q4 => 0.6,
            Quant::Q5 => 0.7,
            Quant::Q6 => 0.85,
            Quant::Q8 => 1.1,
            Quant::F16 => 2.0,
            Quant::F32 => 4.0,
        }
    }

    /// Understands "Q4_K_M", "Q8_0", "F16", "BF16", "IQ4_XS", ...
    pub fn parse(s: &str) -> Option<Quant> {
        let u = s.trim().to_uppercase();
        let u = u.strip_prefix("IQ").map(|r| format!("Q{r}")).unwrap_or(u);
        match u.as_str() {
            "F16" | "BF16" => return Some(Quant::F16),
            "F32" => return Some(Quant::F32),
            _ => {}
        }
        match u.strip_prefix('Q')?.chars().next()? {
            '2' => Some(Quant::Q2),
            '3' => Some(Quant::Q3),
            '4' => Some(Quant::Q4),
            '5' => Some(Quant::Q5),
            '6' => Some(Quant::Q6),
            '8' => Some(Quant::Q8),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MemoryClass {
    DiscreteGpu,
    Unified,
    CpuOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Budget {
    pub usable_gb: f64,
    pub class: MemoryClass,
}

/// GPUs with less than this are not worth offloading to.
const MIN_USEFUL_VRAM_GB: f64 = 2.0;

/// The default GPU working-set limit of Apple silicon (Metal's `recommendedMaxWorkingSetSize`): two thirds of the
/// memory up to 36 GB, three quarters above. (llama.cpp lists 21845 MiB for a 32 GB Mac.)
pub fn unified_working_set_gb(total_ram_gb: f64) -> f64 {
    total_ram_gb * if total_ram_gb <= 36.0 { 2.0 / 3.0 } else { 0.75 }
}

pub fn budget(hw: &Hardware) -> Budget {
    let discrete = hw
        .gpus
        .iter()
        .filter(|g| !g.unified)
        .filter_map(|g| g.vram_gb)
        .filter(|v| *v >= MIN_USEFUL_VRAM_GB)
        .fold(0.0, f64::max);
    let unified = if hw.gpus.iter().any(|g| g.unified) {
        unified_working_set_gb(hw.total_ram_gb)
    } else {
        0.0
    };
    if unified > 0.0 && unified >= discrete {
        Budget {
            usable_gb: unified,
            class: MemoryClass::Unified,
        }
    } else if discrete > 0.0 {
        Budget {
            usable_gb: discrete,
            class: MemoryClass::DiscreteGpu,
        }
    } else {
        Budget {
            usable_gb: hw.available_ram_gb * 0.5,
            class: MemoryClass::CpuOnly,
        }
    }
}

pub use crate::fit::{Assessment, Fit};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Speed {
    Fast,
    Moderate,
    Slow,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Recommendation {
    pub model: CatalogModel,
    pub assessment: Assessment,
    pub installed: bool,
    /// Context length to load with (smaller when memory is tight).
    pub context: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Recommendations {
    pub hardware: String,
    pub budget: Budget,
    /// Best quality that fits, preferring models trained for tool calling.
    pub best: Option<Recommendation>,
    /// Smallest good tool-caller, for speed.
    pub fastest: Option<Recommendation>,
    /// Everything, best first.
    pub ranked: Vec<Recommendation>,
    /// Plain-language advice, e.g. when nothing fits.
    pub message: String,
}

fn fit_rank(f: Fit) -> u8 {
    match f {
        Fit::Comfortable => 0,
        Fit::Tight => 1,
        Fit::TooLarge => 2,
    }
}

/// `installed_files` are file names already in the models folder. Uses the default context and no header
/// information (rough KV estimates); see [`recommend_with`].
pub fn recommend(hw: &Hardware, catalog: &Catalog, installed_files: &[String]) -> Recommendations {
    recommend_with(
        hw,
        &Live::default(),
        catalog,
        installed_files,
        AiSettings::default().context_length,
        &HashMap::new(),
    )
}

/// As [`recommend`], with the live state of the machine, the context Debrief will start the server with, and the
/// KV shapes read from model headers (keyed by catalog file name; others get a rough estimate).
pub fn recommend_with(
    hw: &Hardware,
    live: &Live,
    catalog: &Catalog,
    installed_files: &[String],
    context: usize,
    shapes: &HashMap<String, KvShape>,
) -> Recommendations {
    let b = budget(hw);
    let mut ranked: Vec<Recommendation> = catalog
        .models
        .iter()
        .map(|m| {
            // A projector (ADR 0025) goes to the GPU only when the whole model does, else it stays in system RAM:
            // the fit on the GPU is the model's.
            let subject = Subject {
                weights_bytes: m.size_bytes,
                projector_bytes: 0,
                shape: shapes.get(&m.file).copied().unwrap_or_else(|| KvShape::from_params(m.params_b)),
                params_b: m.params_b,
                moe: m.moe,
                min_ram_gb: m.min_ram_gb,
                context,
            };
            let a = assess(hw, live, &subject);
            Recommendation {
                model: m.clone(),
                installed: installed_files.iter().any(|f| f == &m.file),
                context: a.context,
                assessment: a,
            }
        })
        .collect();
    // Best first: fits before not, tool callers before others, dense before MoE (advanced), then size.
    ranked.sort_by(|x, y| {
        let key = |r: &Recommendation| (fit_rank(r.assessment.fit), !r.model.tool_calling, r.model.moe);
        key(x).cmp(&key(y)).then(y.model.params_b.total_cmp(&x.model.params_b))
    });
    let fits = |r: &&Recommendation| r.assessment.fit == Fit::Comfortable;
    let best = ranked
        .iter()
        .find(fits)
        .or_else(|| ranked.iter().find(|r| r.assessment.fit == Fit::Tight))
        .cloned();
    let fastest = ranked
        .iter()
        .filter(|r| r.assessment.fit != Fit::TooLarge && r.model.tool_calling && !r.model.moe)
        .min_by(|x, y| {
            let small = |r: &Recommendation| r.model.params_b < 3.0;
            small(x).cmp(&small(y)).then(x.model.params_b.total_cmp(&y.model.params_b))
        })
        .cloned();
    let message = match (&best, b.class) {
        (None, _) => "No catalog model fits this machine. Close other programs, or add a smaller GGUF model yourself.".to_string(),
        (Some(r), MemoryClass::CpuOnly) => format!(
            "No usable GPU was found, so {} will run on the CPU: {}.",
            r.model.name,
            speed_word(r.assessment.speed)
        ),
        (Some(r), _) if r.assessment.fit == Fit::Tight => format!(
            "{} only just fits; expect it to be slower, or choose a smaller model.",
            r.model.name
        ),
        (Some(r), _) => format!("{} fits comfortably.", r.model.name),
    };
    Recommendations {
        hardware: hw.summary(),
        budget: b,
        best,
        fastest,
        ranked,
        message,
    }
}

fn speed_word(s: Speed) -> &'static str {
    match s {
        Speed::Fast => "fast",
        Speed::Moderate => "moderate speed",
        Speed::Slow => "slow",
    }
}

/// Where to run the model, decided from the memory that is free right now.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OffloadPlan {
    /// Layers to place on the GPU (`u32::MAX` = all). 0 = CPU only.
    pub gpu_layers: u32,
    /// Keep MoE expert tensors in system RAM.
    pub moe_to_cpu: bool,
    pub context: usize,
    pub note: String,
}

pub struct OffloadInput {
    pub file_bytes: u64,
    pub block_count: u32,
    /// KV cache bytes for `context` (see `fit::KvShape::bytes`).
    pub kv_bytes: u64,
    pub context: usize,
    /// Free video memory (or the shared-memory budget on Apple Silicon); `None` = no usable GPU.
    pub free_gpu_gb: Option<f64>,
    pub moe: bool,
}

pub fn plan_offload(i: &OffloadInput) -> OffloadPlan {
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    let cpu = |note: &str| OffloadPlan {
        gpu_layers: 0,
        moe_to_cpu: false,
        context: i.context,
        note: note.to_string(),
    };
    let Some(free) = i.free_gpu_gb.filter(|f| *f >= MIN_USEFUL_VRAM_GB) else {
        return cpu("no usable GPU memory: running on the CPU");
    };
    let file_gb = i.file_bytes as f64 / GB;
    let kv_gb = i.kv_bytes as f64 / GB;
    let room = free - kv_gb - crate::fit::compute_buffer_gb(i.context);
    if room >= file_gb {
        return OffloadPlan {
            gpu_layers: u32::MAX,
            moe_to_cpu: false,
            context: i.context,
            note: "the whole model fits on the GPU".into(),
        };
    }
    if i.moe {
        return OffloadPlan {
            gpu_layers: u32::MAX,
            moe_to_cpu: true,
            context: i.context,
            note: "attention on the GPU, expert layers in system RAM".into(),
        };
    }
    let layers = i.block_count.max(1);
    let per_layer = file_gb / (layers as f64 + 1.0);
    let fit = if room <= 0.0 { 0 } else { (room / per_layer).floor() as u32 };
    if fit == 0 {
        return cpu("the model does not fit the GPU memory that is free: running on the CPU");
    }
    OffloadPlan {
        gpu_layers: fit.min(layers),
        moe_to_cpu: false,
        context: i.context,
        note: format!("{} of {} layers on the GPU, the rest on the CPU", fit.min(layers), layers),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::Gpu;

    fn hw(total: f64, avail: f64, gpus: Vec<Gpu>) -> Hardware {
        Hardware {
            os: "x".into(),
            arch: "x".into(),
            cpu_cores: 8,
            total_ram_gb: total,
            available_ram_gb: avail,
            gpus,
        }
    }
    fn discrete(v: f64) -> Gpu {
        Gpu {
            name: "GPU".into(),
            vram_gb: Some(v),
            unified: false,
        }
    }
    fn rec(h: &Hardware) -> Recommendations {
        recommend(h, &Catalog::embedded(), &[])
    }
    fn fit_of(r: &Recommendations, id: &str) -> Fit {
        r.ranked.iter().find(|x| x.model.id == id).unwrap().assessment.fit
    }

    #[test]
    fn quant_parsing_and_estimates() {
        for (s, q) in [
            ("Q4_K_M", Quant::Q4),
            ("q8_0", Quant::Q8),
            ("F16", Quant::F16),
            ("BF16", Quant::F16),
            ("IQ3_XS", Quant::Q3),
            ("Q6_K", Quant::Q6),
            ("Q5_K_S", Quant::Q5),
            ("Q2_K", Quant::Q2),
            ("F32", Quant::F32),
        ] {
            assert_eq!(Quant::parse(s), Some(q), "{s}");
        }
        assert_eq!(Quant::parse("weird"), None);
        assert_eq!(Quant::parse("Q9"), None);
        assert_eq!(Quant::parse(""), None);
    }

    #[test]
    fn budgets() {
        assert_eq!(budget(&hw(16.0, 10.0, vec![discrete(8.0)])).class, MemoryClass::DiscreteGpu);
        assert_eq!(budget(&hw(16.0, 10.0, vec![discrete(8.0), discrete(12.0)])).usable_gb, 12.0);
        let apple = budget(&hw(
            32.0,
            20.0,
            vec![Gpu {
                name: "A".into(),
                vram_gb: Some(32.0),
                unified: true,
            }],
        ));
        assert_eq!(apple.class, MemoryClass::Unified);
        assert!((apple.usable_gb - 32.0 * 2.0 / 3.0).abs() < 1e-9);
        assert!((unified_working_set_gb(64.0) - 48.0).abs() < 1e-9);
        let cpu = budget(&hw(
            8.0,
            6.0,
            vec![
                discrete(0.5),
                Gpu {
                    name: "m".into(),
                    vram_gb: None,
                    unified: false,
                },
            ],
        ));
        assert_eq!(
            cpu,
            Budget {
                usable_gb: 3.0,
                class: MemoryClass::CpuOnly
            }
        );
    }

    #[test]
    fn eight_gb_cpu_only_machine_gets_the_small_model() {
        let r = rec(&hw(8.0, 6.0, vec![]));
        assert_eq!(r.best.as_ref().unwrap().model.id, "qwen3.5-2b-q4km");
        // with its context the 2B model only just fits 3 GB of free memory
        assert_eq!(r.best.as_ref().unwrap().assessment.fit, Fit::Tight);
        assert_eq!(r.best.as_ref().unwrap().assessment.speed, Speed::Slow);
        assert_eq!(fit_of(&r, "qwen3.5-9b-q4km"), Fit::TooLarge);
        assert!(r.message.contains("CPU"));
    }

    #[test]
    fn sixteen_gb_cpu_only_prefers_the_4b() {
        let r = rec(&hw(16.0, 12.0, vec![]));
        let best = r.best.clone().unwrap();
        assert_eq!(best.model.id, "qwen3.5-4b-q4km");
        assert_eq!(best.assessment.speed, Speed::Moderate);
        assert_eq!(fit_of(&r, "qwen3.5-9b-q4km"), Fit::TooLarge);
    }

    #[test]
    fn eight_gb_gpu_with_16_gb_ram() {
        let r = rec(&hw(16.0, 11.0, vec![discrete(8.0)]));
        // the 9B model (5.3 GB + its 8k context) is no longer called comfortable on an 8 GB card
        assert_eq!(fit_of(&r, "qwen3.5-9b-q4km"), Fit::Tight);
        let best = r.best.as_ref().unwrap();
        assert_eq!(best.model.id, "qwen3.5-4b-q4km");
        assert_eq!(best.assessment.fit, Fit::Comfortable);
        assert_eq!(best.assessment.speed, Speed::Fast);
        assert_eq!(best.context, 8192);
        assert_eq!(r.fastest.as_ref().unwrap().model.id, "qwen3.5-4b-q4km");
        // a 12B model just fits: tight, with a smaller context
        let gemma = r.ranked.iter().find(|x| x.model.id == "gemma-4-12b-qat-q4_0").unwrap();
        assert_eq!(gemma.assessment.fit, Fit::Tight);
        assert!(gemma.assessment.rough && !gemma.assessment.advice.is_empty());
        // a larger 12-14B file spills into system RAM: tight and slower
        let spill = assess(
            &hw(16.0, 11.0, vec![discrete(8.0)]),
            &Live::default(),
            &Subject {
                weights_bytes: 8_500_000_000,
                projector_bytes: 0,
                shape: KvShape::from_params(12.0),
                params_b: 12.0,
                moe: false,
                min_ram_gb: 0.0,
                context: 4096,
            },
        );
        assert_eq!((spill.fit, spill.speed), (Fit::Tight, Speed::Moderate));
        assert_eq!(fit_of(&r, "qwen3.5-27b-q4km"), Fit::TooLarge);
        assert_eq!(fit_of(&r, "qwen3.6-35b-a3b-ud-q4km"), Fit::TooLarge);
        assert!(r.message.contains("fits comfortably"));
        // tool callers rank above the non-tool callers that also fit
        let pos = |id: &str| r.ranked.iter().position(|x| x.model.id == id).unwrap();
        assert!(pos("qwen3-4b-instruct-2507-q4km") < pos("phi-4-mini-q4km"));
    }

    #[test]
    fn eight_gb_gpu_with_32_gb_ram_adds_the_moe_option() {
        let r = rec(&hw(32.0, 24.0, vec![discrete(8.0)]));
        assert_eq!(r.best.as_ref().unwrap().model.id, "qwen3.5-4b-q4km");
        let moe = r.ranked.iter().find(|x| x.model.moe).unwrap();
        assert_ne!(moe.assessment.fit, Fit::TooLarge);
        assert_eq!(moe.assessment.speed, Speed::Moderate);
        assert_ne!(r.best.as_ref().unwrap().model.id, moe.model.id);
        assert_eq!(fit_of(&r, "gemma-4-12b-qat-q4_0"), Fit::Tight);
        assert_ne!(fit_of(&r, "qwen3.5-27b-q4km"), Fit::Comfortable);
    }

    #[test]
    fn apple_silicon_32_gb() {
        let r = rec(&hw(
            32.0,
            20.0,
            vec![Gpu {
                name: "Apple".into(),
                vram_gb: Some(32.0),
                unified: true,
            }],
        ));
        // the working set is 21.3 GB but only 20 GB are free (minus a margin): the 27B file (15.6 GB + context)
        // fits the machine yet not comfortably right now
        assert_eq!(r.best.as_ref().unwrap().model.id, "qwen3.5-9b-q4km");
        let big = &r.ranked.iter().find(|x| x.model.id == "qwen3.5-27b-q4km").unwrap().assessment;
        assert_eq!(big.fit, Fit::Tight);
        assert_eq!(fit_of(&r, "qwen3.5-9b-q4km"), Fit::Comfortable);
        assert_eq!(r.budget.class, MemoryClass::Unified);
    }

    #[test]
    fn sixty_four_gb_with_24_gb_gpu() {
        let r = rec(&hw(64.0, 50.0, vec![discrete(24.0)]));
        assert_eq!(r.best.as_ref().unwrap().model.id, "qwen3.5-27b-q4km");
        assert_eq!(fit_of(&r, "qwen3.5-27b-q4km"), Fit::Comfortable);
        let moe = r.ranked.iter().find(|x| x.model.moe).unwrap();
        assert_eq!(moe.assessment.fit, Fit::Comfortable);
    }

    #[test]
    fn six_gb_gpu_gets_the_4b_and_no_gpu_info_means_cpu() {
        let r = rec(&hw(16.0, 12.0, vec![discrete(6.0)]));
        // 6 GB card, an unknown free figure assumes the desktop uses 1 GB: the 4B model is tight, the 2B comfortable
        assert_eq!(r.best.as_ref().unwrap().model.id, "qwen3.5-2b-q4km");
        assert_eq!(fit_of(&r, "qwen3.5-4b-q4km"), Fit::Tight);
        let unknown = rec(&hw(
            16.0,
            12.0,
            vec![Gpu {
                name: "?".into(),
                vram_gb: None,
                unified: false,
            }],
        ));
        assert_eq!(unknown.budget.class, MemoryClass::CpuOnly);
    }

    #[test]
    fn tiny_machine_has_no_recommendation() {
        let r = rec(&hw(2.0, 1.0, vec![]));
        assert!(r.best.is_none() && r.fastest.is_none());
        assert!(r.message.contains("No catalog model fits"));
    }

    #[test]
    fn installed_models_are_flagged_and_tight_models_get_a_warning() {
        let h = hw(16.0, 11.0, vec![discrete(8.0)]);
        let r = recommend(&h, &Catalog::embedded(), &["Qwen3.5-4B-Q4_K_M.gguf".to_string()]);
        assert!(r.ranked.iter().find(|x| x.model.id == "qwen3.5-4b-q4km").unwrap().installed);
        assert!(!r.ranked.iter().find(|x| x.model.id == "qwen3.5-9b-q4km").unwrap().installed);
        let mut only = Catalog::embedded();
        only.models.retain(|m| m.id == "gemma-4-12b-qat-q4_0");
        let t = recommend(&h, &only, &[]);
        assert!(t.message.contains("only just fits"));
        assert!(t.fastest.is_none());
    }

    #[test]
    fn cpu_speed_classes() {
        let h = hw(32.0, 24.0, vec![]);
        let a = |h: &Hardware, gb: u64, params: f64, moe: bool, min_ram: f64| {
            assess(
                h,
                &Live::default(),
                &Subject {
                    weights_bytes: gb << 30,
                    projector_bytes: 0,
                    shape: KvShape::from_params(params),
                    params_b: params,
                    moe,
                    min_ram_gb: min_ram,
                    context: 4096,
                },
            )
        };
        assert_eq!(a(&h, 3, 4.0, false, 0.0).speed, Speed::Moderate);
        assert_eq!(a(&h, 5, 9.0, false, 0.0).speed, Speed::Slow);
        let tight = a(&hw(16.0, 5.0, vec![]), 1, 9.0, false, 0.0);
        assert_eq!((tight.fit, tight.speed), (Fit::Tight, Speed::Slow));
        let moe_cpu = a(&h, 22, 35.0, true, 32.0);
        assert_eq!(moe_cpu.speed, Speed::Slow);
        assert_eq!(a(&hw(16.0, 12.0, vec![]), 22, 35.0, true, 32.0).fit, Fit::TooLarge);
    }

    fn plan(free: Option<f64>, moe: bool, file_gb: f64) -> OffloadPlan {
        plan_offload(&OffloadInput {
            file_bytes: (file_gb * 1024.0 * 1024.0 * 1024.0) as u64,
            block_count: 32,
            kv_bytes: 131072 * 8192,
            context: 8192,
            free_gpu_gb: free,
            moe,
        })
    }

    #[test]
    fn offload_planning() {
        assert_eq!(plan(None, false, 5.0).gpu_layers, 0);
        assert_eq!(plan(Some(1.0), false, 5.0).gpu_layers, 0);
        assert_eq!(plan(Some(8.0), false, 5.0).gpu_layers, u32::MAX);
        let part = plan(Some(5.0), false, 6.9);
        assert!(part.gpu_layers > 0 && part.gpu_layers < 32, "{part:?}");
        assert!(part.note.contains("of 32 layers"));
        assert_eq!(plan(Some(2.0), false, 20.0).gpu_layers, 0);
        let moe = plan(Some(8.0), true, 22.0);
        assert!(moe.moe_to_cpu && moe.gpu_layers == u32::MAX);
        assert!(!plan(Some(8.0), true, 3.0).moe_to_cpu);
        assert!(plan(Some(2.2), false, 3.0).gpu_layers > 0);
    }
}
