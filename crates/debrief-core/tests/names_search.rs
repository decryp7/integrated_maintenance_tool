//! The name layer over a real store: `Names::refresh` joins look-alike spellings that appear in the same events,
//! `Names::search` returns the new fields (group, spellings, why, evidence) and a clarifying question on a tie, and
//! feedback (accept, reject, undo) changes the next answer.

use debrief_core::names::Names;
use debrief_core::ports::Store;
use debrief_core::workspace::WorkspaceStore;
use debrief_core::{Profile, Workspace};
use std::sync::atomic::{AtomicUsize, Ordering};

fn store(rows: &[&str]) -> WorkspaceStore {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "debrief-core-names-search-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let text: String = rows
        .iter()
        .enumerate()
        .map(|(i, m)| format!("{{\"@t\":\"2026-10-14T09:{:02}:00.000Z\",\"@m\":\"{m}\"}}\n", i % 60))
        .collect();
    std::fs::write(dir.join("host.log"), text).unwrap();
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(std::slice::from_ref(&dir)).unwrap();
    WorkspaceStore::new(ws)
}

#[test]
fn look_alike_spellings_in_the_same_events_become_one_entity() {
    let together = store(&[
        "zoxcalc.exe started zoxcalculator.exe",
        "zoxcalc.exe stopped zoxcalculator.exe",
        "zoxcalc.exe crashed zoxcalculator.exe",
    ]);
    let names = Names::default();
    let s: &dyn Store = &together;
    assert_eq!(names.refresh(s).unwrap(), 1, "they are always seen together");
    assert_eq!(names.read(|b, _| b.groups()[0].spellings.len()), 2);

    let apart = store(&[
        "zoxcalc.exe started",
        "zoxcalculator.exe started",
        "zoxcalc.exe stopped",
        "zoxcalculator.exe stopped",
    ]);
    let names = Names::default();
    let s: &dyn Store = &apart;
    assert_eq!(names.refresh(s).unwrap(), 2, "never in one event: two things");
}

#[test]
fn search_explains_every_match_and_remembers_what_the_user_said() {
    let s = store(&[
        "Started CompanyA.Graphic.Builder.exe pid 4242",
        "Render queue drained by graphicbuilder.exe",
        "Loaded graphicbuilder.exe again",
    ]);
    let names = Names::default();
    let st: &dyn Store = &s;
    names.refresh(st).unwrap();
    let r = names.search(st, "grafic bilder", 3).unwrap();
    let top = &r.values[0];
    assert_eq!(top.group.as_deref(), Some("executable|graphic builder"));
    assert!(!top.why.is_empty() && !top.evidence.is_empty() && top.spellings.len() >= 2);
    assert!(r.clarify.is_none());
    assert_eq!(names.context().recent, ["grafic bilder"], "the search is remembered as context");

    // "gfx" means nothing yet; the user says it means the group, and it outranks everything after that.
    assert!(names.search(st, "gfx", 3).unwrap().values.is_empty());
    names.write(|b| b.accept("gfx", "executable|graphic builder", false));
    let r = names.search(st, "gfx", 3).unwrap();
    assert_eq!(serde_json::to_value(&r.values[0]).unwrap()["via"], "userAlias");
    names.write(|b| b.undo());
    assert!(names.search(st, "gfx", 3).unwrap().values.is_empty());
}

#[test]
fn two_things_that_match_equally_produce_a_question() {
    let s = store(&[
        "Started report-builder.exe",
        "Started reportbuilder.exe",
        "Started image-builder.exe",
        "Started imagebuilder.exe",
    ]);
    let names = Names::default();
    let st: &dyn Store = &s;
    names.refresh(st).unwrap();
    let r = names.search(st, "builder", 5).unwrap();
    let q = r.clarify.clone().expect("a tie");
    assert!(q.question.contains('?') && q.options.len() >= 2, "{q:?}");
    let json = serde_json::to_value(&r).unwrap();
    assert!(json["clarify"]["question"].is_string() && json["values"].is_array());
}
