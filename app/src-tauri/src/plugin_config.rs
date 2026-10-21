//! Which plugins run (ADR 0002, ADR 0017: each in its own process): `plugins.json` (through [`crate::config_store`]) is the one place their
//! enablement is kept, read when a window's registry is built and changed by the Plugins settings section
//! and `plugin_set_enabled` alike; `plugin_reset` lifts a quarantine. Split out of `lifecycle.rs`.

use crate::windows::Windows;
use crate::{err, AppState, CmdResult, State};
use imt_core::{Plugin, PluginRegistry};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;

/// The file of the enabled plugins, in the config folder.
pub(crate) const PLUGINS_FILE: &str = "plugins.json";

#[derive(Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginSettings {
    #[serde(default)]
    pub disabled: Vec<String>,
}

/// The format of `plugins.json`, the one place plugin enablement is kept (the Plugins settings section and
/// `plugin_set_enabled` both go through [`set_plugin_enabled`]).
const PLUGINS_VERSION: u32 = 1;

pub fn load_plugin_settings(dir: Option<&Path>) -> PluginSettings {
    crate::config_store::ConfigStore::new(dir.map(Path::to_path_buf)).load(PLUGINS_FILE, PLUGINS_VERSION)
}

pub(crate) fn save_plugin_settings(dir: &Path, s: &PluginSettings) -> anyhow::Result<()> {
    Ok(crate::config_store::ConfigStore::new(Some(dir.to_path_buf())).save(PLUGINS_FILE, PLUGINS_VERSION, s)?)
}

/// Installs `all` and resolves them. A plugin that fails to install does not
/// stop IMT: the error is logged, and a plugin whose registration failed is
/// listed as inactive with the reason (so are its dependents).
pub(crate) fn install(all: Vec<Arc<dyn Plugin>>, safe_mode: bool, disabled: &[String]) -> anyhow::Result<PluginRegistry> {
    let mut registry = PluginRegistry::new();
    if safe_mode {
        return Ok(registry);
    }
    registry.disable(disabled.iter().cloned());
    for plugin in all {
        let id = plugin.id().to_string();
        if let Err(e) = registry.install_shared(plugin) {
            log::error!(Plugin = id.as_str(); "Plugin not installed: {e:#}");
        }
    }
    registry.resolve();
    for p in registry.plugins() {
        if let Some(reason) = &p.inactive_reason {
            log::warn!(Plugin = p.id.as_str(); "Plugin inactive: {reason}");
        }
        for w in &p.warnings {
            log::info!(Plugin = p.id.as_str(); "Plugin note: {w}");
        }
    }
    Ok(registry)
}

/// Lifts a plugin's quarantine (after repeated crashes) immediately, in every window.
#[tauri::command]
pub async fn plugin_reset(windows: State<'_, Windows>, state: State<'_, AppState>, op: crate::activity::OpId, id: String) -> CmdResult<()> {
    // Its process may be quarantined too.
    if let Some(p) = state.plugins.process(&id) {
        p.reset();
    }
    for w in windows.all() {
        w.store_read().reset_plugin(&id);
    }
    log::info!(Plugin = id.as_str(); "Plugin re-enabled after quarantine");
    crate::activity::domain_event("plugin.reset", op.as_deref(), &[("Plugin", &id)]);
    Ok(())
}

/// Enables or disables a plugin from the next start (returns `true`: restart needed).
#[tauri::command]
pub async fn plugin_set_enabled(state: State<'_, AppState>, op: crate::activity::OpId, id: String, enabled: bool) -> CmdResult<bool> {
    set_plugin_enabled(&state, id, enabled, op.as_deref())
}

/// Enables or disables a plugin from the next start (what `plugin_set_enabled` does); `true`: restart needed.
pub fn set_plugin_enabled(state: &AppState, id: String, enabled: bool, op: Option<&str>) -> CmdResult<bool> {
    let dir = state.config_dir.as_ref().ok_or("no configuration folder")?;
    let mut s = load_plugin_settings(Some(dir));
    s.disabled.retain(|d| *d != id);
    if !enabled {
        s.disabled.push(id.clone());
    }
    save_plugin_settings(dir, &s).map_err(err)?;
    log::info!(Plugin = id.as_str(), Enabled = enabled; "Plugin setting changed");
    let event = if enabled { "plugin.enabled" } else { "plugin.disabled" };
    crate::activity::domain_event(event, op, &[("Plugin", &id)]);
    Ok(true)
}
