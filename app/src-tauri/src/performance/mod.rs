//! Settings ▸ Performance (ADR 0031): how much memory and CPU Debrief's processes may use (the store worker, plugins,
//! the AI worker), DuckDB's knobs, the warning before a cap stops a process and suggested values for this computer.
//!
//! - here: what `performance.json` holds ([`PerformanceSettings`], format 2; missing, older, newer or damaged: the
//!   defaults) and its loading, bounded by the section's schema;
//! - [`section`]: the schema (the one validator), the relations between fields, the presets;
//! - [`tuning`]: the pure derivation of the processes' knobs from the settings (budgets in, knobs out);
//! - [`suggest`]: the pure suggestion from the environment, and the environment probe;
//! - [`watch`]: the memory watcher's events turned into notification-centre messages, and the peaks;
//! - [`live`]: the running values handed to the store worker, the plugin processes and the AI launcher;
//! - [`commands`]: Suggest settings, presets, plugin caps, memory use, retry.

pub mod commands;
pub mod live;
pub mod section;
pub mod suggest;
pub mod tuning;
pub mod watch;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

pub(crate) const FILE: &str = "performance.json";
/// The format of `performance.json` (1 held only Low memory mode, which is now a preset; it is not migrated).
const VERSION: u32 = 2;
/// Most per-plugin caps kept.
pub const MAX_PLUGIN_CAPS: usize = 256;
/// Longest plugin id a per-plugin cap is kept for.
const MAX_PLUGIN_ID: usize = 200;

/// What `performance.json` holds. Sizes are MB; 0 means "automatic" where the field says so.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PerformanceSettings {
    /// The store worker's hard memory cap: the watcher stops it above.
    pub store_cap_mb: u32,
    /// What an import may use in the store worker (sizes the parse threads and the index writer).
    pub import_budget_mb: u32,
    /// What queries may use in the store worker (sizes DuckDB's memory limit).
    pub query_budget_mb: u32,
    /// The cap of a plugin that declares no need of its own.
    pub plugin_memory_mb: u32,
    /// The AI worker's own process (not llama-server, which holds the model); 0: no cap.
    pub ai_worker_memory_mb: u32,
    /// The store worker gives freed memory back to the system at once (the old Low memory mode's allocator).
    pub return_memory_at_once: bool,
    /// Parse threads of an import; 0: automatic.
    pub parse_threads: u32,
    /// DuckDB query threads.
    pub query_threads: u32,
    /// Full-text index writer threads.
    pub index_threads: u32,
    /// The plugins' CPU share, percent.
    pub plugin_cpu_percent: u32,
    /// Cores no thread pool of Debrief's uses.
    pub leave_cores_free: u32,
    /// DuckDB's memory limit; 0: from the query budget.
    pub duck_memory_limit_mb: u32,
    /// Where DuckDB spills; empty: each store's own `tmp` folder.
    pub spill_folder: String,
    /// The most DuckDB may spill; 0: DuckDB's default (90% of the free disk space).
    pub max_spill_mb: u32,
    /// Freed memory DuckDB's allocator may keep cached.
    pub allocator_flush_mb: u32,
    /// DuckDB's allocator background threads.
    pub allocator_background_threads: bool,
    /// Caps the user set for single plugins in the Plugins panel, by plugin id (not a field of the section).
    pub plugin_memory: BTreeMap<String, u32>,
}

impl Default for PerformanceSettings {
    /// The tested budgets of ADR 0014 (768 MB importing, 300 MB querying, a 2 GiB cap) and today's knobs.
    fn default() -> Self {
        PerformanceSettings {
            store_cap_mb: 2048,
            import_budget_mb: 768,
            query_budget_mb: 300,
            plugin_memory_mb: 1024,
            ai_worker_memory_mb: 1024,
            return_memory_at_once: false,
            parse_threads: 0,
            query_threads: 4,
            index_threads: 2,
            plugin_cpu_percent: 100,
            leave_cores_free: 1,
            duck_memory_limit_mb: 0,
            spill_folder: String::new(),
            max_spill_mb: 0,
            allocator_flush_mb: 32,
            allocator_background_threads: true,
            plugin_memory: BTreeMap::new(),
        }
    }
}

impl PerformanceSettings {
    /// The section's values (every field but the per-plugin caps).
    pub fn to_values(&self) -> Value {
        let mut v = serde_json::to_value(self).unwrap_or(Value::Null);
        if let Some(o) = v.as_object_mut() {
            o.remove("pluginMemory");
        }
        v
    }

    /// These settings with the section's `values` over them (keys the section does not know are ignored).
    pub fn with_values(&self, values: &Value) -> PerformanceSettings {
        let mut merged = self.to_values();
        if let (Some(m), Some(o)) = (merged.as_object_mut(), values.as_object()) {
            for (k, v) in o {
                if m.contains_key(k) {
                    m.insert(k.clone(), v.clone());
                }
            }
        }
        let mut s: PerformanceSettings = serde_json::from_value(merged).unwrap_or_default();
        s.plugin_memory = self.plugin_memory.clone();
        s
    }

    /// Every field within the schema's limits (an out-of-range value takes its default) and the per-plugin caps
    /// bounded: a hand-edited file cannot ask for what the section would refuse.
    pub fn bounded(self) -> PerformanceSettings {
        let schema = section::schema();
        let (valid, problems) = debrief_core::settings::sanitize(&schema, &self.to_values());
        for p in &problems {
            log::warn!("{FILE}: {} ({}); its default is used", p.message, p.key);
        }
        let mut s = PerformanceSettings::default().with_values(&valid.to_json());
        s.plugin_memory = self
            .plugin_memory
            .into_iter()
            .filter(|(id, _)| !id.is_empty() && id.len() <= MAX_PLUGIN_ID)
            .take(MAX_PLUGIN_CAPS)
            .map(|(id, mb)| (id, mb.clamp(section::MIN_PLUGIN_MB, section::MAX_PLUGIN_MB)))
            .collect();
        s
    }
}

/// Memory warnings and stops go to the notification centre from now on (called once it is managed).
pub fn attach_notifications<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    use tauri::Manager;
    let Some(state) = app.try_state::<crate::app_state::AppState>() else {
        return;
    };
    let handle = app.clone();
    state.performance.hub.attach(Box::new(move |notice| {
        crate::notification_centre::post(&handle, None, notice, None);
    }));
}

/// The settings in `dir` (none, older, newer or damaged: the defaults; a damaged file is kept aside).
pub fn load(dir: Option<&Path>) -> PerformanceSettings {
    crate::config_store::ConfigStore::new(dir.map(Path::to_path_buf))
        .load::<PerformanceSettings>(FILE, VERSION)
        .bounded()
}

/// Whether `dir` has no settings file yet (the first run, which applies the suggested values).
pub fn first_run(dir: Option<&Path>) -> bool {
    dir.is_some_and(|d| !d.join(FILE).exists())
}

pub fn save(dir: &Path, s: &PerformanceSettings) -> anyhow::Result<()> {
    Ok(crate::config_store::ConfigStore::new(Some(dir.to_path_buf())).save(FILE, VERSION, s)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    pub(crate) fn temp(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("debrief-performance-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_defaults_are_the_tested_budgets() {
        let d = PerformanceSettings::default();
        assert_eq!((d.store_cap_mb, d.import_budget_mb, d.query_budget_mb), (2048, 768, 300));
        assert!(!d.return_memory_at_once && d.allocator_background_threads);
        assert_eq!(load(None), d);
        assert!(!first_run(None));
    }

    #[test]
    fn a_missing_damaged_older_or_newer_file_gives_the_defaults() {
        let d = temp("damaged");
        assert!(first_run(Some(&d)));
        assert_eq!(load(Some(&d)), PerformanceSettings::default());
        std::fs::write(d.join(FILE), "{ not json").unwrap();
        assert_eq!(load(Some(&d)), PerformanceSettings::default());
        assert!(d.join("performance.json.corrupt").exists(), "kept aside");
        // Format 1 held only Low memory mode: not migrated (no compatibility before v1).
        std::fs::write(d.join(FILE), r#"{"version":1,"lowMemory":true}"#).unwrap();
        assert_eq!(load(Some(&d)), PerformanceSettings::default());
        std::fs::write(d.join(FILE), r#"{"version":9,"storeCapMb":4096}"#).unwrap();
        assert_eq!(load(Some(&d)), PerformanceSettings::default(), "a newer file is not trusted");
        // A wrong type in one field: the whole document is damaged.
        std::fs::write(d.join(FILE), r#"{"version":2,"storeCapMb":"lots"}"#).unwrap();
        assert_eq!(load(Some(&d)), PerformanceSettings::default());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn hand_edited_values_out_of_range_take_their_defaults_and_plugin_caps_are_bounded() {
        let d = temp("bounded");
        let mut caps = serde_json::Map::new();
        for i in 0..300 {
            caps.insert(format!("p{i:03}"), json!(1));
        }
        caps.insert(String::new(), json!(512));
        let doc = json!({"version": 2, "storeCapMb": 1, "queryThreads": 8, "pluginMemory": caps});
        std::fs::write(d.join(FILE), doc.to_string()).unwrap();
        let s = load(Some(&d));
        assert_eq!(s.store_cap_mb, 2048, "below the schema's minimum: the default");
        assert_eq!(s.query_threads, 8, "valid values are kept");
        assert_eq!(s.plugin_memory.len(), MAX_PLUGIN_CAPS);
        assert!(s.plugin_memory.values().all(|mb| *mb == section::MIN_PLUGIN_MB));
        assert!(!s.plugin_memory.contains_key(""));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn values_round_trip_through_the_section_and_keep_the_plugin_caps() {
        let mut s = PerformanceSettings::default();
        s.plugin_memory.insert("p".into(), 300);
        let v = s.to_values();
        assert!(v.get("pluginMemory").is_none());
        let changed = s.with_values(&json!({"storeCapMb": 4096, "unknown": 1}));
        assert_eq!(changed.store_cap_mb, 4096);
        assert_eq!(changed.plugin_memory["p"], 300);
        let d = temp("save");
        save(&d, &changed).unwrap();
        assert_eq!(load(Some(&d)), changed);
        assert!(!first_run(Some(&d)));
        let _ = std::fs::remove_dir_all(&d);
    }
}
