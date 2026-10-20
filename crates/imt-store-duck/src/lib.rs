//! IMT's DuckDB store (ADR 0014): one database per project, events appended as Arrow batches by a
//! single writer, queries compiled to SQL with a batch-scan fallback for what SQL cannot express yet.
//!
//! Phase R2a: in-process, sequential import; `Query` serves summary, page, timeline, facet and scan.
//! The other operations answer [`StoreError::Unsupported`] naming the phase that adds them.

mod analysis;
mod context;
mod diag;
mod profiles;
mod project;
mod qlang;
mod read;
mod schema;
mod sql;
mod writer;

pub use schema::FORMAT_VERSION;

use duckdb::{params, Config, Connection};
use imt_core::api::{UnrecognizedFile, WorkspaceState};
use imt_core::batch::{EventSchema, SourceId};
use imt_core::ingest::{self, ImportEnv, Outcome};
use imt_core::ports::{ImportReport, ImportSession, IndexStatus, Ingest, Search, SearchReply, SearchRequest, StoreError, StoreResult};
use imt_core::profile::{CompiledProfile, Profile};
use imt_core::workspace::{ImportProgress, NOT_IMPORTED};
use imt_core::PluginRegistry;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// DuckDB's memory limit (ADR 0014 memory budget); it spills to the store's `tmp` folder beyond it.
pub const MEMORY_LIMIT: &str = "256MB";

/// File name of the database in a store folder.
pub const DB_FILE: &str = "data.duckdb";

/// Idle connections kept for reuse.
const POOL: usize = 8;

pub(crate) fn engine(e: impl std::fmt::Display) -> StoreError {
    StoreError::Engine(e.to_string())
}

/// What the store was opened with and what was opened in it.
pub(crate) struct State {
    pub(crate) profile: Arc<CompiledProfile>,
    pub(crate) roots: Vec<PathBuf>,
    removed: HashSet<PathBuf>,
    unrecognized: Vec<UnrecognizedFile>,
}

pub(crate) struct Inner {
    /// The store's (project's) folder.
    pub(crate) dir: PathBuf,
    /// The connection the database was opened with; every other connection is cloned from it.
    db: Mutex<Connection>,
    pool: Mutex<Vec<Connection>>,
    registry: PluginRegistry,
    pub(crate) schema: Arc<EventSchema>,
    pub(crate) state: RwLock<State>,
    /// Bumped by every change to the events (keys read caches).
    pub(crate) generation: AtomicU64,
    pub(crate) importing: AtomicBool,
    pub(crate) progress: Arc<ImportProgress>,
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

pub(crate) fn read_lock<T>(m: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    m.read().unwrap_or_else(|p| p.into_inner())
}

pub(crate) fn write_lock<T>(m: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    m.write().unwrap_or_else(|p| p.into_inner())
}

impl Inner {
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

    /// Forgets every event, source and problem (the opened roots stay).
    pub(crate) fn forget_events(&self) -> StoreResult<()> {
        let conn = self.conn()?;
        conn.execute_batch("DELETE FROM events; DELETE FROM diagnostics; DELETE FROM sources;")
            .map_err(engine)?;
        write_lock(&self.state).unrecognized.clear();
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.bump_version(&conn)
    }

    pub(crate) fn is_importing(&self) -> bool {
        self.importing.load(Ordering::SeqCst)
    }

    /// Bumps the persisted data version (cached analyses of older versions are dropped).
    fn bump_version(&self, conn: &Connection) -> StoreResult<()> {
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
        Ok(())
    }

    pub(crate) fn unrecognized(&self) -> Vec<UnrecognizedFile> {
        read_lock(&self.state).unrecognized.clone()
    }

    /// Reads `paths` (canonical) into the database.
    pub(crate) fn import(&self, paths: &[PathBuf], progress: &ImportProgress) -> StoreResult<ImportReport> {
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
        let mut writer = writer::DuckWriter::new(&conn, &self.generation).map_err(|e| engine(format!("{e:#}")))?;
        let outcomes = ingest::import_files(&env, &files, &removed, progress, &mut writer);
        writer.finish().map_err(|e| engine(format!("{e:#}")))?;
        self.bump_version(&conn)?;
        // Write the import into the database file now (the WAL may grow to `checkpoint_threshold`
        // while importing), so a reopen does not replay it.
        // Not fatal: DuckDB checkpoints again later.
        if let Err(e) = conn.execute_batch("CHECKPOINT") {
            log::warn!("checkpoint after the import failed: {e}");
        }
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

fn open_db(dir: &Path) -> StoreResult<Connection> {
    let tmp = dir.join("tmp");
    std::fs::create_dir_all(&tmp)?;
    let config = Config::default()
        .max_memory(MEMORY_LIMIT)
        .and_then(|c| c.with("temp_directory", tmp.to_string_lossy()))
        .and_then(|c| c.enable_autoload_extension(false))
        // Fewer checkpoints while importing: each one rewrote the growing table (3x slower imports).
        .and_then(|c| c.with("checkpoint_threshold", "1GB"))
        .map_err(engine)?;
    Connection::open_with_flags(dir.join(DB_FILE), config).map_err(engine)
}

pub(crate) fn invalid(e: anyhow::Error) -> StoreError {
    StoreError::Invalid(format!("{e:#}"))
}

impl DuckStore {
    /// A new, empty store in `dir` (an existing database there is replaced) reading with `profile`
    /// and the plugins in `registry`.
    pub fn create(dir: &Path, profile: Profile, registry: PluginRegistry) -> StoreResult<DuckStore> {
        registry.resolve();
        let compiled = registry.compile_profile(profile).map_err(invalid)?;
        std::fs::create_dir_all(dir)?;
        for f in [DB_FILE.to_string(), format!("{DB_FILE}.wal")] {
            match std::fs::remove_file(dir.join(f)) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                _ => {}
            }
        }
        let conn = open_db(dir)?;
        let schema = ingest::schema_for(&compiled);
        conn.execute_batch(&schema::ddl(&schema)).map_err(engine)?;
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
        Self::with(dir, conn, schema, compiled, registry)
    }

    /// Reopens the store in `dir`. A database written by a newer IMT is refused with a message.
    pub fn open(dir: &Path, profile: Profile, registry: PluginRegistry) -> StoreResult<DuckStore> {
        if !dir.join(DB_FILE).is_file() {
            return Err(StoreError::NotFound(format!("no store in {}", dir.display())));
        }
        registry.resolve();
        let compiled = registry.compile_profile(profile).map_err(invalid)?;
        let conn = open_db(dir)?;
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
        Self::with(dir, conn, schema, compiled, registry)
    }

    fn with(
        dir: &Path,
        conn: Connection,
        schema: EventSchema,
        profile: CompiledProfile,
        registry: PluginRegistry,
    ) -> StoreResult<DuckStore> {
        // Added after format 1 was first written; created on open so older stores gain it.
        conn.execute_batch(schema::CACHE_DDL).map_err(engine)?;
        Ok(DuckStore {
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
                generation: AtomicU64::new(0),
                importing: AtomicBool::new(false),
                progress: Arc::new(ImportProgress::default()),
                derived: Mutex::new(None),
            }),
        })
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

    fn changed(&self) -> StoreResult<()> {
        self.inner.generation.fetch_add(1, Ordering::SeqCst);
        self.inner.bump_version(&*self.inner.conn()?)
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
        self.reload_job()
    }

    fn refresh(&self, _changed: &[PathBuf]) -> StoreResult<ImportSession> {
        Err(StoreError::unsupported("refresh", "R2b"))
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
        for sql in [
            "DELETE FROM events WHERE source = ?",
            "DELETE FROM diagnostics WHERE source = ?",
            "DELETE FROM sources WHERE id = ?",
        ] {
            conn.execute(sql, params![id.0]).map_err(engine)?;
        }
        let mut s = write_lock(&self.inner.state);
        let p = PathBuf::from(&path);
        s.roots.retain(|r| *r != p);
        s.removed.insert(p);
        drop(s);
        self.changed()
    }

    fn clear(&self) -> StoreResult<()> {
        self.idle()?;
        self.inner.forget_events()?;
        let mut s = write_lock(&self.inner.state);
        s.roots.clear();
        s.removed.clear();
        Ok(())
    }

    fn restore(&self, _state: &WorkspaceState) -> StoreResult<ImportSession> {
        Err(StoreError::unsupported("restore", "R2c"))
    }

    fn append_tail(&self, _changed: &[PathBuf]) -> StoreResult<usize> {
        Err(StoreError::unsupported("live tail", "6c"))
    }
}

impl Search for DuckStore {
    fn search(&self, _req: &SearchRequest) -> StoreResult<SearchReply> {
        Err(StoreError::unsupported("full-text search", "R2b"))
    }

    fn index_status(&self) -> StoreResult<IndexStatus> {
        Err(StoreError::unsupported("full-text search", "R2b"))
    }
}
