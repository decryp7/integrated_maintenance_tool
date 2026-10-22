//! The processes' knobs derived from the settings, by pure functions (ADR 0031 §2): budgets in, knobs out. With the
//! defaults they are today's tested values (8 cores: 7 parse threads, DuckDB 4 threads and 192 MB, a 64 MB index
//! writer with 2 threads, a 2 GiB cap), so ADR 0014's memory tests keep guarding them.

use super::section::WORKER_BASE_MB;
use super::PerformanceSettings;
use imt_ipc::memory_watch::MemoryListener;
use imt_plugin_host::PluginLimits;
use imt_store_client::{MemoryMode, StoreTuning, WorkerLaunch};
use std::path::PathBuf;

const MB: u64 = 1 << 20;
/// Import memory one parse thread needs (it holds a file's chunk and its batches in flight): 768 MB → 8 threads.
pub const MB_PER_PARSE_THREAD: u32 = 96;
/// Most parse threads chosen automatically (`imt_core::ingest::MAX_DEFAULT_THREADS`).
pub const MAX_AUTO_PARSE_THREADS: u32 = 8;
/// The smallest DuckDB memory limit derived from the query budget, MB.
const MIN_DUCK_MB: u32 = 64;
/// The index writer's heap is this fraction of the import budget (768 MB → 64 MB).
const INDEX_HEAP_SHARE: u32 = 12;

/// Cores IMT's thread pools may use: all but the ones left free, at least one.
pub fn usable_cores(s: &PerformanceSettings, cores: u32) -> u32 {
    cores.saturating_sub(s.leave_cores_free).max(1)
}

/// Parse threads of an import: the setting, or (0) one per [`MB_PER_PARSE_THREAD`] of the import budget, within the
/// usable cores and [`MAX_AUTO_PARSE_THREADS`].
pub fn parse_threads(s: &PerformanceSettings, cores: u32) -> u32 {
    if s.parse_threads > 0 {
        return s.parse_threads;
    }
    // At least one: the usable cores are at least one.
    (s.import_budget_mb / MB_PER_PARSE_THREAD).clamp(1, usable_cores(s, cores).min(MAX_AUTO_PARSE_THREADS))
}

/// DuckDB's memory limit, MB: the setting, or (0) the query budget minus the worker's base.
pub fn duck_memory_mb(s: &PerformanceSettings) -> u32 {
    if s.duck_memory_limit_mb > 0 {
        return s.duck_memory_limit_mb;
    }
    s.query_budget_mb.saturating_sub(WORKER_BASE_MB).max(MIN_DUCK_MB)
}

/// The store worker's tuning (`--tuning`) for `s` on a computer with `cores`.
pub fn store_tuning(s: &PerformanceSettings, cores: u32) -> StoreTuning {
    let usable = usable_cores(s, cores);
    let spill = s.spill_folder.trim();
    StoreTuning {
        parse_threads: Some(parse_threads(s, cores) as usize),
        duck_threads: Some(s.query_threads.min(usable).max(1) as usize),
        memory_limit_mb: Some(u64::from(duck_memory_mb(s))),
        index_heap_mb: Some(u64::from(s.import_budget_mb / INDEX_HEAP_SHARE)),
        index_threads: Some(s.index_threads.min(usable).max(1) as usize),
        spill_dir: (!spill.is_empty()).then(|| PathBuf::from(spill)),
        max_spill_mb: (s.max_spill_mb > 0).then_some(u64::from(s.max_spill_mb)),
        allocator_flush_mb: Some(u64::from(s.allocator_flush_mb)),
        allocator_background_threads: Some(s.allocator_background_threads),
        plugin_memory_mb: Some(u64::from(s.plugin_memory_mb)),
        plugin_memory_overrides_mb: s.plugin_memory.iter().map(|(id, mb)| (id.clone(), u64::from(*mb))).collect(),
        plugin_cpu_percent: (s.plugin_cpu_percent < 100).then_some(s.plugin_cpu_percent as u8),
    }
}

/// What the store worker starts with: its cap, its allocator mode and its tuning.
pub fn worker_launch(s: &PerformanceSettings, cores: u32) -> WorkerLaunch {
    WorkerLaunch {
        memory_cap: Some(u64::from(s.store_cap_mb) * MB),
        memory_mode: if s.return_memory_at_once {
            MemoryMode::Low
        } else {
            MemoryMode::Normal
        },
        tuning: store_tuning(s, cores),
    }
}

/// The limits of IMT's plugin processes (the app's; the worker's own plugins get them through its tuning).
pub fn plugin_limits(s: &PerformanceSettings, listener: Option<MemoryListener>) -> PluginLimits {
    PluginLimits {
        default_memory: Some(u64::from(s.plugin_memory_mb) * MB),
        memory: s.plugin_memory.iter().map(|(id, mb)| (id.clone(), u64::from(*mb) * MB)).collect(),
        cpu_rate_percent: (s.plugin_cpu_percent < 100).then_some(s.plugin_cpu_percent as u8),
        memory_listener: listener,
    }
}

/// The AI worker's cap, bytes (none for 0).
pub fn ai_cap(s: &PerformanceSettings) -> Option<u64> {
    (s.ai_worker_memory_mb > 0).then(|| u64::from(s.ai_worker_memory_mb) * MB)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_derive_todays_tested_knobs() {
        let d = PerformanceSettings::default();
        let t = store_tuning(&d, 8);
        // imt_store_duck's defaults: default_threads() on 8 cores (7), DUCK_THREADS, MEMORY_LIMIT, the index writer.
        assert_eq!(t.parse_threads, Some(7));
        assert_eq!((t.duck_threads, t.memory_limit_mb), (Some(4), Some(192)));
        assert_eq!((t.index_heap_mb, t.index_threads), (Some(64), Some(2)));
        assert_eq!((t.allocator_flush_mb, t.allocator_background_threads), (Some(32), Some(true)));
        assert_eq!((t.spill_dir, t.max_spill_mb, t.plugin_cpu_percent), (None, None, None));
        let l = worker_launch(&d, 8);
        // imt_store_client's DEFAULT_MEMORY_CAP.
        assert_eq!((l.memory_cap, l.memory_mode), (Some(2 << 30), MemoryMode::Normal));
        // 16 cores: still 8 parse threads (the automatic maximum and the import budget agree).
        assert_eq!(store_tuning(&d, 16).parse_threads, Some(8));
        assert_eq!(ai_cap(&d), Some(1024 * MB));
    }

    #[test]
    fn budgets_and_cores_size_the_knobs() {
        let s = PerformanceSettings {
            import_budget_mb: 384,
            query_budget_mb: 236,
            leave_cores_free: 2,
            query_threads: 8,
            index_threads: 4,
            ..PerformanceSettings::default()
        };
        // 384 MB: 4 parse threads; 4 cores minus 2 free: 2.
        assert_eq!(parse_threads(&s, 16), 4);
        assert_eq!(parse_threads(&s, 4), 2);
        let t = store_tuning(&s, 4);
        assert_eq!((t.duck_threads, t.index_threads, t.memory_limit_mb), (Some(2), Some(2), Some(128)));
        assert_eq!(t.index_heap_mb, Some(32));
        // A single core with one left free still gets a thread.
        assert_eq!(usable_cores(&s, 1), 1);
        // Explicit values win.
        let explicit = PerformanceSettings {
            parse_threads: 12,
            duck_memory_limit_mb: 150,
            ..s.clone()
        };
        assert_eq!((parse_threads(&explicit, 4), duck_memory_mb(&explicit)), (12, 150));
        // A tiny query budget keeps DuckDB at its floor.
        let tiny = PerformanceSettings {
            query_budget_mb: 120,
            ..PerformanceSettings::default()
        };
        assert_eq!(duck_memory_mb(&tiny), MIN_DUCK_MB);
    }

    #[test]
    fn spill_plugins_cpu_and_the_ai_cap_follow_the_settings() {
        let mut s = PerformanceSettings {
            spill_folder: "  /fast/disk ".into(),
            max_spill_mb: 10_240,
            plugin_cpu_percent: 50,
            plugin_memory_mb: 768,
            ai_worker_memory_mb: 0,
            return_memory_at_once: true,
            ..PerformanceSettings::default()
        };
        s.plugin_memory.insert("p".into(), 300);
        let t = store_tuning(&s, 8);
        assert_eq!(t.spill_dir, Some(PathBuf::from("/fast/disk")));
        assert_eq!(
            (t.max_spill_mb, t.plugin_cpu_percent, t.plugin_memory_mb),
            (Some(10_240), Some(50), Some(768))
        );
        assert_eq!(t.plugin_memory_overrides_mb["p"], 300);
        assert_eq!(worker_launch(&s, 8).memory_mode, MemoryMode::Low);
        let l = plugin_limits(&s, None);
        assert_eq!(
            (l.default_memory, l.memory["p"], l.cpu_rate_percent),
            (Some(768 * MB), 300 * MB, Some(50))
        );
        assert_eq!(ai_cap(&s), None);
    }
}
