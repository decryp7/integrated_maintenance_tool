//! Running a command over a store (through the ports only): prepare the store, import, then query, analyse or
//! report. Failures become a message and an [`Exit`]; with `--json` the message is also a JSON object on standard
//! output, so scripts read one shape.

use crate::args::{AnalyzeKind, Command, ReportFormat, Source, TableFormat, MAX_LIMIT};
use crate::host::{SharedStore, StoreHost, STORE_DB_FILE};
use crate::output::{error_json, write_table, Table};
use crate::{Exit, BIN_NAME};
use debrief_core::api::QueryRequest;
use debrief_core::known_issues::{match_known_issues, parse_known_issues, KnowledgeText};
use debrief_core::ports::{EventView, EventsRequest, ImportReport, Store, StoreError};
use debrief_core::ql::QlResult;
use debrief_core::query::Query;
use debrief_core::report::{html::HtmlRenderer, ReportRenderer, Theme};
use debrief_core::Profile;
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Largest knowledge file read (a Markdown note, never megabytes).
const MAX_KNOWLEDGE_BYTES: u64 = 4 << 20;
/// Events fetched per call when a query returns events.
const EVENT_CHUNK: usize = 500;

/// Commands that need no store: `help` and `--version`. `None` for the others.
pub fn run_static(command: &Command, out: &mut dyn Write) -> Option<Exit> {
    match command {
        Command::Help => {
            let _ = writeln!(out, "{}", crate::args::usage());
            Some(Exit::Ok)
        }
        Command::Version => {
            let _ = writeln!(out, "{BIN_NAME} {}", env!("CARGO_PKG_VERSION"));
            Some(Exit::Ok)
        }
        _ => None,
    }
}

/// Runs `command`; `out` gets the results, `err` the progress and notes.
pub fn run(command: Command, host: &dyn StoreHost, out: &mut dyn Write, err: &mut dyn Write) -> Exit {
    if let Some(exit) = run_static(&command, out) {
        return exit;
    }
    let json = wants_json(&command);
    match execute(command, host, out, err) {
        Ok(exit) => exit,
        Err(message) => {
            let _ = writeln!(err, "{BIN_NAME}: {message}");
            if json {
                let _ = writeln!(out, "{}", error_json(&message));
            }
            Exit::Failure
        }
    }
}

fn wants_json(c: &Command) -> bool {
    match c {
        Command::Import { json, .. } | Command::Analyze { json, .. } | Command::Report { json, .. } => *json,
        Command::Query { format, .. } => *format == TableFormat::Json,
        Command::Watch { json, .. } => *json,
        Command::Help | Command::Version => false,
    }
}

pub(crate) fn store_message(e: StoreError) -> String {
    e.to_string()
}

/// A scratch store folder, removed when it goes out of scope.
struct Scratch(Option<PathBuf>);

impl Drop for Scratch {
    fn drop(&mut self) {
        if let Some(p) = self.0.take() {
            let _ = std::fs::remove_dir_all(p);
        }
    }
}

fn scratch_folder() -> Result<PathBuf, String> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("{BIN_NAME}-cli-{}-{n}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not make a scratch store in {}: {e}", dir.display()))?;
    Ok(dir)
}

/// The store a command works on, with its scratch folder (if any) removed when this is dropped. The store is
/// declared first so it lets go of its files before the folder is removed.
pub(crate) struct Prepared {
    pub(crate) store: SharedStore,
    _scratch: Scratch,
}

fn canonical(paths: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    paths
        .iter()
        .map(|p| {
            std::fs::canonicalize(p).map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => format!("{} does not exist", p.display()),
                _ => format!("cannot read {}: {e}", p.display()),
            })
        })
        .collect()
}

/// Opens (or creates) the store of `src`: the kept folder, or a scratch one removed when the result is dropped.
pub(crate) fn open_store(src: &Source, host: &dyn StoreHost) -> Result<Prepared, String> {
    let profile = Profile::default_profile();
    let (dir, scratch) = match &src.store {
        Some(d) => {
            std::fs::create_dir_all(d).map_err(|e| format!("cannot use the store folder {}: {e}", d.display()))?;
            (d.clone(), Scratch(None))
        }
        None => {
            let d = scratch_folder()?;
            (d.clone(), Scratch(Some(d)))
        }
    };
    let store = if dir.join(STORE_DB_FILE).is_file() {
        host.open(&dir, &profile)
    } else {
        host.create(&dir, &profile)
    }
    .map_err(store_message)?;
    Ok(Prepared { store, _scratch: scratch })
}

/// Opens the store, imports `src.open` and reports on `err`. `None` report: nothing was imported.
fn prepare(src: &Source, host: &dyn StoreHost, err: &mut dyn Write) -> Result<(Prepared, Option<ImportReport>), String> {
    let paths = canonical(&src.open)?;
    if paths.is_empty() && src.store.is_none() {
        return Err("nothing to read: give --open PATH, or --store DIR with a store from an earlier run".into());
    }
    let prepared = open_store(src, host)?;
    if paths.is_empty() {
        return Ok((prepared, None));
    }
    if src.since.is_some() || src.until.is_some() {
        prepared
            .store
            .set_import_window(&debrief_core::ingest::ImportWindow {
                from: src.since,
                to: src.until,
            })
            .map_err(store_message)?;
    }
    let report = prepared.store.open(&paths).and_then(|s| s.wait()).map_err(store_message)?;
    for u in &report.unrecognized {
        let _ = writeln!(err, "{BIN_NAME}: skipped {}: {}", u.name, u.reason);
    }
    if report.events == 0 && !report.unrecognized.is_empty() && src.store.is_none() {
        return Err(format!(
            "no file could be read ({} skipped); the first reason: {}",
            report.unrecognized.len(),
            report.unrecognized[0].reason
        ));
    }
    Ok((prepared, Some(report)))
}

fn execute(command: Command, host: &dyn StoreHost, out: &mut dyn Write, err: &mut dyn Write) -> Result<Exit, String> {
    match command {
        Command::Help | Command::Version => Ok(Exit::Ok),
        Command::Watch {
            src,
            dir,
            knowledge,
            interval_ms,
            debounce_ms,
            once,
            json,
            fail_on_findings,
        } => {
            let prepared = open_store(&src, host)?;
            let opts = crate::watch::WatchOptions {
                dir,
                knowledge,
                interval_ms,
                debounce_ms,
                once,
                json,
            };
            let findings = crate::watch::watch(prepared.store.as_ref(), &opts, &crate::watch::stop_flag(), out, err)?;
            Ok(if fail_on_findings && findings > 0 {
                Exit::Findings
            } else {
                Exit::Ok
            })
        }
        Command::Import { src, json } => {
            let (_prepared, report) = prepare(&src, host, err)?;
            let report = report.unwrap_or_default();
            if json {
                let sources: Vec<Value> = report
                    .sources
                    .iter()
                    .map(|s| json!({"name": s.name, "importer": s.importer, "records": s.records, "skipped": s.skipped, "from": s.from, "to": s.to}))
                    .collect();
                let unrecognized: Vec<Value> = report
                    .unrecognized
                    .iter()
                    .map(|u| json!({"name": u.name, "reason": u.reason}))
                    .collect();
                writeln!(
                    out,
                    "{}",
                    json!({"ok": true, "events": report.events, "bytes": report.bytes, "elapsedMs": report.elapsed_ms,
                           "sources": sources, "unrecognized": unrecognized})
                )
                .map_err(io)?;
            } else {
                writeln!(
                    out,
                    "imported {} events from {} sources ({} skipped) in {} ms",
                    report.events,
                    report.sources.len(),
                    report.unrecognized.len(),
                    report.elapsed_ms
                )
                .map_err(io)?;
            }
            Ok(Exit::Ok)
        }
        Command::Query { src, ql, format, limit } => {
            let (prepared, _) = prepare(&src, host, err)?;
            let table = run_ql(prepared.store.as_ref(), &ql, limit)?;
            write_table(&table, format, out, err).map_err(io)?;
            Ok(Exit::Ok)
        }
        Command::Analyze {
            src,
            kind,
            filter,
            limit,
            json,
            knowledge,
            fail_on_findings,
            subset,
            fields,
        } => {
            let (prepared, _) = prepare(&src, host, err)?;
            let ask = Ask {
                kind,
                filter: &filter,
                limit,
                knowledge: &knowledge,
                subset: &subset,
                fields: &fields,
            };
            let found = analyze(prepared.store.as_ref(), &ask)?;
            write_analysis(&found, json, out, err).map_err(io)?;
            Ok(if fail_on_findings && found.total > 0 {
                Exit::Findings
            } else {
                Exit::Ok
            })
        }
        Command::Report {
            src,
            out: target,
            format,
            title,
            knowledge,
            from,
            to,
            top,
            overwrite,
            json,
        } => {
            debrief_core::overwrite::check_overwrite(&target, overwrite).map_err(|e| e.to_string())?;
            let (prepared, _) = prepare(&src, host, err)?;
            let bytes = build_report(prepared.store.as_ref(), format, &title, &knowledge, from, to, top)?;
            write_atomically(&target, &bytes)?;
            if json {
                writeln!(
                    out,
                    "{}",
                    json!({"ok": true, "path": target.display().to_string(), "bytes": bytes.len()})
                )
                .map_err(io)?;
            } else {
                writeln!(out, "wrote {} ({} bytes)", target.display(), bytes.len()).map_err(io)?;
            }
            Ok(Exit::Ok)
        }
    }
}

fn io(e: std::io::Error) -> String {
    format!("could not write the output: {e}")
}

fn query_with(ql: &str) -> QueryRequest {
    QueryRequest {
        query: Query {
            ql: ql.to_string(),
            ..Query::default()
        },
    }
}

fn event_row(e: &EventView) -> Vec<Value> {
    vec![
        json!(e.id),
        json!(debrief_core::report::format_ts(e.ts)),
        json!(e.level.as_str()),
        json!(e.source_name),
        json!(e.message),
    ]
}

/// Runs a query-language text and shapes the answer as a bounded table.
pub fn run_ql(store: &dyn Store, ql: &str, limit: usize) -> Result<Table, String> {
    let limit = limit.clamp(1, MAX_LIMIT);
    match store.run_query(&query_with(ql)).map_err(store_message)? {
        QlResult::Table { columns, rows } => {
            let total = rows.len();
            Ok(Table {
                columns,
                rows: rows.into_iter().take(limit).collect(),
                total,
            })
        }
        QlResult::Events { ids, fields } => {
            let total = ids.len();
            let wanted: Vec<debrief_core::batch::EventId> = ids
                .iter()
                .take(limit)
                .map(|i| debrief_core::batch::EventId(u64::from(*i)))
                .collect();
            let mut columns: Vec<String> = ["id", "time", "level", "source", "message"].map(String::from).to_vec();
            let extra = fields.unwrap_or_default();
            columns.extend(extra.iter().cloned());
            let mut rows = Vec::with_capacity(wanted.len());
            for chunk in wanted.chunks(EVENT_CHUNK) {
                let views = store
                    .events(&EventsRequest {
                        ids: chunk.to_vec(),
                        group_by: Vec::new(),
                    })
                    .map_err(store_message)?;
                for v in &views {
                    let mut row = event_row(v);
                    row.extend(extra.iter().map(|f| v.fields.get(f).cloned().unwrap_or(Value::Null)));
                    rows.push(row);
                }
            }
            Ok(Table { columns, rows, total })
        }
    }
}

/// An analysis' answer: the (bounded) items as JSON, the total and what bounded it.
pub struct Found {
    pub kind: AnalyzeKind,
    pub total: usize,
    pub items: Vec<Value>,
    /// Plain lines for the text output, one per item.
    pub lines: Vec<String>,
    pub matched: u64,
    pub analysed: u64,
    pub limits: Vec<String>,
    pub warnings: Vec<String>,
}

fn to_value<T: serde::Serialize>(t: &T) -> Value {
    serde_json::to_value(t).unwrap_or(Value::Null)
}

pub(crate) fn read_knowledge(files: &[PathBuf]) -> Result<Vec<(String, String)>, String> {
    files
        .iter()
        .map(|p| {
            let meta = std::fs::metadata(p).map_err(|e| format!("cannot read {}: {e}", p.display()))?;
            if meta.len() > MAX_KNOWLEDGE_BYTES {
                return Err(format!("{} is larger than {} MB", p.display(), MAX_KNOWLEDGE_BYTES >> 20));
            }
            let text = std::fs::read_to_string(p).map_err(|e| format!("cannot read {}: {e}", p.display()))?;
            Ok((
                p.file_name()
                    .map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned()),
                text,
            ))
        })
        .collect()
}

/// What `analyze` is asked.
pub struct Ask<'a> {
    pub kind: AnalyzeKind,
    /// A query-language filter narrowing the events analysed.
    pub filter: &'a str,
    pub limit: usize,
    pub knowledge: &'a [PathBuf],
    /// Outliers: the selected events (empty: errors) and the fields examined (empty: all).
    pub subset: &'a str,
    pub fields: &'a [String],
}

/// Runs one analysis over the events matching the ask's filter.
pub fn analyze(store: &dyn Store, ask: &Ask<'_>) -> Result<Found, String> {
    let (kind, knowledge) = (ask.kind, ask.knowledge);
    let limit = ask.limit.clamp(1, MAX_LIMIT);
    let req = query_with(ask.filter);
    let mut found = Found {
        kind,
        total: 0,
        items: Vec::new(),
        lines: Vec::new(),
        matched: 0,
        analysed: 0,
        limits: Vec::new(),
        warnings: Vec::new(),
    };
    match kind {
        AnalyzeKind::Patterns => {
            let a = store.patterns(&req).map_err(store_message)?;
            let mut p = a.result;
            p.sort_by_key(|x| std::cmp::Reverse(x.count));
            found.total = p.len();
            for x in p.iter().take(limit) {
                found.lines.push(format!("{:>8}  {}", x.count, x.template));
                found.items.push(to_value(x));
            }
            fill_coverage(&mut found, &a.coverage);
        }
        AnalyzeKind::Exceptions => {
            let a = store.exception_groups(&req).map_err(store_message)?;
            let mut g = a.result;
            g.sort_by_key(|x| std::cmp::Reverse(x.count));
            found.total = g.len();
            for x in g.iter().take(limit) {
                found
                    .lines
                    .push(format!("{:>8}  {}: {}", x.count, x.type_name, first_line(&x.message)));
                found.items.push(to_value(x));
            }
            fill_coverage(&mut found, &a.coverage);
        }
        AnalyzeKind::Anomalies => {
            let a = store.anomalies(&req).map_err(store_message)?;
            let mut v = a.result;
            v.sort_by(|x, y| y.score.total_cmp(&x.score));
            found.total = v.len();
            for x in v.iter().take(limit) {
                found.lines.push(format!("{:?} {}", x.kind, first_line(&x.message)));
                found.items.push(to_value(x));
            }
            fill_coverage(&mut found, &a.coverage);
        }
        AnalyzeKind::Outliers => {
            let a = debrief_core::analysis::error_outliers(
                store,
                &debrief_core::analysis::OutlierRequest {
                    query: req.query.clone(),
                    subset: ask.subset.to_string(),
                    fields: ask.fields.to_vec(),
                    limit,
                },
            )
            .map_err(store_message)?;
            found.total = a.result.outliers.len();
            for o in &a.result.outliers {
                found.lines.push(format!(
                    "{:>8} vs {:<8}  {:>6.1}x  {}={}",
                    o.subset_count, o.rest_count, o.lift, o.field, o.value
                ));
                found.items.push(to_value(o));
            }
            fill_coverage(&mut found, &a.coverage);
        }
        AnalyzeKind::Issues => {
            let texts = read_knowledge(knowledge)?;
            for (name, text) in &texts {
                let (_, warnings) = parse_known_issues(name, text);
                found.warnings.extend(warnings);
            }
            let files: Vec<KnowledgeText<'_>> = texts.iter().map(|(name, text)| KnowledgeText { name, text }).collect();
            let report = match_known_issues(store, &files).map_err(store_message)?;
            found.warnings.extend(report.warnings.iter().cloned());
            found.warnings.dedup();
            found.total = report.matches.len();
            for m in report.matches.iter().take(limit) {
                found
                    .lines
                    .push(format!("{:>8}  {}: {}", m.count, m.issue.title, first_line(&m.issue.fix)));
                found.items.push(to_value(m));
            }
        }
    }
    Ok(found)
}

fn fill_coverage(found: &mut Found, c: &debrief_core::ports::Coverage) {
    found.matched = c.matched;
    found.analysed = c.analysed;
    found.limits = c.limits.clone();
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or("")
}

fn write_analysis(found: &Found, json: bool, out: &mut dyn Write, err: &mut dyn Write) -> std::io::Result<()> {
    let shown = found.items.len();
    if json {
        let v = json!({"ok": true, "kind": found.kind.name(), "total": found.total, "truncated": shown < found.total,
                       "matched": found.matched, "analysed": found.analysed, "limits": found.limits,
                       "warnings": found.warnings, "items": found.items});
        writeln!(out, "{v}")?;
    } else {
        for l in &found.lines {
            writeln!(out, "{l}")?;
        }
        if found.total == 0 {
            writeln!(out, "no {}", found.kind.name())?;
        }
    }
    for w in &found.warnings {
        writeln!(err, "{BIN_NAME}: {w}")?;
    }
    for l in &found.limits {
        writeln!(err, "{BIN_NAME}: limited: {l}")?;
    }
    if shown < found.total {
        writeln!(err, "{shown} of {} shown; raise --limit (at most {MAX_LIMIT})", found.total)?;
    }
    Ok(())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Builds the incident report and renders it.
pub fn build_report(
    store: &dyn Store,
    format: ReportFormat,
    title: &str,
    knowledge: &[PathBuf],
    from: Option<i64>,
    to: Option<i64>,
    top: Option<usize>,
) -> Result<Vec<u8>, String> {
    let texts = read_knowledge(knowledge)?;
    let files: Vec<KnowledgeText<'_>> = texts.iter().map(|(name, text)| KnowledgeText { name, text }).collect();
    let issues = match_known_issues(store, &files).map_err(store_message)?.matches;
    let now = now_ms();
    let report = debrief_core::incident::build_incident_report(
        store,
        &debrief_core::incident::IncidentOptions {
            id: format!("incident-{now}"),
            title: title.to_string(),
            from,
            to,
            top,
            notes: String::new(),
            issues,
            now,
        },
    )
    .map_err(store_message)?;
    let resolved = debrief_core::report::resolve::resolve_in(store, &report, None).map_err(|e| format!("{e:#}"))?;
    let mut bytes = Vec::new();
    match format {
        ReportFormat::Html => HtmlRenderer.render(&resolved, Theme::Light, &mut bytes),
        ReportFormat::Pdf => debrief_pdf::PdfRenderer.render(&resolved, Theme::Light, &mut bytes),
    }
    .map_err(|e| format!("could not render the report: {e:#}"))?;
    Ok(bytes)
}

/// Writes `bytes` to a sibling temporary file and renames it, so a failed run leaves no half-written report.
fn write_atomically(target: &Path, bytes: &[u8]) -> Result<(), String> {
    let name = target
        .file_name()
        .map_or_else(|| "report".to_string(), |n| n.to_string_lossy().into_owned());
    let tmp = target.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    std::fs::write(&tmp, bytes).map_err(|e| format!("cannot write {}: {e}", target.display()))?;
    std::fs::rename(&tmp, target).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("cannot write {}: {e}", target.display())
    })
}
