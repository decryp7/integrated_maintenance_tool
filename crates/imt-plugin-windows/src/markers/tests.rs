//! The Windows marker rules against records shaped by the EVTX importer.

use super::*;
use crate::evtx_import::to_record;
use imt_core::model::Record;
use imt_core::profile::{CompiledProfile, Profile};
use imt_core::PluginRegistry;
use serde_json::json;

fn compiled() -> CompiledProfile {
    let mut r = PluginRegistry::new();
    r.install(&crate::WindowsPlugin).unwrap();
    r.compile_profile(Profile::default_profile()).unwrap()
}

/// An event as the EVTX importer turns it into a record.
fn event(provider: &str, id: u32, level: u8, data: serde_json::Value) -> Record {
    to_record(
        &json!({"Event": {
            "System": {
                "Provider": {"#attributes": {"Name": provider}},
                "EventID": id, "Level": level, "Channel": "System", "Computer": "WEB01",
                "TimeCreated": {"#attributes": {"SystemTime": "2026-10-14T10:00:00.000Z"}}
            },
            "EventData": data
        }}),
        1,
        1,
    )
}

fn kinds_of(c: &CompiledProfile, r: &Record) -> Vec<String> {
    let mask = c.compute_markers(r, "System.evtx");
    c.markers
        .iter()
        .enumerate()
        .filter(|(i, _)| mask & (1 << i) != 0)
        .map(|(_, m)| m.kind.clone())
        .collect()
}

#[test]
fn every_rule_matches_its_event_and_takes_priority_over_builtins() {
    let c = compiled();
    let ns = |k: &str| format!("net.decryptology.windows/{k}");
    let cases: &[(&str, u32, u8, &str)] = &[
        (APPLICATION_ERROR, 1000, 2, "app-crash"),
        (WER, 1001, 4, "wer-report"),
        (DOTNET, 1026, 2, "dotnet-crash"),
        (APPLICATION_HANG, 1002, 2, "app-hang"),
        (SCM, 7031, 2, "service-crash"),
        (SCM, 7034, 2, "service-crash"),
        (EVENTLOG, 6008, 2, "unexpected-shutdown"),
        (KERNEL_POWER, 41, 1, "kernel-power"),
        (BUGCHECK, 1001, 2, "bugcheck"),
        (EVENTLOG, 6005, 4, "eventlog-started"),
        (EVENTLOG, 6006, 4, "eventlog-stopped"),
        (KERNEL_GENERAL, 12, 4, "os-started"),
        (KERNEL_GENERAL, 13, 4, "os-shutdown"),
        (DISK, 7, 2, "disk-error"),
        (DISK, 51, 3, "disk-error"),
        (DISK, 153, 3, "disk-error"),
        (UPDATE, 19, 4, "update-installed"),
        (UPDATE, 20, 2, "update-failed"),
        (UPDATE, 43, 4, "update-started"),
        (SECURITY, 4625, 0, "logon-failure"),
    ];
    for (provider, id, level, kind) in cases {
        let r = event(provider, *id, *level, json!({"param1": "x"}));
        let found = kinds_of(&c, &r);
        // The plugin kind comes before built-in kinds such as "error".
        assert_eq!(found.first(), Some(&ns(kind)), "{provider} {id}: {found:?}");
    }
    // Same id, other provider: WER 1001 is not a bugcheck, and vice versa.
    assert_eq!(kinds_of(&c, &event(WER, 1001, 4, json!({})))[0], ns("wer-report"));
    assert!(kinds_of(&c, &event("Some App", 1000, 4, json!({}))).is_empty());
    assert!(kinds_of(&c, &event(EVENTLOG, 6009, 4, json!({}))).is_empty());
    // Annotation-only kinds have no rules.
    assert!(kinds().iter().filter(|k| k.when.is_empty()).count() == 2);
}

#[test]
fn the_sample_security_log_marks_failed_logons_only_where_present() {
    use imt_core::importers::{BinaryImporter, ImportCtx};
    let c = compiled();
    let bytes = include_bytes!("../../tests/fixtures/security.evtx");
    let ctx = ImportCtx {
        profile: &c,
        path: std::path::Path::new("security.evtx"),
        file_date: None,
    };
    let records = crate::EvtxImporter.import(&ctx, bytes).unwrap().records;
    let ns = "net.decryptology.windows/logon-failure".to_string();
    for r in &records {
        let failed = r.fields.get("EventId").and_then(|v| v.as_u64()) == Some(4625);
        assert_eq!(kinds_of(&c, r).contains(&ns), failed, "{:?}", r.fields.get("EventId"));
    }
}

#[test]
fn procmon_denied_results_are_marked() {
    use imt_core::importers::{ImportCtx, Importer};
    let c = compiled();
    let ctx = ImportCtx {
        profile: &c,
        path: std::path::Path::new("Logfile.CSV"),
        file_date: None,
    };
    let csv = include_str!("../../tests/fixtures/procmon.csv");
    let records = crate::ProcmonCsvImporter.import(&ctx, csv).unwrap().records;
    let ns = "net.decryptology.windows/access-denied".to_string();
    let mut marked = 0;
    for r in &records {
        let denied = PROCMON_DENIED.iter().any(|d| r.fields["Result"] == *d);
        assert_eq!(kinds_of(&c, r).contains(&ns), denied, "{:?}", r.fields["Result"]);
        marked += usize::from(denied);
    }
    assert_eq!(marked, 3);
}
