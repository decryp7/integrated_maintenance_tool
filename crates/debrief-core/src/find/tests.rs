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

#[test]
fn a_name_only_in_message_text_is_resolved_with_its_kind() {
    let s = store_with("entity", &[("Started Acme.Render.Worker.exe", "agent"), ("idle", "agent")]);
    for text in ["render worker", "rendr wrker", "RenderWorker"] {
        let v = resolve_terms(&s, text, 5).unwrap();
        let m = v
            .iter()
            .find(|m| m.value == "Acme.Render.Worker.exe")
            .unwrap_or_else(|| panic!("{text}: {v:#?}"));
        assert_eq!(
            (m.field.as_str(), m.kind),
            (ENTITY_FIELD, Some(crate::entities::EntityKind::Executable)),
            "{text}"
        );
        assert_eq!(m.seen_in, ["host-agent.log"]);
    }
    // The Value step finds the events of an entity by its text.
    let r = find(&s, "rendr wrker", 0, 5).unwrap();
    assert!(r.note.contains("Did you mean Acme.Render.Worker.exe (executable)"), "{}", r.note);
    let m = &resolve_terms(&s, "render worker", 1).unwrap()[0];
    assert_eq!(
        (m.label().as_str(), m.describe().as_str()),
        ("executable", "executable Acme.Render.Worker.exe")
    );
}

/// A store whose catalogue is not built yet: everything else is the workspace's.
struct Unbuilt(WorkspaceStore);

impl SearchSource for Unbuilt {
    fn summary(&self) -> StoreResult<crate::ports::SummaryReply> {
        SearchSource::summary(&self.0)
    }
    fn page(&self, req: &PageQuery) -> StoreResult<crate::ports::PageReply> {
        SearchSource::page(&self.0, req)
    }
    fn facet(&self, req: &FacetRequest) -> StoreResult<crate::api::Facet> {
        SearchSource::facet(&self.0, req)
    }
    fn exception_groups(&self) -> StoreResult<Vec<crate::exceptions::ExceptionGroup>> {
        SearchSource::exception_groups(&self.0)
    }
    fn catalogue(&self, _: &CatalogueRequest) -> StoreResult<crate::catalogue::CatalogueReply> {
        Ok(crate::catalogue::CatalogueReply {
            built: false,
            pending: 1,
            candidates: Vec::new(),
        })
    }
}

#[test]
fn until_the_catalogue_is_built_values_come_from_facets() {
    let s = Unbuilt(store_with(
        "unbuilt",
        &[("Started Acme.Render.Worker.exe", "Acme.Render.Worker.exe"), ("idle", "agent")],
    ));
    let v = resolve_terms(&s, "render worker", 5).unwrap();
    assert_eq!(v.len(), 1, "{v:#?}");
    assert_eq!(
        (v[0].field.as_str(), v[0].kind),
        ("ProcessName", None),
        "the names in message text need the catalogue"
    );
}

/// A store whose catalogue still lists a name whose events are gone: nothing matches any text, the catalogue knows the name.
struct Stale(WorkspaceStore, Scope);

impl SearchSource for Stale {
    fn summary(&self) -> StoreResult<crate::ports::SummaryReply> {
        SearchSource::summary(&self.0)
    }
    fn page(&self, req: &PageQuery) -> StoreResult<crate::ports::PageReply> {
        let mut p = SearchSource::page(&self.0, req)?;
        if req.query.text.is_empty() && req.query.filters.is_empty() && req.query.sources.is_empty() && req.query.ql.is_empty() {
            return Ok(p);
        }
        p.total = 0;
        p.rows.clear();
        Ok(p)
    }
    fn facet(&self, req: &FacetRequest) -> StoreResult<crate::api::Facet> {
        SearchSource::facet(&self.0, req)
    }
    fn exception_groups(&self) -> StoreResult<Vec<crate::exceptions::ExceptionGroup>> {
        SearchSource::exception_groups(&self.0)
    }
    fn catalogue(&self, _: &CatalogueRequest) -> StoreResult<crate::catalogue::CatalogueReply> {
        Ok(crate::catalogue::CatalogueReply {
            built: true,
            pending: 0,
            candidates: vec![crate::catalogue::Candidate {
                scope: self.1.clone(),
                name: "Gone.Worker.exe".into(),
                count: 3,
                first_ts: None,
                last_ts: None,
                seen_in: Vec::new(),
            }],
        })
    }
}

#[test]
fn a_name_the_catalogue_remembers_is_searched_by_its_text_when_the_words_find_nothing() {
    let store = store_with("stale", &[("idle", "agent")]);
    // The Value step finds nothing here either (the stub has no events): the reply suggests the name.
    let r = find(
        &Stale(store, Scope::Entity(crate::entities::EntityKind::Executable)),
        "Gone.Worker.exe",
        0,
        5,
    )
    .unwrap();
    assert_eq!(r.step, None);
    assert!(r.note.contains("Did you mean Gone.Worker.exe (executable)"), "{}", r.note);
    assert_eq!(r.values[0].value, "Gone.Worker.exe");
}
