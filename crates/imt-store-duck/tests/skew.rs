//! Clock-skew correction on the store: an offset moves a file's events (ids stay), survives a reopen, applies to
//! files read later and to appended lines, refuses bad requests with what to fix, and the store suggests and
//! applies the same offsets as the in-memory reference.

mod common;

use common::{pair_of, store_with, TempDir};
use imt_core::ports::{Filter, Ingest, PageQuery, Positions, Query, Skew};
use imt_core::skew::{OffsetSpec, SetOffsetRequest, SuggestRequest};
use imt_core::{PluginRegistry, Profile};
use imt_store_duck::DuckStore;
use std::io::Write;

fn manual(source: u32, offset_ms: i64) -> SetOffsetRequest {
    SetOffsetRequest {
        spec: OffsetSpec::Manual { source, offset_ms },
        track: Vec::new(),
    }
}

fn times(store: &DuckStore) -> Vec<(String, Option<i64>)> {
    store
        .page(&PageQuery {
            limit: 100,
            ..PageQuery::default()
        })
        .unwrap()
        .rows
        .iter()
        .map(|r| (r.message.clone(), r.ts))
        .collect()
}

fn source_of(store: &DuckStore, file: &str) -> u32 {
    store.summary().unwrap().sources.iter().find(|s| s.path.ends_with(file)).unwrap().id
}

#[test]
fn an_offset_moves_a_files_events_and_is_kept_with_the_store() {
    let logs = TempDir::new("skew-logs");
    std::fs::write(logs.0.join("a.log"), "2026-10-14 10:00:00 INFO a1\n2026-10-14 10:00:10 INFO a2\n").unwrap();
    std::fs::write(logs.0.join("b.log"), "2026-10-14 10:00:05 INFO b1\n2026-10-14 10:00:15 INFO b2\n").unwrap();
    let dir = TempDir::new("skew-store");
    let (store, _) = store_with(&dir, Profile::default_profile(), std::slice::from_ref(&logs.0)).unwrap();
    let before = times(&store);
    let order: Vec<&str> = before.iter().map(|(m, _)| m.as_str()).collect();
    assert_eq!(order, ["a1", "b1", "a2", "b2"]);
    let ids_before: Vec<_> = store
        .page(&PageQuery {
            limit: 10,
            ..PageQuery::default()
        })
        .unwrap()
        .rows
        .iter()
        .map(|r| (r.message.clone(), r.id))
        .collect();
    let b = source_of(&store, "b.log");

    let (first, last) = (ids_before[0].1 .0 as u32, ids_before[3].1 .0 as u32);
    let change = store
        .set_source_offset(&SetOffsetRequest {
            track: vec![first, last, 9_999_999],
            ..manual(b, 20_000)
        })
        .unwrap();
    assert_eq!(
        (change.source, change.offset_ms, change.previous_ms, change.ids_changed_from),
        (b, 20_000, 0, None)
    );
    assert_eq!(change.tracked, [Some(first), Some(last), None], "ids stay; unknown ones are None");
    let after = times(&store);
    let order: Vec<&str> = after.iter().map(|(m, _)| m.as_str()).collect();
    assert_eq!(order, ["a1", "a2", "b1", "b2"], "b's events moved after a's");
    let shift = |m: &str| after.iter().find(|r| r.0 == m).unwrap().1.unwrap() - before.iter().find(|r| r.0 == m).unwrap().1.unwrap();
    assert_eq!((shift("a1"), shift("b1"), shift("b2")), (0, 20_000, 20_000));
    let ids_after: Vec<_> = store
        .page(&PageQuery {
            limit: 10,
            ..PageQuery::default()
        })
        .unwrap()
        .rows
        .iter()
        .map(|r| (r.message.clone(), r.id))
        .collect();
    for (m, id) in &ids_before {
        assert_eq!(ids_after.iter().find(|x| &x.0 == m).unwrap().1, *id, "{m} keeps its id");
    }
    let summary = store.summary().unwrap();
    let info = summary.sources.iter().find(|s| s.id == b).unwrap();
    assert_eq!(info.offset_ms, 20_000);
    assert_eq!(
        (info.from, info.to),
        (Some(before[1].1.unwrap() + 20_000), Some(before[3].1.unwrap() + 20_000))
    );
    assert_eq!(summary.to, Some(before[3].1.unwrap() + 20_000));
    assert_eq!(store.positioned().unwrap(), 4);
    assert_eq!(store.clock_offsets().unwrap().get(&info.path), 20_000);

    // Another change moves by the difference only.
    let c = store.set_source_offset(&manual(b, 5_000)).unwrap();
    assert_eq!((c.previous_ms, c.offset_ms), (20_000, 5_000));
    assert_eq!(shift_of(&store, &before, "b1"), 5_000);

    // Refused requests change nothing and say what to fix.
    for (req, wants) in [
        (manual(b, 8 * 24 * 3_600_000), "too large"),
        (manual(9_999, 1), "not open any more"),
    ] {
        let e = store.set_source_offset(&req).unwrap_err().to_string();
        assert!(e.contains(wants), "{e}");
    }
    assert_eq!(store.clock_offsets().unwrap().get(&info.path), 5_000);

    // Lines appended later take the offset.
    let mut f = std::fs::OpenOptions::new().append(true).open(logs.0.join("b.log")).unwrap();
    f.write_all(b"2026-10-14 10:00:30 INFO b3\n2026-10-14 10:00:40 INFO b4\n").unwrap();
    drop(f);
    store.append_tail(&[logs.0.join("b.log")]).unwrap();
    store.append_tail(&[]).unwrap();
    let now = times(&store);
    let b3 = now.iter().find(|r| r.0 == "b3").unwrap().1.unwrap();
    assert_eq!(b3, utc("2026-10-14 10:00:30") + 5_000);
    assert_eq!(store.positioned().unwrap(), 6);

    // Kept with the store: a reopened store still holds the corrected times and the offset.
    drop(store);
    let reopened = DuckStore::open(&dir.0, Profile::default_profile(), PluginRegistry::new()).unwrap();
    assert_eq!(reopened.clock_offsets().unwrap().get(&info.path), 5_000);
    assert_eq!(times(&reopened).iter().find(|r| r.0 == "b3").unwrap().1, Some(b3));

    // A saved project repeats the offsets.
    let saved = imt_core::ports::Projects::save_project(&reopened, &imt_core::package::ProjectManifest::default());
    if let Ok(m) = saved {
        assert_eq!(
            m.clock_offsets.iter().map(|o| (o.path.clone(), o.offset_ms)).collect::<Vec<_>>(),
            [(info.path.clone(), 5_000)]
        );
    }

    // Back to none: the original times.
    let c = reopened.set_source_offset(&manual(b, 0)).unwrap();
    assert_eq!(c.previous_ms, 5_000);
    assert!(reopened.clock_offsets().unwrap().is_empty());
    assert_eq!(times(&reopened).iter().find(|r| r.0 == "b1").unwrap().1, before[1].1);
}

fn shift_of(store: &DuckStore, before: &[(String, Option<i64>)], m: &str) -> i64 {
    times(store).iter().find(|r| r.0 == m).unwrap().1.unwrap() - before.iter().find(|r| r.0 == m).unwrap().1.unwrap()
}

fn utc(text: &str) -> i64 {
    chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S")
        .unwrap()
        .and_utc()
        .timestamp_millis()
}

#[test]
fn files_read_later_and_restored_states_take_the_offsets() {
    let logs = TempDir::new("skew-later-logs");
    std::fs::write(logs.0.join("a.log"), "2026-10-14 10:00:00 INFO a1\n").unwrap();
    let dir = TempDir::new("skew-later-store");
    let (store, _) = store_with(&dir, Profile::default_profile(), &[]).unwrap();
    let path = std::fs::canonicalize(logs.0.join("a.log")).unwrap().to_string_lossy().into_owned();
    let mut offsets = imt_core::skew::ClockOffsets::default();
    offsets.set(&path, -3_000).unwrap();
    store.set_clock_offsets(&offsets).unwrap();
    store.open(std::slice::from_ref(&logs.0)).unwrap().wait().unwrap();
    assert_eq!(times(&store)[0].1, Some(utc("2026-10-14 10:00:00") - 3_000));
    let summary = store.summary().unwrap();
    assert_eq!(summary.sources[0].offset_ms, -3_000);
    assert_eq!(summary.from, Some(utc("2026-10-14 10:00:00") - 3_000));

    // Replacing the set clears the offsets it leaves out and moves their events back.
    store.set_clock_offsets(&imt_core::skew::ClockOffsets::default()).unwrap();
    assert_eq!(times(&store)[0].1, Some(utc("2026-10-14 10:00:00")));

    // A restored state brings its offsets before it reads the files.
    let state = imt_core::api::WorkspaceState {
        roots: vec![logs.0.to_string_lossy().into_owned()],
        removed: Vec::new(),
        offsets,
    };
    store.restore(&state).unwrap().wait().unwrap();
    assert_eq!(times(&store)[0].1, Some(utc("2026-10-14 10:00:00") - 3_000));
}

#[test]
fn aligning_two_events_and_suggesting_offsets_match_the_reference_store() {
    // Two machines logging the same requests, one clock 2.5 s behind.
    let logs = TempDir::new("skew-align-logs");
    for (host, skew_ms) in [("web01", 0i64), ("web02", -2_500)] {
        std::fs::create_dir_all(logs.0.join(host)).unwrap();
        let mut text = String::new();
        for i in 0..8i64 {
            let t = 1_791_972_000_000 + i * 7_000 + skew_ms;
            text.push_str(&format!(
                "{{\"@t\":\"{}\",\"@m\":\"req {i}\",\"CorrelationId\":\"c{i}\"}}\n",
                iso(t)
            ));
        }
        std::fs::write(logs.0.join(host).join("app.clef"), text).unwrap();
    }
    let p = pair_of(Profile::default_profile(), std::slice::from_ref(&logs.0));
    let req = SuggestRequest::default();
    let (w, d) = (p.ws.suggest_offsets(&req), Skew::suggest_offsets(&p.duck, &req).unwrap());
    assert_eq!(d.fields, w.fields);
    assert_eq!(d.suggestions.len(), 1);
    assert_eq!(
        d.suggestions
            .iter()
            .map(|s| (s.suggested_ms, s.pairs, s.agreeing))
            .collect::<Vec<_>>(),
        w.suggestions
            .iter()
            .map(|s| (s.suggested_ms, s.pairs, s.agreeing))
            .collect::<Vec<_>>()
    );
    assert_eq!(d.suggestions[0].suggested_ms.abs(), 2_500);

    // Aligning the first event of web02 on the first event of web01 gives the same offset in both.
    let by_path = |paths: &std::collections::HashMap<u32, String>, host: &str| *paths.iter().find(|(_, p)| p.contains(host)).unwrap().0;
    let first_event =
        |ids: &std::collections::HashMap<u32, String>, host: &str, store_ids: &dyn Fn(u32) -> u32| store_ids(by_path(ids, host));
    let ws_ref = first_event(&p.ws_paths, "web01", &|s| p.ws.records().iter().find(|r| r.source == s).unwrap().id);
    let ws_tgt = first_event(&p.ws_paths, "web02", &|s| p.ws.records().iter().find(|r| r.source == s).unwrap().id);
    let duck_first = |host: &str| {
        let s = by_path(&p.duck_paths, host);
        let page = p
            .duck
            .page(&PageQuery {
                query: Filter {
                    sources: vec![s],
                    ..Filter::default()
                },
                limit: 1,
                ..PageQuery::default()
            })
            .unwrap();
        page.rows[0].id.0 as u32
    };
    let mut ws = p.ws;
    let wc = ws
        .set_source_offset(&SetOffsetRequest {
            spec: OffsetSpec::Align {
                reference: ws_ref,
                target: ws_tgt,
            },
            track: vec![],
        })
        .unwrap();
    let dc = p
        .duck
        .set_source_offset(&SetOffsetRequest {
            spec: OffsetSpec::Align {
                reference: duck_first("web01"),
                target: duck_first("web02"),
            },
            track: vec![],
        })
        .unwrap();
    assert_eq!(dc.offset_ms, wc.offset_ms);
    assert_eq!(dc.offset_ms, 2_500);
    // Events of one source cannot align with each other.
    let e = p
        .duck
        .set_source_offset(&SetOffsetRequest {
            spec: OffsetSpec::Align {
                reference: duck_first("web01"),
                target: duck_first("web01"),
            },
            track: vec![],
        })
        .unwrap_err()
        .to_string();
    assert!(e.contains("same source"), "{e}");
}

fn iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}
