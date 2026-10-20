//! Shared by the end-to-end tests: the built-in plugins as IMT runs them (each in its own process of the
//! `imt-plugins` binary, over the real IPC), installed into engine registries and workspaces.

#![allow(dead_code)]

use imt_core::{PluginRegistry, Profile, Workspace};
use imt_plugin_api::importer::ImportCtx;
use imt_plugin_api::time::TimestampParser;
use imt_plugin_host::Builtins;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub const REGISTRY: &str = "net.decryptology.registry";
pub const OTEL: &str = "net.decryptology.otel";
pub const SYSLOG: &str = "net.decryptology.syslog";
pub const WINDOWS: &str = "net.decryptology.windows";
pub const WEB: &str = "net.decryptology.web";
pub const FORMATS: &str = "net.decryptology.formats";
pub const PROFILING: &str = "net.decryptology.profiling";
pub const TOOLS: &str = "net.decryptology.tools";

/// The plugins binary built for these tests.
pub fn binary() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_imt-plugins"))
}

/// Every built-in, as the app loads them (one process per plugin, shared by the tests of this binary).
pub fn builtins() -> &'static Builtins {
    static B: OnceLock<Builtins> = OnceLock::new();
    B.get_or_init(|| {
        let b = Builtins::load(binary());
        assert!(b.problem().is_none(), "{:?}", b.problem());
        b
    })
}

/// A registry with the built-ins `ids` installed (over IPC).
pub fn registry(ids: &[&str]) -> PluginRegistry {
    let mut r = PluginRegistry::new();
    for p in builtins().plugins() {
        if ids.contains(&p.id()) {
            r.install_shared(p).unwrap();
        }
    }
    r.resolve();
    r
}

/// A workspace with the default profile and the built-ins `ids`.
pub fn workspace(ids: &[&str]) -> Workspace {
    Workspace::with_registry(Profile::default_profile(), registry(ids)).unwrap()
}

pub fn samples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples")
}

/// A fixture of a plugin crate (`plugin` is the crate folder, e.g. `imt-plugin-otel`).
pub fn fixture(plugin: &str, name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(plugin)
        .join("tests/fixtures")
        .join(name)
}

/// Runs `f` with an importer context for `path` and the default timestamp settings (importers called directly).
pub fn with_ctx<T>(path: &Path, f: impl FnOnce(&ImportCtx) -> T) -> T {
    let ts = TimestampParser::default();
    f(&ImportCtx::new(path, &ts))
}
