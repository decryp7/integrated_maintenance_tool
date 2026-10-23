//! Debrief's own folders. Everything in the shell resolves them here, never through `app.path().app_*_dir()`
//! directly, so a debug build's `DEBRIEF_DATA_DIR` ([`crate::dev_overrides`]) relocates all of them at once:
//!
//! | folder        | normally (Tauri)               | with `DEBRIEF_DATA_DIR=<root>` |
//! |---------------|--------------------------------|----------------------------|
//! | data          | `app_data_dir`                 | `<root>/data`              |
//! | config        | `app_config_dir`               | `<root>/config`            |
//! | logs          | `app_log_dir`                  | `<root>/logs`              |
//! | webview data  | `app_local_data_dir` (Tauri's) | `<root>/local`             |
//!
//! Debrief keeps no cache of its own; the webview's caches live in its data folder.

use std::path::{Path, PathBuf};

use tauri::{Manager, Runtime};

/// One of Debrief's folders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppDir {
    Data,
    Config,
    Log,
}

impl AppDir {
    fn sub(self) -> &'static str {
        match self {
            Self::Data => "data",
            Self::Config => "config",
            Self::Log => "logs",
        }
    }
}

/// The folder under an override root.
pub fn under(root: &Path, dir: AppDir) -> PathBuf {
    root.join(dir.sub())
}

/// The webview's user data folder under an override root.
pub fn webview_data_dir(root: &Path) -> PathBuf {
    root.join("local")
}

/// Resolves a folder: the override root's when one is set, else Tauri's.
pub fn resolve<R: Runtime, M: Manager<R>>(app: &M, dir: AppDir) -> tauri::Result<PathBuf> {
    resolve_with(app, dir, crate::dev_overrides::current().data_dir.as_deref())
}

fn resolve_with<R: Runtime, M: Manager<R>>(app: &M, dir: AppDir, root: Option<&Path>) -> tauri::Result<PathBuf> {
    if let Some(root) = root {
        return Ok(under(root, dir));
    }
    let path = app.path();
    match dir {
        AppDir::Data => path.app_data_dir(),
        AppDir::Config => path.app_config_dir(),
        AppDir::Log => path.app_log_dir(),
    }
}

/// The webview's user data folder (its caches live inside): `<root>/local` under an override root, else
/// Tauri's `app_local_data_dir`, which Tauri gives WebView2 and WebKitGTK when no folder is set.
pub fn webview_data<R: Runtime, M: Manager<R>>(app: &M) -> tauri::Result<PathBuf> {
    webview_data_with(app, crate::dev_overrides::current().data_dir.as_deref())
}

fn webview_data_with<R: Runtime, M: Manager<R>>(app: &M, root: Option<&Path>) -> tauri::Result<PathBuf> {
    match root {
        Some(root) => Ok(webview_data_dir(root)),
        None => app.path().app_local_data_dir(),
    }
}

/// Debrief's data folder (sessions, the running marker, models, agent runs).
pub fn data<R: Runtime, M: Manager<R>>(app: &M) -> tauri::Result<PathBuf> {
    resolve(app, AppDir::Data)
}

/// Debrief's config folder (settings, profiles, window state).
pub fn config<R: Runtime, M: Manager<R>>(app: &M) -> tauri::Result<PathBuf> {
    resolve(app, AppDir::Config)
}

/// Debrief's log folder (CLEF logs, error snapshots).
pub fn log<R: Runtime, M: Manager<R>>(app: &M) -> tauri::Result<PathBuf> {
    resolve(app, AppDir::Log)
}

/// The window-state plugin's file: absolute under an override root (the plugin joins it onto Tauri's config
/// folder, and joining an absolute path replaces that), else the plugin's default name.
pub fn window_state_file(root: Option<&Path>) -> Option<PathBuf> {
    root.map(|r| under(r, AppDir::Config).join(".window-state.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_override_root_holds_every_folder() {
        let app = crate::test_support::test_app_with_commands(None);
        let root = std::env::temp_dir().join("debrief-app-dirs-test");
        for (dir, sub) in [(AppDir::Data, "data"), (AppDir::Config, "config"), (AppDir::Log, "logs")] {
            assert_eq!(resolve_with(&app, dir, Some(&root)).unwrap(), root.join(sub));
        }
        assert_eq!(webview_data_dir(&root), root.join("local"));
        assert_eq!(webview_data_with(&app, Some(&root)).unwrap(), root.join("local"));
        assert_eq!(window_state_file(Some(&root)), Some(root.join("config").join(".window-state.json")));
        assert!(window_state_file(Some(&root)).unwrap().is_absolute());
        assert_eq!(window_state_file(None), None);
    }

    #[test]
    fn without_an_override_tauri_resolves_them() {
        let app = crate::test_support::test_app_with_commands(None);
        let p = app.path();
        assert_eq!(resolve_with(&app, AppDir::Data, None).ok(), p.app_data_dir().ok());
        assert_eq!(resolve_with(&app, AppDir::Config, None).ok(), p.app_config_dir().ok());
        assert_eq!(resolve_with(&app, AppDir::Log, None).ok(), p.app_log_dir().ok());
        // Unit tests never call `dev_overrides::init`.
        assert_eq!(data(&app).ok(), p.app_data_dir().ok());
        assert_eq!(config(&app).ok(), p.app_config_dir().ok());
        assert_eq!(log(&app).ok(), p.app_log_dir().ok());
        assert_eq!(webview_data(&app).ok(), p.app_local_data_dir().ok());
    }
}
