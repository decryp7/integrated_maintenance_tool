//! App-wide state shared by every window (configuration folders, safe mode, which plugins are enabled, the
//! plugin processes) and the factory that gives each new window its own workspace.

use crate::profile::{load_profile, ProfileDoc};
use crate::windows::{WindowFactory, WindowState};
use imt_core::{LogStore, Workspace};
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
    /// monitor is the same in every window. None in safe mode.
    pub(crate) plugins: Arc<Builtins>,
}

impl AppState {
    pub fn new(profile_dir: Option<PathBuf>, safe_mode: bool) -> AppState {
        let plugins = if safe_mode {
            Builtins::none()
        } else {
            Builtins::load(&imt_plugin_host::builtins::default_binary())
        };
        AppState::with_plugins(profile_dir, safe_mode, plugins)
    }

    /// The state with the given plugins (tests).
    pub fn with_plugins(profile_dir: Option<PathBuf>, safe_mode: bool, plugins: Builtins) -> AppState {
        let config_dir = profile_dir.as_ref().and_then(|d| d.parent()).map(PathBuf::from);
        let disabled = crate::plugin_config::load_plugin_settings(config_dir.as_deref()).disabled;
        AppState {
            profile_dir,
            config_dir,
            safe_mode,
            disabled: Arc::new(disabled),
            plugins: Arc::new(plugins),
        }
    }

    /// A new window's workspace: its own plugin registry (crash health is per window; the plugin processes are
    /// shared), the saved profile (the default in safe mode, where no plugin is installed).
    pub fn new_window(&self, label: &str) -> anyhow::Result<WindowState> {
        let registry = crate::plugin_config::install(self.plugins.plugins(), self.safe_mode, &self.disabled)?;
        let mut workspace = Workspace::with_registry(imt_core::Profile::default_profile(), registry)?;
        let (profile, doc) = if self.safe_mode {
            ProfileDoc::builtin()
        } else {
            load_profile(self.profile_dir.as_ref(), &workspace)
        };
        workspace.set_profile(profile)?;
        let progress = workspace.progress();
        let profiling = crate::profiling::ProfilingSlot::from_registry(workspace.registry());
        let dumps = crate::dumps::DumpsSlot::from_registry(workspace.registry());
        let store: Box<dyn LogStore> = Box::new(workspace);
        let mut state = WindowState::new(label, store, progress, doc);
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
