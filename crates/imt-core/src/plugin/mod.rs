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
//! | Services | any trait, via [`PluginRegistry::provide`] | APIs other plugins call ([`Service`]) |
//! | Extension points | any trait, via [`PluginRegistry::declare_extension_point`] | plugin-defined contribution points |
//! | Events | [`crate::events::EventBus`] | reacting to records, profile, alerts and plugin state |
//! | Tool | [`tools::Tool`] | typed operations the shell dispatches generically (PerfMon, process monitor, ...) |
//!
//! Plugins declare a [`Manifest`] (version, dependencies, provided services).
//! The registry resolves the dependency graph ([`graph`]): plugins with an
//! unavailable required dependency are inactive with a stated reason, and
//! quarantining or disabling a provider deactivates its dependents. See
//! `docs/plugins.md` for a step-by-step guide.

pub mod annotate;
mod graph;
pub mod hosting;
pub mod manifest;
pub mod markers;
mod runtime;
pub mod services;
pub mod tools;
pub mod watchdog;

pub use manifest::{Dep, DepInfo, Manifest, ServiceDecl, ServiceInfo};
pub use services::Service;

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
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    fn version(&self) -> &str;
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
    /// Identity, dependencies and provided services. The default has no
    /// dependencies and is derived from `id`, `version` and `api_version`.
    fn manifest(&self) -> Manifest {
        Manifest::new(self.id(), self.version()).api_version(self.api_version())
    }
    /// Called once the plugin's required dependencies are active (after
    /// every plugin registered, in dependency order): look up services and
    /// contributions here. An error leaves the plugin inactive with that
    /// reason. Runs only for plugins installed with
    /// [`PluginRegistry::install_shared`] (the registry must keep them).
    fn activate(&self, _ctx: &ActivationCtx) -> Result<()> {
        Ok(())
    }
    /// Called when the plugin stops being active (a dependency failed, it was
    /// disabled, or the host shuts down), dependents first. Drop service
    /// handles here.
    fn deactivate(&self) {}
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
    marks: Option<&'a markers::MarkIndex>,
    scope: Option<&'a markers::MarkScope<'a>>,
}

impl<'a> TransformCtx<'a> {
    pub fn new(source_name: &'a str) -> Self {
        TransformCtx {
            source_name,
            marks: None,
            scope: None,
        }
    }

    /// A context whose transforms may mark records with the kinds in
    /// `marks` that their plugin declared (see [`markers::MarkIndex::of_kinds`]).
    pub fn with_marks(source_name: &'a str, marks: &'a markers::MarkIndex) -> Self {
        TransformCtx {
            source_name,
            marks: Some(marks),
            scope: None,
        }
    }

    /// The same context, recording marks for `scope`'s plugin.
    pub(crate) fn scoped<'b>(&'b self, scope: &'b markers::MarkScope<'b>) -> TransformCtx<'b> {
        TransformCtx {
            source_name: self.source_name,
            marks: self.marks,
            scope: Some(scope),
        }
    }

    /// Marks the record being transformed with a marker kind the calling
    /// plugin declared ([`PluginRegistry::add_marker_kinds`]), by its short
    /// (`app-crash`) or full (`<plugin-id>/app-crash`) name. Returns false and
    /// records a warning on the plugin when the kind is not its own (or the
    /// kind is switched off by the profile).
    pub fn mark(&self, kind: &str) -> bool {
        match (self.scope, self.marks) {
            (Some(scope), Some(marks)) => scope.mark(marks, kind),
            (Some(scope), None) => scope.reject(kind),
            (None, _) => false,
        }
    }

    /// [`TransformCtx::mark`] for [`RecordTransform::apply_batch`]: marks
    /// `record` (one of the batch) instead of the record being transformed.
    pub fn mark_record(&self, record: &mut Record, kind: &str) -> bool {
        match (self.scope, self.marks) {
            (Some(scope), Some(marks)) => match scope.bit_of(marks, kind) {
                Some(bit) => {
                    record.markers |= bit;
                    true
                }
                None => scope.reject(kind),
            },
            (Some(scope), None) => scope.reject(kind),
            (None, _) => false,
        }
    }
}

/// A record-level pipeline step, referenced from a profile as
/// `{ step = "plugin", id = "<id>" }`. Runs during import, in parallel across
/// files, after the profile's extractors and before marker rules, so it must
/// be cheap, deterministic and free of shared mutable state.
pub trait RecordTransform: Send + Sync {
    fn id(&self) -> &str;
    fn apply(&self, record: &mut Record, ctx: &TransformCtx) -> Verdict;
    /// Whether the pipeline should hand this transform all records of a
    /// step at once ([`RecordTransform::apply_batch`]) instead of one by one.
    /// True for transforms whose per-call cost dominates, such as those of a
    /// plugin running out of process (one round trip per batch, not per record).
    fn batched(&self) -> bool {
        false
    }
    /// Applies the step to many records, returning one verdict per record
    /// (missing verdicts mean [`Verdict::Keep`]). Marks go through
    /// [`TransformCtx::mark_record`]. The default calls `apply` on each.
    fn apply_batch(&self, records: &mut [Record], ctx: &TransformCtx) -> Vec<Verdict> {
        records.iter_mut().map(|r| self.apply(r, ctx)).collect()
    }
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
    /// Turned off by the user (or safe mode).
    Disabled,
    /// Crashed repeatedly; installed but no longer called until reset.
    Quarantined,
    /// Installed and enabled, but not running: a required dependency is
    /// unavailable, or registration/activation failed (see `inactive_reason`).
    Inactive,
}

/// Registered plugin, as listed in the workspace summary.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    /// e.g. `["importer:registry", "transform:x", "exporter:y", "hook", "service:z"]`.
    pub contributes: Vec<String>,
    pub status: PluginStatus,
    pub failures: u32,
    pub last_error: Option<String>,
    /// Dependencies from the manifest.
    pub requires: Vec<DepInfo>,
    /// Services the plugin provides.
    pub provides: Vec<ServiceInfo>,
    /// Why an `inactive` plugin is not running, e.g.
    /// `needs net.decryptology.perfmon ^1.0 (installed 0.9.0)`.
    pub inactive_reason: Option<String>,
    /// Plugins that require this one (they stop when it stops).
    pub dependents: Vec<String>,
    /// Non-fatal problems: unavailable optional dependencies, inactive contributions.
    pub warnings: Vec<String>,
    /// A call is running past its time budget right now.
    pub hung: bool,
}

/// What [`Plugin::activate`] can reach: services and contributions of the
/// other plugins, and the event bus.
pub struct ActivationCtx<'a> {
    plugin: &'a str,
    runtime: &'a runtime::Runtime,
}

impl ActivationCtx<'_> {
    /// The plugin being activated.
    pub fn plugin_id(&self) -> &str {
        self.plugin
    }

    /// A service by id and semver requirement (e.g. `"^1.0"`), as a guarded handle.
    pub fn service<T: ?Sized + Send + Sync + 'static>(&self, id: &str, req: &str) -> Result<Service<T>> {
        service_from(self.runtime, id, req)
    }

    /// Contributions to an extension point.
    pub fn contributions<T: ?Sized + Send + Sync + 'static>(&self, point: &str) -> Vec<Service<T>> {
        contributions_from(self.runtime, point)
    }

    pub fn events(&self) -> &Arc<crate::events::EventBus> {
        &self.runtime.events
    }

    /// Whether another plugin can be called (e.g. an optional dependency).
    pub fn is_available(&self, plugin: &str) -> bool {
        self.runtime.is_installed(plugin) && self.runtime.available(plugin).is_ok()
    }
}

fn service_from<T: ?Sized + Send + Sync + 'static>(rt: &runtime::Runtime, id: &str, req: &str) -> Result<Service<T>> {
    let req =
        semver::VersionReq::parse(req.trim()).map_err(|e| anyhow!("service \"{id}\": version requirement \"{req}\" is not valid ({e})"))?;
    let table = rt.table.read().unwrap_or_else(|p| p.into_inner());
    table.service::<T>(id, &req, &|owner| rt.available(owner))
}

fn contributions_from<T: ?Sized + Send + Sync + 'static>(rt: &runtime::Runtime, point: &str) -> Vec<Service<T>> {
    let table = rt.table.read().unwrap_or_else(|p| p.into_inner());
    table.contributions::<T>(point, &|owner| rt.available(owner))
}

/// The plugin whose `register` is running.
struct Registering {
    guard: crate::guard::Guard,
    manifest: Manifest,
}

/// Everything the engine can be extended with.
pub struct PluginRegistry {
    pub(crate) importers: Importers,
    pub(crate) transforms: HashMap<String, Arc<dyn RecordTransform>>,
    pub(crate) hooks: Vec<Arc<dyn WorkspaceHook>>,
    pub(crate) exporters: Vec<Arc<dyn Exporter>>,
    collectors: Vec<Arc<dyn Collector>>,
    /// Contributions of the plugin currently registering.
    pending: Vec<String>,
    /// The plugin currently registering; built-ins are not wrapped.
    current: Option<Registering>,
    runtime: Arc<runtime::Runtime>,
    disabled: std::collections::HashSet<String>,
    /// Settings of installed plugins (including disabled ones), by plugin id.
    configurables: Vec<(String, Arc<crate::settings::GuardedConfigurable>)>,
    /// Marker kinds contributed by plugins, by owner.
    marker_kinds: Vec<(String, markers::PluginMarkerKind)>,
    /// Notes (owner, text) from the last marker set built.
    marker_notes: std::sync::Mutex<Vec<(String, String)>>,
    annotators: Vec<annotate::Registered>,
    annotations: annotate::Store,
    /// Notes (owner, text) from the last annotator run.
    annotate_notes: std::sync::Mutex<Vec<(String, String)>>,
    tools: Vec<Arc<tools::Registered>>,
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
        // Configuration formats plugins add (see `crate::config`); the host declares the point.
        let _ = r.declare_extension_point::<dyn crate::config::ConfigParser>(crate::config::PARSERS_POINT);
        r.pending.clear();
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
            pending: Vec::new(),
            current: None,
            runtime: runtime::Runtime::new(Arc::new(crate::guard::Health::default())),
            disabled: Default::default(),
            configurables: Vec::new(),
            marker_kinds: Vec::new(),
            marker_notes: Default::default(),
            annotators: Vec::new(),
            annotations: Default::default(),
            annotate_notes: Default::default(),
            tools: Vec::new(),
        }
    }

    /// Registers a plugin and records what it contributed. The registry does
    /// not keep `plugin`, so its [`Plugin::activate`]/[`Plugin::deactivate`]
    /// hooks are not called; use [`PluginRegistry::install_shared`] for that.
    pub fn install(&mut self, plugin: &dyn Plugin) -> Result<()> {
        self.install_inner(plugin, None)
    }

    /// Registers a plugin and keeps it, so its activation hooks run once its
    /// dependencies are active (after [`PluginRegistry::resolve`]).
    pub fn install_shared(&mut self, plugin: Arc<dyn Plugin>) -> Result<()> {
        self.install_inner(plugin.as_ref(), Some(plugin.clone()))
    }

    fn install_inner(&mut self, plugin: &dyn Plugin, keep: Option<Arc<dyn Plugin>>) -> Result<()> {
        let manifest = plugin.manifest();
        if manifest.api_version != API_VERSION {
            return Err(anyhow!(
                "plugin \"{}\" targets plugin API {}, this IMT provides {API_VERSION}",
                plugin.id(),
                manifest.api_version
            ));
        }
        if manifest.id != plugin.id() {
            return Err(anyhow!(
                "plugin \"{}\": its manifest names a different id \"{}\"",
                plugin.id(),
                manifest.id
            ));
        }
        manifest.check().map_err(|e| anyhow!("plugin \"{}\": {e}", plugin.id()))?;
        if self.runtime.is_installed(plugin.id()) {
            return Err(anyhow!("plugin \"{}\" is already installed", plugin.id()));
        }
        let guard = crate::guard::Guard::new(plugin.id(), self.runtime.health.clone());
        let settings = crate::settings::GuardedConfigurable::describe(plugin, guard.clone()).map(|c| (plugin.id().to_string(), c));
        let disabled = self.disabled.contains(plugin.id());
        let mut installed = runtime::Installed {
            name: plugin.name().into(),
            version_text: plugin.version().into(),
            manifest,
            disabled,
            contributes: Vec::new(),
            failure: None,
            plugin: keep,
        };
        let mut failure = None;
        if !disabled {
            self.pending.clear();
            self.current = Some(Registering {
                guard,
                manifest: installed.manifest.clone(),
            });
            let registered = crate::guard::catch(&format!("plugin \"{}\" (register)", plugin.id()), || plugin.register(self));
            // Services provided without a manifest entry are recorded on it.
            if let Some(current) = self.current.take() {
                installed.manifest = current.manifest;
            }
            if let Err(e) = registered {
                installed.failure = Some(format!("{e:#}"));
                failure = Some(e);
            }
            installed.contributes = std::mem::take(&mut self.pending);
            if settings.is_some() {
                installed.contributes.push("settings".into());
            }
        }
        // A plugin that failed to register shows no settings section.
        if failure.is_none() {
            self.configurables.extend(settings);
        }
        self.runtime.add(installed);
        match failure {
            Some(e) => Err(anyhow!("plugin \"{}\": {e:#}", plugin.id())),
            None => Ok(()),
        }
    }

    /// Plugins to skip at install (user choice or safe mode); call before installing.
    pub fn disable<I: IntoIterator<Item = S>, S: Into<String>>(&mut self, ids: I) -> &mut Self {
        self.disabled.extend(ids.into_iter().map(Into::into));
        self
    }

    /// The installed importers (built-in and plugin), for stores that read files.
    pub fn importers(&self) -> &Importers {
        &self.importers
    }

    /// Runs activation hooks in dependency order. Call once every plugin is
    /// installed; [`crate::Workspace::with_registry`] calls it too (it is idempotent).
    pub fn resolve(&self) -> &Self {
        self.runtime.resolve();
        self
    }

    /// Disables (or re-enables) an installed plugin now; its dependents
    /// become inactive (or active again). Returns `true` when the plugin was
    /// disabled at install and needs a restart to register.
    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<bool> {
        self.runtime.set_enabled(id, enabled).map_err(|e| anyhow!(e))
    }

    /// Deactivates every plugin, dependents first (on exit).
    pub fn shutdown(&self) {
        self.runtime.shutdown();
    }

    /// Crash counts shared by every guarded contribution.
    pub fn health(&self) -> Arc<crate::guard::Health> {
        self.runtime.health.clone()
    }

    /// Time budgets of guarded calls (see [`crate::guard::Budgets`]).
    pub fn set_budgets(&self, budgets: crate::guard::Budgets) -> &Self {
        self.runtime.health.set_budgets(budgets);
        self
    }

    /// The event bus shared by the engine and every plugin.
    pub fn events(&self) -> Arc<crate::events::EventBus> {
        self.runtime.events.clone()
    }

    /// The registering plugin's id, guard and version (the host's otherwise).
    fn owner(&self) -> (String, Option<crate::guard::Guard>, semver::Version) {
        match &self.current {
            Some(c) => (c.manifest.id.clone(), Some(c.guard.clone()), c.manifest.version.clone()),
            None => (services::HOST.to_string(), None, semver::Version::new(0, 0, 0)),
        }
    }

    /// Provides a service other plugins look up with [`PluginRegistry::service`]
    /// or [`ActivationCtx::service`]. Call from `register`; `version` is semver
    /// and must match the manifest's `provides` entry when there is one.
    pub fn provide<T: ?Sized + Send + Sync + 'static>(&mut self, id: &str, version: &str, service: Arc<T>) -> Result<&mut Self> {
        let version = manifest::parse_version(version).map_err(|e| anyhow!("service \"{id}\": {e}"))?;
        if let Some(c) = &mut self.current {
            match c.manifest.provides.iter().find(|d| d.id == id) {
                Some(d) if d.version != version => {
                    return Err(anyhow!(
                        "service \"{id}\" is provided as {version} but the manifest declares {}",
                        d.version
                    ))
                }
                Some(_) => {}
                None => c.manifest.provides.push(ServiceDecl {
                    id: id.into(),
                    version: version.clone(),
                }),
            }
        }
        let (owner, guard, _) = self.owner();
        self.runtime
            .table
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .provide(&owner, guard, id, version, service)?;
        self.pending.push(format!("service:{id}"));
        Ok(self)
    }

    /// A service by id and semver requirement, as a guarded handle. Plugins
    /// should look services up in [`Plugin::activate`] instead.
    pub fn service<T: ?Sized + Send + Sync + 'static>(&self, id: &str, req: &str) -> Result<Service<T>> {
        service_from(&self.runtime, id, req)
    }

    /// Declares an extension point other plugins contribute `T`s to.
    pub fn declare_extension_point<T: ?Sized + 'static>(&mut self, id: &str) -> Result<&mut Self> {
        let (owner, _, _) = self.owner();
        self.runtime
            .table
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .declare::<T>(&owner, id)?;
        self.pending.push(format!("extension-point:{id}"));
        Ok(self)
    }

    /// Contributes to an extension point (declared by any plugin, in any order).
    pub fn contribute<T: ?Sized + Send + Sync + 'static>(&mut self, point: &str, item: Arc<T>) -> &mut Self {
        let (owner, guard, version) = self.owner();
        self.runtime
            .table
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .contribute(&owner, guard, version, point, item);
        self.pending.push(format!("contribution:{point}"));
        self
    }

    /// Contributions to an extension point from available plugins, guarded.
    pub fn contributions<T: ?Sized + Send + Sync + 'static>(&self, point: &str) -> Vec<Service<T>> {
        contributions_from(&self.runtime, point)
    }

    fn guard(&self) -> Option<crate::guard::Guard> {
        self.current.as_ref().map(|c| c.guard.clone())
    }

    pub fn add_binary_importer(&mut self, importer: Box<dyn crate::importers::BinaryImporter>) -> &mut Self {
        self.pending.push(format!("importer:{}", importer.id()));
        let importer: Box<dyn crate::importers::BinaryImporter> = match self.guard() {
            Some(guard) => Box::new(crate::guard::GuardedBinaryImporter {
                inner: Arc::from(importer),
                guard,
            }),
            None => importer,
        };
        self.importers.register_binary(importer);
        self
    }

    pub fn add_importer(&mut self, importer: Box<dyn Importer>) -> &mut Self {
        self.pending.push(format!("importer:{}", importer.id()));
        let importer: Box<dyn Importer> = match self.guard() {
            Some(guard) => Box::new(crate::guard::GuardedImporter {
                inner: Arc::from(importer),
                guard,
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
        let transform: Arc<dyn RecordTransform> = match self.guard() {
            Some(guard) => Arc::new(crate::guard::GuardedTransform { inner: transform, guard }),
            None => transform,
        };
        self.transforms.insert(id, transform);
        Ok(self)
    }

    pub fn add_hook(&mut self, hook: Arc<dyn WorkspaceHook>) -> &mut Self {
        self.pending.push("hook".into());
        let hook: Arc<dyn WorkspaceHook> = match self.guard() {
            Some(guard) => Arc::new(crate::guard::GuardedHook { inner: hook, guard }),
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
        let exporter: Arc<dyn Exporter> = match self.guard() {
            Some(guard) => Arc::new(crate::guard::GuardedExporter { inner: exporter, guard }),
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
        let collector: Arc<dyn Collector> = match self.guard() {
            Some(guard) => Arc::new(crate::guard::GuardedCollector { inner: collector, guard }),
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

    /// Installed plugins with their resolved status and health.
    pub fn plugins(&self) -> Vec<PluginInfo> {
        let table = self.runtime.table.read().unwrap_or_else(|p| p.into_inner());
        let marker_notes = self.marker_notes.lock().unwrap_or_else(|p| p.into_inner()).clone();
        let annotate_notes = self.annotate_notes.lock().unwrap_or_else(|p| p.into_inner()).clone();
        self.runtime.infos(&|id| {
            let mut w = table.warnings(id);
            let kinds = self.marker_kinds.iter().filter(|(o, _)| o == id).count();
            if kinds > 0 && self.runtime.available(id).is_err() {
                w.push(format!("its {kinds} marker kind(s) are hidden until the plugin is active again"));
            }
            w.extend(
                marker_notes
                    .iter()
                    .chain(&annotate_notes)
                    .filter(|(o, _)| o == id)
                    .map(|(_, n)| n.clone()),
            );
            w.extend(self.runtime.health.notes(id));
            w
        })
    }

    /// Adds marker kinds (call from `register`). Each is shown as
    /// `<plugin-id>/<kind>`; its rules use the profile condition language.
    pub fn add_marker_kinds(&mut self, kinds: Vec<markers::PluginMarkerKind>) -> Result<&mut Self> {
        let (owner, _, _) = self.owner();
        for k in kinds {
            markers::check(&k, &owner).map_err(|e| anyhow!("{e:#}"))?;
            if self.marker_kinds.iter().any(|(o, x)| *o == owner && x.kind == k.kind) {
                return Err(anyhow!("marker kind \"{owner}/{}\" is declared twice", k.kind));
            }
            self.pending.push(format!("marker:{}", k.kind));
            self.marker_kinds.push((owner.clone(), k));
        }
        Ok(self)
    }

    /// The default rule of the plugin marker kind `kind` (full name) owned
    /// by `owner`; empty for kinds of the profile or built-ins.
    pub fn marker_rule(&self, owner: &str, kind: &str) -> Vec<crate::condition::ConditionCfg> {
        self.marker_kinds
            .iter()
            .find(|(o, k)| o == owner && kind.strip_prefix(o.as_str()).and_then(|r| r.strip_prefix('/')) == Some(k.kind.as_str()))
            .map(|(_, k)| k.when.clone())
            .unwrap_or_default()
    }

    /// Adds an annotator (call from `register`).
    pub fn add_annotator(&mut self, annotator: Arc<dyn annotate::Annotator>) -> Result<&mut Self> {
        let id = annotator.id().to_string();
        if self.annotators.iter().any(|a| a.annotator.id() == id) {
            return Err(anyhow!("an annotator with id \"{id}\" is already registered"));
        }
        let (owner, guard, _) = self.owner();
        self.pending.push(format!("annotator:{id}"));
        self.annotators.push(annotate::Registered { owner, guard, annotator });
        Ok(self)
    }

    /// Adds a tool (call from `register`). Its description is checked and
    /// its id must be unique across plugins.
    pub fn add_tool(&mut self, tool: Arc<dyn tools::Tool>) -> Result<&mut Self> {
        let info = tool.describe();
        info.check().map_err(|e| anyhow!(e))?;
        if info.id != tool.id() {
            return Err(anyhow!("tool \"{}\" describes itself as \"{}\"", tool.id(), info.id));
        }
        if let Some(t) = self.tools.iter().find(|t| t.info.id == info.id) {
            return Err(anyhow!("a tool with id \"{}\" is already registered by \"{}\"", info.id, t.owner));
        }
        let (owner, guard, _) = self.owner();
        self.pending.push(format!("tool:{}", info.id));
        self.tools.push(Arc::new(tools::Registered { owner, guard, info, tool }));
        Ok(self)
    }

    /// The registered tools, for listing and dispatch by the shell.
    pub fn tools(&self) -> tools::ToolHost {
        tools::ToolHost::new(&self.tools)
    }

    /// Compiles a profile against the installed plugins: their transforms
    /// and their marker kinds (see [`markers`]).
    pub fn compile_profile(&self, profile: crate::profile::Profile) -> Result<crate::profile::CompiledProfile> {
        let mut compiled = crate::profile::CompiledProfile::compile_with(profile, &self.transforms)?;
        let notes = markers::apply(&mut compiled, &self.marker_kinds)?;
        compiled.resolve_spans();
        *self.marker_notes.lock().unwrap_or_else(|p| p.into_inner()) = notes;
        Ok(compiled)
    }

    /// Bits of `markers` that may be shown now (kinds of unavailable plugins are hidden).
    pub fn visible_markers(&self, markers: &[crate::profile::Marker]) -> crate::model::MarkerMask {
        markers::visible_mask(markers, |owner| self.runtime.available(owner).is_ok())
    }

    pub fn has_annotators(&self) -> bool {
        !self.annotators.is_empty()
    }

    /// Runs every annotator of an available plugin over `view`.
    pub fn run_annotators(&self, view: &dyn annotate::AnnotationView, since: Option<annotate::RecordCursor>) {
        let notes = self
            .annotations
            .run(&self.annotators, view, since, &|owner| self.runtime.available(owner).is_ok());
        *self.annotate_notes.lock().unwrap_or_else(|p| p.into_inner()) = notes;
    }

    /// Stored annotations of available plugins overlapping the request.
    pub fn annotations(&self, req: &annotate::AnnotationsRequest) -> Vec<annotate::AnnotationDto> {
        self.annotations.query(req, &|owner| self.runtime.available(owner).is_ok())
    }

    /// Forgets every annotation (the workspace was cleared).
    pub fn clear_annotations(&self) {
        self.annotations.clear();
    }

    /// Settings sections of installed plugins (also disabled, inactive and
    /// quarantined ones, which are read-only), with their current status.
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

    /// Lifts a quarantine and forgets a plugin's crashes (and a failed
    /// activation); its dependents become active again.
    pub fn reset_plugin(&self, id: &str) {
        self.runtime.clear_activation_error(id);
        self.runtime.health.reset(id);
    }

    pub fn exporter(&self, id: &str) -> Option<&Arc<dyn Exporter>> {
        self.exporters.iter().find(|e| e.id() == id)
    }
}
