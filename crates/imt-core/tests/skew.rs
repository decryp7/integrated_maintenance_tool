//! Clock-skew correction: manual offsets, aligning on an event, suggestions
//! from shared correlation ids (clean and noisy), every view shifted, ids
//! stable before the first moved record, live tail and rotation, reload,
//! clear and the project round trip.

use imt_core::api::{ChartRequest, FacetRequest, PageRequest, QueryRequest, TimelineRequest};
use imt_core::project::ProjectFile;
use imt_core::query::Query;
use imt_core::skew::{OffsetSpec, SetOffsetRequest, SkewError, SuggestRequest, MAX_OFFSET_MS};
use imt_core::workspace::WorkspaceState;
use imt_core::{LogStore, Profile, Workspace};
use std::io::Write;
use std::path::{Path, PathBuf};

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        let p = std::env::temp_dir().join(format!("imt-skew-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Dir(std::fs::canonicalize(p).unwrap())
    }
    fn write(&self, name: &str, text: &str) -> PathBuf {
        let p = self.0.join(name);
        std::fs::write(&p, text).unwrap();
        p
    }
    fn append(&self, name: &str, text: &str) -> PathBuf {
        let p = self.0.join(name);
        let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        f.write_all(text.as_bytes()).unwrap();
        p
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const BASE: i64 = 1_791_972_000_000; // 2026-10-14T10:00:00Z

/// One CLEF line at `ms` after BASE.
fn clef(ms: i64, msg: &str, corr: Option<&str>) -> String {
    let t = chrono::DateTime::from_timestamp_millis(BASE + ms)
        .unwrap()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ");
    match corr {
        Some(c) => format!("{{\"@t\":\"{t}\",\"@m\":\"{msg}\",\"CorrelationId\":\"{c}\"}}\n"),
        None => format!("{{\"@t\":\"{t}\",\"@m\":\"{msg}\"}}\n"),
    }
}

/// Server A logs request i at i·1 s; server B sees it `lag` ms later on its
/// own clock, which runs `skew` ms behind A (B's timestamps are low by `skew`).
fn two_servers(d: &Dir, n: i64, skew: i64) -> (PathBuf, PathBuf) {
    let a: String = (0..n)
        .map(|i| clef(i * 1000, &format!("A got {i}"), Some(&format!("c{i}"))))
        .collect();
    let b: String = (0..n)
        .map(|i| clef(i * 1000 + 20 - skew, &format!("B got {i}"), Some(&format!("c{i}"))))
        .collect();
    (d.write("a.log", &a), d.write("b.log", &b))
}

fn source_id(ws: &Workspace, name: &str) -> u32 {
    ws.summary().sources.iter().find(|s| s.name.ends_with(name)).unwrap().id
}

fn manual(source: u32, offset_ms: i64) -> SetOffsetRequest {
    SetOffsetRequest {
        spec: OffsetSpec::Manual { source, offset_ms },
        track: vec![],
    }
}

fn find(ws: &Workspace, msg: &str) -> (u32, i64) {
    let r = ws.records().iter().find(|r| r.message == msg).unwrap();
    (r.id, r.ts.unwrap())
}

fn assert_sorted(ws: &Workspace) {
    let keys: Vec<_> = ws.records().iter().map(|r| (r.ts.unwrap_or(i64::MAX), r.source, r.line)).collect();
    assert!(keys.windows(2).all(|w| w[0] <= w[1]), "records stay in time order");
    assert!(
        ws.records().iter().enumerate().all(|(i, r)| r.id as usize == i),
        "ids are positions"
    );
}

#[test]
fn manual_offset_shifts_every_view_and_keeps_earlier_ids() {
    let d = Dir::new("manual");
    // B's events come after all of A's, so moving B leaves A's ids alone.
    d.write(
        "a.log",
        &(0..10).map(|i| clef(i * 1000, &format!("A {i}"), None)).collect::<String>(),
    );
    d.write(
        "b.log",
        &(20..30).map(|i| clef(i * 1000, &format!("B {i}"), None)).collect::<String>(),
    );
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(std::slice::from_ref(&d.0)).unwrap();
    let b = source_id(&ws, "b.log");
    let a_ids: Vec<(u32, String)> = ws.records().iter().take(10).map(|r| (r.id, r.message.clone())).collect();
    let (b25, b25_ts) = find(&ws, "B 25");
    let (a3, _) = find(&ws, "A 3");

    let change = ws
        .set_source_offset(&SetOffsetRequest {
            spec: OffsetSpec::Manual {
                source: b,
                offset_ms: 2_350,
            },
            track: vec![b25, a3, 9_999],
        })
        .unwrap();
    assert_eq!((change.source, change.offset_ms, change.previous_ms), (b, 2_350, 0));
    assert_eq!(change.ids_changed_from, Some(10));
    assert_eq!(change.tracked[1], Some(a3));
    assert_eq!(change.tracked[2], None);
    let moved = change.tracked[0].unwrap();
    assert_eq!(ws.records()[moved as usize].message, "B 25");
    assert_eq!(ws.records()[moved as usize].ts, Some(b25_ts + 2_350));
    let a_after: Vec<(u32, String)> = ws.records().iter().take(10).map(|r| (r.id, r.message.clone())).collect();
    assert_eq!(a_ids, a_after, "ids before the first moved record are stable");
    assert_sorted(&ws);

    // Summary, page rows, timeline, facets and charts all see shifted time.
    let s = ws.summary();
    let src = s.sources.iter().find(|x| x.id == b).unwrap();
    assert_eq!(src.offset_ms, 2_350);
    assert_eq!(src.from, Some(BASE + 20_000 + 2_350));
    assert_eq!(s.to, Some(BASE + 29_000 + 2_350));
    let page = ws
        .page(&PageRequest {
            query: Query::default(),
            offset: 0,
            limit: 100,
            descending: false,
            anchor: None,
        })
        .unwrap();
    let json = serde_json::to_value(&page).unwrap();
    assert_eq!(json["rows"][0].get("skewMs"), None, "unshifted rows carry no skew");
    assert_eq!(json["rows"][10]["skewMs"], 2_350);
    let ranged = Query {
        from: Some(BASE + 22_000),
        to: Some(BASE + 23_000),
        ..Default::default()
    };
    let tl = ws
        .timeline(&TimelineRequest {
            query: Query::default(),
            buckets: 10,
            always_show_lifecycle: true,
        })
        .unwrap();
    assert_eq!(tl.to, BASE + 29_000 + 2_350);
    let facet = ws
        .facet(&FacetRequest {
            query: ranged.clone(),
            field: "@message".into(),
            limit: 10,
        })
        .unwrap();
    // 22.000–23.000 now holds B 20 (at 22.350) only.
    assert_eq!(facet.values.iter().map(|v| v.value.as_str()).collect::<Vec<_>>(), ["B 20"]);
    let chart = ws.chart(&ChartRequest {
        query: Query::default(),
        chart_id: None,
        def: Some(serde_json::from_value(serde_json::json!({"id": "c", "title": "n", "y": "count()"})).unwrap()),
        buckets: 10,
    });
    assert_eq!(chart.unwrap().to, BASE + 29_000 + 2_350);

    // Setting it back returns everything to the original times; the same offset is a no-op.
    let change = ws.set_source_offset(&manual(b, 0)).unwrap();
    assert_eq!(change.previous_ms, 2_350);
    assert_eq!(find(&ws, "B 25").1, b25_ts);
    assert!(ws.clock_offsets().is_empty());
    assert_eq!(ws.set_source_offset(&manual(b, 0)).unwrap().ids_changed_from, None);
    assert_sorted(&ws);
}

#[test]
fn interleaving_sources_are_merged_in_time_order() {
    let d = Dir::new("merge");
    let (_, _) = two_servers(&d, 50, 2_500);
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(std::slice::from_ref(&d.0)).unwrap();
    let b = source_id(&ws, "b.log");
    ws.set_source_offset(&manual(b, 2_500)).unwrap();
    assert_sorted(&ws);
    // Every B event is now 20 ms after its A counterpart.
    for i in 0..50 {
        let (ia, ta) = find(&ws, &format!("A got {i}"));
        let (ib, tb) = find(&ws, &format!("B got {i}"));
        assert_eq!(tb - ta, 20);
        assert_eq!(ib, ia + 1);
    }
    // Queries are answered from the moved records (match cache updated).
    let q = Query {
        text: "B got 7".into(),
        ..Default::default()
    };
    let ids = ws.matching(&q).unwrap();
    assert_eq!(ws.records()[ids[0] as usize].ts, Some(BASE + 7_020));
    let mut ws2 = Workspace::new(Profile::default_profile()).unwrap();
    ws2.open(std::slice::from_ref(&d.0)).unwrap();
    assert!(ws2.matching(&q).is_ok());
}

#[test]
fn align_on_an_event_sets_ts_a_minus_ts_b() {
    let d = Dir::new("align");
    two_servers(&d, 5, 4_000);
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(std::slice::from_ref(&d.0)).unwrap();
    let (a2, ta) = find(&ws, "A got 2");
    let (b2, tb) = find(&ws, "B got 2");
    let change = ws
        .set_source_offset(&SetOffsetRequest {
            spec: OffsetSpec::Align { reference: a2, target: b2 },
            track: vec![b2],
        })
        .unwrap();
    assert_eq!(change.offset_ms, ta - tb);
    let moved = change.tracked[0].unwrap();
    assert_eq!(ws.records()[moved as usize].ts, Some(ta));
    assert_eq!(ws.records()[moved as usize].message, "B got 2");

    // Aligning again on another pair uses B's original time (not its shifted one).
    let (a3, ta3) = find(&ws, "A got 3");
    let (b3, _) = find(&ws, "B got 3");
    let change = ws
        .set_source_offset(&SetOffsetRequest {
            spec: OffsetSpec::Align { reference: a3, target: b3 },
            track: vec![],
        })
        .unwrap();
    assert_eq!(change.offset_ms, 3_980);
    assert_eq!(find(&ws, "B got 3").1, ta3);

    // A shifted reference counts as shown: aligning A on B now moves A.
    let a = source_id(&ws, "a.log");
    ws.set_source_offset(&manual(a, 1_000)).unwrap();
    let (a1, _) = find(&ws, "A got 1");
    let (b1, tb1) = find(&ws, "B got 1");
    let c = ws
        .set_source_offset(&SetOffsetRequest {
            spec: OffsetSpec::Align { reference: b1, target: a1 },
            track: vec![],
        })
        .unwrap();
    assert_eq!((c.source, c.previous_ms), (a, 1_000));
    assert_eq!(find(&ws, "A got 1").1, tb1);
    assert_sorted(&ws);
}

#[test]
fn invalid_requests_change_nothing() {
    let d = Dir::new("invalid");
    two_servers(&d, 3, 0);
    d.append("a.log", "{\"@m\":\"untimed\"}\n");
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(std::slice::from_ref(&d.0)).unwrap();
    let b = source_id(&ws, "b.log");
    let before: Vec<Option<i64>> = ws.records().iter().map(|r| r.ts).collect();
    assert_eq!(
        ws.set_source_offset(&manual(b, MAX_OFFSET_MS + 1)).unwrap_err(),
        SkewError::TooLarge(MAX_OFFSET_MS + 1)
    );
    assert_eq!(ws.set_source_offset(&manual(77, 5)).unwrap_err(), SkewError::UnknownSource(77));
    let align = |reference, target| SetOffsetRequest {
        spec: OffsetSpec::Align { reference, target },
        track: vec![],
    };
    assert_eq!(ws.set_source_offset(&align(0, 9_999)).unwrap_err(), SkewError::UnknownRecord(9_999));
    let (a0, _) = find(&ws, "A got 0");
    let (a1, _) = find(&ws, "A got 1");
    assert_eq!(
        ws.set_source_offset(&align(a0, a1)).unwrap_err(),
        SkewError::SameSource(source_id(&ws, "a.log"))
    );
    let untimed = ws
        .records()
        .iter()
        .find(|r| r.ts.is_none())
        .map(|r| r.id)
        .expect("a.log has an untimed record");
    assert_eq!(ws.set_source_offset(&align(a0, untimed)).unwrap_err(), SkewError::Untimed(untimed));
    assert_eq!(ws.set_source_offset(&align(untimed, a0)).unwrap_err(), SkewError::Untimed(untimed));
    // Through the store abstraction the error becomes a message.
    let store: &mut dyn LogStore = &mut ws;
    let err = store.set_source_offset(&manual(b, i64::MIN)).unwrap_err().to_string();
    assert!(err.contains("too large"), "{err}");
    let after: Vec<Option<i64>> = ws.records().iter().map(|r| r.ts).collect();
    assert_eq!(before.len(), after.len());
    assert!(ws.summary().sources.iter().all(|s| s.offset_ms == 0));
}

#[test]
fn untimed_records_of_a_shifted_source_stay_last() {
    let d = Dir::new("untimed");
    // A text log with a timestamped line and a line without one.
    let mut text: String = (0..3).map(|i| clef(i * 1000, &format!("A {i}"), None)).collect();
    text.push_str("{\"@m\":\"untimed\"}\n");
    d.write("a.log", &text);
    d.write("b.log", &clef(100, "B", None));
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(std::slice::from_ref(&d.0)).unwrap();
    let a = source_id(&ws, "a.log");
    let n = ws.records().len();
    ws.set_source_offset(&manual(a, -500)).unwrap();
    assert_eq!(ws.records().len(), n);
    assert_eq!(find(&ws, "A 0").1, BASE - 500);
    let last = ws.records().last().unwrap();
    assert_eq!((last.message.as_str(), last.ts), ("untimed", None));
    let page = serde_json::to_value(
        ws.page(&PageRequest {
            query: Query::default(),
            offset: 0,
            limit: 10,
            descending: true,
            anchor: None,
        })
        .unwrap(),
    )
    .unwrap();
    assert_eq!(page["rows"][0].get("skewMs"), None, "untimed rows are not shifted");
    assert_sorted(&ws);
}

#[test]
fn suggestions_from_shared_correlation_ids() {
    let d = Dir::new("suggest");
    two_servers(&d, 30, 2_500);
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(std::slice::from_ref(&d.0)).unwrap();
    let (a, b) = (source_id(&ws, "a.log"), source_id(&ws, "b.log"));
    let store: &dyn LogStore = &ws;
    let r = store.suggest_offsets(&SuggestRequest {
        reference: Some(a),
        ..Default::default()
    });
    assert_eq!(r.reference, Some(a));
    let s = &r.suggestions[0];
    // B is 2.5 s behind and B sees requests 20 ms after A: the latency stays in.
    assert_eq!((s.source, s.suggested_ms, s.pairs, s.confidence), (b, 2_480, 30, 1.0));

    // After applying it, a new suggestion is relative to the current offset.
    ws.set_source_offset(&manual(b, s.suggested_ms)).unwrap();
    let again = ws.suggest_offsets(&SuggestRequest {
        reference: Some(a),
        ..Default::default()
    });
    assert_eq!(again.suggestions[0].current_ms, 2_480);
    assert_eq!(again.suggestions[0].suggested_ms, 2_480);

    // Unknown id fields: nothing to pair.
    let none = ws.suggest_offsets(&SuggestRequest {
        fields: vec!["NoSuchField".into()],
        ..Default::default()
    });
    assert!(none.suggestions.is_empty());
    assert_eq!(none.fields, ["NoSuchField"]);
}

#[test]
fn noisy_ids_lower_the_confidence() {
    let d = Dir::new("noisy");
    let a: String = (0..40).map(|i| clef(i * 60_000, "A", Some(&format!("c{i}")))).collect();
    // Every other id in B belongs to an unrelated event minutes away.
    let b: String = (0..40)
        .map(|i| {
            let noise = if i % 3 == 0 { 600_000 + i * 7_000 } else { 0 };
            clef(i * 60_000 - 3_000 + noise, "B", Some(&format!("c{i}")))
        })
        .collect();
    d.write("a.log", &a);
    d.write("b.log", &b);
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(std::slice::from_ref(&d.0)).unwrap();
    let r = ws.suggest_offsets(&SuggestRequest::default());
    let s = &r.suggestions[0];
    assert_eq!(s.suggested_ms.abs(), 3_000);
    assert!(s.confidence > 0.5 && s.confidence < 0.8, "{}", s.confidence);
    assert!(s.agreeing < s.pairs);
}

#[test]
fn live_tail_and_rotation_keep_the_offset() {
    let d = Dir::new("tail");
    d.write(
        "a.log",
        &(0..3).map(|i| clef(i * 1000, &format!("A {i}"), None)).collect::<String>(),
    );
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(std::slice::from_ref(&d.0)).unwrap();
    let a = source_id(&ws, "a.log");
    ws.set_source_offset(&manual(a, 10_000)).unwrap();
    let p = d.append("a.log", &clef(5_000, "A appended", None));
    ws.refresh_tail(std::slice::from_ref(&p)).unwrap();
    assert_eq!(find(&ws, "A appended").1, BASE + 15_000);
    let row = serde_json::to_value(
        ws.page(&PageRequest {
            query: Query::default(),
            offset: 0,
            limit: 10,
            descending: true,
            anchor: None,
        })
        .unwrap(),
    )
    .unwrap();
    assert_eq!(row["rows"][0]["skewMs"], 10_000);

    // Rotation: the renamed file keeps its source and offset; a new file at the
    // old name (same machine) gets the offset too.
    std::fs::rename(&p, d.0.join("a.log.1")).unwrap();
    d.write("a.log", &clef(9_000, "A new file", None));
    ws.refresh_tail(std::slice::from_ref(&d.0)).unwrap();
    assert_eq!(find(&ws, "A appended").1, BASE + 15_000);
    assert_eq!(find(&ws, "A new file").1, BASE + 19_000);
    let state = ws.state();
    assert_eq!(state.offsets.get(&d.0.join("a.log.1").to_string_lossy()), 10_000);
    assert_sorted(&ws);
}

#[test]
fn reload_keeps_offsets_clear_drops_them() {
    let d = Dir::new("reload");
    two_servers(&d, 3, 0);
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(std::slice::from_ref(&d.0)).unwrap();
    let b = source_id(&ws, "b.log");
    ws.set_source_offset(&manual(b, 700)).unwrap();
    ws.reload().unwrap();
    assert_eq!(find(&ws, "B got 0").1, BASE + 20 + 700);
    let b = source_id(&ws, "b.log");
    assert_eq!(ws.source_offset(b), 700);
    assert_eq!(ws.source_offset(999), 0);
    ws.set_profile(Profile::default_profile()).unwrap();
    assert_eq!(find(&ws, "B got 0").1, BASE + 720);
    ws.clear();
    assert!(ws.clock_offsets().is_empty());
    ws.open(std::slice::from_ref(&d.0)).unwrap();
    assert_eq!(find(&ws, "B got 0").1, BASE + 20);
}

#[test]
fn project_round_trip_restores_offsets_and_ids() {
    let d = Dir::new("project");
    two_servers(&d, 20, 1_500);
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(std::slice::from_ref(&d.0)).unwrap();
    let b = source_id(&ws, "b.log");
    ws.set_source_offset(&manual(b, 1_480)).unwrap();
    let snapshot: Vec<(u32, String, Option<i64>)> = ws.records().iter().map(|r| (r.id, r.message.clone(), r.ts)).collect();

    let state = ws.state();
    let file = d.0.join("p.imtproj");
    ProjectFile {
        version: 0,
        name: "Skew".into(),
        sources: state.roots.clone(),
        removed: state.removed.clone(),
        profile: String::new(),
        ui: serde_json::Value::Null,
        clock: state.offsets.clone(),
        ..Default::default()
    }
    .save(&file)
    .unwrap();
    let stored: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(stored["clock"]["b.log"], 1_480, "offsets are stored relative to the project");
    assert_eq!(stored["version"], 2);

    let loaded = ProjectFile::load(&file).unwrap();
    let mut store: Box<dyn LogStore> = Box::new(Workspace::new(Profile::default_profile()).unwrap());
    store
        .restore(
            Profile::default_profile(),
            &WorkspaceState {
                roots: loaded.sources,
                removed: loaded.removed,
                offsets: loaded.clock,
            },
        )
        .unwrap();
    assert_eq!(store.state(), state);
    let page = store
        .page(&PageRequest {
            query: Query::default(),
            offset: 0,
            limit: 1000,
            descending: false,
            anchor: None,
        })
        .unwrap();
    let restored: Vec<(u32, String, Option<i64>)> = page.rows.iter().map(|r| (r.id, r.message.to_string(), r.ts)).collect();
    assert_eq!(restored, snapshot, "the same records at the same ids and times");
    // Analyses run on the shifted data too.
    assert!(store.anomalies(&QueryRequest { query: Query::default() }).is_ok());
}

#[test]
fn archive_members_are_keyed_by_entry_path() {
    let d = Dir::new("zip");
    let zip_path = d.0.join("logs.zip");
    {
        let f = std::fs::File::create(&zip_path).unwrap();
        let mut z = zip::ZipWriter::new(f);
        let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default();
        z.start_file("web/a.log", opts).unwrap();
        z.write_all(clef(0, "zipped", None).as_bytes()).unwrap();
        z.finish().unwrap();
    }
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(std::slice::from_ref(&zip_path)).unwrap();
    let id = ws.summary().sources[0].id;
    ws.set_source_offset(&manual(id, 42)).unwrap();
    let key = ws.state().offsets.iter().next().unwrap().0.clone();
    assert!(key.ends_with("web/a.log"), "{key}");
    ws.reload().unwrap();
    assert_eq!(find(&ws, "zipped").1, BASE + 42);
    let _ = Path::new(&key);
}
