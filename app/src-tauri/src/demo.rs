//! The demo incident: the sample folders bundled as resources (see
//! `bundle.resources` in tauri.conf.json), opened through the normal open path.

use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager, Runtime};

/// The bundled sample folders, under `samples/` in the resource folder.
pub(crate) const DEMO_FOLDERS: [&str; 5] = ["serilog-clef", "serilog-json", "serilog-text", "log4net", "csv"];

/// Canonical paths of the bundled sample folders that exist under `resources`.
pub(crate) fn demo_paths_in(resources: Option<&Path>) -> Vec<String> {
    let Some(root) = resources.map(|r| r.join("samples")) else {
        return Vec::new();
    };
    DEMO_FOLDERS
        .iter()
        .filter_map(|d| std::fs::canonicalize(root.join(d)).ok())
        .filter(|p| p.is_dir())
        .map(|p: PathBuf| p.to_string_lossy().into_owned())
        .collect()
}

/// The demo's sample folders to open (granted to the calling window: Debrief's own files); empty when they
/// are not installed with this build.
#[tauri::command]
pub(crate) async fn demo_paths<R: Runtime>(app: AppHandle<R>, grants: crate::paths::WindowGrants) -> Vec<String> {
    let paths = demo_paths_in(app.path().resource_dir().ok().as_deref());
    paths
        .into_iter()
        .filter(|p| {
            grants
                .grant(Path::new(p), crate::paths::Access::Read, crate::paths::GrantSource::Output)
                .is_ok()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Now as _;
    use crate::test_support::{test_app, TempDir};

    #[test]
    fn only_installed_folders_are_offered() {
        assert!(demo_paths_in(None).is_empty());
        let dir = TempDir::new("demo");
        assert!(demo_paths_in(Some(&dir.0)).is_empty(), "nothing installed");
        std::fs::create_dir_all(dir.0.join("samples/serilog-clef")).unwrap();
        std::fs::create_dir_all(dir.0.join("samples/log4net")).unwrap();
        std::fs::write(dir.0.join("samples/csv"), "a file, not a folder").unwrap();
        let paths = demo_paths_in(Some(&dir.0));
        assert_eq!(paths.len(), 2);
        assert!(paths[0].ends_with("serilog-clef") && paths[1].ends_with("log4net"));
        assert!(paths.iter().all(|p| Path::new(p).is_absolute()));
        // The command itself answers (whatever the test binary's resource folder holds).
        let app = test_app(None);
        let _ = demo_paths(app.handle().clone(), crate::paths::WindowGrants::of(app.handle(), "main")).now();
    }

    #[test]
    fn the_repository_samples_are_what_is_bundled() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        assert_eq!(demo_paths_in(Some(&repo)).len(), DEMO_FOLDERS.len());
        let conf: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let resources = conf["bundle"]["resources"].as_object().unwrap();
        for d in DEMO_FOLDERS {
            assert_eq!(resources[&format!("../../samples/{d}/")], format!("samples/{d}/"), "{d}");
        }
    }
}
