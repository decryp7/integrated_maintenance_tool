//! The plugin host's live state: installed plugins, their resolved status,
//! activation hooks and status-change events.
//!
//! Status is recomputed (pure [`super::graph::resolve`]) whenever something
//! that affects it changes: an install, a quarantine starting or being
//! lifted, a run-time disable, a failed activation. The result is pushed into
//! [`Health`] so guarded adapters refuse calls of inactive plugins
//! (cascade), activation hooks run for plugins that became active
//! (topological order) and deactivation hooks for those that stopped
//! (reverse order), and a [`PluginStateChanged`] event is published per
//! change.

use super::graph::{self, State};
use super::manifest::{DepInfo, Manifest, ServiceInfo};
use super::services::ServiceTable;
use super::{ActivationCtx, Plugin, PluginInfo, PluginStatus};
use crate::events::{EventBus, PluginStateChanged};
use crate::guard::{CallKind, Guard, Health};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};

/// One installed plugin.
pub(crate) struct Installed {
    pub name: String,
    /// The version text as the plugin reports it.
    pub version_text: String,
    pub manifest: Manifest,
    /// Disabled at install (user choice or safe mode): nothing registered.
    pub disabled: bool,
    pub contributes: Vec<String>,
    /// Why registration failed, if it did.
    pub failure: Option<String>,
    /// Kept when installed with `install_shared`, for activation hooks.
    pub plugin: Option<Arc<dyn Plugin>>,
}

#[derive(Default)]
struct Book {
    installed: Vec<Installed>,
    runtime_disabled: HashSet<String>,
    activation_errors: HashMap<String, String>,
    activated: HashSet<String>,
    resolved: bool,
    last: HashMap<String, PluginStatus>,
}

pub(crate) struct Runtime {
    pub health: Arc<Health>,
    pub events: Arc<EventBus>,
    pub table: RwLock<ServiceTable>,
    state: Mutex<Book>,
    resolving: Mutex<()>,
    dirty: AtomicBool,
}

/// The resolution of every installed plugin, in install order.
pub(crate) struct Snapshot {
    pub ids: Vec<String>,
    pub states: Vec<State>,
    pub order: Vec<usize>,
    pub warnings: Vec<Vec<String>>,
    pub dependents: Vec<Vec<String>>,
}

impl Snapshot {
    pub fn status(&self, i: usize) -> PluginStatus {
        match self.states[i] {
            State::Active => PluginStatus::Active,
            State::Disabled => PluginStatus::Disabled,
            State::Quarantined => PluginStatus::Quarantined,
            State::Inactive(_) => PluginStatus::Inactive,
        }
    }
}

enum Hook {
    Activate(String, Option<Arc<dyn Plugin>>),
    Deactivate(String, Option<Arc<dyn Plugin>>),
}

impl Runtime {
    pub fn new(health: Arc<Health>) -> Arc<Runtime> {
        let rt = Arc::new(Runtime {
            events: Arc::new(EventBus::new(health.clone())),
            health: health.clone(),
            table: RwLock::new(ServiceTable::default()),
            state: Mutex::new(Book::default()),
            resolving: Mutex::new(()),
            dirty: AtomicBool::new(false),
        });
        let weak = Arc::downgrade(&rt);
        health.set_observer(Arc::new(move || {
            if let Some(rt) = weak.upgrade() {
                rt.recompute();
            }
        }));
        rt
    }

    fn lock(&self) -> MutexGuard<'_, Book> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn is_installed(&self, id: &str) -> bool {
        self.lock().installed.iter().any(|p| p.manifest.id == id)
    }

    pub fn add(&self, plugin: Installed) {
        self.lock().installed.push(plugin);
        self.recompute();
    }

    /// Runs activation hooks from now on (after every plugin is installed).
    pub fn resolve(&self) {
        self.lock().resolved = true;
        self.recompute();
    }

    /// Disables or re-enables a plugin at run time; `Ok(true)` when it was
    /// disabled at install and needs a restart to register.
    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<bool, String> {
        {
            let mut s = self.lock();
            let Some(p) = s.installed.iter().find(|p| p.manifest.id == id) else {
                return Err(format!("plugin \"{id}\" is not installed"));
            };
            if p.disabled {
                return Ok(enabled);
            }
            if enabled {
                s.runtime_disabled.remove(id);
            } else {
                s.runtime_disabled.insert(id.to_string());
            }
        }
        self.recompute();
        Ok(false)
    }

    /// Forgets a failed activation so the next recompute retries it.
    pub fn clear_activation_error(&self, id: &str) {
        self.lock().activation_errors.remove(id);
    }

    /// Whether `plugin` may be called now; `Err` says why not.
    pub fn available(&self, plugin: &str) -> Result<(), String> {
        self.health.availability(plugin)
    }

    fn snapshot_of(&self, s: &Book) -> Snapshot {
        let failures: Vec<Option<String>> = s
            .installed
            .iter()
            .map(|p| {
                p.failure
                    .as_ref()
                    .map(|f| format!("registration failed: {f}"))
                    .or_else(|| s.activation_errors.get(&p.manifest.id).cloned())
            })
            .collect();
        let nodes: Vec<graph::Node> = s
            .installed
            .iter()
            .zip(&failures)
            .map(|(p, failure)| graph::Node {
                id: &p.manifest.id,
                version: &p.manifest.version,
                requires: &p.manifest.requires,
                disabled: p.disabled || s.runtime_disabled.contains(&p.manifest.id),
                quarantined: self.health.is_quarantined(&p.manifest.id),
                failure: failure.as_deref(),
            })
            .collect();
        let r = graph::resolve(&nodes);
        Snapshot {
            ids: s.installed.iter().map(|p| p.manifest.id.clone()).collect(),
            states: r.states,
            order: r.order,
            warnings: r.warnings,
            dependents: r.dependents,
        }
    }

    /// Re-resolves, applies the result and runs hooks until stable.
    /// Re-entrant calls (from a hook, or a quarantine on another thread)
    /// only mark the state dirty; the running pass picks them up.
    pub fn recompute(&self) {
        loop {
            let Ok(_running) = self.resolving.try_lock() else {
                self.dirty.store(true, Ordering::SeqCst);
                return;
            };
            loop {
                self.dirty.store(false, Ordering::SeqCst);
                self.pass();
                if !self.dirty.load(Ordering::SeqCst) {
                    break;
                }
            }
            drop(_running);
            if !self.dirty.load(Ordering::SeqCst) {
                return;
            }
        }
    }

    fn pass(&self) {
        let (changes, hooks) = {
            let mut s = self.lock();
            let snap = self.snapshot_of(&s);
            let mut blocked = HashMap::new();
            let mut changes = Vec::new();
            for (i, id) in snap.ids.iter().enumerate() {
                match &snap.states[i] {
                    State::Inactive(reason) => {
                        blocked.insert(id.clone(), reason.clone());
                    }
                    State::Disabled => {
                        blocked.insert(id.clone(), "it is disabled".to_string());
                    }
                    State::Active | State::Quarantined => {}
                }
                let status = snap.status(i);
                if let Some(old) = s.last.insert(id.clone(), status) {
                    if old != status {
                        changes.push(PluginStateChanged { id: id.clone(), status });
                    }
                }
            }
            self.health.set_blocked(blocked);
            let mut hooks = Vec::new();
            if s.resolved {
                for &i in snap.order.iter().rev() {
                    let id = &snap.ids[i];
                    if snap.states[i] != State::Active && s.activated.remove(id) {
                        hooks.push(Hook::Deactivate(id.clone(), s.installed[i].plugin.clone()));
                    }
                }
                for &i in &snap.order {
                    let id = &snap.ids[i];
                    if snap.states[i] == State::Active && !s.activated.contains(id) {
                        hooks.push(Hook::Activate(id.clone(), s.installed[i].plugin.clone()));
                    }
                }
            }
            (changes, hooks)
        };
        for hook in hooks {
            match hook {
                Hook::Deactivate(id, plugin) => {
                    if let Some(p) = plugin {
                        let _ = Guard::new(&id, self.health.clone()).call(CallKind::Lifecycle, "deactivate", || {
                            p.deactivate();
                            Ok(())
                        });
                    }
                }
                Hook::Activate(id, plugin) => {
                    let result = match plugin {
                        Some(p) => {
                            let ctx = ActivationCtx {
                                plugin: &id,
                                runtime: self,
                            };
                            Guard::new(&id, self.health.clone()).call(CallKind::Lifecycle, "activate", || p.activate(&ctx))
                        }
                        None => Ok(()),
                    };
                    let mut s = self.lock();
                    match result {
                        Ok(()) => {
                            s.activated.insert(id);
                        }
                        Err(e) => {
                            s.activation_errors.insert(id, format!("activation failed: {e:#}"));
                            drop(s);
                            // Its dependents must not activate on top of it.
                            self.dirty.store(true, Ordering::SeqCst);
                            break;
                        }
                    }
                }
            }
        }
        for c in changes {
            self.events.publish(c);
        }
    }

    /// Runs every activated plugin's deactivation hook, dependents first.
    pub fn shutdown(&self) {
        let hooks: Vec<(String, Arc<dyn Plugin>)> = {
            let mut s = self.lock();
            let snap = self.snapshot_of(&s);
            s.resolved = false;
            let mut out = Vec::new();
            for &i in snap.order.iter().rev() {
                let was_active = s.activated.remove(&snap.ids[i]);
                let plugin = s.installed[i].plugin.clone().filter(|_| was_active);
                out.extend(plugin.map(|p| (snap.ids[i].clone(), p)));
            }
            out
        };
        for (id, p) in hooks {
            let _ = Guard::new(&id, self.health.clone()).call(CallKind::Lifecycle, "deactivate", || {
                p.deactivate();
                Ok(())
            });
        }
    }

    /// Installed plugins with their resolved status and health.
    pub fn infos(&self, extra_warnings: &dyn Fn(&str) -> Vec<String>) -> Vec<PluginInfo> {
        let s = self.lock();
        let snap = self.snapshot_of(&s);
        s.installed
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let id = &p.manifest.id;
                let h = self.health.get(id);
                let mut warnings = snap.warnings[i].clone();
                warnings.extend(extra_warnings(id));
                PluginInfo {
                    id: id.clone(),
                    name: p.name.clone(),
                    version: p.version_text.clone(),
                    contributes: p.contributes.clone(),
                    status: snap.status(i),
                    failures: h.failures,
                    last_error: h.last_error,
                    requires: p.manifest.requires.iter().map(DepInfo::from).collect(),
                    provides: p.manifest.provides.iter().map(ServiceInfo::from).collect(),
                    inactive_reason: match &snap.states[i] {
                        State::Inactive(r) => Some(r.clone()),
                        _ => None,
                    },
                    dependents: snap.dependents[i].clone(),
                    warnings,
                    hung: h.hung,
                }
            })
            .collect()
    }
}
