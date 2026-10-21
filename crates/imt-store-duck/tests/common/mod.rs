//! Shared by the integration tests: the sample files and a temporary store folder.

#![allow(dead_code)]

pub mod corpus;
pub mod rss;

use imt_core::ports::{Filter, Ingest, Query, StoreResult};
use imt_core::Workspace;
use imt_core::{PluginRegistry, Profile};
use imt_store_duck::DuckStore;
use std::collections::HashMap;
use std::path::PathBuf;

pub fn samples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples")
}

/// Folders of `/samples` that are not logs, or that only plugins read (as in imt-core's tests).
const SKIP: &[&str] = &["profiling", "configs", "share", "live", "windows", "web", "linux", "otel"];

/// Every entry of `/samples` the built-in importers open.
pub fn log_samples() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(samples())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir() && !p.file_name().is_some_and(|n| SKIP.iter().any(|x| n == *x)))
        .collect();
    paths.sort();
    paths
}

pub fn sample_profile() -> Profile {
    let text = std::fs::read_to_string(samples().join("profiles/orderservice.imtprofile.toml")).unwrap();
    Profile::from_toml(&text).unwrap()
}

/// A fresh folder under the system temp folder, removed on drop.
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> TempDir {
        let mut b = [0u8; 8];
        getrandom::fill(&mut b).unwrap();
        let p = std::env::temp_dir().join(format!("imt-duck-{tag}-{}-{:x}", std::process::id(), u64::from_le_bytes(b)));
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A store in `dir` with `paths` imported (`IMT_PARSE_THREADS` sets the parse threads).
pub fn store_with(dir: &TempDir, profile: Profile, paths: &[PathBuf]) -> StoreResult<(DuckStore, imt_core::ports::ImportReport)> {
    let mut opts = imt_store_duck::StoreOptions::default();
    if let Some(n) = std::env::var("IMT_PARSE_THREADS").ok().and_then(|v| v.parse().ok()) {
        opts.parse_threads = n;
    }
    let store = DuckStore::create_with(&dir.0, profile, PluginRegistry::new(), opts)?;
    let report = store.open(paths)?.wait()?;
    Ok((store, report))
}

pub struct Pair {
    pub ws: Workspace,
    pub duck: DuckStore,
    _dir: TempDir,
    /// Source id → path, per store.
    pub ws_paths: HashMap<u32, String>,
    pub duck_paths: HashMap<u32, String>,
    /// The time range of all timed events.
    pub range: (i64, i64),
}

pub fn pair(profile: Profile) -> Pair {
    let paths = log_samples();
    let mut ws = Workspace::new(profile.clone()).unwrap();
    ws.open(&paths).unwrap();
    let dir = TempDir::new("diff");
    let (duck, report) = store_with(&dir, profile, &paths).unwrap();
    assert!(report.events > 10_000, "{} events", report.events);
    // Smart search answers from the index once it caught up (before, it matches substrings of the words).
    assert!(duck.wait_for_index(std::time::Duration::from_secs(120)).unwrap().ready);
    let ws_paths = ws.sources().iter().map(|s| (s.id, s.path.clone())).collect();
    let summary = duck.summary().unwrap();
    let duck_paths = summary.sources.iter().map(|s| (s.id, s.path.clone())).collect();
    let s = ws.summary();
    let range = (s.from.unwrap(), s.to.unwrap());
    Pair {
        ws,
        duck,
        _dir: dir,
        ws_paths,
        duck_paths,
        range,
    }
}

pub fn ranged(p: &Pair, q: Filter) -> Filter {
    Filter {
        from: q.from.or(Some(p.range.0)),
        to: q.to.or(Some(p.range.1)),
        ..q
    }
}
