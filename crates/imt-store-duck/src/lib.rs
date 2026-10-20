//! IMT's DuckDB store (ADR 0014): one store folder per project. Events are zstd Parquet segments written
//! by the parse threads in parallel ([`segments`], `writer`); DuckDB keeps the catalog (sources,
//! diagnostics, the segment manifest, time-order positions) and runs every query over the `events` view.
//! Queries compile to SQL with a batch-scan fallback for what SQL cannot express yet.
//!
//! Operations not offered yet answer [`StoreError::Unsupported`] naming the phase that adds them.

mod compact;
mod fts;
mod pos;
pub mod promote;
mod read;
mod schema;
pub mod segments;
mod sql;
pub mod stats;
mod writer;

pub use compact::Compaction;
pub use schema::FORMAT_VERSION;

use duckdb::{params, Config, Connection};
use imt_core::api::{UnrecognizedFile, WorkspaceState};
use imt_core::batch::{EventId, EventSchema, SourceId};
use imt_core::ingest::{self, ImportEnv, Outcome, ParallelOptions, ProblemKind};
use imt_core::ports::{
    DiagnosticRow, Diagnostics, FixEvaluation, ImportReport, ImportSession, IndexStatus, Ingest, Profiles, QuickFix, Search, SearchReply,
    SearchRequest, StoreError, StoreResult,
};
use imt_core::profile::{CompiledProfile, Profile};
use imt_core::workspace::{ImportProgress, NOT_IMPORTED};
use imt_core::PluginRegistry;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// DuckDB's memory limit (ADR 0014 memory budget); it spills to the store's `tmp` folder beyond it.
pub const MEMORY_LIMIT: &str = "192MB";

/// DuckDB's query threads by default: each holds row groups being read, so more threads need more of
/// the memory limit (a sort over 24 threads ran out of 256 MB in the bake-off).
pub const DUCK_THREADS: usize = 4;

/// Limits of a store (ADR 0014 §2: configurable in Settings for larger machines).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreOptions {
    /// Parse threads of an import (see [`ingest::default_threads`]).
    pub parse_threads: usize,
    /// DuckDB query threads.
    pub duck_threads: usize,
    /// DuckDB memory limit, e.g. `256MB`.
    pub memory_limit: String,
}

impl Default for StoreOptions {
    fn default() -> Self {
        StoreOptions {
            parse_threads: ingest::default_threads(),
            duck_threads: DUCK_THREADS,
            memory_limit: MEMORY_LIMIT.to_string(),
        }
    }
}

/// File name of the database in a store folder.
pub const DB_FILE: &str = "data.duckdb";

/// Idle connections kept for reuse.
const POOL: usize = 8;

pub(crate) fn engine(e: impl std::fmt::Display) -> StoreError {
    StoreError::Engine(e.to_string())
}

/// What the store was opened with and what was opened in it.
struct State {
    profile: Arc<CompiledProfile>,
    roots: Vec<PathBuf>,
    removed: HashSet<PathBuf>,
    unrecognized: Vec<UnrecognizedFile>,
}

pub(crate) struct Inner {
    /// The store folder.
    dir: PathBuf,
    /// The connection the database was opened with; every other connection is cloned from it.
    db: Mutex<Connection>,
    pool: Mutex<Vec<Connection>>,
    registry: PluginRegistry,
    pub(crate) schema: Arc<EventSchema>,
    state: RwLock<State>,
    /// Bumped by every change to the events (keys read caches).
    pub(crate) generation: Arc<AtomicU64>,
    importing: AtomicBool,
    /// Held by whatever rewrites the catalog (an import, compaction, positions, removal).
    pub(crate) write_lock: Arc<Mutex<()>>,
    /// The full-text index and its indexing thread (`None` when the index cannot be opened: search then
    /// fails with the reason, everything else works).
    fts: Option<fts::FtsService>,
    fts_error: Option<String>,
    options: RwLock<StoreOptions>,
    progress: Arc<ImportProgress>,
    pub(crate) derived: Mutex<Option<(u64, Arc<read::Derived>)>>,
}

/// A connection borrowed from the pool, returned on drop.
pub(crate) struct Conn<'a> {
    inner: &'a Inner,
    conn: Option<Connection>,
}

impl std::ops::Deref for Conn<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.conn.as_ref().unwrap_or_else(|| unreachable!("taken only on drop"))
    }
}

impl Drop for Conn<'_> {
    fn drop(&mut self) {
        if let Some(c) = self.conn.take() {
            let mut pool = lock(&self.inner.pool);
            if pool.len() < POOL {
                pool.push(c);
            }
        }
    }
}

/// Locks recover from poisoning: a panicking reader must not take the store down.
pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn read_lock<T>(m: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    m.read().unwrap_or_else(|p| p.into_inner())
}

fn write_lock<T>(m: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    m.write().unwrap_or_else(|p| p.into_inner())
}

impl Inner {
    /// After the events changed: compaction, positions and a checkpoint. Failures are logged (the data is
    /// complete without them; the next change retries).
    fn maintain(&self, conn: &Connection) {
        let tc = std::time::Instant::now();
        match compact::compact(conn, &self.dir, &self.schema) {
            Ok(c) if c.replaced > 0 => log::info!("compacted {} segments into {}", c.replaced, c.written),
            Ok(_) => {}
            Err(e) => log::warn!("segment compaction failed: {e:#}"),
        }
        log::debug!("import: compact {:?}", tc.elapsed());
        if let Err(e) = pos::rebuild(conn) {
            log::warn!("time-order positions could not be rebuilt: {e}");
        }
        log::debug!("import: pos {:?}", tc.elapsed());
        // Write the catalog into the database file now, so a reopen does not replay the WAL.
        if let Err(e) = conn.execute_batch("CHECKPOINT") {
            log::warn!("checkpoint failed: {e}");
        }
        self.generation.fetch_add(1, Ordering::SeqCst);
        if let Some(f) = &self.fts {
            f.wake();
        }
    }

    pub(crate) fn conn(&self) -> StoreResult<Conn<'_>> {
        let pooled = lock(&self.pool).pop();
        let conn = match pooled {
            Some(c) => c,
            None => lock(&self.db).try_clone().map_err(engine)?,
        };
        Ok(Conn {
            inner: self,
            conn: Some(conn),
        })
    }

    pub(crate) fn profile(&self) -> Arc<CompiledProfile> {
        read_lock(&self.state).profile.clone()
    }

    pub(crate) fn registry(&self) -> &PluginRegistry {
        &self.registry
    }

    pub(crate) fn unrecognized(&self) -> Vec<UnrecognizedFile> {
        read_lock(&self.state).unrecognized.clone()
    }

    /// Reads `paths` (canonical) into the database.
    fn import(&self, paths: &[PathBuf], progress: &ImportProgress) -> StoreResult<ImportReport> {
        let started = std::time::Instant::now();
        let (profile, roots, removed) = {
            let s = read_lock(&self.state);
            (s.profile.clone(), s.roots.clone(), s.removed.clone())
        };
        let conn = self.conn()?;
        let mut known: HashSet<String> = HashSet::new();
        {
            let mut stmt = conn.prepare("SELECT path FROM sources").map_err(engine)?;
            let paths = stmt.query_map([], |r| r.get::<_, String>(0)).map_err(engine)?;
            for p in paths {
                let p = p.map_err(engine)?;
                known.insert(p.split(imt_core::archive::ENTRY_SEPARATOR).next().unwrap_or(&p).to_string());
            }
        }
        known.extend(self.unrecognized().into_iter().map(|u| u.path));
        let files = ingest::files_to_read(paths, &roots, &profile, &removed, &known);
        let env = ImportEnv {
            importers: self.registry.importers(),
            profile: &profile,
            schema: self.schema.clone(),
            cancel: progress.cancel_flag(),
        };
        let opts = ParallelOptions {
            threads: read_lock(&self.options).parse_threads.max(1),
            ..ParallelOptions::default()
        };
        let _writing = lock(&self.write_lock);
        let sink = writer::SegmentSink::new(
            conn.try_clone().map_err(engine)?,
            &self.dir,
            self.schema.clone(),
            opts.threads,
            self.generation.clone(),
            None,
        )
        .map_err(|e| engine(format!("{e:#}")))?;
        let t0 = std::time::Instant::now();
        let outcomes = ingest::import_files_parallel(&env, &files, &removed, progress, &opts, &sink);
        let t1 = t0.elapsed();
        sink.finish().map_err(|e| engine(format!("{e:#}")))?;
        let t2 = t0.elapsed();
        self.maintain(&conn);
        log::debug!("import: parse {t1:?} finish {t2:?} maintained {:?}", t0.elapsed());
        let mut report = ImportReport::default();
        let mut unrecognized = Vec::new();
        for o in outcomes {
            match o {
                Outcome::Imported(r) => {
                    report.events += r.events;
                    report.bytes += r.bytes;
                    report.sources.push(read::report_source(&r));
                }
                Outcome::Unrecognized(u) => unrecognized.push(u),
            }
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

/// A DuckDB store in a folder (`data.duckdb` plus a `tmp` spill folder). Cheap to share: clones use
/// the same database.
#[derive(Clone)]
pub struct DuckStore {
    inner: Arc<Inner>,
}

fn open_db(dir: &Path, opts: &StoreOptions) -> StoreResult<Connection> {
    let tmp = dir.join("tmp");
    std::fs::create_dir_all(&tmp)?;
    let config = Config::default()
        .max_memory(&opts.memory_limit)
        .and_then(|c| c.threads(opts.duck_threads.max(1) as i64))
        .and_then(|c| c.with("temp_directory", tmp.to_string_lossy()))
        .and_then(|c| c.enable_autoload_extension(false))
        // The catalog is small; a checkpoint at the end of each change is enough.
        .and_then(|c| c.with("checkpoint_threshold", "1GB"))
        .map_err(engine)?;
    let conn = Connection::open_with_flags(dir.join(DB_FILE), config).map_err(engine)?;
    // Segment footers are parsed once, not per query (a setting of the Parquet reader, so set after opening).
    if let Err(e) = conn.execute_batch("SET GLOBAL parquet_metadata_cache = true") {
        log::warn!("Parquet metadata cache unavailable: {e}");
    }
    Ok(conn)
}

/// After opening: drops what a crash left (unlisted segment files, the trash) and points the `events`
/// view at this folder's segments (the store may have been moved or unpacked elsewhere).
fn recover(conn: &Connection, dir: &Path, schema: &EventSchema) -> anyhow::Result<()> {
    let listed: std::collections::BTreeSet<String> = {
        let mut stmt = conn.prepare("SELECT file FROM segments")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect::<duckdb::Result<_>>()?
    };
    let seg_dir = dir.join(segments::SEGMENTS_DIR);
    std::fs::create_dir_all(&seg_dir)?;
    for f in &listed {
        anyhow::ensure!(
            seg_dir.join(f).is_file(),
            "the segment {f} of this project is missing; open its log files again"
        );
    }
    for left in segments::sweep(&seg_dir, &listed) {
        log::warn!("could not delete {}", left.display());
    }
    compact::empty_trash(conn)?;
    writer::refresh_view(conn, dir, schema)
}

fn invalid(e: anyhow::Error) -> StoreError {
    StoreError::Invalid(format!("{e:#}"))
}

impl DuckStore {
    /// A new, empty store in `dir` (an existing database there is replaced) reading with `profile`
    /// and the plugins in `registry`.
    pub fn create(dir: &Path, profile: Profile, registry: PluginRegistry) -> StoreResult<DuckStore> {
        Self::create_with(dir, profile, registry, StoreOptions::default())
    }

    /// [`DuckStore::create`] with chosen limits.
    pub fn create_with(dir: &Path, profile: Profile, registry: PluginRegistry, opts: StoreOptions) -> StoreResult<DuckStore> {
        registry.resolve();
        let compiled = registry.compile_profile(profile).map_err(invalid)?;
        std::fs::create_dir_all(dir)?;
        for f in [DB_FILE.to_string(), format!("{DB_FILE}.wal")] {
            match std::fs::remove_file(dir.join(f)) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                _ => {}
            }
        }
        match std::fs::remove_dir_all(dir.join(segments::SEGMENTS_DIR)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
        let conn = open_db(dir, &opts)?;
        let schema = ingest::schema_for(&compiled);
        conn.execute_batch(&schema::ddl(&schema)).map_err(engine)?;
        conn.execute_batch(&schema::events_view(&schema, &[])).map_err(engine)?;
        for (k, v) in [
            ("format_version", FORMAT_VERSION.to_string()),
            ("declared", schema::encode_declared(&schema)),
            ("profile", compiled.profile.name.clone()),
        ] {
            conn.execute("INSERT INTO meta VALUES (?, ?)", params![k, v]).map_err(engine)?;
        }
        for (bit, m) in compiled.markers.iter().enumerate() {
            conn.execute("INSERT INTO marker_kinds VALUES (0, ?, ?, ?)", params![bit as u8, m.kind, m.owner])
                .map_err(engine)?;
        }
        Ok(Self::with(dir, conn, schema, compiled, registry, opts))
    }

    /// Reopens the store in `dir`. A database written by a newer IMT is refused with a message.
    pub fn open(dir: &Path, profile: Profile, registry: PluginRegistry) -> StoreResult<DuckStore> {
        Self::open_with(dir, profile, registry, StoreOptions::default())
    }

    /// [`DuckStore::open`] with chosen limits.
    pub fn open_with(dir: &Path, profile: Profile, registry: PluginRegistry, opts: StoreOptions) -> StoreResult<DuckStore> {
        if !dir.join(DB_FILE).is_file() {
            return Err(StoreError::NotFound(format!("no store in {}", dir.display())));
        }
        registry.resolve();
        let compiled = registry.compile_profile(profile).map_err(invalid)?;
        let conn = open_db(dir, &opts)?;
        let meta = |key: &str| -> StoreResult<String> {
            conn.query_row("SELECT value FROM meta WHERE key = ?", params![key], |r| r.get(0))
                .map_err(|e| StoreError::Invalid(format!("not an IMT store ({key}: {e})")))
        };
        let version: u32 = meta("format_version")?.parse().unwrap_or(u32::MAX);
        if version > FORMAT_VERSION {
            return Err(StoreError::Invalid(format!(
                "this project was saved by a newer IMT (store format {version}; this IMT reads up to {FORMAT_VERSION}); update IMT to open it"
            )));
        }
        let schema =
            schema::decode_declared(&meta("declared")?).ok_or_else(|| StoreError::Invalid("the store's column list is damaged".into()))?;
        if version < FORMAT_VERSION {
            return Err(StoreError::Invalid(format!(
                "this project was saved by an older preview of IMT (store format {version}); open its log files again"
            )));
        }
        recover(&conn, dir, &schema).map_err(|e| engine(format!("{e:#}")))?;
        Ok(Self::with(dir, conn, schema, compiled, registry, opts))
    }

    fn with(
        dir: &Path,
        conn: Connection,
        schema: EventSchema,
        profile: CompiledProfile,
        registry: PluginRegistry,
        opts: StoreOptions,
    ) -> DuckStore {
        let write_lock = Arc::new(Mutex::new(()));
        let started = conn
            .try_clone()
            .map_err(anyhow::Error::from)
            .and_then(|c| fts::FtsService::start(dir, c, write_lock.clone()));
        let (fts, fts_error) = match started {
            Ok(f) => (Some(f), None),
            Err(e) => {
                log::warn!("the full-text index is unavailable: {e:#}");
                (None, Some(format!("the full-text index is unavailable: {e:#}")))
            }
        };
        DuckStore {
            inner: Arc::new(Inner {
                dir: dir.to_path_buf(),
                db: Mutex::new(conn),
                pool: Mutex::new(Vec::new()),
                registry,
                schema: Arc::new(schema),
                state: RwLock::new(State {
                    profile: Arc::new(profile),
                    roots: Vec::new(),
                    removed: HashSet::new(),
                    unrecognized: Vec::new(),
                }),
                generation: Arc::new(AtomicU64::new(0)),
                importing: AtomicBool::new(false),
                write_lock,
                fts,
                fts_error,
                options: RwLock::new(opts),
                progress: Arc::new(ImportProgress::default()),
                derived: Mutex::new(None),
            }),
        }
    }

    /// The store folder.
    pub fn dir(&self) -> &Path {
        &self.inner.dir
    }

    /// Changes the limits; parse threads apply to the next import, DuckDB's settings at once.
    pub fn configure(&self, opts: StoreOptions) -> StoreResult<()> {
        let conn = self.inner.conn()?;
        let limit = sql::lit(&opts.memory_limit).ok_or_else(|| StoreError::Invalid("bad memory limit".into()))?;
        conn.execute_batch(&format!("SET memory_limit = {limit}; SET threads = {};", opts.duck_threads.max(1)))
            .map_err(|e| StoreError::Invalid(format!("{e}")))?;
        *write_lock(&self.inner.options) = opts;
        Ok(())
    }

    /// Merges small segments and drops rows of removed sources now (normally done after each change).
    pub fn compact(&self) -> StoreResult<Compaction> {
        self.idle()?;
        let _writing = lock(&self.inner.write_lock);
        let conn = self.inner.conn()?;
        let c = compact::compact(&conn, &self.inner.dir, &self.inner.schema).map_err(|e| engine(format!("{e:#}")))?;
        self.changed();
        Ok(c)
    }

    /// Progress of the running (or last) import.
    pub fn progress(&self) -> Arc<ImportProgress> {
        self.inner.progress.clone()
    }
}

/// Clears the "import running" flag when the job ends, however it ends.
struct Importing(Arc<Inner>);

impl Drop for Importing {
    fn drop(&mut self) {
        self.0.importing.store(false, Ordering::SeqCst);
    }
}

impl DuckStore {
    fn idle(&self) -> StoreResult<()> {
        if self.inner.importing.load(Ordering::SeqCst) {
            return Err(StoreError::Busy("an import is running; wait for it or cancel it".into()));
        }
        Ok(())
    }

    fn changed(&self) {
        self.inner.generation.fetch_add(1, Ordering::SeqCst);
    }
}

impl Ingest for DuckStore {
    fn open(&self, paths: &[PathBuf]) -> StoreResult<ImportSession> {
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
            // Files a cancelled import did not read are read by this one.
            s.unrecognized.retain(|u| u.reason != NOT_IMPORTED);
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
            inner.import(&canonical, progress)
        })
    }

    fn reload(&self) -> StoreResult<ImportSession> {
        Err(StoreError::unsupported("reload", "R2c"))
    }

    /// Changed files (and the members of changed archives) are read again, deleted ones dropped, and new
    /// files under the opened folders read.
    fn refresh(&self, changed: &[PathBuf]) -> StoreResult<ImportSession> {
        self.idle()?;
        {
            let _writing = lock(&self.inner.write_lock);
            let conn = self.inner.conn()?;
            let mut gone = Vec::new();
            for p in changed {
                // A deleted file cannot be canonicalised: its folder is.
                let p = std::fs::canonicalize(p).unwrap_or_else(|_| match (p.parent().map(std::fs::canonicalize), p.file_name()) {
                    (Some(Ok(dir)), Some(name)) => dir.join(name),
                    _ => p.clone(),
                });
                let path = p.to_string_lossy().into_owned();
                let member = format!("{path}{}%", imt_core::archive::ENTRY_SEPARATOR.replace('%', "\\%"));
                let mut stmt = conn
                    .prepare("SELECT id FROM sources WHERE path = ? OR path LIKE ? ESCAPE '\\'")
                    .map_err(engine)?;
                let ids = stmt
                    .query_map(params![path, member], |r| r.get::<_, u32>(0))
                    .map_err(engine)?
                    .collect::<duckdb::Result<Vec<_>>>()
                    .map_err(engine)?;
                gone.extend(ids);
                write_lock(&self.inner.state)
                    .unrecognized
                    .retain(|u| u.path != path && !u.path.starts_with(&format!("{path}{}", imt_core::archive::ENTRY_SEPARATOR)));
            }
            if !gone.is_empty() {
                let list = gone.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", ");
                conn.execute_batch(&format!(
                    "BEGIN TRANSACTION; DELETE FROM diagnostics WHERE source IN ({list}); DELETE FROM sources WHERE id IN ({list}); COMMIT;"
                ))
                .map_err(engine)?;
                if let Some(f) = &self.inner.fts {
                    for id in &gone {
                        if let Err(e) = f.delete_source(*id) {
                            log::warn!("could not remove source {id} from the full-text index: {e:#}");
                        }
                    }
                }
                self.changed();
            }
        }
        // Reading the opened roots again reads what is not in the store: the changed and the new files.
        let roots: Vec<PathBuf> = read_lock(&self.inner.state).roots.iter().filter(|r| r.exists()).cloned().collect();
        self.open(&roots)
    }

    fn remove_source(&self, id: SourceId) -> StoreResult<()> {
        self.idle()?;
        let conn = self.inner.conn()?;
        let path: Option<String> = conn
            .query_row("SELECT path FROM sources WHERE id = ?", params![id.0], |r| r.get(0))
            .ok();
        let Some(path) = path else {
            return Err(StoreError::NotFound(format!("source {id}")));
        };
        let _writing = lock(&self.inner.write_lock);
        // The view hides the source's rows at once; compaction rewrites its segments.
        for sql in ["DELETE FROM diagnostics WHERE source = ?", "DELETE FROM sources WHERE id = ?"] {
            conn.execute(sql, params![id.0]).map_err(engine)?;
        }
        if let Some(f) = &self.inner.fts {
            if let Err(e) = f.delete_source(id.0) {
                log::warn!("could not remove source {id} from the full-text index: {e:#}");
            }
        }
        self.inner.maintain(&conn);
        let mut s = write_lock(&self.inner.state);
        let p = PathBuf::from(&path);
        s.roots.retain(|r| *r != p);
        s.removed.insert(p);
        drop(s);
        self.changed();
        Ok(())
    }

    fn clear(&self) -> StoreResult<()> {
        self.idle()?;
        let conn = self.inner.conn()?;
        let _writing = lock(&self.inner.write_lock);
        let files = writer::segment_files(&conn, &self.inner.dir).map_err(|e| engine(format!("{e:#}")))?;
        conn.execute_batch(
            "BEGIN TRANSACTION; DELETE FROM segments; DELETE FROM segment_fields; DELETE FROM histogram; DELETE FROM ord; DELETE FROM diagnostics; DELETE FROM sources; COMMIT;",
        )
        .map_err(engine)?;
        writer::refresh_view(&conn, &self.inner.dir, &self.inner.schema).map_err(|e| engine(format!("{e:#}")))?;
        if let Some(f) = &self.inner.fts {
            if let Err(e) = f.clear() {
                log::warn!("could not clear the full-text index: {e:#}");
            }
        }
        for f in files {
            if std::fs::remove_file(&f).is_err() && Path::new(&f).exists() {
                let _ = conn.execute("INSERT OR IGNORE INTO trash VALUES (?)", params![f]);
            }
        }
        let mut s = write_lock(&self.inner.state);
        s.roots.clear();
        s.removed.clear();
        s.unrecognized.clear();
        drop(s);
        self.changed();
        Ok(())
    }

    fn restore(&self, _state: &WorkspaceState) -> StoreResult<ImportSession> {
        Err(StoreError::unsupported("restore", "R2c"))
    }

    fn append_tail(&self, _changed: &[PathBuf]) -> StoreResult<usize> {
        Err(StoreError::unsupported("live tail", "6c"))
    }
}

impl Profiles for DuckStore {
    fn active_profile(&self) -> StoreResult<Profile> {
        Ok(self.inner.profile().profile.clone())
    }

    fn validate_profile(&self, profile: &Profile) -> StoreResult<()> {
        self.inner.registry.compile_profile(profile.clone()).map(|_| ()).map_err(invalid)
    }

    fn set_profile(&self, _profile: Profile) -> StoreResult<ImportSession> {
        Err(StoreError::unsupported("changing the profile", "R2c"))
    }

    fn preview_parse(&self, _req: &imt_core::api::ParsePreviewRequest) -> StoreResult<imt_core::api::ParsePreview> {
        Err(StoreError::unsupported("parse preview", "R2c"))
    }

    fn preview_extractor(&self, _req: &imt_core::api::ExtractPreviewRequest) -> StoreResult<imt_core::api::ExtractPreview> {
        Err(StoreError::unsupported("extractor preview", "R2c"))
    }
}

impl Diagnostics for DuckStore {
    fn problems(&self) -> StoreResult<Vec<DiagnosticRow>> {
        let conn = self.inner.conn()?;
        let mut stmt = conn
            .prepare("SELECT d.source, s.name, d.kind, d.count, d.samples FROM diagnostics d JOIN sources s ON s.id = d.source ORDER BY s.name, d.kind")
            .map_err(engine)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, u32>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, u64>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })
            .map_err(engine)?;
        let mut out = Vec::new();
        for row in rows {
            let (source, name, kind, count, samples) = row.map_err(engine)?;
            // Rows of kinds this build does not know (a newer writer) are skipped.
            let Some(kind) = ProblemKind::from_id(&kind) else { continue };
            out.push(DiagnosticRow::new(
                SourceId(source),
                name,
                ingest::Problem {
                    kind,
                    count,
                    sample_lines: serde_json::from_str(&samples).unwrap_or_default(),
                },
            ));
        }
        Ok(out)
    }

    fn evaluate_fix(&self, _fix: &QuickFix) -> StoreResult<FixEvaluation> {
        Err(StoreError::unsupported("quick-fix evaluation", "R2c"))
    }
}

impl DuckStore {
    fn fts(&self) -> StoreResult<&fts::FtsService> {
        self.inner
            .fts
            .as_ref()
            .ok_or_else(|| StoreError::Engine(self.inner.fts_error.clone().unwrap_or_else(|| "no full-text index".into())))
    }

    /// Waits until the full-text index covers every segment (tests and benchmarks), at most `timeout`.
    pub fn wait_for_index(&self, timeout: std::time::Duration) -> StoreResult<IndexStatus> {
        let started = std::time::Instant::now();
        loop {
            let status = self.index_status()?;
            if status.ready || started.elapsed() > timeout {
                return Ok(status);
            }
            self.fts()?.wake();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}

impl Search for DuckStore {
    /// Words of `text` (all required), case-insensitive and split on punctuation; the first `limit` hits
    /// (default 1,000, at most 10,000) in time order. A quoted phrase must also appear as written (ignoring
    /// case) in the message, raw text or exception; up to 1,000 candidates are checked, beyond that the
    /// total is not exact. Only indexed segments are searched ([`Search::index_status`] tells how far
    /// indexing is).
    fn search(&self, req: &SearchRequest) -> StoreResult<SearchReply> {
        let limit = if req.limit == 0 {
            fts::DEFAULT_HITS
        } else {
            req.limit.min(fts::MAX_HITS)
        };
        let index = &self.fts()?.shared.fts;
        let (words, phrases) = fts::split_phrases(&req.text);
        if phrases.is_empty() {
            let (total, ids) = index.search(&words, limit).map_err(|e| engine(format!("{e:#}")))?;
            return Ok(SearchReply { total, ids, exact: true });
        }
        // The index has no positions: its hits hold every word; the phrases are checked on the rows.
        let (candidates, ids) = index.search(&words, fts::MAX_VERIFY).map_err(|e| engine(format!("{e:#}")))?;
        let mut matched = Vec::new();
        let mut verified = 0u64;
        for chunk in ids.chunks(imt_core::ports::MAX_PAGE_ROWS) {
            let ok: HashSet<EventId> = self
                .texts(chunk)?
                .into_iter()
                .filter(|(_, text)| phrases.iter().all(|p| text.contains(p.as_str())))
                .map(|(id, _)| id)
                .collect();
            for id in chunk.iter().filter(|id| ok.contains(*id)) {
                verified += 1;
                if matched.len() < limit {
                    matched.push(*id);
                }
            }
        }
        let exact = candidates as usize <= ids.len();
        Ok(SearchReply {
            total: if exact { verified } else { candidates },
            ids: matched,
            exact,
        })
    }

    fn index_status(&self) -> StoreResult<IndexStatus> {
        let f = self.fts()?;
        let conn = self.inner.conn()?;
        let (pending, total): (u64, u64) = conn
            .query_row(
                "SELECT (SELECT count(*) FROM segments WHERE NOT indexed), (SELECT coalesce(sum(events), 0) FROM sources WHERE ready)",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(engine)?;
        if let Some(e) = crate::lock(&f.shared.failure).clone() {
            log::debug!("last indexing failure: {e}");
        }
        Ok(IndexStatus {
            indexed: f.shared.fts.docs(),
            total,
            ready: pending == 0,
        })
    }
}
