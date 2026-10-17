//! Compiled-in plugins and the extension points they contribute to.
//!
//! A [`Plugin`] registers contributions into a [`PluginRegistry`]:
//!
//! | Extension point | Trait | Used for |
//! |---|---|---|
//! | Importer | [`Importer`] / [`crate::importers::BinaryImporter`] | reading a new text / binary file format |
//! | Record transform | [`RecordTransform`] | a `{ step = "plugin", id = ... }` pipeline step |
//! | Workspace hook | [`WorkspaceHook`] | reacting to loads, clears and profile changes |
//! | Exporter | [`Exporter`] | writing events in a new format |
//! | Collector | [`Collector`] | producing events (monitors, probes) into an [`crate::export::EventSink`] |
//! | Settings | [`crate::settings::Configurable`] via [`Plugin::configurable`] | user-facing knobs in the Settings screen |
//!
//! See `docs/plugins.md` for a step-by-step guide.

use crate::api::WorkspaceSummary;
use crate::export::{ClefExporter, CsvExporter, Exporter};
use crate::importers::{Importer, Importers};
use crate::model::Record;
use crate::profile::Profile;
use anyhow::{anyhow, Result};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;

/// Version of the plugin API. A plugin built against another version is
/// rejected at registration instead of misbehaving at run time.
pub const API_VERSION: u32 = 1;

/// A bundle of contributions, registered once at startup.
pub trait Plugin: Send + Sync {
    /// Stable, unique id, reverse-DNS style (`"net.decryptology.registry"`).
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    fn version(&self) -> &'static str;
    /// The [`API_VERSION`] the plugin was written against.
    fn api_version(&self) -> u32 {
        API_VERSION
    }
    /// Adds the plugin's contributions. Must not panic; return an error instead.
    fn register(&self, registry: &mut PluginRegistry) -> Result<()>;
    /// The plugin's user-facing settings, if it has any. Asked once at install,
    /// even when the plugin is disabled, so the Settings screen can still show
    /// the section (read-only). Share state between the returned object and
    /// the contributions made in [`Plugin::register`].
    fn configurable(&self) -> Option<Arc<dyn crate::settings::Configurable>> {
        None
    }
}

/// What a pipeline step decides for one record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Keep,
    Drop,
}

/// Context passed to transforms.
pub struct TransformCtx<'a> {
    /// Display name of the file the record came from.
    pub source_name: &'a str,
}

/// A record-level pipeline step, referenced from a profile as
/// `{ step = "plugin", id = "<id>" }`. Runs during import, in parallel across
/// files, after the profile's extractors and before marker rules, so it must
/// be cheap, deterministic and free of shared mutable state.
pub trait RecordTransform: Send + Sync {
    fn id(&self) -> &str;
    fn apply(&self, record: &mut Record, ctx: &TransformCtx) -> Verdict;
}

/// Notifications about the workspace. Called synchronously on the thread that
/// changed the workspace: keep them fast and never block.
pub trait WorkspaceHook: Send + Sync {
    /// Files were opened or re-imported.
    fn on_loaded(&self, _summary: &WorkspaceSummary) {}
    /// Everything was closed.
    fn on_cleared(&self) {}
    /// A new profile was applied (before re-import).
    fn on_profile_changed(&self, _profile: &Profile) {}
}

/// Produces events instead of reading files (e.g. a process monitor). The host
/// runs `run` on its own thread until `stop` becomes true, then joins it.
pub trait Collector: Send + Sync {
    /// Stable id (`"process-monitor"`).
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    /// Collects until `stop` is set, writing events to `sink`. `config` is the
    /// collector's own settings (JSON, validated by the collector).
    fn run(&self, config: &serde_json::Value, sink: &dyn crate::export::EventSink, stop: &std::sync::atomic::AtomicBool) -> Result<()>;
}

/// Whether a plugin's contributions are being used.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PluginStatus {
    Active,
    /// Turned off by the user (or safe mode); not registered at all.
    Disabled,
    /// Crashed repeatedly; installed but no longer called until reset.
    Quarantined,
}

/// Registered plugin, as listed in the workspace summary.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    /// e.g. `["importer:registry", "transform:x", "exporter:y", "hook"]`.
    pub contributes: Vec<String>,
    pub status: PluginStatus,
    pub failures: u32,
    pub last_error: Option<String>,
}

/// Everything the engine can be extended with.
pub struct PluginRegistry {
    pub(crate) importers: Importers,
    pub(crate) transforms: HashMap<String, Arc<dyn RecordTransform>>,
    pub(crate) hooks: Vec<Arc<dyn WorkspaceHook>>,
    pub(crate) exporters: Vec<Arc<dyn Exporter>>,
    collectors: Vec<Arc<dyn Collector>>,
    plugins: Vec<PluginInfo>,
    /// Contributions of the plugin currently registering.
    pending: Vec<String>,
    /// Guard of the plugin currently registering; built-ins are not wrapped.
    current: Option<crate::guard::Guard>,
    health: Arc<crate::guard::Health>,
    disabled: std::collections::HashSet<String>,
    /// Settings of installed plugins (including disabled ones), by plugin id.
    configurables: Vec<(String, Arc<crate::settings::GuardedConfigurable>)>,
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginRegistry {
    /// The built-in importers (json, csv, text) and exporters (csv, clef).
    pub fn new() -> Self {
        let mut r = Self::empty();
        r.importers = Importers::new();
        r.exporters = vec![Arc::new(CsvExporter), Arc::new(ClefExporter)];
        r
    }

    /// No contributions at all.
    pub fn empty() -> Self {
        PluginRegistry {
            importers: Importers::empty(),
            transforms: HashMap::new(),
            hooks: Vec::new(),
            exporters: Vec::new(),
            collectors: Vec::new(),
            plugins: Vec::new(),
            pending: Vec::new(),
            current: None,
            health: Arc::new(crate::guard::Health::default()),
            disabled: Default::default(),
            configurables: Vec::new(),
        }
    }

    /// Registers a plugin and records what it contributed.
    pub fn install(&mut self, plugin: &dyn Plugin) -> Result<()> {
        if plugin.api_version() != API_VERSION {
            return Err(anyhow!(
                "plugin \"{}\" targets plugin API {}, this IMT provides {API_VERSION}",
                plugin.id(),
                plugin.api_version()
            ));
        }
        if self.plugins.iter().any(|p| p.id == plugin.id()) {
            return Err(anyhow!("plugin \"{}\" is already installed", plugin.id()));
        }
        let mut info = PluginInfo {
            id: plugin.id().into(),
            name: plugin.name().into(),
            version: plugin.version().into(),
            contributes: Vec::new(),
            status: PluginStatus::Active,
            failures: 0,
            last_error: None,
        };
        let guard = crate::guard::Guard {
            owner: plugin.id().into(),
            health: self.health.clone(),
        };
        let settings = crate::settings::GuardedConfigurable::describe(plugin, guard.clone()).map(|c| (plugin.id().to_string(), c));
        if self.disabled.contains(plugin.id()) {
            info.status = PluginStatus::Disabled;
            self.plugins.push(info);
            self.configurables.extend(settings);
            return Ok(());
        }
        self.pending.clear();
        self.current = Some(guard);
        let registered = crate::guard::catch(&format!("plugin \"{}\" (register)", plugin.id()), || plugin.register(self));
        self.current = None;
        registered.map_err(|e| anyhow!("plugin \"{}\": {e:#}", plugin.id()))?;
        info.contributes = std::mem::take(&mut self.pending);
        if settings.is_some() {
            info.contributes.push("settings".into());
        }
        self.plugins.push(info);
        self.configurables.extend(settings);
        Ok(())
    }

    /// Plugins to skip at install (user choice or safe mode); call before installing.
    pub fn disable<I: IntoIterator<Item = S>, S: Into<String>>(&mut self, ids: I) -> &mut Self {
        self.disabled.extend(ids.into_iter().map(Into::into));
        self
    }

    /// Crash counts shared by every guarded contribution.
    pub fn health(&self) -> Arc<crate::guard::Health> {
        self.health.clone()
    }

    pub fn add_binary_importer(&mut self, importer: Box<dyn crate::importers::BinaryImporter>) -> &mut Self {
        self.pending.push(format!("importer:{}", importer.id()));
        let importer: Box<dyn crate::importers::BinaryImporter> = match &self.current {
            Some(guard) => Box::new(crate::guard::GuardedBinaryImporter {
                inner: importer,
                guard: guard.clone(),
            }),
            None => importer,
        };
        self.importers.register_binary(importer);
        self
    }

    pub fn add_importer(&mut self, importer: Box<dyn Importer>) -> &mut Self {
        self.pending.push(format!("importer:{}", importer.id()));
        let importer: Box<dyn Importer> = match &self.current {
            Some(guard) => Box::new(crate::guard::GuardedImporter {
                inner: importer,
                guard: guard.clone(),
            }),
            None => importer,
        };
        self.importers.register(importer);
        self
    }

    pub fn add_transform(&mut self, transform: Arc<dyn RecordTransform>) -> Result<&mut Self> {
        let id = transform.id().to_string();
        if self.transforms.contains_key(&id) {
            return Err(anyhow!("a transform with id \"{id}\" is already registered"));
        }
        self.pending.push(format!("transform:{id}"));
        let transform: Arc<dyn RecordTransform> = match &self.current {
            Some(guard) => Arc::new(crate::guard::GuardedTransform {
                inner: transform,
                guard: guard.clone(),
            }),
            None => transform,
        };
        self.transforms.insert(id, transform);
        Ok(self)
    }

    pub fn add_hook(&mut self, hook: Arc<dyn WorkspaceHook>) -> &mut Self {
        self.pending.push("hook".into());
        let hook: Arc<dyn WorkspaceHook> = match &self.current {
            Some(guard) => Arc::new(crate::guard::GuardedHook {
                inner: hook,
                guard: guard.clone(),
            }),
            None => hook,
        };
        self.hooks.push(hook);
        self
    }

    pub fn add_exporter(&mut self, exporter: Arc<dyn Exporter>) -> Result<&mut Self> {
        if self.exporters.iter().any(|e| e.id() == exporter.id()) {
            return Err(anyhow!("an exporter with id \"{}\" is already registered", exporter.id()));
        }
        self.pending.push(format!("exporter:{}", exporter.id()));
        let exporter: Arc<dyn Exporter> = match &self.current {
            Some(guard) => Arc::new(crate::guard::GuardedExporter {
                inner: exporter,
                guard: guard.clone(),
            }),
            None => exporter,
        };
        self.exporters.push(exporter);
        Ok(self)
    }

    pub fn add_collector(&mut self, collector: Arc<dyn Collector>) -> Result<&mut Self> {
        if self.collector(collector.id()).is_some() {
            return Err(anyhow!("a collector with id \"{}\" is already registered", collector.id()));
        }
        self.pending.push(format!("collector:{}", collector.id()));
        let collector: Arc<dyn Collector> = match &self.current {
            Some(guard) => Arc::new(crate::guard::GuardedCollector {
                inner: collector,
                guard: guard.clone(),
            }),
            None => collector,
        };
        self.collectors.push(collector);
        Ok(self)
    }

    pub fn collector(&self, id: &str) -> Option<&Arc<dyn Collector>> {
        self.collectors.iter().find(|c| c.id() == id)
    }

    pub fn collectors(&self) -> &[Arc<dyn Collector>] {
        &self.collectors
    }

    /// Installed plugins with their current health.
    pub fn plugins(&self) -> Vec<PluginInfo> {
        self.plugins
            .iter()
            .map(|p| {
                let h = self.health.get(&p.id);
                let status = match p.status {
                    PluginStatus::Active if h.quarantined => PluginStatus::Quarantined,
                    s => s,
                };
                PluginInfo {
                    status,
                    failures: h.failures,
                    last_error: h.last_error,
                    ..p.clone()
                }
            })
            .collect()
    }

    /// Settings sections of installed plugins (also disabled and quarantined
    /// ones, which are read-only), with their current status.
    pub fn plugin_settings(&self) -> Vec<crate::settings::PluginSettings> {
        let plugins = self.plugins();
        self.configurables
            .iter()
            .filter_map(|(id, c)| {
                let plugin = plugins.iter().find(|p| p.id == *id)?.clone();
                let configurable: Arc<dyn crate::settings::Configurable> = c.clone();
                Some(crate::settings::PluginSettings { plugin, configurable })
            })
            .collect()
    }

    /// Lifts a quarantine and forgets a plugin's crashes.
    pub fn reset_plugin(&self, id: &str) {
        self.health.reset(id);
    }

    pub fn exporter(&self, id: &str) -> Option<&Arc<dyn Exporter>> {
        self.exporters.iter().find(|e| e.id() == id)
    }
}
