//! Helpers shared by the fuzz targets (included with `#[path]`).
#![allow(dead_code)]

use debrief_core::importers::{self, ImportCtx, Importer};
use debrief_core::profile::{CompiledProfile, Profile};
use std::path::Path;
use std::sync::OnceLock;

pub fn default_profile() -> &'static CompiledProfile {
    static P: OnceLock<CompiledProfile> = OnceLock::new();
    P.get_or_init(|| CompiledProfile::compile(Profile::default_profile()).expect("default profile compiles"))
}

pub fn ctx() -> ImportCtx<'static> {
    ImportCtx {
        profile: default_profile(),
        path: Path::new("fuzz"),
        file_date: chrono::NaiveDate::from_ymd_opt(2026, 10, 14),
    }
}

/// Decodes like the workspace does, then sniffs and imports with `imp` (import runs even when sniff says no).
pub fn text_importer(imp: &dyn Importer, data: &[u8]) {
    let Some(text) = importers::decode(data) else { return };
    let c = ctx();
    let head: String = text.chars().take(64 * 1024).collect();
    assert!(imp.sniff(&c, &head) <= 100);
    if let Ok(imported) = imp.import(&c, &text) {
        for r in &imported.records {
            // Records cross the IPC boundary as JSON.
            serde_json::to_string(&r.fields).expect("fields serialise");
        }
    }
}

fn timestamps() -> &'static debrief_plugin_api::time::TimestampParser {
    static T: OnceLock<debrief_plugin_api::time::TimestampParser> = OnceLock::new();
    T.get_or_init(debrief_plugin_api::time::TimestampParser::default)
}

fn plugin_ctx() -> debrief_plugin_api::ImportCtx<'static> {
    let mut c = debrief_plugin_api::ImportCtx::new(Path::new("fuzz"), timestamps());
    c.file_date = chrono::NaiveDate::from_ymd_opt(2026, 10, 14);
    c
}

/// A plugin's text importer (`debrief-plugin-api`), as its process runs it; rows cross the IPC as wire events.
pub fn plugin_text(imp: &dyn debrief_plugin_api::TextImporter, data: &[u8]) {
    let Some(text) = importers::decode(data) else { return };
    let c = plugin_ctx();
    let head: String = text.chars().take(64 * 1024).collect();
    assert!(imp.sniff(&c, &head) <= 100);
    if let Ok(imported) = imp.import(&c, &text) {
        wire_round_trip(&imported.records);
    }
}

/// A plugin's binary importer, as its process runs it.
pub fn plugin_binary(imp: &dyn debrief_plugin_api::BinaryImporter, data: &[u8]) {
    let c = plugin_ctx();
    assert!(imp.sniff(&c, &data[..data.len().min(4096)]) <= 100);
    if let Ok(imported) = imp.import(&c, data) {
        wire_round_trip(&imported.records);
    }
}

/// Records of a plugin cross the IPC as postcard wire events: every record encodes and decodes.
pub fn wire_round_trip(records: &[debrief_plugin_api::Record]) {
    let rows: Vec<_> = records.iter().map(debrief_plugin_api::protocol::WireEvent::of).collect();
    let bytes = debrief_ipc::wire::encode(&rows).expect("wire events encode");
    let back: Vec<debrief_plugin_api::protocol::WireEvent> = debrief_ipc::wire::decode(&bytes).expect("wire events decode");
    assert_eq!(back.len(), rows.len());
}

/// Sniffs and imports a profile or heap snapshot with small limits (import
/// runs even when sniff says no), then runs the views over what came out.
pub fn capture_importer(imp: &dyn debrief_profile::CaptureImporter, data: &[u8]) {
    let limits = debrief_profile::Limits {
        max_input_bytes: 4 << 20,
        max_frames: 20_000,
        max_stack_nodes: 50_000,
        max_samples: 200_000,
        max_objects: 50_000,
        max_edges: 200_000,
        ..debrief_profile::Limits::default()
    };
    assert!(imp.sniff("fuzz", &data[..data.len().min(64 * 1024)]) <= 100);
    if let Ok(capture) = imp.import("fuzz", data, &limits) {
        debrief_profile::fuzz_views(capture);
    }
}
