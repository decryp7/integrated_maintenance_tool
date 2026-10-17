//! App-wide state shared by every window (configuration folders, safe mode,
//! which plugins are enabled, plugins with app-wide state) and the factory
//! that gives each new window its own workspace.

use crate::lifecycle;
use crate::profile::{load_profile, ProfileDoc};
use crate::windows::{WindowFactory, WindowState};
use imt_core::Plugin;
use imt_core::{LogStore, Workspace};
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
    /// The tools plugin: one value installed in every window's registry, so
    /// its tools (a running process monitor, ...) are app-wide.
    tools: imt_plugin_tools::ToolsPlugin,
    /// Which plugins run out of process, and their processes (one per plugin, shared by every window).
    pub(crate) isolation: Arc<crate::plugin_isolation::AppIsolation>,
}

impl AppState {
    pub fn new(profile_dir: Option<PathBuf>, safe_mode: bool) -> AppState {
        let config_dir = profile_dir.as_ref().and_then(|d| d.parent()).map(PathBuf::from);
        let disabled = lifecycle::load_plugin_settings(config_dir.as_deref()).disabled;
        let isolation = Arc::new(crate::plugin_isolation::AppIsolation::new(config_dir.as_deref(), safe_mode));
        AppState {
            profile_dir,
            config_dir,
            safe_mode,
            disabled: Arc::new(disabled),
            tools: imt_plugin_tools::ToolsPlugin::new(),
            isolation,
        }
    }

    /// A new window's workspace: its own plugin registry (crash health is per
    /// window), the saved profile (the default in safe mode, where no plugin is installed).
    pub fn new_window(&self, label: &str) -> anyhow::Result<WindowState> {
        let registry = lifecycle::plugins_from(self.safe_mode, &self.disabled, self.builtins(), &[self.shared_plugin()])?;
        self.isolation.watch(&registry);
        let mut workspace = Workspace::with_registry(imt_core::Profile::default_profile(), registry)?;
        let (profile, doc) = if self.safe_mode {
            ProfileDoc::builtin()
        } else {
            load_profile(self.profile_dir.as_ref(), &workspace)
        };
        workspace.set_profile(profile)?;
        let progress = workspace.progress();
        let profiling = crate::profiling::ProfilingSlot::from_registry(workspace.registry());
        let store: Box<dyn LogStore> = Box::new(workspace);
        let mut state = WindowState::new(label, store, progress, doc);
        state.profiling = profiling;
        Ok(state)
    }

    /// The built-in plugins, those set to run isolated replaced by their plugin process (none in safe mode).
    fn builtins(&self) -> Vec<Arc<dyn Plugin>> {
        if self.safe_mode {
            Vec::new()
        } else {
            self.isolation.builtins()
        }
    }

    fn shared_plugin(&self) -> Arc<dyn Plugin> {
        Arc::new(self.tools.clone())
    }

    /// The factory for [`crate::windows::Windows`].
    pub fn factory(&self) -> WindowFactory {
        let shared = self.clone();
        Box::new(move |label| shared.new_window(label))
    }
}
