//! Projects (ADR 0014 §6): a store lives in a project folder `<name>.imt/` next to its
//! `project.toml`; it opens without the original log files, and exports as an `.imtz` without caches.

use crate::{engine, read_lock, write_lock, DuckStore};
use imt_core::package::{open_package, Package, ProjectDir, ProjectManifest};
use imt_core::ports::{StoreError, StoreResult};
use imt_core::{PluginRegistry, Profile};
use std::path::{Path, PathBuf};

fn io(e: anyhow::Error) -> StoreError {
    StoreError::Io(format!("{e:#}"))
}

impl DuckStore {
    /// A new, empty project in the folder `dir` (a store there is replaced).
    pub fn create_project(dir: &Path, name: &str, profile: Profile, registry: PluginRegistry) -> StoreResult<DuckStore> {
        ProjectDir::create(dir, name).map_err(io)?;
        DuckStore::create(dir, profile, registry)
    }

    /// Opens a project folder or an exported `.imtz` (extracted under `dest_root`), with `profile`
    /// and the project's profile overrides merged over it. Share and collector bundles are refused with a
    /// message saying what they are.
    pub fn open_project(
        path: &Path,
        dest_root: &Path,
        profile: Profile,
        registry: PluginRegistry,
    ) -> StoreResult<(DuckStore, ProjectManifest)> {
        let (dir, manifest) = match open_package(path, dest_root).map_err(|e| StoreError::Invalid(format!("{e:#}")))? {
            Package::Project { dir, manifest } => (dir, manifest),
            Package::Share(_) => {
                return Err(StoreError::Invalid(format!(
                    "{} is a share bundle, not a project; open it as a bundle",
                    path.display()
                )))
            }
            Package::Collector(_) => {
                return Err(StoreError::Invalid(format!(
                    "{} is a collector bundle, not a project; open its logs as files",
                    path.display()
                )))
            }
        };
        let profile = if manifest.profile_overrides.trim().is_empty() {
            profile
        } else {
            profile
                .with_override(&manifest.profile_overrides)
                .map_err(|e| StoreError::Invalid(format!("the project's profile overrides are not valid: {e}")))?
        };
        let store = DuckStore::open(&dir.dir, profile, registry)?;
        write_lock(&store.inner.state).roots = manifest.roots.iter().map(PathBuf::from).collect();
        Ok((store, manifest))
    }

    /// The project folder of this store.
    pub fn project_dir(&self) -> ProjectDir {
        ProjectDir {
            dir: self.inner.dir.clone(),
        }
    }

    /// Writes `project.toml` with `manifest` and the store's opened roots.
    pub fn save_project(&self, manifest: &mut ProjectManifest) -> StoreResult<()> {
        manifest.parsers = self.parser_refs()?;
        // The store holds the offsets (already applied to its events); the manifest repeats them.
        manifest.clock_offsets = {
            let conn = self.inner.conn()?;
            let mut rows: Vec<_> = crate::writer::clock_offsets(&conn)
                .map_err(|e| engine(format!("{e:#}")))?
                .into_iter()
                .collect();
            rows.sort();
            rows.into_iter()
                .map(|(path, offset_ms)| imt_core::package::SourceOffset { path, offset_ms })
                .collect()
        };
        manifest.roots = read_lock(&self.inner.state)
            .roots
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        self.project_dir().save(manifest).map_err(io)
    }

    /// Saves the project and writes it as an `.imtz` to `out` (no caches); returns the bytes written.
    pub fn export_project(&self, manifest: &mut ProjectManifest, out: &Path) -> StoreResult<u64> {
        self.idle()?;
        // Everything in the database file itself, so the archive needs no write-ahead log. A background writer (the
        // index, positions) may hold a write transaction for a moment, which DuckDB reports instead of waiting: ask
        // again for a few seconds. The archive holds a copy made by `COPY FROM DATABASE` below either way, so a
        // checkpoint that never gets through costs only a larger write-ahead log, not a wrong export.
        for attempt in 0..100 {
            match self.inner.conn()?.execute_batch("CHECKPOINT") {
                Ok(()) => break,
                Err(e) if attempt < 99 && e.to_string().contains("other write transactions") => {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(e) => {
                    log::warn!("checkpoint before the export failed: {e}");
                    break;
                }
            }
        }
        self.save_project(manifest)?;
        // The open database cannot be read by another handle (Windows): export a copy of it.
        let tmp = self.inner.dir.join("tmp").join(format!("export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp)?;
        let copy = tmp.join(crate::DB_FILE);
        let result = (|| {
            let conn = self.inner.conn()?;
            let current: String = conn.query_row("SELECT current_database()", [], |r| r.get(0)).map_err(engine)?;
            let target = crate::sql::lit(&copy.to_string_lossy())
                .ok_or_else(|| StoreError::Io("the project folder's path cannot be used".into()))?;
            conn.execute_batch(&format!(
                "ATTACH {target} AS imt_export; COPY FROM DATABASE {} TO imt_export; DETACH imt_export;",
                crate::sql::ident(&current)
            ))
            .map_err(engine)?;
            self.project_dir()
                .export_with(out, &[(PathBuf::from(crate::DB_FILE), copy.clone())])
                .map_err(io)
        })();
        let _ = std::fs::remove_dir_all(&tmp);
        result
    }
}

impl imt_core::ports::Projects for DuckStore {
    fn store_dir(&self) -> PathBuf {
        self.inner.dir.clone()
    }

    fn save_project(&self, manifest: &ProjectManifest) -> StoreResult<ProjectManifest> {
        let mut m = manifest.clone();
        DuckStore::save_project(self, &mut m)?;
        Ok(m)
    }

    fn export_project(&self, manifest: &ProjectManifest, out: &Path) -> StoreResult<u64> {
        let mut m = manifest.clone();
        DuckStore::export_project(self, &mut m, out)
    }
}
