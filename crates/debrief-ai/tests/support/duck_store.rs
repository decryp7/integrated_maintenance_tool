//! A real DuckDB store over invented events, for the tests that need the SQL console or the full chat loop.
//! Nothing here is taken from a real log.
#![allow(dead_code)]

use debrief_core::ports::{Ingest, Store};
use debrief_core::{PluginRegistry, Profile};
use debrief_store_duck::DuckStore;

pub struct Fixture {
    pub store: DuckStore,
    _dir: tempfile::TempDir,
}

impl Fixture {
    pub fn store(&self) -> &dyn Store {
        &self.store
    }
}

/// `(timestamp, level, message, ProcessName)` rows as a Serilog-compact (CLEF) file named `name`.
pub fn clef(rows: &[(&str, &str, &str, &str)]) -> String {
    rows.iter()
        .map(|(t, l, m, p)| format!("{{\"@t\":\"{t}\",\"@l\":\"{l}\",\"@m\":\"{m}\",\"ProcessName\":\"{p}\"}}\n"))
        .collect()
}

/// A store holding the files `(name, text)`.
pub fn open(files: &[(&str, String)]) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let logs = dir.path().join("logs");
    std::fs::create_dir_all(&logs).unwrap();
    for (name, text) in files {
        std::fs::write(logs.join(name), text).unwrap();
    }
    let store = DuckStore::create(&dir.path().join("store"), Profile::default_profile(), PluginRegistry::new()).unwrap();
    store.open(std::slice::from_ref(&logs)).unwrap().wait().unwrap();
    Fixture { store, _dir: dir }
}
