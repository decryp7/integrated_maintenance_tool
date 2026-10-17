//! Which plugins run out of process, and with which permissions.
//!
//! A built-in plugin runs isolated (in the `imt-plugin-host` process) when the user switched "Run
//! isolated" on for it, when it was quarantined while running in process and "isolate after a quarantine"
//! is on (the default; effective from the next start or new window), or when isolated mode is on (the
//! setting or the `--isolated` command-line flag). Plugins that cannot run out of process (see
//! [`crate::builtins::isolable`]) stay in process.
//!
//! Permissions a plugin asks for must be granted; the grants are stored with these settings. Built-in
//! plugins are first-party, so their permissions are granted until the user revokes them.

use crate::builtins::{self, Isolable};
use crate::remote::RemotePlugin;
use anyhow::{anyhow, Result};
use imt_core::{Plugin, PluginRegistry};
use imt_plugin_sdk::{Permission, PermissionRequest};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// The command-line flag for isolated mode.
pub const ISOLATED_FLAG: &str = "--isolated";

/// The stored isolation settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct IsolationSettings {
    /// Every plugin that can runs out of process.
    pub isolated_mode: bool,
    /// A plugin quarantined in process runs isolated from then on.
    pub auto_isolate: bool,
    /// Plugins set to "Run isolated" (by the user or after a quarantine).
    pub isolated: Vec<String>,
    /// Permissions the user revoked, by plugin id (permission keys, see [`Permission::key`]). Requested
    /// permissions of built-in plugins are granted unless revoked here.
    pub revoked: BTreeMap<String, Vec<String>>,
    /// Permissions the user granted to plugins that are not built in, by plugin id.
    pub granted: BTreeMap<String, Vec<String>>,
}

impl Default for IsolationSettings {
    fn default() -> Self {
        IsolationSettings {
            isolated_mode: false,
            auto_isolate: true,
            isolated: Vec::new(),
            revoked: BTreeMap::new(),
            granted: BTreeMap::new(),
        }
    }
}

impl IsolationSettings {
    /// Whether `id` is set to run isolated (by isolated mode, `cli_isolated` or its own switch).
    pub fn wants_isolated(&self, id: &str, cli_isolated: bool) -> bool {
        cli_isolated || self.isolated_mode || self.isolated.iter().any(|i| i == id)
    }

    /// Records a quarantine of `id` running in process; returns whether the settings changed.
    pub fn on_quarantine(&mut self, id: &str) -> bool {
        if !self.auto_isolate || self.isolated.iter().any(|i| i == id) {
            return false;
        }
        self.isolated.push(id.to_string());
        true
    }

    pub fn set_isolated(&mut self, id: &str, on: bool) {
        self.isolated.retain(|i| i != id);
        if on {
            self.isolated.push(id.to_string());
        }
    }

    /// The granted subset of what a built-in requests.
    pub fn builtin_grants(&self, id: &str, requested: &[PermissionRequest]) -> Vec<PermissionRequest> {
        let revoked = self.revoked.get(id);
        requested
            .iter()
            .filter(|p| !revoked.is_some_and(|r| r.contains(&p.permission.key())))
            .cloned()
            .collect()
    }

    /// The granted subset of what a plugin that is not built in requests.
    pub fn external_grants(&self, id: &str, requested: &[PermissionRequest]) -> Vec<PermissionRequest> {
        let granted = self.granted.get(id);
        requested
            .iter()
            .filter(|p| granted.is_some_and(|g| g.contains(&p.permission.key())))
            .cloned()
            .collect()
    }

    /// Grants or revokes a built-in's permission.
    pub fn set_builtin_grant(&mut self, id: &str, permission: &Permission, granted: bool) {
        let list = self.revoked.entry(id.to_string()).or_default();
        list.retain(|k| *k != permission.key());
        if !granted {
            list.push(permission.key());
        }
        if list.is_empty() {
            self.revoked.remove(id);
        }
    }

    /// Reads settings from JSON; anything unreadable means the defaults.
    pub fn from_json(v: &serde_json::Value) -> IsolationSettings {
        serde_json::from_value(v.clone()).unwrap_or_default()
    }
}

/// Stands in for a plugin that should run isolated but could not be started: it is listed, inactive,
/// with the reason (it never runs in process instead, which would defeat the isolation).
pub struct Unavailable {
    id: &'static str,
    name: &'static str,
    version: &'static str,
    reason: String,
}

impl Plugin for Unavailable {
    fn id(&self) -> &'static str {
        self.id
    }
    fn name(&self) -> &'static str {
        self.name
    }
    fn version(&self) -> &'static str {
        self.version
    }
    fn register(&self, _registry: &mut PluginRegistry) -> Result<()> {
        Err(anyhow!("{}", self.reason))
    }
}

/// How a plugin runs in this session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    InProcess,
    Isolated,
    /// Set to run isolated but could not start (see the plugin's inactive reason).
    Unavailable,
}

/// A plugin set to run isolated, as started: its process (or a placeholder) and how it runs.
type Started = (Arc<dyn Plugin>, Mode);

/// The session's isolation: the settings read at start, and the plugin processes started for them
/// (shared by every window, so each isolated plugin has one process).
pub struct Isolation {
    settings: IsolationSettings,
    cli_isolated: bool,
    host: PathBuf,
    started: Mutex<HashMap<String, Started>>,
    remotes: Mutex<HashMap<String, Arc<RemotePlugin>>>,
    modes: Mutex<BTreeMap<String, Mode>>,
}

impl Isolation {
    pub fn new(settings: IsolationSettings, cli_isolated: bool, host: PathBuf) -> Isolation {
        Isolation {
            settings,
            cli_isolated,
            host,
            started: Mutex::new(HashMap::new()),
            remotes: Mutex::new(HashMap::new()),
            modes: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn settings(&self) -> &IsolationSettings {
        &self.settings
    }

    /// Whether isolated mode is on for this session (setting or flag).
    pub fn isolated_mode(&self) -> bool {
        self.cli_isolated || self.settings.isolated_mode
    }

    /// How each plugin resolved so far runs.
    pub fn modes(&self) -> BTreeMap<String, Mode> {
        self.modes.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// The plugins to install: `plugins` with those that should run isolated replaced by their plugin
    /// process (started once, in parallel, on first use). A plugin whose process cannot start is replaced
    /// by an [`Unavailable`] placeholder that says why.
    pub fn resolve(&self, plugins: Vec<Arc<dyn Plugin>>) -> Vec<Arc<dyn Plugin>> {
        let wanted: Vec<bool> = plugins
            .iter()
            .map(|p| self.settings.wants_isolated(p.id(), self.cli_isolated) && builtins::isolable(p.as_ref()) != Isolable::No)
            .collect();
        let to_start: Vec<Arc<dyn Plugin>> = {
            let started = self.started.lock().unwrap_or_else(|p| p.into_inner());
            plugins
                .iter()
                .zip(&wanted)
                .filter(|(p, w)| **w && !started.contains_key(p.id()))
                .map(|(p, _)| p.clone())
                .collect()
        };
        let fresh: Vec<(String, Started)> = std::thread::scope(|s| {
            let handles: Vec<_> = to_start
                .iter()
                .map(|p| {
                    let p = p.clone();
                    s.spawn(move || (p.id().to_string(), self.start(p.as_ref())))
                })
                .collect();
            handles.into_iter().filter_map(|h| h.join().ok()).collect()
        });
        let mut started = self.started.lock().unwrap_or_else(|p| p.into_inner());
        started.extend(fresh);
        let mut modes = self.modes.lock().unwrap_or_else(|p| p.into_inner());
        plugins
            .into_iter()
            .zip(wanted)
            .map(|(p, w)| {
                if !w {
                    modes.insert(p.id().to_string(), Mode::InProcess);
                    return p;
                }
                match started.get(p.id()) {
                    Some((remote, mode)) => {
                        modes.insert(p.id().to_string(), *mode);
                        remote.clone()
                    }
                    None => p,
                }
            })
            .collect()
    }

    /// The process of isolated plugin `id`, if it runs isolated.
    pub fn remote(&self, id: &str) -> Option<Arc<RemotePlugin>> {
        self.remotes.lock().unwrap_or_else(|e| e.into_inner()).get(id).cloned()
    }

    /// Lifts the process quarantine of `id` (with `PluginRegistry::reset_plugin`, which lifts the plugin's).
    pub fn reset(&self, id: &str) {
        if let Some(p) = self.remote(id) {
            p.reset();
        }
    }

    /// Stops every plugin process (on exit; they would also exit when IMT does).
    pub fn shutdown(&self) {
        for p in self.remotes.lock().unwrap_or_else(|e| e.into_inner()).values() {
            p.shutdown();
        }
    }

    fn start(&self, builtin: &dyn Plugin) -> Started {
        let id = builtin.id();
        let requested = builtins::serve_options(id).permissions;
        let grants = self.settings.builtin_grants(id, &requested);
        match RemotePlugin::start(builtins::spec(&self.host, id, grants)) {
            Ok(p) => {
                self.remotes
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(id.to_string(), p.clone());
                (p, Mode::Isolated)
            }
            Err(e) => {
                log::error!("Plugin {id} could not start isolated: {e:#}");
                let placeholder = Unavailable {
                    id: builtin.id(),
                    name: builtin.name(),
                    version: builtin.version(),
                    reason: format!("could not start it isolated ({e:#}); turn \"Run isolated\" off to run it in process"),
                };
                (Arc::new(placeholder), Mode::Unavailable)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_decide_isolation_and_grants() {
        let mut s = IsolationSettings::default();
        assert!(s.auto_isolate && !s.isolated_mode);
        assert!(!s.wants_isolated("a", false));
        assert!(s.wants_isolated("a", true));
        assert!(s.on_quarantine("a"));
        assert!(!s.on_quarantine("a"), "once");
        assert!(s.wants_isolated("a", false));
        s.set_isolated("a", false);
        assert!(!s.wants_isolated("a", false));
        s.auto_isolate = false;
        assert!(!s.on_quarantine("a"));
        s.isolated_mode = true;
        assert!(s.wants_isolated("b", false));

        let files = PermissionRequest {
            permission: Permission::ReadOpenedFiles,
            reason: String::new(),
        };
        let net = PermissionRequest {
            permission: Permission::Network { host: "x.org".into() },
            reason: String::new(),
        };
        let both = vec![files.clone(), net.clone()];
        assert_eq!(s.builtin_grants("p", &both).len(), 2);
        s.set_builtin_grant("p", &Permission::ReadOpenedFiles, false);
        assert_eq!(s.builtin_grants("p", &both), vec![net.clone()]);
        s.set_builtin_grant("p", &Permission::ReadOpenedFiles, true);
        assert!(s.revoked.is_empty());
        assert!(s.external_grants("ext", &both).is_empty(), "external plugins get nothing ungranted");
        s.granted.insert("ext".into(), vec!["network:x.org".into()]);
        assert_eq!(s.external_grants("ext", &both), vec![net]);

        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(IsolationSettings::from_json(&json), s);
        assert_eq!(
            IsolationSettings::from_json(&serde_json::json!("garbage")),
            IsolationSettings::default()
        );
        assert_eq!(
            IsolationSettings::from_json(&serde_json::json!({"isolatedMode": true})).isolated,
            Vec::<String>::new()
        );
    }

    #[test]
    fn an_unstartable_plugin_is_a_placeholder_never_in_process() {
        let iso = Isolation::new(
            IsolationSettings {
                isolated_mode: true,
                ..IsolationSettings::default()
            },
            false,
            PathBuf::from("/definitely/not/a/host"),
        );
        assert!(iso.isolated_mode());
        let resolved = iso.resolve(builtins::all());
        let mut registry = PluginRegistry::empty();
        for p in resolved {
            let _ = registry.install_shared(p);
        }
        registry.resolve();
        let web = registry.plugins().into_iter().find(|p| p.id == "net.decryptology.web").unwrap();
        assert_eq!(web.status, imt_core::plugin::PluginStatus::Inactive);
        assert!(web.inactive_reason.unwrap().contains("could not start it isolated"));
        let modes = iso.modes();
        assert_eq!(modes["net.decryptology.web"], Mode::Unavailable);
        // Profiling cannot run out of process: it stays in process.
        assert_eq!(modes["net.decryptology.profiling"], Mode::InProcess);
        assert!(!iso.settings().isolated.contains(&"x".to_string()));
    }
}
