#![no_main]
// GC heap-walk events (GCBulkType/Node/Edge/roots) with fuzzed payloads in a well-formed nettrace,
// so the input reaches the heap decoder and the snapshot builder rather than the container checks.
use debrief_plugin_profiling::nettrace::heap::ids as h;
use debrief_plugin_profiling::nettrace::writer::{Meta, Row, TraceHeader, Writer, RUNTIME};
use debrief_plugin_profiling::nettrace::NettraceImporter;
use debrief_profile::{CaptureImporter, Limits};
use libfuzzer_sys::fuzz_target;

const IDS: [u32; 7] = [
    h::BULK_TYPE,
    h::BULK_NODE,
    h::BULK_EDGE,
    h::BULK_ROOT_EDGE,
    h::BULK_ROOT_STATIC,
    h::BULK_ROOT_CCW,
    h::BULK_ROOT_CWT,
];

fuzz_target!(|data: &[u8]| {
    let ptr = if data.first().is_some_and(|b| b & 1 == 1) { 4 } else { 8 };
    let mut w = Writer::new(&TraceHeader {
        pointer_size: ptr,
        ..TraceHeader::default()
    });
    let metas: Vec<Meta<'_>> = IDS
        .iter()
        .enumerate()
        .map(|(i, &event_id)| Meta {
            id: i as u32 + 1,
            provider: RUNTIME,
            event_id,
            name: "",
        })
        .collect();
    w.metadata(true, &metas);
    let mut rows = Vec::new();
    let mut rest = data.get(1..).unwrap_or_default();
    while rest.len() >= 2 && rows.len() < 256 {
        let (kind, len) = (rest[0], usize::from(rest[1]) * 4);
        let len = len.min(rest.len() - 2);
        rows.push(Row {
            metadata_id: u32::from(kind) % 7 + 1,
            thread: 1,
            capture_thread: 1,
            ts: 1_000_100,
            payload: rest[2..2 + len].to_vec(),
            ..Row::default()
        });
        rest = &rest[2 + len..];
    }
    w.events(true, &rows);
    let small = Limits {
        max_objects: 100_000,
        max_edges: 400_000,
        max_frames: 10_000,
        ..Limits::default()
    };
    let _ = NettraceImporter.import("fuzz.nettrace", &w.finish(), &small);
});
