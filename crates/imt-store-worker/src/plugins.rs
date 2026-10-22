//! The plugins of the worker's imports: IMT's built-in plugins (the `imt-plugins` binary next to the worker) and
//! the installed third-party plugins the app names (ADR 0027). The worker starts both itself, each in its own
//! process with the sandbox the app would give it (third-party ones contained by the launcher, their program
//! checked by hash before every start), and installs them into each store's registry through
//! [`PluginRegistry`], whose guarded adapters contain their failures. One process per plugin serves every store.

use imt_core::PluginRegistry;
use imt_plugin_host::installed::PluginsDir;
use imt_plugin_host::{Builtins, PluginLimits, PluginProcess, RemotePlugin};
use imt_store_client::protocol::ThirdPartyPlugin;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Which plugins a store's registry gets (from the open request).
#[derive(Debug, Clone, Copy, Default)]
pub struct PluginChoice<'a> {
    /// Plugins the user turned off.
    pub disabled: &'a [String],
    /// Safe mode: no plugin at all.
    pub safe_mode: bool,
    /// Installed third-party plugins.
    pub third_party: &'a [ThirdPartyPlugin],
}

/// Where a store's plugins come from (port: the real plugin processes, or none in tests).
pub trait PluginSource: Send + Sync {
    /// The registry of a store opened with `choice`.
    fn registry(&self, choice: &PluginChoice<'_>) -> PluginRegistry;
    /// The app's third-party plugins now: changed grants and removed plugins take effect.
    fn set_third_party(&self, _plugins: &[ThirdPartyPlugin]) {}
}

/// No plugins: only the built-in importers of the engine read files.
pub struct NoPlugins;

impl PluginSource for NoPlugins {
    fn registry(&self, _choice: &PluginChoice<'_>) -> PluginRegistry {
        PluginRegistry::new()
    }
}

/// A registry with `plugins` installed except the `disabled` ones (the same rules as IMT's own registry: a
/// plugin that cannot be installed is logged and left out).
pub fn install_plugins(plugins: Vec<Arc<dyn imt_core::Plugin>>, disabled: &[String]) -> PluginRegistry {
    let mut registry = PluginRegistry::new();
    registry.disable(disabled.iter().cloned());
    for plugin in plugins {
        let id = plugin.id().to_string();
        if let Err(e) = registry.install_shared(plugin) {
            log::error!("plugin {id} not installed in the store worker: {e:#}");
        }
    }
    registry.resolve();
    registry
}

/// A third-party plugin's process and what it was started from.
struct Started {
    spec: ThirdPartyPlugin,
    process: Arc<PluginProcess>,
}

/// The worker's plugin processes: the built-ins (loaded on first use) and the third-party plugins by id.
pub struct WorkerPlugins {
    builtins_exe: PathBuf,
    builtins: OnceLock<Builtins>,
    third_party: Mutex<HashMap<String, Started>>,
    /// Settings ▸ Performance's limits of the plugin processes (ADR 0030).
    limits: PluginLimits,
}

impl WorkerPlugins {
    /// The built-ins served by `builtins_exe` (none when it cannot be used) and no third-party plugin yet.
    pub fn new(builtins_exe: PathBuf) -> WorkerPlugins {
        WorkerPlugins {
            builtins_exe,
            builtins: OnceLock::new(),
            third_party: Mutex::default(),
            limits: PluginLimits::default(),
        }
    }

    /// The plugin processes with these limits (memory caps, CPU share).
    pub fn with_limits(mut self, limits: PluginLimits) -> WorkerPlugins {
        self.limits = limits;
        self
    }

    fn builtins(&self) -> &Builtins {
        self.builtins.get_or_init(|| {
            let b = Builtins::load(&self.builtins_exe);
            b.set_limits(&self.limits);
            b
        })
    }

    /// The process of `spec`: the one already started for the same program, else a new one from its install
    /// record. Refused when the record is gone, turned off, names another program, or `spec` grants what the
    /// record does not ask for.
    fn third_party_process(&self, spec: &ThirdPartyPlugin) -> Result<Arc<PluginProcess>, String> {
        let mut started = lock(&self.third_party);
        if let Some(s) = started.get_mut(&spec.id) {
            if s.spec.sha256 == spec.sha256 && s.spec.plugins_dir == spec.plugins_dir {
                if s.spec.granted != spec.granted {
                    s.process.set_granted(spec.granted.clone());
                    s.spec.granted = spec.granted.clone();
                }
                return Ok(s.process.clone());
            }
        }
        let process = start_third_party(spec)?;
        process.set_limits(self.limits.clone());
        let old = started.insert(
            spec.id.clone(),
            Started {
                spec: spec.clone(),
                process: process.clone(),
            },
        );
        if let Some(old) = old {
            old.process.shutdown();
        }
        Ok(process)
    }
}

/// A new process for an installed plugin, checked against its record.
fn start_third_party(spec: &ThirdPartyPlugin) -> Result<Arc<PluginProcess>, String> {
    let dir = PluginsDir::new(&spec.plugins_dir);
    let record = dir
        .record(&spec.id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "it is no longer installed".to_string())?;
    if !record.enabled {
        return Err("it is turned off".into());
    }
    if !record.sha256.eq_ignore_ascii_case(&spec.sha256) {
        return Err("it was updated or reinstalled after IMT started; restart IMT to use the new version".into());
    }
    if let Some(p) = spec.granted.iter().find(|p| !record.requested.iter().any(|r| r.permission == **p)) {
        return Err(format!("it is granted {p:?}, which it does not ask for"));
    }
    let process = dir.process(&record).map_err(|e| e.to_string())?;
    process.set_granted(spec.granted.clone());
    Ok(process)
}

impl PluginSource for WorkerPlugins {
    fn registry(&self, choice: &PluginChoice<'_>) -> PluginRegistry {
        if choice.safe_mode {
            return PluginRegistry::new();
        }
        let mut plugins = self.builtins().plugins();
        for spec in choice.third_party {
            if self.builtins().process(&spec.id).is_some() {
                log::warn!("plugin {} is already part of IMT; the installed one is not used", spec.id);
                continue;
            }
            match self.third_party_process(spec) {
                Ok(p) => plugins.push(Arc::new(RemotePlugin::new(p))),
                Err(e) => log::warn!("installed plugin {} does not take part in imports: {e}", spec.id),
            }
        }
        install_plugins(plugins, choice.disabled)
    }

    fn set_third_party(&self, plugins: &[ThirdPartyPlugin]) {
        let mut started = lock(&self.third_party);
        started.retain(|id, s| {
            let Some(now) = plugins.iter().find(|p| p.id == *id && p.sha256 == s.spec.sha256) else {
                s.process.shutdown();
                return false;
            };
            if now.granted != s.spec.granted {
                s.process.set_granted(now.granted.clone());
                s.spec.granted = now.granted.clone();
            }
            true
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_plugin_api::manifest::Permission;

    fn spec(dir: &std::path::Path) -> ThirdPartyPlugin {
        ThirdPartyPlugin {
            plugins_dir: dir.to_path_buf(),
            id: "com.example.kv".into(),
            sha256: "0".repeat(64),
            granted: Vec::new(),
        }
    }

    fn record(dir: &std::path::Path, enabled: bool) {
        let d = dir.join("com.example.kv");
        std::fs::create_dir_all(&d).unwrap();
        let r = serde_json::json!({
            "format": imt_plugin_host::installed::RECORD_FORMAT, "id": "com.example.kv", "version": "1.0.0", "name": "KV",
            "publisher": "P", "description": "", "signature": "unsigned", "signer": null, "warning": null,
            "digest": "0".repeat(64), "folder": "v-1.0.0-00000000", "program": "kv", "sha256": "0".repeat(64),
            "requested": [], "granted": [], "enabled": enabled, "installedAtMs": 0
        });
        std::fs::write(d.join(imt_plugin_host::installed::RECORD_FILE), r.to_string()).unwrap();
    }

    fn refused(s: &ThirdPartyPlugin) -> String {
        match start_third_party(s) {
            Ok(_) => panic!("{} started", s.id),
            Err(e) => e,
        }
    }

    #[test]
    fn a_third_party_plugin_is_used_only_as_the_app_loaded_it() {
        let t = tempfile::tempdir().unwrap();
        let s = spec(t.path());
        assert!(refused(&s).contains("no longer installed"));
        record(t.path(), false);
        assert!(refused(&s).contains("turned off"));
        record(t.path(), true);
        let other = ThirdPartyPlugin {
            sha256: "1".repeat(64),
            ..s.clone()
        };
        assert!(refused(&other).contains("restart IMT"));
        let greedy = ThirdPartyPlugin {
            granted: vec![Permission::Network],
            ..s.clone()
        };
        assert!(refused(&greedy).contains("does not ask for"));
        // The record is fine but its program's contract is missing: unavailable with the reason, no panic.
        assert!(!refused(&s).is_empty());
    }

    #[test]
    fn a_plugin_that_cannot_start_is_left_out_of_the_registry() {
        let t = tempfile::tempdir().unwrap();
        let w = WorkerPlugins::new(t.path().join("no-imt-plugins"));
        let s = [spec(t.path())];
        let r = w.registry(&PluginChoice {
            third_party: &s,
            ..PluginChoice::default()
        });
        assert!(r.plugins().is_empty());
        assert!(w
            .registry(&PluginChoice {
                safe_mode: true,
                third_party: &s,
                ..PluginChoice::default()
            })
            .plugins()
            .is_empty());
        w.set_third_party(&[]);
        assert!(lock(&w.third_party).is_empty());
    }
}
