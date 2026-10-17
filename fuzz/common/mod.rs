//! Helpers shared by the fuzz targets (included with `#[path]`).
#![allow(dead_code)]

use imt_core::importers::{self, BinaryImporter, ImportCtx, Importer};
use imt_core::profile::{CompiledProfile, Profile};
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

pub fn binary_importer(imp: &dyn BinaryImporter, data: &[u8]) {
    let c = ctx();
    assert!(imp.sniff(&c, &data[..data.len().min(4096)]) <= 100);
    let _ = imp.import(&c, data);
}

/// Sniffs and imports a profile or heap snapshot with small limits (import
/// runs even when sniff says no), then runs the views over what came out.
pub fn capture_importer(imp: &dyn imt_profile::CaptureImporter, data: &[u8]) {
    let limits = imt_profile::Limits {
        max_input_bytes: 4 << 20,
        max_frames: 20_000,
        max_stack_nodes: 50_000,
        max_samples: 200_000,
        max_objects: 50_000,
        max_edges: 200_000,
        ..imt_profile::Limits::default()
    };
    assert!(imp.sniff("fuzz", &data[..data.len().min(64 * 1024)]) <= 100);
    if let Ok(capture) = imp.import("fuzz", data, &limits) {
        imt_profile::fuzz_views(capture);
    }
}
