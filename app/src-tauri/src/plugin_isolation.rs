//! Running plugins out of process (ADR 0002): the "Plugin isolation" settings section, the session's
//! [`Isolation`] (one plugin process per isolated plugin, shared by every window), isolation after a
//! quarantine, and the `plugin_isolation_status` command.
//!
//! Settings (`settings.json`, section `plugin-isolation`; they take effect after a restart):
//! - `isolatedMode`: every plugin that can runs out of process (also the `--isolated` flag);
//! - `autoIsolate`: a plugin quarantined in process runs isolated from the next start;
//! - `isolate.<plugin id>`: "Run isolated" per plugin;
//! - `grant.<plugin id>.<permission>`: permissions a plugin asks for (shown, and enforced: a plugin whose
//!   permission is revoked is inactive with the reason; without "read files you open" it only ever
//!   receives the bytes IMT read, never a path).

use imt_core::settings::{Configurable, FieldKind, SectionOwner, SettingField, SettingsSection, ValidValues};
use imt_core::Plugin;
use imt_plugin_host::builtins::{self, Isolable};
use imt_plugin_host::isolation::{Isolation, IsolationSettings, Mode, ISOLATED_FLAG};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

pub const SECTION: &str = "plugin-isolation";

/// Settings values (as stored) to isolation settings.
pub fn from_values(values: &Value) -> IsolationSettings {
    let mut s = IsolationSettings::default();
    let Some(map) = values.as_object() else { return s };
    if let Some(v) = map.get("isolatedMode").and_then(Value::as_bool) {
        s.isolated_mode = v;
    }
    if let Some(v) = map.get("autoIsolate").and_then(Value::as_bool) {
        s.auto_isolate = v;
    }
    for p in builtins::all() {
        if map.get(&format!("isolate.{}", p.id())).and_then(Value::as_bool) == Some(true) {
            s.set_isolated(p.id(), true);
        }
        for req in builtins::serve_options(p.id()).permissions {
            let key = format!("grant.{}.{}", p.id(), req.permission.key());
            if map.get(&key).and_then(Value::as_bool) == Some(false) {
                s.set_builtin_grant(p.id(), &req.permission, false);
            }
        }
    }
    s
}

/// Isolation settings as stored in `settings.json` (defaults when absent or unreadable).
pub fn load(config_dir: Option<&Path>) -> IsolationSettings {
    crate::settings::load_file(config_dir)
        .sections
        .get(SECTION)
        .map(from_values)
        .unwrap_or_default()
}

/// What to do when a built-in is quarantined in process (given its id).
type OnQuarantine = Box<dyn Fn(&str) + Send + Sync>;

/// The session's isolation, shared by every window.
pub struct AppIsolation {
    pub isolation: Isolation,
    /// Persists "run isolated" for a plugin quarantined in process (set once the settings service runs).
    on_quarantine: OnceLock<OnQuarantine>,
    handled: Mutex<Vec<String>>,
}

impl AppIsolation {
    pub fn new(config_dir: Option<&Path>, safe_mode: bool) -> AppIsolation {
        let cli = std::env::args().any(|a| a == ISOLATED_FLAG);
        let settings = if safe_mode {
            IsolationSettings::default()
        } else {
            load(config_dir)
        };
        AppIsolation {
            isolation: Isolation::new(settings, cli && !safe_mode, builtins::default_host_path()),
            on_quarantine: OnceLock::new(),
            handled: Mutex::new(Vec::new()),
        }
    }

    /// The built-in plugins for a new window, isolated ones replaced by their plugin process.
    pub fn builtins(&self) -> Vec<Arc<dyn Plugin>> {
        self.isolation.resolve(builtins::all())
    }

    /// Watches a window's registry: a built-in quarantined while running in process is set to run
    /// isolated (when the setting allows it).
    pub fn watch(self: &Arc<Self>, registry: &imt_core::PluginRegistry) {
        let me = self.clone();
        let _ = registry
            .events()
            .subscribe::<imt_core::events::PluginStateChanged>("imt.app", move |e| {
                if e.status == imt_core::plugin::PluginStatus::Quarantined {
                    me.quarantined(&e.id);
                }
            });
    }

    fn quarantined(&self, id: &str) {
        let in_process = self.isolation.modes().get(id) == Some(&Mode::InProcess);
        let isolable = builtins::find(id).is_some_and(|p| builtins::isolable(p.as_ref()) != Isolable::No);
        if !in_process || !isolable || !self.isolation.settings().auto_isolate {
            return;
        }
        let mut handled = self.handled.lock().unwrap_or_else(|p| p.into_inner());
        if handled.iter().any(|h| h == id) {
            return;
        }
        handled.push(id.to_string());
        log::warn!(Plugin = id; "Plugin quarantined in process; it runs isolated from the next start");
        if let Some(f) = self.on_quarantine.get() {
            f(id);
        }
    }

    /// Sets how a quarantine is persisted (from `setup`, once settings are available).
    pub fn set_on_quarantine(&self, f: OnQuarantine) {
        let _ = self.on_quarantine.set(f);
    }
}

/// Persists "run isolated" for `id` through the settings service.
pub fn install<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    use tauri::Manager;
    let Some(state) = app.try_state::<crate::AppState>() else { return };
    let handle = app.clone();
    state.isolation.set_on_quarantine(Box::new(move |id| {
        let current = crate::settings::settings_sections(handle.clone())
            .ok()
            .and_then(|o| o.sections.into_iter().find(|s| s.section.id == SECTION))
            .map(|s| s.values)
            .unwrap_or_else(|| json!({}));
        let mut values = current.as_object().cloned().unwrap_or_default();
        values.insert(format!("isolate.{id}"), json!(true));
        if let Err(e) = crate::settings::set_settings(handle.clone(), SECTION.into(), Value::Object(values)) {
            log::error!(Plugin = id; "Could not save the isolation setting: {e}");
        }
    }));
}

/// The "Plugin isolation" settings section.
pub struct IsolationSection {
    /// (id, name, how it can run isolated) of every built-in plugin.
    plugins: Vec<(String, String, Isolable)>,
}

impl IsolationSection {
    pub fn new() -> IsolationSection {
        IsolationSection {
            plugins: builtins::all()
                .iter()
                .map(|p| (p.id().to_string(), p.name().to_string(), builtins::isolable(p.as_ref())))
                .collect(),
        }
    }
}

impl Configurable for IsolationSection {
    fn settings_section(&self) -> SettingsSection {
        let mut s = SettingsSection::new(SECTION, "Plugin isolation", SectionOwner::App)
            .describe(
                "Run plugins in a separate process, so a crash, hang or memory exhaustion in a plugin cannot take IMT down. \
                 Slightly slower (records cross a local socket). Changes take effect after a restart.",
            )
            .field(
                SettingField::new("isolatedMode", "Isolated mode", FieldKind::Bool, false)
                    .describe("Run every plugin that can out of process (same as starting IMT with --isolated).")
                    .needs_restart(),
            )
            .field(
                SettingField::new("autoIsolate", "Isolate a plugin after it is quarantined", FieldKind::Bool, true)
                    .describe("A plugin that crashed repeatedly in process runs isolated from the next start.")
                    .needs_restart(),
            );
        for (id, name, isolable) in &self.plugins {
            let description = match isolable {
                Isolable::Fully => id.to_string(),
                Isolable::Partly(lost) => format!("{id} — while isolated, unavailable: {}", lost.join(", ")),
                Isolable::No => continue,
            };
            s = s.field(
                SettingField::new(format!("isolate.{id}"), format!("Run {name} isolated"), FieldKind::Bool, false)
                    .describe(description)
                    .needs_restart(),
            );
            for req in builtins::serve_options(id).permissions {
                s = s.field(
                    SettingField::new(
                        format!("grant.{id}.{}", req.permission.key()),
                        format!("Allow {name} to: {}", req.permission.describe()),
                        FieldKind::Bool,
                        true,
                    )
                    .describe(format!("When it runs isolated. {}", req.reason))
                    .needs_restart(),
                );
            }
        }
        s
    }

    /// Stored by the settings service; read at the next start (see [`load`]).
    fn apply_settings(&self, _values: &ValidValues) -> anyhow::Result<()> {
        Ok(())
    }
}

/// How each plugin runs in this session, for the health view.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IsolationStatus {
    pub isolated_mode: bool,
    /// Plugin id → inProcess / isolated / unavailable.
    pub modes: BTreeMap<String, Mode>,
}

pub fn status(iso: &AppIsolation) -> IsolationStatus {
    IsolationStatus {
        isolated_mode: iso.isolation.isolated_mode(),
        modes: iso.isolation.modes(),
    }
}

/// How each plugin runs in this session (in process, isolated, or isolated but unavailable).
#[tauri::command]
pub fn plugin_isolation_status(state: tauri::State<'_, crate::AppState>) -> IsolationStatus {
    status(&state.isolation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Map;

    /// Values of the section for `settings`.
    fn to_values(settings: &IsolationSettings) -> Value {
        let mut map = Map::new();
        map.insert("isolatedMode".into(), json!(settings.isolated_mode));
        map.insert("autoIsolate".into(), json!(settings.auto_isolate));
        for id in &settings.isolated {
            map.insert(format!("isolate.{id}"), json!(true));
        }
        for (id, keys) in &settings.revoked {
            for k in keys {
                map.insert(format!("grant.{id}.{k}"), json!(false));
            }
        }
        Value::Object(map)
    }

    #[test]
    fn values_round_trip_and_unknown_or_bad_values_mean_defaults() {
        let mut s = IsolationSettings {
            isolated_mode: true,
            auto_isolate: false,
            ..IsolationSettings::default()
        };
        s.set_isolated("net.decryptology.web", true);
        s.set_builtin_grant("net.decryptology.windows", &imt_plugin_sdk::Permission::ReadOpenedFiles, false);
        assert_eq!(from_values(&to_values(&s)), s);
        assert_eq!(from_values(&json!("garbage")), IsolationSettings::default());
        assert_eq!(from_values(&json!({"isolate.unknown.plugin": true})), IsolationSettings::default());
    }

    #[test]
    fn the_section_lists_isolable_plugins_and_their_permissions() {
        let s = IsolationSection::new().settings_section();
        s.check().unwrap();
        let keys: Vec<&str> = s.fields.iter().map(|f| f.key.as_str()).collect();
        assert!(keys.contains(&"isolatedMode") && keys.contains(&"autoIsolate"));
        assert!(keys.contains(&"isolate.net.decryptology.web"));
        assert!(keys.contains(&"grant.net.decryptology.windows.read-opened-files"));
        assert!(!keys.iter().any(|k| k.contains("profiling")), "profiling cannot run isolated");
        let windows = s.fields.iter().find(|f| f.key == "isolate.net.decryptology.windows").unwrap();
        assert!(windows.description.contains("config.parsers"));
        let valid = imt_core::settings::validate(&s, &json!({})).unwrap();
        IsolationSection::new().apply_settings(&valid).unwrap();
    }

    #[test]
    fn settings_load_from_the_settings_file() {
        let dir = std::env::temp_dir().join(format!("imt-iso-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(load(Some(&dir)), IsolationSettings::default());
        std::fs::write(
            dir.join("settings.json"),
            json!({"version": 1, "sections": {SECTION: {"isolatedMode": true}}}).to_string(),
        )
        .unwrap();
        assert!(load(Some(&dir)).isolated_mode);
        assert!(load(None) == IsolationSettings::default());
        let iso = AppIsolation::new(Some(&dir), true);
        assert!(
            !iso.isolation.isolated_mode(),
            "safe mode ignores isolation (no plugins run at all)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_quarantine_in_process_asks_to_isolate_once() {
        let iso = Arc::new(AppIsolation::new(None, false));
        let _ = iso.builtins();
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let s2 = seen.clone();
        iso.set_on_quarantine(Box::new(move |id| s2.lock().unwrap().push(id.to_string())));
        iso.quarantined("net.decryptology.web");
        iso.quarantined("net.decryptology.web");
        iso.quarantined("net.decryptology.profiling");
        iso.quarantined("not.a.builtin");
        assert_eq!(*seen.lock().unwrap(), ["net.decryptology.web"]);
        let st = status(&iso);
        assert_eq!(st.modes["net.decryptology.web"], Mode::InProcess);
        // Through the event bus of a window's registry.
        let mut r = imt_core::PluginRegistry::new();
        r.install_shared(builtins::find("net.decryptology.otel").unwrap()).unwrap();
        iso.watch(&r);
        r.events().publish(imt_core::events::PluginStateChanged {
            id: "net.decryptology.otel".into(),
            status: imt_core::plugin::PluginStatus::Quarantined,
        });
        let started = std::time::Instant::now();
        while seen.lock().unwrap().len() < 2 && started.elapsed() < std::time::Duration::from_secs(5) {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(seen.lock().unwrap().len(), 2);
    }
}
