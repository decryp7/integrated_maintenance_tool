//! The store worker (ADR 0014 §1): one process per app instance hosts the DuckDB stores of all windows and
//! their full-text indexes. [`server`] is the worker side and [`plugins`] the importer plugins it starts; Debrief's
//! side (the protocol, the supervisor and `RemoteStore`) is `debrief-store-client`, which links no database.

pub mod plugins;
pub mod server;

pub use debrief_store_client::{protocol, MemoryMode, RemoteStore, StoreTuning, StoreWorker, WorkerConfig};

/// The worker program: serves Debrief with the built-in plugins next to it and the installed third-party plugins Debrief
/// names (ADR 0027), each started on first use, in the memory mode its command line names (low memory mode gives
/// freed memory back at once) with the tuning it names (ADR 0031). Exits with status 1 when it cannot connect.
pub fn main() {
    if std::env::args().any(|a| a == "--self-test") {
        match self_test() {
            Ok(()) => println!("debrief-store-worker: self-test ok"),
            Err(e) => {
                eprintln!("debrief-store-worker: self-test failed: {e}");
                std::process::exit(1);
            }
        }
        return;
    }
    let mode = MemoryMode::from_args(std::env::args());
    let tuning = StoreTuning::from_args(std::env::args());
    if mode == MemoryMode::Low {
        debrief_alloc::purge_at_once();
    }
    let plugins = plugins::WorkerPlugins::new(debrief_plugin_host::builtins::default_binary()).with_limits(plugin_limits(&tuning));
    if let Err(e) = server::run(std::sync::Arc::new(plugins), mode, tuning) {
        eprintln!("debrief-store-worker: {e}");
        std::process::exit(1);
    }
}

/// Creates, reopens and removes a tiny store in a temporary folder: proves that the database library loads (the packaged
/// app's library validation, run paths) and works, without Debrief. Run by `--self-test` (the CI smoke check of a bundle).
pub fn self_test() -> Result<(), String> {
    use debrief_core::{PluginRegistry, Profile};
    use debrief_store_duck::{DuckStore, StoreOptions};
    let dir = std::env::temp_dir().join(format!("debrief-store-self-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let create = DuckStore::create_with(&dir, Profile::default_profile(), PluginRegistry::new(), StoreOptions::default());
        drop(create.map_err(|e| e.to_string())?);
        DuckStore::open_with(&dir, Profile::default_profile(), PluginRegistry::new(), StoreOptions::default())
            .map(drop)
            .map_err(|e| e.to_string())
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// The limits of the worker's plugin processes that `tuning` names (the watcher's events stay in the worker's log).
pub fn plugin_limits(tuning: &StoreTuning) -> debrief_plugin_host::PluginLimits {
    const MB: u64 = 1 << 20;
    debrief_plugin_host::PluginLimits {
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
    fn the_self_test_opens_and_reopens_a_store() {
        self_test().unwrap();
    }

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
