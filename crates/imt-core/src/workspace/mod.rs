//! The set of opened files and everything the UI asks about them.

mod annotate;
mod bundle;
mod cache_key;
mod chart;
mod context;
mod dashboard;
mod export;
mod facet;
mod insights;
mod page;
mod ports;
pub use ports::{event_view, summary_reply, WorkspaceStore};
mod preview;
mod redaction;
pub use insights::preview_parse;
pub use preview::{preview_profile, ExtractPreviewer};
pub use redaction::{RedactionPreviewRequest, RedactionSample, RedactionSummary, RedactionSummaryRequest};
mod report;
mod skew;
mod spans;
mod tail;
mod timeline;

pub use crate::api::*;
pub use tail::{
    duration_ms, family, group_of, identity_of, initial_mode, is_network_path, kind_name, mode_of, AlertMutes, AlertRuleCfg, AlertTarget,
    BatchFn, Clock, Coalescer, DeletedFiles, FallbackFn, FileIdentity, FileWatcher, LiveAlert, LiveCfg, LiveRules, LiveTail, OpenOptions,
    RotatedSource, StopToken, TailFileChange, TailFileState, TailFileStatus, TailOptions, TailReport, TailStatus, TailTotals,
    TimeWindow, WatchMode, WatchPreference, WatchSettings, WatchSignal, WatchSink, WatchedRoot, WatcherFactory, MAX_DETECT_EVERY_MS,
    MAX_MUTES, MAX_MUTE_MS,
};

use crate::chart::ChartDef;
use crate::importers::{decode, ImportCtx, Importers};
use crate::model::{Level, Record};
use crate::plugin::PluginRegistry;
use crate::profile::{CompiledProfile, Profile};
use crate::query::{CompiledQuery, Query};
use crate::time::utc_date_of;
use anyhow::{anyhow, Result};
use rayon::prelude::*;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Progress of the current import, readable (and cancellable) without the
/// workspace lock the import holds.
#[derive(Debug, Default)]
pub struct ImportProgress {
    cancel: std::sync::atomic::AtomicBool,
    running: std::sync::atomic::AtomicBool,
    done: std::sync::atomic::AtomicUsize,
    total: std::sync::atomic::AtomicUsize,
    bytes_done: std::sync::atomic::AtomicU64,
    bytes_total: std::sync::atomic::AtomicU64,
    /// The file being read now, and when the import started (behind a lock: rarely contended).
    detail: Mutex<ProgressDetail>,
}

#[derive(Debug, Default)]
struct ProgressDetail {
    current: Option<String>,
    started: Option<Instant>,
    finished_ms: Option<u64>,
}

/// Reason of the files a cancelled import did not read (listed as unrecognised, re-imported on the next open).
pub const NOT_IMPORTED: &str = "Not imported: the import was cancelled";

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressSnapshot {
    pub running: bool,
    pub done: usize,
    pub total: usize,
    pub cancelled: bool,
    pub bytes_done: u64,
    pub bytes_total: u64,
    /// Path of the file being read now (`None` between files and after the import).
    pub current: Option<String>,
    pub elapsed_ms: u64,
    /// Estimated time left from the bytes read so far; `None` until it can be estimated.
    pub eta_ms: Option<u64>,
}

impl ProgressSnapshot {
    /// Nothing running (also what a progress that cannot be asked for reads as).
    pub fn stopped() -> ProgressSnapshot {
        ProgressSnapshot {
            running: false,
            done: 0,
            total: 0,
            cancelled: false,
            bytes_done: 0,
            bytes_total: 0,
            current: None,
            elapsed_ms: 0,
            eta_ms: None,
        }
    }
}

/// Time left at the observed rate; `None` before anything was read or after the end.
fn eta(elapsed_ms: u64, done: u64, total: u64) -> Option<u64> {
    if done == 0 || elapsed_ms == 0 || done >= total {
        return None;
    }
    Some(((total - done) as u128 * elapsed_ms as u128 / done as u128).min(u64::MAX as u128) as u64)
}

impl ImportProgress {
    /// Asks the running import to stop after the files in flight.
    pub fn cancel(&self) {
        self.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn snapshot(&self) -> ProgressSnapshot {
        use std::sync::atomic::Ordering::SeqCst;
        let running = self.running.load(SeqCst);
        let (current, elapsed_ms) = {
            let d = self.detail();
            let elapsed = match (d.finished_ms, d.started) {
                (Some(ms), _) => ms,
                (None, Some(t)) => t.elapsed().as_millis() as u64,
                (None, None) => 0,
            };
            (d.current.clone(), elapsed)
        };
        let (bytes_done, bytes_total) = (self.bytes_done.load(SeqCst), self.bytes_total.load(SeqCst));
        ProgressSnapshot {
            running,
            done: self.done.load(SeqCst),
            total: self.total.load(SeqCst),
            cancelled: self.cancel.load(SeqCst),
            bytes_done,
            bytes_total,
            current: if running { current } else { None },
            elapsed_ms,
            eta_ms: if running { eta(elapsed_ms, bytes_done, bytes_total) } else { None },
        }
    }

    fn detail(&self) -> std::sync::MutexGuard<'_, ProgressDetail> {
        self.detail.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub(crate) fn start(&self, total: usize, bytes: u64) {
        use std::sync::atomic::Ordering::SeqCst;
        self.cancel.store(false, SeqCst);
        self.done.store(0, SeqCst);
        self.total.store(total, SeqCst);
        self.bytes_done.store(0, SeqCst);
        self.bytes_total.store(bytes, SeqCst);
        *self.detail() = ProgressDetail {
            current: None,
            started: Some(Instant::now()),
            finished_ms: None,
        };
        self.running.store(true, SeqCst);
    }

    pub(crate) fn reading(&self, path: &Path) {
        self.detail().current = Some(path.to_string_lossy().into_owned());
    }

    pub(crate) fn file_done(&self, bytes: u64) {
        use std::sync::atomic::Ordering::SeqCst;
        self.bytes_done.fetch_add(bytes, SeqCst);
        self.done.fetch_add(1, SeqCst);
    }

    pub(crate) fn finish(&self) {
        let mut d = self.detail();
        d.current = None;
        d.finished_ms = Some(d.started.map_or(0, |t| t.elapsed().as_millis() as u64));
        drop(d);
        self.running.store(false, std::sync::atomic::Ordering::SeqCst);
    }

    /// The flag [`ImportProgress::cancel`] sets, for code that checks it in a loop.
    pub fn cancel_flag(&self) -> &std::sync::atomic::AtomicBool {
        &self.cancel
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancel.load(std::sync::atomic::Ordering::SeqCst)
    }
}

pub struct Workspace {
    /// The profile with the generic parser: everything that does not depend on a file's format.
    profile: Arc<CompiledProfile>,
    /// The parser library compiled with the profile (ADR 0026): each file is read with its own parser.
    parsers: Arc<crate::parser::ParserSet>,
    /// The parser each source was read with.
    parser_refs: std::collections::HashMap<u32, crate::ports::ParserRef>,
    registry: PluginRegistry,
    roots: Vec<PathBuf>,
    /// Files the user removed; folder scans skip them until opened explicitly.
    removed: HashSet<PathBuf>,
    sources: Vec<SourceInfo>,
    /// Source names indexed by source id (ids are never reused).
    names: Vec<String>,
    unrecognized: Vec<UnrecognizedFile>,
    records: Vec<Record>,
    fields: Vec<FieldInfo>,
    default_group_by: Vec<String>,
    /// Where the identity fields of the default key apply (ADR 0020).
    key_scopes: Arc<crate::instance::KeyScopes>,
    marker_kinds: Vec<String>,
    load_ms: u64,
    /// Matching ids of recent queries, updated incrementally on append.
    cache: Mutex<tail::MatchCache>,
    /// The anomalies of the last query, keyed by generation, record count and the resolved query.
    anomaly_cache: Mutex<Option<(cache_key::CacheKey, Vec<crate::analysis::Anomaly>)>>,
    /// The pattern miner of the last patterns query, fed only new records.
    pattern_cache: Mutex<Option<tail::PatternCache>>,
    progress: Arc<ImportProgress>,
    /// Field statistics, maintained record by record.
    field_index: tail::FieldIndex,
    /// Bumped whenever existing record ids or contents change (not on pure appends).
    generation: u64,
    /// Bumped whenever the profile is replaced (caches never compare profile pointers).
    profile_revision: u64,
    /// Live tail: per-file read positions, options, alert state.
    tail: tail::TailBook,
    /// Clock-skew offsets by source path, applied to timestamps on import.
    offsets: crate::skew::ClockOffsets,
    /// The "Redacted view": pages, facets and source context pass through it.
    view_redaction: Option<Arc<crate::redact::Redactor>>,
    /// Runs between lifecycle markers, computed on first use and fed live-tail appends.
    spans: Mutex<spans::SpanBook>,
}

impl Workspace {
    /// An empty workspace with the built-in importers and exporters.
    pub fn new(profile: Profile) -> Result<Workspace> {
        Self::with_registry(profile, PluginRegistry::new())
    }

    /// An empty workspace that reads files with the given importers (and the
    /// built-in exporters).
    pub fn with_importers(profile: Profile, importers: Importers) -> Result<Workspace> {
        let mut registry = PluginRegistry::new();
        registry.importers = importers;
        Self::with_registry(profile, registry)
    }

    /// An empty workspace extended by the plugins installed in `registry`, reading with the built-in parsers.
    pub fn with_registry(profile: Profile, registry: PluginRegistry) -> Result<Workspace> {
        Self::with_parsers(profile, registry, &crate::parser::ParserSetup::builtins())
    }

    /// An empty workspace extended by the plugins in `registry`, reading with the parser library `setup`.
    pub fn with_parsers(profile: Profile, registry: PluginRegistry, setup: &crate::parser::ParserSetup) -> Result<Workspace> {
        registry.resolve();
        let parsers = Arc::new(registry.compile_parsers(&profile, setup)?);
        let profile = parsers.base().clone();
        let marker_kinds = profile.markers.iter().map(|m| m.kind.clone()).collect();
        Ok(Workspace {
            profile,
            parsers,
            parser_refs: Default::default(),
            registry,
            roots: Vec::new(),
            removed: HashSet::new(),
            sources: Vec::new(),
            names: Vec::new(),
            unrecognized: Vec::new(),
            records: Vec::new(),
            fields: Vec::new(),
            default_group_by: vec!["@source".into()],
            key_scopes: Arc::default(),
            marker_kinds,
            load_ms: 0,
            cache: Mutex::new(tail::MatchCache::default()),
            anomaly_cache: Mutex::new(None),
            pattern_cache: Mutex::new(None),
            progress: Arc::new(ImportProgress::default()),
            field_index: tail::FieldIndex::default(),
            generation: 0,
            profile_revision: 0,
            tail: tail::TailBook::default(),
            offsets: crate::skew::ClockOffsets::default(),
            view_redaction: None,
            spans: Mutex::new(spans::SpanBook::default()),
        })
    }

    pub fn profile(&self) -> &Profile {
        &self.profile.profile
    }

    /// The opened sources, by name.
    pub fn sources(&self) -> &[SourceInfo] {
        &self.sources
    }

    pub fn records(&self) -> &[Record] {
        &self.records
    }

    /// Opens files and folders (recursively), skipping files already loaded.
    pub fn open(&mut self, paths: &[PathBuf]) -> Result<()> {
        self.open_with(paths, &OpenOptions::default())
    }

    /// [`Workspace::open`] with options; `start_at_end` tails text files from
    /// their current end, skipping their history.
    pub fn open_with(&mut self, paths: &[PathBuf], options: &OpenOptions) -> Result<()> {
        let start = Instant::now();
        // Files a cancelled import did not read are not "known": this open reads them.
        self.unrecognized.retain(|u| u.reason != NOT_IMPORTED);
        let mut files = Vec::new();
        for p in paths {
            let p = std::fs::canonicalize(p).map_err(|e| anyhow!("{}: {e}", p.display()))?;
            if !self.roots.contains(&p) {
                self.roots.push(p.clone());
            }
            self.removed.remove(&p);
            self.collect_files(&p, &mut files);
        }
        // Archive entries are recorded as "<archive>!/<entry>"; the archive itself counts as known.
        let known: HashSet<&str> = self
            .sources
            .iter()
            .map(|s| s.path.as_str())
            .chain(self.unrecognized.iter().map(|u| u.path.as_str()))
            .map(|p| p.split(crate::archive::ENTRY_SEPARATOR).next().unwrap_or(p))
            .collect();
        files.retain(|f| !known.contains(f.to_string_lossy().as_ref()));
        files.sort();
        files.dedup();
        if options.start_at_end {
            files = self.skip_history(files);
        }
        let (records, added) = self.import_files(files);
        let count = records.len();
        self.records.extend(records);
        self.rebuild();
        self.run_annotators(None);
        self.load_ms = start.elapsed().as_millis() as u64;
        if count > 0 {
            self.registry
                .events()
                .publish(crate::events::RecordsAppended { count, sources: added });
        }
        if !self.registry.hooks.is_empty() {
            let summary = self.summary();
            for h in &self.registry.hooks {
                h.on_loaded(&summary);
            }
        }
        Ok(())
    }

    pub fn reload(&mut self) -> Result<()> {
        let roots = std::mem::take(&mut self.roots);
        let removed = std::mem::take(&mut self.removed);
        self.reset();
        self.removed = removed;
        self.open(&roots.into_iter().filter(|r| r.exists()).collect::<Vec<_>>())
    }

    /// Closes everything and notifies hooks.
    pub fn clear(&mut self) {
        self.reset();
        self.offsets = crate::skew::ClockOffsets::default();
        self.registry.clear_annotations();
        for h in &self.registry.hooks {
            h.on_cleared();
        }
        self.registry.events().publish(crate::events::WorkspaceCleared);
    }

    fn reset(&mut self) {
        self.tail.reset();
        self.roots.clear();
        self.removed.clear();
        self.sources.clear();
        self.parser_refs.clear();
        self.unrecognized.clear();
        self.records.clear();
        self.rebuild();
    }

    /// Removes a source. The logical sources a `[[route]]` split from one file share its path and go with it
    /// (the file is no longer read).
    pub fn remove_source(&mut self, id: u32) {
        let mut ids = vec![id];
        if let Some(src) = self.sources.iter().find(|s| s.id == id) {
            let path = PathBuf::from(&src.path);
            ids = self.sources.iter().filter(|s| s.path == src.path).map(|s| s.id).collect();
            self.roots.retain(|r| *r != path);
            self.removed.insert(path);
        }
        self.sources.retain(|s| !ids.contains(&s.id));
        self.records.retain(|r| !ids.contains(&r.source));
        for id in ids {
            self.tail.forget(id);
        }
        self.rebuild();
        self.run_annotators(None);
    }

    /// Replaces the profile and re-imports everything.
    pub fn set_profile(&mut self, profile: Profile) -> Result<()> {
        self.swap_profile(profile)?;
        for h in &self.registry.hooks {
            h.on_profile_changed(&self.profile.profile);
        }
        self.reload()
    }

    /// Compiles the parser library with `profile` and makes both current (nothing is read again).
    fn swap_profile(&mut self, profile: Profile) -> Result<()> {
        let parsers = Arc::new(self.registry.compile_parsers(&profile, self.parsers.setup())?);
        let compiled = parsers.base().clone();
        self.registry.events().publish(crate::events::ProfileChanged {
            name: compiled.profile.name.clone(),
        });
        self.marker_kinds = compiled.markers.iter().map(|m| m.kind.clone()).collect();
        self.profile = compiled;
        self.parsers = parsers;
        self.profile_revision += 1;
        self.refresh_view_redaction()
    }

    /// The parser library the workspace reads with.
    pub fn parsers(&self) -> &crate::parser::ParserSet {
        &self.parsers
    }

    /// Replaces the parser library (and the per-source choices); returns the ids of the sources read with a
    /// parser that is no longer the one the library would use (re-read them with [`Workspace::reread`]).
    pub fn set_parsers(&mut self, setup: &crate::parser::ParserSetup) -> Result<Vec<u32>> {
        self.parsers = Arc::new(self.registry.compile_parsers(&self.profile.profile, setup)?);
        Ok(self.stale_sources())
    }

    /// The sources whose recorded parser is not the one the library would read them with.
    pub fn stale_sources(&self) -> Vec<u32> {
        self.parsers.stale(
            self.sources
                .iter()
                .filter_map(|s| Some((s.id, s.path.as_str(), self.parser_refs.get(&s.id)?))),
        )
    }

    /// The parser that read a source.
    pub fn parser_of(&self, id: u32) -> Option<&crate::ports::ParserRef> {
        self.parser_refs.get(&id)
    }

    /// Reads these sources' files again (with the parser the library chooses now, or the override); the
    /// files keep their place in the workspace.
    pub fn reread(&mut self, ids: &[u32]) -> Result<()> {
        let paths: Vec<PathBuf> = self
            .sources
            .iter()
            .filter(|s| ids.contains(&s.id))
            .map(|s| PathBuf::from(s.path.split(crate::archive::ENTRY_SEPARATOR).next().unwrap_or(&s.path)))
            .collect();
        let gone: Vec<u32> = self
            .sources
            .iter()
            .filter(|s| {
                paths
                    .iter()
                    .any(|p| Path::new(s.path.split(crate::archive::ENTRY_SEPARATOR).next().unwrap_or(&s.path)) == p)
            })
            .map(|s| s.id)
            .collect();
        self.sources.retain(|s| !gone.contains(&s.id));
        self.records.retain(|r| !gone.contains(&r.source));
        for id in gone {
            self.tail.forget(id);
            self.parser_refs.remove(&id);
        }
        let (records, _) = self.import_files(paths);
        self.records.extend(records);
        self.rebuild();
        self.run_annotators(None);
        Ok(())
    }

    /// Reads what changed on disk (see [`Workspace::refresh_tail`]): appended
    /// lines, rotated, truncated, deleted and new files under the opened
    /// folders. Paths outside the workspace and removed files are ignored.
    /// Returns how many sources changed.
    pub fn refresh_files(&mut self, changed: &[PathBuf]) -> Result<usize> {
        self.refresh_tail(changed).map(|r| r.changed_sources())
    }

    /// Whether a file found under the opened roots should be read: not
    /// removed, under a root, and (inside a folder) matching the profile's
    /// include/exclude globs.
    fn wanted(&self, p: &Path) -> bool {
        if !p.is_file() || self.removed.contains(p) {
            return false;
        }
        if !self.roots.iter().any(|r| r == p || (r.is_dir() && p.starts_with(r))) {
            return false;
        }
        match self.roots.iter().find(|r| r.is_dir() && p.starts_with(r)) {
            Some(root) => {
                let rel = p.strip_prefix(root).unwrap_or(p);
                !self.profile.exclude.is_match(rel) && self.profile.include.is_match(rel)
            }
            None => true,
        }
    }

    /// The opened paths and removed files.
    pub fn state(&self) -> WorkspaceState {
        let s = |p: &PathBuf| p.to_string_lossy().into_owned();
        let mut removed: Vec<String> = self.removed.iter().map(s).collect();
        removed.sort();
        WorkspaceState {
            roots: self.roots.iter().map(s).collect(),
            removed,
            offsets: self.offsets.clone(),
        }
    }

    /// Replaces everything with `state` under `profile`. Paths that no longer
    /// exist are skipped and returned.
    pub fn restore(&mut self, profile: Profile, state: &WorkspaceState) -> Result<Vec<String>> {
        self.swap_profile(profile)?;
        self.reset();
        self.removed = state.removed.iter().map(PathBuf::from).collect();
        self.offsets = state.offsets.clone().sanitized();
        let (present, missing): (Vec<&String>, Vec<&String>) = state
            .roots
            .iter()
            .partition(|p| Path::new(p.split(crate::archive::ENTRY_SEPARATOR).next().unwrap_or(p)).exists());
        self.open(&present.into_iter().map(PathBuf::from).collect::<Vec<_>>())?;
        Ok(missing.into_iter().cloned().collect())
    }

    /// Checks that a profile compiles against the installed plugins.
    pub fn validate_profile(&self, profile: Profile) -> Result<()> {
        self.registry.compile_profile(profile).map(|_| ())
    }

    /// Lifts a plugin's quarantine and forgets its crashes.
    pub fn reset_plugin(&self, id: &str) {
        self.registry.reset_plugin(id);
    }

    /// Plugins installed in this workspace.
    pub fn plugins(&self) -> Vec<crate::plugin::PluginInfo> {
        self.registry.plugins()
    }

    /// The plugin registry (services, extension points, status).
    pub fn registry(&self) -> &PluginRegistry {
        &self.registry
    }

    /// The event bus the workspace publishes to.
    pub fn events(&self) -> Arc<crate::events::EventBus> {
        self.registry.events()
    }

    fn collect_files(&self, path: &Path, out: &mut Vec<PathBuf>) {
        crate::ingest::collect_files(path, &self.profile, &self.removed, out);
    }

    /// Progress and cancellation of the current import.
    pub fn progress(&self) -> Arc<ImportProgress> {
        self.progress.clone()
    }

    /// Imports files as new sources; returns their records (not yet in the
    /// workspace) and the new source ids.
    fn import_files(&mut self, files: Vec<PathBuf>) -> (Vec<Record>, Vec<u32>) {
        let progress = self.progress.clone();
        let parsers = self.parsers.clone();
        let importers = &self.registry.importers;
        let named: Vec<(PathBuf, String, u64)> = files
            .into_iter()
            .map(|p| {
                let name = display_name(&p, &self.roots);
                let size = std::fs::metadata(&p).map_or(0, |m| m.len());
                (p, name, size)
            })
            .collect();
        progress.start(named.len(), named.iter().map(|f| f.2).sum());
        let removed = &self.removed;
        let results: Vec<(String, String, ImportResult)> = named
            .into_par_iter()
            .flat_map_iter(|(path, name, size)| {
                // Cancelled: the remaining files are listed as not imported (the next open reads them).
                let out = if progress.is_cancelled() {
                    vec![(path.to_string_lossy().into_owned(), name, Err(NOT_IMPORTED.to_string()))]
                } else {
                    progress.reading(&path);
                    import_file(importers, &parsers, &path, &name, removed)
                };
                progress.file_done(size);
                out
            })
            .collect();
        progress.finish();

        let mut records = Vec::new();
        let mut added = Vec::new();
        for (path_s, name, result) in results {
            match result {
                Ok(loaded) => {
                    let ids = self.add_loaded(path_s, name, loaded, &mut records);
                    added.extend(ids);
                }
                Err(reason) => {
                    if reason != NOT_IMPORTED {
                        self.tail.remember_unrecognized(source_file(&path_s));
                    }
                    self.unrecognized.push(UnrecognizedFile {
                        crashed: reason.contains(" crashed: "),
                        path: path_s,
                        name,
                        reason,
                    })
                }
            }
        }
        self.sources.sort_by(|a, b| a.name.cmp(&b.name));
        self.unrecognized.sort_by(|a, b| a.name.cmp(&b.name));
        (records, added)
    }

    /// Registers the sources of one loaded file (its own and the logical sources its routes split off), with
    /// their records appended to `records`; returns their ids. File-wide counts (skipped lines, pipeline
    /// statistics, problems, the timestamp issue) go to the first of them.
    fn add_loaded(&mut self, path_s: String, name: String, mut loaded: Loaded, records: &mut Vec<Record>) -> Vec<u32> {
        let own = std::mem::take(&mut loaded.imported.records);
        let mut parts: Vec<(Option<String>, Vec<Record>)> = Vec::new();
        if !own.is_empty() || loaded.logical.is_empty() {
            parts.push((None, own));
        }
        parts.extend(loaded.logical.drain(..).map(|(label, recs)| (Some(label), recs)));
        let mut problems = Some(std::mem::take(&mut loaded.imported.problems).into_vec());
        let mut stats = Some(std::mem::take(&mut loaded.stats));
        let mut skipped = loaded.imported.skipped;
        let mut ids = Vec::new();
        for (label, mut recs) in parts {
            let offset = self.apply_offset(&path_s, &mut recs);
            let source_name = label.clone().unwrap_or_else(|| name.clone());
            let id = self.add_source(
                path_s.clone(),
                source_name,
                loaded.importer.clone(),
                std::mem::take(&mut skipped),
                stats.take().unwrap_or_default(),
            );
            let (from, to) = time_range(&recs);
            if let Some(s) = self.sources.last_mut() {
                s.records = recs.len();
                s.split_events = crate::importers::text::split_events(&recs);
                (s.from, s.to) = (from, to);
                s.offset_ms = offset;
                s.file_kind = loaded.file_kind;
                s.timestamp_issue = loaded.timestamp_issue.take();
                s.problems = problems.take().unwrap_or_default();
                s.identity = loaded.identity.clone();
                s.parser = Some(loaded.parser.id.clone());
                if let Some(label) = &label {
                    s.group = format!("{} › {label}", group_of(&name));
                    s.routed_from = Some(name.clone());
                }
            }
            if let Some(seed) = loaded.seed.take() {
                let path = PathBuf::from(&path_s);
                let name = self.source_name(id).to_string();
                self.tail.track(id, &path, &name, seed);
            }
            self.parser_refs.insert(id, loaded.parser.clone());
            records.extend(recs.into_iter().map(|mut r| {
                r.source = id;
                r
            }));
            ids.push(id);
        }
        ids
    }

    /// The profile compiled with the parser that read a source (the base profile when unknown).
    pub(crate) fn profile_of_source(&self, id: u32) -> Arc<CompiledProfile> {
        self.parser_refs
            .get(&id)
            .and_then(|r| self.parsers.profile_of(&r.id))
            .cloned()
            .unwrap_or_else(|| self.profile.clone())
    }

    /// Registers a source (without records) and returns its id.
    fn add_source(
        &mut self,
        path: String,
        name: String,
        importer: String,
        skipped: usize,
        step_stats: Vec<crate::pipeline::StepStats>,
    ) -> u32 {
        let dropped = step_stats.first().map_or(0, |s| s.input) - step_stats.last().map_or(0, |s| s.output);
        let id = self.names.len() as u32;
        self.names.push(name.clone());
        self.sources.push(SourceInfo {
            id,
            path,
            group: group_of(&name),
            name,
            importer,
            records: 0,
            skipped,
            split_events: 0,
            dropped,
            step_stats,
            error: None,
            from: None,
            to: None,
            offset_ms: 0,
            file_kind: FileKind::Text,
            timestamp_issue: None,
            routed_from: None,
            problems: Vec::new(),
            identity: Vec::new(),
            parser: None,
        });
        id
    }

    /// Re-sorts, re-numbers and re-derives everything after the record set changed.
    fn rebuild(&mut self) {
        self.records.par_sort_by_key(|r| (r.ts.unwrap_or(i64::MAX), r.source, r.line));
        self.records.par_iter_mut().enumerate().for_each(|(i, r)| r.id = i as u32);
        self.compute_fields();
        (self.default_group_by, self.key_scopes) = self.pick_group_by();
        self.generation += 1;
        self.cache.lock().unwrap_or_else(|p| p.into_inner()).clear();
        *self.anomaly_cache.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }

    fn compute_fields(&mut self) {
        self.field_index = tail::FieldIndex::build(&self.records);
        self.refresh_field_infos();
    }

    /// The summary's field list from the field index.
    fn refresh_field_infos(&mut self) {
        let derived: HashSet<&str> = self
            .profile
            .extractors
            .iter()
            .flat_map(|e| e.fields.iter().map(|(n, _)| n.as_str()))
            .chain(self.profile.pipeline.calc_fields())
            .collect();
        self.fields = self.field_index.infos(&derived);
    }

    /// The instance key, its candidates checked per source, and where its identity fields apply (ADR 0020).
    fn pick_group_by(&self) -> (Vec<String>, Arc<crate::instance::KeyScopes>) {
        let mut identity: Vec<String> = Vec::new();
        for s in &self.sources {
            for f in &s.identity {
                if !identity.contains(f) {
                    identity.push(f.clone());
                }
            }
        }
        let cfg = &self.profile.profile.instance;
        let scopes = crate::instance::KeyScopes::new(cfg, self.sources.iter().map(|s| (s.name.as_str(), s.identity.as_slice())));
        let names: Vec<String> = self.sources.iter().map(|s| s.name.clone()).collect();
        let presence = crate::instance::ScopedPresence {
            inner: self,
            scopes: &scopes,
            names: &names,
        };
        let events: Vec<usize> = self.sources.iter().map(|s| s.records).collect();
        let key = crate::instance::instance_key(cfg, &identity, &self.fields, &events, &presence);
        (key, Arc::new(scopes))
    }

    /// The instance (timeline lane) of `r` under the default key.
    fn default_key_of(&self, r: &Record) -> String {
        crate::query::group_key_in(&self.default_group_by, &self.key_scopes, r, self.source_name(r.source))
    }

    fn source_name(&self, id: u32) -> &str {
        self.names.get(id as usize).map(String::as_str).unwrap_or("")
    }

    fn compile(&self, q: &Query) -> Result<CompiledQuery> {
        CompiledQuery::new_scoped(
            &self.resolve_window(q),
            &self.default_group_by,
            &self.key_scopes,
            &self.marker_kinds,
        )
    }

    fn marker_names(&self, mask: crate::model::MarkerMask) -> Vec<&str> {
        let mask = mask & self.visible_markers();
        (0..self.marker_kinds.len())
            .filter(|i| mask & (1 << i) != 0)
            .map(|i| self.marker_kinds[i].as_str())
            .collect()
    }

    /// Ids (ascending) of every record matching the query, cached for paging
    /// and kept up to date on append (only new records are evaluated).
    pub fn matching(&self, q: &Query) -> Result<Arc<Vec<u32>>> {
        self.matching_with(q, &crate::ql::QueryControl::new())
    }

    /// Like [`matching`](Self::matching), checking `control` between batches of records (stage
    /// `Matching` of its progress). A cancelled run fails with [`crate::ql::Cancelled`] and is not cached.
    pub fn matching_with(&self, q: &Query, control: &crate::ql::QueryControl) -> Result<Arc<Vec<u32>>> {
        let key = cache_key::CacheKey::of(self.generation, q)?;
        let resolved = self.resolve_window(q).into_owned();
        if let Some(ids) = self
            .cache
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(key, &resolved, &self.records)
        {
            return Ok(ids);
        }
        let cq = CompiledQuery::new_scoped(&resolved, &self.default_group_by, &self.key_scopes, &self.marker_kinds)?;
        control.begin(crate::ql::QueryStage::Matching, self.records.len());
        let ids: Vec<u32> = self
            .records
            .par_chunks(crate::ql::BATCH)
            .flat_map_iter(|chunk| {
                let skip = control.is_cancelled();
                control.advance(chunk.len());
                let cq = &cq;
                chunk
                    .iter()
                    .filter(move |r| !skip && cq.matches(r, self.source_name(r.source)))
                    .map(|r| r.id)
            })
            .collect();
        control.check()?;
        let ids = Arc::new(ids);
        self.cache
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .put(key, resolved, q.window.is_some(), ids.clone());
        Ok(ids)
    }

    pub fn summary(&self) -> WorkspaceSummary {
        let mut levels: BTreeMap<Level, usize> = Level::ALL.iter().map(|l| (*l, 0)).collect();
        let (mut from, mut to) = (None::<i64>, None::<i64>);
        for r in &self.records {
            *levels.get_mut(&r.level).unwrap() += 1;
            if let Some(ts) = r.ts {
                from = Some(from.map_or(ts, |f| f.min(ts)));
                to = Some(to.map_or(ts, |t| t.max(ts)));
            }
        }
        WorkspaceSummary {
            sources: self.sources.clone(),
            unrecognized: self.unrecognized.clone(),
            total: self.records.len(),
            untimed: self.records.iter().filter(|r| r.ts.is_none()).count(),
            from,
            to,
            levels,
            fields: self.fields.clone(),
            default_group_by: self.default_group_by.clone(),
            marker_kinds: self
                .profile
                .markers
                .iter()
                .enumerate()
                .filter(|(i, _)| self.visible_markers() & (1 << i) != 0)
                .map(|(_, m)| MarkerKind {
                    kind: m.kind.clone(),
                    owner: m.owner.clone(),
                    role: m.role,
                    label: m.label.clone(),
                    lifecycle: m.lifecycle,
                    color: m.color.clone(),
                    icon: m.icon.clone(),
                    badge: m.badge.clone(),
                    shape: m.shape.clone(),
                    description: m.description.clone(),
                    when: self.registry.marker_rule(&m.owner, &m.kind),
                })
                .collect(),
            charts: self.chart_defs(),
            profile_name: self.profile.profile.name.clone(),
            load_ms: self.load_ms,
            dropped: self.sources.iter().map(|s| s.dropped).sum(),
            pipeline: self.pipeline_stats(),
            icons: self.profile.icons.clone(),
            plugins: self.registry.plugins(),
            exporters: self.registry.exporter_infos(),
            dashboards: self.profile.profile.dashboard.clone(),
            redacted_view: self.view_redaction.is_some(),
        }
    }

    fn pipeline_stats(&self) -> Vec<PipelineStepInfo> {
        self.profile
            .profile
            .pipeline
            .iter()
            .enumerate()
            .map(|(index, cfg)| {
                let (input, output, errors) = self
                    .sources
                    .iter()
                    .filter_map(|s| s.step_stats.get(index))
                    .fold((0, 0, 0), |(i, o, e), s| (i + s.input, o + s.output, e + s.errors));
                PipelineStepInfo {
                    index,
                    step: cfg.step.clone(),
                    input,
                    output,
                    errors,
                }
            })
            .collect()
    }

    fn chart_defs(&self) -> Vec<ChartDef> {
        crate::chart::chart_defs(&self.profile.profile)
    }
}

/// A file read by an importer: its records after the pipeline (those no route claims), the logical sources
/// its routes split off, the pipeline's statistics, and how to tail it.
struct Loaded {
    importer: String,
    file_kind: FileKind,
    imported: crate::importers::Imported,
    logical: Vec<(String, Vec<Record>)>,
    /// The importer's and the parser's identity fields (instance-key candidates).
    identity: Vec<String>,
    /// The parser that read it.
    parser: crate::ports::ParserRef,
    stats: Vec<crate::pipeline::StepStats>,
    seed: Option<tail::TailSeed>,
    timestamp_issue: Option<crate::api::TimestampIssue>,
}

/// Field counts per source, by position in the source list.
impl crate::instance::FieldPresence for Workspace {
    fn count_in(&self, field: &str, source: usize) -> usize {
        self.sources.get(source).map_or(0, |s| self.field_index.count_in(field, s.id))
    }
}

/// The file read, or why it could not be read.
type ImportResult = std::result::Result<Loaded, String>;

fn time_range(records: &[Record]) -> (Option<i64>, Option<i64>) {
    let (mut from, mut to) = (None::<i64>, None::<i64>);
    for r in records {
        if let Some(ts) = r.ts {
            from = Some(from.map_or(ts, |f| f.min(ts)));
            to = Some(to.map_or(ts, |t| t.max(ts)));
        }
    }
    (from, to)
}

/// Reads one file and runs the profile's ingest pipeline on its records.
/// Reads one file from disk, expands it if it is compressed, and imports
/// every resulting file. Returns (source path, display name, result) per file.
fn import_file(
    importers: &Importers,
    parsers: &crate::parser::ParserSet,
    path: &Path,
    name: &str,
    removed: &HashSet<PathBuf>,
) -> Vec<(String, String, ImportResult)> {
    let path_s = path.to_string_lossy().into_owned();
    let bytes = match read_with_retry(path) {
        Ok(b) => b,
        Err(e) => return vec![(path_s, name.to_string(), Err(e))],
    };
    let file_name = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    let compressed = crate::archive::is_zip(&bytes);
    let packed = compressed || crate::archive::is_gzip(&bytes);
    // Archives are tailed as a whole (by their compressed bytes).
    let whole = packed.then(|| tail::TailSeed::whole(path, &bytes));
    let entries = match crate::archive::expand(&file_name, bytes) {
        Ok(e) => e,
        Err(e) => return vec![(path_s, name.to_string(), Err(format!("{e:#}")))],
    };
    let file_date = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| utc_date_of(d.as_millis() as i64));
    entries
        .into_iter()
        .filter_map(|entry| {
            // Zip members are separate sources; a gzip file stays one source.
            let (source_path, source_name) = match (&entry.name, compressed) {
                (Some(inner), true) => (
                    format!("{path_s}{}{inner}", crate::archive::ENTRY_SEPARATOR),
                    format!("{name}/{inner}"),
                ),
                _ => (path_s.clone(), name.to_string()),
            };
            if removed.contains(Path::new(&source_path)) {
                return None;
            }
            // A crash anywhere while reading one file (even in built-in code) fails only that file.
            let plain = entry.name.is_none();
            let result = crate::guard::catch("IMT", || {
                let file = FileRef {
                    source: &source_path,
                    path,
                    name: &source_name,
                    file_date,
                };
                Ok(import_bytes(importers, parsers, &file, &entry.bytes, plain))
            })
            .unwrap_or_else(|e| Err(format!("{e:#} (please report this file)")))
            .map(|mut loaded| {
                if compressed {
                    loaded.file_kind = FileKind::ArchiveMember;
                } else if packed {
                    loaded.file_kind = FileKind::Compressed;
                }
                // A file split by routes is not tailed (a change re-reads it, so events find their logical sources).
                if !compressed && loaded.seed.is_none() && loaded.logical.is_empty() {
                    loaded.seed = whole.clone().or_else(|| Some(tail::TailSeed::whole(path, &entry.bytes)));
                }
                loaded
            });
            Some((source_path, source_name, result))
        })
        .collect()
}

/// Imports one file's bytes. `plain` (not an archive member) also prepares
/// tailing it incrementally when its importer can append.
/// The file `import_bytes` reads: its source path, the file on disk, its display name and date.
struct FileRef<'a> {
    source: &'a str,
    path: &'a Path,
    name: &'a str,
    file_date: Option<chrono::NaiveDate>,
}

fn import_bytes(importers: &Importers, parsers: &crate::parser::ParserSet, file: &FileRef, bytes: &[u8], plain: bool) -> ImportResult {
    let (path, source_name, file_date) = (file.path, file.name, file.file_date);
    if bytes.is_empty() {
        return Err("empty file".into());
    }
    let head = &bytes[..bytes.len().min(crate::ingest::HEAD_BYTES)];
    let score_text = crate::ingest::score_text(head, head.len() == bytes.len());
    let candidate = crate::parser::set::Candidate {
        source: file.source,
        path,
        rel: source_name,
        file_date,
        bytes: &head[..head.len().min(crate::parser::set::SCORE_CHARS)],
        text: score_text.as_deref(),
    };
    let choice = match parsers.choose(&candidate, importers) {
        Ok(c) => c,
        Err(_) if score_text.is_none() => return Err("binary file".into()),
        Err(reason) => return Err(reason),
    };
    let profile: &CompiledProfile = choice.profile;
    let ctx = ImportCtx { profile, path, file_date };
    let mut append = None;
    let mut file_kind = FileKind::Text;
    let mut text = None;
    let identity;
    let (id, mut imported) = match importers.select_binary(&ctx, &bytes[..bytes.len().min(64 * 1024)]) {
        Some(importer) => {
            file_kind = FileKind::Binary;
            identity = importer.identity_fields();
            (
                importer.id().to_string(),
                importer.import(&ctx, bytes).map_err(|e| format!("{e:#}"))?,
            )
        }
        None => {
            let content = decode(bytes).ok_or("binary file")?;
            let head: String = content.chars().take(64 * 1024).collect();
            let importer = importers.select(&ctx, &head).ok_or("no importer recognises this format")?;
            identity = importer.identity_fields();
            let imported = importer.import(&ctx, &content).map_err(|e| format!("{e:#}"))?;
            // Probe: an importer that cannot append is tailed by re-reading the file.
            if plain && importer.import_tail(&ctx, &head, "").is_some_and(|r| r.is_ok()) {
                append = Some(tail::AppendInfo {
                    importer: importer.id().to_string(),
                    multiline: importer.multiline(),
                    last_line: imported.records.last().map(|r| r.line),
                });
            }
            text = Some(content);
            (importer.id().to_string(), imported)
        }
    };
    if imported.records.is_empty() {
        return Err(format!("{id} importer found no events"));
    }
    let seed = append.map(|info| {
        let stats_from = |line: u32| {
            let tail: Vec<Record> = imported.records.iter().filter(|r| r.line >= line).cloned().collect();
            profile.ingest(tail, source_name).1
        };
        tail::TailSeed::append(path, bytes, &info, &stats_from)
    });
    let (records, stats) = profile.ingest(std::mem::take(&mut imported.records), source_name);
    let timestamp_issue = text.and_then(|t| crate::timestamp_issue::assess(&id, &records, &t, &profile.timestamps));
    let split = crate::route::split(&profile.routes, records, source_name);
    imported.records = split.unrouted;
    imported.problems.merge(split.problems);
    // A file split by routes is re-read whole when it changes, so its new events find their logical sources.
    let seed = seed.filter(|_| split.logical.is_empty());
    let mut identity = identity;
    for f in &choice.parser.format.identity {
        if !identity.contains(f) {
            identity.push(f.clone());
        }
    }
    Ok(Loaded {
        importer: id.to_string(),
        file_kind,
        imported,
        logical: split.logical,
        identity,
        parser: choice.reference.clone(),
        stats,
        seed,
        timestamp_issue,
    })
}

/// Reads a file, retrying briefly when another process holds it open
/// exclusively (common for logs being written on Windows).
pub(crate) fn read_with_retry(path: &Path) -> std::result::Result<Vec<u8>, String> {
    const ATTEMPTS: u32 = 4;
    let mut delay = std::time::Duration::from_millis(100);
    for attempt in 1..=ATTEMPTS {
        match std::fs::read(path) {
            Ok(b) => return Ok(b),
            Err(e) if is_locked(&e) && attempt < ATTEMPTS => {
                std::thread::sleep(delay);
                delay *= 2;
            }
            Err(e) if is_locked(&e) => return Err(format!("the file is locked by another process ({e})")),
            Err(e) => return Err(e.to_string()),
        }
    }
    unreachable!("the last attempt always returns")
}

/// Opens a file for reading, retrying briefly while another process holds it exclusively.
pub(crate) fn open_with_retry(path: &Path) -> std::io::Result<std::fs::File> {
    const ATTEMPTS: u32 = 4;
    let mut delay = std::time::Duration::from_millis(100);
    let mut attempt = 1;
    loop {
        match std::fs::File::open(path) {
            Err(e) if is_locked(&e) && attempt < ATTEMPTS => {
                std::thread::sleep(delay);
                delay *= 2;
                attempt += 1;
            }
            other => return other,
        }
    }
}

/// Sharing/lock violations (Windows error 32/33) or permission errors that may be transient.
pub(crate) fn is_locked(e: &std::io::Error) -> bool {
    matches!(e.raw_os_error(), Some(32) | Some(33)) && cfg!(windows) || e.kind() == std::io::ErrorKind::ResourceBusy
}

/// The file part of a source path ("x.zip!/a.log" -> "x.zip").
fn source_file(path: &str) -> &str {
    path.split(crate::archive::ENTRY_SEPARATOR).next().unwrap_or(path)
}

/// Path relative to the opened folder it came from, else `<parent dir>/<file name>`.
pub(crate) fn display_name(path: &Path, roots: &[PathBuf]) -> String {
    roots
        .iter()
        .filter(|r| r.is_dir())
        .filter_map(|r| path.strip_prefix(r).ok())
        .min_by_key(|p| p.components().count())
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|| explicit_name(path))
}

fn explicit_name(path: &Path) -> String {
    let file = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    match path.parent().and_then(Path::file_name) {
        Some(dir) => format!("{}/{file}", dir.to_string_lossy()),
        None => file,
    }
}

#[cfg(test)]
mod progress_tests {
    use super::*;

    #[test]
    fn eta_follows_the_byte_rate() {
        assert_eq!(eta(1000, 0, 100), None, "nothing read yet");
        assert_eq!(eta(0, 10, 100), None);
        assert_eq!(eta(1000, 100, 100), None, "finished");
        assert_eq!(eta(1000, 25, 100), Some(3000));
    }

    #[test]
    fn a_running_import_reports_the_file_in_flight() {
        let p = ImportProgress::default();
        let idle = p.snapshot();
        assert_eq!((idle.running, idle.elapsed_ms, idle.current), (false, 0, None));
        p.start(4, 400);
        p.reading(Path::new("/logs/a.log"));
        std::thread::sleep(std::time::Duration::from_millis(5));
        p.file_done(100);
        let s = p.snapshot();
        assert!(s.running && s.done == 1 && s.bytes_done == 100 && s.bytes_total == 400);
        assert_eq!(s.current.as_deref(), Some("/logs/a.log"));
        assert!(s.elapsed_ms >= 5 && s.eta_ms.is_some());
        p.finish();
        let s = p.snapshot();
        assert!(!s.running && s.current.is_none() && s.eta_ms.is_none() && s.elapsed_ms >= 5);
    }
}
