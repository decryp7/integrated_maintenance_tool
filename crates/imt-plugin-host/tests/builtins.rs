//! The built-in plugins served by the `imt-plugin-host` binary: the conformance kit passes for each, and
//! switching a plugin (or everything) to isolated mode keeps it working.

use imt_core::{Plugin, PluginRegistry};
use imt_plugin_host::builtins::{self, Isolable};
use imt_plugin_host::isolation::{Isolation, IsolationSettings, Mode};
use imt_plugin_sdk::conformance::{run_binary, Options};
use std::path::{Path, PathBuf};

fn host() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_imt-plugin-host"))
}

#[test]
fn every_isolable_builtin_passes_the_conformance_kit() {
    for p in builtins::all() {
        if builtins::isolable(p.as_ref()) == Isolable::No {
            continue;
        }
        let opts = Options {
            fuzz_cases: 16,
            ..Options::default()
        };
        let report = run_binary(host(), &["--plugin".into(), p.id().into()], opts);
        assert!(report.passed(), "{report}");
        assert!(report.plugin.starts_with(p.id()));
    }
}

#[test]
fn the_host_binary_refuses_unknown_plugins() {
    let report = run_binary(host(), &["--plugin".into(), "no.such.plugin".into()], Options::default());
    assert!(!report.passed());
    let out = std::process::Command::new(host()).arg("--list").output().unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("net.decryptology.web"), "{text}");
    let status = std::process::Command::new(host()).status().unwrap();
    assert_eq!(status.code(), Some(2));
}

fn workspace(plugins: Vec<std::sync::Arc<dyn Plugin>>) -> imt_core::Workspace {
    let mut r = PluginRegistry::new();
    for p in plugins {
        r.install_shared(p).unwrap();
    }
    imt_core::Workspace::with_registry(imt_core::Profile::default_profile(), r).unwrap()
}

fn sample(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples").join(name)
}

#[test]
fn switching_isolation_on_keeps_imports_identical() {
    // A W3C log through the web plugin, in process and isolated: the same records.
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("u_ex240101.log");
    std::fs::write(
        &file,
        "#Software: Microsoft Internet Information Services 10.0\n#Version: 1.0\n#Fields: date time cs-method cs-uri-stem sc-status time-taken\n2024-01-01 10:00:00 GET /a 200 15\n2024-01-01 10:00:01 GET /b 500 30\n",
    )
    .unwrap();
    let mut in_process = workspace(builtins::all());
    in_process.open(std::slice::from_ref(&file)).unwrap();

    let iso = Isolation::new(
        IsolationSettings {
            isolated: vec!["net.decryptology.web".into()],
            ..IsolationSettings::default()
        },
        false,
        host(),
    );
    let resolved = iso.resolve(builtins::all());
    let modes = iso.modes();
    assert_eq!(modes["net.decryptology.web"], Mode::Isolated);
    assert_eq!(modes["net.decryptology.windows"], Mode::InProcess);
    let mut isolated = workspace(resolved);
    isolated.open(std::slice::from_ref(&file)).unwrap();
    let (a, b) = (in_process.summary(), isolated.summary());
    assert_eq!(a.total, 2);
    assert_eq!((a.total, a.levels.clone(), a.from, a.to), (b.total, b.levels.clone(), b.from, b.to));
    let req = serde_json::from_value(serde_json::json!({})).unwrap();
    let ra: Vec<String> = in_process
        .page(&req)
        .unwrap()
        .rows
        .iter()
        .map(|r| format!("{:?}", r.message))
        .collect();
    let rb: Vec<String> = isolated
        .page(&req)
        .unwrap()
        .rows
        .iter()
        .map(|r| format!("{:?}", r.message))
        .collect();
    assert_eq!(ra, rb);
    // A second window reuses the same process.
    let again = iso.resolve(builtins::all());
    assert_eq!(again.len(), builtins::all().len());
    let web = again.iter().find(|p| p.id() == "net.decryptology.web").unwrap();
    assert_eq!(web.name(), "Web server logs");
    let remote = iso.remote("net.decryptology.web").unwrap();
    assert!(remote.process_status().pid.is_some());
    assert!(iso.remote("net.decryptology.windows").is_none());
    iso.reset("net.decryptology.web");
    iso.reset("net.decryptology.windows");
    iso.shutdown();
    assert!(remote.process_status().pid.is_none());
}

#[test]
fn isolated_mode_and_the_flag_isolate_every_isolable_plugin() {
    for (settings, flag) in [
        (
            IsolationSettings {
                isolated_mode: true,
                ..IsolationSettings::default()
            },
            false,
        ),
        (IsolationSettings::default(), true),
    ] {
        let iso = Isolation::new(settings, flag, host());
        assert!(iso.isolated_mode());
        let resolved = iso.resolve(builtins::all());
        let modes = iso.modes();
        for p in builtins::all() {
            let expected = if builtins::isolable(p.as_ref()) == Isolable::No {
                Mode::InProcess
            } else {
                Mode::Isolated
            };
            assert_eq!(modes[p.id()], expected, "{}", p.id());
        }
        // Everything still installs and the samples open.
        let mut ws = workspace(resolved);
        let syslog = sample("csv/perfmon-export.csv");
        if syslog.exists() {
            ws.open(&[syslog]).unwrap();
            assert!(ws.summary().total > 0);
        }
    }
}

#[test]
fn a_revoked_permission_is_enforced_for_an_isolated_builtin() {
    let mut settings = IsolationSettings {
        isolated: vec!["net.decryptology.windows".into()],
        ..IsolationSettings::default()
    };
    settings.set_builtin_grant("net.decryptology.windows", &imt_plugin_sdk::Permission::ReadOpenedFiles, false);
    let iso = Isolation::new(settings, false, host());
    let resolved = iso.resolve(builtins::all());
    let mut r = PluginRegistry::new();
    for p in resolved {
        let _ = r.install_shared(p);
    }
    r.resolve();
    let windows = r.plugins().into_iter().find(|p| p.id == "net.decryptology.windows").unwrap();
    assert!(windows.inactive_reason.unwrap_or_default().contains("needs permission"));
}
