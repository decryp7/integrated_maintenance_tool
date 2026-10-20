//! The Settings screen's backend: every configurable part of IMT described by
//! the typed schema of [`imt_core::settings`], validated in Rust, applied and
//! persisted in one versioned `settings.json` in the config folder.
//!
//! Sections come from built-in adapters (diagnostics, workspace, plugin
//! management), from engine plugins ([`imt_core::Plugin::configurable`]) and
//! from app modules registered later ([`SettingsService::register`], e.g. the
//! AI assistant; until then the `ai` section shows "not installed").
//!
//! `settings.json` is `{ "version": 1, "sections": { "<id>": { "<key>": value } } }`.
//! A missing, unreadable or corrupt file means defaults (a corrupt one is kept
//! as `settings.json.corrupt`); sections nobody provides (e.g. an uninstalled
//! plugin) are kept untouched; writes are atomic (temp file plus rename).
//! Sections whose values already live elsewhere (diagnostics in
//! `diagnostics.json`, enabled plugins in `plugins.json`) keep that file as
//! their single source of truth and are not duplicated here.
//!
//! Commands: `settings_sections`, `set_settings`, `reset_settings`,
//! `export_settings`, `import_settings`.

use crate::windows::Windows;
use crate::{err, lifecycle, AppState, CmdResult};
use imt_core::plugin::PluginStatus;
use imt_core::settings::{sanitize, validate, Configurable, FieldError, PluginSettings, SectionOwner, SettingsSection, ValidValues};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::time::Duration;
use tauri::{AppHandle, Manager, Runtime};

pub use crate::builtin_sections::*;

const FILE: &str = "settings.json";
const VERSION: u32 = 1;
/// Settings files (and imports) larger than this are rejected.
const MAX_FILE_BYTES: u64 = 1 << 20;
const MAX_ID_LEN: usize = 128;
/// Id of the AI assistant's section, registered by the AI module when installed.
pub const AI_SECTION: &str = "ai";
/// Default debounce of live tail (file change bursts are merged within it).
pub const DEFAULT_LIVE_TAIL_DEBOUNCE: Duration = Duration::from_millis(750);

/// Where the Settings screen lists a section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SectionGroup {
    /// Application-wide behaviour (windows, tray, notifications).
    App,
    Diagnostics,
    Plugins,
    Ai,
    Advanced,
}

/// Whether a section can be changed now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SectionStatus {
    Active,
    /// The plugin is turned off.
    Disabled,
    /// The plugin crashed repeatedly.
    Quarantined,
    /// Nothing provides this section in this build.
    NotInstalled,
    /// IMT runs in safe mode; plugins are not loaded.
    SafeMode,
}

/// A section as the UI shows it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SectionView {
    #[serde(flatten)]
    pub section: SettingsSection,
    pub group: SectionGroup,
    /// Current values, every field present.
    pub values: Value,
    pub status: SectionStatus,
    pub read_only: bool,
    /// Why the section is read-only (or another hint).
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsOverview {
    pub sections: Vec<SectionView>,
    /// A change made in this run takes effect after a restart.
    pub restart_pending: bool,
    pub safe_mode: bool,
}

/// Result of `set_settings` / `reset_settings`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetResult {
    /// False when validation failed (see `errors`); nothing was changed then.
    pub applied: bool,
    /// The change takes effect after a restart.
    pub restart_required: bool,
    pub errors: Vec<FieldError>,
    /// The section's values now.
    pub values: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SectionErrors {
    pub section: String,
    pub errors: Vec<FieldError>,
}

/// Result of `import_settings`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    /// Sections validated and applied.
    pub applied: Vec<String>,
    /// Sections kept for later (not installed, disabled or quarantined).
    pub kept: Vec<String>,
    /// Sections rejected, with why; they were left unchanged.
    pub rejected: Vec<SectionErrors>,
    pub restart_required: bool,
}

/// The persisted file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SettingsFile {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub sections: BTreeMap<String, Value>,
}

/// Reads `settings.json`; missing, unreadable, oversized or corrupt files give
/// defaults. A corrupt file is moved aside so the next save does not destroy it.
pub fn load_file(dir: Option<&Path>) -> SettingsFile {
    let Some(path) = dir.map(|d| d.join(FILE)) else {
        return SettingsFile::default();
    };
    match read_settings_file(&path) {
        Ok(Some(file)) => file,
        Ok(None) => SettingsFile::default(),
        Err(e) => {
            log::warn!("ignoring the settings file {}: {e}", path.display());
            let _ = std::fs::rename(&path, path.with_extension("json.corrupt"));
            SettingsFile::default()
        }
    }
}

/// `Ok(None)` when the file does not exist.
fn read_settings_file(path: &Path) -> Result<Option<SettingsFile>, String> {
    use crate::config_store::{read_json, ReadError};
    if std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() > MAX_FILE_BYTES) {
        return Err(format!("the file is larger than {} KB", MAX_FILE_BYTES / 1024));
    }
    let value = match read_json(path) {
        Ok(v) => v,
        Err(ReadError::Missing) => return Ok(None),
        Err(ReadError::Damaged(why)) => return Err(format!("not a settings file: {why}")),
        Err(ReadError::Newer(v)) => return Err(format!("written by a newer IMT (format {v})")),
    };
    if !value.is_object() {
        return Err("not a settings file: expected a JSON object".into());
    }
    let file: SettingsFile = serde_json::from_value(value).map_err(|e| format!("not a settings file: {e}"))?;
    if file.version > VERSION {
        log::warn!(
            "settings file version {} is newer than {VERSION}; values are re-validated",
            file.version
        );
    }
    Ok(Some(file))
}

/// Writes `file` to `path` atomically: a temp file next to it, then a rename.
pub fn write_atomic(path: &Path, file: &SettingsFile) -> anyhow::Result<()> {
    let text = serde_json::to_vec_pretty(&SettingsFile {
        version: VERSION,
        sections: file.sections.clone(),
    })?;
    Ok(crate::config_store::write_atomic(path, &text)?)
}

/// Values that live outside `settings.json` (their owner's file).
pub trait ExternalValues: Send + Sync {
    fn load(&self) -> Value;
}

/// A section provided by the app (not by an engine plugin).
#[derive(Clone)]
pub struct Builtin {
    pub configurable: Arc<dyn Configurable>,
    pub group: SectionGroup,
    /// `None`: the values are stored in `settings.json`.
    pub external: Option<Arc<dyn ExternalValues>>,
}

/// The settings of the engine plugins the running workspace uses (its own
/// registry, so crashes count against the same health record).
pub type PluginSource = Box<dyn Fn() -> Vec<PluginSettings> + Send + Sync>;

/// Everything the service needs; built from the app at startup or by tests.
pub struct SettingsConfig {
    pub dir: Option<PathBuf>,
    pub safe_mode: bool,
    pub builtins: Vec<Builtin>,
    /// The engine plugins (their settings and status).
    pub plugins: PluginSource,
}

/// The settings registry and store. Managed as Tauri state.
pub struct SettingsService {
    dir: Option<PathBuf>,
    safe_mode: bool,
    file: Mutex<SettingsFile>,
    builtins: RwLock<Vec<Builtin>>,
    plugins: PluginSource,
    restart_pending: AtomicBool,
    live_tail_debounce_ms: Arc<AtomicU64>,
}

/// One section, wherever it comes from.
enum Source {
    Builtin(Builtin),
    Plugin(Box<imt_core::settings::PluginSettings>),
}

impl SettingsService {
    /// Loads `settings.json`; call [`SettingsService::start`] to apply it.
    pub fn new(config: SettingsConfig, live_tail_debounce_ms: Arc<AtomicU64>) -> Self {
        SettingsService {
            file: Mutex::new(load_file(config.dir.as_deref())),
            dir: config.dir,
            safe_mode: config.safe_mode,
            builtins: RwLock::new(config.builtins),
            plugins: config.plugins,
            restart_pending: AtomicBool::new(false),
            live_tail_debounce_ms,
        }
    }

    fn file(&self) -> MutexGuard<'_, SettingsFile> {
        crate::sync::lock(&self.file)
    }

    /// Debounce for live tail, as last applied.
    pub fn live_tail_debounce(&self) -> Duration {
        Duration::from_millis(self.live_tail_debounce_ms.load(Ordering::Relaxed))
    }

    /// Applies the stored values of every section kept in `settings.json`.
    /// Invalid stored values fall back to defaults and are logged. Never fails.
    pub fn start(&self) {
        for source in self.sources() {
            if let Some(Builtin { external: Some(_), .. }) = source_builtin(&source) {
                continue; // its owner applied it when loading its own file
            }
            let section = section_of(&source);
            if self.status(&source).0 != SectionStatus::Active {
                continue;
            }
            let stored = self.file().sections.get(&section.id).cloned().unwrap_or(Value::Null);
            let (values, errors) = sanitize(&section, &stored);
            for e in &errors {
                log::warn!(Section = section.id.as_str(), Key = e.key.as_str(); "Invalid stored setting replaced by its default: {}", e.message);
            }
            if let Err(e) = apply(&source, &values) {
                log::warn!(Section = section.id.as_str(); "Could not apply settings: {e:#}");
            }
        }
    }

    /// Adds an app module's section (e.g. the AI assistant) and applies its stored values.
    pub fn register(&self, configurable: Arc<dyn Configurable>, group: SectionGroup) -> anyhow::Result<()> {
        let section = configurable.settings_section();
        section.check().map_err(|e| anyhow::anyhow!("invalid settings schema: {e}"))?;
        if self.sources().iter().any(|s| section_of(s).id == section.id) {
            anyhow::bail!("a settings section \"{}\" is already registered", section.id);
        }
        let builtin = Builtin {
            configurable,
            group,
            external: None,
        };
        let stored = self.file().sections.get(&section.id).cloned().unwrap_or(Value::Null);
        let (values, _) = sanitize(&section, &stored);
        builtin.configurable.apply_settings(&values)?;
        crate::sync::write(&self.builtins).push(builtin);
        Ok(())
    }

    fn sources(&self) -> Vec<Source> {
        let builtins = crate::sync::read(&self.builtins).clone();
        let taken: Vec<String> = builtins.iter().map(|b| b.configurable.settings_section().id).collect();
        let mut out: Vec<Source> = builtins.into_iter().map(Source::Builtin).collect();
        for p in (self.plugins)() {
            if taken.contains(&p.plugin.id) {
                log::warn!(Plugin = p.plugin.id.as_str(); "plugin settings section clashes with a built-in one and is ignored");
                continue;
            }
            out.push(Source::Plugin(Box::new(p)));
        }
        out
    }

    fn find(&self, id: &str) -> Option<Source> {
        self.sources().into_iter().find(|s| section_of(s).id == id)
    }

    fn status(&self, source: &Source) -> (SectionStatus, Option<String>) {
        match source {
            Source::Builtin(b) if self.safe_mode && b.configurable.settings_section().id == PLUGINS_SECTION => (
                SectionStatus::SafeMode,
                Some("IMT is running in safe mode: plugins are not loaded and their settings are not shown.".into()),
            ),
            Source::Builtin(_) => (SectionStatus::Active, None),
            Source::Plugin(p) => {
                let status = match p.plugin.status {
                    PluginStatus::Active => SectionStatus::Active,
                    PluginStatus::Disabled => SectionStatus::Disabled,
                    PluginStatus::Quarantined => SectionStatus::Quarantined,
                    // Installed but not running (a dependency is missing): nothing to configure now.
                    PluginStatus::Inactive => SectionStatus::Disabled,
                };
                (status, p.note().map(String::from))
            }
        }
    }

    /// Current values of a section, validated (invalid ones are defaults).
    fn current(&self, source: &Source, section: &SettingsSection) -> ValidValues {
        let raw = match source_builtin(source).and_then(|b| b.external.as_ref()) {
            Some(external) => external.load(),
            None => self.file().sections.get(&section.id).cloned().unwrap_or(Value::Null),
        };
        sanitize(section, &raw).0
    }

    /// Every section with its values and status.
    pub fn overview(&self) -> SettingsOverview {
        let mut sections: Vec<SectionView> = self
            .sources()
            .iter()
            .map(|source| {
                let section = section_of(source);
                let (status, note) = self.status(source);
                SectionView {
                    values: self.current(source, &section).to_json(),
                    group: match source {
                        Source::Builtin(b) => b.group,
                        Source::Plugin(_) => SectionGroup::Plugins,
                    },
                    read_only: status != SectionStatus::Active,
                    status,
                    note,
                    section,
                }
            })
            .collect();
        if !sections.iter().any(|s| s.section.id == AI_SECTION) {
            sections.push(SectionView {
                section: SettingsSection::new(AI_SECTION, "AI assistant", SectionOwner::App)
                    .describe("Local AI features: summaries, explanations and query suggestions."),
                group: SectionGroup::Ai,
                values: json!({}),
                status: SectionStatus::NotInstalled,
                read_only: true,
                note: Some("The AI assistant is not installed in this build.".into()),
            });
        }
        SettingsOverview {
            sections,
            restart_pending: self.restart_pending.load(Ordering::Relaxed),
            safe_mode: self.safe_mode,
        }
    }

    /// Validates, applies, then persists a section's values.
    pub fn set(&self, id: &str, values: &Value) -> Result<SetResult, String> {
        check_section_id(id)?;
        let source = self.find(id).ok_or_else(|| format!("there is no settings section \"{id}\""))?;
        let (status, note) = self.status(&source);
        if status != SectionStatus::Active {
            return Err(note.unwrap_or_else(|| "This section is read-only.".into()));
        }
        let section = section_of(&source);
        let before = self.current(&source, &section);
        let valid = match validate(&section, values) {
            Ok(v) => v,
            Err(errors) => {
                return Ok(SetResult {
                    applied: false,
                    restart_required: false,
                    errors,
                    values: before.to_json(),
                })
            }
        };
        apply(&source, &valid).map_err(|e| format!("{e:#}"))?;
        if source_builtin(&source).is_none_or(|b| b.external.is_none()) {
            self.persist(|f| {
                f.sections.insert(section.id.clone(), valid.to_json());
            })
            .map_err(|e| format!("the settings were applied but could not be saved: {e:#}"))?;
        }
        let restart_required = section
            .fields
            .iter()
            .any(|f| f.restart_required && before.get(&f.key) != valid.get(&f.key));
        if restart_required {
            self.restart_pending.store(true, Ordering::Relaxed);
        }
        log::info!(Section = id, RestartRequired = restart_required; "Settings changed");
        Ok(SetResult {
            applied: true,
            restart_required,
            errors: Vec::new(),
            values: valid.to_json(),
        })
    }

    /// Puts every field of a section back to its default.
    pub fn reset(&self, id: &str) -> Result<SetResult, String> {
        self.set(id, &json!({}))
    }

    fn persist(&self, change: impl FnOnce(&mut SettingsFile)) -> anyhow::Result<()> {
        let dir = self
            .dir
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("there is no configuration folder"))?;
        let mut file = self.file();
        let mut next = file.clone();
        change(&mut next);
        std::fs::create_dir_all(dir)?;
        write_atomic(&dir.join(FILE), &next)?;
        *file = next;
        Ok(())
    }

    /// Writes every section's current values (and kept ones) to `path`.
    pub fn export(&self, path: &Path) -> anyhow::Result<()> {
        let mut out = self.file().clone();
        for source in self.sources() {
            let section = section_of(&source);
            out.sections.insert(section.id.clone(), self.current(&source, &section).to_json());
        }
        write_atomic(path, &out)
    }

    /// Reads a settings file and applies each section it contains. A section
    /// with any invalid value is rejected whole; sections that cannot be
    /// applied now (not installed, disabled) are kept for later.
    pub fn import(&self, path: &Path) -> Result<ImportResult, String> {
        let imported = read_settings_file(path)?.ok_or("the file does not exist")?;
        let mut result = ImportResult {
            applied: Vec::new(),
            kept: Vec::new(),
            rejected: Vec::new(),
            restart_required: false,
        };
        let mut keep = Map::new();
        for (id, values) in imported.sections {
            if let Err(message) = check_section_id(&id) {
                let section: String = id.chars().take(MAX_ID_LEN).collect();
                result.rejected.push(SectionErrors {
                    section,
                    errors: vec![FieldError {
                        key: String::new(),
                        message,
                    }],
                });
                continue;
            }
            match self.find(&id) {
                Some(source) if self.status(&source).0 == SectionStatus::Active => match self.set(&id, &values) {
                    Ok(r) if r.applied => {
                        result.restart_required |= r.restart_required;
                        result.applied.push(id);
                    }
                    Ok(r) => result.rejected.push(SectionErrors {
                        section: id,
                        errors: r.errors,
                    }),
                    Err(message) => result.rejected.push(SectionErrors {
                        section: id,
                        errors: vec![FieldError {
                            key: String::new(),
                            message,
                        }],
                    }),
                },
                _ => {
                    keep.insert(id.clone(), values);
                    result.kept.push(id);
                }
            }
        }
        if !keep.is_empty() {
            self.persist(|f| f.sections.extend(keep)).map_err(|e| format!("{e:#}"))?;
        }
        Ok(result)
    }
}

fn source_builtin(source: &Source) -> Option<&Builtin> {
    match source {
        Source::Builtin(b) => Some(b),
        Source::Plugin(_) => None,
    }
}

fn section_of(source: &Source) -> SettingsSection {
    match source {
        Source::Builtin(b) => b.configurable.settings_section(),
        Source::Plugin(p) => p.configurable.settings_section(),
    }
}

/// Plugin code runs guarded (see [`imt_core::settings::GuardedConfigurable`]).
fn apply(source: &Source, values: &ValidValues) -> anyhow::Result<()> {
    match source {
        Source::Builtin(b) => b.configurable.apply_settings(values),
        Source::Plugin(p) => p.apply(values),
    }
}

fn check_section_id(id: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > MAX_ID_LEN {
        return Err("invalid settings section id".into());
    }
    Ok(())
}

// ---- Tauri wiring ----

/// Creates the service from the running app and applies the stored settings.
/// Call from `setup` (after `AppState` and `Lifecycle` are managed); commands
/// also create it on first use.
#[allow(dead_code)] // called from `setup` once integrated
pub fn init<R: Runtime>(app: &AppHandle<R>) {
    let _ = service(app);
}

/// Live tail debounce chosen in the settings (default before they are loaded).
#[allow(dead_code)] // used by `set_live` once integrated
pub fn live_tail_debounce<R: Runtime>(app: &AppHandle<R>) -> Duration {
    app.try_state::<SettingsService>()
        .map_or(DEFAULT_LIVE_TAIL_DEBOUNCE, |s| s.live_tail_debounce())
}

/// Adds an app module's settings section (window & tray, the AI assistant, ...)
/// and applies its stored values. Call from `setup` after [`init`].
pub fn register<R: Runtime>(app: &AppHandle<R>, configurable: Arc<dyn Configurable>, group: SectionGroup) -> anyhow::Result<()> {
    service(app).register(configurable, group)
}

fn service<R: Runtime>(app: &AppHandle<R>) -> tauri::State<'_, SettingsService> {
    if app.try_state::<SettingsService>().is_none() {
        let service = build(app);
        if app.manage(service) {
            app.state::<SettingsService>().start();
        }
    }
    app.state::<SettingsService>()
}

fn build<R: Runtime>(app: &AppHandle<R>) -> SettingsService {
    let dir = app.try_state::<AppState>().and_then(|s| s.config_dir.clone());
    let safe_mode = app.try_state::<lifecycle::Lifecycle>().is_some_and(|l| l.safe_mode);
    // Every window has the same plugins (one factory); any window can tell.
    let installed = app.try_state::<Windows>().and_then(|w| w.any()).map_or_else(Vec::new, |state| {
        state
            .store_read()
            .summary()
            .plugins
            .into_iter()
            .map(|p| (p.id, p.name, p.version))
            .collect()
    });
    let handle = app.clone();
    let set_enabled: SetEnabled = Box::new(move |id, enabled| {
        let state = handle
            .try_state::<AppState>()
            .ok_or_else(|| anyhow::anyhow!("the app is not ready"))?;
        crate::plugin_config::set_plugin_enabled(&state, id.to_string(), enabled, None)
            .map(|_| ())
            .map_err(|e| anyhow::anyhow!(e))
    });
    let handle = app.clone();
    let last_known: Mutex<Vec<PluginSettings>> = Mutex::new(Vec::new());
    let plugins: PluginSource = Box::new(move || {
        let Some(state) = handle.try_state::<Windows>().and_then(|w| w.any()) else {
            return Vec::new();
        };
        let mut last = crate::sync::lock(&last_known);
        // Never wait for a long import: while the workspace is busy, the last known list is shown.
        match state.workspace.try_read() {
            Ok(store) => *last = store.plugin_settings(),
            Err(std::sync::TryLockError::Poisoned(p)) => *last = p.into_inner().plugin_settings(),
            Err(std::sync::TryLockError::WouldBlock) => {}
        }
        last.clone()
    });
    let debounce = Arc::new(AtomicU64::new(DEFAULT_LIVE_TAIL_DEBOUNCE.as_millis() as u64));
    let config = SettingsConfig {
        builtins: builtins(dir.clone(), installed, set_enabled, debounce.clone()),
        dir,
        safe_mode,
        plugins,
    };
    SettingsService::new(config, debounce)
}

/// The app's own sections, in navigation order.
pub fn builtins(
    dir: Option<PathBuf>,
    plugins: Vec<(String, String, String)>,
    set_enabled: SetEnabled,
    debounce: Arc<AtomicU64>,
) -> Vec<Builtin> {
    let diagnostics = Arc::new(DiagnosticsSection { dir: dir.clone() });
    let plugins = Arc::new(PluginsSection { dir, plugins, set_enabled });
    vec![
        Builtin {
            configurable: diagnostics.clone(),
            group: SectionGroup::Diagnostics,
            external: Some(diagnostics),
        },
        Builtin {
            configurable: plugins.clone(),
            group: SectionGroup::Plugins,
            external: Some(plugins),
        },
        Builtin {
            configurable: Arc::new(WorkspaceSection { debounce_ms: debounce }),
            group: SectionGroup::Advanced,
            external: None,
        },
    ]
}

#[tauri::command]
pub async fn settings_sections<R: Runtime>(app: AppHandle<R>) -> CmdResult<SettingsOverview> {
    Ok(overview(&app))
}

/// Every section with its values (what `settings_sections` answers), for the shell's own callers.
pub fn overview<R: Runtime>(app: &AppHandle<R>) -> SettingsOverview {
    service(app).overview()
}

#[tauri::command]
pub async fn set_settings<R: Runtime>(
    app: AppHandle<R>,
    op: crate::activity::OpId,
    section: String,
    values: Value,
) -> CmdResult<SetResult> {
    apply_in(&app, &section, values, op.as_deref())
}

/// Validates and applies a section's values (what `set_settings` does), for the shell's own callers.
pub fn apply_section<R: Runtime>(app: &AppHandle<R>, section: &str, values: Value) -> CmdResult<SetResult> {
    apply_in(app, section, values, None)
}

fn apply_in<R: Runtime>(app: &AppHandle<R>, section: &str, values: Value, op: Option<&str>) -> CmdResult<SetResult> {
    let (app, section) = (app.clone(), section.to_string());
    let result = service(&app).set(&section, &values)?;
    if result.applied {
        settings_changed("settings.changed", &section, &values, op);
    }
    Ok(result)
}

#[tauri::command]
pub async fn reset_settings<R: Runtime>(app: AppHandle<R>, op: crate::activity::OpId, section: String) -> CmdResult<SetResult> {
    let result = service(&app).reset(&section)?;
    if result.applied {
        settings_changed("settings.reset", &section, &Value::Null, op.as_deref());
    }
    Ok(result)
}

/// Most changed keys named in one `settings.changed` event.
const MAX_LOGGED_KEYS: usize = 12;

/// The `settings.changed` domain event: the section and the names of the changed keys, never their values
/// (a value may be a secret or a path).
fn settings_changed(event: &str, section: &str, values: &Value, op: Option<&str>) {
    let keys: Vec<&str> = values
        .as_object()
        .map(|o| o.keys().map(String::as_str).take(MAX_LOGGED_KEYS).collect())
        .unwrap_or_default();
    let keys = keys.join(",");
    let section: String = section.chars().take(MAX_ID_LEN).collect();
    crate::activity::domain_event(event, op, &[("Section", &section), ("Fields", &keys)]);
}

#[tauri::command]
pub async fn export_settings<R: Runtime>(
    app: AppHandle<R>,
    grants: crate::paths::WindowGrants,
    path: String,
    overwrite: Option<bool>,
) -> CmdResult<()> {
    let target = grants.write(&path, &["json"])?.path().to_path_buf();
    imt_core::overwrite::check_overwrite(Path::new(&path), overwrite.unwrap_or(false))?;
    let path = target;
    service(&app).export(&path).map_err(err)?;
    log::info!("Settings exported");
    Ok(())
}

#[tauri::command]
pub async fn import_settings<R: Runtime>(app: AppHandle<R>, grants: crate::paths::WindowGrants, path: String) -> CmdResult<ImportResult> {
    let path = grants.read_file(&path, &["json"])?.path().to_path_buf();
    let result = service(&app).import(&path)?;
    log::info!(Applied = result.applied.len() as u64, Rejected = result.rejected.len() as u64; "Settings imported");
    let (applied, rejected) = (result.applied.join(","), result.rejected.len().to_string());
    crate::activity::domain_event("settings.imported", None, &[("Sections", &applied), ("Rejected", &rejected)]);
    Ok(result)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::diagnostics;
    use crate::test_support::Now as _;
    use imt_core::guard::QUARANTINE_AFTER;
    use imt_core::settings::{FieldKind, SettingField};
    use imt_core::{Plugin, PluginRegistry};
    use std::sync::atomic::AtomicUsize;

    struct Temp(PathBuf);
    impl Temp {
        fn new(tag: &str) -> Temp {
            let d = std::env::temp_dir().join(format!("imt-settings-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            Temp(d)
        }
        fn file(&self) -> Value {
            serde_json::from_str(&std::fs::read_to_string(self.0.join(FILE)).unwrap()).unwrap()
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A plugin whose `apply_settings` panics on demand (value 13).
    struct Fragile;
    struct FragileSettings;
    impl Configurable for FragileSettings {
        fn settings_section(&self) -> SettingsSection {
            SettingsSection::new("x", "Fragile", SectionOwner::Core).field(SettingField::new("n", "N", int(0, 20, None), 1))
        }
        fn apply_settings(&self, values: &ValidValues) -> anyhow::Result<()> {
            if values.int("n") == Some(13) {
                panic!("unlucky");
            }
            Ok(())
        }
    }
    impl Plugin for Fragile {
        fn id(&self) -> &'static str {
            "t.fragile"
        }
        fn name(&self) -> &'static str {
            "Fragile"
        }
        fn version(&self) -> &'static str {
            "1"
        }
        fn register(&self, _r: &mut PluginRegistry) -> anyhow::Result<()> {
            Ok(())
        }
        fn configurable(&self) -> Option<Arc<dyn Configurable>> {
            Some(Arc::new(FragileSettings))
        }
    }

    struct Fixture {
        service: SettingsService,
        debounce: Arc<AtomicU64>,
        /// The workspace whose registry the plugin sections come from.
        workspace: Arc<imt_core::Workspace>,
        enabled_calls: Arc<Mutex<Vec<(String, bool)>>>,
    }

    /// Tests that apply the Windows plugin's process-wide settings run one at a time.
    pub(crate) fn serial() -> MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        crate::sync::lock(&LOCK)
    }

    fn fixture(dir: Option<&Path>, safe_mode: bool, disabled: &[&str]) -> Fixture {
        if let Some(d) = dir {
            let s = crate::plugin_config::PluginSettings {
                disabled: disabled.iter().map(|s| s.to_string()).collect(),
            };
            std::fs::write(d.join("plugins.json"), serde_json::to_string(&s).unwrap()).unwrap();
        }
        let mut plugins = PluginRegistry::empty();
        plugins.disable(disabled.iter().map(|s| s.to_string()));
        if !safe_mode {
            plugins.install(&imt_plugin_windows::WindowsPlugin).unwrap();
            plugins.install(&imt_plugin_web::WebPlugin).unwrap();
            plugins.install(&Fragile).unwrap();
        }
        let installed = plugins.plugins().into_iter().map(|p| (p.id, p.name, p.version)).collect();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let recorded = calls.clone();
        let plugins_dir = dir.map(Path::to_path_buf);
        let set_enabled: SetEnabled = Box::new(move |id, enabled| {
            recorded.lock().unwrap().push((id.to_string(), enabled));
            let dir = plugins_dir.as_ref().ok_or_else(|| anyhow::anyhow!("no dir"))?;
            let mut s = crate::plugin_config::load_plugin_settings(Some(dir));
            s.disabled.retain(|d| d != id);
            if !enabled {
                s.disabled.push(id.into());
            }
            std::fs::write(dir.join("plugins.json"), serde_json::to_string(&s)?)?;
            Ok(())
        });
        let workspace = Arc::new(imt_core::Workspace::with_registry(imt_core::Profile::default_profile(), plugins).unwrap());
        let ws = workspace.clone();
        let debounce = Arc::new(AtomicU64::new(DEFAULT_LIVE_TAIL_DEBOUNCE.as_millis() as u64));
        let config = SettingsConfig {
            dir: dir.map(Path::to_path_buf),
            safe_mode,
            builtins: builtins(dir.map(Path::to_path_buf), installed, set_enabled, debounce.clone()),
            plugins: Box::new(move || ws.plugin_settings()),
        };
        Fixture {
            service: SettingsService::new(config, debounce.clone()),
            debounce,
            workspace,
            enabled_calls: calls,
        }
    }

    fn view<'a>(o: &'a SettingsOverview, id: &str) -> &'a SectionView {
        o.sections.iter().find(|s| s.section.id == id).unwrap()
    }

    #[test]
    fn file_round_trip_and_tolerance() {
        let t = Temp::new("file");
        assert_eq!(load_file(None), SettingsFile::default());
        assert_eq!(load_file(Some(&t.0)), SettingsFile::default());
        let mut f = SettingsFile::default();
        f.sections.insert("a".into(), json!({ "k": 1 }));
        write_atomic(&t.0.join(FILE), &f).unwrap();
        let back = load_file(Some(&t.0));
        assert_eq!((back.version, back.sections), (VERSION, f.sections.clone()));
        // No temp files are left behind.
        let names: Vec<String> = std::fs::read_dir(&t.0)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, [FILE]);
        // A newer version is still read (values are re-validated anyway).
        std::fs::write(t.0.join(FILE), r#"{"version": 99, "sections": {"b": {}}}"#).unwrap();
        assert!(load_file(Some(&t.0)).sections.contains_key("b"));
        // Corrupt: defaults, and the file is moved aside.
        std::fs::write(t.0.join(FILE), "{ nope").unwrap();
        assert_eq!(load_file(Some(&t.0)), SettingsFile::default());
        assert!(t.0.join("settings.json.corrupt").exists() && !t.0.join(FILE).exists());
        // Oversized.
        std::fs::write(t.0.join(FILE), " ".repeat(MAX_FILE_BYTES as usize + 1)).unwrap();
        assert_eq!(load_file(Some(&t.0)), SettingsFile::default());
        // A folder where the file should be.
        std::fs::create_dir_all(t.0.join(FILE)).unwrap();
        assert_eq!(load_file(Some(&t.0)), SettingsFile::default());
        // A missing folder is created; a folder in the way fails cleanly.
        assert!(write_atomic(&t.0.join("missing/settings.json"), &f).is_ok());
        assert!(write_atomic(Path::new("/"), &f).is_err());
    }

    #[test]
    fn overview_lists_builtins_plugins_and_the_ai_placeholder() {
        let _serial = serial();
        let t = Temp::new("overview");
        let fx = fixture(Some(&t.0), false, &["net.decryptology.web", "t.fragile"]);
        let o = fx.service.overview();
        let ids: Vec<&str> = o.sections.iter().map(|s| s.section.id.as_str()).collect();
        assert_eq!(
            ids,
            ["diagnostics", "plugins", "workspace", "net.decryptology.windows", "t.fragile", "ai"]
        );
        assert!(!o.restart_pending && !o.safe_mode);
        let diag = view(&o, "diagnostics");
        assert_eq!(
            (diag.group, diag.status, diag.read_only),
            (SectionGroup::Diagnostics, SectionStatus::Active, false)
        );
        assert_eq!(diag.values["logLevel"], "info");
        assert_eq!(diag.values["logMaxFileMb"], 10);
        let plugins = view(&o, "plugins");
        assert_eq!(plugins.values["net.decryptology.windows"], true);
        assert_eq!(plugins.values["net.decryptology.web"], false);
        assert_eq!(plugins.section.fields.len(), 3);
        let windows = view(&o, "net.decryptology.windows");
        assert_eq!((windows.group, windows.status), (SectionGroup::Plugins, SectionStatus::Active));
        assert_eq!(windows.values["relogTimeout"], 120_000);
        let fragile = view(&o, "t.fragile");
        assert_eq!((fragile.status, fragile.read_only), (SectionStatus::Disabled, true));
        assert!(fragile.note.as_deref().unwrap().contains("disabled"));
        assert_eq!(fragile.values["n"], 1);
        let ai = view(&o, AI_SECTION);
        assert_eq!(
            (ai.status, ai.read_only, ai.group),
            (SectionStatus::NotInstalled, true, SectionGroup::Ai)
        );
        assert!(ai.note.as_deref().unwrap().contains("not installed"));
        // The wire shape: the section's fields are flattened next to the status.
        let v = serde_json::to_value(&o).unwrap();
        assert_eq!(v["sections"][0]["id"], "diagnostics");
        assert_eq!(v["sections"][0]["readOnly"], false);
        assert_eq!(v["sections"][0]["fields"][0]["kind"]["type"], "enum");
        assert_eq!(v["restartPending"], false);
    }

    #[test]
    fn set_validates_applies_then_persists() {
        let _serial = serial();
        let t = Temp::new("set");
        let fx = fixture(Some(&t.0), false, &[]);
        std::fs::write(t.0.join(FILE), r#"{"version":1,"sections":{"uninstalled.plugin":{"x":1}}}"#).unwrap();
        let fx = Fixture {
            service: SettingsService::new(
                SettingsConfig {
                    dir: Some(t.0.clone()),
                    safe_mode: false,
                    builtins: fx.service.builtins.read().unwrap().clone(),
                    plugins: Box::new(Vec::new),
                },
                fx.debounce.clone(),
            ),
            ..fx
        };
        let bad = fx.service.set(WORKSPACE_SECTION, &json!({ "liveTailDebounce": 5 })).unwrap();
        assert!(!bad.applied && !bad.restart_required);
        assert_eq!(bad.errors[0].key, "liveTailDebounce");
        assert_eq!(bad.errors[0].message, "Must be between 100 ms and 10 s.");
        assert_eq!(bad.values["liveTailDebounce"], 750);
        assert_eq!(fx.debounce.load(Ordering::SeqCst), 750);

        let ok = fx
            .service
            .set(WORKSPACE_SECTION, &json!({ "liveTailDebounce": 2000, "junk": 1 }))
            .unwrap();
        assert!(ok.applied && !ok.restart_required && ok.errors.is_empty());
        assert_eq!(ok.values, json!({ "liveTailDebounce": 2000 }));
        assert_eq!(fx.service.live_tail_debounce(), Duration::from_secs(2));
        let file = t.file();
        assert_eq!(file["version"], VERSION);
        assert_eq!(file["sections"]["workspace"], json!({ "liveTailDebounce": 2000 }));
        // Sections nobody provides are kept.
        assert_eq!(file["sections"]["uninstalled.plugin"], json!({ "x": 1 }));

        let reset = fx.service.reset(WORKSPACE_SECTION).unwrap();
        assert_eq!(reset.values["liveTailDebounce"], 750);
        assert_eq!(fx.service.live_tail_debounce(), DEFAULT_LIVE_TAIL_DEBOUNCE);

        assert!(fx.service.set("nope", &json!({})).unwrap_err().contains("no settings section"));
        assert!(fx.service.set("", &json!({})).unwrap_err().contains("invalid"));
        assert!(fx.service.set(&"x".repeat(200), &json!({})).unwrap_err().contains("invalid"));
    }

    #[test]
    fn diagnostics_adapter_writes_diagnostics_json_and_flags_restarts() {
        let _serial = serial();
        let t = Temp::new("diag");
        let fx = fixture(Some(&t.0), false, &[]);
        let r = fx.service.set(DIAGNOSTICS_SECTION, &json!({ "logLevel": "debug" })).unwrap();
        assert!(r.applied && !r.restart_required);
        assert_eq!(diagnostics::load(Some(&t.0)).log.level, diagnostics::LogLevel::Debug);
        assert!(!fx.service.overview().restart_pending);
        let r = fx
            .service
            .set(
                DIAGNOSTICS_SECTION,
                &json!({ "logLevel": "debug", "logMaxFileMb": 20, "snapshotsEnabled": false }),
            )
            .unwrap();
        assert!(r.applied && r.restart_required);
        let d = diagnostics::load(Some(&t.0));
        assert_eq!((d.log.max_file_mb, d.snapshots.enabled), (20, false));
        assert!(fx.service.overview().restart_pending);
        // Not duplicated in settings.json: diagnostics.json is the source of truth.
        assert!(!t.0.join(FILE).exists());
        assert_eq!(view(&fx.service.overview(), "diagnostics").values["logMaxFileMb"], 20);
        let e = fx
            .service
            .set(DIAGNOSTICS_SECTION, &json!({ "logKeepFiles": 0, "logLevel": "loud" }))
            .unwrap();
        let keys: Vec<&str> = e.errors.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(keys, ["logLevel", "logKeepFiles"]);
        log::set_max_level(log::LevelFilter::Info);
    }

    #[test]
    fn diagnostics_limits_match_the_clamping_in_diagnostics_rs() {
        let section = DiagnosticsSection { dir: None }.settings_section();
        for f in &section.fields {
            if let FieldKind::Int { min, max, .. } = f.kind {
                for v in [min, max] {
                    let mut values = section.defaults().to_json();
                    values[&f.key] = json!(v);
                    let valid = validate(&section, &values).unwrap();
                    let s = diagnostics_from(&valid).unwrap();
                    let back = sanitize(&section, &DiagnosticsSection { dir: None }.load()).0;
                    assert_eq!(back, section.defaults(), "defaults agree");
                    let as_json = json!({
                        "logMaxFileMb": s.log.max_file_mb, "logKeepFiles": s.log.keep_files, "logMaxAgeDays": s.log.max_age_days,
                        "snapshotsMaxCount": s.snapshots.max_count, "snapshotsMaxTotalMb": s.snapshots.max_total_mb,
                        "snapshotsMaxAgeDays": s.snapshots.max_age_days,
                    });
                    assert_eq!(as_json[&f.key], json!(v), "{} = {v} survives clamped()", f.key);
                }
            }
        }
        assert!(DiagnosticsSection { dir: None }
            .apply_settings(&section.defaults())
            .unwrap_err()
            .to_string()
            .contains("configuration folder"));
        assert_eq!(capitalized(""), "");
    }

    #[test]
    fn plugin_switches_go_through_the_lifecycle_setting() {
        let _serial = serial();
        let t = Temp::new("switch");
        let fx = fixture(Some(&t.0), false, &[]);
        let r = fx.service.set(PLUGINS_SECTION, &json!({ "net.decryptology.web": false })).unwrap();
        assert!(r.applied && r.restart_required);
        assert_eq!(*fx.enabled_calls.lock().unwrap(), [("net.decryptology.web".to_string(), false)]);
        assert_eq!(
            crate::plugin_config::load_plugin_settings(Some(&t.0)).disabled,
            ["net.decryptology.web"]
        );
        assert_eq!(view(&fx.service.overview(), PLUGINS_SECTION).values["net.decryptology.web"], false);
        // Unchanged switches are not touched again.
        fx.service.set(PLUGINS_SECTION, &json!({ "net.decryptology.web": false })).unwrap();
        assert_eq!(fx.enabled_calls.lock().unwrap().len(), 1);
        fx.service.reset(PLUGINS_SECTION).unwrap();
        assert!(crate::plugin_config::load_plugin_settings(Some(&t.0)).disabled.is_empty());
        // A failing switch is reported.
        let none = fixture(None, false, &[]);
        assert!(none
            .service
            .set(PLUGINS_SECTION, &json!({ "t.fragile": false }))
            .unwrap_err()
            .contains("no dir"));
    }

    #[test]
    fn plugin_sections_apply_and_crashes_are_contained_and_quarantine() {
        let _serial = serial();
        let t = Temp::new("crash");
        let fx = fixture(Some(&t.0), false, &[]);
        let r = fx
            .service
            .set("net.decryptology.windows", &json!({ "relogTimeout": 300_000 }))
            .unwrap();
        assert!(r.applied);
        assert_eq!(imt_plugin_windows::settings::relog_timeout(), Duration::from_secs(300));
        fx.service.reset("net.decryptology.windows").unwrap();
        assert_eq!(t.file()["sections"]["net.decryptology.windows"]["relogTimeout"], 120_000);

        for _ in 0..QUARANTINE_AFTER {
            let e = fx.service.set("t.fragile", &json!({ "n": 13 })).unwrap_err();
            assert!(e.contains("crashed: unlucky"), "{e}");
        }
        // Nothing crashed was saved.
        assert!(t.file()["sections"].get("t.fragile").is_none());
        let o = fx.service.overview();
        let fragile = view(&o, "t.fragile");
        assert_eq!((fragile.status, fragile.read_only), (SectionStatus::Quarantined, true));
        assert!(fx
            .service
            .set("t.fragile", &json!({ "n": 1 }))
            .unwrap_err()
            .contains("repeated crashes"));

        // The crashes were counted by the workspace's own registry (one health record).
        let info = fx.workspace.plugins().into_iter().find(|p| p.id == "t.fragile").unwrap();
        assert_eq!((info.status, info.failures), (PluginStatus::Quarantined, QUARANTINE_AFTER));
        // Lifting the quarantine in the workspace makes the section editable again.
        fx.workspace.reset_plugin("t.fragile");
        assert_eq!(view(&fx.service.overview(), "t.fragile").status, SectionStatus::Active);
        assert!(fx.service.set("t.fragile", &json!({ "n": 1 })).unwrap().applied);
    }

    #[test]
    fn disabled_plugins_are_read_only() {
        let _serial = serial();
        let t = Temp::new("disabled");
        let fx = fixture(Some(&t.0), false, &["net.decryptology.windows"]);
        let o = fx.service.overview();
        let w = view(&o, "net.decryptology.windows");
        assert_eq!((w.status, w.read_only), (SectionStatus::Disabled, true));
        assert_eq!(w.section.fields.len(), 1);
        assert!(fx
            .service
            .set("net.decryptology.windows", &json!({}))
            .unwrap_err()
            .contains("disabled"));
    }

    #[test]
    fn startup_applies_stored_values_and_defaults_invalid_ones() {
        let _serial = serial();
        let t = Temp::new("start");
        std::fs::write(
            t.0.join(FILE),
            r#"{"version":1,"sections":{"workspace":{"liveTailDebounce":1500},"t.fragile":{"n":"bad"},"net.decryptology.windows":{"relogTimeout":60000}}}"#,
        )
        .unwrap();
        let fx = fixture(Some(&t.0), false, &[]);
        fx.service.start();
        assert_eq!(fx.service.live_tail_debounce(), Duration::from_millis(1500));
        assert_eq!(imt_plugin_windows::settings::relog_timeout(), Duration::from_secs(60));
        assert_eq!(view(&fx.service.overview(), "t.fragile").values["n"], 1);
        fx.service.reset("net.decryptology.windows").unwrap();
        // A crash at startup is contained too.
        std::fs::write(
            t.0.join(FILE),
            r#"{"version":1,"sections":{"t.fragile":{"n":13},"workspace":{"liveTailDebounce":3}}}"#,
        )
        .unwrap();
        let fx = fixture(Some(&t.0), false, &["net.decryptology.web"]);
        fx.service.start();
        assert_eq!(fx.service.live_tail_debounce(), DEFAULT_LIVE_TAIL_DEBOUNCE);
        // A disabled plugin's stored values are not applied.
        let fx = fixture(Some(&t.0), false, &["t.fragile"]);
        fx.service.start();
        assert_eq!(view(&fx.service.overview(), "t.fragile").status, SectionStatus::Disabled);
    }

    #[test]
    fn safe_mode_shows_no_plugin_sections() {
        let _serial = serial();
        let t = Temp::new("safe");
        let fx = fixture(Some(&t.0), true, &[]);
        let o = fx.service.overview();
        assert!(o.safe_mode);
        let p = view(&o, PLUGINS_SECTION);
        assert_eq!((p.status, p.read_only), (SectionStatus::SafeMode, true));
        assert!(p.note.as_deref().unwrap().contains("safe mode"));
        assert!(fx.service.set(PLUGINS_SECTION, &json!({})).unwrap_err().contains("safe mode"));
        assert_eq!(o.sections.len(), 4);
    }

    #[test]
    fn without_a_config_folder_changes_apply_but_report_the_save_failure() {
        let _serial = serial();
        let fx = fixture(None, false, &[]);
        let e = fx.service.set(WORKSPACE_SECTION, &json!({ "liveTailDebounce": 900 })).unwrap_err();
        assert!(e.contains("could not be saved") && e.contains("no configuration folder"), "{e}");
        assert_eq!(fx.debounce.load(Ordering::SeqCst), 900);
    }

    #[test]
    fn export_and_import() {
        let _serial = serial();
        let t = Temp::new("export");
        let fx = fixture(Some(&t.0), false, &["net.decryptology.web"]);
        fx.service.set(WORKSPACE_SECTION, &json!({ "liveTailDebounce": 1200 })).unwrap();
        let out = t.0.join("exported.json");
        fx.service.export(&out).unwrap();
        let exported: Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
        assert_eq!(exported["version"], VERSION);
        assert_eq!(exported["sections"]["workspace"]["liveTailDebounce"], 1200);
        assert_eq!(exported["sections"]["diagnostics"]["logLevel"], "info");
        assert_eq!(exported["sections"]["plugins"]["net.decryptology.web"], false);

        fx.service.reset(WORKSPACE_SECTION).unwrap();
        let input = t.0.join("in.json");
        std::fs::write(
            &input,
            json!({
                "version": 1,
                "sections": {
                    "workspace": { "liveTailDebounce": 1300 },
                    "diagnostics": { "logMaxFileMb": 0 },
                    "net.decryptology.windows": { "relogTimeout": 240000 },
                    "net.decryptology.web": { "anything": true },
                    "some.future.plugin": { "a": 1 },
                    "": {}
                }
            })
            .to_string(),
        )
        .unwrap();
        let r = fx.service.import(&input).unwrap();
        assert_eq!(r.applied, ["net.decryptology.windows", "workspace"]);
        assert_eq!(r.kept, ["net.decryptology.web", "some.future.plugin"]);
        let rejected: Vec<&str> = r.rejected.iter().map(|s| s.section.as_str()).collect();
        assert_eq!(rejected, ["", "diagnostics"]);
        assert_eq!(r.rejected[1].errors[0].key, "logMaxFileMb");
        assert!(!r.restart_required);
        assert_eq!(fx.service.live_tail_debounce(), Duration::from_millis(1300));
        assert_eq!(
            diagnostics::load(Some(&t.0)).log.max_file_mb,
            10,
            "a rejected section is left unchanged"
        );
        let file = t.file();
        assert_eq!(file["sections"]["some.future.plugin"], json!({ "a": 1 }));
        fx.service.reset("net.decryptology.windows").unwrap();

        std::fs::write(&input, "not json").unwrap();
        assert!(fx.service.import(&input).unwrap_err().contains("not a settings file"));
        assert!(fx.service.import(&t.0.join("missing.json")).unwrap_err().contains("does not exist"));
        // A setter error (here: no dir for the plugin switch) rejects the section.
        let none = fixture(None, false, &[]);
        std::fs::write(&input, r#"{"sections":{"plugins":{"t.fragile":false}}}"#).unwrap();
        let r = none.service.import(&input).unwrap();
        assert_eq!(r.rejected[0].section, "plugins");
    }

    struct AiSettings(Arc<AtomicUsize>);
    impl Configurable for AiSettings {
        fn settings_section(&self) -> SettingsSection {
            SettingsSection::new(AI_SECTION, "AI assistant", SectionOwner::App).field(SettingField::new("on", "On", FieldKind::Bool, false))
        }
        fn apply_settings(&self, _values: &ValidValues) -> anyhow::Result<()> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[test]
    fn app_modules_register_later() {
        let _serial = serial();
        let t = Temp::new("register");
        std::fs::write(t.0.join(FILE), r#"{"version":1,"sections":{"ai":{"on":true}}}"#).unwrap();
        let fx = fixture(Some(&t.0), false, &[]);
        let applied = Arc::new(AtomicUsize::new(0));
        fx.service
            .register(Arc::new(AiSettings(applied.clone())), SectionGroup::Ai)
            .unwrap();
        assert_eq!(applied.load(Ordering::SeqCst), 1);
        let o = fx.service.overview();
        let ai = view(&o, AI_SECTION);
        assert_eq!((ai.status, ai.values["on"].clone()), (SectionStatus::Active, json!(true)));
        assert_eq!(o.sections.iter().filter(|s| s.section.id == AI_SECTION).count(), 1);
        let again = fx
            .service
            .register(Arc::new(AiSettings(applied.clone())), SectionGroup::Ai)
            .unwrap_err();
        assert!(again.to_string().contains("already registered"));
        struct Broken;
        impl Configurable for Broken {
            fn settings_section(&self) -> SettingsSection {
                SettingsSection::new("bad id", "B", SectionOwner::App)
            }
            fn apply_settings(&self, _values: &ValidValues) -> anyhow::Result<()> {
                Ok(())
            }
        }
        assert!(fx
            .service
            .register(Arc::new(Broken), SectionGroup::Advanced)
            .unwrap_err()
            .to_string()
            .contains("invalid settings schema"));
    }

    #[test]
    fn a_plugin_section_clashing_with_a_builtin_is_ignored() {
        struct Clash;
        impl Plugin for Clash {
            fn id(&self) -> &'static str {
                "workspace"
            }
            fn name(&self) -> &'static str {
                "Clash"
            }
            fn version(&self) -> &'static str {
                "1"
            }
            fn register(&self, _r: &mut PluginRegistry) -> anyhow::Result<()> {
                Ok(())
            }
            fn configurable(&self) -> Option<Arc<dyn Configurable>> {
                Some(Arc::new(FragileSettings))
            }
        }
        let mut plugins = PluginRegistry::empty();
        plugins.install(&Clash).unwrap();
        let clash = imt_core::Workspace::with_registry(imt_core::Profile::default_profile(), plugins).unwrap();
        let debounce = Arc::new(AtomicU64::new(750));
        let service = SettingsService::new(
            SettingsConfig {
                dir: None,
                safe_mode: false,
                builtins: builtins(None, vec![], Box::new(|_, _| Ok(())), debounce.clone()),
                plugins: Box::new(move || clash.plugin_settings()),
            },
            debounce,
        );
        let o = service.overview();
        assert_eq!(o.sections.iter().filter(|s| s.section.id == "workspace").count(), 1);
        assert_eq!(view(&o, "workspace").section.title, "Workspace");
    }

    #[test]
    fn commands_through_the_tauri_app() {
        let _serial = serial();
        let _log = crate::activity::test_capture();
        let t = Temp::new("commands");
        let app = crate::test_support::test_app(Some(t.0.join("profiles")));
        app.manage(lifecycle::Lifecycle::begin(Some(t.0.join("data")), false));
        let h = app.handle().clone();
        assert_eq!(live_tail_debounce(&h), DEFAULT_LIVE_TAIL_DEBOUNCE);
        init(&h);
        let o = settings_sections(h.clone()).now().unwrap();
        assert!(o.sections.iter().any(|s| s.section.id == "net.decryptology.windows"));
        assert!(!o.safe_mode);
        // The plugin switch goes through `crate::plugin_config::plugin_set_enabled`.
        let r = set_settings(
            h.clone(),
            crate::activity::OpId::default(),
            PLUGINS_SECTION.into(),
            json!({ "net.decryptology.web": false }),
        )
        .now()
        .unwrap();
        assert!(r.applied && r.restart_required);
        assert_eq!(
            crate::plugin_config::load_plugin_settings(Some(&t.0)).disabled,
            ["net.decryptology.web"]
        );
        set_settings(
            h.clone(),
            crate::activity::OpId::default(),
            WORKSPACE_SECTION.into(),
            json!({ "liveTailDebounce": 400 }),
        )
        .now()
        .unwrap();
        assert_eq!(live_tail_debounce(&h), Duration::from_millis(400));
        // A domain event names the section and the changed fields, never their values.
        let changed = crate::tools::tests::domain_events();
        let e = changed
            .iter()
            .find(|c| c.props.get("Section").is_some_and(|s| s == WORKSPACE_SECTION))
            .expect("settings.changed");
        assert_eq!(
            (e.props["Operation"].as_str(), e.props["Fields"].as_str()),
            ("settings.changed", "liveTailDebounce")
        );
        assert!(!format!("{changed:?}").contains("400"));
        let out = t.0.join("export.json").to_string_lossy().into_owned();
        let grants = || crate::paths::WindowGrants::of(&h, "main");
        let refused = export_settings(h.clone(), grants(), out.clone(), None).now().unwrap_err();
        assert!(refused.contains("only writes where you choose"), "{refused}");
        crate::paths::allow_under(&h, "main", &t.0);
        export_settings(h.clone(), grants(), out.clone(), None).now().unwrap();
        assert_eq!(
            export_settings(h.clone(), grants(), out.clone(), None).now().unwrap_err(),
            crate::error::file_exists(format!("{out} already exists; confirm to replace it"))
        );
        export_settings(h.clone(), grants(), out.clone(), Some(true)).now().unwrap();
        reset_settings(h.clone(), crate::activity::OpId::default(), WORKSPACE_SECTION.into())
            .now()
            .unwrap();
        let r = import_settings(h.clone(), grants(), out).now().unwrap();
        assert!(r.applied.contains(&WORKSPACE_SECTION.to_string()));
        let events = crate::tools::tests::domain_events();
        assert!(events
            .iter()
            .any(|c| c.props["Operation"] == "settings.reset" && c.props.get("Section").is_some_and(|s| s == WORKSPACE_SECTION)));
        assert!(events.iter().any(|c| c.props["Operation"] == "settings.imported"));
        assert_eq!(live_tail_debounce(&h), Duration::from_millis(400));
        assert!(export_settings(h.clone(), grants(), "relative.json".into(), None).now().is_err());
        assert!(
            import_settings(h.clone(), grants(), t.0.join("none.json").to_string_lossy().into_owned())
                .now()
                .is_err()
        );
        let bad = t.0.join("bad.json");
        std::fs::write(&bad, "[]").unwrap();
        assert!(import_settings(h.clone(), grants(), bad.to_string_lossy().into_owned())
            .now()
            .is_err());
        // While an import holds the workspace, the status is read without waiting for it.
        {
            let state = crate::test_support::main_window(&app);
            let _busy = state.workspace.write().unwrap();
            assert!(!settings_sections(h.clone()).now().unwrap().sections.is_empty());
        }
        // The plugin sections come from the workspace's registry (nothing is quarantined here).
        let web = settings_sections(h.clone()).now().unwrap();
        assert_eq!(view(&web, "net.decryptology.windows").status, SectionStatus::Active);
        let failing = export_settings(
            h.clone(),
            grants(),
            t.0.join("missing/dir/x.json").to_string_lossy().into_owned(),
            None,
        )
        .now();
        assert!(failing.is_err());
    }

    #[test]
    fn the_service_is_created_lazily_without_app_state() {
        let _serial = serial();
        use tauri::test::{mock_builder, mock_context, noop_assets};
        let app = mock_builder().build(mock_context(noop_assets())).unwrap();
        let h = app.handle().clone();
        let o = settings_sections(h.clone()).now().unwrap();
        assert!(o.sections.iter().any(|s| s.section.id == DIAGNOSTICS_SECTION));
        // Without a workspace there are no plugins: no plugin sections and nothing to switch.
        assert!(!o
            .sections
            .iter()
            .any(|s| s.group == SectionGroup::Plugins && s.section.id != PLUGINS_SECTION));
        assert!(view(&o, PLUGINS_SECTION).section.fields.is_empty());
        let r = set_settings(
            h.clone(),
            crate::activity::OpId::default(),
            PLUGINS_SECTION.into(),
            json!({ "net.decryptology.web": false }),
        )
        .now()
        .unwrap();
        assert!(!r.restart_required);
    }
}
