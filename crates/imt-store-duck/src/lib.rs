//! IMT's DuckDB store (ADR 0014): one store folder per project. Events are zstd Parquet segments written
//! by the parse threads in parallel ([`segments`], `writer`); DuckDB keeps the catalog (sources,
//! diagnostics, the segment manifest, time-order positions) and runs every query over the `events` view.
//! Queries compile to SQL with a batch-scan fallback for what SQL cannot express yet.
//!
//! Operations not offered yet answer [`StoreError::Unsupported`] naming the phase that adds them.

pub mod allocator;
mod analysis;
mod anomalies;
mod compact;
mod console;
mod context;
mod diag;
mod fts;
mod hits;
mod inspect;
mod lanes;
mod parsers;
mod pos;
mod profiles;
mod project;
pub mod promote;
mod qlang;
mod read;
mod schema;
pub mod segments;
mod spans;
mod sql;
pub mod stats;
mod writer;

pub use compact::Compaction;
pub use fts::WriterBudget;
pub use schema::FORMAT_VERSION;

use duckdb::{params, Config, Connection};
use imt_core::api::{UnrecognizedFile, WorkspaceState};
use imt_core::batch::{EventId, EventSchema, SourceId};
use imt_core::ingest::{self, ImportEnv, Outcome, ParallelOptions};
use imt_core::parser::{ParserSet, ParserSetup};
use imt_core::ports::{ImportReport, ImportSession, IndexStatus, Ingest, Search, SearchReply, SearchRequest, StoreError, StoreResult};
use imt_core::profile::{CompiledProfile, Profile};
use imt_core::workspace::{ImportProgress, NOT_IMPORTED};
use imt_core::PluginRegistry;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// DuckDB's memory limit (ADR 0014 memory budget); it spills to the store's `tmp` folder beyond it.
pub const MEMORY_LIMIT: &str = "192MB";

/// Allocations DuckDB's allocator may keep cached after a task (`allocator_flush_threshold`).
pub const ALLOCATOR_FLUSH: &str = "32MiB";

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
    /// The full-text index writer's heap and threads (applies when the store opens).
    pub index: fts::WriterBudget,
    /// Where DuckDB spills beyond its memory limit: a folder per store below this one (see [`spill_folder`]);
    /// `None`: the store folder's `tmp` (applies when the store opens).
    pub spill_dir: Option<PathBuf>,
    /// The most DuckDB may spill (`max_temp_directory_size`, e.g. `10GB`); `None`: DuckDB's default (90% of the free
    /// disk space).
    pub max_spill: Option<String>,
    /// Freed memory DuckDB's allocator may keep cached (`allocator_flush_threshold`).
    pub allocator_flush: String,
    /// DuckDB's allocator background threads (`allocator_background_threads`).
    pub allocator_background_threads: bool,
}

impl Default for StoreOptions {
    fn default() -> Self {
        StoreOptions {
            parse_threads: ingest::default_threads(),
            duck_threads: DUCK_THREADS,
            memory_limit: MEMORY_LIMIT.to_string(),
            index: fts::WriterBudget::default(),
            spill_dir: None,
            max_spill: None,
            allocator_flush: ALLOCATOR_FLUSH.to_string(),
            allocator_background_threads: true,
        }
    }
}

/// The spill folder of the store in `dir` below the chosen spill folder `base`: one folder per store (stores never
/// share DuckDB's temporary files), named by a stable hash of the store folder.
pub fn spill_folder(base: &Path, dir: &Path) -> PathBuf {
    // FNV-1a, 64 bits: stable across runs, so a store reuses (and cleans) its own folder.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in dir.to_string_lossy().bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    base.join(format!("imt-spill-{h:016x}"))
}

/// Parse threads in low memory mode (each holds a file's chunk and its batches in flight; 2 halved the speed).
pub const LOW_MEMORY_PARSE_THREADS: usize = 4;
/// DuckDB's memory limit in low memory mode.
pub const LOW_MEMORY_LIMIT: &str = "128MB";

impl StoreOptions {
    /// Low memory mode (a user setting, off by default): fewer parse threads and so fewer batches in flight, a
    /// smaller DuckDB limit with fewer threads, a smaller index writer. Imports are slower (20-40%); the worker
    /// also gives freed memory back at once (`imt_alloc::purge_at_once`).
    pub fn low_memory() -> StoreOptions {
        StoreOptions {
            parse_threads: LOW_MEMORY_PARSE_THREADS.min(ingest::default_threads()),
            duck_threads: 2,
            memory_limit: LOW_MEMORY_LIMIT.to_string(),
            index: fts::WriterBudget {
                heap: 16 << 20,
                threads: 1,
            },
            ..StoreOptions::default()
        }
    }
}

/// Stores with at least this many events answer timelines from the pre-aggregated histogram.
pub const PREAGGREGATE_ABOVE: u64 = 1_000_000;

/// File name of the database in a store folder.
pub const DB_FILE: &str = "data.duckdb";

/// Idle connections kept for reuse.
const POOL: usize = 8;

pub(crate) fn engine(e: impl std::fmt::Display) -> StoreError {
    classify(e.to_string())
}

/// A database failure as the typed error it is: a lock held by another process, a damaged data file, or an
/// engine failure.
fn classify(message: String) -> StoreError {
    let lower = message.to_lowercase();
    // Linux/macOS (a POSIX lock) and Windows (a sharing violation).
    if lower.contains("could not set lock on file") || lower.contains("conflicting lock") || lower.contains("file is already open in") {
        StoreError::Locked(format!("this project is open in another IMT window or program ({message})"))
    } else if lower.contains("no magic bytes found")
        || lower.contains("too small to be a parquet file")
        || (lower.contains("parquet") && (lower.contains("corrupt") || lower.contains("invalid footer")))
    {
        StoreError::Damaged(format!("a data file of this project is damaged ({message})"))
    } else {
        StoreError::Engine(message)
    }
}

/// A segment the catalog lists is not in the folder.
#[derive(Debug)]
struct MissingSegment(String);

impl std::fmt::Display for MissingSegment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the data file {} of this project is missing", self.0)
    }
}

impl std::error::Error for MissingSegment {}

/// What the store was opened with and what was opened in it.
pub(crate) struct State {
    /// The profile with the generic parser (format-independent analysis).
    pub(crate) profile: Arc<CompiledProfile>,
    /// The parser library compiled with the profile, with this store's per-source choices (ADR 0026).
    pub(crate) parsers: Arc<ParserSet>,
    pub(crate) roots: Vec<PathBuf>,
    removed: HashSet<PathBuf>,
    unrecognized: Vec<UnrecognizedFile>,
}

pub(crate) struct Inner {
    /// The store folder.
    pub(crate) dir: PathBuf,
    /// The connection the database was opened with; every other connection is cloned from it.
    db: Mutex<Connection>,
    pool: Mutex<Vec<Connection>>,
    registry: PluginRegistry,
    pub(crate) schema: Arc<EventSchema>,
    pub(crate) state: RwLock<State>,
    /// Bumped by every change to the events (keys read caches).
    pub(crate) generation: Arc<AtomicU64>,
    pub(crate) importing: AtomicBool,
    /// Held by whatever rewrites the catalog (an import, compaction, positions, removal).
    pub(crate) write_lock: Arc<Mutex<()>>,
    /// The full-text index and its indexing thread (`None` when the index cannot be opened: search then
    /// fails with the reason, everything else works).
    fts: Option<fts::FtsService>,
    fts_error: Option<String>,
    /// Word-search hits handed to SQL (smart search).
    pub(crate) hits: hits::HitsCache,
    /// The SQL console's sandboxed database and its kept results (ADR 0030).
    pub(crate) console: console::ConsoleState,
    options: RwLock<StoreOptions>,
    pub(crate) progress: Arc<ImportProgress>,
    pub(crate) derived: Mutex<Option<(u64, Arc<read::Derived>)>>,
    /// From this many events on, timelines use the pre-aggregated histogram (approximate at bucket edges)
    /// instead of grouping the events.
    pub(crate) preaggregate_above: AtomicU64,
}

/// A connection borrowed from the pool, returned on drop.
pub(crate) struct Conn<'a> {
    inner: &'a Inner,
    conn: Option<Connection>,
    /// Interrupts this connection's statement when the thread's current control (`QueryControl::scope`) is cancelled.
    _interrupt: Option<imt_core::ql::HookGuard>,
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

pub(crate) fn read_lock<T>(m: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    m.read().unwrap_or_else(|p| p.into_inner())
}

pub(crate) fn write_lock<T>(m: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
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
        let interrupt = match imt_core::ql::QueryControl::current() {
            Some(control) if control.is_cancelled() => return Err(StoreError::Cancelled),
            Some(control) => {
                let handle = conn.interrupt_handle();
                Some(control.on_cancel(Box::new(move || handle.interrupt())))
            }
            None => None,
        };
        Ok(Conn {
            inner: self,
            conn: Some(conn),
            _interrupt: interrupt,
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

    pub(crate) fn is_importing(&self) -> bool {
        self.importing.load(Ordering::SeqCst)
    }

    /// Bumps the persisted data version (cached analyses of older versions are dropped).
    pub(crate) fn bump_version(&self, conn: &Connection) -> StoreResult<()> {
        let current: u64 = conn
            .query_row("SELECT value FROM meta WHERE key = 'data_version'", [], |r| r.get::<_, String>(0))
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let next = current + 1;
        conn.execute("INSERT OR REPLACE INTO meta VALUES ('data_version', ?)", params![next.to_string()])
            .map_err(engine)?;
        conn.execute("DELETE FROM analysis_cache WHERE version < ?", params![next])
            .map_err(engine)?;
        conn.execute_batch(&lanes::prune_sql(next)).map_err(engine)?;
        Ok(())
    }

    /// Forgets every event, source and problem (the opened roots stay): the manifest is emptied, the views
    /// refreshed and the segment files deleted (or moved to `trash` while a reader holds them).
    pub(crate) fn forget_events(&self) -> StoreResult<()> {
        let conn = self.conn()?;
        let _writing = lock(&self.write_lock);
        let files = writer::segment_files(&conn, &self.dir).map_err(|e| engine(format!("{e:#}")))?;
        conn.execute_batch(
            "BEGIN TRANSACTION; DELETE FROM segments; DELETE FROM segment_fields; DELETE FROM histogram; DELETE FROM ord; DELETE FROM diagnostics; DELETE FROM sources; COMMIT;",
        )
        .map_err(engine)?;
        writer::refresh_view(&conn, &self.dir, &self.schema).map_err(|e| engine(format!("{e:#}")))?;
        if let Some(f) = &self.fts {
            if let Err(e) = f.clear() {
                log::warn!("could not clear the full-text index: {e:#}");
            }
        }
        for f in files {
            if std::fs::remove_file(&f).is_err() && Path::new(&f).exists() {
                let _ = conn.execute("INSERT OR IGNORE INTO trash VALUES (?)", params![f]);
            }
        }
        write_lock(&self.state).unrecognized.clear();
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.bump_version(&conn)
    }

    /// Reads `paths` (canonical) into the database.
    pub(crate) fn import(&self, paths: &[PathBuf], progress: &ImportProgress) -> StoreResult<ImportReport> {
        let started = std::time::Instant::now();
        let (profile, parser_set, roots, removed) = {
            let s = read_lock(&self.state);
            (s.profile.clone(), s.parsers.clone(), s.roots.clone(), s.removed.clone())
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
            parsers: &parser_set,
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
        // Each source recorded its parser; the catalog keeps a copy of every parser's TOML (ADR 0026).
        parsers::record(&conn, &parser_set).map_err(engine)?;
        let t2 = t0.elapsed();
        self.maintain(&conn);
        self.bump_version(&conn)?;
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
        inspect::record_import(&conn, &report);
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
    let tmp = match &opts.spill_dir {
        Some(base) => spill_folder(base, dir),
        None => dir.join("tmp"),
    };
    std::fs::create_dir_all(&tmp)?;
    let background = if opts.allocator_background_threads { "true" } else { "false" };
    let config = Config::default()
        .max_memory(&opts.memory_limit)
        .and_then(|c| c.threads(opts.duck_threads.max(1) as i64))
        .and_then(|c| c.with("temp_directory", tmp.to_string_lossy()))
        .and_then(|c| c.enable_autoload_extension(false))
        // The catalog is small; a checkpoint at the end of each change is enough.
        .and_then(|c| c.with("checkpoint_threshold", "1GB"))
        // Freed memory goes back to the OS after each task above this (DuckDB's jemalloc on Linux keeps up to
        // 128 MB by default, which the worker's 300 MB query budget cannot spare).
        .and_then(|c| c.with("allocator_flush_threshold", &opts.allocator_flush))
        // DuckDB's own jemalloc (IMT's Rust code uses mimalloc) returns freed pages through background threads, off by
        // default: without them a Linux store kept anomaly detection's freed memory (+165 MB over a 64 MB limit in
        // CI). No effect where DuckDB uses the system allocator.
        .and_then(|c| c.with("allocator_background_threads", background))
        .and_then(|c| match &opts.max_spill {
            Some(max) => c.with("max_temp_directory_size", max),
            None => Ok(c),
        })
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
        if !seg_dir.join(f).is_file() {
            return Err(MissingSegment(f.clone()).into());
        }
    }
    for left in segments::sweep(&seg_dir, &listed) {
        log::warn!("could not delete {}", left.display());
    }
    compact::empty_trash(conn)?;
    writer::refresh_view(conn, dir, schema)
}

pub(crate) fn invalid(e: anyhow::Error) -> StoreError {
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
        let parsers = registry.compile_parsers(&profile, &ParserSetup::builtins()).map_err(invalid)?;
        let compiled = parsers.base().clone();
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
        conn.execute_batch(schema::CACHE_DDL).map_err(engine)?;
        conn.execute_batch(lanes::DDL).map_err(engine)?;
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
        Ok(Self::with(dir, conn, schema, parsers, registry, opts))
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
        let parsers = registry.compile_parsers(&profile, &ParserSetup::builtins()).map_err(invalid)?;
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
        recover(&conn, dir, &schema).map_err(|e| match e.downcast_ref::<MissingSegment>() {
            Some(m) => StoreError::Damaged(m.to_string()),
            None => engine(format!("{e:#}")),
        })?;
        // Added after the format was first written; created on open so older stores gain it.
        conn.execute_batch(schema::CACHE_DDL).map_err(engine)?;
        conn.execute_batch(lanes::DDL).map_err(engine)?;
        Ok(Self::with(dir, conn, schema, parsers, registry, opts))
    }

    fn with(
        dir: &Path,
        conn: Connection,
        schema: EventSchema,
        parsers: ParserSet,
        registry: PluginRegistry,
        opts: StoreOptions,
    ) -> DuckStore {
        // The store keeps its own "Parse with…" choices (ADR 0026); the library comes from the app.
        let overrides = parsers::overrides(&conn);
        let parsers = if overrides.is_empty() {
            parsers
        } else {
            let mut setup = parsers.setup().clone();
            setup.overrides = overrides;
            match registry.compile_parsers(&parsers.base().profile, &setup) {
                Ok(p) => p,
                Err(e) => {
                    log::warn!("the parser choices of this store cannot be applied: {e:#}");
                    parsers
                }
            }
        };
        let parsers = Arc::new(parsers);
        let write_lock = Arc::new(Mutex::new(()));
        let started = conn
            .try_clone()
            .map_err(anyhow::Error::from)
            .and_then(|c| fts::FtsService::start(dir, c, write_lock.clone(), opts.index));
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
                    profile: parsers.base().clone(),
                    parsers,
                    roots: Vec::new(),
                    removed: HashSet::new(),
                    unrecognized: Vec::new(),
                }),
                generation: Arc::new(AtomicU64::new(0)),
                importing: AtomicBool::new(false),
                write_lock,
                fts,
                fts_error,
                hits: hits::HitsCache::new(&dir.join("tmp").join("hits")),
                console: console::ConsoleState::default(),
                options: RwLock::new(opts),
                progress: Arc::new(ImportProgress::default()),
                derived: Mutex::new(None),
                preaggregate_above: AtomicU64::new(PREAGGREGATE_ABOVE),
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

    /// From how many events on timelines are answered from the histogram (tests lower it).
    pub fn set_preaggregate_above(&self, events: u64) {
        self.inner.preaggregate_above.store(events, Ordering::Relaxed);
    }

    /// Runs `f` with `control` interrupting every statement the store runs for it on this thread: a cancelled
    /// analysis, export or preview stops at once and fails with [`StoreError::Cancelled`].
    pub fn with_control<R>(control: &imt_core::ql::QueryControl, f: impl FnOnce() -> StoreResult<R>) -> StoreResult<R> {
        match control.scope(f) {
            Err(_) if control.is_cancelled() => Err(StoreError::Cancelled),
            other => other,
        }
    }

    /// Progress of the running (or last) import.
    pub fn progress(&self) -> Arc<ImportProgress> {
        self.inner.progress.clone()
    }
}

/// Clears the "import running" flag when the job ends, however it ends.
pub(crate) struct Importing(pub(crate) Arc<Inner>);

impl Drop for Importing {
    fn drop(&mut self) {
        self.0.importing.store(false, Ordering::SeqCst);
    }
}

impl DuckStore {
    pub(crate) fn idle(&self) -> StoreResult<()> {
        if self.inner.importing.load(Ordering::SeqCst) {
            return Err(StoreError::Busy("an import is running; wait for it or cancel it".into()));
        }
        Ok(())
    }

    fn changed(&self) {
        self.inner.generation.fetch_add(1, Ordering::SeqCst);
        // Not fatal: a stale data version only means cached analyses are recomputed late.
        let bumped = self.inner.conn().and_then(|c| self.inner.bump_version(&c));
        if let Err(e) = bumped {
            log::warn!("the data version could not be bumped: {e}");
        }
    }
}

/// How long live tail waits for a running import before it gives up for this change.
const TAIL_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

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
        self.reload_job()
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
        self.inner.forget_events()?;
        let mut s = write_lock(&self.inner.state);
        s.roots.clear();
        s.removed.clear();
        Ok(())
    }

    /// Forgets everything and imports the state's roots (files it removed stay out). Clock offsets are kept
    /// in the project, not applied to the events yet.
    fn restore(&self, state: &WorkspaceState) -> StoreResult<ImportSession> {
        self.clear()?;
        write_lock(&self.inner.state).removed = state.removed.iter().map(PathBuf::from).collect();
        let roots: Vec<PathBuf> = state.roots.iter().map(PathBuf::from).collect();
        self.open(&roots)
    }

    /// Live tail: the changed files are read again (new segments; the full-text index follows) and new files
    /// under the opened folders read; returns how many events were added. Waits for a running import first
    /// (at most [`TAIL_WAIT`]).
    fn append_tail(&self, changed: &[PathBuf]) -> StoreResult<usize> {
        let started = std::time::Instant::now();
        while self.inner.is_importing() {
            if started.elapsed() > TAIL_WAIT {
                return Err(StoreError::Busy(
                    "an import is running; live tail reads the changes afterwards".into(),
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let count = |conn: &Connection| -> StoreResult<u64> {
            conn.query_row("SELECT count(*) FROM events", [], |r| r.get::<_, u64>(0))
                .map_err(engine)
        };
        let before = count(&*self.inner.conn()?)?;
        let grown = self.inner.grown_sources(changed)?;
        self.refresh(&grown)?.wait()?;
        let after = count(&*self.inner.conn()?)?;
        Ok(usize::try_from(after.saturating_sub(before)).unwrap_or(usize::MAX))
    }
}

impl Inner {
    /// The files of sources at or under `paths` (a watcher may name a folder) whose size differs from what was
    /// imported (appended, truncated, rewritten) or that are gone.
    fn grown_sources(&self, paths: &[PathBuf]) -> StoreResult<Vec<PathBuf>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT path, bytes FROM sources WHERE ready").map_err(engine)?;
        let sources = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)))
            .map_err(engine)?
            .collect::<duckdb::Result<Vec<_>>>()
            .map_err(engine)?;
        let sep = imt_core::archive::ENTRY_SEPARATOR;
        let mut out = Vec::new();
        for p in paths {
            let canonical = std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
            let under = canonical.to_string_lossy().into_owned();
            for (path, bytes) in &sources {
                let file = path.split(sep).next().unwrap_or(path);
                let inside = file == under || Path::new(file).starts_with(&canonical);
                if !inside {
                    continue;
                }
                let changed = match std::fs::metadata(file) {
                    Ok(m) => m.len() != *bytes,
                    Err(_) => true,
                };
                if changed && !out.iter().any(|o: &PathBuf| o.as_path() == Path::new(file)) {
                    out.push(PathBuf::from(file));
                }
            }
        }
        Ok(out)
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
    /// Words of `text` ([`imt_core::words`]: all required, the last a prefix, case-insensitive, split on
    /// punctuation and inside `camelCase`); the first `limit` hits (default 1,000, at most 10,000) in time
    /// order. A quoted phrase must also appear as written (ignoring case) in the message, exception or
    /// template; up to 1,000 candidates are checked, beyond that the total is not exact. Only indexed
    /// segments are searched ([`Search::index_status`] tells how far indexing is).
    fn search(&self, req: &SearchRequest) -> StoreResult<SearchReply> {
        let limit = if req.limit == 0 {
            fts::DEFAULT_HITS
        } else {
            req.limit.min(fts::MAX_HITS)
        };
        let index = &self.fts()?.shared.fts;
        let words = imt_core::words::WordQuery::parse(&req.text);
        let phrases = &words.phrases;
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

    /// Counts the events matching `filter` with its text as a substring but not as words: one SQL count when
    /// SQL expresses the filter, else a scan of at most the analysis limit of events (then `complete` is
    /// false). Interrupted by a cancelled [`imt_core::ql::QueryControl`].
    fn partial_matches(&self, filter: &imt_core::ports::Filter) -> StoreResult<imt_core::ports::PartialMatches> {
        use imt_core::query::TextMode;
        if filter.text.trim().is_empty() || filter.regex || filter.case_sensitive {
            return Ok(imt_core::ports::PartialMatches { added: 0, complete: true });
        }
        let with = |mode| imt_core::ports::Filter {
            text_mode: mode,
            ..filter.clone()
        };
        let (substring, words) = (with(TextMode::Substring), with(TextMode::Words));
        let inner = &self.inner;
        let conn = inner.conn()?;
        let ctx = inner.ctx(&conn)?;
        let (fs, _) = inner.compile(&ctx, &substring)?;
        let (fw, cw) = inner.compile(&ctx, &words)?;
        if fs.exact && fw.exact {
            let sql = format!(
                "SELECT count(*) FROM {} WHERE {} AND NOT coalesce({}, FALSE)",
                sql::FROM,
                fs.all_sql(),
                fw.content_sql()
            );
            let cancelled = || imt_core::ql::QueryControl::current().is_some_and(|c| c.is_cancelled());
            let added = match read::count(&conn, &sql) {
                Err(_) if cancelled() => return Err(StoreError::Cancelled),
                other => other?,
            };
            return Ok(imt_core::ports::PartialMatches {
                added: added as u64,
                complete: true,
            });
        }
        drop(conn);
        let mut added = 0u64;
        let coverage = inner.stream_rows(&substring, analysis::MAX_ANALYSED, &mut |r| {
            if !cw.content(&r.rec, &r.source) {
                added += 1;
            }
        })?;
        Ok(imt_core::ports::PartialMatches {
            added,
            complete: coverage.complete(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_failures_become_typed_errors() {
        let locked = classify("IO Error: Could not set lock on file \"x/catalog.duckdb\": Conflicting lock is held".into());
        assert!(
            matches!(&locked, StoreError::Locked(m) if m.contains("another IMT window")),
            "{locked:?}"
        );
        assert!(locked.is_retryable());
        let windows = classify(
            "IO Error: Cannot open file \"data.duckdb\": The process cannot access the file because it is being used by another \
             process.\r\n\nFile is already open in \nimt-store-worker.exe (PID 1)"
                .into(),
        );
        assert!(matches!(windows, StoreError::Locked(_)), "{windows:?}");
        for damaged in [
            "Invalid Input Error: No magic bytes found at end of file 'seg.parquet'",
            "Invalid Input Error: File 'seg.parquet' too small to be a Parquet file",
            "IO Error: Parquet file is corrupt",
        ] {
            let e = classify(damaged.into());
            assert!(matches!(&e, StoreError::Damaged(m) if m.contains("damaged")), "{e:?}");
            assert!(!e.is_retryable());
        }
        assert!(matches!(classify("Out of Memory Error".into()), StoreError::Engine(_)));
        assert_eq!(
            MissingSegment("seg-1.parquet".into()).to_string(),
            "the data file seg-1.parquet of this project is missing"
        );
    }

    #[test]
    fn the_database_gives_freed_memory_back_and_keeps_its_limit() {
        let dir = std::env::temp_dir().join(format!("imt-open-db-{}", std::process::id()));
        let conn = open_db(&dir, &StoreOptions::default()).unwrap();
        let setting = |name: &str| -> String {
            conn.query_row(&format!("SELECT CAST(current_setting('{name}') AS VARCHAR)"), [], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(setting("allocator_flush_threshold"), "32.0 MiB");
        assert_eq!(setting("allocator_background_threads"), "true");
        assert_eq!(setting("threads"), DUCK_THREADS.to_string());
        assert_eq!(setting("temp_directory"), dir.join("tmp").to_string_lossy());
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_database_takes_the_chosen_spill_folder_and_allocator_settings() {
        let root = std::env::temp_dir().join(format!("imt-open-db-opts-{}", std::process::id()));
        let (dir, spill) = (root.join("store"), root.join("spill"));
        std::fs::create_dir_all(&dir).unwrap();
        let opts = StoreOptions {
            duck_threads: 3,
            memory_limit: "100MB".into(),
            spill_dir: Some(spill.clone()),
            max_spill: Some("2GB".into()),
            allocator_flush: "8MiB".into(),
            allocator_background_threads: false,
            ..StoreOptions::default()
        };
        let conn = open_db(&dir, &opts).unwrap();
        let setting = |name: &str| -> String {
            conn.query_row(&format!("SELECT CAST(current_setting('{name}') AS VARCHAR)"), [], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(setting("threads"), "3");
        assert_eq!(setting("allocator_flush_threshold"), "8.0 MiB");
        assert_eq!(setting("allocator_background_threads"), "false");
        assert!(
            setting("max_temp_directory_size").starts_with("1.8 GiB"),
            "{}",
            setting("max_temp_directory_size")
        );
        let own = spill_folder(&spill, &dir);
        assert_eq!(setting("temp_directory"), own.to_string_lossy());
        assert!(own.is_dir() && own.starts_with(&spill));
        assert_ne!(own, spill_folder(&spill, &root.join("other")), "a folder per store");
        assert_eq!(own, spill_folder(&spill, &dir), "stable");
        drop(conn);
        let _ = std::fs::remove_dir_all(&root);
    }
}
