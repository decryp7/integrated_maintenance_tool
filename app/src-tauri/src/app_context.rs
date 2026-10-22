//! The app's context, built once in `setup` (ADR 0016): Debrief's folders and run flags, and the app-wide
//! services that used to be created on first use (path grants, the recent lists, the per-window scopes, the
//! UI dispatcher). Built up front, they exist before any command or window event needs them, in the same
//! order every run, and tests build the same context ([`AppContext::for_tests`]).
//!
//! Process-wide statics that remain are the ones needed before Tauri exists (the debug overrides, the
//! activity log's configuration, the child-process log folder: the `--export-diagnostics` command line
//! uses them without an app).

use std::path::PathBuf;
use tauri::{AppHandle, Manager, Runtime};

/// Debrief's folders and how this run was started.
#[derive(Debug, Clone, Default)]
pub struct AppContext {
    pub data: Option<PathBuf>,
    pub config: Option<PathBuf>,
    pub log: Option<PathBuf>,
    pub safe_mode: bool,
    /// This process is the single (main) instance.
    pub single_instance: bool,
}

impl AppContext {
    /// The context of the running app: its folders as [`crate::app_dirs`] resolves them.
    pub fn of<R: Runtime, M: Manager<R>>(app: &M, safe_mode: bool, single_instance: bool) -> AppContext {
        AppContext {
            data: crate::app_dirs::data(app).ok(),
            config: crate::app_dirs::config(app).ok(),
            log: crate::app_dirs::log(app).ok(),
            safe_mode,
            single_instance,
        }
    }

    /// A context without folders (nothing is written): what the tests run with.
    #[cfg(test)]
    pub fn for_tests() -> AppContext {
        AppContext::default()
    }
}

/// Manages `ctx` and creates the app-wide services from it.
pub fn install<R: Runtime>(app: &AppHandle<R>, ctx: AppContext) {
    app.manage(crate::paths::PathGrants::default());
    app.manage(crate::recent::Recents::load(ctx.config.as_deref()));
    app.manage(crate::ui_dispatch::UiDispatch::default());
    app.manage(crate::instance::SingleInstance(ctx.single_instance));
    crate::window_scoped::install(app);
    app.manage(ctx);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_context_and_its_services_exist_from_the_start() {
        let app = crate::test_support::test_app(None);
        let ctx = app.state::<AppContext>();
        assert!(ctx.config.is_none() && !ctx.safe_mode);
        assert!(app.try_state::<crate::paths::PathGrants>().is_some());
        assert!(app.try_state::<crate::recent::Recents>().is_some());
        assert!(app.try_state::<crate::ui_dispatch::UiDispatch>().is_some());
        assert!(app
            .try_state::<crate::window_scoped::WindowScopes<tauri::test::MockRuntime>>()
            .is_some());
    }
}
