//! IMT's own log: `tracing` with a CLEF layer (Serilog's compact JSON, see [`crate::clef`]) writing to
//! daily rolling files (`imt.YYYY-MM-DD.log` via `tracing-appender`) and stdout.
//!
//! Record flow:
//! - `log` records (IMT's own `log::info!(...)`, dependencies, and the webview's lines forwarded by the
//!   log plugin's `log` command) go through [`ActivityLogger`] (breadcrumbs, error dedup) and then
//!   [`TracingBridge`], which hands them to `tracing` (via `tracing-log`; key/values travel as one
//!   `log.kv` JSON field so CLEF properties keep their types).
//! - `tracing` events and spans (IPC dispatch spans carry `Window`, `OperationId` and `Command`) reach
//!   [`ClefLayer`] directly; the fields of the enclosing spans become properties of every event.
//!
//! Limits come from the diagnostics settings ([`LogLimits`]): the level applies at once ([`set_level`]);
//! `keep_files` old daily files are kept; after `max_file_mb` in one day only errors are written until the
//! next file. Writes are synchronous (lines are small), so a crash's last entries reach the file. What may be
//! logged at all is the callers' business: see `activity::privacy` (never file contents, query text only as
//! a fingerprint in normal mode).

use crate::activity::ActivityLogger;
use crate::diagnostics::{LogLevel, LogLimits, LOG_NAME};
use chrono::{NaiveDate, Utc};
use serde_json::{Map, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use tracing::field::{Field, Visit};
use tracing::level_filters::LevelFilter;
use tracing::span::{Attributes, Id, Record as SpanRecord};
use tracing::{Event, Subscriber};
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{reload, Layer, Registry};

/// Field carrying a bridged `log` record's key/values as a JSON object.
const KV_FIELD: &str = "log.kv";

/// The level filter of the installed subscriber, so [`set_level`] can change it at run time.
static LEVEL: OnceLock<reload::Handle<LevelFilter, Registry>> = OnceLock::new();

fn tracing_level(level: LogLevel) -> LevelFilter {
    match level {
        LogLevel::Error => LevelFilter::ERROR,
        LogLevel::Warn => LevelFilter::WARN,
        LogLevel::Info => LevelFilter::INFO,
        LogLevel::Debug => LevelFilter::DEBUG,
        LogLevel::Trace => LevelFilter::TRACE,
    }
}

/// Applies `level` to both `log` and `tracing` (no restart needed).
pub fn set_level(level: LogLevel) {
    log::set_max_level(level.filter());
    if let Some(handle) = LEVEL.get() {
        let _ = handle.modify(|f| *f = tracing_level(level));
    }
}

/// The daily file IMT writes to today in `dir`.
pub fn active_log_file(dir: &Path) -> PathBuf {
    dir.join(file_for(Utc::now().date_naive()))
}

fn file_for(day: NaiveDate) -> String {
    format!("{LOG_NAME}.{}.log", day.format("%Y-%m-%d"))
}

/// Daily files in `dir`, keeping the current one plus `keep_files` old ones.
pub fn rolling_files(dir: &Path, limits: &LogLimits) -> Result<RollingFileAppender, tracing_appender::rolling::InitError> {
    RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(LOG_NAME)
        .filename_suffix("log")
        .max_log_files(limits.keep_files as usize + 1)
        .build(dir)
}

/// Where formatted lines go.
pub trait LineSink: Send + Sync + 'static {
    fn write_line(&self, level: log::Level, line: &str);
}

/// Today's bytes so far, against the daily limit.
struct Budget {
    day: NaiveDate,
    written: u64,
    warned: bool,
}

/// The rolling files, with the daily size limit (errors always pass).
pub struct FileSink {
    files: Mutex<RollingFileAppender>,
    budget: Mutex<Budget>,
    max_bytes: u64,
}

impl FileSink {
    pub fn new(dir: &Path, limits: &LogLimits) -> Result<FileSink, tracing_appender::rolling::InitError> {
        let files = rolling_files(dir, limits)?;
        let day = Utc::now().date_naive();
        let written = std::fs::metadata(dir.join(file_for(day))).map(|m| m.len()).unwrap_or(0);
        Ok(FileSink {
            files: Mutex::new(files),
            budget: Mutex::new(Budget {
                day,
                written,
                warned: false,
            }),
            max_bytes: u64::from(limits.max_file_mb) * 1024 * 1024,
        })
    }

    /// Whether a line of `len` bytes at `level` may be written today, and the one-time notice to write
    /// first when the limit was just reached.
    fn admit(&self, level: log::Level, len: usize, today: NaiveDate) -> (bool, Option<String>) {
        let mut b = crate::sync::lock(&self.budget);
        if b.day != today {
            *b = Budget {
                day: today,
                written: 0,
                warned: false,
            };
        }
        if b.written + len as u64 <= self.max_bytes || level == log::Level::Error {
            b.written += len as u64;
            return (true, None);
        }
        let notice = (!b.warned).then(|| {
            b.warned = true;
            crate::clef::format_event(
                Utc::now(),
                log::Level::Warn,
                "imt_app_lib::logging",
                &format!(
                    "The log reached its daily limit of {}; only errors are written until tomorrow",
                    imt_plugin_api::units::bytes(self.max_bytes)
                ),
                Map::new(),
                &crate::clef::Enrichment::current(),
            ) + "\n"
        });
        (false, notice)
    }
}

impl LineSink for FileSink {
    fn write_line(&self, level: log::Level, line: &str) {
        let (ok, notice) = self.admit(level, line.len() + 1, Utc::now().date_naive());
        let mut files = crate::sync::lock(&self.files);
        // A full disk or a locked file must not take the app down: the line is dropped.
        if let Some(n) = notice {
            let _ = files.write_all(n.as_bytes());
        }
        if ok {
            let _ = files.write_all(format!("{line}\n").as_bytes());
        }
    }
}

/// Standard output (a terminal while developing; nowhere in a GUI build).
pub struct StdoutSink;

impl LineSink for StdoutSink {
    fn write_line(&self, _: log::Level, line: &str) {
        let _ = writeln!(std::io::stdout().lock(), "{line}");
    }
}

/// Fields of a span, kept in its extensions.
struct SpanFields(Map<String, Value>);

#[derive(Default)]
struct FieldVisitor {
    message: Option<String>,
    target: Option<String>,
    kv: Option<String>,
    fields: Map<String, Value>,
}

impl FieldVisitor {
    fn put(&mut self, field: &Field, value: Value) {
        match field.name() {
            "message" => self.message = Some(value.as_str().map(str::to_string).unwrap_or_else(|| value.to_string())),
            "log.target" => self.target = value.as_str().map(str::to_string),
            KV_FIELD => self.kv = value.as_str().map(str::to_string),
            // Location of bridged records: not properties.
            "log.module_path" | "log.file" | "log.line" => {}
            name => {
                self.fields.insert(name.to_string(), value);
            }
        }
    }
}

impl Visit for FieldVisitor {
    fn record_f64(&mut self, field: &Field, value: f64) {
        self.put(field, Value::from(value));
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.put(field, Value::from(value));
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.put(field, Value::from(value));
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.put(field, Value::from(value));
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.put(field, Value::from(value));
    }
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.put(field, Value::from(format!("{value:?}")));
    }
}

fn log_level(level: &tracing::Level) -> log::Level {
    match *level {
        tracing::Level::ERROR => log::Level::Error,
        tracing::Level::WARN => log::Level::Warn,
        tracing::Level::INFO => log::Level::Info,
        tracing::Level::DEBUG => log::Level::Debug,
        tracing::Level::TRACE => log::Level::Trace,
    }
}

/// Formats every event as CLEF (see the module docs) and writes it to its sinks.
pub struct ClefLayer {
    sinks: Vec<Box<dyn LineSink>>,
}

impl ClefLayer {
    pub fn new(sinks: Vec<Box<dyn LineSink>>) -> Self {
        ClefLayer { sinks }
    }
}

impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for ClefLayer {
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let mut v = FieldVisitor::default();
        attrs.record(&mut v);
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(SpanFields(v.fields));
        }
    }

    fn on_record(&self, id: &Id, values: &SpanRecord<'_>, ctx: Context<'_, S>) {
        let mut v = FieldVisitor::default();
        values.record(&mut v);
        if let Some(span) = ctx.span(id) {
            let mut ext = span.extensions_mut();
            if let Some(f) = ext.get_mut::<SpanFields>() {
                f.0.extend(v.fields);
            }
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let mut v = FieldVisitor::default();
        event.record(&mut v);
        let mut props = Map::new();
        if let Some(scope) = ctx.event_scope(event) {
            for span in scope.from_root() {
                if let Some(f) = span.extensions().get::<SpanFields>() {
                    props.extend(f.0.clone());
                }
            }
        }
        props.extend(std::mem::take(&mut v.fields));
        if let Some(Ok(Value::Object(kv))) = v.kv.as_deref().map(serde_json::from_str::<Value>) {
            props.extend(kv);
        }
        let meta = event.metadata();
        let level = log_level(meta.level());
        let line = crate::clef::format_event(
            Utc::now(),
            level,
            v.target.as_deref().unwrap_or(meta.target()),
            v.message.as_deref().unwrap_or_default(),
            props,
            &crate::clef::Enrichment::current(),
        );
        for sink in &self.sinks {
            sink.write_line(level, &line);
        }
    }
}

/// The `log` side's sink: forwards records to `tracing`. Records without key/values take
/// `tracing-log`'s path; others carry them as [`KV_FIELD`].
pub struct TracingBridge;

macro_rules! bridged {
    ($level:expr, $record:expr, $kv:expr) => {
        tracing::event!(target: "log", $level, { log.target = $record.target(), log.kv = $kv }, "{}", $record.args())
    };
}

impl log::Log for TracingBridge {
    fn enabled(&self, _: &log::Metadata) -> bool {
        // `log::max_level` and the subscriber's level filter decide.
        true
    }

    fn log(&self, record: &log::Record) {
        thread_local! {
            static INSIDE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
        }
        // Before a subscriber exists, `tracing` (with its `log` feature) would hand the event back to
        // `log`, and so to this bridge: there is nowhere to forward to yet. Never recurse either.
        if !tracing::dispatcher::has_been_set() || INSIDE.with(|i| i.replace(true)) {
            return;
        }
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                INSIDE.with(|i| i.set(false));
            }
        }
        let _reset = Reset;
        let kv = crate::clef::key_values(record.key_values());
        if kv.is_empty() {
            let _ = tracing_log::format_trace(record);
            return;
        }
        let kv = Value::Object(kv).to_string();
        let kv = kv.as_str();
        match record.level() {
            log::Level::Error => bridged!(tracing::Level::ERROR, record, kv),
            log::Level::Warn => bridged!(tracing::Level::WARN, record, kv),
            log::Level::Info => bridged!(tracing::Level::INFO, record, kv),
            log::Level::Debug => bridged!(tracing::Level::DEBUG, record, kv),
            log::Level::Trace => bridged!(tracing::Level::TRACE, record, kv),
        }
    }

    fn flush(&self) {}
}

/// The subscriber: a reloadable level filter and the CLEF layer over `sinks`.
fn subscriber(level: LogLevel, sinks: Vec<Box<dyn LineSink>>) -> (impl Subscriber + Send + Sync, reload::Handle<LevelFilter, Registry>) {
    let (filter, handle) = reload::Layer::new(tracing_level(level));
    (Registry::default().with(filter).with(ClefLayer::new(sinks)), handle)
}

/// Installs IMT's logging (see the module docs) and the log plugin (for the webview's `log` command,
/// which logs through the `log` facade). Logging to files fails soft: without a log folder IMT logs to
/// stdout only.
pub fn init<R: tauri::Runtime>(app: &tauri::AppHandle<R>, log_dir: Option<&Path>, limits: &LogLimits) -> tauri::Result<()> {
    app.plugin(tauri_plugin_log::Builder::new().skip_logger().build())?;
    let mut sinks: Vec<Box<dyn LineSink>> = vec![Box::new(StdoutSink)];
    let mut file_error = None;
    match log_dir.map(|d| FileSink::new(d, limits)) {
        Some(Ok(files)) => sinks.insert(0, Box::new(files)),
        Some(Err(e)) => file_error = Some(e.to_string()),
        None => file_error = Some("there is no log folder".into()),
    }
    let (subscriber, handle) = subscriber(limits.level, sinks);
    if tracing::subscriber::set_global_default(subscriber).is_ok() {
        let _ = LEVEL.set(handle);
    }
    let _ = log::set_boxed_logger(Box::new(ActivityLogger::new(Box::new(TracingBridge))));
    set_level(limits.level);
    if let Some(e) = file_error {
        log::warn!("IMT's log file is unavailable ({e}); logging to standard output only");
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    /// Lines written, for tests.
    #[derive(Clone, Default)]
    pub struct Lines(pub std::sync::Arc<Mutex<Vec<String>>>);

    impl LineSink for Lines {
        fn write_line(&self, _: log::Level, line: &str) {
            crate::sync::lock(&self.0).push(line.to_string());
        }
    }

    /// A subscriber like the real one over `sinks`, at Trace.
    pub fn test_subscriber(sinks: Vec<Box<dyn LineSink>>) -> impl Subscriber + Send + Sync {
        subscriber(LogLevel::Trace, sinks).0
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use log::Log;

    fn parse(line: &str) -> Map<String, Value> {
        serde_json::from_str::<Value>(line).unwrap().as_object().unwrap().clone()
    }

    #[test]
    fn spans_and_bridged_records_become_clef_properties() {
        let lines = Lines::default();
        tracing::subscriber::with_default(test_subscriber(vec![Box::new(lines.clone())]), || {
            let span = tracing::info_span!("ipc", Window = "w-1", OperationId = "op-7", Command = "run_query");
            let _e = span.enter();
            tracing::warn!(Rows = 3u64, "native {}", "event");
            // A `log` record with key/values, as the activity logger forwards it.
            let kvs: &[(&str, log::kv::Value)] = &[
                ("Sources", log::kv::Value::from(6u64)),
                ("SourceContext", log::kv::Value::from("IMT.Activity")),
            ];
            TracingBridge.log(
                &log::Record::builder()
                    .level(log::Level::Info)
                    .target("imt_app_lib::insights")
                    .key_values(&kvs)
                    .args(format_args!("Workspace opened"))
                    .build(),
            );
            // A dependency's record without key/values (tracing-log's path).
            TracingBridge.log(
                &log::Record::builder()
                    .level(log::Level::Error)
                    .target("hyper::client")
                    .args(format_args!("boom"))
                    .build(),
            );
            span.record("Command", "renamed");
            tracing::debug!("after record");
        });
        let got: Vec<_> = lines.0.lock().unwrap().iter().map(|l| parse(l)).collect();
        assert_eq!(got.len(), 4, "{got:?}");
        let native = &got[0];
        assert_eq!(
            (native["@l"].as_str(), native["@m"].as_str()),
            (Some("Warning"), Some("native event"))
        );
        assert_eq!(
            (native["Window"].as_str(), native["OperationId"].as_str()),
            (Some("w-1"), Some("op-7"))
        );
        assert_eq!(native["Rows"], 3);
        let bridged = &got[1];
        assert!(!bridged.contains_key("@l"), "Information omits @l");
        assert_eq!(bridged["@m"], "Workspace opened");
        assert_eq!(bridged["Sources"], 6, "key/values keep their type");
        assert_eq!(bridged["SourceContext"], "IMT.Activity");
        assert_eq!(bridged["Command"], "run_query", "span fields reach bridged records too");
        assert!(!bridged.contains_key("log.kv") && !bridged.contains_key("log.target"));
        let dep = &got[2];
        assert_eq!(
            (dep["@l"].as_str(), dep["SourceContext"].as_str(), dep["@m"].as_str()),
            (Some("Error"), Some("hyper::client"), Some("boom"))
        );
        assert!(!dep.contains_key("log.file") && !dep.contains_key("log.line"));
        assert_eq!(got[3]["Command"], "renamed");
        assert_eq!(got[3]["@l"], "Debug");
    }

    /// The log folder of a process-wide subscriber like the app's (installed once).
    fn global_log_dir() -> &'static Path {
        static DIR: OnceLock<PathBuf> = OnceLock::new();
        DIR.get_or_init(|| {
            let dir = std::env::temp_dir().join(format!("imt-app-test-global-log-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            let files = FileSink::new(&dir, &LogLimits::default()).unwrap();
            tracing::subscriber::set_global_default(test_subscriber(vec![Box::new(files)])).expect("no other global subscriber in tests");
            dir
        })
    }

    #[test]
    fn file_contents_and_query_text_never_reach_the_log_file_in_normal_mode() {
        use crate::activity::OP_HEADER;
        use tauri::ipc::{CallbackFn, InvokeBody};
        use tauri::test::{get_ipc_response, INVOKE_KEY};
        use tauri::webview::InvokeRequest;

        let _g = crate::activity::test_capture();
        crate::activity::set_config(crate::activity::ActivitySettings::default());
        let log_dir = global_log_dir();
        let log_file = active_log_file(log_dir);
        let before = std::fs::metadata(&log_file).map(|m| m.len()).unwrap_or(0) as usize;

        let data = crate::test_support::TempDir::new("logging-privacy");
        let file = data.0.join("app.clef");
        std::fs::write(
            &file,
            "{\"@t\":\"2026-10-14T10:00:00Z\",\"@l\":\"Error\",\"@mt\":\"payment for contents-secret-7f3a failed\",\"Card\":\"contents-secret-7f3a\"}
",
        )
        .unwrap();
        let app = crate::test_support::test_app_with_commands(None);
        crate::paths::allow_under(app.handle(), crate::windows::MAIN, &data.0);
        let w = crate::test_support::window(&app, crate::windows::MAIN);
        let call = |cmd: &str, body: Value| {
            let mut headers = tauri::http::HeaderMap::new();
            headers.insert(OP_HEADER, "op-privacy".parse().unwrap());
            get_ipc_response(
                &w,
                InvokeRequest {
                    cmd: cmd.into(),
                    callback: CallbackFn(0),
                    error: CallbackFn(1),
                    url: crate::test_support::ipc_url(),
                    body: InvokeBody::Json(body),
                    headers,
                    invoke_key: INVOKE_KEY.to_string(),
                },
            )
        };
        assert!(call("open_paths", serde_json::json!({ "paths": [file.to_string_lossy()] })).is_ok());
        let query = |ql: &str| serde_json::json!({ "request": { "query": { "text": "query-secret-91b2", "ql": ql } } });
        assert!(call("run_query", query("message contains \"query-secret-91b2\" | stats count()")).is_ok());
        // A failing query: its error (which quotes the query) goes to the UI, not to the log.
        assert!(call("run_query", query("message ~ \"(query-secret-91b2\"")).is_ok());
        assert!(call(
            "query_page",
            serde_json::json!({ "request": { "query": { "text": "query-secret-91b2" }, "offset": 0, "limit": 10, "descending": false } })
        )
        .is_ok());
        log::logger().flush();

        // Async commands log when their task ends, which can be after the reply: wait (bounded) for their lines.
        let read = || {
            let text = std::fs::read(&log_file).unwrap_or_default();
            String::from_utf8_lossy(&text[before.min(text.len())..]).into_owned()
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut written = read();
        while !written.contains("IPC run_query from main") && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(50));
            log::logger().flush();
            written = read();
        }
        assert!(
            written.contains("IPC run_query from main"),
            "the commands were logged to the file: {written}"
        );
        assert!(written.contains("\"OperationId\":\"op-privacy\""));
        for secret in ["contents-secret-7f3a", "query-secret-91b2"] {
            let leaked: Vec<&str> = written.lines().filter(|l| l.contains(secret)).collect();
            assert!(leaked.is_empty(), "{secret} leaked: {leaked:?}");
        }
    }

    #[test]
    fn levels_map_and_filter() {
        let all = [LogLevel::Error, LogLevel::Warn, LogLevel::Info, LogLevel::Debug, LogLevel::Trace];
        assert_eq!(
            all.map(tracing_level),
            [
                LevelFilter::ERROR,
                LevelFilter::WARN,
                LevelFilter::INFO,
                LevelFilter::DEBUG,
                LevelFilter::TRACE
            ]
        );
        for l in [
            tracing::Level::ERROR,
            tracing::Level::WARN,
            tracing::Level::INFO,
            tracing::Level::DEBUG,
            tracing::Level::TRACE,
        ] {
            assert_eq!(log_level(&l).as_str().to_ascii_uppercase(), l.as_str());
        }
        let lines = Lines::default();
        let (sub, handle) = subscriber(LogLevel::Warn, vec![Box::new(lines.clone())]);
        tracing::subscriber::with_default(sub, || {
            tracing::info!("hidden");
            tracing::warn!("shown");
            handle.modify(|f| *f = tracing_level(LogLevel::Info)).unwrap();
            tracing::info!("now shown");
        });
        let msgs: Vec<String> = lines
            .0
            .lock()
            .unwrap()
            .iter()
            .map(|l| parse(l)["@m"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(msgs, ["shown", "now shown"]);
    }

    #[test]
    fn daily_files_with_a_size_limit_errors_always_pass() {
        let dir = crate::test_support::TempDir::new("logging-files");
        let limits = LogLimits {
            max_file_mb: 1,
            keep_files: 2,
            ..LogLimits::default()
        };
        let sink = FileSink::new(&dir.0, &limits).unwrap();
        let big = "x".repeat(600 * 1024);
        sink.write_line(log::Level::Info, &big);
        sink.write_line(log::Level::Info, &big);
        sink.write_line(log::Level::Info, &big);
        sink.write_line(log::Level::Error, "an error");
        let text = std::fs::read_to_string(active_log_file(&dir.0)).unwrap();
        assert_eq!(text.matches(&big).count(), 1);
        assert_eq!(text.matches("daily limit of 1 MB").count(), 1, "the notice is written once");
        assert!(text.ends_with("an error\n"));
        // A new day starts a new budget.
        let tomorrow = Utc::now().date_naive().succ_opt().unwrap();
        assert_eq!(sink.admit(log::Level::Info, 10, tomorrow), (true, None));
        assert!(active_log_file(&dir.0).file_name().unwrap().to_string_lossy().starts_with("imt.20"));
        // Without a usable folder, the file sink cannot be created (init falls back to stdout).
        std::fs::write(dir.0.join("blocker"), "a file, not a folder").unwrap();
        assert!(FileSink::new(&dir.0.join("blocker").join("logs"), &limits).is_err());
        StdoutSink.write_line(log::Level::Info, "{}");
    }
}
