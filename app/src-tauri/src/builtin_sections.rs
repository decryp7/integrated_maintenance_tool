//! The settings sections the app provides itself (Diagnostics, Workspace, Plugins), served by
//! [`crate::settings`] beside the plugins' sections. Split out of `settings.rs`.

use crate::diagnostics;
use crate::settings::ExternalValues;
use crate::settings::DEFAULT_LIVE_TAIL_DEBOUNCE;
use imt_core::settings::{Configurable, EnumOption, FieldKind, SectionOwner, SettingField, SettingsSection, ValidValues};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

pub const DIAGNOSTICS_SECTION: &str = "diagnostics";
pub const WORKSPACE_SECTION: &str = "workspace";
pub const PLUGINS_SECTION: &str = "plugins";

pub(crate) fn int(min: i64, max: i64, unit: Option<&str>) -> FieldKind {
    FieldKind::Int {
        min,
        max,
        step: 1,
        unit: unit.map(String::from),
    }
}

/// Adapter over `diagnostics.rs`: IMT's own log files and error snapshots.
/// `diagnostics.json` stays the source of truth (it is read at startup,
/// before this service exists, and by the snapshot store).
pub struct DiagnosticsSection {
    pub dir: Option<PathBuf>,
}

impl Configurable for DiagnosticsSection {
    fn settings_section(&self) -> SettingsSection {
        let d = diagnostics::DiagnosticsSettings::default();
        let levels = ["error", "warn", "info", "debug", "trace"]
            .iter()
            .map(|l| EnumOption::new(*l, capitalized(l)))
            .collect();
        SettingsSection::new(DIAGNOSTICS_SECTION, "Diagnostics", SectionOwner::App)
            .describe("IMT's own log files and the screenshots taken when something fails. Nothing is sent anywhere.")
            .field(
                SettingField::new(
                    "logLevel",
                    "Log level",
                    FieldKind::Enum { options: levels },
                    level_name(d.log.level),
                )
                .describe("How much IMT writes to its own log. Debug and Trace help when reporting a problem."),
            )
            .field(
                SettingField::new("logMaxFileMb", "Log file size", int(1, 100, Some("MB")), d.log.max_file_mb)
                    .describe("IMT starts a new log file every day. Past this size in one day, only errors are written until the next day.")
                    .needs_restart(),
            )
            .field(
                SettingField::new("logKeepFiles", "Old log files kept", int(1, 50, None), d.log.keep_files)
                    .describe("How many older daily log files are kept besides the current one.")
                    .needs_restart(),
            )
            .field(
                SettingField::new(
                    "logMaxAgeDays",
                    "Delete logs older than",
                    int(0, 3650, Some("days")),
                    d.log.max_age_days,
                )
                .describe("Old log files are deleted at startup. 0 keeps them regardless of age.")
                .needs_restart()
                .as_advanced(),
            )
            .field(
                SettingField::new(
                    "activityDetail",
                    "Activity log detail",
                    FieldKind::Enum {
                        options: vec![EnumOption::new("normal", "Normal"), EnumOption::new("detailed", "Detailed")],
                    },
                    "normal",
                )
                .describe(
                    "What the activity log records about your operations. Normal keeps only a fingerprint of query \
                     text and the file name of paths; Detailed records full query text and file paths. File contents, \
                     log lines and AI text are never recorded.",
                ),
            )
            .field(
                SettingField::new(
                    "activityQueriesAtDebug",
                    "Log read-only requests at Debug",
                    FieldKind::Bool,
                    d.activity.queries_at_debug,
                )
                .describe("Frequent read-only requests (pages, timeline, charts) are logged only at the Debug level.")
                .as_advanced(),
            )
            .field(
                SettingField::new("snapshotsEnabled", "Error snapshots", FieldKind::Bool, d.snapshots.enabled)
                    .describe("Save a screenshot of IMT locally when an error is shown, to attach to a bug report."),
            )
            .field(
                SettingField::new("snapshotsMaxCount", "Snapshots kept", int(1, 500, None), d.snapshots.max_count)
                    .describe("The newest snapshots are kept; older ones are deleted."),
            )
            .field(
                SettingField::new(
                    "snapshotsMaxTotalMb",
                    "Snapshot storage",
                    int(10, 2000, Some("MB")),
                    d.snapshots.max_total_mb,
                )
                .describe("Total disk space snapshots may use.")
                .as_advanced(),
            )
            .field(
                SettingField::new(
                    "snapshotsMaxAgeDays",
                    "Delete snapshots older than",
                    int(0, 3650, Some("days")),
                    d.snapshots.max_age_days,
                )
                .describe("0 keeps snapshots regardless of age.")
                .as_advanced(),
            )
    }

    fn apply_settings(&self, values: &ValidValues) -> anyhow::Result<()> {
        let dir = self
            .dir
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("there is no configuration folder"))?;
        let s = diagnostics_from(values)?;
        diagnostics::save(dir, &s)?;
        crate::logging::set_level(s.log.level);
        crate::activity::set_config(s.activity);
        Ok(())
    }
}

impl ExternalValues for DiagnosticsSection {
    fn load(&self) -> Value {
        let s = diagnostics::load(self.dir.as_deref());
        json!({
            "logLevel": level_name(s.log.level),
            "logMaxFileMb": s.log.max_file_mb,
            "logKeepFiles": s.log.keep_files,
            "logMaxAgeDays": s.log.max_age_days,
            "snapshotsEnabled": s.snapshots.enabled,
            "snapshotsMaxCount": s.snapshots.max_count,
            "snapshotsMaxTotalMb": s.snapshots.max_total_mb,
            "snapshotsMaxAgeDays": s.snapshots.max_age_days,
            "activityDetail": if s.activity.detail == crate::activity::Detail::Detailed { "detailed" } else { "normal" },
            "activityQueriesAtDebug": s.activity.queries_at_debug,
        })
    }
}

pub(crate) fn diagnostics_from(values: &ValidValues) -> anyhow::Result<diagnostics::DiagnosticsSettings> {
    let level = values.text("logLevel").unwrap_or("info");
    let n = |key: &str| -> anyhow::Result<u32> {
        let v = values.int(key).ok_or_else(|| anyhow::anyhow!("missing setting {key}"))?;
        Ok(u32::try_from(v)?)
    };
    let mut s = diagnostics::DiagnosticsSettings::default();
    s.log.level = serde_json::from_value(json!(level))?;
    s.log.max_file_mb = n("logMaxFileMb")?;
    s.log.keep_files = n("logKeepFiles")?;
    s.log.max_age_days = n("logMaxAgeDays")?;
    s.snapshots.enabled = values.bool("snapshotsEnabled").unwrap_or(true);
    s.snapshots.max_count = n("snapshotsMaxCount")?;
    s.snapshots.max_total_mb = n("snapshotsMaxTotalMb")?;
    s.snapshots.max_age_days = n("snapshotsMaxAgeDays")?;
    s.activity.detail = match values.text("activityDetail") {
        Some("detailed") => crate::activity::Detail::Detailed,
        _ => crate::activity::Detail::Normal,
    };
    s.activity.queries_at_debug = values.bool("activityQueriesAtDebug").unwrap_or(true);
    Ok(s.clamped())
}

pub(crate) fn level_name(level: diagnostics::LogLevel) -> String {
    serde_json::to_value(level)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| "info".into())
}

pub(crate) fn capitalized(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// Workspace knobs of the shell: the live tail debounce.
pub struct WorkspaceSection {
    pub debounce_ms: Arc<AtomicU64>,
}

impl Configurable for WorkspaceSection {
    fn settings_section(&self) -> SettingsSection {
        SettingsSection::new(WORKSPACE_SECTION, "Workspace", SectionOwner::App)
            .describe("How IMT follows files that change while they are open.")
            .field(
                SettingField::new(
                    "liveTailDebounce",
                    "Live tail delay",
                    FieldKind::Duration {
                        min_ms: 100,
                        max_ms: 10_000,
                    },
                    DEFAULT_LIVE_TAIL_DEBOUNCE.as_millis() as u64,
                )
                .describe("Changes arriving within this time are re-imported together. Takes effect the next time live tail is turned on."),
            )
    }

    fn apply_settings(&self, values: &ValidValues) -> anyhow::Result<()> {
        let d = values
            .duration("liveTailDebounce")
            .ok_or_else(|| anyhow::anyhow!("missing setting liveTailDebounce"))?;
        self.debounce_ms.store(d.as_millis() as u64, Ordering::Relaxed);
        Ok(())
    }
}

/// Turns a plugin on or off from the next start (`crate::plugin_config::plugin_set_enabled`).
pub type SetEnabled = Box<dyn Fn(&str, bool) -> anyhow::Result<()> + Send + Sync>;

/// Adapter over the plugin enable/disable switches kept in `plugins.json`.
pub struct PluginsSection {
    pub dir: Option<PathBuf>,
    /// Installed plugins: id, name, version.
    pub plugins: Vec<(String, String, String)>,
    pub set_enabled: SetEnabled,
}

impl Configurable for PluginsSection {
    fn settings_section(&self) -> SettingsSection {
        let mut s = SettingsSection::new(PLUGINS_SECTION, "Plugins", SectionOwner::App)
            .describe("Turn engine plugins (file formats, exporters, tools) on or off. Changes take effect after a restart.");
        for (id, name, version) in &self.plugins {
            s = s.field(
                SettingField::new(id.clone(), name.clone(), FieldKind::Bool, true)
                    .describe(format!("{id} {version}"))
                    .needs_restart(),
            );
        }
        s
    }

    fn apply_settings(&self, values: &ValidValues) -> anyhow::Result<()> {
        let before = self.load();
        for (id, _, _) in &self.plugins {
            let enabled = values.bool(id).unwrap_or(true);
            if before.get(id).and_then(Value::as_bool) != Some(enabled) {
                (self.set_enabled)(id, enabled)?;
            }
        }
        Ok(())
    }
}

impl ExternalValues for PluginsSection {
    fn load(&self) -> Value {
        let disabled = crate::plugin_config::load_plugin_settings(self.dir.as_deref()).disabled;
        Value::Object(
            self.plugins
                .iter()
                .map(|(id, _, _)| (id.clone(), json!(!disabled.contains(id))))
                .collect(),
        )
    }
}
