//! Resolving an incident report through a store (`report::resolve_in`): events blocks look their ids up in the
//! store (in pages), ids that are gone are counted as missing, and an unknown (newer) block is kept in the report.

use debrief_core::ports::{EventsRequest, Query as _};
use debrief_core::report::{resolve::resolve_in, Report, ResolvedBlock};
use debrief_core::workspace::WorkspaceStore;
use debrief_core::{Profile, Workspace};
use serde_json::json;
use std::path::PathBuf;

fn store() -> WorkspaceStore {
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(&[PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples/serilog-clef")])
        .unwrap();
    WorkspaceStore::new(ws)
}

#[test]
fn events_blocks_are_looked_up_in_the_store_and_gone_ids_are_counted() {
    let store = store();
    let total = store.summary().unwrap().total as u32;
    // More ids than one page of the store, plus two that do not exist.
    let mut ids: Vec<u32> = (0..total.min(1200)).collect();
    ids.extend([u32::MAX, u32::MAX - 1]);
    let report: Report = serde_json::from_value(json!({
        "id": "r1",
        "title": "Outage",
        "summary": "**Root cause**: the database",
        "sections": [{
            "id": "s1",
            "title": "Timeline",
            "blocks": [
                {"id": "b1", "type": "text", "markdown": "What happened"},
                {"id": "b2", "type": "events", "caption": "The errors", "ids": ids},
                {"id": "b3", "type": "fromTheFuture", "anything": 1}
            ]
        }]
    }))
    .unwrap();
    // The newer block is kept verbatim in the report (saving never loses it) and left out of the export.
    assert!(serde_json::to_string(&report).unwrap().contains("fromTheFuture"));
    let resolved = resolve_in(&store, &report, None).unwrap();
    assert_eq!(resolved.title, "Outage");
    assert!(!resolved.redacted);
    let blocks = &resolved.sections[0].blocks;
    assert!(matches!(&blocks[0], ResolvedBlock::Text { markdown } if markdown == "What happened"));
    let ResolvedBlock::Events { caption, events, missing } = &blocks[1] else {
        panic!("{:?}", blocks[1])
    };
    assert_eq!(caption, "The errors");
    assert_eq!(*missing, 2);
    assert_eq!(events.len(), total.min(1200) as usize);
    // Each line is the stored event.
    let first = store
        .events(&EventsRequest {
            ids: vec![debrief_core::batch::EventId(u64::from(events[0].id))],
            group_by: vec![],
        })
        .unwrap();
    assert_eq!(events[0].message, first[0].message);
    assert_eq!(events[0].source, first[0].source_name);
}
