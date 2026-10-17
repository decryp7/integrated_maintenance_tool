//! Robustness around the app's lifetime: safe mode, detection of unclean
//! shutdowns, persisted plugin enable/disable, session autosave/restore and a
//! crash handler that records panics in IMT's own log.

use crate::{err, AppState, CmdResult, State};
use imt_core::project::ProjectFile;
use imt_core::PluginRegistry;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Command-line flag that starts IMT with built-in importers only and the default profile.
pub const SAFE_MODE_FLAG: &str = "--safe-mode";
const MARKER: &str = "running.marker";
const PLUGINS_FILE: &str = "plugins.json";
const SESSION_FILE: &str = "last-session.imtproj";

/// How this run started; managed as Tauri state.
pub struct Lifecycle {
    pub safe_mode: bool,
    /// IMT did not shut down cleanly last time (crash, kill, power loss).
    pub previous_crash: bool,
    dir: Option<PathBuf>,
}

impl Lifecycle {
    /// Records that IMT is running; detects whether the previous run ended uncleanly.
    pub fn begin(dir: Option<PathBuf>, safe_mode: bool) -> Lifecycle {
        let mut previous_crash = false;
        if let Some(d) = &dir {
            let marker = d.join(MARKER);
            previous_crash = marker.exists();
            let _ = std::fs::create_dir_all(d);
            let _ = std::fs::write(marker, std::process::id().to_string());
        }
        Lifecycle {
            safe_mode,
            previous_crash,
            dir,
        }
    }

    /// Clean shutdown: the next start will not report a crash.
    pub fn end(&self) {
        if let Some(d) = &self.dir {
            let _ = std::fs::remove_file(d.join(MARKER));
        }
    }

    fn session_path(&self) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join(SESSION_FILE))
    }
}

#[derive(Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginSettings {
    #[serde(default)]
    pub disabled: Vec<String>,
}

pub fn load_plugin_settings(dir: Option<&Path>) -> PluginSettings {
    dir.and_then(|d| std::fs::read_to_string(d.join(PLUGINS_FILE)).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_plugin_settings(dir: &Path, s: &PluginSettings) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(PLUGINS_FILE), serde_json::to_string_pretty(s)?)?;
    Ok(())
}

/// The engine's plugins: none in safe mode, else every compiled-in plugin
/// except those the user disabled. Add new plugins here.
pub fn plugins(safe_mode: bool, disabled: &[String]) -> anyhow::Result<PluginRegistry> {
    let mut registry = PluginRegistry::new();
    if safe_mode {
        return Ok(registry);
    }
    registry.disable(disabled.iter().cloned());
    registry.install(&imt_plugin_registry::RegistryPlugin)?;
    registry.install(&imt_plugin_otel::OtelPlugin)?;
    registry.install(&imt_plugin_syslog::SyslogPlugin)?;
    registry.install(&imt_plugin_windows::WindowsPlugin)?;
    registry.install(&imt_plugin_web::WebPlugin)?;
    Ok(registry)
}

/// Logs panics (message and location) to IMT's own CLEF log before the default handler runs.
pub fn install_panic_logging() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        log::error!(Location = location.as_str(); "IMT crashed: {}", imt_core::guard::panic_message(info.payload()));
        previous(info);
    }));
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStatus {
    pub version: &'static str,
    pub safe_mode: bool,
    pub previous_crash: bool,
    pub session_available: bool,
}

#[tauri::command]
pub fn app_status(lifecycle: State<'_, Lifecycle>) -> AppStatus {
    AppStatus {
        version: env!("CARGO_PKG_VERSION"),
        safe_mode: lifecycle.safe_mode,
        previous_crash: lifecycle.previous_crash,
        session_available: lifecycle.session_path().is_some_and(|p| p.exists()),
    }
}

/// Relaunches IMT, optionally in safe mode, reopening `reopen` paths.
#[tauri::command]
pub fn restart_app<R: tauri::Runtime>(app: tauri::AppHandle<R>, safe_mode: bool, reopen: Vec<String>) -> CmdResult<()> {
    let exe = std::env::current_exe().map_err(err)?;
    let mut cmd = std::process::Command::new(exe);
    if safe_mode {
        cmd.arg(SAFE_MODE_FLAG);
    }
    cmd.args(&reopen).spawn().map_err(err)?;
    log::info!(SafeMode = safe_mode; "Restarting");
    app.exit(0);
    Ok(())
}

/// Lifts a plugin's quarantine (after repeated crashes) immediately.
#[tauri::command]
pub fn plugin_reset(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state.store_read().reset_plugin(&id);
    log::info!(Plugin = id.as_str(); "Plugin re-enabled after quarantine");
    Ok(())
}

/// Enables or disables a plugin from the next start (returns `true`: restart needed).
#[tauri::command]
pub fn plugin_set_enabled(state: State<'_, AppState>, id: String, enabled: bool) -> CmdResult<bool> {
    let dir = state.config_dir.as_ref().ok_or("no configuration folder")?;
    let mut s = load_plugin_settings(Some(dir));
    s.disabled.retain(|d| *d != id);
    if !enabled {
        s.disabled.push(id.clone());
    }
    save_plugin_settings(dir, &s).map_err(err)?;
    log::info!(Plugin = id.as_str(), Enabled = enabled; "Plugin setting changed");
    Ok(true)
}

/// Autosaves the session (open files, profile, UI state) for crash recovery.
#[tauri::command]
pub fn save_session(state: State<'_, AppState>, lifecycle: State<'_, Lifecycle>, ui: Value) -> CmdResult<()> {
    let Some(path) = lifecycle.session_path() else { return Ok(()) };
    let profile = state.profile.read().map_err(err)?.toml.clone();
    let ws = state.store_read().state();
    let project = ProjectFile {
        version: 1,
        name: "Last session".into(),
        sources: ws.roots,
        removed: ws.removed,
        profile,
        ui,
    };
    project.save(&path).map_err(err)
}

/// Path of the autosaved session (open it with `open_project`), if any.
#[tauri::command]
pub fn last_session(lifecycle: State<'_, Lifecycle>) -> Option<String> {
    lifecycle
        .session_path()
        .filter(|p| p.exists())
        .map(|p| p.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("imt-life-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn detects_unclean_shutdown() {
        let d = temp("marker");
        let first = Lifecycle::begin(Some(d.clone()), false);
        assert!(!first.previous_crash);
        // No end(): simulates a crash.
        let second = Lifecycle::begin(Some(d.clone()), true);
        assert!(second.previous_crash && second.safe_mode);
        second.end();
        assert!(!Lifecycle::begin(Some(d.clone()), false).previous_crash);
        assert!(!Lifecycle::begin(None, false).previous_crash);
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn plugin_settings_and_safe_mode() {
        let d = temp("plugins");
        assert_eq!(load_plugin_settings(Some(&d)), PluginSettings::default());
        save_plugin_settings(
            &d,
            &PluginSettings {
                disabled: vec!["net.decryptology.web".into()],
            },
        )
        .unwrap();
        let s = load_plugin_settings(Some(&d));
        let registry = plugins(false, &s.disabled).unwrap();
        let web = registry.plugins().into_iter().find(|p| p.id == "net.decryptology.web").unwrap();
        assert_eq!(web.status, imt_core::plugin::PluginStatus::Disabled);
        assert!(plugins(true, &[]).unwrap().plugins().is_empty());
        std::fs::write(d.join(PLUGINS_FILE), "not json").unwrap();
        assert_eq!(load_plugin_settings(Some(&d)), PluginSettings::default());
        std::fs::remove_dir_all(d).unwrap();
    }
}
