//! Limits for IMT's own diagnostic output: the rolling CLEF log files and the
//! error screenshots. Persisted as `diagnostics.json` in the config folder;
//! a missing or corrupt file, or out-of-range values, fall back to defaults.

use crate::{err, AppState, CmdResult, State};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{Duration, SystemTime};

const FILE: &str = "diagnostics.json";
const VERSION: u32 = 1;
/// Base name of IMT's log files in the app log folder (daily `imt.YYYY-MM-DD.log`; `imt.log` and
/// `imt_*.log` from earlier versions).
pub const LOG_NAME: &str = "imt";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Error,
    Warn,
    #[default]
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    pub fn filter(self) -> log::LevelFilter {
        match self {
            LogLevel::Error => log::LevelFilter::Error,
            LogLevel::Warn => log::LevelFilter::Warn,
            LogLevel::Info => log::LevelFilter::Info,
            LogLevel::Debug => log::LevelFilter::Debug,
            LogLevel::Trace => log::LevelFilter::Trace,
        }
    }
}

/// Rolling log files: a new file every day (at most `max_file_mb` a day, then only errors), keeping
/// `keep_files` old ones, and deleting files older than `max_age_days` (0 = no age limit).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LogLimits {
    pub level: LogLevel,
    pub max_file_mb: u32,
    pub keep_files: u32,
    pub max_age_days: u32,
}

impl Default for LogLimits {
    fn default() -> Self {
        LogLimits {
            level: LogLevel::Info,
            max_file_mb: 10,
            keep_files: 5,
            max_age_days: 30,
        }
    }
}

/// Error screenshots: the newest `max_count` are kept, within `max_total_mb`
/// and `max_age_days` (0 = no age limit).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SnapshotLimits {
    pub enabled: bool,
    pub max_count: u32,
    pub max_total_mb: u32,
    pub max_age_days: u32,
}

impl Default for SnapshotLimits {
    fn default() -> Self {
        SnapshotLimits {
            enabled: true,
            max_count: 50,
            max_total_mb: 100,
            max_age_days: 30,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DiagnosticsSettings {
    pub version: u32,
    pub log: LogLimits,
    pub snapshots: SnapshotLimits,
    /// What the activity log records (see `crate::activity`).
    pub activity: crate::activity::ActivitySettings,
}

impl Default for DiagnosticsSettings {
    fn default() -> Self {
        DiagnosticsSettings {
            version: VERSION,
            log: LogLimits::default(),
            snapshots: SnapshotLimits::default(),
            activity: crate::activity::ActivitySettings::default(),
        }
    }
}

impl DiagnosticsSettings {
    /// Brings every value into its allowed range.
    pub fn clamped(mut self) -> Self {
        self.version = VERSION;
        self.log.max_file_mb = self.log.max_file_mb.clamp(1, 100);
        self.log.keep_files = self.log.keep_files.clamp(1, 50);
        self.log.max_age_days = self.log.max_age_days.min(3650);
        self.snapshots.max_count = self.snapshots.max_count.clamp(1, 500);
        self.snapshots.max_total_mb = self.snapshots.max_total_mb.clamp(10, 2000);
        self.snapshots.max_age_days = self.snapshots.max_age_days.min(3650);
        self
    }
}

pub fn load(dir: Option<&Path>) -> DiagnosticsSettings {
    dir.and_then(|d| std::fs::read_to_string(d.join(FILE)).ok())
        .and_then(|t| serde_json::from_str::<DiagnosticsSettings>(&t).ok())
        .unwrap_or_default()
        .clamped()
}

pub fn save(dir: &Path, s: &DiagnosticsSettings) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(FILE), serde_json::to_string_pretty(s)?)?;
    Ok(())
}

/// Deletes IMT log files older than `max_age_days` (the active file is kept).
/// Returns how many were removed.
pub fn prune_old_logs(log_dir: &Path, max_age_days: u32, now: SystemTime) -> usize {
    if max_age_days == 0 {
        return 0;
    }
    let max_age = Duration::from_secs(u64::from(max_age_days) * 86_400);
    let active = format!("{LOG_NAME}.log");
    let Ok(entries) = std::fs::read_dir(log_dir) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == active || !name.starts_with(LOG_NAME) || !name.ends_with(".log") {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age > max_age);
        if old && std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// What a settings change means for the running app.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SettingsApplied {
    pub settings: DiagnosticsSettings,
    /// Log file size/count changes take effect after a restart; the level applies now.
    pub restart_required: bool,
}

#[tauri::command]
pub fn diagnostics_settings(state: State<'_, AppState>) -> CmdResult<DiagnosticsSettings> {
    Ok(load(state.config_dir.as_deref()))
}

#[tauri::command]
pub fn set_diagnostics_settings(state: State<'_, AppState>, settings: DiagnosticsSettings) -> CmdResult<SettingsApplied> {
    let dir = state.config_dir.as_ref().ok_or("no configuration folder")?;
    let before = load(Some(dir));
    let settings = settings.clamped();
    save(dir, &settings).map_err(err)?;
    crate::logging::set_level(settings.log.level);
    crate::activity::set_config(settings.activity);
    log::info!(Detail = format!("{:?}", settings.activity.detail).as_str(), Level = format!("{:?}", settings.log.level), MaxFileMb = settings.log.max_file_mb, KeepFiles = settings.log.keep_files; "Diagnostics settings changed");
    let restart_required = before.log.max_file_mb != settings.log.max_file_mb || before.log.keep_files != settings.log.keep_files;
    Ok(SettingsApplied {
        settings,
        restart_required,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("imt-diag-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn defaults_round_trip_and_tolerate_bad_files() {
        let d = dir("load");
        assert_eq!(load(Some(&d)), DiagnosticsSettings::default());
        assert_eq!(load(None), DiagnosticsSettings::default());
        std::fs::write(d.join(FILE), "{ not json").unwrap();
        assert_eq!(load(Some(&d)), DiagnosticsSettings::default());
        // Partial files keep the other defaults.
        std::fs::write(d.join(FILE), r#"{"log":{"level":"debug"}}"#).unwrap();
        let s = load(Some(&d));
        assert_eq!((s.log.level, s.log.keep_files, s.snapshots.max_count), (LogLevel::Debug, 5, 50));
        let mut s = DiagnosticsSettings::default();
        s.snapshots.enabled = false;
        save(&d, &s).unwrap();
        assert!(!load(Some(&d)).snapshots.enabled);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn values_are_clamped() {
        let d = dir("clamp");
        std::fs::write(
            d.join(FILE),
            r#"{"version":7,"log":{"maxFileMb":0,"keepFiles":1000,"maxAgeDays":99999},"snapshots":{"maxCount":0,"maxTotalMb":999999}}"#,
        )
        .unwrap();
        let s = load(Some(&d));
        assert_eq!(s.version, VERSION);
        assert_eq!((s.log.max_file_mb, s.log.keep_files, s.log.max_age_days), (1, 50, 3650));
        assert_eq!((s.snapshots.max_count, s.snapshots.max_total_mb), (1, 2000));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn levels_map_to_filters() {
        use log::LevelFilter::*;
        let all = [LogLevel::Error, LogLevel::Warn, LogLevel::Info, LogLevel::Debug, LogLevel::Trace];
        assert_eq!(all.map(LogLevel::filter), [Error, Warn, Info, Debug, Trace]);
    }

    #[test]
    fn prunes_only_old_imt_logs() {
        let d = dir("prune");
        for f in ["imt.log", "imt_2026-01-01.log", "imt_2026-10-16.log", "other.log", "imt.txt"] {
            std::fs::write(d.join(f), "x").unwrap();
        }
        let later = SystemTime::now() + Duration::from_secs(40 * 86_400);
        assert_eq!(prune_old_logs(&d, 0, later), 0);
        assert_eq!(prune_old_logs(&d, 30, SystemTime::now()), 0);
        assert_eq!(prune_old_logs(&d, 30, later), 2);
        let mut left: Vec<String> = std::fs::read_dir(&d)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["imt.log", "imt.txt", "other.log"]);
        assert_eq!(prune_old_logs(&d.join("missing"), 30, later), 0);
        let _ = std::fs::remove_dir_all(&d);
    }
}
