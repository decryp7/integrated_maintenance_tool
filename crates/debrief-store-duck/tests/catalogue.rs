//! The persistent catalogue: names found only in event text are resolved, kept up to date by imports, removals and
//! clears, and looked up with typos.

mod common;

use common::TempDir;
use debrief_core::catalogue::{CatalogueRequest, Scope};
use debrief_core::entities::EntityKind;
use debrief_core::find::{find, resolve_terms, Via, ENTITY_FIELD};
use debrief_core::ports::{Ingest, Query};
use debrief_core::{PluginRegistry, Profile};
use debrief_store_duck::DuckStore;
use std::path::Path;
use std::time::Duration;

fn lines(path: &Path, rows: &[&str]) {
    let mut text = String::new();
    for (i, r) in rows.iter().enumerate() {
        text.push_str(&format!("2026-10-14 10:00:{:02} INFO {r}\n", i % 60));
    }
    std::fs::write(path, text).unwrap();
}

fn built(store: &DuckStore) {
    assert!(store.wait_for_catalogue(Duration::from_secs(60)).unwrap(), "catalogue built");
}

fn lookup(store: &DuckStore, text: &str) -> Vec<(Scope, String, u64)> {
    store
        .catalogue(&CatalogueRequest { text: text.into() })
        .unwrap()
        .candidates
        .into_iter()
        .map(|c| (c.scope, c.name, c.count))
        .collect()
}

fn setup(name: &str) -> (TempDir, TempDir, DuckStore) {
    let logs = TempDir::new(&format!("cat-logs-{name}"));
    let dir = TempDir::new(&format!("cat-store-{name}"));
    let store = DuckStore::create(&dir.0, Profile::default_profile(), PluginRegistry::new()).unwrap();
    (logs, dir, store)
}

#[test]
fn a_name_only_in_message_text_is_resolved_with_typos_and_kind() {
    let (logs, _dir, store) = setup("text");
    lines(
        &logs.0.join("a.log"),
        &[
            "Started CompanyA.Graphic.Builder.exe from C:\\Tools\\Acme\\Graphic.Builder.exe",
            "CompanyA.Graphic.Builder.exe crashed with System.TimeoutException, HRESULT 0x80070005",
            "connect to api.example.com failed for 10.1.2.3",
            "heartbeat",
        ],
    );
    store.open(std::slice::from_ref(&logs.0)).unwrap().wait().unwrap();
    built(&store);

    for text in ["graphic builder", "grafic bilder", "GraphicBuilder"] {
        let v = resolve_terms(&store, text, 5).unwrap();
        let top = v
            .iter()
            .find(|m| m.value == "CompanyA.Graphic.Builder.exe")
            .unwrap_or_else(|| panic!("{text}: {v:#?}"));
        assert_eq!(
            (top.field.as_str(), top.kind, top.count),
            (ENTITY_FIELD, Some(EntityKind::Executable), 2),
            "{text}"
        );
        assert_eq!(top.seen_in, ["a.log"]);
        assert!(top.first_seen.is_some() && top.last_seen >= top.first_seen);
    }
    let typo = resolve_terms(&store, "grafic bilder", 5).unwrap();
    assert_eq!(typo[0].via, Via::Fuzzy);
    let r = find(&store, "timeoutexception", 0, 5).unwrap();
    assert!(r.total >= 1, "{r:#?}");
    let hosts = lookup(&store, "api example");
    assert!(
        hosts
            .iter()
            .any(|(s, n, c)| *s == Scope::Entity(EntityKind::Host) && n == "api.example.com" && *c == 1),
        "{hosts:?}"
    );
    assert!(lookup(&store, "0x80070005")
        .iter()
        .any(|(s, ..)| *s == Scope::Entity(EntityKind::ErrorCode)));
    assert!(lookup(&store, "10.1.2.3").iter().any(|(s, ..)| *s == Scope::Entity(EntityKind::Ip)));
    assert!(lookup(&store, "zz").is_empty() && lookup(&store, "  ").is_empty());
    assert!(!lookup(&store, "Co").is_empty(), "a short text matches name starts");
}

#[test]
fn the_catalogue_follows_imports_removals_and_clear() {
    let (logs, _dir, store) = setup("life");
    lines(&logs.0.join("a.log"), &["ran alpha.exe", "ran alpha.exe"]);
    let first = store.open(std::slice::from_ref(&logs.0)).unwrap().wait().unwrap();
    built(&store);
    assert_eq!(lookup(&store, "alpha")[0].2, 2);

    // A second file adds to the counts (incremental) and a new name appears.
    lines(&logs.0.join("b.log"), &["ran alpha.exe", "ran beta.dll"]);
    store.open(std::slice::from_ref(&logs.0)).unwrap().wait().unwrap();
    built(&store);
    assert_eq!(lookup(&store, "alpha")[0].2, 3);
    assert_eq!(lookup(&store, "beta")[0].1, "beta.dll");

    // Removing a source takes its names with it, at once.
    let id = store.summary().unwrap().sources.iter().find(|s| s.name == "b.log").unwrap().id;
    store.remove_source(debrief_core::batch::SourceId(id)).unwrap();
    assert_eq!(lookup(&store, "alpha")[0].2, 2);
    assert!(lookup(&store, "beta").is_empty());

    store.clear().unwrap();
    assert!(lookup(&store, "alpha").is_empty());
    let reply = store.catalogue(&CatalogueRequest { text: "alpha".into() }).unwrap();
    assert!(reply.built, "nothing to catalogue is built");

    // Reload (a re-import) keeps the names consistent: no doubled counts.
    lines(&logs.0.join("a.log"), &["ran alpha.exe", "ran alpha.exe"]);
    std::fs::remove_file(logs.0.join("b.log")).unwrap();
    store.open(std::slice::from_ref(&logs.0)).unwrap().wait().unwrap();
    built(&store);
    store.reload().unwrap().wait().unwrap();
    built(&store);
    assert_eq!(lookup(&store, "alpha")[0].2, 2);
    drop(first);
}

#[test]
fn field_values_are_catalogued_and_numbers_and_ids_are_not() {
    let (logs, _dir, store) = setup("fields");
    let mut text = String::new();
    for i in 0..30 {
        text.push_str(&format!(
            "{{\"@t\":\"2026-10-14T10:00:{i:02}Z\",\"@mt\":\"event {i}\",\"Service\":\"billing-api\",\"Count\":\"{i}\"}}\n"
        ));
    }
    std::fs::write(logs.0.join("a.clef"), text).unwrap();
    store.open(std::slice::from_ref(&logs.0)).unwrap().wait().unwrap();
    built(&store);
    let v = lookup(&store, "billing api");
    assert!(
        v.iter()
            .any(|(s, n, c)| *s == Scope::Field("Service".into()) && n == "billing-api" && *c == 30),
        "{v:?}"
    );
    let r = resolve_terms(&store, "billing api", 3).unwrap();
    assert_eq!((r[0].field.as_str(), r[0].kind), ("Service", None));
    assert!(lookup(&store, "event")
        .iter()
        .all(|(s, ..)| !matches!(s, Scope::Field(f) if f == "Count")));
}

#[test]
fn an_unbuilt_catalogue_falls_back_to_facets_and_reports_pending() {
    let (_logs, _dir, store) = setup("empty");
    let reply = store.catalogue(&CatalogueRequest { text: "x".into() }).unwrap();
    assert!(reply.built && reply.candidates.is_empty() && reply.pending == 0);
}
