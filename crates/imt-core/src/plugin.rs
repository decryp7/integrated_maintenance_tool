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

/// Registered plugin, as listed in the workspace summary.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    /// e.g. `["importer:registry", "transform:x", "exporter:y", "hook"]`.
    pub contributes: Vec<String>,
}

/// Everything the engine can be extended with.
pub struct PluginRegistry {
    pub(crate) importers: Importers,
    pub(crate) transforms: HashMap<String, Arc<dyn RecordTransform>>,
    pub(crate) hooks: Vec<Arc<dyn WorkspaceHook>>,
    pub(crate) exporters: Vec<Arc<dyn Exporter>>,
    plugins: Vec<PluginInfo>,
    /// Contributions of the plugin currently registering.
    pending: Vec<String>,
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
            plugins: Vec::new(),
            pending: Vec::new(),
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
        self.pending.clear();
        plugin.register(self).map_err(|e| anyhow!("plugin \"{}\": {e:#}", plugin.id()))?;
        self.plugins.push(PluginInfo {
            id: plugin.id().into(),
            name: plugin.name().into(),
            version: plugin.version().into(),
            contributes: std::mem::take(&mut self.pending),
        });
        Ok(())
    }

    pub fn add_binary_importer(&mut self, importer: Box<dyn crate::importers::BinaryImporter>) -> &mut Self {
        self.pending.push(format!("importer:{}", importer.id()));
        self.importers.register_binary(importer);
        self
    }

    pub fn add_importer(&mut self, importer: Box<dyn Importer>) -> &mut Self {
        self.pending.push(format!("importer:{}", importer.id()));
        self.importers.register(importer);
        self
    }

    pub fn add_transform(&mut self, transform: Arc<dyn RecordTransform>) -> Result<&mut Self> {
        let id = transform.id().to_string();
        if self.transforms.contains_key(&id) {
            return Err(anyhow!("a transform with id \"{id}\" is already registered"));
        }
        self.pending.push(format!("transform:{id}"));
        self.transforms.insert(id, transform);
        Ok(self)
    }

    pub fn add_hook(&mut self, hook: Arc<dyn WorkspaceHook>) -> &mut Self {
        self.pending.push("hook".into());
        self.hooks.push(hook);
        self
    }

    pub fn add_exporter(&mut self, exporter: Arc<dyn Exporter>) -> Result<&mut Self> {
        if self.exporters.iter().any(|e| e.id() == exporter.id()) {
            return Err(anyhow!("an exporter with id \"{}\" is already registered", exporter.id()));
        }
        self.pending.push(format!("exporter:{}", exporter.id()));
        self.exporters.push(exporter);
        Ok(self)
    }

    pub fn plugins(&self) -> &[PluginInfo] {
        &self.plugins
    }

    pub fn exporter(&self, id: &str) -> Option<&Arc<dyn Exporter>> {
        self.exporters.iter().find(|e| e.id() == id)
    }
}
