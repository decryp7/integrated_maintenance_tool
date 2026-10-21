//! App-wide state shared by every window (configuration folders, safe mode, which plugins are enabled, the
//! plugin processes, the store worker) and the factory that gives each new window its own store and host.

use crate::engine::{Engine, Host, StoreHost};
use crate::profile::{load_profile, ProfileDoc};
use crate::windows::{WindowFactory, WindowState};
use imt_plugin_host::Builtins;
use std::path::PathBuf;
use std::sync::Arc;

/// Managed as Tauri state.
#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) profile_dir: Option<PathBuf>,
    /// The app's configuration folder (plugin and app settings).
    pub(crate) config_dir: Option<PathBuf>,
    pub(crate) safe_mode: bool,
    /// Plugins the user turned off (read once: changes need a restart).
    disabled: Arc<Vec<String>>,
    /// The built-in plugins, each in its own process (ADR 0017), shared by every window: a running process
    /// monitor is the same in every window. None in safe mode. Installed third-party plugins are among them.
    pub(crate) plugins: Arc<Builtins>,
    /// Installed third-party plugins (ADR 0027): the install folder, the trust settings and what was loaded.
    pub(crate) installed: Arc<crate::plugin_packages::Installed>,
    /// Where every window's store is made and opened (the store worker).
    pub(crate) stores: Arc<dyn StoreHost>,
    /// The folder of scratch stores (`<data>/stores`).
    pub(crate) store_root: PathBuf,
}

/// The folder of scratch stores under IMT's data folder (the temp folder when there is none).
pub(crate) fn store_root(data: Option<&std::path::Path>) -> PathBuf {
    match data {
        Some(d) => d.join("stores"),
        None => std::env::temp_dir().join(format!("imt-stores-{}", std::process::id())),
    }
}

impl AppState {
    /// The app's state: plugin processes (with the third-party plugins installed below `plugins_dir`, loaded unless
    /// in safe mode) and the store worker (both start on first use).
    pub fn new(
        profile_dir: Option<PathBuf>,
        data_dir: Option<PathBuf>,
        lib_dir: Option<PathBuf>,
        plugins_dir: Option<PathBuf>,
        safe_mode: bool,
    ) -> anyhow::Result<AppState> {
        let mut plugins = if safe_mode {
            Builtins::none()
        } else {
            Builtins::load(&imt_plugin_host::builtins::default_binary())
        };
        let installed = crate::plugin_packages::Installed::load(plugins_dir, safe_mode, &mut plugins);
        let config_dir = profile_dir.as_ref().and_then(|d| d.parent()).map(PathBuf::from);
        let disabled = crate::plugin_config::load_plugin_settings(config_dir.as_deref()).disabled;
        let stores = crate::engine::WorkerHost::new(disabled, safe_mode, lib_dir)?;
        let mut state = AppState::with(
            profile_dir,
            safe_mode,
            Arc::new(plugins),
            Arc::new(stores),
            store_root(data_dir.as_deref()),
        );
        state.installed = Arc::new(installed);
        Ok(state)
    }

    /// The state with the given plugins and store host (tests).
    pub fn with(
        profile_dir: Option<PathBuf>,
        safe_mode: bool,
        plugins: Arc<Builtins>,
        stores: Arc<dyn StoreHost>,
        store_root: PathBuf,
    ) -> AppState {
        let config_dir = profile_dir.as_ref().and_then(|d| d.parent()).map(PathBuf::from);
        let disabled = crate::plugin_config::load_plugin_settings(config_dir.as_deref()).disabled;
        AppState {
            profile_dir,
            config_dir,
            safe_mode,
            disabled: Arc::new(disabled),
            plugins,
            installed: Arc::new(crate::plugin_packages::Installed::none()),
            stores,
            store_root,
        }
    }

    /// A new window: its own plugin registry (crash health is per window; the plugin processes are shared) with
    /// the saved profile (the default in safe mode, where no plugin is installed), and its store slot (the store
    /// is made on first use).
    pub fn new_window(&self, label: &str) -> anyhow::Result<WindowState> {
        let registry = crate::plugin_config::install(self.plugins.plugins(), self.safe_mode, &self.disabled)?;
        let (profile, doc) = if self.safe_mode {
            ProfileDoc::builtin()
        } else {
            load_profile(self.profile_dir.as_ref(), &registry)
        };
        let profiling = crate::profiling::ProfilingSlot::from_registry(&registry);
        let dumps = crate::dumps::DumpsSlot::from_registry(&registry);
        let host = Host::new(registry, profile.clone())?;
        let engine = Engine::new(self.stores.clone(), self.store_root.clone(), label, profile);
        let mut state = WindowState::new(label, engine, host, doc);
        state.profiling = profiling;
        state.dumps = dumps;
        Ok(state)
    }

    /// The factory for [`crate::windows::Windows`].
    pub fn factory(&self) -> WindowFactory {
        let shared = self.clone();
        Box::new(move |label| shared.new_window(label))
    }
}
