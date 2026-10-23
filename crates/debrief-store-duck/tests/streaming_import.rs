//! The streaming import (ADR 0050) numbers sources and orders events exactly like reading a complete sorted list, at
//! any number of workers and in any arrival order, and reloads keep the ids of sources that were renamed or moved.

mod common;

use common::TempDir;
use debrief_core::ports::{Ingest, Query};
use debrief_core::query::Query as QueryDef;
use debrief_core::{PluginRegistry, Profile};
use debrief_store_duck::{DuckStore, StoreOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

fn line(file: usize, n: usize) -> String {
    // The same timestamp everywhere: only the source order and the line number tell the events apart.
    format!("2026-10-14 10:00:00 INFO file{file} line{n}\n")
}

fn tree(root: &Path, files: usize) {
    for i in 0..files {
        let dir = root.join(format!("d{}/e{}", i % 7, (i / 7) % 5));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("f{i:03}.log")), (0..4).map(|n| line(i, n)).collect::<String>()).unwrap();
    }
    std::fs::write(root.join("zz-big.log"), (0..30_000).map(|n| line(9999, n)).collect::<String>()).unwrap();
    for i in 0..2 {
        let f = std::fs::File::create(root.join(format!("arch{i}.zip"))).unwrap();
        let mut z = zip::ZipWriter::new(f);
        for j in 0..3 {
            z.start_file(format!("e{j}.log"), zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all((0..3).map(|n| line(5000 + i * 10 + j, n)).collect::<String>().as_bytes())
                .unwrap();
        }
        z.finish().unwrap();
    }
}

fn store(dir: &TempDir, threads: usize) -> DuckStore {
    let opts = StoreOptions {
        parse_threads: threads,
        ..StoreOptions::default()
    };
    DuckStore::create_with(&dir.0, Profile::default_profile(), PluginRegistry::new(), opts).unwrap()
}

fn open(s: &DuckStore, root: &Path) {
    s.open(&[root.to_path_buf()]).unwrap().wait().unwrap();
}

/// What a user sees, without anything that depends on the run: sources with ids, and every event in default order.
type Observed = (Vec<(u32, String, usize)>, Vec<(u32, u32, String)>);

fn observed(s: &DuckStore, root: &Path) -> Observed {
    let root = std::fs::canonicalize(root).unwrap();
    let rel = |p: &str| p.strip_prefix(root.to_string_lossy().as_ref()).unwrap_or(p).to_string();
    let mut sources: Vec<_> = s
        .summary()
        .unwrap()
        .sources
        .iter()
        .map(|x| (x.id, rel(&x.path), x.records))
        .collect();
    sources.sort();
    let page = s
        .page(&debrief_core::ports::PageQuery {
            query: QueryDef::default(),
            offset: 0,
            limit: 500,
            descending: false,
            anchor: None,
        })
        .unwrap();
    let total = page.total;
    let mut events = Vec::new();
    let mut offset = 0;
    while offset < total.min(2_000) {
        let page = s
            .page(&debrief_core::ports::PageQuery {
                query: QueryDef::default(),
                offset,
                limit: 500,
                descending: false,
                anchor: None,
            })
            .unwrap();
        events.extend(page.rows.iter().map(|r| (r.source.0, r.line, r.message.clone())));
        offset += 500;
    }
    (sources, events)
}

#[test]
fn ids_positions_and_results_do_not_depend_on_workers_or_arrival_order() {
    let data = TempDir::new("stream-data");
    tree(&data.0, 60);
    let mut runs = Vec::new();
    for (threads, jitter) in [(1, 0), (2, 0), (8, 0), (16, 0), (8, 1), (8, 7), (3, 99), (16, 12345)] {
        let dir = TempDir::new("stream-store");
        let s = store(&dir, threads);
        s.set_import_jitter(jitter);
        open(&s, &data.0);
        runs.push(((threads, jitter), observed(&s, &data.0)));
    }
    let (first_label, first) = &runs[0];
    assert!(first.0.len() > 60 + 6, "{} sources", first.0.len());
    // Ids follow the sorted order of the paths, without gaps.
    let by_path: Vec<_> = first.0.iter().map(|x| x.1.clone()).collect();
    assert_eq!(
        first.0.iter().map(|x| x.0).collect::<Vec<_>>(),
        (0..first.0.len() as u32).collect::<Vec<_>>()
    );
    let mut sorted = by_path.clone();
    sorted.sort_by(|a, b| Path::new(a).cmp(Path::new(b)));
    assert_eq!(by_path, sorted, "ids are in path order");
    for (label, run) in &runs[1..] {
        assert_eq!(run, first, "{label:?} differs from {first_label:?}");
    }
}

fn ids(s: &DuckStore) -> Vec<(u32, String)> {
    s.source_ids().unwrap()
}

fn reload(s: &DuckStore) -> debrief_core::ports::ImportReport {
    s.reload().unwrap().wait().unwrap()
}

fn small_tree(root: &Path) {
    std::fs::create_dir_all(root.join("sub")).unwrap();
    for (name, n) in [("a.log", 1), ("b.log", 2), ("sub/c.log", 3), ("sub/d.log", 4)] {
        std::fs::write(root.join(name), (0..5).map(|i| line(n * 10 + i, i)).collect::<String>()).unwrap();
    }
}

#[test]
fn a_renamed_file_keeps_its_id_after_a_reload() {
    let data = TempDir::new("reuse-rename");
    small_tree(&data.0);
    let dir = TempDir::new("reuse-rename-store");
    let s = store(&dir, 2);
    open(&s, &data.0);
    let before = ids(&s);
    let root = std::fs::canonicalize(&data.0).unwrap();
    std::fs::rename(root.join("b.log"), root.join("sub/renamed-b.log")).unwrap();
    let report = reload(&s);
    assert!(report.notices.is_empty(), "{:?}", report.notices);
    let after = ids(&s);
    assert_eq!(after.len(), before.len());
    let id_of = |v: &[(u32, String)], suffix: &str| v.iter().find(|x| x.1.ends_with(suffix)).map(|x| x.0);
    assert_eq!(id_of(&before, "/b.log"), id_of(&after, "/renamed-b.log"), "{before:?} -> {after:?}");
    for keep in ["/a.log", "/c.log", "/d.log"] {
        assert_eq!(id_of(&before, keep), id_of(&after, keep), "{keep}");
    }
}

#[test]
fn deleting_one_and_adding_one_leaves_the_other_ids_and_appends_the_new_one() {
    let data = TempDir::new("reuse-delete");
    small_tree(&data.0);
    let dir = TempDir::new("reuse-delete-store");
    let s = store(&dir, 4);
    open(&s, &data.0);
    let before = ids(&s);
    let max = before.iter().map(|x| x.0).max().unwrap();
    let root = std::fs::canonicalize(&data.0).unwrap();
    std::fs::remove_file(root.join("a.log")).unwrap();
    // A new file that sorts before every old one still gets an id after the highest used.
    std::fs::write(root.join("0-new.log"), line(777, 1)).unwrap();
    reload(&s);
    let after = ids(&s);
    for (id, path) in &before {
        if !path.ends_with("/a.log") {
            assert!(after.contains(&(*id, path.clone())), "{path} kept its id: {after:?}");
        }
    }
    let new = after.iter().find(|x| x.1.ends_with("/0-new.log")).unwrap();
    assert!(new.0 > max, "{new:?} is after {max}");
    assert!(!after.iter().any(|x| x.1.ends_with("/a.log")));
}

#[test]
fn identical_content_in_two_files_is_never_merged_by_guessing() {
    let data = TempDir::new("reuse-dup");
    small_tree(&data.0);
    let dir = TempDir::new("reuse-dup-store");
    let s = store(&dir, 2);
    open(&s, &data.0);
    let before = ids(&s);
    let root = std::fs::canonicalize(&data.0).unwrap();
    // b.log vanishes and two copies of it appear: either could be "the" renamed file.
    let content = std::fs::read(root.join("b.log")).unwrap();
    std::fs::remove_file(root.join("b.log")).unwrap();
    std::fs::write(root.join("copy1.log"), &content).unwrap();
    std::fs::write(root.join("copy2.log"), &content).unwrap();
    let report = reload(&s);
    assert_eq!(report.notices.len(), 1, "{:?}", report.notices);
    assert!(report.notices[0].contains("ambiguous"));
    let after = ids(&s);
    let old_b = before.iter().find(|x| x.1.ends_with("/b.log")).unwrap().0;
    assert!(
        !after
            .iter()
            .any(|x| x.0 == old_b && (x.1.ends_with("copy1.log") || x.1.ends_with("copy2.log"))),
        "no copy took the old id: {after:?}"
    );
    assert_eq!(after.len(), before.len() + 1);
}

#[test]
fn moving_the_whole_folder_gives_the_same_ids() {
    let a = TempDir::new("reuse-move-a");
    small_tree(&a.0);
    let b = TempDir::new("reuse-move-b");
    let dir_a = TempDir::new("reuse-move-store-a");
    let sa = store(&dir_a, 2);
    open(&sa, &a.0);
    // The same files in another folder (the first one moved): a fresh import numbers them identically.
    let moved: PathBuf = b.0.join("moved-here");
    std::fs::create_dir_all(&moved).unwrap();
    small_tree(&moved);
    let dir_b = TempDir::new("reuse-move-store-b");
    let sb = store(&dir_b, 8);
    open(&sb, &moved);
    let rel = |v: Vec<(u32, String)>, root: &Path| {
        let root = std::fs::canonicalize(root).unwrap().to_string_lossy().into_owned();
        v.into_iter()
            .map(|(i, p)| (i, p.strip_prefix(&root).unwrap().to_string()))
            .collect::<Vec<_>>()
    };
    assert_eq!(rel(ids(&sa), &a.0), rel(ids(&sb), &moved));
}

#[test]
fn a_reload_of_untouched_folders_keeps_every_id() {
    let data = TempDir::new("reuse-same");
    small_tree(&data.0);
    let dir = TempDir::new("reuse-same-store");
    let s = store(&dir, 4);
    open(&s, &data.0);
    let before = ids(&s);
    reload(&s);
    assert_eq!(ids(&s), before);
}
