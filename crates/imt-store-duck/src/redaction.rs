//! Redaction on the store: what it would change in the matching events (the engine's summary
//! ([`imt_core::redact::view::summary`]) over the store's own scan, with the active profile's `[redact]` rules), and the
//! redacted copy of a project ([`imt_core::redact::copy`], ADR 0041).
//!
//! The copy reads the `raw` text of every copyable source, redacts it, writes it as a file in a work folder under this
//! store's `tmp`, imports those files into a new store with the same profile and parsers, and exports that store. Nothing
//! the original held reaches the new store except through the redactor.

use crate::{engine, DuckStore};
use duckdb::params;
use imt_core::api::FileKind;
use imt_core::ports::{Ingest, Profiles, Projects, Redaction, StoreError, StoreResult};
use imt_core::redact::copy::{
    copy_path, excluded_reason_of, without_redact_section, ExcludedSource, RedactedExport, RedactedExportRequest, UI_FILE,
};
use imt_core::redact::{hits_of, Counts, RedactionSummary, RedactionSummaryRequest, Redactor, Salt};
use imt_core::PluginRegistry;
use std::collections::HashSet;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_WORK: AtomicU64 = AtomicU64::new(1);

fn io(e: impl std::fmt::Display) -> StoreError {
    StoreError::Io(e.to_string())
}

fn kind_of(text: &str) -> FileKind {
    match text {
        "Binary" => FileKind::Binary,
        "Compressed" => FileKind::Compressed,
        "ArchiveMember" => FileKind::ArchiveMember,
        _ => FileKind::Text,
    }
}

impl Redaction for DuckStore {
    fn redaction_summary(&self, req: &RedactionSummaryRequest) -> StoreResult<RedactionSummary> {
        let cfg = self.inner.profile().profile.redact.clone();
        imt_core::redact::view::summary(self, &cfg, req)
    }

    fn export_redacted_project(&self, req: &RedactedExportRequest) -> StoreResult<RedactedExport> {
        let redactor = Redactor::from_config(&req.cfg, Salt::random()).map_err(|e| StoreError::Invalid(format!("{e:#}")))?;
        if redactor.is_empty() {
            return Err(StoreError::Invalid(imt_core::redact::view::NO_RULES.into()));
        }
        self.idle()?;
        let work = self.inner.dir.join("tmp").join(format!(
            "redacted-{}-{}",
            std::process::id(),
            NEXT_WORK.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&work);
        std::fs::create_dir_all(work.join("files")).map_err(io)?;
        let result = self.write_copy(req, &redactor, &work);
        // Whatever happened, the redacted text must not outlive the export.
        let _ = std::fs::remove_dir_all(&work);
        result
    }
}

impl DuckStore {
    fn write_copy(&self, req: &RedactedExportRequest, redactor: &Redactor, work: &Path) -> StoreResult<RedactedExport> {
        let mut counts = Counts::new();
        let mut report = RedactedExport::default();
        let mut taken = HashSet::new();
        let mut offsets = imt_core::skew::ClockOffsets::default();
        let files = std::fs::canonicalize(work.join("files")).map_err(io)?;
        {
            let conn = self.inner.conn()?;
            let mut sources = conn
                .prepare("SELECT id, name, path, importer, file_kind FROM sources WHERE ready ORDER BY id")
                .map_err(engine)?;
            let rows = sources
                .query_map([], |r| {
                    Ok((
                        r.get::<_, u32>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                        r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                    ))
                })
                .map_err(engine)?
                .collect::<duckdb::Result<Vec<_>>>()
                .map_err(engine)?;
            let skew = crate::writer::clock_offsets(&conn).map_err(|e| engine(format!("{e:#}")))?;
            for (id, name, path, importer, kind) in rows {
                // The shown name is redacted too: folder and host names can be personal.
                let shown = redactor.apply(&name, &mut counts).into_owned();
                if let Some(reason) = excluded_reason_of(&importer, kind_of(&kind)) {
                    report.excluded.push(ExcludedSource { name: shown, reason });
                    continue;
                }
                let rel = copy_path(&shown, &mut taken);
                let file = files.join(&rel);
                if let Some(parent) = file.parent() {
                    std::fs::create_dir_all(parent).map_err(io)?;
                }
                let mut out = std::io::BufWriter::new(std::fs::File::create(&file).map_err(io)?);
                let mut stmt = conn
                    .prepare("SELECT raw FROM events WHERE source = ? ORDER BY line, id")
                    .map_err(engine)?;
                let raws = stmt.query_map(params![id], |r| r.get::<_, String>(0)).map_err(engine)?;
                let mut events = 0usize;
                for raw in raws {
                    let raw = raw.map_err(engine)?;
                    out.write_all(redactor.apply(&raw, &mut counts).as_bytes()).map_err(io)?;
                    out.write_all(b"\n").map_err(io)?;
                    events += 1;
                }
                out.flush().map_err(io)?;
                report.events += events;
                report.sources += 1;
                if let Some(ms) = skew.get(&path) {
                    let _ = offsets.set(&file.to_string_lossy(), *ms);
                }
            }
        }
        if report.sources == 0 {
            return Err(StoreError::Invalid(
                "nothing can be copied: every source is in a format that cannot be redacted (see the list of left-out sources)".into(),
            ));
        }
        report.hits = hits_of(redactor, &counts);

        // The new store: the same profile (without the redaction rules) and parsers, reading only redacted text.
        let mut profile = self.inner.profile().profile.clone();
        profile.redact = Default::default();
        let dir = work.join("store");
        let copy = DuckStore::create(&dir, profile, PluginRegistry::new())?;
        let result = (|| -> StoreResult<()> {
            copy.set_parsers(self.inner.parser_set().setup())?;
            imt_core::ports::Skew::set_clock_offsets(&copy, &offsets)?;
            copy.open(std::slice::from_ref(&files))?.wait()?;
            if let Some(ui) = &req.ui_json {
                std::fs::write(copy.store_dir().join(UI_FILE), ui).map_err(io)?;
            }
            // Sources remember where they were read: the work folder lies inside the user's data folder, whose path
            // can name them. The copy says `redacted/<name>` instead.
            {
                let conn = copy.inner.conn()?;
                let prefix = format!("{}/", files.to_string_lossy().trim_end_matches('/'));
                let cut = prefix.chars().count() as i64 + 1;
                for table in ["sources", "source_encodings", "clock_offsets"] {
                    conn.execute(
                        &format!("UPDATE {table} SET path = 'redacted/' || substr(path, ?) WHERE path LIKE ? ESCAPE '\\'"),
                        params![
                            cut,
                            format!("{}%", prefix.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"))
                        ],
                    )
                    .map_err(engine)?;
                }
            }
            // The events view names the segment files by absolute path (inside the user's data folder); it is
            // recreated whenever a store opens, so the copy is written with the empty one.
            copy.inner
                .conn()?
                .execute_batch(&crate::schema::events_view(&copy.inner.schema, &[]))
                .map_err(engine)?;
            // The work folder is not a place the copy's readers have: no roots.
            crate::write_lock(&copy.inner.state).roots.clear();
            let mut manifest = req.manifest.clone();
            manifest.profile_overrides = without_redact_section(&manifest.profile_overrides);
            manifest.roots.clear();
            manifest.clock_offsets.clear();
            report.bytes = copy.export_project(&mut manifest, &req.out)?;
            Ok(())
        })();
        imt_core::ports::Projects::close(&copy);
        result?;
        Ok(report)
    }
}
