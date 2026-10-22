//! The streaming import when things go wrong: a store writer that fails (disk full) at each step, a cancelled
//! import, damaged archives and missing files. Each failure fails only its file, with a reason the user can
//! read, and a partly written source is aborted; the other files still import.

use imt_core::batch::{EventSchema, PendingBatch, SourceId};
use imt_core::importers::Importers;
use imt_core::ingest::{self, import_path, BatchSink, ImportEnv, NewSource, Outcome, SourceReport};
use imt_core::parser::ParserSet;
use imt_core::profile::CompiledProfile;
use imt_core::workspace::{ImportProgress, NOT_IMPORTED};
use imt_core::Profile;
use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

#[derive(Default, Clone, Copy, PartialEq)]
enum Fail {
    #[default]
    Never,
    Begin,
    Write,
    Finish,
    FinishAndAbort,
}

#[derive(Default)]
struct Sink {
    fail: Fail,
    next: u32,
    rows: usize,
    begun: Vec<String>,
    finished: Vec<SourceReport>,
    aborted: Vec<SourceId>,
}

impl Sink {
    fn failing(fail: Fail) -> Sink {
        Sink { fail, ..Sink::default() }
    }
}

impl BatchSink for Sink {
    fn begin_source(&mut self, source: &NewSource) -> anyhow::Result<SourceId> {
        if self.fail == Fail::Begin {
            anyhow::bail!("the store is read-only");
        }
        self.begun.push(source.name.clone());
        self.next += 1;
        Ok(SourceId(self.next - 1))
    }
    fn write(&mut self, batch: PendingBatch) -> anyhow::Result<()> {
        if self.fail == Fail::Write {
            anyhow::bail!("disk full");
        }
        self.rows += batch.len();
        Ok(())
    }
    fn finish_source(&mut self, report: &SourceReport) -> anyhow::Result<()> {
        if matches!(self.fail, Fail::Finish | Fail::FinishAndAbort) {
            anyhow::bail!("cannot commit: disk full");
        }
        self.finished.push(report.clone());
        Ok(())
    }
    fn abort_source(&mut self, id: SourceId) -> anyhow::Result<()> {
        if self.fail == Fail::FinishAndAbort {
            anyhow::bail!("cannot roll back");
        }
        self.aborted.push(id);
        Ok(())
    }
}

fn compiled() -> &'static CompiledProfile {
    static P: OnceLock<CompiledProfile> = OnceLock::new();
    P.get_or_init(|| CompiledProfile::compile(Profile::default_profile()).unwrap())
}

fn parsers() -> &'static ParserSet {
    static P: OnceLock<ParserSet> = OnceLock::new();
    P.get_or_init(|| ParserSet::of_compiled(CompiledProfile::compile(Profile::default_profile()).unwrap()))
}

fn importers() -> &'static Importers {
    static I: OnceLock<Importers> = OnceLock::new();
    I.get_or_init(Importers::default)
}

fn env(cancel: &AtomicBool) -> ImportEnv<'_> {
    ImportEnv {
        importers: importers(),
        parsers: parsers(),
        schema: Arc::new(EventSchema::plain()),
        cancel,
    }
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let p = std::env::temp_dir().join(format!("imt-ingest-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let p = self.0.join(name);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(&p, bytes).unwrap();
        p
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const LOG: &[u8] = b"2026-10-14 10:00:00 INFO one\n2026-10-14 10:00:01 WARN two\n2026-10-14 10:00:02 ERROR three\n";

fn reason(out: &[Outcome]) -> String {
    match out {
        [Outcome::Unrecognized(u)] => u.reason.clone(),
        other => panic!("expected one unrecognised file: {other:?}"),
    }
}

#[test]
fn a_healthy_sink_imports_the_file() {
    let d = TempDir::new("ok");
    let p = d.write("app.log", LOG);
    let cancel = AtomicBool::new(false);
    let mut sink = Sink::default();
    let out = import_path(&env(&cancel), &p, "app.log", &HashSet::new(), &mut sink);
    let [Outcome::Imported(r)] = out.as_slice() else {
        panic!("{out:?}")
    };
    assert_eq!((r.events, sink.rows), (3, 3));
    assert_eq!(sink.finished.len(), 1);
    assert!(sink.aborted.is_empty());
}

#[test]
fn a_store_that_cannot_start_a_source_fails_the_file_with_its_message() {
    let d = TempDir::new("begin");
    let p = d.write("app.log", LOG);
    let cancel = AtomicBool::new(false);
    let mut sink = Sink::failing(Fail::Begin);
    let r = reason(&import_path(&env(&cancel), &p, "app.log", &HashSet::new(), &mut sink));
    assert!(r.contains("read-only"), "{r}");
    assert!(sink.aborted.is_empty(), "nothing was begun, nothing to abort");
}

#[test]
fn a_disk_full_while_writing_aborts_the_partial_source() {
    let d = TempDir::new("write");
    let p = d.write("app.log", LOG);
    let cancel = AtomicBool::new(false);
    let mut sink = Sink::failing(Fail::Write);
    let out = import_path(&env(&cancel), &p, "app.log", &HashSet::new(), &mut sink);
    let r = reason(&out);
    assert!(r.contains("disk full"), "{r}");
    assert_eq!(sink.aborted, [SourceId(0)]);
    assert!(sink.finished.is_empty());
    let Outcome::Unrecognized(u) = &out[0] else { unreachable!() };
    assert!(!u.crashed, "an I/O failure is not a crash");
}

#[test]
fn a_failed_commit_aborts_and_a_failed_abort_still_reports_the_first_reason() {
    let d = TempDir::new("finish");
    let p = d.write("app.log", LOG);
    let cancel = AtomicBool::new(false);
    let mut sink = Sink::failing(Fail::Finish);
    assert!(reason(&import_path(&env(&cancel), &p, "app.log", &HashSet::new(), &mut sink)).contains("cannot commit"));
    assert_eq!(sink.aborted, [SourceId(0)]);

    let mut sink = Sink::failing(Fail::FinishAndAbort);
    let r = reason(&import_path(&env(&cancel), &p, "app.log", &HashSet::new(), &mut sink));
    assert!(r.contains("cannot commit") && !r.contains("roll back"), "{r}");
}

#[test]
fn a_cancelled_import_does_not_import_the_file_being_read() {
    let d = TempDir::new("cancel");
    let p = d.write("app.log", &LOG.repeat(20_000));
    let cancel = AtomicBool::new(true);
    let mut sink = Sink::default();
    let r = reason(&import_path(&env(&cancel), &p, "app.log", &HashSet::new(), &mut sink));
    assert_eq!(r, NOT_IMPORTED);
    assert_eq!(sink.aborted.len(), sink.begun.len(), "a begun source is rolled back");
}

#[test]
fn missing_empty_and_damaged_files_are_unrecognised_with_a_reason() {
    let d = TempDir::new("damaged");
    let cancel = AtomicBool::new(false);
    let mut sink = Sink::default();
    let missing = reason(&import_path(
        &env(&cancel),
        &d.0.join("gone.log"),
        "gone.log",
        &HashSet::new(),
        &mut sink,
    ));
    assert!(!missing.is_empty());

    // A gzip header with garbage after it, and a zip signature with nothing behind it.
    let gz = d.write("bad.log.gz", &[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3, 0xde, 0xad, 0xbe, 0xef]);
    let r = reason(&import_path(&env(&cancel), &gz, "bad.log.gz", &HashSet::new(), &mut sink));
    assert!(!r.is_empty());
    let zip = d.write("bad.zip", b"PK\x03\x04garbage that is not a zip archive");
    let out = import_path(&env(&cancel), &zip, "bad.zip", &HashSet::new(), &mut sink);
    assert!(out.iter().all(|o| matches!(o, Outcome::Unrecognized(_))), "{out:?}");

    let empty = d.write("empty.log", b"");
    let r = reason(&import_path(&env(&cancel), &empty, "empty.log", &HashSet::new(), &mut sink));
    assert!(!r.is_empty());
    assert!(sink.finished.is_empty());
}

#[test]
fn a_zip_member_that_was_removed_is_left_out() {
    let d = TempDir::new("zip");
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for name in ["a.log", "b.log"] {
        w.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        w.write_all(LOG).unwrap();
    }
    let bytes = w.finish().unwrap().into_inner();
    let p = d.write("logs.zip", &bytes);
    let cancel = AtomicBool::new(false);
    let mut sink = Sink::default();
    let removed: HashSet<PathBuf> = [PathBuf::from(format!(
        "{}{}b.log",
        p.to_string_lossy(),
        imt_core::archive::ENTRY_SEPARATOR
    ))]
    .into();
    let out = import_path(&env(&cancel), &p, "logs.zip", &removed, &mut sink);
    assert_eq!(out.len(), 1, "{out:?}");
    assert!(matches!(&out[0], Outcome::Imported(r) if r.name == "logs.zip/a.log"), "{out:?}");
}

#[test]
fn files_to_read_follows_the_globs_skips_removed_and_known_files_and_sorts() {
    let d = TempDir::new("files");
    let a = d.write("b/app.log", LOG);
    let b = d.write("a/app.log", LOG);
    let removed_file = d.write("a/removed.log", LOG);
    let known_file = d.write("known.log", LOG);
    let removed: HashSet<PathBuf> = [removed_file.clone()].into();
    let known: HashSet<String> = [known_file.to_string_lossy().into_owned()].into();
    let files = ingest::files_to_read(std::slice::from_ref(&d.0), std::slice::from_ref(&d.0), compiled(), &removed, &known);
    let paths: Vec<&Path> = files.iter().map(|(p, _)| p.as_path()).collect();
    assert_eq!(paths, [b.as_path(), a.as_path()], "sorted, without the removed and known files");
    assert!(files.iter().all(|(_, name)| name.ends_with("app.log")));
    // A file opened directly is read even though it was not found by a folder scan.
    let single = ingest::files_to_read(std::slice::from_ref(&a), &[], compiled(), &HashSet::new(), &HashSet::new());
    assert_eq!(single.len(), 1);
}

#[test]
fn import_files_reports_progress_and_lists_the_rest_after_a_cancel() {
    let d = TempDir::new("progress");
    let files: Vec<(PathBuf, String)> = (0..3).map(|i| (d.write(&format!("f{i}.log"), LOG), format!("f{i}.log"))).collect();
    let cancel = AtomicBool::new(false);
    let progress = ImportProgress::default();
    let mut sink = Sink::default();
    let out = ingest::import_files(&env(&cancel), &files, &HashSet::new(), &progress, &mut sink);
    assert_eq!(out.iter().filter(|o| matches!(o, Outcome::Imported(_))).count(), 3);
    let snap = progress.snapshot();
    assert!(!snap.running);
    assert_eq!((snap.done, snap.total), (3, 3));
    assert_eq!(snap.bytes_done, (LOG.len() * 3) as u64);

    // Cancelled while the first file is committed: it is kept, the rest are listed as not imported.
    let progress = Arc::new(ImportProgress::default());
    let mut sink = CancelAfterFirst {
        inner: Sink::default(),
        progress: progress.clone(),
    };
    let out = ingest::import_files(&env(&cancel), &files, &HashSet::new(), &progress, &mut sink);
    assert_eq!(out.len(), 3);
    assert!(matches!(&out[0], Outcome::Imported(_)), "{out:?}");
    assert!(
        out[1..]
            .iter()
            .all(|o| matches!(o, Outcome::Unrecognized(u) if u.reason == NOT_IMPORTED)),
        "{out:?}"
    );
    assert_eq!(sink.inner.begun.len(), 1, "the rest were not read");
    assert!(progress.snapshot().cancelled);
    assert!(!cancel.load(Ordering::SeqCst));
}

/// Cancels the import once the first source is committed.
struct CancelAfterFirst {
    inner: Sink,
    progress: Arc<ImportProgress>,
}

impl BatchSink for CancelAfterFirst {
    fn begin_source(&mut self, source: &NewSource) -> anyhow::Result<SourceId> {
        self.inner.begin_source(source)
    }
    fn write(&mut self, batch: PendingBatch) -> anyhow::Result<()> {
        self.inner.write(batch)
    }
    fn finish_source(&mut self, report: &SourceReport) -> anyhow::Result<()> {
        self.progress.cancel();
        self.inner.finish_source(report)
    }
    fn abort_source(&mut self, id: SourceId) -> anyhow::Result<()> {
        self.inner.abort_source(id)
    }
}
