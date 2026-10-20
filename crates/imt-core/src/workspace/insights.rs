//! Analyses over the records matching a query: the query language, patterns,
//! exceptions, anomalies, window comparison, field statistics, traces and the
//! Moment summary ("what was happening between from and to").

use super::Workspace;
use crate::analysis::{self, Anomaly, AnomalyConfig, CompareConfig, Comparison, FieldStats, RequestSpan, WaterfallRow};
use crate::api::{
    CompareRequest, CompletionRequest, CorrelateRequest, FieldStatsRequest, MomentRequest, MomentSummary, QueryRequest, RequestsRequest,
    TraceRequest, TraceSummary,
};
use crate::exceptions::{self, ExceptionGroup};
use crate::model::Record;
use crate::patterns::Pattern;
use crate::ql::{self, Cancelled, Completion, QlError, QlResult, QueryControl, RunError};
use crate::query::{group_key, Query};
use crate::row::RecordRow;
use anyhow::Result;

/// Most records any single analysis looks at, to keep the UI responsive.
const MAX_ANALYSED: usize = 2_000_000;
/// Most events a correlation lists.
pub const MAX_CORRELATED: usize = 10_000;

impl Workspace {
    fn matched(&self, q: &Query) -> Result<Vec<&Record>> {
        let ids = self.matching(q)?;
        Ok(ids.iter().take(MAX_ANALYSED).map(|&id| &self.records[id as usize]).collect())
    }

    fn keyed<'a>(&'a self, records: &[&'a Record], q: &Query) -> Vec<(&'a Record, String)> {
        let by = if q.group_by.is_empty() {
            &self.default_group_by
        } else {
            &q.group_by
        };
        records.iter().map(|r| (*r, group_key(by, r, self.source_name(r.source)))).collect()
    }

    /// Runs a full query-language program (including `stats`, `sort`, `head`)
    /// over the records matching `query` without its `ql` (which is the program).
    pub fn run_query(&self, req: &QueryRequest) -> Result<QlResult, QlError> {
        self.run_query_with(req, &QueryControl::new()).map_err(RunError::into_ql_error)
    }

    /// Like [`run_query`](Self::run_query), cancellable through `control` (checked between batches of
    /// records while matching and evaluating), which also reports progress. Cancelling leaves the
    /// workspace unchanged: nothing partial is cached.
    pub fn run_query_with(&self, req: &QueryRequest, control: &QueryControl) -> Result<QlResult, RunError> {
        let program = ql::parse(&req.query.ql)?;
        let base = Query {
            ql: String::new(),
            ..req.query.clone()
        };
        let ids = self
            .matching_with(&base, control)
            .map_err(|e| match e.downcast_ref::<Cancelled>() {
                Some(_) => RunError::Cancelled,
                None => RunError::Invalid(QlError::new(format!("{e:#}"), 0, 0)),
            })?;
        let mut it = ids
            .iter()
            .take(MAX_ANALYSED)
            .map(|&id| &self.records[id as usize])
            .map(|r| (r, self.source_name(r.source)));
        ql::run_with(&program, &mut it, control)
    }

    /// Checks a query-language text: syntax, then field names against the loaded data (an unknown
    /// field comes back with the closest known names). `None` when it is fine.
    pub fn check_query(&self, text: &str) -> Option<QlError> {
        match ql::parse(text) {
            Err(e) => Some(e),
            Ok(program) => {
                let fields: Vec<String> = self.fields.iter().map(|f| f.name.clone()).collect();
                ql::unknown_field(&program, &fields)
            }
        }
    }

    pub fn complete_query(&self, req: &CompletionRequest) -> Vec<Completion> {
        let fields: Vec<String> = self.fields.iter().map(|f| f.name.clone()).collect();
        ql::completions(&req.text, req.cursor, &fields)
    }

    /// Message patterns, mined incrementally as live tail appends records.
    pub fn patterns(&self, req: &QueryRequest) -> Result<Vec<Pattern>> {
        self.mined_patterns(&req.query)
    }

    pub fn exception_groups(&self, req: &QueryRequest) -> Result<Vec<ExceptionGroup>> {
        let records = self.matched(&req.query)?;
        let keyed = self.keyed(&records, &req.query);
        Ok(exceptions::group(keyed.iter().map(|(r, k)| (*r, k.as_str()))))
    }

    /// Anomalies over the records matching the query (cached per query).
    pub fn anomalies(&self, req: &QueryRequest) -> Result<Vec<Anomaly>> {
        // A sliding window is keyed by where it currently starts.
        let key = serde_json::to_string(&self.resolve_window(&req.query))?;
        if let Some((k, a)) = self.anomaly_cache.lock().unwrap().as_ref() {
            if *k == key {
                return Ok(a.clone());
            }
        }
        let records = self.matched(&req.query)?;
        let keyed = self.keyed(&records, &req.query);
        let found = analysis::detect(keyed.iter().map(|(r, k)| (*r, k.as_str())), &AnomalyConfig::default());
        *self.anomaly_cache.lock().unwrap() = Some((key, found.clone()));
        Ok(found)
    }

    pub fn compare(&self, req: &CompareRequest) -> Result<Comparison> {
        let a = self.matched(&req.a)?;
        let b = self.matched(&req.b)?;
        Ok(analysis::compare_windows(
            a.iter().copied(),
            b.iter().copied(),
            &CompareConfig::default(),
        ))
    }

    pub fn field_stats(&self, req: &FieldStatsRequest) -> Result<FieldStats> {
        let records = self.matched(&req.query)?;
        Ok(analysis::field_stats(
            records.iter().map(|r| self.row(r)),
            &req.field,
            req.buckets.clamp(1, 200),
        ))
    }

    /// Ids (time order) of every event carrying `field = value`, across all sources.
    pub fn correlate(&self, req: &CorrelateRequest) -> Vec<u32> {
        let mut ids = analysis::correlate(self.records.iter().map(|r| self.row(r)), &req.field, &req.value);
        ids.truncate(MAX_CORRELATED);
        ids
    }

    /// Without a trace id: the traces in the matching records, longest first.
    /// With one: that trace's waterfall.
    pub fn traces(&self, req: &TraceRequest) -> Result<(Vec<TraceSummary>, Vec<WaterfallRow>)> {
        let records = self.matched(&req.query)?;
        let spans = analysis::spans(records.iter().copied());
        if !req.trace_id.is_empty() {
            return Ok((Vec::new(), analysis::waterfall(&spans, &req.trace_id)));
        }
        Ok((analysis::moment::trace_summaries(&spans), Vec::new()))
    }

    /// Requests (by id field) in the matching records, longest first.
    pub fn requests(&self, req: &RequestsRequest) -> Result<Vec<RequestSpan>> {
        let records = self.matched(&req.query)?;
        let keyed = self.keyed(&records, &req.query);
        let mut out = analysis::request_durations(keyed.iter().map(|(r, k)| (*r, k.as_str())), &req.id_field);
        analysis::trace::rank_requests(&mut out);
        Ok(out)
    }

    /// What was happening between `from` and `to`, across every source.
    pub fn moment(&self, req: &MomentRequest) -> Result<MomentSummary> {
        let (from, to) = match req.query.window {
            // A live window: the last N ms up to now.
            Some(_) => (self.resolve_window(&req.query).from.unwrap_or(req.from), self.live_now()),
            None => (req.from.min(req.to), req.from.max(req.to)),
        };
        let base = Query {
            window: None,
            ..req.query.clone()
        };
        let [window, previous, all] = analysis::moment::moment_queries(&base, from, to);
        let activity_cfg = &self.profile.profile.activity;
        let records = self.matched(&window)?;
        let mut builder = analysis::moment::MomentBuilder::new(activity_cfg, from);
        for (r, key) in self.keyed(&records, &window) {
            builder.add(&self.row(r), &key);
        }
        let mut before = analysis::moment::MetricCollector::new(&activity_cfg.metrics);
        let prev_records = self.matched(&previous)?;
        for (r, key) in self.keyed(&prev_records, &previous) {
            before.add(&self.row(r), &key);
        }
        let all = self.anomalies(&QueryRequest { query: all })?;
        let anomalies = all.into_iter().filter(|a| a.from <= to && a.to >= from).collect();
        Ok(builder.finish(to, before, anomalies))
    }

    /// A record with its source name.
    fn row<'a>(&'a self, rec: &'a Record) -> RecordRow<'a> {
        RecordRow {
            rec,
            source: self.source_name(rec.source),
        }
    }
}

impl Workspace {
    /// Parses sample text with a candidate profile exactly as an import would
    /// (importer selection, extractors, pipeline, markers) without touching
    /// the workspace. Powers the "teach a format" wizard and profile previews.
    pub fn preview_parse(&self, req: &crate::api::ParsePreviewRequest) -> crate::api::ParsePreview {
        use crate::api::{ParsePreview, ParsedPreview};
        let run = || -> Result<ParsePreview> {
            let profile = crate::profile::Profile::from_toml(&req.toml).map_err(|e| anyhow::anyhow!(e.to_string()))?;
            let compiled = self.registry.compile_profile(profile)?;
            let name = if req.file_name.is_empty() {
                "sample.log"
            } else {
                req.file_name.as_str()
            };
            let path = std::path::Path::new(name);
            let ctx = crate::importers::ImportCtx {
                profile: &compiled,
                path,
                file_date: None,
            };
            let head: String = req.sample.chars().take(64 * 1024).collect();
            let importer = self
                .registry
                .importers
                .select(&ctx, &head)
                .ok_or_else(|| anyhow::anyhow!("no importer recognises this sample; adjust the output template or regex"))?;
            let imported = importer.import(&ctx, &req.sample)?;
            let (records, stats) = compiled.ingest(imported.records, name);
            let dropped = stats.first().map_or(0, |s| s.input) - stats.last().map_or(0, |s| s.output);
            let kinds: Vec<&str> = compiled.markers.iter().map(|m| m.kind.as_str()).collect();
            Ok(ParsePreview {
                importer: Some(importer.id().to_string()),
                total: records.len(),
                skipped: imported.skipped,
                dropped,
                records: records
                    .into_iter()
                    .take(req.limit.clamp(1, 1000))
                    .map(|r| ParsedPreview {
                        markers: (0..kinds.len())
                            .filter(|i| r.markers & (1 << i) != 0)
                            .map(|i| kinds[i].to_string())
                            .collect(),
                        line: r.line,
                        ts: r.ts,
                        level: r.level,
                        message: r.message,
                        exception: r.exception,
                        fields: r.fields,
                    })
                    .collect(),
                error: None,
            })
        };
        run().unwrap_or_else(|e| ParsePreview {
            error: Some(format!("{e:#}")),
            ..Default::default()
        })
    }
}

impl Workspace {
    /// The automatic health check: what went wrong and what needs attention,
    /// most severe first, each with a suggested action.
    pub fn overview(&self, req: &QueryRequest) -> Result<Vec<crate::api::Finding>> {
        let anomalies = self.anomalies(req)?;
        let groups = self.exception_groups(req)?;
        let quarantined = self
            .registry
            .plugins()
            .into_iter()
            .filter(|p| p.status == crate::plugin::PluginStatus::Quarantined)
            .map(|p| (p.name, p.last_error.unwrap_or_default()))
            .collect();
        let inputs = analysis::moment::OverviewInputs {
            sources: &self.sources,
            unrecognized: &self.unrecognized,
            quarantined,
        };
        Ok(analysis::moment::findings(&anomalies, &groups, &inputs))
    }
}
