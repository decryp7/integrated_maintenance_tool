use super::*;
use crate::gguf::parse;
use crate::hardware::Gpu;
use crate::testing::gguf::{Kv, build};
use std::io::Cursor;

const GB: u64 = 1024 * 1024 * 1024;

fn info(arch: &str, kvs: &[(&str, Kv)]) -> GgufInfo {
    let b = build(arch, kvs, 1 << 17);
    parse(Cursor::new(b.clone()), b.len() as u64).unwrap()
}

/// Qwen3-8B: 36 layers, 32 heads, 8 KV heads, embedding 4096, head size 128 (values from its published config).
fn qwen3_8b() -> GgufInfo {
    info(
        "qwen3",
        &[
            ("qwen3.block_count", Kv::U32(36)),
            ("qwen3.embedding_length", Kv::U32(4096)),
            ("qwen3.attention.head_count", Kv::U32(32)),
            ("qwen3.attention.head_count_kv", Kv::U32(8)),
            ("qwen3.attention.key_length", Kv::U32(128)),
            ("qwen3.attention.value_length", Kv::U32(128)),
        ],
    )
}

/// Qwen3-32B: 64 layers, 64 heads, 8 KV heads, embedding 5120, head size 128.
fn qwen3_32b() -> GgufInfo {
    info(
        "qwen3",
        &[
            ("qwen3.block_count", Kv::U32(64)),
            ("qwen3.embedding_length", Kv::U32(5120)),
            ("qwen3.attention.head_count", Kv::U32(64)),
            ("qwen3.attention.head_count_kv", Kv::U32(8)),
            ("qwen3.attention.key_length", Kv::U32(128)),
        ],
    )
}

/// Gemma 3 12B: 48 layers, 16 heads, 8 KV heads, embedding 3840, head size 256, 1024-token window on 5 of 6 layers.
fn gemma3_12b() -> GgufInfo {
    info(
        "gemma3",
        &[
            ("gemma3.block_count", Kv::U32(48)),
            ("gemma3.embedding_length", Kv::U32(3840)),
            ("gemma3.attention.head_count", Kv::U32(16)),
            ("gemma3.attention.head_count_kv", Kv::U32(8)),
            ("gemma3.attention.key_length", Kv::U32(256)),
            ("gemma3.attention.value_length", Kv::U32(256)),
            ("gemma3.attention.sliding_window", Kv::U32(1024)),
        ],
    )
}

fn kv_bytes(i: &GgufInfo, ctx: usize) -> u64 {
    KvShape::from_gguf(i).bytes(ctx, 1, KvType::F16, KvType::F16)
}

#[test]
fn kv_cache_of_a_dense_model_follows_its_header() {
    let i = qwen3_8b();
    // 36 layers x 8 KV heads x (128 key + 128 value) x 2 bytes = 147456 bytes per token
    assert_eq!(KvShape::from_gguf(&i).bytes_per_token_full(), 147_456);
    assert_eq!(kv_bytes(&i, 32768), 147_456 * 32768);
    assert_eq!(kv_bytes(&qwen3_32b(), 32768), 64 * 8 * 256 * 2 * 32768);
}

#[test]
fn head_size_defaults_to_embedding_over_heads_and_quantised_cache_is_smaller() {
    let i = info(
        "llama",
        &[
            ("llama.block_count", Kv::U32(32)),
            ("llama.embedding_length", Kv::U32(4096)),
            ("llama.attention.head_count", Kv::U32(32)),
            ("llama.attention.head_count_kv", Kv::U32(8)),
        ],
    );
    assert_eq!(kv_bytes(&i, 8192), 32 * 8 * 128 * 2 * 2 * 8192);
    let s = KvShape::from_gguf(&i);
    assert!(s.bytes(8192, 1, KvType::Q8_0, KvType::Q8_0) < s.bytes(8192, 1, KvType::F16, KvType::F16));
}

#[test]
fn sliding_window_layers_cache_only_their_window() {
    let i = gemma3_12b();
    // 8 full-attention layers + 40 window layers of 1024 + 512 positions
    let per_pos = 8 * (256 + 256) * 2;
    assert_eq!(kv_bytes(&i, 32768), 8 * per_pos * 32768 + 40 * per_pos * 1536);
    // a context shorter than the window caches only the context
    assert_eq!(kv_bytes(&i, 1000), 48 * per_pos * 1000);
    // far smaller than if every layer cached the whole context
    assert!(kv_bytes(&i, 32768) * 3 < KvShape::from_gguf(&i).bytes_per_token_full() * 32768);
}

#[test]
fn hybrid_models_cache_only_their_attention_layers() {
    let i = info(
        "qwen35",
        &[
            ("qwen35.block_count", Kv::U32(40)),
            ("qwen35.embedding_length", Kv::U32(2048)),
            ("qwen35.attention.head_count", Kv::U32(16)),
            ("qwen35.attention.head_count_kv", Kv::U32(2)),
            ("qwen35.attention.key_length", Kv::U32(256)),
            ("qwen35.full_attention_interval", Kv::U32(4)),
        ],
    );
    assert_eq!(kv_bytes(&i, 4096), 10 * 2 * 512 * 2 * 4096);
}

#[test]
fn an_unreadable_layout_gets_a_rough_cache_with_a_margin() {
    let i = info("mystery", &[]);
    assert!(KvShape::from_gguf(&i).is_rough());
    assert_eq!(KvShape::from_gguf(&i).bytes_per_token_full(), 128 * 1024);
    let rough = KvShape::from_params(8.0);
    assert!(rough.is_rough());
    assert!(rough.bytes(8192, 1, KvType::F16, KvType::F16) > 160 * 1024 * 8192);
    assert!(KvShape::from_params(70.0).bytes(8192, 1, KvType::F16, KvType::F16) > rough.bytes(8192, 1, KvType::F16, KvType::F16));
}

fn mac(total: f64, available: f64) -> Hardware {
    Hardware {
        os: "macos".into(),
        arch: "aarch64".into(),
        cpu_cores: 14,
        total_ram_gb: total,
        available_ram_gb: available,
        gpus: vec![Gpu {
            name: "Apple Silicon GPU".into(),
            vram_gb: Some(total),
            unified: true,
        }],
    }
}

fn laptop(vram: f64, ram: f64, avail: f64) -> Hardware {
    Hardware {
        os: "windows".into(),
        arch: "x86_64".into(),
        cpu_cores: 16,
        total_ram_gb: ram,
        available_ram_gb: avail,
        gpus: vec![Gpu {
            name: "NVIDIA GeForce RTX".into(),
            vram_gb: Some(vram),
            unified: false,
        }],
    }
}

fn subject(i: &GgufInfo, weights_gb: f64, ctx: usize) -> Subject {
    Subject {
        weights_bytes: (weights_gb * GB as f64) as u64,
        params_b: 8.0,
        ..Subject::from_gguf(i, 0, ctx)
    }
}

#[test]
fn a_big_model_at_a_long_context_on_a_busy_mac_is_not_comfortable() {
    // 64 GB Mac, 38 GB in use by other programs, a 28 GB model with a 32k context: the old judgement
    // (weights + 2 GB against 65% of the total RAM) said "fits comfortably".
    let hw = mac(64.0, 26.0);
    let a = assess(&hw, &Live::default(), &subject(&qwen3_32b(), 28.0, 32768));
    assert_eq!(a.fits_machine, Fit::Comfortable, "{a:?}");
    assert_eq!(a.fits_now, Fit::TooLarge, "{a:?}");
    assert_eq!(a.fit, Fit::Tight, "never comfortable while the check for now fails");
    assert!((a.capacity_gb - 48.0).abs() < 1e-9, "the GPU working set is 3/4 of 64 GB");
    assert!(a.available_now_gb < 24.0 && a.in_use_gb > 20.0, "{a:?}");
    assert!((a.weights_gb - 28.0).abs() < 1e-9);
    assert!((a.need_gb - (28.0 + a.context_gb)).abs() < 1e-9);
    assert!(a.context_gb > 8.0, "the KV cache alone is 8 GB: {a:?}");
    assert!(
        a.advice.iter().any(|x| matches!(x, Advice::CloseApps { gb } if *gb > 10.0)),
        "{:?}",
        a.advice
    );
}

#[test]
fn the_same_mac_when_idle_is_comfortable_and_a_loaded_model_counts_as_free() {
    let idle = assess(&mac(64.0, 55.0), &Live::default(), &subject(&qwen3_32b(), 20.0, 8192));
    assert_eq!(idle.fit, Fit::Comfortable, "{idle:?}");
    assert!(idle.advice.is_empty());
    let busy = mac(64.0, 6.0);
    let s = subject(&qwen3_32b(), 20.0, 8192);
    assert_ne!(assess(&busy, &Live::default(), &s).fits_now, Fit::Comfortable);
    // Debrief's own 20 GB model would be unloaded first
    let freed = Live {
        reclaim_gb: 30.0,
        ..Live::default()
    };
    assert_eq!(assess(&busy, &freed, &s).fits_now, Fit::Comfortable);
}

#[test]
fn the_working_set_reported_by_the_system_wins_over_the_default_fraction() {
    let live = Live {
        gpu_working_set_gb: Some(40.0),
        ..Live::default()
    };
    let a = assess(&mac(64.0, 60.0), &live, &subject(&qwen3_8b(), 5.0, 8192));
    assert!((a.capacity_gb - 40.0).abs() < 1e-9);
}

#[test]
fn an_eight_gb_laptop_gpu_that_is_already_in_use_is_not_comfortable_for_an_8b_model() {
    // 8 GB video memory, 16 GB RAM, 1 GB of video memory used by the desktop and the browser, a 4.9 GB Q4
    // model at the default 8k context: the old judgement (file + 1 GB against 0.88 x 8 GB) said comfortable.
    let hw = laptop(8.0, 16.0, 9.0);
    let live = Live {
        free_vram_gb: Some(7.0),
        ..Live::default()
    };
    let llama_8b = info(
        "llama",
        &[
            ("llama.block_count", Kv::U32(32)),
            ("llama.embedding_length", Kv::U32(4096)),
            ("llama.attention.head_count", Kv::U32(32)),
            ("llama.attention.head_count_kv", Kv::U32(8)),
        ],
    );
    let a = assess(&hw, &live, &subject(&llama_8b, 4.9, 8192));
    assert_ne!(a.fit, Fit::Comfortable, "{a:?}");
    assert_eq!(a.fit, Fit::Tight);
    assert!(a.weights_gb > 4.8 && a.context_gb > 1.5, "{a:?}");
    assert!((a.available_now_gb - 6.75).abs() < 1e-9, "7 GB free minus the safety margin: {a:?}");
    assert!((a.in_use_gb - 1.0).abs() < 1e-9, "{a:?}");
    // a 6 GB file does not fit the card at all once the context is added: it spills to system RAM
    let big = assess(&hw, &live, &subject(&llama_8b, 6.5, 8192));
    assert_eq!(big.fit, Fit::Tight);
    assert!(big.spills && big.speed == Speed::Moderate, "{big:?}");
    // an unknown free figure assumes the desktop uses 1 GB
    let unknown = assess(&hw, &Live::default(), &subject(&llama_8b, 4.9, 8192));
    assert!((unknown.available_now_gb - 6.75).abs() < 1e-9);
}

#[test]
fn a_small_model_on_the_same_laptop_is_comfortable() {
    let live = Live {
        free_vram_gb: Some(7.0),
        ..Live::default()
    };
    let a = assess(&laptop(8.0, 16.0, 9.0), &live, &subject(&qwen3_8b(), 2.5, 4096));
    assert_eq!(a.fit, Fit::Comfortable, "{a:?}");
    assert_eq!(a.speed, Speed::Fast);
}

#[test]
fn shorter_context_and_smaller_quant_are_suggested() {
    let hw = laptop(8.0, 16.0, 9.0);
    let live = Live {
        free_vram_gb: Some(7.0),
        ..Live::default()
    };
    // weights fit, the 32k context does not: shorten it
    let a = assess(&hw, &live, &subject(&qwen3_8b(), 3.5, 32768));
    assert_ne!(a.fit, Fit::Comfortable);
    assert!(
        a.advice
            .iter()
            .any(|x| matches!(x, Advice::ShorterContext { tokens } if *tokens < 32768 && *tokens >= 2048)),
        "{:?}",
        a.advice
    );
    // the weights alone are too much: no context helps
    let huge = assess(&hw, &live, &subject(&qwen3_8b(), 40.0, 8192));
    assert_eq!(huge.fit, Fit::TooLarge);
    assert!(huge.advice.contains(&Advice::SmallerQuant));
    assert!(
        !huge
            .advice
            .iter()
            .any(|x| matches!(x, Advice::ShorterContext { .. } | Advice::CloseApps { .. }))
    );
}

#[test]
fn a_rough_estimate_is_labelled_and_wider_than_an_exact_one() {
    let hw = mac(64.0, 60.0);
    let exact = assess(&hw, &Live::default(), &subject(&qwen3_8b(), 5.0, 8192));
    let rough = assess(
        &hw,
        &Live::default(),
        &Subject {
            shape: KvShape::from_params(8.0),
            ..subject(&qwen3_8b(), 5.0, 8192)
        },
    );
    assert!(!exact.rough && rough.rough);
    assert!(rough.context_gb > exact.context_gb);
}

#[test]
fn before_and_after_download_agree() {
    // The header of the downloaded file gives the same subject as the catalog entry plus the fetched header.
    let i = qwen3_32b();
    let before = Subject {
        weights_bytes: i.file_size,
        projector_bytes: 0,
        shape: KvShape::from_gguf(&i),
        params_b: i.params_b,
        moe: false,
        min_ram_gb: 0.0,
        context: 16384,
    };
    let after = Subject::from_gguf(&i, 0, 16384);
    assert_eq!(before, after);
    for hw in [mac(8.0, 3.0), mac(64.0, 20.0), laptop(8.0, 16.0, 8.0), laptop(24.0, 64.0, 40.0)] {
        let live = Live::default();
        assert_eq!(assess(&hw, &live, &before), assess(&hw, &live, &after));
        // and the load-time guard refuses exactly what the verdict calls too large
        let refused = crate::runtime::check_memory(&hw, &i, 0, 16384).is_err();
        assert_eq!(refused, assess(&hw, &live, &after).fit == Fit::TooLarge);
    }
}

#[test]
fn too_little_ram_for_the_model_is_too_large() {
    let hw = laptop(8.0, 4.0, 2.0);
    let s = Subject {
        min_ram_gb: 16.0,
        ..subject(&qwen3_8b(), 1.0, 2048)
    };
    assert_eq!(assess(&hw, &Live::default(), &s).fit, Fit::TooLarge);
}

#[test]
fn cpu_only_and_moe_use_system_memory() {
    let cpu = Hardware {
        gpus: vec![],
        ..laptop(0.0, 32.0, 24.0)
    };
    let small = assess(&cpu, &Live::default(), &subject(&qwen3_8b(), 2.0, 4096));
    assert_eq!(small.fit, Fit::Comfortable);
    assert_eq!(small.speed, Speed::Slow);
    let moe = Subject {
        moe: true,
        ..subject(&qwen3_8b(), 15.0, 4096)
    };
    let on_cpu = assess(&cpu, &Live::default(), &moe);
    assert_ne!(on_cpu.fit, Fit::TooLarge, "{on_cpu:?}");
    let on_gpu = assess(&laptop(8.0, 32.0, 24.0), &Live::default(), &moe);
    assert_eq!((on_gpu.speed, on_gpu.fit == Fit::TooLarge), (Speed::Moderate, false));
    let big_moe = Subject {
        moe: true,
        ..subject(&qwen3_8b(), 60.0, 4096)
    };
    assert_eq!(assess(&cpu, &Live::default(), &big_moe).fit, Fit::TooLarge);
}

#[test]
fn compute_buffer_grows_with_the_context() {
    assert!(compute_buffer_gb(32768) > compute_buffer_gb(4096));
    assert!(compute_buffer_gb(1024) >= 0.6);
}

#[test]
fn the_load_time_refusal_names_the_numbers_and_what_to_change() {
    let i = qwen3_8b();
    // a 6 GB machine cannot hold an 8B model with a 128k context, but a short one would fit
    let hw = Hardware {
        gpus: vec![],
        ..laptop(0.0, 12.0, 11.0)
    };
    let big = GgufInfo { file_size: 3 * GB, ..i };
    let err = crate::runtime::check_memory(&hw, &big, 0, 131_072).unwrap_err().to_string();
    assert!(err.contains("weights 3.0 + context"), "{err}");
    assert!(err.contains("shorter context"), "{err}");
    assert!(crate::runtime::check_memory(&hw, &big, 0, 4096).is_ok());
}

#[test]
fn the_shape_cache_remembers_and_survives_a_poisoned_lock() {
    let c = ShapeCache::default();
    assert!(!c.contains("a.gguf"));
    c.insert("a.gguf", KvShape::from_params(8.0));
    assert!(c.contains("a.gguf"));
    assert_eq!(c.snapshot().len(), 1);
    std::thread::scope(|s| {
        let _ = s
            .spawn(|| {
                let _g = c.0.lock().unwrap();
                panic!("poison");
            })
            .join();
    });
    assert!(c.contains("a.gguf"), "a poisoned lock is recovered");
}
