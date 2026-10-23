//! `samples/profiling` holds profiler captures, not logs: every file there
//! must be recognised by the importers the plugin registers (the sniff picks
//! one, confidently enough for drag and drop to route it to profiling) and
//! import into a non-empty profile or heap snapshot.

use debrief_plugin_profiling::builtin_importers;
use debrief_profile::api::{CaptureSummary, SniffKind};
use debrief_profile::import::SNIFF_BYTES;
use debrief_profile::{CaptureCatalog, CaptureImporter, Captures, Limits};
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn samples() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples/profiling");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_file())
        .collect();
    files.sort();
    files
}

/// The importers (in Debrief they run in the profiling plugin's process; `debrief-plugins`' tests cover that path).
fn registered() -> Vec<Arc<dyn CaptureImporter>> {
    builtin_importers()
}

#[test]
fn every_sample_is_a_capture_that_imports_and_is_not_empty() {
    let files = samples();
    assert!(files.len() >= 10, "samples/profiling has {} files", files.len());
    for path in files {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let bytes = std::fs::read(&path).unwrap();
        let mut store = Captures::new(registered(), Limits::default());
        let head = &bytes[..bytes.len().min(SNIFF_BYTES)];
        let kind = store.sniff(&name, head);
        assert_ne!(kind, SniffKind::None, "{name} is not recognised as a capture");
        let summary = store
            .open(&name, Some(&path.to_string_lossy()), &bytes)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        match summary {
            CaptureSummary::Profile(p) => {
                assert_eq!(kind, SniffKind::Capture, "{name}");
                assert!(p.sample_count > 0 && p.frame_count > 0, "{name} is empty");
            }
            CaptureSummary::Heap(h) => {
                assert_eq!(kind, SniffKind::Heap, "{name}");
                assert!(h.type_count > 0 && h.total_bytes > 0, "{name} is empty");
            }
        }
    }
}
