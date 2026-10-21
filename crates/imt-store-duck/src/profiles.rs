//! The `Profiles` port: profile changes classified by the work they need (engine review finding 4),
//! reloads, and previews.
//!
//! - A presentation change swaps the profile; nothing stored changes.
//! - A derivation or parsing change re-reads the sources from their files, as a job, when every file is
//!   still there and unchanged (its size matches what was imported); otherwise it is refused with a
//!   message naming the files. Re-deriving from the stored `raw` text without the files needs the
//!   segment writer of R2b (events are read-only Parquet segments there); see ADR 0014 notes (R2c).

use crate::{engine, read_lock, write_lock, DuckStore, Importing, Inner};
use imt_core::api::{ExtractPreview, ExtractPreviewRequest, FileKind, ParsePreview, ParsePreviewRequest};
use imt_core::ports::{ImportReport, ImportSession, Profiles, StoreError, StoreResult};
use imt_core::profile::{CompiledProfile, Profile};
use imt_core::profile_change::{classify, ChangeClass};
use imt_core::workspace::{preview_parse, preview_profile, ExtractPreviewer};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// Events an extractor preview looks at.
const MAX_PREVIEWED: usize = 200_000;

impl Inner {
    /// The sources whose files are missing or changed since they were imported.
    fn changed_files(&self) -> StoreResult<Vec<String>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare("SELECT path, file_kind, bytes FROM sources WHERE ready")
            .map_err(engine)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, u64>(2)?))
            })
            .map_err(engine)?;
        let mut out = Vec::new();
        for row in rows {
            let (path, kind, bytes) = row.map_err(engine)?;
            let file = path.split(imt_core::archive::ENTRY_SEPARATOR).next().unwrap_or(&path).to_string();
            let ok = match std::fs::metadata(&file) {
                // A plain file is unchanged when its size is (compressed files and archive members keep the
                // decompressed size, so only their presence is checked).
                Ok(m) if kind.as_deref() == Some(format!("{:?}", FileKind::Text).as_str()) => m.len() == bytes,
                Ok(_) => true,
                Err(_) => false,
            };
            if !ok {
                out.push(path);
            }
        }
        out.sort();
        out.dedup();
        Ok(out)
    }

    /// Forgets every event and source (keeping the opened roots), then imports the roots again.
    fn reimport(&self, progress: &imt_core::workspace::ImportProgress) -> StoreResult<ImportReport> {
        let roots = read_lock(&self.state).roots.clone();
        self.forget_events()?;
        self.import(&roots, progress)
    }

    fn swap_profile(&self, compiled: CompiledProfile) {
        write_lock(&self.state).profile = Arc::new(compiled);
        // The field list and instance key depend on the profile.
        *crate::lock(&self.derived) = None;
    }
}

impl DuckStore {
    fn begin_job(&self) -> StoreResult<Importing> {
        if self.inner.importing.swap(true, Ordering::SeqCst) {
            return Err(StoreError::Busy("an import is running; wait for it or cancel it".into()));
        }
        Ok(Importing(self.inner.clone()))
    }

    /// Re-imports every opened root (the files are read again).
    pub(crate) fn reload_job(&self) -> StoreResult<ImportSession> {
        let guard = self.begin_job()?;
        let inner = self.inner.clone();
        ImportSession::spawn(self.inner.progress.clone(), move |progress| {
            let _guard = guard;
            inner.reimport(progress)
        })
    }
}

impl Profiles for DuckStore {
    fn active_profile(&self) -> StoreResult<Profile> {
        Ok(self.inner.profile().profile.clone())
    }

    fn validate_profile(&self, profile: &Profile) -> StoreResult<()> {
        self.inner
            .registry()
            .compile_profile(profile.clone())
            .map(|_| ())
            .map_err(crate::invalid)
    }

    fn set_profile(&self, profile: Profile) -> StoreResult<ImportSession> {
        let compiled = self.inner.registry().compile_profile(profile).map_err(crate::invalid)?;
        let change = classify(&self.inner.profile().profile, &compiled.profile);
        if change.class == ChangeClass::Presentation {
            self.inner.swap_profile(compiled);
            return ImportSession::spawn(self.inner.progress.clone(), |_| Ok(ImportReport::default()));
        }
        if !imt_core::ingest::schema_for(&compiled).same_columns(&self.inner.schema) {
            return Err(StoreError::Invalid(
                "this change declares other typed fields than the project was imported with; open the files in a new project to use it"
                    .into(),
            ));
        }
        let missing = self.inner.changed_files()?;
        if !missing.is_empty() {
            let shown: Vec<&str> = missing.iter().take(5).map(String::as_str).collect();
            return Err(StoreError::Invalid(format!(
                "this change ({}) needs the original files again, but {} {} missing or changed since the import: {}{}",
                change.sections.join(", "),
                missing.len(),
                if missing.len() == 1 { "is" } else { "are" },
                shown.join(", "),
                if missing.len() > shown.len() { ", ..." } else { "" }
            )));
        }
        // Only the sources read with another parser are read again (ADR 0026: per parser; today every source
        // shares the profile, so all of them).
        let stale: Vec<PathBuf> = self
            .inner
            .sources_not_read_with(&imt_core::ports::ParserRef::of_profile(&compiled.profile))?
            .into_iter()
            .map(|(_, path)| PathBuf::from(path.split(imt_core::archive::ENTRY_SEPARATOR).next().unwrap_or(&path)))
            .collect();
        self.inner.swap_profile(compiled);
        if stale.is_empty() {
            return ImportSession::spawn(self.inner.progress.clone(), |_| Ok(ImportReport::default()));
        }
        let all = {
            let conn = self.inner.conn()?;
            let n: u64 = conn
                .query_row("SELECT count(*) FROM sources WHERE ready", [], |r| r.get(0))
                .map_err(engine)?;
            n as usize <= stale.len()
        };
        if all {
            let guard = self.begin_job()?;
            let inner = self.inner.clone();
            return ImportSession::spawn(self.inner.progress.clone(), move |progress| {
                let _guard = guard;
                inner.reimport(progress)
            });
        }
        imt_core::ports::Ingest::refresh(self, &stale)
    }

    fn preview_parse(&self, req: &ParsePreviewRequest) -> StoreResult<ParsePreview> {
        Ok(preview_parse(self.inner.registry(), req))
    }

    fn parsers(&self) -> StoreResult<Vec<imt_core::ports::ParserUse>> {
        self.inner.parser_uses()
    }

    fn classify_parser(&self, parser: &imt_core::ports::ParserDef) -> StoreResult<imt_core::ports::ParserChange> {
        self.classify_parser_change(parser)
    }

    fn assign_parser(&self, _source: imt_core::batch::SourceId, _parser: &imt_core::ports::ParserDef) -> StoreResult<ImportSession> {
        Err(StoreError::unsupported("assigning a parser to a source", "ADR 0026"))
    }

    fn preview_spans(&self, req: &imt_core::spans::SpanPreviewRequest) -> StoreResult<imt_core::spans::SpanPreview> {
        self.inner.preview_spans(req)
    }

    fn propose_span_rule(&self, req: &imt_core::spans::infer::SpanRuleRequest) -> StoreResult<imt_core::spans::infer::SpanProposal> {
        self.inner.propose_span_rule(req)
    }

    fn evaluate_span_rule(
        &self,
        req: &imt_core::spans::infer::SpanRuleCheck,
    ) -> StoreResult<imt_core::ports::Analyzed<imt_core::spans::infer::SpanEvaluation>> {
        self.inner.evaluate_span_rule(req)
    }

    fn preview_extractor(&self, req: &ExtractPreviewRequest) -> StoreResult<ExtractPreview> {
        let (compiled, i) = match preview_profile(self.inner.registry(), &req.toml, &req.extractor) {
            Ok(p) => p,
            Err(e) => {
                return Ok(ExtractPreview {
                    error: Some(format!("{e:#}")),
                    ..ExtractPreview::default()
                })
            }
        };
        let mut previewer = ExtractPreviewer::new(&compiled.extractors[i], req.limit);
        self.inner
            .stream_rows(&req.query, MAX_PREVIEWED, &mut |r| previewer.add(&r.rec, &r.source))?;
        Ok(previewer.finish())
    }
}
