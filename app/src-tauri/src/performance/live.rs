//! The running values of Settings ▸ Performance: loaded (or, on the first run, suggested and saved) when Debrief starts,
//! handed to the store worker's supervisor, the plugin processes and the AI launcher, and updated when the section
//! changes. Each process reads them at its next start (ADR 0031 §7).

use super::suggest::{probe, suggest};
use super::tuning;
use super::watch::MemoryHub;
use super::PerformanceSettings;
use crate::sync::lock;
use debrief_plugin_host::{Builtins, PluginLimits};
use debrief_store_client::{StoreWorker, WorkerLaunch};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

/// Managed through [`crate::app_state::AppState`].
pub struct Live {
    dir: Option<PathBuf>,
    cores: u32,
    settings: RwLock<PerformanceSettings>,
    pub hub: Arc<MemoryHub>,
    worker: Mutex<Option<StoreWorker>>,
    plugins: Mutex<Option<Arc<Builtins>>>,
}

impl Live {
    /// The settings in the configuration folder `dir`; on the first run (no file yet) the values suggested for this
    /// computer (`data_dir` is where projects are kept), saved so they show in Settings.
    pub fn start(dir: Option<PathBuf>, data_dir: Option<&Path>) -> Arc<Live> {
        let settings = match (&dir, super::first_run(dir.as_deref())) {
            (Some(d), true) => {
                let env = probe(data_dir.unwrap_or(d), 0);
                let s = suggest(&env, &PerformanceSettings::default());
                log::info!(TotalMb = env.total_memory_mb, Cores = env.cores, StoreCapMb = s.store_cap_mb; "First run: performance settings suggested for this computer");
                if let Err(e) = super::save(d, &s) {
                    log::warn!("The suggested performance settings could not be saved: {e:#}");
                }
                s
            }
            _ => super::load(dir.as_deref()),
        };
        Live::with(dir, settings, super::section::cores())
    }

    /// Fixed settings and cores (tests).
    pub fn with(dir: Option<PathBuf>, settings: PerformanceSettings, cores: u32) -> Arc<Live> {
        Arc::new(Live {
            dir,
            cores,
            settings: RwLock::new(settings),
            hub: MemoryHub::new(),
            worker: Mutex::new(None),
            plugins: Mutex::new(None),
        })
    }

    pub fn settings(&self) -> PerformanceSettings {
        self.settings.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// What the store worker starts with.
    pub fn worker_launch(&self) -> WorkerLaunch {
        tuning::worker_launch(&self.settings(), self.cores)
    }

    /// The limits of Debrief's plugin processes, with the hub's listener.
    pub fn plugin_limits(&self) -> PluginLimits {
        tuning::plugin_limits(&self.settings(), Some(self.hub.listener()))
    }

    /// The AI worker's cap now, bytes.
    pub fn ai_cap(&self) -> Option<u64> {
        tuning::ai_cap(&self.settings())
    }

    /// The store worker to update after a change.
    pub fn attach_worker(&self, worker: StoreWorker) {
        *lock(&self.worker) = Some(worker);
    }

    /// Debrief's plugin processes: they get the limits now, and the hub their names.
    pub fn attach_plugins(&self, plugins: Arc<Builtins>) {
        plugins.set_limits(&self.plugin_limits());
        let names: HashMap<String, String> = plugins
            .processes()
            .iter()
            .map(|p| (p.id().to_string(), p.manifest().name.to_string()))
            .collect();
        self.hub.set_names(names);
        *lock(&self.plugins) = Some(plugins);
    }

    pub fn plugins(&self) -> Option<Arc<Builtins>> {
        lock(&self.plugins).clone()
    }

    pub fn worker(&self) -> Option<StoreWorker> {
        lock(&self.worker).clone()
    }

    /// New settings (already saved): every process gets them at its next start.
    pub fn apply(&self, s: &PerformanceSettings) {
        *self.settings.write().unwrap_or_else(|p| p.into_inner()) = s.clone();
        if let Some(w) = lock(&self.worker).as_ref() {
            w.set_launch(self.worker_launch());
        }
        if let Some(p) = lock(&self.plugins).as_ref() {
            p.set_limits(&self.plugin_limits());
        }
    }

    /// Sets (or with `None` removes) the memory cap of plugin `id`, saved and applied from its next start. Refused for
    /// a plugin Debrief does not have or a size outside the bounds every plugin gets.
    pub fn set_plugin_cap(&self, id: &str, mb: Option<u32>) -> Result<(), String> {
        let known = lock(&self.plugins).as_ref().is_some_and(|p| p.process(id).is_some());
        if !known {
            return Err(format!("there is no plugin \"{}\"", id.chars().take(80).collect::<String>()));
        }
        let (min, max) = (super::section::MIN_PLUGIN_MB, super::section::MAX_PLUGIN_MB);
        let mut s = self.settings();
        match mb {
            Some(m) if !(min..=max).contains(&m) => return Err(format!("a plugin's memory cap is {min} to {max} MB")),
            Some(m) => {
                if !s.plugin_memory.contains_key(id) && s.plugin_memory.len() >= super::MAX_PLUGIN_CAPS {
                    return Err("too many plugin caps; remove one first".into());
                }
                s.plugin_memory.insert(id.to_string(), m);
            }
            None => {
                s.plugin_memory.remove(id);
            }
        }
        if let Some(d) = &self.dir {
            super::save(d, &s).map_err(|e| format!("the cap could not be saved: {e:#}"))?;
        }
        self.apply(&s);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::performance::tests::temp;
    use debrief_plugin_api::manifest::Manifest;

    fn builtins(ids: &[&str]) -> Arc<Builtins> {
        let manifests: Vec<Manifest> = ids
            .iter()
            .map(|id| {
                serde_json::from_value(serde_json::json!({
                    "id": id, "name": format!("Name of {id}"), "version": "1.0.0", "protocol": debrief_plugin_api::PROTOCOL,
                    "permissions": [], "importers": [], "configParsers": [], "markerKinds": [], "annotators": [],
                    "transforms": [], "tools": [], "profileImporters": [], "documentImporters": []
                }))
                .unwrap()
            })
            .collect();
        Arc::new(Builtins::from_manifests(Path::new("/no/program"), manifests))
    }

    #[test]
    fn the_first_run_saves_suggested_values_and_later_runs_load_them() {
        let d = temp("live-first");
        let first = Live::start(Some(d.clone()), None);
        assert!(d.join(super::super::FILE).exists(), "saved on the first run");
        let again = Live::start(Some(d.clone()), None);
        assert_eq!(again.settings(), first.settings());
        assert_eq!(
            Live::start(None, None).settings(),
            PerformanceSettings::default(),
            "no folder: the defaults"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_change_reaches_the_plugins_and_names_reach_the_hub() {
        let live = Live::with(None, PerformanceSettings::default(), 8);
        let plugins = builtins(&["p.one", "p.two"]);
        live.attach_plugins(plugins.clone());
        assert_eq!(plugins.process("p.one").unwrap().memory_cap(), 1 << 30);
        live.apply(&PerformanceSettings {
            plugin_memory_mb: 768,
            plugin_cpu_percent: 30,
            ..PerformanceSettings::default()
        });
        let p = plugins.process("p.two").unwrap();
        assert_eq!((p.memory_cap(), p.sandbox().cpu_rate_percent), (768 << 20, Some(30)));
        assert_eq!(live.ai_cap(), Some(1024 << 20));
        assert_eq!(live.worker_launch().memory_cap, Some(2 << 30));
        live.hub.hear(
            "plugin \"p.one\"",
            debrief_ipc::memory_watch::MemoryEvent::Ended(debrief_ipc::memory_watch::MemoryUse {
                used: 1,
                peak: 1,
                limit: 2,
            }),
        );
        assert_eq!(live.hub.seen().len(), 1);
    }

    #[test]
    fn plugin_caps_are_validated_saved_and_applied() {
        let d = temp("live-caps");
        let live = Live::with(Some(d.clone()), PerformanceSettings::default(), 8);
        assert!(live.set_plugin_cap("p.one", Some(512)).is_err(), "no plugins attached yet");
        let plugins = builtins(&["p.one"]);
        live.attach_plugins(plugins.clone());
        assert!(live.set_plugin_cap("p.unknown", Some(512)).unwrap_err().contains("no plugin"));
        assert!(live.set_plugin_cap("p.one", Some(10)).unwrap_err().contains("64 to 4096"));
        live.set_plugin_cap("p.one", Some(512)).unwrap();
        assert_eq!(plugins.process("p.one").unwrap().memory_cap(), 512 << 20);
        assert_eq!(super::super::load(Some(&d)).plugin_memory["p.one"], 512);
        live.set_plugin_cap("p.one", None).unwrap();
        assert_eq!(plugins.process("p.one").unwrap().memory_cap(), 1 << 30);
        assert!(super::super::load(Some(&d)).plugin_memory.is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }
}
