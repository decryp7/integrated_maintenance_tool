//! The built-in "Windows Event Log (EVTX)" profile template against a real
//! .evtx file (tests/fixtures/security.evtx, see NOTICE.txt).

mod common;

use imt_core::{Profile, Workspace};
use std::path::PathBuf;

#[test]
fn the_evtx_template_reads_event_logs() {
    let t = imt_core::templates::find("windows-event").unwrap();
    let registry = common::registry(&[common::WINDOWS]);
    let mut ws = Workspace::with_registry(Profile::from_toml(t.toml).unwrap(), registry).unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../imt-plugin-windows/tests/fixtures/security.evtx");
    ws.open(&[fixture]).unwrap();
    let s = ws.summary();
    assert_eq!(s.profile_name, "Windows Event Log (EVTX)");
    assert_eq!(s.sources.len(), 1, "{:?}", s.unrecognized);
    assert_eq!(s.sources[0].importer, "evtx");
    assert!(s.total > 0 && s.sources[0].from.is_some());
    assert_eq!(s.default_group_by, ["Computer", "Provider"]);
    assert!(ws.records().iter().all(|r| r.ts.is_some() && r.fields.contains_key("EventId")));
}
