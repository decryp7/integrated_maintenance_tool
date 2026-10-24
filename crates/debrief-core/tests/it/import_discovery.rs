//! Finding the files of an import: the parallel walk lists the same files in the same order as one walker,
//! reports growing counts from the first moments (phase "finding") and stops promptly when cancelled.

use debrief_core::ingest::{discover, find_files};
use debrief_core::profile::CompiledProfile;
use debrief_core::workspace::{ImportPhase, ImportProgress};
use debrief_core::Profile;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let p = std::env::temp_dir().join(format!("debrief-discover-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        TempDir(std::fs::canonicalize(p).unwrap())
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn profile() -> CompiledProfile {
    CompiledProfile::compile(Profile::default_profile()).unwrap()
}

/// `n` logs over nested folders, plus files the default profile leaves out.
fn tree(root: &std::path::Path, n: usize) {
    for i in 0..n {
        let dir = root.join(format!("a{}/b{}/c{}", i % 10, (i / 10) % 10, (i / 100) % 10));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("f{i}.log")), "2026-10-14 10:00:00 INFO x\n").unwrap();
    }
    std::fs::write(root.join("skip.exe"), "x").unwrap();
    std::fs::write(root.join("a0/.hidden"), "x").unwrap();
}

#[test]
fn parallel_and_sequential_discovery_list_the_same_files_in_the_same_order() {
    let d = TempDir::new("same");
    tree(&d.0, 3_000);
    let p = profile();
    let roots = vec![d.0.clone()];
    let none = HashSet::new();
    let run = |threads| discover(&roots, &p, &none, &none_known(), &ImportProgress::default(), threads);
    let one = run(1);
    assert_eq!(one.len(), 3_000, "the excluded files are not listed");
    assert!(one.windows(2).all(|w| w[0] < w[1]), "sorted, no duplicates");
    for threads in [2, 8, 16] {
        assert_eq!(run(threads), one, "{threads} walkers");
    }
    let named = find_files(&roots, &roots, &p, &none, &none_known(), &ImportProgress::default(), 8);
    assert_eq!(named.iter().map(|(f, _)| f.clone()).collect::<Vec<_>>(), one);
    assert!(named[0].1.starts_with("a0/"), "names are relative to the folder: {}", named[0].1);
}

fn none_known() -> HashSet<String> {
    HashSet::new()
}

#[test]
fn the_first_progress_is_finding_and_the_found_count_only_grows() {
    let d = TempDir::new("grows");
    tree(&d.0, 20_000);
    let p = profile();
    let roots = vec![d.0.clone()];
    let progress = ImportProgress::default();
    let _run = progress.begin();
    let first = progress.snapshot();
    assert!(
        (first.running, first.phase, first.done, first.total) == (true, ImportPhase::Finding, 0, 0),
        "{first:?}"
    );
    let started = Instant::now();
    let mut counts: Vec<(Duration, usize, u64)> = Vec::new();
    let found = std::thread::scope(|s| {
        let walk = s.spawn(|| discover(&roots, &p, &HashSet::new(), &none_known(), &progress, 8));
        while !walk.is_finished() {
            let snap = progress.snapshot();
            assert_eq!(snap.phase, ImportPhase::Finding);
            counts.push((started.elapsed(), snap.done, snap.bytes_total));
            std::thread::sleep(Duration::from_millis(1));
        }
        walk.join().unwrap()
    });
    assert_eq!(found.len(), 20_000);
    let last = progress.snapshot();
    assert_eq!((last.done, last.bytes_total), (20_000, 20_000 * 27));
    assert!(counts.windows(2).all(|w| w[0].1 <= w[1].1 && w[0].2 <= w[1].2), "counts went down");
    assert!(last.current.is_some(), "the folder being listed is named");
}

#[test]
fn cancelling_while_finding_stops_the_walk_promptly() {
    let d = TempDir::new("cancel");
    tree(&d.0, 20_000);
    let p = profile();
    let roots = vec![d.0.clone()];
    // Cancelled before the first folder: nothing is listed.
    let progress = ImportProgress::default();
    let _run = progress.begin();
    progress.cancel();
    let t = Instant::now();
    assert!(discover(&roots, &p, &HashSet::new(), &none_known(), &progress, 8).is_empty());
    assert!(t.elapsed() < Duration::from_secs(2));
    // Cancelled once files were found: the walk ends with fewer than all of them.
    let progress = ImportProgress::default();
    let _run = progress.begin();
    let found = std::thread::scope(|s| {
        let walk = s.spawn(|| discover(&roots, &p, &HashSet::new(), &none_known(), &progress, 2));
        while progress.snapshot().done == 0 && !walk.is_finished() {
            std::thread::yield_now();
        }
        progress.cancel();
        walk.join().unwrap()
    });
    assert!(found.len() < 20_000, "{} files after the cancel", found.len());
}

#[test]
fn begin_keeps_a_cancel_asked_for_in_a_later_phase_and_ends_with_its_guard() {
    let progress = ImportProgress::default();
    {
        let _run = progress.begin();
        assert!(progress.snapshot().running);
        progress.cancel();
        assert!(progress.snapshot().cancelled);
    }
    let after = progress.snapshot();
    assert!(
        !after.running && after.phase == ImportPhase::Idle,
        "an early return ends the import: {after:?}"
    );
}
