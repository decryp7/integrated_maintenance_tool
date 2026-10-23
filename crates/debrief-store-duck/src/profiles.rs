//! The `Profiles` port: profile changes classified by the work they need (engine review finding 4),
//! reloads, and previews.
//!
//! - A presentation change swaps the profile; nothing stored changes.
//! - A derivation or parsing change re-reads the sources from their files, as a job, when every file is
//!   still there and unchanged (its size matches what was imported); otherwise it is refused with a
//!   message naming the files. Re-deriving from the stored `raw` text without the files needs the
//!   segment writer of R2b (events are read-only Parquet segments there); see ADR 0014 notes (R2c).

use crate::{engine, read_lock, write_lock, DuckStore, Importing, Inner};
use debrief_core::api::{ExtractPreview, ExtractPreviewRequest, FileKind, ParsePreview, ParsePreviewRequest};
use debrief_core::parser::ParserSet;
use debrief_core::ports::{ImportReport, ImportSession, Profiles, StoreError, StoreResult};
use debrief_core::profile::Profile;
use debrief_core::profile_change::{classify, ChangeClass};
use debrief_core::workspace::{preview_parse, preview_profile, ExtractPreviewer};
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
            let file = path
                .split(debrief_core::archive::ENTRY_SEPARATOR)
                .next()
                .unwrap_or(&path)
                .to_string();
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

    /// Imports the opened roots again, replacing what is stored only when the new data is complete: the old
    /// catalog rows and segment files are kept aside until the import ends, so a cancel (or a failure) puts the
    /// previous store back untouched.
    fn reimport(&self, progress: &debrief_core::workspace::ImportProgress) -> StoreResult<ImportReport> {
        let roots = read_lock(&self.state).roots.clone();
        progress.set_phase(Some("Keeping the current data safe"));
        let kept = self.set_aside_events()?;
        let result = self.import(&roots, progress);
        let cancelled = progress.is_cancelled_flag();
        match result {
            Ok(report) if !cancelled => {
                self.discard_aside(kept);
                Ok(report)
            }
            other => {
                progress.set_phase(Some("Restoring the previous data"));
                self.restore_aside(kept)?;
                other.map(|_| ImportReport::default())
            }
        }
    }

    /// Makes `parsers` (the library compiled with the new profile) current.
    fn swap_profile(&self, parsers: ParserSet) {
        let mut s = write_lock(&self.state);
        s.profile = parsers.base().clone();
        s.parsers = Arc::new(parsers);
        drop(s);
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
        let setup = self.inner.parser_set().setup().clone();
        let parsers = self.inner.registry().compile_parsers(&profile, &setup).map_err(crate::invalid)?;
        let compiled = parsers.base().clone();
        let change = classify(&self.inner.profile().profile, &compiled.profile);
        if change.class == ChangeClass::Presentation {
            self.inner.swap_profile(parsers);
            return ImportSession::spawn(self.inner.progress.clone(), |_| Ok(ImportReport::default()));
        }
        if !debrief_core::ingest::schema_for(&compiled).same_columns(&self.inner.schema) {
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
        // The profile's extractors, pipeline and markers apply to every source whatever its parser, so a
        // derivation or parsing change reads every source again (a parser change reads only its sources).
        self.inner.swap_profile(parsers);
        let guard = self.begin_job()?;
        let inner = self.inner.clone();
        ImportSession::spawn(self.inner.progress.clone(), move |progress| {
            let _guard = guard;
            inner.reimport(progress)
        })
    }

    fn preview_parse(&self, req: &ParsePreviewRequest) -> StoreResult<ParsePreview> {
        Ok(preview_parse(self.inner.registry(), &self.inner.parser_set(), req))
    }

    fn parsers(&self) -> StoreResult<Vec<debrief_core::ports::ParserUse>> {
        self.inner.parser_uses()
    }

    fn set_parsers(&self, setup: &debrief_core::parser::ParserSetup) -> StoreResult<Vec<debrief_core::batch::SourceId>> {
        self.replace_parsers(setup)
    }

    fn reread_parsers(&self) -> StoreResult<ImportSession> {
        DuckStore::reread_parsers(self)
    }

    fn classify_parser(&self, parser: &debrief_core::ports::ParserDef) -> StoreResult<debrief_core::ports::ParserChange> {
        self.classify_parser_change(parser)
    }

    fn assign_parser(&self, source: debrief_core::batch::SourceId, parser: Option<&str>) -> StoreResult<ImportSession> {
        self.assign(source, parser)
    }

    fn preview_spans(&self, req: &debrief_core::spans::SpanPreviewRequest) -> StoreResult<debrief_core::spans::SpanPreview> {
        self.inner.preview_spans(req)
    }

    fn propose_span_rule(
        &self,
        req: &debrief_core::spans::infer::SpanRuleRequest,
    ) -> StoreResult<debrief_core::spans::infer::SpanProposal> {
        self.inner.propose_span_rule(req)
    }

    fn evaluate_span_rule(
        &self,
        req: &debrief_core::spans::infer::SpanRuleCheck,
    ) -> StoreResult<debrief_core::ports::Analyzed<debrief_core::spans::infer::SpanEvaluation>> {
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
