//! The built-in plugins, and how each runs out of process in the `imt-plugin-host` binary.

use crate::connection::PluginSpec;
use imt_core::{Plugin, PluginRegistry};
use imt_plugin_sdk::{Permission, PermissionRequest, ServeOptions};
use std::path::PathBuf;
use std::sync::Arc;

/// Every compiled-in plugin (the tools plugin is not listed: it is installed shared, in process, by the
/// app). Install order does not matter (activation follows the dependency graph).
pub fn all() -> Vec<Arc<dyn Plugin>> {
    vec![
        Arc::new(imt_plugin_registry::RegistryPlugin),
        Arc::new(imt_plugin_otel::OtelPlugin),
        Arc::new(imt_plugin_syslog::SyslogPlugin),
        Arc::new(imt_plugin_windows::WindowsPlugin),
        Arc::new(imt_plugin_web::WebPlugin),
        Arc::new(imt_plugin_formats::FormatsPlugin),
        Arc::new(imt_plugin_profiling::ProfilingPlugin),
    ]
}

/// The built-in plugin with this id.
pub fn find(id: &str) -> Option<Arc<dyn Plugin>> {
    all().into_iter().find(|p| p.id() == id)
}

/// What a built-in declares when served out of process.
pub fn serve_options(id: &str) -> ServeOptions {
    let mut opts = ServeOptions::default();
    if id == "net.decryptology.windows" {
        opts.permissions.push(PermissionRequest {
            permission: Permission::ReadOpenedFiles,
            reason: "BLG files are converted by relog.exe, which reads the file by its path".into(),
        });
    }
    opts
}

/// How well a plugin can run out of process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Isolable {
    /// Every contribution crosses the contract.
    Fully,
    /// These contributions stay unavailable while it runs isolated (in-process extension points,
    /// collectors, tools, hooks, typed services).
    Partly(Vec<String>),
    /// Nothing it contributes crosses the contract (e.g. only in-process extension points).
    No,
}

/// Contributions the contract carries.
fn crosses(contribution: &str) -> bool {
    ["importer:", "transform:", "exporter:", "marker:", "annotator:"]
        .iter()
        .any(|p| contribution.starts_with(p))
        || contribution == "settings"
}

/// Whether `plugin` can run isolated, found by registering it in a scratch registry (guarded).
pub fn isolable(plugin: &dyn Plugin) -> Isolable {
    let mut r = PluginRegistry::empty();
    if r.install(plugin).is_err() {
        return Isolable::No;
    }
    let contributes = r.plugins().into_iter().next().map(|p| p.contributes).unwrap_or_default();
    let lost: Vec<String> = contributes.iter().filter(|c| !crosses(c)).cloned().collect();
    if contributes.iter().any(|c| crosses(c)) {
        if lost.is_empty() {
            Isolable::Fully
        } else {
            Isolable::Partly(lost)
        }
    } else {
        Isolable::No
    }
}

/// File name of the host binary on this platform (as bundled: no target-triple suffix).
pub fn host_file_name() -> &'static str {
    if cfg!(windows) {
        "imt-plugin-host.exe"
    } else {
        "imt-plugin-host"
    }
}

/// Where the bundled host binary lives: next to the application executable (Tauri puts `externalBin`
/// sidecars there). `IMT_PLUGIN_HOST` overrides it, for development.
pub fn default_host_path() -> PathBuf {
    if let Some(p) = std::env::var_os("IMT_PLUGIN_HOST") {
        return PathBuf::from(p);
    }
    let dir = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(PathBuf::from))
        .unwrap_or_default();
    dir.join(host_file_name())
}

/// How to start built-in `id` in the host binary at `host`, with the permissions the user `granted`.
pub fn spec(host: impl Into<PathBuf>, id: &str, granted: Vec<PermissionRequest>) -> PluginSpec {
    let mut s = PluginSpec::new(host, format!("plugin \"{id}\""));
    s.args = vec!["--plugin".into(), id.into()];
    s.expected_id = Some(id.into());
    s.grants = granted;
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_are_listed_and_classified() {
        let ids: Vec<&str> = all().iter().map(|p| p.id()).collect();
        assert_eq!(ids.len(), 7);
        assert!(ids.contains(&"net.decryptology.formats"));
        assert!(find("net.decryptology.web").is_some() && find("nope").is_none());
        assert_eq!(isolable(find("net.decryptology.web").unwrap().as_ref()), Isolable::Fully);
        assert_eq!(isolable(find("net.decryptology.profiling").unwrap().as_ref()), Isolable::No);
        match isolable(find("net.decryptology.windows").unwrap().as_ref()) {
            Isolable::Partly(lost) => assert_eq!(lost, ["contribution:config.parsers"]),
            other => panic!("{other:?}"),
        }
        assert_eq!(serve_options("net.decryptology.windows").permissions.len(), 1);
        assert!(serve_options("net.decryptology.web").permissions.is_empty());
        let s = spec("/x/host", "net.decryptology.web", Vec::new());
        assert_eq!(s.args, ["--plugin", "net.decryptology.web"]);
        assert_eq!(s.expected_id.as_deref(), Some("net.decryptology.web"));
    }

    #[test]
    fn the_host_path_is_next_to_the_executable_unless_overridden() {
        std::env::remove_var("IMT_PLUGIN_HOST");
        let p = default_host_path();
        assert_eq!(p.file_name().unwrap().to_str().unwrap(), host_file_name());
        std::env::set_var("IMT_PLUGIN_HOST", "/custom/host");
        assert_eq!(default_host_path(), PathBuf::from("/custom/host"));
        std::env::remove_var("IMT_PLUGIN_HOST");
    }
}
