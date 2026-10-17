//! Live tail: incremental reading of files that grow, rotate, get truncated
//! or disappear while they are open, and the derived state that follows.
//!
//! Per file the workspace keeps a [`TailState`]: the file's identity
//! ((device, inode) / (volume, file index)), the size seen, the offset up to
//! which bytes became records, fingerprints of the first bytes and of the
//! bytes before the offset, and where the file's last record starts. A
//! refresh then:
//!
//! - **appends**: reads from the start of the last record (multi-line
//!   formats) or the offset to the size observed when the refresh began,
//!   holds back an unterminated last line (the carry) until its newline
//!   arrives or it times out, parses the region with the importer's
//!   [`crate::importers::Importer::import_tail`] using the file's start for
//!   headers and format, runs extractors, the pipeline and markers exactly as
//!   a full import does, and replaces the re-read last record instead of
//!   duplicating it;
//! - **follows renames**: a tracked identity found under another name keeps
//!   its source (rotation) and a new file at the old name is a new source;
//! - **detects truncation and rewrites**: a smaller size or changed
//!   fingerprints detach the source (its records stay) and the file is read
//!   from 0; a copy of the old content (copy-truncate, or a `.gz` made by
//!   logrotate) is recognised by its fingerprints and adopted instead of
//!   imported again;
//! - **falls back** to re-importing a file whole for binary formats,
//!   archives, plugin importers and any inconsistency.
//!
//! Every per-file step runs under [`crate::guard::catch`].

mod alerts;
mod apply;
mod files;
mod identity;
mod index;
mod reader;
mod rolling;
mod types;
mod watch;

pub use alerts::{duration_ms, AlertRuleCfg, LiveCfg, LiveRules};
pub use identity::{identity_of, is_network_path, FileIdentity};
pub(super) use index::{Change, FieldIndex, MatchCache, PatternCache};
pub use rolling::{family, group_of};
pub use types::*;
pub use watch::{
    initial_mode, mode_of, BatchFn, Coalescer, FallbackFn, FileWatcher, LiveTail, WatchPreference, WatchSettings, WatchSignal, WatchSink,
    WatchedRoot, WatcherFactory,
};

use super::{display_name, import_file, source_file, Workspace};
use crate::importers::{ImportCtx, MAX_LINE_LEN};
use crate::model::Record;
use crate::pipeline::StepStats;
use crate::query::group_key;
use alerts::{AlertState, InstanceSeen};
use anyhow::Result;
use identity::{hash, Fingerprint, ANCHOR_BYTES, HEAD_BYTES};
use reader::{ByteSource, Encoding, FileBytes};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// Bytes of a file's start given to importers for headers and format detection.
const HEAD_TEXT_BYTES: u64 = 64 * 1024;
/// Most bytes parsed per step; a larger backlog is read in several steps.
const MAX_STEP_BYTES: u64 = 8 << 20;
/// Steps per file per refresh (a 256 MiB backlog at most per refresh).
const MAX_STEPS: usize = 32;
/// A last record re-read from its start may be at most this long; beyond it
/// (a "record" that never ends) only new lines are read.
const MAX_RESUME_BYTES: u64 = 8 << 20;
/// Window for lines per second.
const RATE_WINDOW: Duration = Duration::from_secs(10);
const RATE_SAMPLES: usize = 64;

/// Where the next read starts again: the file's last record (so continuation
/// lines merge into it) or an unterminated line parsed after a timeout.
#[derive(Debug, Clone)]
pub(super) struct Resume {
    start: u64,
    line: u32,
    /// Pipeline statistics of the records from `line` on, subtracted when they are re-read.
    stats: Vec<StepStats>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Mode {
    /// Text that grows by appending, read incrementally by this importer.
    Append { importer: &'static str },
    /// Read only as a whole (binary formats, archives, importers without tail support).
    Whole,
}

/// What an import learned about a file for tailing it.
#[derive(Debug, Clone)]
pub(super) struct TailSeed {
    identity: Option<FileIdentity>,
    head: Fingerprint,
    len: u64,
    mtime: Option<SystemTime>,
    offset: u64,
    anchor: u64,
    resume: Option<Resume>,
    next_line: u32,
    encoding: Encoding,
    bom: u64,
    mode: Mode,
}

/// What an importer reported about the end of a file it read.
pub(super) struct AppendInfo {
    pub importer: &'static str,
    pub multiline: bool,
    /// Line of the importer's last record (before the pipeline).
    pub last_line: Option<u32>,
}

impl TailSeed {
    /// A file read whole (`bytes` as read from disk).
    pub(super) fn whole(path: &Path, bytes: &[u8]) -> TailSeed {
        let (encoding, bom) = Encoding::detect(&bytes[..bytes.len().min(512)]);
        TailSeed {
            identity: identity_of(path),
            head: Fingerprint::of(bytes),
            len: bytes.len() as u64,
            mtime: mtime(path),
            offset: bytes.len() as u64,
            anchor: anchor_of(bytes, bytes.len() as u64),
            resume: None,
            next_line: 1,
            encoding,
            bom,
            mode: Mode::Whole,
        }
    }

    /// A text file that can be appended to. `stats_from` gives the pipeline
    /// statistics of the importer's records from a line on.
    pub(super) fn append(path: &Path, bytes: &[u8], info: &AppendInfo, stats_from: &dyn Fn(u32) -> Vec<StepStats>) -> TailSeed {
        let mut seed = TailSeed::whole(path, bytes);
        let body = &bytes[seed.bom as usize..];
        let enc = seed.encoding;
        let newlines = enc.line_ends(body).count() as u32;
        let complete = enc.complete_len(body);
        let resume_line = if info.multiline {
            info.last_line
        } else if complete < body.len() {
            // An unterminated last line is read again once it is complete.
            Some(newlines + 1)
        } else {
            None
        };
        seed.resume = resume_line.and_then(|line| {
            enc.line_start(body, line).map(|start| Resume {
                start: seed.bom + start as u64,
                line,
                stats: stats_from(line),
            })
        });
        seed.next_line = newlines + 1;
        seed.mode = Mode::Append { importer: info.importer };
        seed
    }
}

/// Lines per second over the last seconds.
#[derive(Debug, Clone, Default)]
struct Rate {
    samples: VecDeque<(Instant, u64)>,
}

impl Rate {
    fn add(&mut self, now: Instant, lines: u64) {
        self.samples.push_back((now, lines));
        if self.samples.len() > RATE_SAMPLES {
            self.samples.pop_front();
        }
    }

    fn per_sec(&self, now: Instant) -> f64 {
        let lines: u64 = self
            .samples
            .iter()
            .filter(|(t, _)| now.duration_since(*t) <= RATE_WINDOW)
            .map(|(_, n)| n)
            .sum();
        lines as f64 / RATE_WINDOW.as_secs_f64()
    }
}

/// Live state of one file (one source).
#[derive(Debug, Clone)]
pub(super) struct TailState {
    path: PathBuf,
    /// Still the file at `path` (not truncated or deleted away).
    attached: bool,
    seed: TailSeed,
    /// Size and time of the carry (unterminated bytes after `offset`) when first seen.
    carry_since: Option<(u64, Instant)>,
    state: TailFileState,
    group: String,
    /// Files whose content is a copy of this one (copy-truncate in progress).
    copies: Vec<PathBuf>,
    rate: Rate,
    last_event: Option<i64>,
}

/// Live tail bookkeeping of a workspace.
#[derive(Default)]
pub(super) struct TailBook {
    states: BTreeMap<u32, TailState>,
    pub(super) options: TailOptions,
    pub(super) evicted: usize,
    /// Unrecognised files with the size and time seen; retried only when they change.
    unrecognized_seen: HashMap<String, (u64, Option<SystemTime>)>,
    alerts: AlertState,
    /// Instance keys seen (with the grouping they were computed with).
    instances: Option<(Vec<String>, HashMap<String, InstanceSeen>)>,
    /// Truncated files to read again from their start as new sources.
    restart: Vec<PathBuf>,
}

impl TailBook {
    /// Forgets every file (on clear/reload); options stay.
    pub(super) fn reset(&mut self) {
        let options = self.options;
        *self = TailBook {
            options,
            ..TailBook::default()
        };
    }

    pub(super) fn forget(&mut self, source: u32) {
        self.states.remove(&source);
    }

    /// Starts tailing a source imported from `path`.
    pub(super) fn track(&mut self, source: u32, path: &Path, name: &str, seed: TailSeed) {
        let state = match seed.mode {
            Mode::Whole => TailFileState::Whole,
            Mode::Append { .. } => TailFileState::Tailing,
        };
        self.states.insert(
            source,
            TailState {
                path: path.to_path_buf(),
                attached: true,
                seed,
                carry_since: None,
                state,
                group: group_of(name),
                copies: Vec::new(),
                rate: Rate::default(),
                last_event: None,
            },
        );
    }

    pub(super) fn remember_unrecognized(&mut self, path: &str) {
        let p = Path::new(path);
        self.unrecognized_seen
            .insert(path.to_string(), (std::fs::metadata(p).map_or(0, |m| m.len()), mtime(p)));
    }
}

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn wall_ms() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

fn anchor_of(bytes: &[u8], offset: u64) -> u64 {
    let end = (offset as usize).min(bytes.len());
    hash(&bytes[end.saturating_sub(ANCHOR_BYTES as usize)..end])
}

fn add_stats(total: &mut Vec<StepStats>, add: &[StepStats], sign: i64) {
    if total.len() < add.len() {
        total.resize(add.len(), StepStats::default());
    }
    for (t, a) in total.iter_mut().zip(add) {
        let f = |x: usize, y: usize| if sign >= 0 { x + y } else { x.saturating_sub(y) };
        t.input = f(t.input, a.input);
        t.output = f(t.output, a.output);
    }
}

/// Record-set change being assembled by a refresh.
#[derive(Default)]
pub(super) struct Delta {
    /// (source, from line): remove that source's records from the line on (all with `None`).
    remove: Vec<(u32, Option<u32>)>,
    insert: Vec<Record>,
}

impl Delta {
    fn remove_from(&mut self, source: u32, line: Option<u32>) {
        self.insert.retain(|r| r.source != source || line.is_some_and(|l| r.line < l));
        self.remove.push((source, line));
    }

    fn is_empty(&self) -> bool {
        self.remove.is_empty() && self.insert.is_empty()
    }
}

/// What one step over a file found.
enum Step {
    Unchanged,
    Parsed {
        remove_from: Option<u32>,
        records: Vec<Record>,
        stats_add: Vec<StepStats>,
        stats_sub: Vec<StepStats>,
        skipped: usize,
        lines: u64,
        more: bool,
    },
    Truncated,
    Reimport(String),
}

/// The files of a folder with their identities.
fn list_dir(dir: &Path) -> Vec<(PathBuf, Option<FileIdentity>)> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| {
            let p = e.path();
            let id = identity_of(&p);
            (p, id)
        })
        .collect()
}

/// A file looked at by a refresh: canonical path when it exists, else the
/// canonical folder plus the file name.
fn normalize(p: &Path) -> PathBuf {
    if let Ok(c) = std::fs::canonicalize(p) {
        return c;
    }
    match (p.parent().and_then(|d| std::fs::canonicalize(d).ok()), p.file_name()) {
        (Some(dir), Some(name)) => dir.join(name),
        _ => p.to_path_buf(),
    }
}

impl Workspace {
    /// Live tail options (deleted files, retention, carry timeout).
    pub fn set_tail_options(&mut self, options: TailOptions) {
        self.tail.options = options;
        let mut report = TailReport::default();
        self.evict(&mut report);
    }

    pub fn tail_options(&self) -> TailOptions {
        self.tail.options
    }

    /// Reads what changed in `changed` paths (files or folders) and in the
    /// files next to them: appended lines, rotations, truncations, deletions
    /// and new files under the opened folders. An empty list only flushes
    /// partial lines that timed out and re-evaluates live alerts.
    pub fn refresh_tail(&mut self, changed: &[PathBuf]) -> Result<TailReport> {
        let started = Instant::now();
        let mut report = TailReport::default();
        let mut delta = Delta::default();
        self.ensure_instances();

        let mut cands: BTreeSet<PathBuf> = BTreeSet::new();
        let mut dirs: BTreeSet<PathBuf> = BTreeSet::new();
        // Folders named in `changed`: every tailed file below them is checked.
        let mut scopes: Vec<PathBuf> = Vec::new();
        for p in changed {
            let p = normalize(p);
            if p.is_dir() {
                for entry in std::fs::read_dir(&p).into_iter().flatten().flatten() {
                    cands.insert(entry.path());
                }
                scopes.push(p.clone());
                dirs.insert(p);
            } else {
                if let Some(d) = p.parent() {
                    dirs.insert(d.to_path_buf());
                }
                cands.insert(p);
            }
        }

        // 1. Files already tailed: appended, renamed, truncated or gone.
        let now = Instant::now();
        let ids: Vec<u32> = self
            .tail
            .states
            .iter()
            .filter(|(_, s)| {
                s.attached
                    && (cands.contains(&s.path)
                        || s.path.parent().is_some_and(|d| dirs.contains(d))
                        || scopes.iter().any(|d| s.path.starts_with(d))
                        || s.carry_since.is_some())
            })
            .map(|(id, _)| *id)
            .collect();
        let mut listings: HashMap<PathBuf, Vec<(PathBuf, Option<FileIdentity>)>> = HashMap::new();
        for id in ids {
            let Some(mut st) = self.tail.states.remove(&id) else { continue };
            self.check(id, &mut st, &dirs, &mut listings, &mut cands, &mut delta, &mut report, now);
            if self.tail.options.deleted_files == DeletedFiles::Drop && st.state == TailFileState::Deleted {
                self.drop_source(id, &mut delta);
            } else {
                self.tail.states.insert(id, st);
            }
        }

        // 2. Files not tailed yet: copies to adopt, archives changed, new files.
        cands.extend(std::mem::take(&mut self.tail.restart));
        let mut new_files = Vec::new();
        for p in cands {
            if let Some(path) = self.new_candidate(&p, &mut delta, &mut report) {
                new_files.push(path);
            }
        }
        if !new_files.is_empty() {
            let (records, added) = self.import_files(new_files);
            report.added = added;
            delta.insert.extend(records);
        }

        self.finish(delta, &mut report, started);
        Ok(report)
    }

    /// Applies the record changes and updates everything derived.
    fn finish(&mut self, delta: Delta, report: &mut TailReport, started: Instant) {
        let mut sources: BTreeSet<u32> = delta.remove.iter().map(|(s, _)| *s).collect();
        sources.extend(delta.insert.iter().map(|r| r.source));
        let change = if delta.is_empty() { None } else { Some(self.apply_delta(delta)) };
        if let Some(change) = &change {
            report.appended = change.inserted.len();
            report.updated = change.replaced.len();
            self.recount(&sources);
            if change.disturbs() {
                report.ids_changed_from = change.cut.map(|c| c as u32);
            }
        }
        if !report.rotated.is_empty() || !report.deleted.is_empty() {
            // Source names changed (or sources went away): name-based results are stale.
            self.invalidate();
        }
        report
            .sources_changed
            .extend(sources.iter().filter(|s| self.sources.iter().any(|x| x.id == **s)));
        self.evict(report);
        report.newest_ts = self.newest_ts();
        let inserted = change.map(|c| c.inserted).unwrap_or_default();
        self.track_instances(&inserted, report);
        self.live_alerts(&inserted, report);
        report.alerts_raised = report.alerts.len();
        report.elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        if !report.is_empty() {
            self.load_ms = report.elapsed_ms as u64;
        }
    }

    /// Looks at one tailed file.
    #[allow(clippy::too_many_arguments)]
    fn check(
        &mut self,
        id: u32,
        st: &mut TailState,
        dirs: &BTreeSet<PathBuf>,
        listings: &mut HashMap<PathBuf, Vec<(PathBuf, Option<FileIdentity>)>>,
        cands: &mut BTreeSet<PathBuf>,
        delta: &mut Delta,
        report: &mut TailReport,
        now: Instant,
    ) {
        let exists = st.path.is_file();
        let current = if exists { identity_of(&st.path) } else { None };
        let same = exists && (st.seed.identity.is_none() || current == st.seed.identity);
        if !same {
            // Renamed (rotation)? Look for the identity next to the old name and in the changed folders.
            let moved = st.seed.identity.and_then(|ident| {
                let mut search: Vec<PathBuf> = st.path.parent().map(Path::to_path_buf).into_iter().collect();
                search.extend(dirs.iter().cloned());
                search.into_iter().find_map(|d| {
                    let list = listings.entry(d.clone()).or_insert_with(|| list_dir(&d));
                    list.iter()
                        .find(|(p, i)| *i == Some(ident) && *p != st.path)
                        .map(|(p, _)| p.clone())
                })
            });
            match moved {
                Some(to) => {
                    let from = std::mem::replace(&mut st.path, to.clone());
                    self.rename_source(id, &to);
                    st.state = TailFileState::Rotated;
                    report.rotated.push(RotatedSource {
                        source_id: id,
                        from: from.to_string_lossy().into_owned(),
                        to: to.to_string_lossy().into_owned(),
                    });
                    // A new file under the old name is a new source.
                    cands.insert(from);
                    cands.remove(&to);
                }
                None => {
                    st.attached = false;
                    st.state = TailFileState::Deleted;
                    report.deleted.push(id);
                    if exists {
                        cands.insert(st.path.clone());
                    }
                    return;
                }
            }
        }
        self.follow(id, st, delta, report, now);
    }

    /// Reads what was appended to a file (in steps for a large backlog).
    fn follow(&mut self, id: u32, st: &mut TailState, delta: &mut Delta, report: &mut TailReport, now: Instant) {
        let Some(len) = std::fs::metadata(&st.path).ok().map(|m| m.len()) else {
            st.state = TailFileState::Error {
                message: "the file cannot be read".into(),
            };
            return;
        };
        let path = st.path.clone();
        self.follow_from(id, st, &FileBytes { path: &path, len }, delta, report, now);
    }

    /// [`Workspace::follow`] over any byte source (a file, or a decompressed rotated archive).
    fn follow_from(&mut self, id: u32, st: &mut TailState, src: &dyn ByteSource, delta: &mut Delta, report: &mut TailReport, now: Instant) {
        for _ in 0..MAX_STEPS {
            let step = self.guarded_step(id, st, src, now);
            match step {
                Step::Unchanged => return,
                Step::Parsed {
                    remove_from,
                    records,
                    stats_add,
                    stats_sub,
                    skipped,
                    lines,
                    more,
                } => {
                    if let Some(s) = self.sources.iter_mut().find(|s| s.id == id) {
                        add_stats(&mut s.step_stats, &stats_sub, -1);
                        add_stats(&mut s.step_stats, &stats_add, 1);
                        s.dropped = s
                            .step_stats
                            .first()
                            .map_or(0, |f| f.input)
                            .saturating_sub(s.step_stats.last().map_or(0, |l| l.output));
                        s.skipped += skipped;
                        s.error = None;
                    }
                    if !matches!(st.state, TailFileState::Rotated) {
                        st.state = TailFileState::Tailing;
                    }
                    st.rate.add(now, lines);
                    st.last_event = Some(wall_ms());
                    if remove_from.is_some() {
                        delta.remove_from(id, remove_from);
                    }
                    delta.insert.extend(records.into_iter().map(|mut r| {
                        r.source = id;
                        r
                    }));
                    if !more {
                        return;
                    }
                }
                Step::Truncated => {
                    self.truncated(id, st, delta, report, now);
                    return;
                }
                Step::Reimport(reason) => {
                    self.reimport(id, st, &reason, delta, report);
                    return;
                }
            }
        }
    }

    fn guarded_step(&self, id: u32, st: &mut TailState, src: &dyn ByteSource, now: Instant) -> Step {
        let name = self.source_name(id).to_string();
        let mut work = st.clone();
        match crate::guard::catch("IMT", || self.step(&mut work, src, now, &name)) {
            Ok(step) => {
                *st = work;
                step
            }
            Err(e) => Step::Reimport(format!("{e:#}")),
        }
    }

    /// One read of new bytes; updates `st` to describe what was consumed.
    fn step(&self, st: &mut TailState, src: &dyn ByteSource, now: Instant, source_name: &str) -> Result<Step> {
        let len = src.len();
        let seed = &st.seed;
        let importer = match &seed.mode {
            Mode::Whole => {
                let changed = len != seed.len || mtime(&st.path) != seed.mtime;
                return Ok(if changed {
                    Step::Reimport("the file changed".into())
                } else {
                    Step::Unchanged
                });
            }
            Mode::Append { importer } => *importer,
        };
        if len < seed.offset {
            return Ok(Step::Truncated);
        }
        let head = src.read(0, HEAD_TEXT_BYTES.max(HEAD_BYTES as u64))?;
        if !seed.head.matches(&head) {
            return Ok(Step::Truncated);
        }
        if len == seed.len && st.carry_since.is_none() {
            return Ok(Step::Unchanged);
        }
        let mut region_start = seed.resume.as_ref().map_or(seed.offset, |r| r.start);
        let mut resume = seed.resume.clone();
        if seed.offset - region_start > MAX_RESUME_BYTES {
            // A record that never ends: stop merging into it.
            region_start = seed.offset;
            resume = None;
        }
        let anchor_start = seed.offset.saturating_sub(ANCHOR_BYTES);
        let read_from = region_start.min(anchor_start);
        let read_to = len.min(seed.offset + MAX_STEP_BYTES);
        let bytes = src.read(read_from, read_to)?;
        if (bytes.len() as u64) < seed.offset - read_from
            || hash(&bytes[(anchor_start - read_from) as usize..(seed.offset - read_from) as usize]) != seed.anchor
        {
            return Ok(Step::Truncated);
        }
        let enc = seed.encoding;
        let data = &bytes[(region_start - read_from) as usize..];
        let min_end = (seed.offset - region_start) as usize;
        let mut end = enc.complete_len(data).max(min_end);
        let more = read_to < len;
        let carry = data.len() - end;
        if carry > 0 && !more {
            let carry_at = region_start + end as u64;
            let since = match st.carry_since {
                Some((at, t)) if at == carry_at => t,
                _ => now,
            };
            let flush = Duration::from_millis(self.tail.options.carry_flush_ms);
            if carry >= MAX_LINE_LEN || now.duration_since(since) >= flush {
                end = data.len();
                st.carry_since = None;
            } else {
                st.carry_since = Some((carry_at, since));
            }
        } else {
            st.carry_since = None;
        }
        if end == min_end {
            return Ok(Step::Unchanged);
        }

        let region = &data[..end];
        let text = enc.decode(region);
        let bom = seed.bom;
        let head_bytes = &head[(bom as usize).min(head.len())..];
        let head_bytes = &head_bytes[..enc.complete_len(head_bytes).max(head_bytes.len().min(1))];
        let head_text = enc.decode(head_bytes);
        let file_date = mtime(&st.path)
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .and_then(|d| crate::time::utc_date_of(d.as_millis() as i64));
        let ctx = ImportCtx {
            profile: &self.profile,
            path: &st.path,
            file_date,
        };
        let parsed = self
            .registry
            .importers
            .find(importer)
            .and_then(|imp| imp.import_tail(&ctx, &head_text, &text).map(|r| (imp, r)));
        let Some((imp, parsed)) = parsed else {
            return Ok(Step::Reimport(format!("the {importer} importer cannot append")));
        };
        let mut imported = match parsed {
            Ok(i) => i,
            Err(e) => return Ok(Step::Reimport(format!("{e:#}"))),
        };
        let base = resume.as_ref().map_or(seed.next_line, |r| r.line);
        for r in &mut imported.records {
            r.line = base + r.line.saturating_sub(1);
        }
        if resume.is_some() && imported.records.is_empty() {
            return Ok(Step::Reimport("the last event could not be read again".into()));
        }
        let newlines = enc.line_ends(region).count() as u32;
        let new_resume_line = if imp.multiline() {
            imported.records.last().map(|r| r.line)
        } else if enc.complete_len(region) < region.len() {
            Some(base + newlines)
        } else {
            None
        };
        let new_resume = new_resume_line.and_then(|line| {
            enc.line_start(region, line - base + 1).map(|s| Resume {
                start: region_start + s as u64,
                line,
                stats: self
                    .profile
                    .ingest(imported.records.iter().filter(|r| r.line >= line).cloned().collect(), source_name)
                    .1,
            })
        });
        let (records, stats_add) = self.profile.ingest(std::mem::take(&mut imported.records), source_name);
        let lines = enc.line_ends(&data[min_end..end]).count() as u64;

        let offset = region_start + end as u64;
        let seed = &mut st.seed;
        let remove_from = resume.as_ref().map(|r| r.line);
        let stats_sub = resume.map(|r| r.stats).unwrap_or_default();
        seed.anchor = hash(&bytes[(offset.saturating_sub(ANCHOR_BYTES) - read_from) as usize..(offset - read_from) as usize]);
        seed.offset = offset;
        seed.len = if more { offset } else { len };
        seed.mtime = mtime(&st.path);
        seed.resume = new_resume;
        seed.next_line = base + newlines;
        if seed.head.len < HEAD_BYTES {
            seed.head = Fingerprint::of(&head);
        }
        Ok(Step::Parsed {
            remove_from,
            records,
            stats_add,
            stats_sub,
            skipped: imported.skipped,
            lines,
            more: more && offset < len,
        })
    }
}
