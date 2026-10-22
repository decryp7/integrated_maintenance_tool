//! The built-in "Windows Event Log (EVTX)" parser (ADR 0026) against a real .evtx file
//! (tests/fixtures/security.evtx, see NOTICE.txt): chosen for the file by the library, read by the plugin.

mod common;

use imt_core::{Profile, Workspace};
use std::path::PathBuf;

#[test]
fn the_evtx_parser_reads_event_logs() {
    let registry = common::registry(&[common::WINDOWS]);
    let mut ws = Workspace::with_registry(Profile::default_profile(), registry).unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../imt-plugin-windows/tests/fixtures/security.evtx");
    ws.open(&[fixture]).unwrap();
    let s = ws.summary();
    assert_eq!(s.sources.len(), 1, "{:?}", s.unrecognized);
    assert_eq!(s.sources[0].importer, "evtx");
    assert_eq!(s.sources[0].parser.as_deref(), Some("windows-event"));
    assert!(s.total > 0 && s.sources[0].from.is_some());
    assert!(ws.records().iter().all(|r| r.ts.is_some() && r.fields.contains_key("EventId")));
}
