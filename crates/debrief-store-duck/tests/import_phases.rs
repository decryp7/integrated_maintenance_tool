//! A reload that is cancelled keeps the previous store.

mod common;

use common::{samples, TempDir};
use debrief_core::ports::{Ingest, Query};
use debrief_core::{PluginRegistry, Profile};
use debrief_store_duck::DuckStore;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// A reload that is cancelled leaves the previous store exactly as it was (same events, same files on disk).
#[test]
fn a_cancelled_reload_keeps_the_previous_data() {
    let dir = TempDir::new("reload-cancel");
    let store = DuckStore::create_project(&dir.0.join("p.debrief"), "p", Profile::default_profile(), PluginRegistry::new()).unwrap();
    store.open(&[samples().join("serilog-text")]).unwrap().wait().unwrap();
    let before = store.summary().unwrap().total;
    assert!(before > 0);
    let session = store.reload().unwrap();
    let done = Arc::new(AtomicBool::new(false));
    let (d, p) = (done.clone(), store.progress());
    let spam = std::thread::spawn(move || {
        while !d.load(Ordering::SeqCst) {
            p.cancel();
            std::thread::sleep(std::time::Duration::from_micros(50));
        }
    });
    session.wait().unwrap();
    done.store(true, Ordering::SeqCst);
    spam.join().unwrap();
    assert_eq!(store.summary().unwrap().total, before, "the old data is back");
    // A later reload that is not cancelled replaces it normally.
    store.reload().unwrap().wait().unwrap();
    assert_eq!(store.summary().unwrap().total, before);
}
