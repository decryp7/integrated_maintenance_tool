//! The store worker (ADR 0014 §1): one process per app instance hosts the DuckDB stores of all windows and
//! their full-text indexes. [`server`] is the worker side and [`plugins`] the importer plugins it starts; IMT's
//! side (the protocol, the supervisor and `RemoteStore`) is `imt-store-client`, which links no database.

pub mod plugins;
pub mod server;

pub use imt_store_client::{protocol, MemoryMode, RemoteStore, StoreTuning, StoreWorker, WorkerConfig};

/// The worker program: serves IMT with the built-in plugins next to it and the installed third-party plugins IMT
/// names (ADR 0027), each started on first use, in the memory mode its command line names (low memory mode gives
/// freed memory back at once) with the tuning it names (ADR 0031). Exits with status 1 when it cannot connect.
pub fn main() {
    // First of all, while the process has one thread and before any database opens (jemalloc reads its options once).
    let tuning = StoreTuning::from_args(std::env::args());
    imt_store_duck::allocator::configure(tuning.allocator_arenas.unwrap_or(imt_store_duck::allocator::ARENAS));
    let mode = MemoryMode::from_args(std::env::args());
    if mode == MemoryMode::Low {
        imt_alloc::purge_at_once();
    }
    let plugins = plugins::WorkerPlugins::new(imt_plugin_host::builtins::default_binary()).with_limits(plugin_limits(&tuning));
    if let Err(e) = server::run(std::sync::Arc::new(plugins), mode, tuning) {
        eprintln!("imt-store-worker: {e}");
        std::process::exit(1);
    }
}

/// The limits of the worker's plugin processes that `tuning` names (the watcher's events stay in the worker's log).
pub fn plugin_limits(tuning: &StoreTuning) -> imt_plugin_host::PluginLimits {
    const MB: u64 = 1 << 20;
    imt_plugin_host::PluginLimits {
        default_memory: tuning.plugin_memory_mb.map(|m| m * MB),
        memory: tuning
            .plugin_memory_overrides_mb
            .iter()
            .map(|(id, m)| (id.clone(), m * MB))
            .collect(),
        cpu_rate_percent: tuning.plugin_cpu_percent,
        memory_listener: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tuning_names_the_limits_of_the_workers_plugins() {
        let mut t = StoreTuning {
            plugin_memory_mb: Some(768),
            plugin_cpu_percent: Some(40),
            ..StoreTuning::default()
        };
        t.plugin_memory_overrides_mb.insert("p".into(), 300);
        let l = plugin_limits(&t);
        assert_eq!(
            (l.default_memory, l.memory["p"], l.cpu_rate_percent),
            (Some(768 << 20), 300 << 20, Some(40))
        );
        let none = plugin_limits(&StoreTuning::default());
        assert!(none.default_memory.is_none() && none.memory.is_empty() && none.cpu_rate_percent.is_none());
    }
}
