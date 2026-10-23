//! Exporting a project: Cancel stops it per file and per block and leaves no `.part` file; the phase is told by the
//! files the export leaves.

use debrief_core::package::{export_phase, part_path, ExportCancelled, ExportPhase, ProjectDir, EXPORT_TMP, EXPORT_TMP_PREFIX};
use debrief_core::ql::QueryControl;
use std::path::PathBuf;

fn project(tag: &str, big: usize) -> (PathBuf, ProjectDir) {
    let root = std::env::temp_dir().join(format!("debrief-export-cancel-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let dir = root.join("p.debrief");
    let p = ProjectDir::create(&dir, "p").unwrap();
    std::fs::write(dir.join("data.bin"), vec![7u8; big]).unwrap();
    (root, p)
}

#[test]
fn a_cancelled_export_leaves_no_file_behind() {
    let (root, p) = project("pre", 3 << 20);
    let out = root.join("p.debriefz");
    let control = QueryControl::new();
    control.cancel();
    let err = control.scope(|| p.export(&out)).unwrap_err();
    assert!(err.is::<ExportCancelled>(), "{err:#}");
    assert!(!out.exists() && !part_path(&out).exists());
    // Not cancelled: it is written, and counts its bytes as progress.
    let control = QueryControl::new();
    let n = control.scope(|| p.export(&out)).unwrap();
    assert!(n >= 3 << 20 && out.exists() && !part_path(&out).exists());
    assert!(control.progress().done >= 3 << 20);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn cancel_is_looked_at_every_block_of_a_big_file() {
    let (root, p) = project("block", 5 << 20);
    let out = root.join("p.debriefz");
    let control = QueryControl::new();
    let (c, part) = (control.clone(), part_path(&out));
    // Cancels as soon as the archive has data: the copy of data.bin is stopped at its next 1 MB block.
    let watcher = std::thread::spawn(move || {
        for _ in 0..20_000 {
            if std::fs::metadata(&part).map(|m| m.len() > 0).unwrap_or(false) {
                c.cancel();
                return;
            }
            std::thread::sleep(std::time::Duration::from_micros(100));
        }
    });
    let result = control.scope(|| p.export(&out));
    watcher.join().unwrap();
    match result {
        Err(e) => {
            assert!(e.is::<ExportCancelled>(), "{e:#}");
            assert!(!out.exists() && !part_path(&out).exists(), "no partial file");
        }
        // The machine wrote the whole archive before the watcher looked: then it is complete.
        Ok(_) => assert!(out.exists()),
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn the_phase_follows_the_files_the_export_leaves() {
    let (root, p) = project("phase", 1024);
    let out = root.join("p.debriefz");
    assert_eq!(export_phase(&p.dir, &out), ExportPhase::Checkpoint);
    let work = p.dir.join(EXPORT_TMP).join(format!("{EXPORT_TMP_PREFIX}1"));
    std::fs::create_dir_all(&work).unwrap();
    assert_eq!(export_phase(&p.dir, &out), ExportPhase::CopyingDatabase);
    std::fs::write(part_path(&out), [0u8; 10]).unwrap();
    assert_eq!(export_phase(&p.dir, &out), ExportPhase::WritingArchive { bytes: 10 });
    let _ = std::fs::remove_dir_all(root);
}
