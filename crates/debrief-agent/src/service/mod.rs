//! Running as an OS service. The service logic is [`run_service`], shared by
//! every flavour; the OS-specific parts are thin: Windows Service Control
//! Manager integration ([`windows`]), and generated systemd units
//! ([`systemd`]) and launchd property lists ([`launchd`]) that the
//! administrator installs with the usual tools.

pub mod launchd;
pub mod systemd;
#[cfg(windows)]
pub mod windows;

use crate::agent::{Agent, Deps};
use crate::config::{describe, AgentConfig};
use crate::log::AgentLog;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// Windows service name and systemd unit name.
pub const SERVICE_NAME: &str = "debrief-agent";
pub const DISPLAY_NAME: &str = "Debrief Agent";
pub const DESCRIPTION: &str = "Debrief agent: scheduled and triggered diagnostics collection (local files only)";
/// launchd job label.
pub const LAUNCHD_LABEL: &str = "net.decryptology.debrief-agent";

/// Loads and validates the config, opens the log and runs the agent until `stop`.
pub fn run_service(config: &Path, stop: Arc<AtomicBool>, echo: bool) -> Result<(), String> {
    let cfg = AgentConfig::load(config).map_err(|i| format!("{}:\n{}", config.display(), describe(&i)))?;
    let log =
        AgentLog::open(&cfg.log_dir, &cfg.log, echo).map_err(|e| format!("cannot open the log folder {}: {e:#}", cfg.log_dir.display()))?;
    let deps = Deps::system(Arc::new(log), &cfg);
    let mut agent = Agent::new(cfg, deps, stop);
    agent.run();
    Ok(())
}

/// The absolute path of a config file (services run with another working folder).
pub fn absolute_config(path: &Path) -> Result<PathBuf, String> {
    std::fs::canonicalize(path).map_err(|e| format!("cannot find the config file {}: {e}", path.display()))
}

/// Turns a Service Control Manager error code into advice.
pub fn explain_scm_error(code: Option<i32>, action: &str, detail: &str) -> String {
    match code {
        Some(5) => format!("{action} needs administrator rights: run this command from an elevated (\"Run as administrator\") prompt"),
        Some(1073) => format!("{action}: the service is already installed; run `debrief-agent service uninstall` first"),
        Some(1060) => format!("{action}: the service is not installed"),
        Some(1072) => format!("{action}: the service is marked for deletion; close the Services console and try again"),
        _ => format!("{action} failed: {detail}"),
    }
}

/// Quotes a path for a generated file; refuses line breaks, which would let a
/// path inject extra directives.
pub(crate) fn single_line(p: &Path) -> Result<String, String> {
    let s = p.to_string_lossy().into_owned();
    if s.contains(['\n', '\r', '\0']) {
        return Err(format!("the path {s:?} contains a line break"));
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scm_errors_are_explained() {
        assert!(explain_scm_error(Some(5), "Installing the service", "").contains("elevated"));
        assert!(explain_scm_error(Some(1073), "Installing", "").contains("already installed"));
        assert!(explain_scm_error(Some(1060), "Uninstalling", "").contains("not installed"));
        assert!(explain_scm_error(Some(1072), "Installing", "").contains("marked for deletion"));
        assert_eq!(explain_scm_error(None, "Starting", "boom"), "Starting failed: boom");
    }

    #[test]
    fn run_service_reports_bad_config_and_runs_until_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("debrief-agent.toml");
        assert!(run_service(&cfg, Arc::new(AtomicBool::new(true)), false)
            .unwrap_err()
            .contains("cannot read"));
        std::fs::write(&cfg, "version = 1\n[agent]\noutputDir = \"runs\"\n").unwrap();
        run_service(&cfg, Arc::new(AtomicBool::new(true)), false).unwrap();
        let log = std::fs::read_to_string(dir.path().join("runs/.debrief-agent/logs/debrief-agent.clef")).unwrap();
        assert!(log.contains("Agent started") && log.contains("Agent stopped"), "{log}");
        // The log folder cannot be created.
        std::fs::write(dir.path().join("file"), "x").unwrap();
        std::fs::write(&cfg, "version = 1\n[agent]\noutputDir = \"runs\"\nlogDir = \"file/logs\"\n").unwrap();
        assert!(run_service(&cfg, Arc::new(AtomicBool::new(true)), false)
            .unwrap_err()
            .contains("log folder"));
        assert!(absolute_config(&dir.path().join("none")).is_err());
        assert!(absolute_config(&cfg).unwrap().is_absolute());
        assert!(single_line(Path::new("a\nb")).is_err());
    }
}
