//! Live tail: incremental append, partial lines, multi-line records, rotation
//! (rename, log4net chains, Serilog rolling, copy-truncate, logrotate .gz),
//! deletion, folder tail, start-at-end, retention, fallbacks, watchers,
//! status, the same ingest pipeline for appended records, incremental
//! derived state, live windows and live alerts. Refreshes are driven
//! directly (no OS watcher timing) so every test is deterministic.

use imt_core::api::{FacetRequest, MomentRequest, PageRequest, QueryRequest, RequestsRequest, TimelineRequest, TraceRequest};
use imt_core::importers::{BinaryImporter, ImportCtx, Imported, Importer, Importers};
use imt_core::model::{Level, Record};
use imt_core::query::Query;
use imt_core::workspace::{
    identity_of, Coalescer, DeletedFiles, FileWatcher, LiveTail, OpenOptions, TailFileState, TailOptions, TimeWindow, WatchMode,
    WatchPreference, WatchSettings, WatchSignal, WatchSink, WatcherFactory,
};
use imt_core::{LogStore, Profile, Workspace};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        let p = std::env::temp_dir().join(format!("imt-tail-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Dir(std::fs::canonicalize(p).unwrap())
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn write(&self, name: &str, text: &str) -> PathBuf {
        let p = self.path(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, text).unwrap();
        p
    }
    fn append(&self, name: &str, text: &str) -> PathBuf {
        let p = self.path(name);
        let mut f = std::fs::OpenOptions::new().append(true).create(true).open(&p).unwrap();
        f.write_all(text.as_bytes()).unwrap();
        p
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// One Serilog file-sink line at second `s`.
fn line(s: u32, level: &str, machine: &str, msg: &str) -> String {
    format!(
        "2026-10-14 10:{:02}:{:02}.000 +00:00 [{level}] {msg} {{\"MachineName\": \"{machine}\"}}\n",
        s / 60,
        s % 60
    )
}

fn lines(range: std::ops::Range<u32>, machine: &str) -> String {
    range.map(|i| line(i, "INF", machine, &format!("event {i}"))).collect()
}

fn ws_on(dir: &Path) -> Workspace {
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(&[dir.to_path_buf()]).unwrap();
    ws
}

fn messages(ws: &Workspace) -> Vec<String> {
    ws.records().iter().map(|r| r.message.clone()).collect()
}

fn source_of(ws: &Workspace, name: &str) -> imt_core::api::SourceInfo {
    ws.summary().sources.into_iter().find(|s| s.name == name).unwrap_or_else(|| {
        panic!(
            "no source {name} in {:?}",
            ws.summary().sources.iter().map(|s| &s.name).collect::<Vec<_>>()
        )
    })
}

fn json(v: impl serde::Serialize) -> serde_json::Value {
    serde_json::to_value(v).unwrap()
}

#[test]
fn appends_text_without_reimporting() {
    let d = Dir::new("text");
    let app = d.write("app.log", &lines(0..3, "A"));
    let mut ws = ws_on(&d.0);
    let first_id = ws.records()[0].id;
    d.append("app.log", &lines(3..5, "A"));
    let report = ws.refresh_tail(std::slice::from_ref(&app)).unwrap();
    assert_eq!(report.appended, 2);
    assert_eq!(report.updated, 0);
    assert_eq!(report.ids_changed_from, None);
    assert!(report.reimported.is_empty());
    assert_eq!(report.newest_ts, ws.records().last().unwrap().ts);
    assert_eq!(ws.summary().total, 5);
    assert_eq!(ws.records()[0].id, first_id);
    assert_eq!(ws.records()[4].message, "event 4");
    assert_eq!(ws.records()[4].line, 5);
    let s = source_of(&ws, "app.log");
    assert_eq!((s.records, s.to), (5, ws.records()[4].ts));
    // Nothing new: nothing changes.
    let again = ws.refresh_tail(&[app]).unwrap();
    assert!(again.is_empty() && again.appended == 0);
}

#[test]
fn appends_clef_json_lines() {
    let d = Dir::new("clef");
    let ev = |i: u32| format!("{{\"@t\":\"2026-10-14T10:00:{i:02}Z\",\"@mt\":\"Order {{OrderId}}\",\"OrderId\":{i}}}\n");
    let p = d.write("app.clef", &(0..3).map(ev).collect::<String>());
    let mut ws = ws_on(&d.0);
    d.append("app.clef", &(3..6).map(ev).collect::<String>());
    let report = ws.refresh_tail(std::slice::from_ref(&p)).unwrap();
    assert_eq!(report.appended, 3);
    assert_eq!(ws.records()[5].fields["OrderId"], 5);
    assert_eq!(ws.records()[5].line, 6);
    // A half-written JSON line waits for its newline.
    d.append("app.clef", "{\"@t\":\"2026-10-14T10:00:07Z\",\"@mt\":\"half");
    assert_eq!(ws.refresh_tail(std::slice::from_ref(&p)).unwrap().appended, 0);
    d.append("app.clef", "\"}\n");
    assert_eq!(ws.refresh_tail(&[p]).unwrap().appended, 1);
    assert_eq!(ws.records()[6].message, "half");
    assert_eq!(source_of(&ws, "app.clef").skipped, 0);
}

#[test]
fn csv_keeps_its_header() {
    let d = Dir::new("csv");
    let p = d.write(
        "perf.csv",
        "Timestamp,Level,Machine,Cpu,Message\n2026-10-14 10:00:00,Information,WEB01,10,ok\n",
    );
    let mut ws = ws_on(&d.0);
    d.append(
        "perf.csv",
        "2026-10-14 10:00:01,Warning,WEB02,95,hot\n2026-10-14 10:00:02,Information,WEB01,12,ok\n",
    );
    let report = ws.refresh_tail(&[p]).unwrap();
    assert_eq!(report.appended, 2);
    let r = &ws.records()[1];
    assert_eq!(r.level, Level::Warning);
    assert_eq!(r.fields["Machine"], "WEB02");
    assert_eq!(r.fields["Cpu"], 95);
    assert_eq!(r.message, "hot");
    assert_eq!(source_of(&ws, "perf.csv").importer, "csv");
}

#[test]
fn partial_line_waits_then_completes_or_flushes() {
    let d = Dir::new("partial");
    let p = d.write("app.log", &lines(0..2, "A"));
    let mut ws = ws_on(&d.0);
    let full = line(2, "INF", "A", "partial line");
    let (head, rest) = full.split_at(30);
    d.append("app.log", head);
    let r = ws.refresh_tail(std::slice::from_ref(&p)).unwrap();
    assert_eq!((r.appended, ws.summary().total), (0, 2));
    assert_eq!(ws.tail_status().files[0].offset, lines(0..2, "A").len() as u64);
    // Still waiting (the same partial line, seen again).
    assert_eq!(ws.refresh_tail(std::slice::from_ref(&p)).unwrap().appended, 0);
    d.append("app.log", rest);
    assert_eq!(ws.refresh_tail(std::slice::from_ref(&p)).unwrap().appended, 1);
    assert_eq!(ws.records()[2].message, "partial line");

    // With a zero timeout the unterminated line is parsed at once, and
    // replaced (not duplicated) when it is completed.
    ws.set_tail_options(TailOptions {
        carry_flush_ms: 0,
        ..TailOptions::default()
    });
    let full = line(3, "INF", "A", "flushed early");
    let (head, rest) = full.split_at(full.len() - 6);
    d.append("app.log", head);
    let r = ws.refresh_tail(std::slice::from_ref(&p)).unwrap();
    assert_eq!(r.appended, 1);
    d.append("app.log", rest);
    let r = ws.refresh_tail(std::slice::from_ref(&p)).unwrap();
    assert_eq!((r.appended, r.updated), (0, 1));
    assert_eq!(ws.summary().total, 4);
    assert_eq!(ws.records()[3].message, "flushed early");
    // A carry longer than the line limit is flushed regardless of time.
    ws.set_tail_options(TailOptions {
        carry_flush_ms: 3_600_000,
        ..TailOptions::default()
    });
    d.append(
        "app.log",
        &format!("2026-10-14 10:00:09 INFO {}", "x".repeat(imt_core::importers::MAX_LINE_LEN)),
    );
    let r = ws.refresh_tail(&[p]).unwrap();
    assert!(r.sources_changed.len() == 1 && ws.summary().sources[0].skipped >= 1, "{r:?}");
}

#[test]
fn multiline_exception_merges_into_its_event() {
    let d = Dir::new("multiline");
    let p = d.write("app.log", &(lines(0..2, "A") + &*line(2, "ERR", "A", "Payment failed")));
    let mut ws = ws_on(&d.0);
    let id = ws.records()[2].id;
    d.append("app.log", "System.TimeoutException: The operation has timed out.\n");
    let r = ws.refresh_tail(std::slice::from_ref(&p)).unwrap();
    assert_eq!((r.appended, r.updated, r.ids_changed_from), (0, 1, None));
    d.append("app.log", "   at Payments.Gateway.Charge()\n   at Payments.Api.Post()\n");
    ws.refresh_tail(std::slice::from_ref(&p)).unwrap();
    d.append("app.log", &line(3, "INF", "A", "next"));
    let r = ws.refresh_tail(&[p]).unwrap();
    assert_eq!(r.appended, 1);
    assert_eq!(ws.summary().total, 4);
    let rec = &ws.records()[2];
    assert_eq!(rec.id, id);
    assert_eq!(rec.message, "Payment failed");
    let ex = rec.exception.as_deref().unwrap();
    assert!(
        ex.starts_with("System.TimeoutException") && ex.ends_with("at Payments.Api.Post()"),
        "{ex}"
    );
    // The exception marker came with the merge.
    assert_ne!(rec.markers, 0);
    // Pipeline statistics count the event once.
    let s = source_of(&ws, "app.log");
    assert_eq!(s.records, 4);
}

#[test]
fn rename_rotation_keeps_records_and_source() {
    let d = Dir::new("rename");
    let app = d.write("app.log", &lines(0..3, "A"));
    let mut ws = ws_on(&d.0);
    let id = source_of(&ws, "app.log").id;
    d.append("app.log", &lines(3..4, "A"));
    let rotated = d.path("app.log.1");
    std::fs::rename(&app, &rotated).unwrap();
    d.write("app.log", &lines(10..12, "A"));
    let r = ws.refresh_tail(&[app.clone(), rotated.clone()]).unwrap();
    assert_eq!(r.rotated.len(), 1);
    assert_eq!(r.rotated[0].source_id, id);
    assert!(r.rotated[0].to.ends_with("app.log.1"));
    assert_eq!(r.added.len(), 1);
    assert_eq!(ws.summary().total, 4 + 2, "{:?}", messages(&ws));
    let old = source_of(&ws, "app.log.1");
    assert_eq!((old.id, old.records, old.group.as_str()), (id, 4, "app.log"));
    let new = source_of(&ws, "app.log");
    assert_eq!((new.records, new.group.as_str()), (2, "app.log"));
    let status = ws.tail_status();
    assert!(status.files.iter().any(|f| f.source_id == id && f.state == TailFileState::Rotated));
    // Writing to the rotated file is still read.
    d.append("app.log.1", &lines(4..5, "A"));
    assert_eq!(ws.refresh_tail(&[rotated]).unwrap().appended, 1);
    assert_eq!(source_of(&ws, "app.log.1").records, 5);
}

#[test]
fn log4net_rotation_chain() {
    let d = Dir::new("log4net");
    d.write("app.log.1", &lines(0..2, "A"));
    d.write("app.log", &lines(2..4, "A"));
    let mut ws = ws_on(&d.0);
    assert_eq!(ws.summary().total, 4);
    std::fs::rename(d.path("app.log.1"), d.path("app.log.2")).unwrap();
    std::fs::rename(d.path("app.log"), d.path("app.log.1")).unwrap();
    d.write("app.log", &lines(4..6, "A"));
    let r = ws
        .refresh_tail(&[d.path("app.log"), d.path("app.log.1"), d.path("app.log.2")])
        .unwrap();
    assert_eq!(r.rotated.len(), 2);
    assert_eq!(ws.summary().total, 6, "{:?}", messages(&ws));
    let groups: Vec<String> = ws.summary().sources.iter().map(|s| s.group.clone()).collect();
    assert_eq!(groups, ["app.log", "app.log", "app.log"]);
    assert_eq!(source_of(&ws, "app.log.2").records, 2);
}

#[test]
fn serilog_rolls_to_a_new_dated_file() {
    let d = Dir::new("serilog");
    d.write("logs/log-20261016.txt", &lines(0..2, "A"));
    let mut ws = ws_on(&d.0);
    let new = d.write("logs/log-20261017.txt", &lines(2..3, "A"));
    let size = d.write("logs/log-20261017_001.txt", &lines(3..4, "A"));
    let r = ws.refresh_tail(&[new, size]).unwrap();
    assert_eq!(r.added.len(), 2);
    assert_eq!(ws.summary().total, 4);
    assert!(ws.summary().sources.iter().all(|s| s.group == "logs/log.txt"));
}

#[test]
fn copy_truncate_does_not_duplicate() {
    let d = Dir::new("copytrunc");
    let app = d.write("app.log", &lines(0..3, "A"));
    let mut ws = ws_on(&d.0);
    let id = source_of(&ws, "app.log").id;
    // Written after our last read, then copied away and truncated.
    d.append("app.log", &lines(3..4, "A"));
    let copy = d.path("app.log.1");
    std::fs::copy(&app, &copy).unwrap();
    std::fs::OpenOptions::new().write(true).open(&app).unwrap().set_len(0).unwrap();
    d.append("app.log", &lines(10..12, "A"));
    let r = ws.refresh_tail(&[app.clone(), copy.clone()]).unwrap();
    assert_eq!(r.truncated, [id]);
    assert_eq!(ws.summary().total, 4 + 2, "{:?}", messages(&ws));
    // The old content now lives in the copy (adopted, not imported again).
    assert_eq!(source_of(&ws, "app.log.1").id, id);
    assert_eq!(source_of(&ws, "app.log.1").records, 4);
    assert_eq!(source_of(&ws, "app.log").records, 2);
    let mut msgs = messages(&ws);
    msgs.dedup();
    assert_eq!(msgs.len(), 6);
}

#[test]
fn copy_seen_before_the_truncation_is_not_imported() {
    let d = Dir::new("copyfirst");
    let app = d.write("app.log", &lines(0..3, "A"));
    let mut ws = ws_on(&d.0);
    let copy = d.path("app.log.1");
    std::fs::copy(&app, &copy).unwrap();
    let r = ws.refresh_tail(std::slice::from_ref(&copy)).unwrap();
    assert!(r.added.is_empty());
    assert_eq!(ws.summary().total, 3);
    std::fs::write(&app, lines(20..21, "A")).unwrap();
    let r = ws.refresh_tail(std::slice::from_ref(&app)).unwrap();
    assert_eq!(r.truncated.len(), 1);
    assert_eq!(r.rotated.len(), 1);
    assert_eq!(ws.summary().total, 4);
}

#[test]
fn rewritten_in_place_is_a_truncation() {
    let d = Dir::new("rewrite");
    let app = d.write("app.log", &lines(0..3, "A"));
    let mut ws = ws_on(&d.0);
    // Same size or larger, different content.
    std::fs::write(&app, lines(30..34, "B")).unwrap();
    let r = ws.refresh_tail(std::slice::from_ref(&app)).unwrap();
    assert_eq!(r.truncated.len(), 1);
    assert_eq!(ws.summary().total, 3 + 4);
    let detached = ws
        .tail_status()
        .files
        .into_iter()
        .filter(|f| f.state == TailFileState::Truncated)
        .count();
    assert_eq!(detached, 1);
}

#[test]
fn logrotate_gzip_of_a_deleted_file_is_adopted() {
    let d = Dir::new("gzrot");
    let rotated = d.write("app.log.1", &lines(0..3, "A"));
    d.write("app.log", &lines(3..4, "A"));
    let mut ws = ws_on(&d.0);
    let id = source_of(&ws, "app.log.1").id;
    let content = std::fs::read(&rotated).unwrap();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&content).unwrap();
    let gz_path = d.path("app.log.2.gz");
    std::fs::write(&gz_path, gz.finish().unwrap()).unwrap();
    std::fs::remove_file(&rotated).unwrap();
    let r = ws.refresh_tail(&[rotated, gz_path.clone()]).unwrap();
    assert_eq!(r.deleted, [id]);
    assert_eq!(r.rotated.len(), 1);
    assert!(r.added.is_empty());
    assert_eq!(ws.summary().total, 4);
    assert_eq!(source_of(&ws, "app.log.2.gz").id, id);
    // An unrelated rotated archive appearing is imported once.
    let mut other = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    other.write_all(lines(40..42, "Z").as_bytes()).unwrap();
    let old = d.path("app.log.9.gz");
    std::fs::write(&old, other.finish().unwrap()).unwrap();
    assert_eq!(ws.refresh_tail(std::slice::from_ref(&old)).unwrap().added.len(), 1);
    assert_eq!(ws.refresh_tail(&[old]).unwrap().appended, 0);
    assert_eq!(ws.summary().total, 6);
}

#[test]
fn deleted_files_keep_history_unless_asked() {
    let d = Dir::new("deleted");
    let a = d.write("a.log", &lines(0..3, "A"));
    let b = d.write("b.log", &lines(3..5, "B"));
    let mut ws = ws_on(&d.0);
    std::fs::remove_file(&a).unwrap();
    let r = ws.refresh_tail(std::slice::from_ref(&a)).unwrap();
    assert_eq!(r.deleted.len(), 1);
    assert_eq!(ws.summary().total, 5);
    assert!(ws.tail_status().files.iter().any(|f| f.state == TailFileState::Deleted));
    ws.set_tail_options(TailOptions {
        deleted_files: DeletedFiles::Drop,
        ..TailOptions::default()
    });
    std::fs::remove_file(&b).unwrap();
    let r = ws.refresh_tail(std::slice::from_ref(&b)).unwrap();
    assert_eq!(r.deleted.len(), 1);
    assert_eq!(ws.summary().total, 3);
    assert_eq!(ws.summary().sources.len(), 1);
}

#[test]
fn new_files_in_the_folder_honour_include_and_exclude() {
    let d = Dir::new("newfiles");
    d.write("logs/a.log", &lines(0..1, "A"));
    let mut ws = ws_on(&d.0);
    let new = d.write("logs/deeper/b.log", &lines(1..2, "B"));
    let excluded = d.write("logs/b-filtered.log", &lines(2..3, "C"));
    let hidden = d.write("logs/.hidden", &lines(3..4, "D"));
    let r = ws.refresh_tail(&[new, excluded, hidden, d.path("logs/deeper")]).unwrap();
    assert_eq!(r.added.len(), 1);
    assert_eq!(ws.summary().total, 2);
    // An unrecognised file is not re-read until it changes.
    let junk = d.write("logs/junk.bin", "no events here\n");
    ws.refresh_tail(std::slice::from_ref(&junk)).unwrap();
    assert_eq!(ws.summary().unrecognized.len(), 1);
    assert!(ws.refresh_tail(std::slice::from_ref(&junk)).unwrap().is_empty());
    std::fs::write(&junk, lines(5..6, "J")).unwrap();
    assert_eq!(ws.refresh_tail(&[junk]).unwrap().added.len(), 1);
    assert!(ws.summary().unrecognized.is_empty());
    // A folder event looks at the folder's files.
    d.write("logs/c.log", &lines(6..7, "C"));
    assert_eq!(ws.refresh_tail(&[d.path("logs")]).unwrap().added.len(), 1);
}

#[test]
fn start_at_end_skips_history() {
    let d = Dir::new("atend");
    d.write("app.log", &(lines(0..50, "A") + "2026-10-14 10:00:59.000 +00:00 [INF] unterminat"));
    d.write("data.csv", "Timestamp,Message\n2026-10-14 10:00:00,a\n");
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    let mut store: &mut dyn LogStore = &mut ws;
    store
        .open_with(std::slice::from_ref(&d.0), &OpenOptions { start_at_end: true })
        .unwrap();
    assert_eq!(store.summary().total, 0);
    assert_eq!(store.summary().sources.len(), 2);
    d.append("app.log", "ed\n");
    d.append("app.log", &lines(60..62, "A"));
    d.append("data.csv", "2026-10-14 10:00:05,b\n");
    store = &mut ws;
    let r = store.refresh_tail(&[d.path("app.log"), d.path("data.csv")]).unwrap();
    assert_eq!(r.appended, 4, "{:?}", messages(&ws));
    let msgs = messages(&ws);
    assert!(
        msgs.contains(&"unterminated".to_string()) && msgs.contains(&"b".to_string()),
        "{msgs:?}"
    );
    let first = ws.records().iter().find(|r| r.message == "unterminated").unwrap();
    assert_eq!(first.line, 51);
}

/// A binary format: 4-byte magic then one record per remaining byte.
struct Blob;
impl BinaryImporter for Blob {
    fn id(&self) -> &'static str {
        "blob"
    }
    fn sniff(&self, _: &ImportCtx, head: &[u8]) -> u8 {
        if head.starts_with(b"BLB1") {
            100
        } else {
            0
        }
    }
    fn import(&self, _: &ImportCtx, bytes: &[u8]) -> anyhow::Result<Imported> {
        let records = bytes[4..]
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let mut r = Record::new(i as u32 + 1);
                r.ts = Some(1_700_000_000_000 + i as i64);
                r.message = format!("byte {b}");
                r
            })
            .collect();
        Ok(Imported {
            records,
            skipped: 0,
            ..Default::default()
        })
    }
}

#[test]
fn binary_formats_are_reimported_whole() {
    let d = Dir::new("binary");
    let p = d.0.join("perf.blg");
    std::fs::write(&p, b"BLB1abc").unwrap();
    let mut importers = Importers::new();
    importers.register_binary(Box::new(Blob));
    let mut ws = Workspace::with_importers(Profile::default_profile(), importers).unwrap();
    ws.open(std::slice::from_ref(&d.0)).unwrap();
    let id = source_of(&ws, "perf.blg").id;
    assert_eq!(source_of(&ws, "perf.blg").file_kind, imt_core::api::FileKind::Binary);
    assert_eq!(ws.summary().total, 3);
    assert!(ws.tail_status().files.iter().any(|f| f.state == TailFileState::Whole));
    std::fs::write(&p, b"BLB1abcde").unwrap();
    let r = ws.refresh_tail(std::slice::from_ref(&p)).unwrap();
    assert_eq!(r.reimported, [id]);
    assert_eq!(ws.summary().total, 5);
    assert_eq!(source_of(&ws, "perf.blg").id, id);
    // Unchanged: not read again.
    assert!(ws.refresh_tail(std::slice::from_ref(&p)).unwrap().reimported.is_empty());
    // Broken now: the error is shown, the old records stay.
    std::fs::write(&p, b"").unwrap();
    let r = ws.refresh_tail(&[p]).unwrap();
    assert!(r.reimported.is_empty());
    assert_eq!(ws.summary().total, 5);
    assert!(ws
        .tail_status()
        .files
        .iter()
        .any(|f| matches!(&f.state, TailFileState::Error { message } if message.contains("empty"))));
}

/// A text importer whose incremental hook crashes: the crash is contained and
/// the file is read whole instead.
struct Crashy;
impl Importer for Crashy {
    fn id(&self) -> &'static str {
        "crashy"
    }
    fn sniff(&self, _: &ImportCtx, head: &str) -> u8 {
        if head.starts_with("CRASHY") {
            100
        } else {
            0
        }
    }
    fn import(&self, _: &ImportCtx, content: &str) -> anyhow::Result<Imported> {
        let records = content
            .lines()
            .skip(1)
            .enumerate()
            .map(|(i, l)| {
                let mut r = Record::new(i as u32 + 2);
                r.ts = Some(1_700_000_000_000 + i as i64);
                r.message = l.to_string();
                r
            })
            .collect();
        Ok(Imported {
            records,
            skipped: 0,
            ..Default::default()
        })
    }
    fn import_tail(&self, _: &ImportCtx, _: &str, tail: &str) -> Option<anyhow::Result<Imported>> {
        if tail.contains("boom") {
            panic!("importer bug");
        }
        if tail.contains("fail") {
            return Some(Err(anyhow::anyhow!("cannot parse")));
        }
        // Says nothing about the re-read last record: inconsistent, so the file is read whole.
        Some(Ok(Imported::default()))
    }
}

#[test]
fn a_crashing_importer_is_contained() {
    let d = Dir::new("crashy");
    let p = d.write("x.txt", "CRASHY\na\n");
    let mut importers = Importers::empty();
    importers.register(Box::new(Crashy));
    let mut ws = Workspace::with_importers(Profile::default_profile(), importers).unwrap();
    ws.open(std::slice::from_ref(&d.0)).unwrap();
    d.append("x.txt", "boom\n");
    let r = ws.refresh_tail(std::slice::from_ref(&p)).unwrap();
    assert_eq!(r.reimported.len(), 1);
    assert_eq!(messages(&ws), ["a", "boom"]);
    d.append("x.txt", "fail\n");
    assert_eq!(ws.refresh_tail(std::slice::from_ref(&p)).unwrap().reimported.len(), 1);
    d.append("x.txt", "quiet\n");
    assert_eq!(ws.refresh_tail(std::slice::from_ref(&p)).unwrap().reimported.len(), 1);
    assert_eq!(messages(&ws), ["a", "boom", "fail", "quiet"]);
}

#[test]
fn utf16_files_append() {
    let d = Dir::new("utf16");
    let enc = |s: &str| -> Vec<u8> { s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect() };
    let p = d.path("win.log");
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend(enc(&lines(0..2, "W")));
    std::fs::write(&p, &bytes).unwrap();
    let mut ws = ws_on(&d.0);
    assert_eq!(ws.summary().total, 2);
    let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
    f.write_all(&enc(&lines(2..4, "W"))).unwrap();
    let r = ws.refresh_tail(&[p]).unwrap();
    assert_eq!(r.appended, 2);
    assert_eq!(ws.records()[3].message, "event 3");
    assert_eq!(ws.records()[3].fields["MachineName"], "W");
}

#[test]
fn identity_is_stable_across_renames() {
    let d = Dir::new("identity");
    let a = d.write("a.log", "x\n");
    let before = identity_of(&a);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let m = std::fs::metadata(&a).unwrap();
        assert_eq!(before.map(|i| (i.device, i.index)), Some((m.dev(), m.ino())));
    }
    let b = d.path("b.log");
    std::fs::rename(&a, &b).unwrap();
    assert_eq!(identity_of(&b), before);
    assert_eq!(identity_of(&a), None);
}

#[test]
fn pipeline_extractors_and_markers_apply_to_appended_records() {
    let d = Dir::new("pipeline");
    let profile = Profile::from_toml(
        r#"
[[extract]]
name = "cpu"
from = "@message"
regex = 'cpu=(?P<cpu>\d+)'
[extract.fields]
cpu = { type = "number" }

[[marker]]
kind = "boom"
label = "Boom"
when = [{ field = "@message", op = "contains", value = "boom" }]

[[pipeline]]
step = "drop"
when = [{ field = "@message", op = "contains", value = "noise" }]
"#,
    )
    .unwrap();
    let p = d.write("app.log", &line(0, "INF", "A", "start cpu=5"));
    let mut ws = Workspace::new(profile).unwrap();
    ws.open(std::slice::from_ref(&d.0)).unwrap();
    d.append("app.log", &(line(1, "INF", "A", "noise") + &*line(2, "ERR", "A", "boom cpu=97")));
    let r = ws.refresh_tail(&[p]).unwrap();
    assert_eq!(r.appended, 1);
    let rec = ws.records().last().unwrap();
    assert_eq!(rec.fields["cpu"], 97.0);
    let summary = ws.summary();
    let boom = summary.marker_kinds.iter().position(|m| m.kind == "boom").unwrap();
    assert_ne!(rec.markers & (1 << boom), 0);
    let s = source_of(&ws, "app.log");
    assert_eq!(s.dropped, 1);
    assert_eq!(summary.pipeline[0].input, 3);
    assert_eq!(summary.pipeline[0].output, 2);
    assert!(summary.fields.iter().any(|f| f.name == "cpu" && f.derived && f.numeric));
}

// ---------------------------------------------------------------------------
// Incremental derived state equals a full recompute.

fn base_content() -> String {
    let mut s = String::new();
    s += &line(0, "INF", "WEB01", "Application started");
    for i in 1..40 {
        let machine = if i % 2 == 0 { "WEB01" } else { "WEB02" };
        s += &line(i, "INF", machine, &format!("Request {i} handled"));
    }
    s += &line(40, "ERR", "WEB02", "Payment failed");
    s += "System.TimeoutException: timed out\n   at Pay.Charge()\n";
    s
}

fn appended_content() -> String {
    let mut s = String::new();
    s += "   at Pay.Api()\n";
    for i in 41..80 {
        let machine = ["WEB01", "WEB02", "WEB03"][i as usize % 3];
        s += &line(i, "INF", machine, &format!("Request {i} handled"));
    }
    s += &line(80, "ERR", "WEB03", "Payment failed");
    s += "System.TimeoutException: timed out\n   at Pay.Charge()\n";
    s += &line(81, "INF", "WEB01", "Application stopping");
    s
}

fn every_view(ws: &Workspace) -> Vec<(&'static str, serde_json::Value)> {
    let q = Query::default();
    let mut s = ws.summary();
    s.load_ms = 0;
    for src in &mut s.sources {
        src.path.clear();
    }
    let qr = QueryRequest { query: q.clone() };
    vec![
        ("summary", json(&s)),
        (
            "timeline",
            json(
                ws.timeline(&TimelineRequest {
                    query: q.clone(),
                    buckets: 50,
                    always_show_lifecycle: true,
                })
                .unwrap(),
            ),
        ),
        (
            "facet",
            json(
                ws.facet(&FacetRequest {
                    query: q.clone(),
                    field: "MachineName".into(),
                    limit: 10,
                })
                .unwrap(),
            ),
        ),
        (
            "chart",
            json(
                ws.chart(&serde_json::from_value(serde_json::json!({ "chartId": "events-by-level", "buckets": 20 })).unwrap())
                    .unwrap(),
            ),
        ),
        (
            "page",
            json(
                ws.page(&PageRequest {
                    query: Query {
                        text: "Request".into(),
                        ..Query::default()
                    },
                    offset: 0,
                    limit: 500,
                    descending: false,
                    anchor: None,
                })
                .unwrap(),
            ),
        ),
        ("patterns", json(ws.patterns(&qr).unwrap())),
        (
            "field stats",
            json(
                ws.field_stats(&imt_core::api::FieldStatsRequest {
                    query: q.clone(),
                    field: "MachineName".into(),
                    buckets: 10,
                })
                .unwrap(),
            ),
        ),
        (
            "ql stats",
            json(
                ws.run_query(&QueryRequest {
                    query: Query {
                        ql: "level >= information | stats count() by MachineName".into(),
                        ..Query::default()
                    },
                })
                .unwrap(),
            ),
        ),
        ("exceptions", json(ws.exception_groups(&qr).unwrap())),
        ("anomalies", json(ws.anomalies(&qr).unwrap())),
        (
            "requests",
            json(
                ws.requests(&RequestsRequest {
                    query: q.clone(),
                    id_field: "MachineName".into(),
                })
                .unwrap(),
            ),
        ),
        (
            "traces",
            json(
                ws.traces(&TraceRequest {
                    query: q.clone(),
                    trace_id: String::new(),
                })
                .unwrap()
                .0,
            ),
        ),
        (
            "moment",
            json(
                ws.moment(&MomentRequest {
                    query: q.clone(),
                    from: 0,
                    to: i64::MAX / 2,
                })
                .unwrap(),
            ),
        ),
    ]
}

#[test]
fn incremental_derived_state_equals_a_full_recompute() {
    let live = Dir::new("inc-live");
    let p = live.write("app.log", &base_content());
    let mut ws = ws_on(&live.0);
    // Warm every cache so the incremental paths are exercised.
    let before = every_view(&ws);
    assert!(before.iter().all(|(_, v)| !v.is_null()));
    live.append("app.log", &appended_content());
    let r = ws.refresh_tail(&[p]).unwrap();
    assert_eq!(r.instances_changed, ["WEB03"]);
    assert_eq!(r.updated, 1);
    assert_eq!(r.ids_changed_from, None);

    let full = Dir::new("inc-full");
    full.write("app.log", &(base_content() + &*appended_content()));
    let fresh = ws_on(&full.0);
    for ((name, inc), (_, exp)) in every_view(&ws).into_iter().zip(every_view(&fresh)) {
        assert_eq!(inc, exp, "{name} differs after an incremental append");
    }
    // A new instance became a lane; the stop marker shows in its lifecycle.
    let tl = json(
        ws.timeline(&TimelineRequest {
            query: Query::default(),
            buckets: 50,
            always_show_lifecycle: true,
        })
        .unwrap(),
    );
    assert!(tl.to_string().contains("WEB03"));
    assert!(tl.to_string().contains("\"stop\""));
}

#[test]
fn out_of_order_appends_renumber_only_the_tail() {
    let d = Dir::new("order");
    let a = d.write("a.log", &lines(0..5, "A"));
    let b = d.write("b.log", &lines(10..12, "B"));
    let mut ws = ws_on(&d.0);
    let q = Query {
        text: "event".into(),
        ..Query::default()
    };
    let page = |ws: &Workspace| {
        json(
            ws.page(&PageRequest {
                query: q.clone(),
                offset: 0,
                limit: 100,
                descending: false,
                anchor: None,
            })
            .unwrap(),
        )
    };
    page(&ws);
    // a.log gets a record older than b.log's newest.
    d.append("a.log", &lines(6..7, "A"));
    let r = ws.refresh_tail(&[a, b]).unwrap();
    assert_eq!(r.ids_changed_from, Some(5));
    assert!(ws.records().windows(2).all(|w| w[0].ts <= w[1].ts));
    assert!(ws.records().iter().enumerate().all(|(i, r)| r.id == i as u32));
    let full = Dir::new("order-full");
    full.write("a.log", &(lines(0..5, "A") + &*lines(6..7, "A")));
    full.write("b.log", &lines(10..12, "B"));
    assert_eq!(page(&ws), page(&ws_on(&full.0)));
}

#[test]
fn live_windows_slide_with_new_records() {
    let d = Dir::new("window");
    let p = d.write("app.log", &lines(0..30, "A"));
    let mut ws = ws_on(&d.0);
    let q = Query {
        window: Some(TimeWindow::Live { last_ms: 10_000 }),
        ..Query::default()
    };
    let count = |ws: &Workspace| ws.matching(&q).unwrap().len();
    assert_eq!(count(&ws), 11);
    d.append("app.log", &lines(30..40, "A"));
    ws.refresh_tail(&[p]).unwrap();
    assert_eq!(count(&ws), 11);
    assert_eq!(ws.records()[ws.matching(&q).unwrap()[0] as usize].message, "event 29");
    let tl = ws
        .timeline(&TimelineRequest {
            query: q.clone(),
            buckets: 10,
            always_show_lifecycle: false,
        })
        .unwrap();
    assert_eq!(json(&tl)["from"], json(ws.records()[29].ts));
    let m = ws
        .moment(&MomentRequest {
            query: q.clone(),
            from: 0,
            to: 0,
        })
        .unwrap();
    assert_eq!(m.total, 11);
    assert!(ws.anomalies(&QueryRequest { query: q.clone() }).is_ok());
    let wall = Profile::from_toml("[live]\nclock = \"wall\"").unwrap();
    ws.set_profile(wall).unwrap();
    assert_eq!(count(&ws), 0);
    assert!(ws.live_now() > ws.newest_ts().unwrap());
}

#[test]
fn live_alerts_rules_silence_and_detectors() {
    let d = Dir::new("alerts");
    let profile = Profile::from_toml(
        r#"
[live]
silence_after = "20s"
cooldown = "1h"
[[live.rule]]
name = "payment errors"
when = 'level = "Error" and message contains "Payment"'
threshold = 3
window = "1m"
severity = "critical"
"#,
    )
    .unwrap();
    let mut base = String::new();
    for i in 0..60 {
        base += &line(i, "INF", "WEB01", &format!("tick {i}"));
        if i < 30 {
            base += &line(i, "INF", "WEB02", &format!("tock {i}"));
        }
    }
    let p = d.write("app.log", &base);
    let mut ws = Workspace::new(profile).unwrap();
    ws.open(std::slice::from_ref(&d.0)).unwrap();
    ws.set_tail_options(TailOptions {
        detect_every_ms: 0,
        ..TailOptions::default()
    });
    // First evaluation sets "now"; WEB02 has been silent since second 29.
    d.append("app.log", &line(60, "INF", "WEB01", "tick"));
    let r = ws.refresh_tail(std::slice::from_ref(&p)).unwrap();
    assert!(r.alerts.iter().all(|a| a.kind != "silence"));
    let mut errors = String::new();
    for i in 61..64 {
        errors += &line(i, "ERR", "WEB01", "Payment failed");
    }
    d.append("app.log", &errors);
    let r = ws.refresh_tail(std::slice::from_ref(&p)).unwrap();
    let rule = r.alerts.iter().find(|a| a.kind == "rule").expect("rule alert");
    assert_eq!(rule.rule.as_deref(), Some("payment errors"));
    assert_eq!(json(rule.severity), "critical");
    assert_eq!(r.alerts_raised, r.alerts.len());
    // Cooldown: the same rule does not fire again.
    d.append("app.log", &line(64, "ERR", "WEB01", "Payment failed"));
    let r = ws.refresh_tail(std::slice::from_ref(&p)).unwrap();
    assert!(r.alerts.iter().all(|a| a.kind != "rule"));

    // An instance going quiet while others keep logging.
    let d2 = Dir::new("alerts-silence");
    let mut base = String::new();
    for i in 0..10 {
        base += &line(i, "INF", "WEB01", "a");
        base += &line(i, "INF", "WEB02", "b");
    }
    let p2 = d2.write("app.log", &base);
    let mut ws = Workspace::new(Profile::from_toml("[live]\nsilence_after = \"20s\"").unwrap()).unwrap();
    ws.open(std::slice::from_ref(&d2.0)).unwrap();
    d2.append("app.log", &line(10, "INF", "WEB01", "a"));
    assert!(ws.refresh_tail(std::slice::from_ref(&p2)).unwrap().alerts.is_empty());
    d2.append("app.log", &lines(11..40, "WEB01"));
    let r = ws.refresh_tail(std::slice::from_ref(&p2)).unwrap();
    let silence: Vec<_> = r.alerts.iter().filter(|a| a.kind == "silence").collect();
    assert_eq!(silence.len(), 1, "{:?}", r.alerts);
    assert_eq!(silence[0].instance.as_deref(), Some("WEB02"));
    assert_eq!(silence[0].evidence_query.groups, ["WEB02"]);
    let payload = serde_json::to_value(&r).unwrap();
    assert_eq!(payload["alertsRaised"], 1);
    assert!(payload.get("alerts").is_none());

    // A burst of errors after a steady stretch is an error spike.
    let d3 = Dir::new("alerts-spike");
    let mut base = String::new();
    for i in 0..600 {
        base += &line(i, "INF", "WEB01", "steady");
    }
    let p3 = d3.write("app.log", &base);
    let mut ws = ws_on(&d3.0);
    ws.set_tail_options(TailOptions {
        detect_every_ms: 0,
        ..TailOptions::default()
    });
    d3.append("app.log", &line(600, "INF", "WEB01", "steady"));
    ws.refresh_tail(std::slice::from_ref(&p3)).unwrap();
    let burst: String = (0..40).map(|_| line(601, "ERR", "WEB01", "Database down")).collect();
    d3.append("app.log", &burst);
    let r = ws.refresh_tail(&[p3]).unwrap();
    let spike = r.alerts.iter().find(|a| a.kind == "errorSpike").expect("error spike");
    assert_eq!(json(spike.severity), "critical");
    assert_eq!(spike.evidence_query.levels, [Level::Error, Level::Fatal]);
}

#[test]
fn tail_status_reports_files_and_totals() {
    let d = Dir::new("status");
    let p = d.write("logs/app.log.1", &lines(0..3, "A"));
    d.write("logs/app.log", &lines(3..4, "A"));
    let mut ws = ws_on(&d.0);
    d.append("logs/app.log.1", &lines(4..6, "A"));
    ws.refresh_tail(&[p]).unwrap();
    let st = ws.tail_status();
    assert_eq!(st.files.len(), 2);
    let f = st.files.iter().find(|f| f.name == "logs/app.log.1").unwrap();
    assert_eq!(f.group, "logs/app.log");
    assert_eq!(f.state, TailFileState::Tailing);
    assert_eq!(f.mode, WatchMode::Off);
    assert_eq!(f.offset, f.size);
    assert!(f.lines_per_sec > 0.0 && f.last_event.is_some());
    assert_eq!(st.totals.files, 2);
    assert_eq!(st.totals.tailing, 2);
    assert_eq!(st.totals.records, 6);
    let with = st.with_modes(|_| WatchMode::Polling);
    assert!(with.files.iter().all(|f| f.mode == WatchMode::Polling));
    let v = json(&with);
    assert_eq!(v["files"][0]["state"]["kind"], "tailing");
    assert_eq!(v["files"][0]["mode"], "polling");
    // Through the store trait.
    let store: Box<dyn LogStore> = Box::new(ws);
    assert_eq!(store.tail_status().files.len(), 2);
}

// ---------------------------------------------------------------------------
// Watchers: coalescing and polling fallback through an injected factory.

#[derive(Default)]
struct FakeFactory {
    fail_events: bool,
    fail_polling: bool,
    created: Mutex<Vec<WatchMode>>,
    sinks: Mutex<Vec<WatchSink>>,
}

struct FakeWatcher(Option<String>);
impl FileWatcher for FakeWatcher {
    fn watch(&mut self, path: &Path, _recursive: bool) -> anyhow::Result<()> {
        match &self.0 {
            Some(bad) if path.ends_with(bad) => anyhow::bail!("cannot watch {bad}"),
            _ => Ok(()),
        }
    }
}

impl WatcherFactory for FakeFactory {
    fn create(&self, mode: WatchMode, _poll: Duration, sink: WatchSink) -> anyhow::Result<Box<dyn FileWatcher>> {
        if mode == WatchMode::Events && self.fail_events {
            anyhow::bail!("inotify limit reached");
        }
        if mode == WatchMode::Polling && self.fail_polling {
            anyhow::bail!("no polling either");
        }
        self.created.lock().unwrap().push(mode);
        self.sinks.lock().unwrap().push(sink);
        Ok(Box::new(FakeWatcher(Some(
            if mode == WatchMode::Events { "flaky" } else { "dead" }.to_string(),
        ))))
    }
}

fn wait_until(mut f: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

#[test]
fn a_burst_of_ten_thousand_writes_is_one_refresh() {
    let s = WatchSettings::default();
    let mut co = Coalescer::new(s);
    let t0 = Instant::now();
    for i in 0..10_000u64 {
        co.push([PathBuf::from("/logs/app.log")], t0 + Duration::from_micros(i * 10));
    }
    assert!(co.due(t0 + Duration::from_millis(300)));
    assert_eq!(co.take(t0 + Duration::from_millis(300)).unwrap().len(), 1);

    let d = Dir::new("burst");
    let factory = Arc::new(FakeFactory::default());
    let batches = Arc::new(Mutex::new(Vec::<Vec<PathBuf>>::new()));
    let sink_batches = batches.clone();
    let live = LiveTail::start(
        std::slice::from_ref(&d.0),
        WatchSettings {
            tick_ms: 60_000,
            ..WatchSettings::default()
        },
        factory.clone(),
        Box::new(move |b| sink_batches.lock().unwrap().push(b)),
        Box::new(|_, _| {}),
    );
    let sink = factory.sinks.lock().unwrap()[0].clone();
    let burst = Instant::now();
    for _ in 0..10_000 {
        sink(WatchSignal::Paths(vec![d.0.join("app.log")]));
    }
    let burst = burst.elapsed();
    assert!(wait_until(|| !batches.lock().unwrap().is_empty()));
    std::thread::sleep(Duration::from_millis(400));
    let got = batches.lock().unwrap().clone();
    // One refresh per `max_delay_ms` of burst (plus one: the watcher may still be draining the sends when the
    // sender is done) at most, each naming the file once: a slow debug or coverage build needs more than one
    // window for the ten thousand sends.
    let windows = 2 + (burst.as_millis() / u128::from(WatchSettings::default().max_delay_ms)) as usize;
    assert!(
        !got.is_empty() && got.len() <= windows,
        "{} refreshes for a burst of {burst:?}",
        got.len()
    );
    assert!(got.iter().all(|b| b == &[d.0.join("app.log")]), "{got:?}");
    assert_eq!(live.mode_of(&d.0.join("app.log")), WatchMode::Events);
    drop(live);

    // The workspace side: one refresh reads all 10k lines appended in the burst.
    let p = d.write("app.log", &lines(0..1, "A"));
    let mut ws = ws_on(&d.0);
    let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
    for i in 0..10_000u32 {
        f.write_all(format!("2026-10-14 11:00:00.{:03} +00:00 [INF] burst {i}\n", i % 1000).as_bytes())
            .unwrap();
    }
    let r = ws.refresh_tail(&[p]).unwrap();
    assert_eq!(r.appended, 10_000);
    assert!(r.reimported.is_empty());
}

#[test]
fn watchers_fall_back_to_polling() {
    let d = Dir::new("fallback");
    std::fs::create_dir_all(d.0.join("flaky")).unwrap();
    std::fs::create_dir_all(d.0.join("ok")).unwrap();
    let file = d.write("solo/app.log", "x\n");
    // Events unavailable: everything polls.
    let factory = Arc::new(FakeFactory {
        fail_events: true,
        ..FakeFactory::default()
    });
    let fallbacks = Arc::new(AtomicUsize::new(0));
    let fb = fallbacks.clone();
    let live = LiveTail::start(
        &[d.0.join("ok"), file.clone()],
        WatchSettings::default(),
        factory.clone(),
        Box::new(|_| {}),
        Box::new(move |_, why| {
            assert!(why.contains("inotify"));
            fb.fetch_add(1, Ordering::SeqCst);
        }),
    );
    assert!(live.roots().iter().all(|r| r.mode == WatchMode::Polling));
    assert_eq!(fallbacks.load(Ordering::SeqCst), 2);
    // A file root is watched through its folder (to see its rotated replacement).
    assert!(live.roots().iter().any(|r| r.path == d.0.join("solo") && !r.recursive));
    assert_eq!(*factory.created.lock().unwrap(), [WatchMode::Polling]);
    drop(live);

    // One root cannot be watched with events: only it polls. A runtime watcher
    // error moves the rest to polling too, and everything is rescanned.
    let factory = Arc::new(FakeFactory::default());
    let batches = Arc::new(Mutex::new(Vec::<Vec<PathBuf>>::new()));
    let b = batches.clone();
    let live = LiveTail::start(
        &[d.0.join("flaky"), d.0.join("ok")],
        WatchSettings {
            tick_ms: 60_000,
            ..WatchSettings::default()
        },
        factory.clone(),
        Box::new(move |x| b.lock().unwrap().push(x)),
        Box::new(|_, _| {}),
    );
    let modes: Vec<WatchMode> = live.roots().iter().map(|r| r.mode).collect();
    assert_eq!(modes, [WatchMode::Polling, WatchMode::Events]);
    assert!(live.roots()[0].error.as_deref().unwrap().contains("cannot watch"));
    live.signal(WatchSignal::Error("event queue overflow".into()));
    assert!(wait_until(|| live.roots().iter().all(|r| r.mode == WatchMode::Polling)));
    assert!(wait_until(|| batches.lock().unwrap().iter().any(|b| b.len() == 2)));
    assert_eq!(live.mode_of(&d.0.join("ok/x.log")), WatchMode::Polling);

    // Forced polling and network paths.
    let polled = LiveTail::start(
        &[d.0.join("ok")],
        WatchSettings {
            preference: WatchPreference::Polling,
            ..WatchSettings::default()
        },
        Arc::new(FakeFactory::default()),
        Box::new(|_| {}),
        Box::new(|_, _| {}),
    );
    assert_eq!(polled.roots()[0].mode, WatchMode::Polling);
    assert_eq!(
        imt_core::workspace::initial_mode(WatchPreference::Auto, Path::new(r"\\nas\logs")),
        WatchMode::Polling
    );
}

#[test]
fn ticks_flush_partial_lines() {
    let factory = Arc::new(FakeFactory::default());
    let ticks = Arc::new(AtomicUsize::new(0));
    let t = ticks.clone();
    let live = LiveTail::start(
        &[std::env::temp_dir()],
        WatchSettings {
            tick_ms: 20,
            ..WatchSettings::default()
        },
        factory,
        Box::new(move |b| {
            if b.is_empty() {
                t.fetch_add(1, Ordering::SeqCst);
            }
        }),
        Box::new(|_, _| {}),
    );
    assert!(wait_until(|| ticks.load(Ordering::SeqCst) >= 2));
    drop(live);

    // Polled roots are re-checked on every tick (pollers miss changes within
    // their timestamp resolution).
    let d = Dir::new("tick-poll");
    let seen = Arc::new(AtomicUsize::new(0));
    let s2 = seen.clone();
    let root = d.0.clone();
    let live = LiveTail::start(
        std::slice::from_ref(&d.0),
        WatchSettings {
            tick_ms: 20,
            preference: WatchPreference::Polling,
            ..WatchSettings::default()
        },
        Arc::new(FakeFactory::default()),
        Box::new(move |b| {
            if b == [root.clone()] {
                s2.fetch_add(1, Ordering::SeqCst);
            }
        }),
        Box::new(|_, _| {}),
    );
    assert!(wait_until(|| seen.load(Ordering::SeqCst) >= 2));
    drop(live);

    // A folder refresh checks every tailed file below it.
    let p = d.write("a/b/deep.log", &lines(0..1, "A"));
    let mut ws = ws_on(&d.0);
    d.append("a/b/deep.log", &lines(1..2, "A"));
    assert_eq!(ws.refresh_tail(std::slice::from_ref(&d.0)).unwrap().appended, 1);
    assert_eq!(ws.refresh_tail(&[p]).unwrap().appended, 0);
}

/// Optional smoke test with the real OS watcher is in the Tauri shell; this
/// benchmark appends 100k lines to a 1M-line workspace and reports timings.
/// Run with `cargo test --release -p imt-core --test tail -- --ignored --nocapture`.
#[test]
#[ignore]
fn bench_append_100k_to_1m() {
    let d = Dir::new("bench");
    let p = d.path("big.log");
    {
        let mut f = std::io::BufWriter::new(std::fs::File::create(&p).unwrap());
        for i in 0..1_000_000u64 {
            let (s, ms) = (i / 1000, i % 1000);
            writeln!(
                f,
                "2026-10-14 {:02}:{:02}:{:02}.{ms:03} +00:00 [INF] Request {i} handled in {} ms {{\"MachineName\": \"WEB0{}\"}}",
                (s / 3600) % 24,
                (s / 60) % 60,
                s % 60,
                i % 97,
                i % 4
            )
            .unwrap();
        }
    }
    let t = Instant::now();
    let mut ws = ws_on(&d.0);
    println!("open 1M lines: {:?}", t.elapsed());
    let q = Query {
        text: "handled".into(),
        ..Query::default()
    };
    ws.matching(&q).unwrap();
    // Sustained load: 5k lines/s delivered every 250 ms = 1250-line batches.
    let mut next = 1_000_000u64;
    let mut write = |n: u64| {
        let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        let mut buf = String::new();
        for i in next..next + n {
            let (s, ms) = (i / 1000, i % 1000);
            buf += &format!(
                "2026-10-14 {:02}:{:02}:{:02}.{ms:03} +00:00 [INF] Request {i} handled {{\"MachineName\": \"WEB0{}\"}}\n",
                (s / 3600) % 24,
                (s / 60) % 60,
                s % 60,
                i % 4
            );
        }
        f.write_all(buf.as_bytes()).unwrap();
        next += n;
    };
    ws.refresh_tail(std::slice::from_ref(&p)).unwrap();
    let mut worst = Duration::ZERO;
    let mut total = Duration::ZERO;
    for _ in 0..20 {
        write(1250);
        let t = Instant::now();
        let r = ws.refresh_tail(std::slice::from_ref(&p)).unwrap();
        ws.matching(&q).unwrap();
        let e = t.elapsed();
        assert_eq!(r.appended, 1250);
        worst = worst.max(e);
        total += e;
    }
    println!("5k lines/s: 1250-line batch avg {:?}, worst {:?}", total / 20, worst);
    write(100_000);
    let t = Instant::now();
    let r = ws.refresh_tail(std::slice::from_ref(&p)).unwrap();
    println!("append 100k lines to 1M: {:?} ({} appended)", t.elapsed(), r.appended);
    let t = Instant::now();
    assert_eq!(ws.matching(&q).unwrap().len(), ws.summary().total);
    println!("cached query after append: {:?}", t.elapsed());
}

#[test]
fn watchers_that_cannot_poll_are_off() {
    let d = Dir::new("nopoll");
    std::fs::create_dir_all(d.0.join("dead")).unwrap();
    std::fs::create_dir_all(d.0.join("flaky")).unwrap();
    let live = LiveTail::start(
        &[d.0.join("dead")],
        WatchSettings {
            preference: WatchPreference::Polling,
            ..WatchSettings::default()
        },
        Arc::new(FakeFactory::default()),
        Box::new(|_| {}),
        Box::new(|_, _| {}),
    );
    assert_eq!(live.roots()[0].mode, WatchMode::Off);
    let live = LiveTail::start(
        &[d.0.join("flaky")],
        WatchSettings::default(),
        Arc::new(FakeFactory {
            fail_polling: true,
            ..FakeFactory::default()
        }),
        Box::new(|_| {}),
        Box::new(|_, _| {}),
    );
    assert_eq!(live.roots()[0].mode, WatchMode::Off);
    assert!(live.roots()[0].error.as_deref().unwrap().contains("cannot poll"));
}

#[test]
fn json_without_a_final_newline_is_read_again_when_completed() {
    let d = Dir::new("json-eof");
    let ev = |i: u32| format!("{{\"@t\":\"2026-10-14T10:00:{i:02}Z\",\"@m\":\"m{i}\"}}");
    let p = d.write("a.json", &format!("{}\n{}", ev(0), ev(1)));
    let mut ws = ws_on(&d.0);
    assert_eq!(ws.summary().total, 2);
    d.append("a.json", &format!("\n{}\n", ev(2)));
    let r = ws.refresh_tail(std::slice::from_ref(&p)).unwrap();
    assert_eq!((r.appended, r.updated), (1, 0));
    assert_eq!(ws.summary().total, 3);
    // Flushed early, then completed: replaced, not duplicated.
    ws.set_tail_options(TailOptions {
        carry_flush_ms: 0,
        ..ws.tail_options()
    });
    d.append("a.json", "{\"@t\":\"2026-10-14T10:00:03Z\",\"@m\":\"m3\"");
    ws.refresh_tail(std::slice::from_ref(&p)).unwrap();
    d.append("a.json", "}\n");
    ws.refresh_tail(std::slice::from_ref(&p)).unwrap();
    assert_eq!(messages(&ws), ["m0", "m1", "m2", "m3"]);
    assert_eq!(source_of(&ws, "a.json").skipped, 1);
}

#[test]
fn large_backlogs_and_endless_records_are_bounded() {
    let d = Dir::new("backlog");
    let p = d.write("app.log", &lines(0..1, "A"));
    let mut ws = ws_on(&d.0);
    // More than one step's worth (8 MiB) appended at once.
    let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
    let mut n = 0u32;
    let mut buf = String::new();
    while buf.len() < 9 << 20 {
        buf += &format!("2026-10-14 11:00:00.000 +00:00 [INF] backlog line {n} {}\n", "y".repeat(100));
        n += 1;
    }
    f.write_all(buf.as_bytes()).unwrap();
    let r = ws.refresh_tail(std::slice::from_ref(&p)).unwrap();
    assert_eq!(r.appended as u32, n);
    assert_eq!(ws.summary().total as u32, n + 1);

    // A record that never ends (9 MiB of continuation lines) stops being re-read.
    let d2 = Dir::new("endless");
    let mut text = line(0, "ERR", "A", "dump follows");
    while text.len() < 9 << 20 {
        text += "    continuation of a very long dump\n";
    }
    let p2 = d2.write("app.log", &text);
    let mut ws = ws_on(&d2.0);
    d2.append("app.log", "    more dump\n");
    d2.append("app.log", &line(1, "INF", "A", "after"));
    let r = ws.refresh_tail(std::slice::from_ref(&p2)).unwrap();
    assert_eq!(r.appended, 1);
    assert_eq!(ws.records().last().unwrap().message, "after");
}

#[test]
fn changed_bytes_before_the_offset_mean_a_rewrite() {
    let d = Dir::new("anchor");
    let p = d.write("app.log", &lines(0..30, "A"));
    let mut ws = ws_on(&d.0);
    // Same start, same size or more, but the end was rewritten.
    let mut text = lines(0..29, "A");
    text += &line(29, "INF", "A", "event X");
    text += &lines(30..31, "A");
    std::fs::write(&p, text).unwrap();
    let r = ws.refresh_tail(std::slice::from_ref(&p)).unwrap();
    assert_eq!(r.truncated.len(), 1);
    // A file that only shares the first kilobyte is not a copy of it.
    let partial: String = lines(0..30, "A").chars().take(1500).collect();
    let other = d.write("app.log.7", &partial);
    assert_eq!(ws.refresh_tail(&[other]).unwrap().added.len(), 1);
}

#[test]
fn copies_that_vanish_or_change_are_not_adopted() {
    let d = Dir::new("copies");
    let app = d.write("app.log", &lines(0..3, "A"));
    let mut ws = ws_on(&d.0);
    let (a, b, c) = (d.path("app.log.a"), d.path("app.log.b"), d.path("app.log.c"));
    for copy in [&a, &b, &c] {
        std::fs::copy(&app, copy).unwrap();
    }
    assert!(ws.refresh_tail(&[a.clone(), b.clone(), c.clone()]).unwrap().added.is_empty());
    std::fs::remove_file(&a).unwrap();
    std::fs::write(&b, lines(50..53, "Q")).unwrap();
    std::fs::write(&app, lines(60..61, "A")).unwrap();
    let r = ws.refresh_tail(std::slice::from_ref(&app)).unwrap();
    assert_eq!(r.rotated.len(), 1);
    assert!(r.rotated[0].to.ends_with("app.log.c"));
    // The file deleted and created again under the same name.
    std::fs::remove_file(&app).unwrap();
    d.write("app.log", &lines(70..71, "A"));
    let r = ws.refresh_tail(std::slice::from_ref(&app)).unwrap();
    assert_eq!(r.deleted.len() + r.truncated.len(), 1);
    assert_eq!(r.added.len(), 1);
    assert_eq!(ws.summary().total, 3 + 1 + 1);
}

#[test]
fn zip_archives_are_read_again_when_they_change() {
    let d = Dir::new("zip");
    let zip_of = |text: &str| {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        w.start_file("inner.log", zip::write::SimpleFileOptions::default()).unwrap();
        w.write_all(text.as_bytes()).unwrap();
        w.finish().unwrap().into_inner()
    };
    let p = d.0.join("logs.zip");
    std::fs::write(&p, zip_of(&lines(0..2, "Z"))).unwrap();
    let mut ws = ws_on(&d.0);
    assert_eq!(ws.summary().total, 2);
    std::fs::write(&p, zip_of(&lines(0..3, "Z"))).unwrap();
    let r = ws.refresh_tail(std::slice::from_ref(&p)).unwrap();
    assert_eq!(r.reimported.len(), 1);
    assert_eq!(ws.summary().total, 3);
}

#[test]
fn start_at_end_falls_back_for_what_cannot_be_tailed() {
    let d = Dir::new("atend-fallback");
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(lines(0..2, "G").as_bytes()).unwrap();
    std::fs::write(d.path("old.log.gz"), gz.finish().unwrap()).unwrap();
    d.write("empty.log", "");
    std::fs::write(d.path("perf.blg"), b"BLB1ab").unwrap();
    let profile =
        Profile::from_toml("[[pipeline]]\nstep = \"drop\"\nwhen = [{ field = \"@message\", op = \"contains\", value = \"noise\" }]")
            .unwrap();
    let mut importers = Importers::new();
    importers.register_binary(Box::new(Blob));
    let mut ws = Workspace::with_importers(profile, importers).unwrap();
    let p = d.write("app.log", &lines(0..2, "A"));
    ws.open_with(std::slice::from_ref(&d.0), &OpenOptions { start_at_end: true })
        .unwrap();
    // The archive and the binary file are imported; the empty file is unrecognised; app.log skipped.
    assert_eq!(ws.summary().total, 2 + 2);
    assert_eq!(ws.summary().unrecognized.len(), 1);
    d.append("app.log", &(line(5, "INF", "A", "noise") + &*line(6, "INF", "A", "kept")));
    ws.refresh_tail(&[p]).unwrap();
    let s = source_of(&ws, "app.log");
    assert_eq!((s.records, s.dropped), (1, 1));
    // Paths in folders that do not exist, and an empty refresh of an empty workspace.
    assert!(ws.refresh_tail(&[PathBuf::from("/no/such/dir/x.log")]).unwrap().is_empty());
    let mut empty = Workspace::new(Profile::default_profile()).unwrap();
    assert!(empty.refresh_tail(&[]).unwrap().is_empty());
}

#[test]
fn rates_keep_a_bounded_history() {
    let d = Dir::new("rates");
    let p = d.write("app.log", &lines(0..1, "A"));
    let mut ws = ws_on(&d.0);
    for i in 1..80 {
        d.append("app.log", &lines(i..i + 1, "A"));
        ws.refresh_tail(std::slice::from_ref(&p)).unwrap();
    }
    assert_eq!(ws.summary().total, 80);
    assert!(ws.tail_status().files[0].lines_per_sec > 0.0);
}

#[test]
fn stopping_never_waits_for_the_batch_in_flight() {
    let d = Dir::new("stop-nonblocking");
    let factory = Arc::new(FakeFactory::default());
    // An import holding the workspace's write lock, as far as the batch can tell.
    let store = Arc::new(Mutex::new(()));
    let held = store.lock().unwrap();
    let (entered, in_batch) = std::sync::mpsc::channel();
    let batch_store = store.clone();
    let stop = imt_core::workspace::StopToken::default();
    let live = LiveTail::start_with(
        std::slice::from_ref(&d.0),
        WatchSettings {
            tick_ms: 60_000,
            ..WatchSettings::default()
        },
        factory.clone(),
        Box::new(move |_| {
            let _ = entered.send(());
            let _waits = batch_store.lock();
        }),
        Box::new(|_, _| {}),
        stop.clone(),
    );
    let sink = factory.sinks.lock().unwrap()[0].clone();
    sink(WatchSignal::Paths(vec![d.0.join("app.log")]));
    in_batch.recv_timeout(Duration::from_secs(5)).unwrap();
    let started = Instant::now();
    drop(live);
    assert!(started.elapsed() < Duration::from_secs(1), "stopping returned at once");
    assert!(stop.is_stopped());
    drop(held);
}

#[test]
fn a_session_dropped_from_its_own_batch_does_not_deadlock() {
    let d = Dir::new("stop-self");
    let factory = Arc::new(FakeFactory::default());
    let slot: Arc<Mutex<Option<LiveTail>>> = Arc::new(Mutex::new(None));
    let (done, finished) = std::sync::mpsc::channel();
    let batch_slot = slot.clone();
    let live = LiveTail::start(
        std::slice::from_ref(&d.0),
        WatchSettings {
            tick_ms: 60_000,
            ..WatchSettings::default()
        },
        factory.clone(),
        // The window state (which owns the session) going away on the worker thread.
        Box::new(move |_| {
            let taken = batch_slot.lock().unwrap().take();
            drop(taken);
            let _ = done.send(());
        }),
        Box::new(|_, _| {}),
    );
    *slot.lock().unwrap() = Some(live);
    let sink = factory.sinks.lock().unwrap()[0].clone();
    sink(WatchSignal::Paths(vec![d.0.join("app.log")]));
    finished.recv_timeout(Duration::from_secs(5)).expect("the batch finished");
    assert!(slot.lock().unwrap().is_none());
}

#[test]
fn a_whole_file_read_again_keeps_the_events_routes_would_claim() {
    // A file without routed events is tailed; when it is read again, events a route names stay in its source.
    let d = Dir::new("routed-reread");
    let p = d.write("app.json", r#"[{"@t":"2026-10-14T10:00:00Z","@m":"a"}]"#);
    let mut ws = Workspace::new(Profile::from_toml("[[route]]\nsource = '{Application}'\n").unwrap()).unwrap();
    ws.open(std::slice::from_ref(&d.0)).unwrap();
    assert_eq!(ws.sources().len(), 1);
    std::fs::write(
        &p,
        r#"[{"@t":"2026-10-14T10:00:00Z","@m":"a"},{"@t":"2026-10-14T10:00:01Z","@m":"b","Application":"x"}]"#,
    )
    .unwrap();
    let r = ws.refresh_tail(std::slice::from_ref(&p)).unwrap();
    assert_eq!(r.reimported.len(), 1);
    assert_eq!(messages(&ws), ["a", "b"]);
    assert_eq!(ws.sources().len(), 1);
}

#[test]
fn a_record_that_grows_past_the_resume_limit_stops_being_merged_into() {
    let d = Dir::new("resume-cap");
    let p = d.write("app.log", &line(0, "ERR", "A", "dump follows"));
    let mut ws = ws_on(&d.0);
    // The open record is held for continuation lines; 9 MiB of them arrive in steps.
    d.append("app.log", &line(1, "ERR", "A", "second"));
    ws.refresh_tail(std::slice::from_ref(&p)).unwrap();
    let mut dump = String::new();
    while dump.len() < 17 << 20 {
        dump += "    continuation of a very long dump\n";
    }
    d.append("app.log", &dump);
    ws.refresh_tail(std::slice::from_ref(&p)).unwrap();
    d.append("app.log", &line(2, "INF", "A", "after"));
    ws.refresh_tail(std::slice::from_ref(&p)).unwrap();
    let last = ws.records().last().unwrap().message.clone();
    assert_eq!(last, "after", "the new event is not swallowed by the endless record");
}

#[test]
fn a_rule_remembers_a_bounded_number_of_hits_and_forgets_those_outside_its_window() {
    let d = Dir::new("rule-hits");
    let profile = Profile::from_toml(
        r#"
[[live.rule]]
name = "payment errors"
when = 'message contains "Payment"'
threshold = 5
window = "1m"
severity = "warning"
"#,
    )
    .unwrap();
    let p = d.write("app.log", &line(0, "INF", "A", "start"));
    let mut ws = Workspace::new(profile).unwrap();
    ws.open(std::slice::from_ref(&d.0)).unwrap();
    ws.set_tail_options(TailOptions {
        detect_every_ms: 0,
        ..TailOptions::default()
    });
    let burst: String = (0..10_050).map(|_| line(5, "ERR", "A", "Payment failed")).collect();
    d.append("app.log", &burst);
    let r = ws.refresh_tail(std::slice::from_ref(&p)).unwrap();
    assert_eq!(r.appended, 10_050);
    assert!(r.alerts.iter().any(|a| a.kind == "rule"), "{:?}", r.alerts);
    // Much later the old hits fall out of the window: one new hit does not reach the threshold again.
    d.append("app.log", &line(3000, "ERR", "A", "Payment failed"));
    let r = ws.refresh_tail(std::slice::from_ref(&p)).unwrap();
    assert!(r.alerts.iter().all(|a| a.kind != "rule"), "{:?}", r.alerts);
}
