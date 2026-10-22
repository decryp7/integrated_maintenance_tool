//! Live tail on the store: reading what was appended to followed files, listing how each file was last read, and
//! opening files at their end ([`debrief_core::ports::Tail`], [`debrief_core::ports::Ingest::append_tail`]).
//!
//! A followed text file is read from where the last read stopped ([`debrief_core::ingest::read_appended`]): the
//! appended events get new ids in a new segment of the same source, existing events keep theirs, and the source's
//! counts, time range and `tail_offsets` row move on. Where a file was last read is kept in `tail_offsets`; a file
//! that was imported in full has no row until it first grows (its position is derived from its last event then).
//! The positions are extended after the events they sort behind ([`crate::pos::append`]), so a refresh costs what
//! was appended, not what the store holds.
//!
//! What cannot be read from the middle is read again in full as a new source (the old one goes, its ids with it):
//! a file that shrank (truncated, rotated in place), archives and binary formats, importers that read whole files,
//! files routed to several sources, and files whose last line was not finished when they were opened. A file that
//! disappeared keeps its events.
//!
//! Plain text events may continue on the lines that follow, so the last event of a read is held back (nothing is
//! written for it, the next read starts at it) until another event follows or the caller asks for the held events (`append_tail` with no paths, after waiting for the
//! file to stop growing); [`TailAppend::held`] tells the caller to ask again.
//!
//! A file imported in full has no held event, so its first follow reads again from its last imported event: when
//! the file still ends it where the import saw it, that event is not written twice; when lines continued it, the
//! source is read again in full (new source, new ids) instead of attaching the lines as separate untimed events.

use crate::writer::{self, SegmentSink};
use crate::{compact, engine, lock, pos, read_lock, write_lock, DuckStore, Importing, Inner};
use debrief_core::api::UnrecognizedFile;
use debrief_core::batch::SourceId;
use debrief_core::ingest::{
    self, begin_at_end, read_appended_from, resume_after, ImportEnv, NewSource, Region, Resume, SharedRef, TailStart,
};
use debrief_core::ports::{ImportReport, ImportSession, NewEvents, StoreError, StoreResult, Tail, TailAppend, TailFile, TailRead};
use duckdb::{params, Connection};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

/// How long live tail waits for a running import before it gives up for this change.
pub(crate) const TAIL_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// Why a file imported in full cannot be followed from where the import stopped.
const UNFINISHED_LINE: &str = "its last line was not finished when it was opened";

/// A source of a followed file.
struct Followed {
    id: u32,
    path: String,
    name: String,
    /// Bytes read when it was imported.
    bytes: u64,
    events: u64,
    file_kind: String,
}

/// Where live tail reads a followed file next.
#[derive(Debug, Clone, Default)]
struct Tracked {
    byte: u64,
    /// The line the next read starts at, minus one (lines consumed).
    lines: u64,
    size: u64,
    whole: bool,
    held: bool,
    note: Option<String>,
    changed_ms: Option<i64>,
}

fn wall_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// A path as the store knows it: canonical, also when the file is gone (its folder is canonicalised then).
fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| match (p.parent().map(std::fs::canonicalize), p.file_name()) {
        (Some(Ok(dir)), Some(name)) => dir.join(name),
        _ => p.to_path_buf(),
    })
}

/// The file of a source path (an archive member's path names the archive first).
fn file_of(path: &str) -> &str {
    path.split(debrief_core::archive::ENTRY_SEPARATOR).next().unwrap_or(path)
}

fn followed(conn: &Connection) -> StoreResult<Vec<Followed>> {
    let mut stmt = conn
        .prepare("SELECT id, path, name, bytes, events, coalesce(file_kind, '') FROM sources WHERE ready ORDER BY id")
        .map_err(engine)?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Followed {
                id: r.get(0)?,
                path: r.get(1)?,
                name: r.get(2)?,
                bytes: r.get(3)?,
                events: r.get(4)?,
                file_kind: r.get(5)?,
            })
        })
        .map_err(engine)?;
    rows.collect::<duckdb::Result<Vec<_>>>().map_err(engine)
}

fn tracked(conn: &Connection) -> StoreResult<HashMap<u32, Tracked>> {
    let mut stmt = conn
        .prepare("SELECT source, byte, line, size, whole, held, note, changed_ms FROM tail_offsets")
        .map_err(engine)?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, u32>(0)?,
                Tracked {
                    byte: r.get(1)?,
                    lines: r.get(2)?,
                    size: r.get(3)?,
                    whole: r.get(4)?,
                    held: r.get(5)?,
                    note: r.get(6)?,
                    changed_ms: r.get(7)?,
                },
            ))
        })
        .map_err(engine)?;
    rows.collect::<duckdb::Result<HashMap<_, _>>>().map_err(engine)
}

fn save_tracked(conn: &Connection, source: u32, t: &Tracked) -> duckdb::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO tail_offsets (source, byte, line, size, whole, held, note, changed_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        params![source, t.byte, t.lines, t.size, t.whole, t.held, t.note, t.changed_ms],
    )
    .map(drop)
}

/// What one refresh does with one followed file.
enum Action {
    /// Read the appended bytes of this source, from where the last read stopped (`None`: derive that first).
    Append(Followed, Option<Tracked>),
    /// The file shrank: read it again from its start as a new source.
    Truncated(Vec<Followed>),
    /// It cannot be read from the middle: read it again in full.
    Whole(Vec<Followed>, String),
    /// The file is gone; its events stay.
    Deleted(Vec<Followed>),
}

/// An appended read to record.
struct Appended {
    id: u32,
    events: usize,
    skipped: usize,
    from: Option<i64>,
    to: Option<i64>,
    tracked: Tracked,
}

impl Inner {
    /// The last event's position of a source imported in full, and from it where a read of the appended bytes
    /// starts (with the last event's start, to read it again first: see [`Resume`]); `Err(reason)` when the file has to be read in full instead.
    fn derive_start(&self, conn: &Connection, f: &Followed, file: &Path) -> Result<(TailStart, Option<TailStart>), String> {
        if f.events == 0 {
            return Ok((TailStart { byte: 0, line: 1 }, None));
        }
        let last = conn.query_row(
            "SELECT byte_offset, line FROM events WHERE source = ? ORDER BY byte_offset DESC, line DESC LIMIT 1",
            params![f.id],
            |r| {
                Ok(TailStart {
                    byte: r.get(0)?,
                    line: r.get(1)?,
                })
            },
        );
        let last = last.map_err(|e| format!("its last event could not be found ({e})"))?;
        match resume_after(file, last, f.bytes) {
            Some(next) if next.byte >= f.bytes => Ok((next, Some(last))),
            Some(_) => Err(UNFINISHED_LINE.into()),
            None => Err("it could not be read".into()),
        }
    }

    /// Decides what to do with each followed file the refresh concerns.
    fn plan_tail(&self, conn: &Connection, hint: &[PathBuf]) -> StoreResult<Vec<Action>> {
        let sources = followed(conn)?;
        let tracked = tracked(conn)?;
        let mut by_file: Vec<(String, Vec<Followed>)> = Vec::new();
        for s in sources {
            let file = file_of(&s.path).to_string();
            match by_file.iter_mut().find(|(f, _)| *f == file) {
                Some((_, group)) => group.push(s),
                None => by_file.push((file, vec![s])),
            }
        }
        let mut actions = Vec::new();
        for (file, group) in by_file {
            let named = hint.iter().any(|h| Path::new(&file).starts_with(h));
            let held = group.iter().any(|s| tracked.get(&s.id).is_some_and(|t| t.held));
            if !named && !held {
                continue;
            }
            let meta = match std::fs::metadata(&file) {
                Ok(m) => m,
                Err(_) => {
                    actions.push(Action::Deleted(group));
                    continue;
                }
            };
            let size = meta.len();
            if group.len() > 1 {
                actions.push(Action::Whole(group, "its lines are routed to several sources".into()));
                continue;
            }
            let Some(only) = group.into_iter().next() else { continue };
            if only.path != file || only.file_kind != "Text" {
                actions.push(Action::Whole(vec![only], "it is an archive or a compressed file".into()));
                continue;
            }
            let t = tracked.get(&only.id).cloned();
            let seen = t.as_ref().map_or(only.bytes, |t| t.size);
            let at = t.as_ref().map_or(only.bytes, |t| t.byte);
            if size < seen || size < at {
                actions.push(Action::Truncated(vec![only]));
            } else if size == seen && !t.as_ref().is_some_and(|t| t.held) {
                continue;
            } else if let Some(t) = t.as_ref().filter(|t| t.whole) {
                let why = t.note.clone().unwrap_or_else(|| "it is read in full when it changes".into());
                actions.push(Action::Whole(vec![only], why));
            } else {
                actions.push(Action::Append(only, t));
            }
        }
        Ok(actions)
    }

    /// Removes sources from the catalog and the full-text index (their events leave the view at once; compaction
    /// rewrites their segments).
    pub(crate) fn forget_sources(&self, conn: &Connection, ids: &[u32]) -> StoreResult<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let list = ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", ");
        conn.execute_batch(&format!(
            "BEGIN TRANSACTION; DELETE FROM diagnostics WHERE source IN ({list}); DELETE FROM tail_offsets WHERE source IN ({list}); DELETE FROM sources WHERE id IN ({list}); COMMIT;"
        ))
        .map_err(engine)?;
        if let Some(f) = &self.fts {
            for id in ids {
                if let Err(e) = f.delete_source(*id) {
                    log::warn!("could not remove source {id} from the full-text index: {e:#}");
                }
            }
        }
        Ok(())
    }

    /// The first id a new segment would get, and the visible events now.
    fn id_mark(&self, conn: &Connection) -> StoreResult<(u64, u64)> {
        let first = conn
            .query_row("SELECT coalesce(max(end_id), 0) FROM segments", [], |r| r.get::<_, u64>(0))
            .map_err(engine)?
            .max(writer::meta_u64(conn, "next_id"));
        let before = conn
            .query_row("SELECT coalesce(sum(events), 0)::UBIGINT FROM sources WHERE ready", [], |r| {
                r.get::<_, u64>(0)
            })
            .map_err(engine)?;
        Ok((first, before))
    }

    /// Reads the appended bytes of the files `actions` name; returns what was appended and the files that turned out
    /// to need a full read.
    fn append_regions(
        &self,
        conn: &Connection,
        actions: Vec<Action>,
        flush_held: bool,
        out: &mut TailAppend,
    ) -> StoreResult<Vec<(Followed, String)>> {
        let appends: Vec<(Followed, Option<Tracked>)> = actions
            .into_iter()
            .filter_map(|a| match a {
                Action::Append(f, t) => Some((f, t)),
                _ => None,
            })
            .collect();
        if appends.is_empty() {
            return Ok(Vec::new());
        }
        let (parser_set, schema) = {
            let s = read_lock(&self.state);
            (s.parsers.clone(), self.schema.clone())
        };
        let cancel = AtomicBool::new(false);
        let encodings = self.encoding_policy(conn);
        let env = ImportEnv {
            importers: self.registry().importers(),
            parsers: &parser_set,
            schema,
            encodings: &encodings,
            cancel: &cancel,
        };
        let offsets = writer::clock_offsets(conn).map_err(|e| engine(format!("{e:#}")))?;
        let _writing = lock(&self.write_lock);
        let (first_new, before) = self.id_mark(conn)?;
        let sink = SegmentSink::new(
            conn.try_clone().map_err(engine)?,
            &self.dir,
            self.schema.clone(),
            1,
            self.generation.clone(),
            None,
        )
        .map_err(|e| engine(format!("{e:#}")))?;
        let mut done: Vec<Appended> = Vec::new();
        let mut whole: Vec<(Followed, String)> = Vec::new();
        for (f, t) in appends {
            let file = PathBuf::from(file_of(&f.path));
            let size = std::fs::metadata(&file).map_or(0, |m| m.len());
            let mut resume = None;
            let t = match t {
                Some(t) => t,
                None => match self.derive_start(conn, &f, &file) {
                    Ok((start, last)) => {
                        resume = last.map(|last| Resume { last, old_end: start.byte });
                        Tracked {
                            byte: start.byte,
                            lines: u64::from(start.line.saturating_sub(1)),
                            size: f.bytes,
                            ..Tracked::default()
                        }
                    }
                    Err(why) => {
                        whole.push((f, why));
                        continue;
                    }
                },
            };
            // The held event is written when the caller asks for the held events (it waited for the file to stop
            // growing, see `TailOptions::carry_flush_ms`) and the file has not grown since.
            let flush = t.held && size == t.size && flush_held;
            if t.held && size == t.size && !flush {
                out.held.push(f.id);
                continue;
            }
            let offset = offsets.get(&f.path).copied().unwrap_or(0);
            sink.shift_source(SourceId(f.id), offset);
            let src = NewSource {
                path: f.path.clone(),
                name: f.name.clone(),
            };
            // A file imported in full is read again from its last event, which may have been continued since.
            let start = match resume {
                Some(r) => r.last,
                None => TailStart {
                    byte: t.byte,
                    line: u32::try_from(t.lines + 1).unwrap_or(u32::MAX),
                },
            };
            let read = debrief_core::guard::catch("reading the appended lines", || {
                read_appended_from(&env, &file, &src, SourceId(f.id), start, flush, resume, &mut SharedRef(&sink))
            });
            match read {
                Ok(Region::Whole(why)) => whole.push((f, why)),
                Ok(Region::Read(r)) => {
                    let grew = r.batched.events > 0;
                    let tracked = Tracked {
                        byte: r.next.byte,
                        lines: u64::from(r.next.line.saturating_sub(1)),
                        size: r.size,
                        whole: false,
                        held: r.held,
                        note: t.note.clone(),
                        changed_ms: if grew { Some(wall_ms()) } else { t.changed_ms },
                    };
                    if r.held {
                        out.held.push(f.id);
                    }
                    done.push(Appended {
                        id: f.id,
                        events: r.batched.events,
                        skipped: r.skipped,
                        from: r.batched.from.map(|t| t.saturating_add(offset)),
                        to: r.batched.to.map(|t| t.saturating_add(offset)),
                        tracked,
                    });
                }
                // One unreadable file does not stop the others; the next change tries again.
                Err(e) => log::warn!("live tail could not read an appended file: {e:#}"),
            }
        }
        sink.finish().map_err(|e| engine(format!("{e:#}")))?;
        conn.execute_batch("BEGIN TRANSACTION").map_err(engine)?;
        let recorded = (|| -> duckdb::Result<()> {
            for a in &done {
                conn.execute(
                    "UPDATE sources SET events = events + ?, skipped = skipped + ?, bytes = ?,
                       ts_from = CASE WHEN ? IS NULL THEN ts_from WHEN ts_from IS NULL THEN ? ELSE least(ts_from, ?) END,
                       ts_to = CASE WHEN ? IS NULL THEN ts_to WHEN ts_to IS NULL THEN ? ELSE greatest(ts_to, ?) END
                     WHERE id = ?",
                    params![
                        a.events as u64,
                        a.skipped as u64,
                        a.tracked.size,
                        a.from,
                        a.from,
                        a.from,
                        a.to,
                        a.to,
                        a.to,
                        a.id
                    ],
                )?;
                save_tracked(conn, a.id, &a.tracked)?;
            }
            Ok(())
        })();
        match recorded {
            Ok(()) => conn.execute_batch("COMMIT").map_err(engine)?,
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(engine(e));
            }
        }
        let events: usize = done.iter().map(|a| a.events).sum();
        if events > 0 {
            out.appended += events;
            out.first_new = Some(NewEvents {
                id: first_new,
                ts: done.iter().filter(|a| a.events > 0).filter_map(|a| a.from).min(),
            });
            out.sources_changed.extend(done.iter().filter(|a| a.events > 0).map(|a| a.id));
            if let Err(e) = pos::append(conn, first_new, before) {
                log::warn!("time-order positions could not be extended: {e}");
            }
            match compact::pending(conn) {
                Ok(p) if !p.is_empty() => {
                    if let Err(e) = compact::compact(conn, &self.dir, &self.schema) {
                        log::warn!("segment compaction failed: {e:#}");
                    }
                }
                Ok(_) => {}
                Err(e) => log::warn!("segment compaction could not be planned: {e:#}"),
            }
            self.generation.fetch_add(1, Ordering::SeqCst);
            self.bump_version(conn)?;
            if let Some(f) = &self.fts {
                f.wake();
            }
        }
        Ok(whole)
    }
}

impl DuckStore {
    /// The live tail refresh (see the module docs).
    pub(crate) fn tail_refresh(&self, changed: &[PathBuf]) -> StoreResult<TailAppend> {
        let waited = Instant::now();
        while self.inner.is_importing() {
            if waited.elapsed() > TAIL_WAIT {
                return Err(StoreError::Busy(
                    "an import is running; live tail reads the changes afterwards".into(),
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        if self.inner.importing.swap(true, Ordering::SeqCst) {
            return Err(StoreError::Busy(
                "an import is running; live tail reads the changes afterwards".into(),
            ));
        }
        let _importing = Importing(self.inner.clone());
        let roots: Vec<PathBuf> = read_lock(&self.inner.state).roots.clone();
        let hint: Vec<PathBuf> = changed.iter().map(|p| canonical(p)).collect();
        let mut out = TailAppend::default();
        let conn = self.inner.conn()?;
        let actions = self.inner.plan_tail(&conn, &hint)?;
        let mut reread: Vec<(Followed, bool, String)> = Vec::new();
        let mut gone: Vec<u32> = Vec::new();
        let mut appends = Vec::new();
        for a in actions {
            match a {
                Action::Deleted(group) => gone.extend(group.iter().map(|f| f.id)),
                Action::Truncated(group) => reread.extend(group.into_iter().map(|f| (f, true, "it shrank".to_string()))),
                Action::Whole(group, why) => reread.extend(group.into_iter().map(|f| (f, false, why.clone()))),
                a @ Action::Append(..) => appends.push(a),
            }
        }
        out.deleted = gone;
        let mark = self.inner.id_mark(&conn)?.0;
        for (f, why) in self.inner.append_regions(&conn, appends, hint.is_empty(), &mut out)? {
            reread.push((f, false, why));
        }
        // Files to read in full: the ones above, and files that appeared under an opened folder.
        let known: HashSet<String> = followed(&conn)?.iter().map(|f| file_of(&f.path).to_string()).collect();
        let mut paths: Vec<PathBuf> = reread.iter().map(|(f, _, _)| PathBuf::from(file_of(&f.path))).collect();
        // New files are only looked for under the opened roots (a watcher may name anything).
        paths.extend(
            hint.iter()
                .filter(|h| !known.contains(h.to_string_lossy().as_ref()))
                .filter(|h| roots.iter().any(|r| h.starts_with(r) || r.starts_with(h)))
                .cloned(),
        );
        paths.sort();
        paths.dedup();
        if paths.is_empty() {
            return Ok(out);
        }
        // Nothing to read after all (a folder named by the watcher with no new log in it).
        if reread.is_empty() {
            let (profile, removed) = {
                let s = read_lock(&self.inner.state);
                (s.profile.clone(), s.removed.clone())
            };
            if ingest::files_to_read(&paths, &roots, &profile, &removed, &known).is_empty() {
                return Ok(out);
            }
        }
        let old: Vec<u32> = reread.iter().map(|(f, _, _)| f.id).collect();
        self.inner.forget_sources(&conn, &old)?;
        let notes: HashMap<String, (bool, String)> = reread
            .iter()
            .map(|(f, truncated, why)| (file_of(&f.path).to_string(), (*truncated, why.clone())))
            .collect();
        let report = self.inner.import(&paths, &self.inner.progress)?;
        out.appended += report.events;
        if report.events > 0 && out.first_new.is_none() {
            out.first_new = Some(NewEvents { id: mark, ts: None });
        }
        for s in &report.sources {
            match notes.get(&s.path) {
                Some((true, _)) => out.truncated.push(s.id),
                Some((false, why)) => {
                    out.reimported.push(s.id);
                    // Remembered, so the next change reads it in full without trying the middle first.
                    let t = Tracked {
                        byte: 0,
                        lines: 0,
                        size: std::fs::metadata(&s.path).map_or(0, |m| m.len()),
                        whole: true,
                        held: false,
                        note: Some(why.clone()),
                        changed_ms: Some(wall_ms()),
                    };
                    if let Err(e) = save_tracked(&conn, s.id, &t) {
                        log::warn!("the tail state of a source could not be saved: {e}");
                    }
                }
                None => out.added.push(s.id),
            }
            out.sources_changed.push(s.id);
        }
        Ok(out)
    }

    /// Opens files and folders at their end (see [`Tail::open_at_end`]).
    fn at_end(&self, paths: &[PathBuf]) -> StoreResult<ImportSession> {
        if self.inner.importing.swap(true, Ordering::SeqCst) {
            return Err(StoreError::Busy("an import is running; wait for it or cancel it".into()));
        }
        let guard = Importing(self.inner.clone());
        let canonical = paths
            .iter()
            .map(|p| std::fs::canonicalize(p).map_err(|e| StoreError::Io(format!("{}: {e}", p.display()))))
            .collect::<StoreResult<Vec<_>>>()?;
        {
            let mut s = write_lock(&self.inner.state);
            s.unrecognized.retain(|u| u.reason != debrief_core::workspace::NOT_IMPORTED);
            for p in &canonical {
                if !s.roots.contains(p) {
                    s.roots.push(p.clone());
                }
                s.removed.remove(p);
            }
        }
        let inner = self.inner.clone();
        ImportSession::spawn(self.inner.progress.clone(), move |progress| {
            let _guard = guard;
            inner.import_at_end(&canonical, progress)
        })
    }
}

impl Inner {
    /// Begins the files under `paths` at their end: sources with no events, followed from there. What cannot be
    /// followed from the middle is imported in full (and read in full when it changes).
    fn import_at_end(&self, paths: &[PathBuf], progress: &debrief_core::workspace::ImportProgress) -> StoreResult<ImportReport> {
        let started = Instant::now();
        let (profile, parser_set, roots, removed) = {
            let s = read_lock(&self.state);
            (s.profile.clone(), s.parsers.clone(), s.roots.clone(), s.removed.clone())
        };
        let conn = self.conn()?;
        let mut known: HashSet<String> = followed(&conn)?.iter().map(|f| file_of(&f.path).to_string()).collect();
        known.extend(self.unrecognized().into_iter().map(|u| u.path));
        let files = ingest::files_to_read(paths, &roots, &profile, &removed, &known);
        let encodings = self.encoding_policy(&conn);
        let env = ImportEnv {
            importers: self.registry().importers(),
            parsers: &parser_set,
            schema: self.schema.clone(),
            encodings: &encodings,
            cancel: progress.cancel_flag(),
        };
        let mut report = ImportReport::default();
        let mut rest: Vec<PathBuf> = Vec::new();
        let mut why: HashMap<String, String> = HashMap::new();
        let mut unrecognized: Vec<UnrecognizedFile> = Vec::new();
        {
            let _writing = lock(&self.write_lock);
            let sink = SegmentSink::new(
                conn.try_clone().map_err(engine)?,
                &self.dir,
                self.schema.clone(),
                1,
                self.generation.clone(),
                None,
            )
            .map_err(|e| engine(format!("{e:#}")))?;
            let mut tails: Vec<(u32, TailStart, u64)> = Vec::new();
            for (path, name) in &files {
                if progress.cancel_flag().load(Ordering::SeqCst) {
                    unrecognized.push(UnrecognizedFile {
                        path: path.to_string_lossy().into_owned(),
                        name: name.clone(),
                        reason: debrief_core::workspace::NOT_IMPORTED.into(),
                        crashed: false,
                    });
                    continue;
                }
                let src = NewSource {
                    path: path.to_string_lossy().into_owned(),
                    name: name.clone(),
                };
                match debrief_core::guard::catch("opening a file at its end", || {
                    begin_at_end(&env, path, &src, &mut SharedRef(&sink))
                }) {
                    Ok(Ok(at)) => {
                        tails.push((at.report.id.0, at.next, at.report.bytes));
                        report.sources.push(crate::read::report_source(&at.report));
                    }
                    Ok(Err(reason)) => {
                        why.insert(src.path.clone(), reason);
                        rest.push(path.clone());
                    }
                    Err(e) => unrecognized.push(UnrecognizedFile {
                        path: src.path,
                        name: name.clone(),
                        reason: format!("{e:#}"),
                        crashed: e.downcast_ref::<debrief_core::guard::Crashed>().is_some(),
                    }),
                }
            }
            sink.finish().map_err(|e| engine(format!("{e:#}")))?;
            for (id, next, size) in tails {
                let t = Tracked {
                    byte: next.byte,
                    lines: u64::from(next.line.saturating_sub(1)),
                    size,
                    note: Some("opened at its end".into()),
                    ..Tracked::default()
                };
                save_tracked(&conn, id, &t).map_err(engine)?;
            }
            crate::parsers::record(&conn, &parser_set).map_err(engine)?;
            self.maintain(&conn);
            self.bump_version(&conn)?;
        }
        if !rest.is_empty() {
            let full = self.import(&rest, progress)?;
            for s in &full.sources {
                let t = Tracked {
                    size: std::fs::metadata(&s.path).map_or(0, |m| m.len()),
                    whole: true,
                    note: why.get(&s.path).map(|w| format!("read in full: {w}")),
                    changed_ms: Some(wall_ms()),
                    ..Tracked::default()
                };
                if let Err(e) = save_tracked(&conn, s.id, &t) {
                    log::warn!("the tail state of a source could not be saved: {e}");
                }
            }
            report.events += full.events;
            report.bytes += full.bytes;
            report.sources.extend(full.sources);
            unrecognized.extend(full.unrecognized);
        }
        {
            let mut s = write_lock(&self.state);
            s.unrecognized.extend(unrecognized.iter().cloned());
            s.unrecognized.sort_by(|a, b| a.name.cmp(&b.name));
        }
        report.unrecognized = unrecognized;
        report.elapsed_ms = started.elapsed().as_millis() as u64;
        Ok(report)
    }
}

impl Tail for DuckStore {
    fn tail_files(&self) -> StoreResult<Vec<TailFile>> {
        let conn = self.inner.conn()?;
        let sources = followed(&conn)?;
        let tracked = tracked(&conn)?;
        Ok(sources
            .into_iter()
            .map(|f| {
                let t = tracked.get(&f.id);
                let archived = f.path != file_of(&f.path) || f.file_kind != "Text";
                let whole = t.is_some_and(|t| t.whole) || archived;
                let note = match t {
                    Some(t) if t.note.is_some() => t.note.clone(),
                    _ if archived => Some("read in full when it changes: it is an archive or a compressed file".into()),
                    _ => None,
                };
                TailFile {
                    source: f.id,
                    path: f.path,
                    name: f.name,
                    offset: t.map_or(f.bytes, |t| t.byte),
                    size: t.map_or(f.bytes, |t| t.size),
                    lines: t.map_or(f.events, |t| t.lines),
                    read: if whole { TailRead::Whole } else { TailRead::Append },
                    note,
                    changed_ms: t.and_then(|t| t.changed_ms),
                }
            })
            .collect())
    }

    fn open_at_end(&self, paths: &[PathBuf]) -> StoreResult<ImportSession> {
        self.at_end(paths)
    }
}
