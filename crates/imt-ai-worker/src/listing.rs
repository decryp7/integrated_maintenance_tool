//! `imt-ai-worker --list-builds`: which inference builds are installed and which of them work on this
//! machine, as one JSON object on stdout ([`BuildsReport`], parsed by `imt_ai::runtimes::list_builds`).
//! IMT runs it on demand for the runtime manager; it reads no model and touches no network.

use crate::locate::{installed_release, LlamaConfig};
use crate::server::Probe;
use crate::variants::{label, try_order, BuildChoice};
pub use imt_ai::runtimes::report::{BuildInfo, BuildsReport};
use std::time::Duration;

/// Probes every installed build (in parallel, each with `timeout`).
pub fn report(cfg: &LlamaConfig, timeout: Duration) -> BuildsReport {
    let choice = match &cfg.choice {
        BuildChoice::Auto => "auto".to_string(),
        BuildChoice::Fixed(id) => id.clone(),
    };
    let found = match cfg.find() {
        Ok(f) => f,
        Err(e) => {
            return BuildsReport {
                release: None,
                source: "missing".into(),
                choice,
                auto: None,
                builds: vec![],
                problem: Some(e.message),
            }
        }
    };
    let probes: Vec<(String, Probe)> = std::thread::scope(|s| {
        let handles: Vec<_> = found
            .builds
            .iter()
            .map(|b| (b.id.clone(), s.spawn(move || Probe::run(&b.exe, timeout))))
            .collect();
        handles
            .into_iter()
            .map(|(id, h)| {
                let probe = h.join().unwrap_or(Probe {
                    devices: vec![],
                    error: Some("the probe failed".into()),
                });
                (id, probe)
            })
            .collect()
    });
    let builds: Vec<BuildInfo> = probes
        .into_iter()
        .map(|(id, p)| {
            let problem = p.usable(&id).err();
            BuildInfo {
                label: label(&id),
                usable: problem.is_none(),
                problem,
                devices: p.devices,
                id,
            }
        })
        .collect();
    let usable: Vec<String> = builds.iter().filter(|b| b.usable).map(|b| b.id.clone()).collect();
    BuildsReport {
        release: found.root.as_deref().and_then(installed_release),
        source: if found.root.is_some() { "installed" } else { "custom" }.into(),
        choice,
        auto: try_order(&usable, &BuildChoice::Auto).into_iter().next(),
        builds,
        problem: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_installed_is_reported_not_failed() {
        let cfg = LlamaConfig {
            custom: None,
            roots: vec![],
            choice: BuildChoice::Fixed("vulkan".into()),
        };
        let r = report(&cfg, Duration::from_secs(1));
        assert_eq!((r.source.as_str(), r.choice.as_str(), r.auto), ("missing", "vulkan", None));
        assert!(r.problem.unwrap().contains("no inference runtime"));
        let v = serde_json::to_value(report(&cfg, Duration::from_secs(1))).unwrap();
        assert!(v.get("builds").is_some() && v.get("release").is_some());
    }

    #[test]
    fn a_broken_build_is_listed_as_unusable() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("cpu")).unwrap();
        // Not an executable: the probe fails and the build is reported with the reason.
        std::fs::write(dir.path().join("cpu").join(crate::variants::server_file_name()), b"not a program").unwrap();
        let cfg = LlamaConfig {
            custom: None,
            roots: vec![dir.path().into()],
            choice: BuildChoice::Auto,
        };
        let r = report(&cfg, Duration::from_secs(5));
        assert_eq!(r.source, "installed");
        assert_eq!(r.builds.len(), 1);
        assert!(!r.builds[0].usable && r.builds[0].problem.is_some());
        assert_eq!((r.auto, r.choice.as_str()), (None, "auto"));
    }
}
