//! End to end against the generated registry exports in `/samples`: the
//! UTF-16 export in `windows/` imported as records, and the WEB01/WEB02
//! exports in `configs/` compared by the configuration diff.

mod common;

use debrief_core::config::{read_config, snapshot, ChangeKind, CompareOptions, ConfigParsers, Limits};
use debrief_core::{PluginRegistry, Profile, Workspace};
use serde_json::Value;
use std::path::{Path, PathBuf};

fn samples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples")
}

fn registry() -> PluginRegistry {
    common::registry(&[common::REGISTRY])
}

#[test]
fn registry_exports_are_imported_as_records() {
    let mut ws = Workspace::with_registry(Profile::default_profile(), registry()).unwrap();
    ws.open(&[samples().join("windows/registry-export.reg"), samples().join("configs")])
        .unwrap();
    let s = ws.summary();
    let regs: Vec<_> = s.sources.iter().filter(|x| x.importer == "registry").collect();
    assert_eq!(
        regs.len(),
        3,
        "{:?}",
        s.sources.iter().map(|x| (&x.name, &x.importer)).collect::<Vec<_>>()
    );
    assert!(regs.iter().all(|x| x.skipped == 0 && x.records > 0));
    let timeout = ws
        .records()
        .iter()
        .find(|r| r.fields.get("ValueName").and_then(Value::as_str) == Some("PaymentTimeoutMs"))
        .unwrap();
    assert_eq!(timeout.fields["ValueType"], "REG_DWORD");
}

#[test]
fn web01_and_web02_differ_in_the_expected_values() {
    let parsers = ConfigParsers::with_registry(&registry());
    let dir = samples().join("configs");
    let left = read_config(&parsers, &dir.join("WEB01/OrderService.reg"), &Limits::default()).unwrap();
    let right = read_config(&parsers, &dir.join("WEB02/OrderService.reg"), &Limits::default()).unwrap();
    let d = debrief_core::config::diff(&left, &right, &CompareOptions::default()).unwrap();
    let changed = |kind: ChangeKind| -> Vec<&str> {
        d.entries
            .iter()
            .filter(|e| e.kind == kind && !e.has_changes)
            .map(|e| e.display.as_str())
            .collect()
    };
    let modified = changed(ChangeKind::Changed);
    for name in ["GcServer", "NewCheckout", "FailureActions"] {
        assert!(modified.iter().any(|p| p.ends_with(name)), "{name} not in {modified:?}");
    }
    assert!(changed(ChangeKind::Added).iter().any(|p| p.ends_with("DumpFolder")));

    // The whole snapshot: every file pairs up and differs.
    let snap = snapshot::diff_folders(
        &parsers,
        &dir.join("WEB01"),
        &dir.join("WEB02"),
        &CompareOptions::default(),
        &Limits::default(),
    )
    .unwrap();
    let reg = snap.files.iter().find(|f| f.rel_path == "OrderService.reg").unwrap();
    assert_eq!(reg.status, snapshot::FileStatus::Changed);
}
