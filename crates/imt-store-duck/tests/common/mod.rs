//! Shared by the integration tests: the sample files and a temporary store folder.

#![allow(dead_code)]

use imt_core::ports::{Ingest, StoreResult};
use imt_core::{PluginRegistry, Profile};
use imt_store_duck::DuckStore;
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

/// A store in `dir` with the samples imported.
pub fn store_with(dir: &TempDir, profile: Profile, paths: &[PathBuf]) -> StoreResult<(DuckStore, imt_core::ports::ImportReport)> {
    let store = DuckStore::create(&dir.0, profile, PluginRegistry::new())?;
    let report = store.open(paths)?.wait()?;
    Ok((store, report))
}
