//! Third-party plugins (ADR 0027): installing `.imtplugin` packages the user chose, reviewing and changing their
//! permissions, turning them on and off, removing them; the "Plugin trust" settings section (developer mode and
//! trusted publisher keys). The package format, verification and the install folder are `imt-plugin-package` and
//! `imt_plugin_host::installed`; this module is the edge: it checks the webview's arguments (paths only from
//! grants), keeps the run's state and converts errors.
//!
//! Wire contract: `app/src/app/core/generated/plugins.ts` (re-exported by `api.types.ts`).

mod commands;
pub mod dto;
mod trust_section;

pub use commands::*;
pub use trust_section::{TrustSection, TRUST_SECTION};

use imt_plugin_host::installed::{InstallRecord, PluginsDir};
use imt_plugin_host::{Builtins, PluginProcess};
use imt_plugin_package::TrustSettings;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

/// What was loaded at start, and where plugins are installed.
pub struct Installed {
    /// The install folder (`<app data>/plugins`); `None` without a data folder.
    pub dir: Option<PluginsDir>,
    /// The trust settings (kept by the "Plugin trust" section).
    pub trust: Arc<RwLock<TrustSettings>>,
    /// The plugins loaded in this run, by id: their record then, and their process or why there is none.
    pub loaded: HashMap<String, (InstallRecord, Result<Arc<PluginProcess>, String>)>,
    pub safe_mode: bool,
}

impl Installed {
    /// Nothing installed (tests, no data folder).
    pub fn none() -> Installed {
        Installed {
            dir: None,
            trust: Arc::default(),
            loaded: HashMap::new(),
            safe_mode: false,
        }
    }

    /// Cleans the install folder `root` and loads its enabled plugins into `builtins` (none in safe mode).
    pub fn load(root: Option<PathBuf>, safe_mode: bool, builtins: &mut Builtins) -> Installed {
        let dir = root.map(PluginsDir::new);
        let mut loaded = HashMap::new();
        if let (Some(d), false) = (&dir, safe_mode) {
            d.cleanup();
            for l in d.load() {
                let process = match l.process {
                    Ok(p) => match builtins.add_installed(p.clone()) {
                        Ok(()) => Ok(p),
                        Err(e) => Err(e),
                    },
                    Err(e) => Err(e),
                };
                if let Err(e) = &process {
                    log::warn!(Plugin = l.record.id.as_str(); "Installed plugin unavailable: {e}");
                }
                loaded.insert(l.record.id.clone(), (l.record, process));
            }
        }
        Installed {
            dir,
            trust: Arc::default(),
            loaded,
            safe_mode,
        }
    }

    /// The trust settings now.
    pub fn trust(&self) -> TrustSettings {
        crate::sync::read(&self.trust).clone()
    }
}

/// Registers the "Plugin trust" settings section, which keeps [`Installed::trust`].
pub fn init<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    use tauri::Manager;
    let Some(state) = app.try_state::<crate::AppState>() else { return };
    let section = TrustSection {
        trust: state.installed.trust.clone(),
    };
    if let Err(e) = crate::settings::register(app, Arc::new(section), crate::settings::SectionGroup::Plugins) {
        log::error!("Plugin trust settings unavailable: {e:#}");
    }
}

#[cfg(test)]
mod tests;
