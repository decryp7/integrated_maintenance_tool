//! The built-in plugins: one binary, `imt-plugins`, serves each of them in its own process
//! (`imt-plugins --plugin <id>`). At start the host asks the binary for every built-in's manifest once
//! (`imt-plugins --manifests`, a short run that loads no file); each plugin's process then starts on first use.

use crate::process::{PluginProcess, PluginSpec};
use crate::remote::RemotePlugin;
use imt_core::Plugin;
use imt_plugin_api::manifest::Manifest;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The flag that prints every built-in's manifest (a JSON array) and exits.
pub const MANIFESTS_FLAG: &str = "--manifests";
/// The flag that serves one built-in.
pub const PLUGIN_FLAG: &str = "--plugin";
/// Longest the manifest run may take.
const MANIFESTS_TIMEOUT: Duration = Duration::from_secs(30);
/// Largest manifest list read.
const MAX_MANIFESTS_BYTES: u64 = 4 << 20;

/// File name of the plugins binary on this platform (as bundled: no target-triple suffix).
pub fn binary_name() -> &'static str {
    if cfg!(windows) {
        "imt-plugins.exe"
    } else {
        "imt-plugins"
    }
}

/// Where the bundled plugins binary lives: next to the application executable (Tauri puts `externalBin`
/// sidecars there). `IMT_PLUGINS` overrides it, for development. A test binary (in cargo's `deps` folder) finds
/// the one cargo built next to that folder.
pub fn default_binary() -> PathBuf {
    if let Some(p) = std::env::var_os("IMT_PLUGINS") {
        return PathBuf::from(p);
    }
    let dir = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(PathBuf::from))
        .unwrap_or_default();
    binary_in(&dir)
}

fn binary_in(dir: &Path) -> PathBuf {
    let here = dir.join(binary_name());
    match dir.parent() {
        Some(up) if !here.exists() && dir.file_name().is_some_and(|n| n == "deps") => up.join(binary_name()),
        _ => here,
    }
}

/// Runs `exe --manifests` and reads the manifests it prints; every manifest is checked.
pub fn load_manifests(exe: &Path) -> Result<Vec<Manifest>, String> {
    // The plugins binary is IMT's own; it runs with a cleared environment and no stdin.
    #[allow(clippy::disallowed_methods)]
    let mut cmd = std::process::Command::new(exe);
    cmd.arg(MANIFESTS_FLAG)
        .env_clear()
        .envs(
            imt_ipc::process::KEPT_ENV
                .iter()
                .filter_map(|k| std::env::var_os(k).map(|v| (k, v))),
        )
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("cannot start the plugins ({}): {e}", exe.display()))?;
    let reader = child.stdout.take().map(|s| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = s.take(MAX_MANIFESTS_BYTES).read_to_end(&mut buf);
            buf
        })
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if started.elapsed() < MANIFESTS_TIMEOUT => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("the plugins did not describe themselves in time".into());
            }
        }
    };
    let out = reader.and_then(|r| r.join().ok()).unwrap_or_default();
    if !status.success() {
        return Err(format!("the plugins could not describe themselves ({status})"));
    }
    let manifests: Vec<Manifest> = serde_json::from_slice(&out).map_err(|e| format!("the plugins' manifests are not valid: {e}"))?;
    for m in &manifests {
        m.check()?;
    }
    Ok(manifests)
}

/// The built-in plugins of this IMT: their processes (one per plugin, shared by every window).
pub struct Builtins {
    processes: Vec<Arc<PluginProcess>>,
    /// Why the plugins are unavailable, when they are.
    problem: Option<String>,
}

impl Builtins {
    /// The built-ins `exe` serves, each granted what it asks for (they are first-party).
    pub fn load(exe: &Path) -> Builtins {
        match load_manifests(exe) {
            Ok(manifests) => Builtins::from_manifests(exe, manifests),
            Err(e) => {
                log::error!("Plugins unavailable: {e}");
                Builtins {
                    processes: Vec::new(),
                    problem: Some(e),
                }
            }
        }
    }

    /// Processes for `manifests` served by `exe`.
    pub fn from_manifests(exe: &Path, manifests: Vec<Manifest>) -> Builtins {
        let processes = manifests
            .into_iter()
            .map(|m| {
                let mut spec = PluginSpec::new(exe, &m.id);
                spec.granted = m.permissions.iter().map(|p| p.permission).collect();
                PluginProcess::new(m, spec)
            })
            .collect();
        Builtins { processes, problem: None }
    }

    /// No plugins (safe mode).
    pub fn none() -> Builtins {
        Builtins {
            processes: Vec::new(),
            problem: None,
        }
    }

    /// Why no plugin is available, when the plugins binary could not be used.
    pub fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    /// One plugin per process, to install into a window's registry.
    pub fn plugins(&self) -> Vec<Arc<dyn Plugin>> {
        self.processes
            .iter()
            .map(|p| Arc::new(RemotePlugin::new(p.clone())) as Arc<dyn Plugin>)
            .collect()
    }

    pub fn process(&self, id: &str) -> Option<&Arc<PluginProcess>> {
        self.processes.iter().find(|p| p.id() == id)
    }

    pub fn processes(&self) -> &[Arc<PluginProcess>] {
        &self.processes
    }

    /// Starts every plugin's process in the background, so the first import does not wait for one.
    pub fn warm_up(&self) {
        for p in &self.processes {
            let p = p.clone();
            let _ = std::thread::Builder::new()
                .name("imt-plugin-start".into())
                .spawn(move || p.warm_up());
        }
    }

    /// Stops every plugin process (on exit; they would also exit when IMT does).
    pub fn shutdown(&self) {
        for p in &self.processes {
            p.shutdown();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_test_binary_in_deps_finds_the_one_cargo_built_beside_it() {
        let t = tempfile::tempdir().unwrap();
        let deps = t.path().join("deps");
        std::fs::create_dir(&deps).unwrap();
        assert_eq!(binary_in(&deps), t.path().join(binary_name()));
        std::fs::write(deps.join(binary_name()), b"").unwrap();
        assert_eq!(binary_in(&deps), deps.join(binary_name()));
        assert_eq!(binary_in(t.path()), t.path().join(binary_name()));
    }

    #[test]
    fn the_binary_is_next_to_the_executable_unless_overridden() {
        std::env::remove_var("IMT_PLUGINS");
        assert_eq!(default_binary().file_name().unwrap().to_str().unwrap(), binary_name());
        std::env::set_var("IMT_PLUGINS", "/custom/plugins");
        assert_eq!(default_binary(), PathBuf::from("/custom/plugins"));
        std::env::remove_var("IMT_PLUGINS");
    }

    #[test]
    fn a_missing_binary_means_no_plugins_with_a_reason() {
        let b = Builtins::load(Path::new("/definitely/not/imt-plugins"));
        assert!(b.plugins().is_empty());
        assert!(b.problem().unwrap().contains("cannot start the plugins"));
        assert!(Builtins::none().problem().is_none());
    }
}
