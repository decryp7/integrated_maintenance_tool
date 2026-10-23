use super::*;
use crate::workspace::{Workspace, WorkspaceStore};
use crate::Profile;

/// A workspace over one invented CLEF log `host-agent.log` in a fresh folder.
fn store_with(name: &str, events: &[(&str, &str)]) -> WorkspaceStore {
    let dir = std::env::temp_dir().join(format!("debrief-find-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut text = String::new();
    for (i, (msg, process)) in events.iter().enumerate() {
        text.push_str(&format!(
            "{{\"@t\":\"2026-10-14T09:{:02}:00.000Z\",\"@m\":\"{msg}\",\"ProcessName\":\"{process}\"}}\n",
            i % 60
        ));
    }
    std::fs::write(dir.join("host-agent.log"), text).unwrap();
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(std::slice::from_ref(&dir)).unwrap();
    WorkspaceStore::new(ws)
}

fn sample(name: &str) -> WorkspaceStore {
    store_with(
        name,
        &[
            ("Started CompanyA.Graphic.Builder.exe pid 4242", "agent.exe"),
            ("Render queue drained", "CompanyA.Graphic.Builder.exe"),
            ("Render queue drained", "CompanyA.Graphic.Builder.exe"),
            ("Loaded GraphicBuilder plugin", "agent.exe"),
            ("Unrelated heartbeat", "agent.exe"),
            ("graphic_builder cache warmed", "agent.exe"),
        ],
    )
}

#[test]
fn a_phrase_finds_the_name_in_every_spelling() {
    let r = find(&sample("phrase"), "graphic builder", 0, 0).unwrap();
    assert_eq!(r.step, Some(FindStep::Phrase));
    assert_eq!(r.total, 3, "dotted, camelCase and underscored spellings: {r:#?}");
    assert_eq!(r.mention.as_deref(), Some("CompanyA.Graphic.Builder.exe"));
    assert!(r.hits.iter().all(|h| h.matched_in.contains(&"message".to_string())));
    assert!(
        r.note
            .contains("Also in field values: ProcessName = CompanyA.Graphic.Builder.exe (2 events)"),
        "{}",
        r.note
    );
    assert!(r.note.contains("3 events contain 'graphic builder'") && r.note.contains("mentioning CompanyA.Graphic.Builder.exe"));
}

#[test]
fn words_in_any_order_when_the_phrase_misses() {
    let r = find(&sample("order"), "builder graphic", 0, 0).unwrap();
    assert_eq!(r.step, Some(FindStep::Words));
    assert!(
        r.total >= 3 && r.note.starts_with("No exact match for 'builder graphic'"),
        "{}",
        r.note
    );
}

#[test]
fn parts_of_longer_words_when_words_miss() {
    let r = find(&sample("partial"), "raphic uilder", 0, 0).unwrap();
    assert_eq!(r.step, Some(FindStep::Partial), "{r:#?}");
    assert!(r.total >= 3 && r.note.contains("part of longer words"));
}

#[test]
fn field_values_are_found_and_say_where() {
    let s = store_with(
        "field",
        &[("Render queue drained", "CompanyA.Graphic.Builder.exe"), ("Idle", "agent.exe")],
    );
    let r = find(&s, "graphic builder", 0, 0).unwrap();
    assert_eq!(r.total, 1);
    assert_eq!(r.values[0].field, "ProcessName");
    assert_eq!(r.values[0].value, "CompanyA.Graphic.Builder.exe");
    assert_eq!((r.values[0].via, r.values[0].count), (Via::Parts, 1));
    assert_eq!(r.step, Some(FindStep::Value), "the name is only in a field: {r:#?}");
    assert!(
        r.hits[0].matched_in.iter().any(|m| m == "field:ProcessName"),
        "{:?}",
        r.hits[0].matched_in
    );
    assert_eq!(r.mention.as_deref(), Some("CompanyA.Graphic.Builder.exe"));
}

#[test]
fn a_value_only_in_a_field_is_found_by_the_last_step() {
    let s = store_with(
        "valueonly",
        &[("Render queue drained", "CompanyA.Graphic.Builder.exe"), ("Idle", "agent.exe")],
    );
    // The text is not in any message: the value is the only place.
    let r = find(&s, "graphic builder exe", 0, 0).unwrap();
    assert!(!r.note.contains("mentioning"), "{}", r.note);
    assert_eq!(r.step, Some(FindStep::Value), "{r:#?}");
    assert_eq!(r.total, 1);
    assert!(r.note.contains("ProcessName = CompanyA.Graphic.Builder.exe"), "{}", r.note);
}

#[test]
fn typos_resolve_to_values_and_nothing_found_suggests_them() {
    let s = store_with(
        "fuzzy",
        &[("Render queue drained", "CompanyA.Graphic.Builder.exe"), ("Idle", "agent.exe")],
    );
    let v = resolve_terms(&s, "grafic bilder", 5).unwrap();
    assert_eq!(
        (v.len(), v[0].via, v[0].value.as_str()),
        (1, Via::Fuzzy, "CompanyA.Graphic.Builder.exe")
    );
    let r = find(&s, "grafic bilder", 0, 0).unwrap();
    assert_eq!((r.step, r.total), (None, 0));
    assert!(
        r.note.contains("Did you mean CompanyA.Graphic.Builder.exe (ProcessName)?"),
        "{}",
        r.note
    );
    let none = find(&s, "zzzzqqq", 0, 0).unwrap();
    assert!(none.note.contains("not even partly") && none.values.is_empty());
}

#[test]
fn exact_and_prefix_values_rank_before_fuzzy_and_by_count() {
    let s = store_with(
        "rank",
        &[
            ("a", "graphic.builder"),
            ("b", "graphic.builder"),
            ("c", "GraphicBuilder"),
            ("d", "graphic.builders.old"),
            ("e", "other"),
        ],
    );
    let v = resolve_terms(&s, "graphic builder", 10).unwrap();
    let ways: Vec<(&str, Via)> = v.iter().map(|m| (m.value.as_str(), m.via)).collect();
    assert_eq!(ways[0], ("graphic.builder", Via::Exact), "{ways:?}");
    assert!(v.iter().all(|m| m.via != Via::Fuzzy));
    assert!(resolve_terms(&s, "--", 5).unwrap().is_empty());
}

#[test]
fn replies_are_bounded_and_paged() {
    let many: Vec<(String, &str)> = (0..70).map(|i| (format!("graphic builder step {i}"), "agent.exe")).collect();
    let refs: Vec<(&str, &str)> = many.iter().map(|(m, p)| (m.as_str(), *p)).collect();
    let s = store_with("paged", &refs);
    let r = find(&s, "graphic builder", 0, 1000).unwrap();
    assert_eq!((r.total, r.hits.len()), (70, MAX_LIMIT));
    let page2 = find(&s, "graphic builder", 50, 1000).unwrap();
    assert_eq!(page2.hits.len(), 20);
    assert_eq!(find(&s, "graphic builder", 0, 0).unwrap().hits.len(), DEFAULT_LIMIT);
    let long = "word ".repeat(200);
    assert!(find(&s, &long, 0, 0).unwrap().text.chars().count() <= MAX_TEXT_CHARS);
    let empty = find(&s, " -- ", 0, 0).unwrap();
    assert_eq!((empty.step, empty.hits.len()), (None, 0));
    assert!(empty.note.contains("no words"));
}

#[test]
fn a_quoted_text_is_one_phrase() {
    let r = find(&sample("quoted"), "\"graphic builder\"", 0, 0).unwrap();
    assert_eq!((r.step, r.total), (Some(FindStep::Phrase), 3));
}

#[test]
fn a_source_name_is_a_value() {
    let s = store_with("source", &[("hello", "agent.exe")]);
    let v = resolve_terms(&s, "host agent", 5).unwrap();
    let src = v.iter().find(|m| m.field == SOURCE_FIELD).expect("source matched");
    assert!(src.source.is_some() && src.via != Via::Fuzzy);
    let r = find(&s, "host agent log", 0, 0).unwrap();
    assert!(r.step.is_some(), "{r:#?}");
}

#[test]
fn hits_say_exception_and_template_and_empty_values_never_match() {
    let dir = std::env::temp_dir().join(format!("debrief-find-{}-exc", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let line = "{\"@t\":\"2026-10-14T09:00:00.000Z\",\"@mt\":\"Crash in {Name}\",\"Name\":\"x\",\"@m\":\"Crash in x\",\"@x\":\"System.InvalidOperationException: Graphic.Builder died\"}\n";
    std::fs::write(dir.join("a.log"), line).unwrap();
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(std::slice::from_ref(&dir)).unwrap();
    let s = WorkspaceStore::new(ws);
    let r = find(&s, "graphic builder", 0, 0).unwrap();
    assert_eq!(r.hits[0].matched_in, ["exception"], "{r:#?}");
    let r = find(&s, "crash name", 0, 0).unwrap();
    assert!(r.hits[0].matched_in.contains(&"template".to_string()), "{r:#?}");
    let w = WordQuery::parse("a");
    assert_eq!(via("a", &w, &trigrams("a"), "--"), None);
    assert_eq!(mention("no match here", &["zzz".to_string()]), None);
    assert_eq!(mention("x", &[]), None);
}
