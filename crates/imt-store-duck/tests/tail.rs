//! Live tail and restore on the store: appended lines are read again into new segments (and searchable),
//! new files under an opened folder are picked up, a truncated file is read from its start, and a saved
//! workspace state imports its roots without the files it removed.

mod common;

use common::{store_with, TempDir};
use imt_core::api::WorkspaceState;
use imt_core::ports::{Ingest, PageQuery, Positions, Query, Search, SearchRequest, Tail, TailRead};
use imt_core::Profile;
use imt_store_duck::DuckStore;
use std::io::Write;
use std::time::Duration;

fn append(path: &std::path::Path, text: &str) {
    let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    f.write_all(text.as_bytes()).unwrap();
}

#[test]
fn appended_lines_new_files_and_truncation_are_followed() {
    let logs = TempDir::new("tail-logs");
    let a = logs.0.join("a.log");
    std::fs::write(&a, "2026-10-14 10:00:00 INFO first\n").unwrap();
    let dir = TempDir::new("tail-store");
    let (store, _) = store_with(&dir, Profile::default_profile(), std::slice::from_ref(&logs.0)).unwrap();
    assert_eq!(store.summary().unwrap().total, 1);
    let first_id = store
        .page(&PageQuery {
            limit: 5,
            ..PageQuery::default()
        })
        .unwrap()
        .rows[0]
        .id;

    // Plain text events may continue on the next lines: the last one is held back until the file is idle.
    append(
        &a,
        "2026-10-14 10:00:01 ERROR second with zebrafish\n2026-10-14 10:00:02 INFO third\n",
    );
    let r = store.append_tail(std::slice::from_ref(&a)).unwrap();
    assert_eq!((r.appended, r.held.len()), (1, 1), "{r:?}");
    assert_eq!(store.summary().unwrap().total, 2);
    // Naming the file again does not write the held event (the file may still be growing); asking for the held
    // events (no paths, after the caller waited for the file to stop growing) does.
    let again = store.append_tail(std::slice::from_ref(&a)).unwrap();
    assert_eq!((again.appended, again.held.len()), (0, 1), "{again:?}");
    let r = store.append_tail(&[]).unwrap();
    assert_eq!((r.appended, r.held.len()), (1, 0), "{r:?}");
    assert_eq!(store.summary().unwrap().total, 3);
    // Ids are stable: the first event kept its id; positions follow the new events.
    let page = store
        .page(&PageQuery {
            limit: 10,
            ..PageQuery::default()
        })
        .unwrap();
    assert_eq!(page.rows[0].id, first_id);
    assert_eq!(
        page.rows.iter().map(|r| r.message.as_str()).collect::<Vec<_>>(),
        ["first", "second with zebrafish", "third"]
    );
    assert_eq!(store.positioned().unwrap(), 3);
    store.wait_for_index(Duration::from_secs(60)).unwrap();
    let hits = store
        .search(&SearchRequest {
            text: "zebrafish".into(),
            limit: 5,
        })
        .unwrap();
    assert_eq!(hits.total, 1, "appended events are searchable");

    // The file's tail state.
    let files = store.tail_files().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(
        (files[0].read, files[0].lines, files[0].offset == files[0].size),
        (TailRead::Append, 3, true)
    );
    assert!(files[0].changed_ms.is_some());

    // A new file in the opened folder.
    let b = logs.0.join("b.log");
    std::fs::write(&b, "2026-10-14 10:00:03 WARN fourth\n").unwrap();
    let r = store.append_tail(std::slice::from_ref(&b)).unwrap();
    assert_eq!((r.appended, r.added.len()), (1, 1), "{r:?}");
    assert_eq!(store.summary().unwrap().sources.len(), 2);

    // A file that disappeared keeps its events.
    std::fs::remove_file(&b).unwrap();
    let r = store.append_tail(std::slice::from_ref(&b)).unwrap();
    assert_eq!((r.deleted.len(), r.appended), (1, 0));
    assert_eq!(store.summary().unwrap().total, 4);

    // Truncated (copy-truncate rotation): read again from its start as a new source.
    let old_source = store
        .summary()
        .unwrap()
        .sources
        .iter()
        .find(|s| s.path.ends_with("a.log"))
        .unwrap()
        .id;
    std::fs::write(&a, "2026-10-14 11:00:00 INFO fresh\n").unwrap();
    let r = store.append_tail(std::slice::from_ref(&a)).unwrap();
    assert_eq!((r.truncated.len(), r.appended), (1, 1), "{r:?}");
    assert_ne!(r.truncated[0], old_source);
    assert_eq!(
        store.summary().unwrap().total,
        2,
        "a's three events were replaced by its one (and b's stays)"
    );

    // Nothing changed: nothing added.
    assert!(store.append_tail(&[]).unwrap().is_empty());
}

#[test]
fn a_saved_state_is_restored_without_its_removed_files() {
    let logs = TempDir::new("restore-logs");
    std::fs::write(logs.0.join("a.log"), "2026-10-14 10:00:00 INFO a\n").unwrap();
    std::fs::write(logs.0.join("b.log"), "2026-10-14 10:00:00 INFO b\n2026-10-14 10:00:01 INFO b2\n").unwrap();
    let dir = TempDir::new("restore-store");
    let (store, _) = store_with(&dir, Profile::default_profile(), &[]).unwrap();
    let removed = std::fs::canonicalize(logs.0.join("b.log")).unwrap();
    let state = WorkspaceState {
        roots: vec![logs.0.to_string_lossy().into_owned()],
        removed: vec![removed.to_string_lossy().into_owned()],
        offsets: Default::default(),
    };
    store.restore(&state).unwrap().wait().unwrap();
    let s = store.summary().unwrap();
    assert_eq!((s.total, s.sources.len()), (1, 1));
}

fn clef(n: u32) -> String {
    format!("{{\"@t\":\"2026-10-14T10:00:{n:02}Z\",\"@m\":\"event {n}\",\"@l\":\"Information\"}}\n")
}

#[test]
fn events_of_one_line_are_not_held_back_and_a_file_opened_at_its_end_follows_from_there() {
    let logs = TempDir::new("tail-end-logs");
    let a = logs.0.join("a.clef");
    std::fs::write(&a, format!("{}{}", clef(0), clef(1))).unwrap();
    let dir = TempDir::new("tail-end-store");
    let (store, _) = store_with(&dir, Profile::default_profile(), &[]).unwrap();
    // Opened at its end: the source is there, with no events yet.
    let report = store.open_at_end(std::slice::from_ref(&logs.0)).unwrap().wait().unwrap();
    assert_eq!((report.events, report.sources.len()), (0, 1));
    let s = store.summary().unwrap();
    assert_eq!((s.total, s.sources.len()), (0, 1));
    let files = store.tail_files().unwrap();
    assert_eq!(
        (files[0].read, files[0].lines, files[0].offset),
        (TailRead::Append, 2, std::fs::metadata(&a).unwrap().len())
    );
    assert_eq!(files[0].note.as_deref(), Some("opened at its end"));
    // Lines written from now on are the events; a one-line format needs no holding back.
    append(&a, &format!("{}{}", clef(2), clef(3)));
    let r = store.append_tail(std::slice::from_ref(&a)).unwrap();
    assert_eq!((r.appended, r.held.len()), (2, 0), "{r:?}");
    let page = store
        .page(&PageQuery {
            limit: 10,
            ..PageQuery::default()
        })
        .unwrap();
    assert_eq!(
        page.rows.iter().map(|r| (r.message.as_str(), r.line)).collect::<Vec<_>>(),
        [("event 2", 3), ("event 3", 4)]
    );
    // Opening the same file again reads nothing new: it is known.
    let again = store.open_at_end(std::slice::from_ref(&a)).unwrap().wait().unwrap();
    assert_eq!(again.sources.len(), 0);

    // What cannot be followed from the middle is read in full, and again in full when it changes.
    let doc = logs.0.join("doc.json");
    std::fs::write(&doc, "[{\"@t\":\"2026-10-14T10:00:00Z\",\"@m\":\"x\"}]").unwrap();
    let report = store.open_at_end(std::slice::from_ref(&doc)).unwrap().wait().unwrap();
    assert_eq!(report.events, 1);
    let tracked = store.tail_files().unwrap();
    let t = tracked.iter().find(|f| f.path.ends_with("doc.json")).unwrap();
    assert_eq!(t.read, TailRead::Whole);
    assert!(t.note.as_deref().unwrap().contains("JSON document"), "{t:?}");
    std::fs::write(
        &doc,
        "[{\"@t\":\"2026-10-14T10:00:00Z\",\"@m\":\"x\"},{\"@t\":\"2026-10-14T10:00:01Z\",\"@m\":\"y\"}]",
    )
    .unwrap();
    let r = store.append_tail(std::slice::from_ref(&doc)).unwrap();
    assert_eq!((r.reimported.len(), r.appended), (1, 2), "{r:?}");
}

#[test]
fn a_file_whose_last_line_was_unfinished_when_opened_is_read_in_full_when_it_grows() {
    let logs = TempDir::new("tail-partial-logs");
    let a = logs.0.join("a.log");
    std::fs::write(&a, "2026-10-14 10:00:00 INFO one\n2026-10-14 10:00:01 INFO tw").unwrap();
    let dir = TempDir::new("tail-partial-store");
    let (store, _) = store_with(&dir, Profile::default_profile(), std::slice::from_ref(&logs.0)).unwrap();
    assert_eq!(store.summary().unwrap().total, 2);
    append(&a, "o\n");
    let r = store.append_tail(std::slice::from_ref(&a)).unwrap();
    assert_eq!((r.reimported.len(), r.appended), (1, 2), "{r:?}");
    let messages: Vec<String> = store
        .page(&PageQuery {
            limit: 10,
            ..PageQuery::default()
        })
        .unwrap()
        .rows
        .iter()
        .map(|r| r.message.clone())
        .collect();
    assert_eq!(messages, ["one", "two"], "no duplicate of the half line");
    let t = &store.tail_files().unwrap()[0];
    assert_eq!(t.read, TailRead::Whole);
    assert!(t.note.as_deref().unwrap().contains("not finished"), "{t:?}");
}

#[test]
fn an_unreadable_or_garbage_appendix_does_not_stop_the_other_files() {
    let logs = TempDir::new("tail-robust-logs");
    let (a, b) = (logs.0.join("a.clef"), logs.0.join("b.clef"));
    std::fs::write(&a, clef(0)).unwrap();
    std::fs::write(&b, clef(0)).unwrap();
    let dir = TempDir::new("tail-robust-store");
    let (store, _) = store_with(&dir, Profile::default_profile(), std::slice::from_ref(&logs.0)).unwrap();
    // Garbage bytes and an unterminated half line in one file, a good line in the other.
    let mut f = std::fs::OpenOptions::new().append(true).open(&a).unwrap();
    f.write_all(&[0xff, 0xfe, 0x00, b'\n', 0x80, 0x81, b'{', b'"', b'@']).unwrap();
    drop(f);
    append(&b, &clef(5));
    let r = store.append_tail(&[a.clone(), b.clone()]).unwrap();
    assert!(r.appended >= 1, "{r:?}");
    let total = store.summary().unwrap().total;
    assert!(total >= 3, "b's line was read ({total})");
    // The store still answers and the next change is read.
    append(&b, &clef(6));
    let r = store.append_tail(std::slice::from_ref(&b)).unwrap();
    assert_eq!(r.appended, 1);
    assert_eq!(store.positioned().unwrap(), store.summary().unwrap().total as u64);
}

#[test]
fn events_out_of_time_order_renumber_the_positions() {
    let logs = TempDir::new("tail-order-logs");
    let a = logs.0.join("a.clef");
    std::fs::write(&a, format!("{}{}", clef(10), clef(20))).unwrap();
    let dir = TempDir::new("tail-order-store");
    let (store, _) = store_with(&dir, Profile::default_profile(), std::slice::from_ref(&logs.0)).unwrap();
    append(&a, &format!("{}{}", clef(5), clef(30)));
    store.append_tail(std::slice::from_ref(&a)).unwrap();
    let page = store
        .page(&PageQuery {
            limit: 10,
            ..PageQuery::default()
        })
        .unwrap();
    assert_eq!(
        page.rows.iter().map(|r| r.message.as_str()).collect::<Vec<_>>(),
        ["event 5", "event 10", "event 20", "event 30"]
    );
    assert_eq!(store.positioned().unwrap(), 4);
    let ids = store.ids_at(0, 4).unwrap();
    assert_eq!(
        ids,
        page.rows.iter().map(|r| r.id).collect::<Vec<_>>(),
        "positions follow the page order"
    );
}

#[test]
fn a_store_written_before_live_tail_state_gains_its_tables_when_opened() {
    let logs = TempDir::new("tail-old-logs");
    let a = logs.0.join("a.clef");
    std::fs::write(&a, format!("{}{}", clef(0), clef(1))).unwrap();
    let dir = TempDir::new("tail-old-store");
    {
        let (store, _) = store_with(&dir, Profile::default_profile(), std::slice::from_ref(&logs.0)).unwrap();
        assert_eq!(store.summary().unwrap().total, 2);
    }
    {
        // As an older build left it: without the tables added since.
        let conn = duckdb::Connection::open(dir.0.join(imt_store_duck::DB_FILE)).unwrap();
        conn.execute_batch("DROP TABLE tail_offsets; DROP TABLE clock_offsets;").unwrap();
    }
    let store = DuckStore::open(&dir.0, Profile::default_profile(), imt_core::PluginRegistry::new()).unwrap();
    assert_eq!(store.tail_files().unwrap().len(), 1);
    assert!(imt_core::ports::Skew::clock_offsets(&store).unwrap().is_empty());
    append(&a, &clef(2));
    let r = store.append_tail(std::slice::from_ref(&a)).unwrap();
    assert_eq!(r.appended, 1, "{r:?}");
    assert_eq!(store.summary().unwrap().total, 3);
}

fn messages(store: &DuckStore) -> Vec<String> {
    store
        .page(&PageQuery {
            limit: 50,
            ..PageQuery::default()
        })
        .unwrap()
        .rows
        .iter()
        .map(|r| r.message.clone())
        .collect()
}

/// Continuation lines (a stack trace) that arrive after a file was imported and then followed belong to the last
/// imported event: the source is read again so they are not separate untimed events.
#[test]
fn continuation_lines_after_import_then_follow_join_the_last_event() {
    let logs = TempDir::new("tail-cont-logs");
    let a = logs.0.join("a.log");
    std::fs::write(&a, "2026-10-14 10:00:00 INFO first\n2026-10-14 10:00:01 ERROR boom\n").unwrap();
    let dir = TempDir::new("tail-cont-store");
    let (store, _) = store_with(&dir, Profile::default_profile(), std::slice::from_ref(&logs.0)).unwrap();
    assert_eq!(store.summary().unwrap().total, 2);
    append(&a, "   at Foo.Bar() in Foo.cs:line 12\n   at Baz.Run()\n");
    let r = store.append_tail(std::slice::from_ref(&a)).unwrap();
    assert_eq!(r.reimported.len(), 1, "the source was read again: {r:?}");
    let all = messages(&store);
    assert_eq!(all.len(), 2, "no separate untimed events: {all:?}");
    // Following goes on from the new source.
    append(&a, "2026-10-14 10:00:09 INFO later\n");
    store.append_tail(std::slice::from_ref(&a)).unwrap();
    store.append_tail(&[]).unwrap();
    assert_eq!(messages(&store).len(), 3);
}

/// New events after an imported file are appended as before when nothing continued the last one (it is not
/// written twice).
#[test]
fn the_last_imported_event_is_not_written_twice_when_nothing_continued_it() {
    let logs = TempDir::new("tail-once-logs");
    let a = logs.0.join("a.log");
    std::fs::write(&a, "2026-10-14 10:00:00 INFO first\n2026-10-14 10:00:01 INFO second\n").unwrap();
    let dir = TempDir::new("tail-once-store");
    let (store, _) = store_with(&dir, Profile::default_profile(), std::slice::from_ref(&logs.0)).unwrap();
    append(&a, "2026-10-14 10:00:02 INFO third\n2026-10-14 10:00:03 INFO fourth\n");
    store.append_tail(std::slice::from_ref(&a)).unwrap();
    store.append_tail(&[]).unwrap();
    assert_eq!(messages(&store), ["first", "second", "third", "fourth"]);
}
