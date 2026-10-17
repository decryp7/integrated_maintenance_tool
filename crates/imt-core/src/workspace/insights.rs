//! Analyses over the records matching a query: the query language, patterns,
//! exceptions, anomalies, window comparison, field statistics, traces and the
//! Moment summary ("what was happening between from and to").

use super::Workspace;
use crate::analysis::{self, Anomaly, AnomalyConfig, CompareConfig, Comparison, FieldStats, RequestSpan, WaterfallRow};
use crate::api::{
    ActivityItem, CompareRequest, CompletionRequest, CorrelateRequest, FieldStatsRequest, MetricWindow, MomentInstance, MomentRequest,
    MomentSummary, QueryRequest, RequestsRequest, TraceRequest, TraceSummary, UserCount,
};
use crate::exceptions::{self, ExceptionGroup};
use crate::model::{Level, Record};
use crate::patterns::{self, MinerConfig, Pattern};
use crate::ql::{self, Completion, QlError, QlResult};
use crate::query::{group_key, Query};
use anyhow::Result;
use std::collections::{BTreeMap, HashMap};

/// Most records any single analysis looks at, to keep the UI responsive.
const MAX_ANALYSED: usize = 2_000_000;
const MAX_ACTIVITY: usize = 200;
const MAX_TRACES: usize = 200;
const MAX_REQUESTS: usize = 500;
const MAX_CORRELATED: usize = 10_000;

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
        let program = ql::parse(&req.query.ql)?;
        let base = Query {
            ql: String::new(),
            ..req.query.clone()
        };
        let records = self.matched(&base).map_err(|e| QlError {
            message: format!("{e:#}"),
            offset: 0,
            len: 0,
        })?;
        let mut it = records.iter().map(|r| (*r, self.source_name(r.source)));
        ql::run(&program, &mut it)
    }

    /// Checks a query-language text; `None` when it parses.
    pub fn check_query(&self, text: &str) -> Option<QlError> {
        ql::parse(text).err()
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
            records.iter().copied(),
            &req.field,
            req.buckets.clamp(1, 200),
        ))
    }

    /// Ids (time order) of every event carrying `field = value`, across all sources.
    pub fn correlate(&self, req: &CorrelateRequest) -> Vec<u32> {
        let mut ids = analysis::correlate(self.records.iter(), &req.field, &req.value);
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
        let mut by_trace: BTreeMap<&str, Vec<&analysis::Span>> = BTreeMap::new();
        for s in &spans {
            by_trace.entry(s.trace_id.as_str()).or_default().push(s);
        }
        let mut out: Vec<TraceSummary> = by_trace
            .into_iter()
            .map(|(id, spans)| {
                let start = spans.iter().map(|s| s.start).min().unwrap_or(0);
                let end = spans.iter().map(|s| s.start as f64 + s.duration_ms).fold(start as f64, f64::max);
                let root = spans
                    .iter()
                    .find(|s| s.parent.is_none())
                    .or(spans.first())
                    .map(|s| s.name.clone())
                    .unwrap_or_default();
                TraceSummary {
                    trace_id: id.to_string(),
                    spans: spans.len(),
                    errors: spans.iter().map(|s| s.errors).sum(),
                    start,
                    duration_ms: end - start as f64,
                    root,
                }
            })
            .collect();
        out.sort_by(|a, b| b.duration_ms.total_cmp(&a.duration_ms).then_with(|| a.trace_id.cmp(&b.trace_id)));
        out.truncate(MAX_TRACES);
        Ok((out, Vec::new()))
    }

    /// Requests (by id field) in the matching records, longest first.
    pub fn requests(&self, req: &RequestsRequest) -> Result<Vec<RequestSpan>> {
        let records = self.matched(&req.query)?;
        let keyed = self.keyed(&records, &req.query);
        let mut out = analysis::request_durations(keyed.iter().map(|(r, k)| (*r, k.as_str())), &req.id_field);
        out.sort_by(|a, b| (b.last - b.first).cmp(&(a.last - a.first)).then_with(|| a.id.cmp(&b.id)));
        out.truncate(MAX_REQUESTS);
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
        let window = Query {
            from: Some(from),
            to: Some(to),
            ..base.clone()
        };
        let records = self.matched(&window)?;
        let keyed = self.keyed(&records, &window);
        let activity_cfg = &self.profile.profile.activity;

        let mut levels: BTreeMap<Level, usize> = Level::ALL.iter().map(|l| (*l, 0)).collect();
        let mut per_instance: BTreeMap<&str, Vec<&Record>> = BTreeMap::new();
        let mut users: HashMap<(String, String), u64> = HashMap::new();
        let mut activity = Vec::new();
        for (r, key) in &keyed {
            *levels.get_mut(&r.level).unwrap() += 1;
            per_instance.entry(key.as_str()).or_default().push(r);
            let source = self.source_name(r.source);
            let user = first_present_field(r, &activity_cfg.user, source).map(|(field, value)| {
                *users.entry((field.to_string(), value.clone())).or_default() += 1;
                value
            });
            let action = first_present(r, &activity_cfg.action, source);
            if (user.is_some() || action.is_some()) && activity.len() < MAX_ACTIVITY {
                activity.push(ActivityItem {
                    id: r.id,
                    ts: r.ts.unwrap_or(from),
                    instance: key.clone(),
                    user,
                    action: action.unwrap_or_else(|| r.message.chars().take(200).collect()),
                    level: r.level,
                });
            }
        }

        let mut instances: Vec<MomentInstance> = per_instance
            .into_iter()
            .map(|(key, recs)| {
                let errors = recs.iter().filter(|r| r.level >= Level::Error).count();
                let mut patterns = patterns::mine(recs.iter().copied(), &MinerConfig::default());
                patterns.truncate(3);
                MomentInstance {
                    key: key.to_string(),
                    events: recs.len(),
                    warnings: recs.iter().filter(|r| r.level == Level::Warning).count(),
                    errors,
                    patterns,
                    first_error: recs.iter().find(|r| r.level >= Level::Error).map(|r| r.id),
                }
            })
            .collect();
        instances.sort_by(|a, b| b.errors.cmp(&a.errors).then_with(|| b.events.cmp(&a.events)));

        let span = (to - from).max(1);
        let previous = Query {
            from: Some(from - span),
            to: Some(from - 1),
            ..base.clone()
        };
        let prev_records = self.matched(&previous)?;
        let metrics = self.metric_windows(&keyed, &self.keyed(&prev_records, &previous), &activity_cfg.metrics);

        let mut users: Vec<UserCount> = users
            .into_iter()
            .map(|((field, value), count)| UserCount { field, value, count })
            .collect();
        users.sort_by(|a, b| {
            b.count
                .cmp(&a.count)
                .then_with(|| a.value.cmp(&b.value))
                .then_with(|| a.field.cmp(&b.field))
        });
        users.truncate(20);

        let all = self.anomalies(&QueryRequest {
            query: Query {
                from: None,
                to: None,
                ..base.clone()
            },
        })?;
        let anomalies = all.into_iter().filter(|a| a.from <= to && a.to >= from).collect();

        Ok(MomentSummary {
            from,
            to,
            total: records.len(),
            levels,
            instances,
            metrics,
            users,
            activity,
            anomalies,
        })
    }

    fn metric_windows(&self, now: &[(&Record, String)], before: &[(&Record, String)], fields: &[String]) -> Vec<MetricWindow> {
        type Series = BTreeMap<(String, String), Vec<f64>>;
        let collect = |rows: &[(&Record, String)]| -> Series {
            let mut out: Series = BTreeMap::new();
            for (r, key) in rows {
                // PerfMon long format: one counter value per record.
                if let (Some(counter), Some(value)) = (r.fields.get("Counter"), r.field_number("Value")) {
                    let machine = r.field_text("MachineName", "").map(|m| m.into_owned()).unwrap_or_default();
                    let series = match r.field_text("Instance", "") {
                        Some(i) if !i.is_empty() => format!("{machine} · {i}"),
                        _ => machine,
                    };
                    out.entry((series, crate::model::value_text(counter).into_owned()))
                        .or_default()
                        .push(value);
                    continue;
                }
                for f in fields {
                    if let Some(v) = r.field_number(f) {
                        out.entry((key.clone(), f.clone())).or_default().push(v);
                    }
                }
            }
            out
        };
        let prev = collect(before);
        let mut result: Vec<MetricWindow> = collect(now)
            .into_iter()
            .map(|((series, metric), values)| {
                let avg = values.iter().sum::<f64>() / values.len() as f64;
                let previous_avg = prev
                    .get(&(series.clone(), metric.clone()))
                    .map(|p| p.iter().sum::<f64>() / p.len() as f64);
                let change_pct = previous_avg.filter(|p| *p != 0.0).map(|p| (avg - p) / p.abs() * 100.0);
                MetricWindow {
                    samples: values.len(),
                    min: values.iter().copied().fold(f64::INFINITY, f64::min),
                    max: values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                    last: *values.last().unwrap(),
                    avg,
                    previous_avg,
                    change_pct,
                    series,
                    metric,
                }
            })
            .collect();
        result.sort_by(|a, b| {
            let ca = a.change_pct.map(f64::abs).unwrap_or(0.0);
            let cb = b.change_pct.map(f64::abs).unwrap_or(0.0);
            cb.total_cmp(&ca)
                .then_with(|| a.series.cmp(&b.series))
                .then_with(|| a.metric.cmp(&b.metric))
        });
        result
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
        use crate::analysis::AnomalyKind;
        use crate::api::{Finding, FindingAction, Severity};
        let mut out = Vec::new();
        let zoom = |a: &Anomaly| FindingAction::Zoom {
            from: a.from,
            to: a.to,
            record_id: a.sample_ids.first().copied(),
        };
        for a in self.anomalies(req)? {
            let severity = match a.kind {
                AnomalyKind::Crash => Severity::Critical,
                AnomalyKind::ErrorSpike | AnomalyKind::MemoryGrowth | AnomalyKind::Silence => Severity::Warning,
                AnomalyKind::SlowOutlier | AnomalyKind::RateChange => Severity::Info,
            };
            let title = match a.kind {
                AnomalyKind::Crash => "Crash",
                AnomalyKind::ErrorSpike => "Error spike",
                AnomalyKind::MemoryGrowth => "Memory growth",
                AnomalyKind::Silence => "Went silent",
                AnomalyKind::SlowOutlier => "Slow outliers",
                AnomalyKind::RateChange => "Rate change",
            };
            out.push(Finding {
                severity,
                title: title.into(),
                detail: a.message.clone(),
                action: Some(zoom(&a)),
            });
        }
        for g in self.exception_groups(req)?.into_iter().filter(|g| g.count >= 5).take(5) {
            let instances = g.instances.len();
            out.push(Finding {
                severity: Severity::Warning,
                title: format!("{} ×{}", short_type(&g.type_name), g.count),
                detail: format!(
                    "{} on {} instance{}{}",
                    g.message.chars().take(160).collect::<String>(),
                    instances,
                    if instances == 1 { "" } else { "s" },
                    g.top_frame.as_deref().map(|f| format!(", at {f}")).unwrap_or_default()
                ),
                action: Some(FindingAction::Exception {
                    fingerprint: g.fingerprint.clone(),
                    record_id: g.sample_ids.first().copied(),
                }),
            });
        }
        for s in &self.sources {
            let lines = s.records + s.skipped;
            if s.skipped > 0 && s.skipped * 5 > lines {
                out.push(Finding {
                    severity: Severity::Warning,
                    title: "Lines not understood".into(),
                    detail: format!(
                        "{}: {}% of the lines could not be read as events",
                        s.name,
                        s.skipped * 100 / lines.max(1)
                    ),
                    action: Some(FindingAction::SetupFormat { path: s.path.clone() }),
                });
            }
            if s.from.is_none() && s.records > 0 {
                out.push(Finding {
                    severity: Severity::Info,
                    title: "No timestamps".into(),
                    detail: format!("{} has no recognisable timestamps, so it is missing from the timeline", s.name),
                    action: Some(FindingAction::SetupFormat { path: s.path.clone() }),
                });
            }
        }
        for u in &self.unrecognized {
            out.push(Finding {
                severity: Severity::Info,
                title: "File not opened".into(),
                detail: format!("{}: {}", u.name, u.reason),
                action: Some(FindingAction::SetupFormat { path: u.path.clone() }),
            });
        }
        let dropped: usize = self.sources.iter().map(|s| s.dropped).sum();
        if dropped > 0 {
            out.push(Finding {
                severity: Severity::Info,
                title: "Events removed by the pipeline".into(),
                detail: format!("{dropped} events were dropped by the profile's cleaning steps (your files are unchanged)"),
                action: Some(FindingAction::OpenPanel { panel: "pipeline".into() }),
            });
        }
        for p in self
            .registry
            .plugins()
            .into_iter()
            .filter(|p| p.status == crate::plugin::PluginStatus::Quarantined)
        {
            out.push(Finding {
                severity: Severity::Warning,
                title: "Plugin disabled after crashes".into(),
                detail: format!("{}: {}", p.name, p.last_error.unwrap_or_default()),
                action: Some(FindingAction::OpenPanel { panel: "plugins".into() }),
            });
        }
        out.sort_by_key(|f| f.severity);
        Ok(out)
    }
}

/// "System.Net.Http.HttpRequestException" -> "HttpRequestException".
fn short_type(t: &str) -> &str {
    t.rsplit('.').next().unwrap_or(t)
}

fn first_present(r: &Record, fields: &[String], source: &str) -> Option<String> {
    first_present_field(r, fields, source).map(|(_, v)| v)
}

/// The first of `fields` with a non-empty value on `r`, with the field's name.
fn first_present_field<'f>(r: &Record, fields: &'f [String], source: &str) -> Option<(&'f str, String)> {
    fields.iter().find_map(|f| {
        r.field_text(f, source)
            .filter(|v| !v.is_empty())
            .map(|v| (f.as_str(), v.into_owned()))
    })
}
