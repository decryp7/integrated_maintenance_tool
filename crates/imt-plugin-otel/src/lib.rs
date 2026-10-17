//! IMT plugin that imports OpenTelemetry OTLP/JSON exports (as written by the
//! OpenTelemetry Collector `file` exporter or SDK file exporters).
//!
//! * Logs (`resourceLogs`): one record per `logRecord`.
//! * Traces (`resourceSpans`): one record per span plus one per span event
//!   (for example `exception`), linked by `TraceId` / `SpanId`.
//!
//! Files may hold one export per line (JSON lines) or a single, possibly
//! pretty-printed, document. Resource and log/span attributes become fields
//! under their own keys (`service.name`, `host.name`, ...).
//!
//! Register it with:
//! ```
//! let mut registry = imt_core::PluginRegistry::new();
//! registry.install(&imt_plugin_otel::OtelPlugin).unwrap();
//! ```

use anyhow::Result;
use imt_core::importers::{ImportCtx, Imported, Importer, MAX_LINE_LEN};
use imt_core::model::{Fields, Level, Record};
use imt_core::time::epoch_from_number;
use imt_core::{Plugin, PluginRegistry};
use serde_json::{Map, Value};

pub struct OtelPlugin;

impl Plugin for OtelPlugin {
    fn id(&self) -> &'static str {
        "net.decryptology.otel"
    }

    fn name(&self) -> &'static str {
        "OpenTelemetry"
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    /// Stand-alone: no dependencies, no services.
    fn manifest(&self) -> imt_core::plugin::Manifest {
        imt_core::plugin::Manifest::new(self.id(), self.version())
    }

    fn register(&self, registry: &mut PluginRegistry) -> Result<()> {
        registry.add_importer(Box::new(OtlpImporter));
        Ok(())
    }
}

/// Imports OTLP/JSON logs and traces.
pub struct OtlpImporter;

const ROOT_KEYS: [&str; 2] = ["resourceLogs", "resourceSpans"];

impl Importer for OtlpImporter {
    fn id(&self) -> &'static str {
        "otlp"
    }

    fn sniff(&self, _ctx: &ImportCtx, head: &str) -> u8 {
        let head = head.trim_start_matches('\u{feff}').trim_start();
        if !head.starts_with('{') {
            return 0;
        }
        let first = head.lines().next().unwrap_or("").trim();
        if let Ok(obj) = serde_json::from_str::<Map<String, Value>>(first) {
            return if ROOT_KEYS.iter().any(|k| obj.contains_key(*k)) { 99 } else { 0 };
        }
        // Pretty-printed or truncated head: look at the first key only.
        let key = head[1..].trim_start();
        let has_root_key = ROOT_KEYS.iter().any(|k| {
            key.strip_prefix('"')
                .and_then(|r| r.strip_prefix(k))
                .and_then(|r| r.strip_prefix('"'))
                .is_some_and(|r| r.trim_start().starts_with(':'))
        });
        if has_root_key {
            99
        } else {
            0
        }
    }

    fn import(&self, _ctx: &ImportCtx, content: &str) -> Result<Imported> {
        let mut out = Imported::default();
        let first = content.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
        if serde_json::from_str::<Value>(first.trim_start_matches('\u{feff}').trim()).is_err() {
            // A pretty-printed document (or an array of exports).
            match serde_json::from_str::<Value>(content.trim_start_matches('\u{feff}').trim()) {
                Ok(doc) => handle_value(&doc, 1, &mut out),
                Err(_) => out.skipped += content.lines().filter(|l| !l.trim().is_empty()).count(),
            }
            return Ok(out);
        }
        for (i, line) in content.lines().enumerate() {
            let line = line.trim_start_matches('\u{feff}').trim();
            if line.is_empty() {
                continue;
            }
            if line.len() > MAX_LINE_LEN {
                out.skipped += 1;
                continue;
            }
            match serde_json::from_str::<Value>(line) {
                Ok(doc) => handle_value(&doc, i as u32 + 1, &mut out),
                Err(_) => out.skipped += 1,
            }
        }
        Ok(out)
    }
}

fn handle_value(doc: &Value, line: u32, out: &mut Imported) {
    match doc {
        Value::Array(items) => items.iter().for_each(|d| handle_value(d, line, out)),
        Value::Object(_) => {
            let before = out.records.len();
            logs(doc, line, &mut out.records);
            spans(doc, line, &mut out.records);
            if out.records.len() == before && !ROOT_KEYS.iter().any(|k| doc.get(*k).is_some()) {
                out.skipped += 1;
            }
        }
        _ => out.skipped += 1,
    }
}

fn items<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// Converts an OTLP `AnyValue` to plain JSON.
fn any_value(v: &Value) -> Value {
    let Some(o) = v.as_object() else { return Value::Null };
    for key in ["stringValue", "doubleValue", "boolValue", "bytesValue"] {
        if let Some(x) = o.get(key) {
            return x.clone();
        }
    }
    if let Some(i) = o.get("intValue") {
        return match i {
            Value::String(s) => s.parse::<i64>().map(Value::from).unwrap_or_else(|_| i.clone()),
            other => other.clone(),
        };
    }
    if let Some(a) = o.get("arrayValue") {
        return Value::Array(items(a, "values").iter().map(any_value).collect());
    }
    if let Some(k) = o.get("kvlistValue") {
        return Value::Object(attributes(items(k, "values")).into_iter().collect());
    }
    Value::Null
}

/// `[{"key": "a", "value": AnyValue}]` -> `[("a", json)]`.
fn attributes(list: &[Value]) -> Vec<(String, Value)> {
    list.iter()
        .filter_map(|kv| {
            Some((
                kv.get("key")?.as_str()?.to_string(),
                kv.get("value").map(any_value).unwrap_or(Value::Null),
            ))
        })
        .collect()
}

fn value_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Removes the `exception.*` attributes and joins them as
/// `"Type: message\nstacktrace"`.
fn take_exception(attrs: &mut Vec<(String, Value)>) -> Option<String> {
    let mut parts: [Option<String>; 3] = [None, None, None];
    attrs.retain(|(k, v)| {
        let slot = match k.as_str() {
            "exception.type" => 0,
            "exception.message" => 1,
            "exception.stacktrace" => 2,
            _ => return true,
        };
        parts[slot] = Some(value_text(v));
        false
    });
    let [kind, message, stack] = parts;
    let head = match (kind, message) {
        (Some(k), Some(m)) => Some(format!("{k}: {m}")),
        (k, m) => k.or(m),
    };
    match (head, stack) {
        (Some(h), Some(s)) => Some(format!("{h}\n{s}")),
        (h, s) => h.or(s),
    }
}

/// Nanoseconds since the epoch from a JSON string or number.
fn nanos(v: Option<&Value>) -> Option<u64> {
    match v? {
        Value::String(s) => s.trim().parse().ok(),
        Value::Number(n) => n.as_u64().or_else(|| n.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64)),
        _ => None,
    }
    .filter(|n| *n > 0)
}

fn epoch_ms(n: Option<u64>) -> Option<i64> {
    n.and_then(|n| epoch_from_number(n as f64))
}

fn resource_fields(resource_owner: &Value) -> Fields {
    let mut fields = Fields::new();
    let list = resource_owner.pointer("/resource/attributes").and_then(Value::as_array);
    for (k, v) in attributes(list.map(Vec::as_slice).unwrap_or(&[])) {
        fields.insert(k, v);
    }
    fields
}

fn scope_name(scope_owner: &Value) -> Option<&str> {
    scope_owner.pointer("/scope/name").and_then(Value::as_str).filter(|s| !s.is_empty())
}

fn base_fields(resource: &Fields, scope: &Value) -> Fields {
    let mut fields = resource.clone();
    if let Some(name) = scope_name(scope) {
        fields.insert("ScopeName".into(), Value::from(name));
    }
    fields
}

fn severity_level(number: Option<&Value>, severity_text: Option<&str>) -> Level {
    let from_number = match number {
        Some(Value::Number(n)) => n.as_i64(),
        Some(Value::String(s)) => s.trim().parse().ok(),
        _ => None,
    };
    let from_name = || {
        let name = number?.as_str()?.strip_prefix("SEVERITY_NUMBER_")?;
        Level::parse(name.trim_end_matches(|c: char| c.is_ascii_digit()))
    };
    match from_number {
        Some(1..=4) => Level::Verbose,
        Some(5..=8) => Level::Debug,
        Some(9..=12) => Level::Information,
        Some(13..=16) => Level::Warning,
        Some(17..=20) => Level::Error,
        Some(21..=24) => Level::Fatal,
        _ => from_name()
            .or_else(|| severity_text.and_then(Level::parse))
            .unwrap_or(Level::Information),
    }
}

fn logs(doc: &Value, line: u32, records: &mut Vec<Record>) {
    for rl in items(doc, "resourceLogs") {
        let resource = resource_fields(rl);
        for sl in items(rl, "scopeLogs") {
            let base = base_fields(&resource, sl);
            for lr in items(sl, "logRecords") {
                records.push(log_record(lr, &base, line));
            }
        }
    }
}

fn log_record(lr: &Value, base: &Fields, line: u32) -> Record {
    let mut rec = Record::new(line);
    rec.fields = base.clone();
    rec.ts = epoch_ms(nanos(lr.get("timeUnixNano")).or_else(|| nanos(lr.get("observedTimeUnixNano"))));
    let severity_text = text(lr, "severityText");
    rec.level = severity_level(lr.get("severityNumber"), severity_text);
    rec.message = lr.get("body").map(|b| value_text(&any_value(b))).unwrap_or_default();
    let mut attrs = attributes(items(lr, "attributes"));
    rec.exception = take_exception(&mut attrs);
    for (k, v) in attrs {
        rec.fields.insert(k, v);
    }
    if let Some(t) = severity_text {
        rec.fields.insert("SeverityText".into(), Value::from(t));
    }
    for (src, dst) in [("traceId", "TraceId"), ("spanId", "SpanId")] {
        if let Some(id) = text(lr, src) {
            rec.fields.insert(dst.into(), Value::from(id));
        }
    }
    rec
}

fn spans(doc: &Value, line: u32, records: &mut Vec<Record>) {
    for rs in items(doc, "resourceSpans") {
        let resource = resource_fields(rs);
        for ss in items(rs, "scopeSpans") {
            let base = base_fields(&resource, ss);
            for span in items(ss, "spans") {
                span_records(span, &base, line, records);
            }
        }
    }
}

fn span_kind(v: Option<&Value>) -> &'static str {
    match v {
        Some(Value::Number(n)) => match n.as_i64() {
            Some(1) => "Internal",
            Some(2) => "Server",
            Some(3) => "Client",
            Some(4) => "Producer",
            Some(5) => "Consumer",
            _ => "Unspecified",
        },
        Some(Value::String(s)) => match s.trim_start_matches("SPAN_KIND_").to_ascii_uppercase().as_str() {
            "INTERNAL" => "Internal",
            "SERVER" => "Server",
            "CLIENT" => "Client",
            "PRODUCER" => "Producer",
            "CONSUMER" => "Consumer",
            _ => "Unspecified",
        },
        _ => "Unspecified",
    }
}

/// 0 = unset, 1 = ok, 2 = error; accepts the numeric and enum-name forms.
fn status_code(v: Option<&Value>) -> i64 {
    match v {
        Some(Value::Number(n)) => n.as_i64().unwrap_or(0),
        Some(Value::String(s)) => match s.trim_start_matches("STATUS_CODE_").to_ascii_uppercase().as_str() {
            "OK" | "1" => 1,
            "ERROR" | "2" => 2,
            _ => 0,
        },
        _ => 0,
    }
}

fn span_records(span: &Value, base: &Fields, line: u32, records: &mut Vec<Record>) {
    let name = text(span, "name").unwrap_or("(unnamed span)");
    let start = nanos(span.get("startTimeUnixNano"));
    let end = nanos(span.get("endTimeUnixNano"));
    let code = status_code(span.pointer("/status/code"));
    let mut ids = Fields::new();
    for (src, dst) in [("traceId", "TraceId"), ("spanId", "SpanId"), ("parentSpanId", "ParentSpanId")] {
        if let Some(id) = text(span, src) {
            ids.insert(dst.into(), Value::from(id));
        }
    }

    let mut rec = Record::new(line);
    rec.ts = epoch_ms(start);
    rec.fields = base.clone();
    rec.fields.extend(ids.clone());
    rec.fields.insert("SpanName".into(), Value::from(name));
    rec.fields.insert("SpanKind".into(), Value::from(span_kind(span.get("kind"))));
    rec.message = name.to_string();
    if let (Some(s), Some(e)) = (start, end) {
        let ms = (e.saturating_sub(s) as f64 / 1e6 * 1000.0).round() / 1000.0;
        rec.fields.insert("DurationMs".into(), Value::from(ms));
        rec.message = format!("{name} ({ms:.1} ms)");
    }
    rec.fields.insert(
        "StatusCode".into(),
        Value::from(match code {
            1 => "Ok",
            2 => "Error",
            _ => "Unset",
        }),
    );
    if let Some(m) = text(&span["status"], "message") {
        rec.fields.insert("StatusMessage".into(), Value::from(m));
    }
    if code == 2 {
        rec.level = Level::Error;
    }
    for (k, v) in attributes(items(span, "attributes")) {
        rec.fields.insert(k, v);
    }
    records.push(rec);

    for ev in items(span, "events") {
        let mut r = Record::new(line);
        r.ts = epoch_ms(nanos(ev.get("timeUnixNano")));
        r.fields = base.clone();
        r.fields.extend(ids.clone());
        r.fields.remove("ParentSpanId");
        r.fields.insert("SpanName".into(), Value::from(name));
        let event_name = text(ev, "name").unwrap_or("event");
        r.fields.insert("EventName".into(), Value::from(event_name));
        r.message = event_name.to_string();
        let mut attrs = attributes(items(ev, "attributes"));
        r.exception = take_exception(&mut attrs);
        if r.exception.is_some() || event_name == "exception" {
            r.level = Level::Error;
        }
        for (k, v) in attrs {
            r.fields.insert(k, v);
        }
        records.push(r);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_core::profile::{CompiledProfile, Profile};
    use serde_json::json;
    use std::path::Path;

    const LOGS: &str = include_str!("../tests/fixtures/logs.jsonl");
    const TRACES: &str = include_str!("../tests/fixtures/traces.json");

    fn with_ctx<T>(f: impl FnOnce(&ImportCtx) -> T) -> T {
        let p = CompiledProfile::compile(Profile::default_profile()).unwrap();
        let ctx = ImportCtx {
            profile: &p,
            path: Path::new("x.json"),
            file_date: None,
        };
        f(&ctx)
    }

    fn import(content: &str) -> Imported {
        with_ctx(|ctx| OtlpImporter.import(ctx, content).unwrap())
    }

    fn sniff(content: &str) -> u8 {
        with_ctx(|ctx| OtlpImporter.sniff(ctx, content))
    }

    #[test]
    fn imports_log_records() {
        let out = import(LOGS);
        assert_eq!(out.skipped, 1);
        assert_eq!(out.records.len(), 5);
        let r = &out.records[0];
        assert_eq!(r.line, 1);
        assert_eq!(r.ts, Some(1_792_000_000_123));
        assert_eq!(r.level, Level::Information);
        assert_eq!(r.message, "Order 42 accepted");
        assert_eq!(r.fields["service.name"], "orders");
        assert_eq!(r.fields["host.name"], "web01");
        assert_eq!(r.fields["ScopeName"], "Orders.Api");
        assert_eq!(r.fields["http.status_code"], 200);
        assert_eq!(r.fields["ok"], true);
        assert_eq!(r.fields["ratio"], 0.5);
        assert_eq!(r.fields["TraceId"], "5b8efff798038103d269b633813fc60c");
        assert_eq!(r.fields["SpanId"], "eee19b7ec3c1b174");
        assert_eq!(r.fields["SeverityText"], "INFO");
        let warn = &out.records[1];
        assert_eq!(warn.level, Level::Warning);
        assert_eq!(warn.ts, Some(1_792_000_001_000));
        assert!(!warn.fields.contains_key("TraceId"));
        // severity from text only, observed time fallback, kvlist body
        let err = &out.records[2];
        assert_eq!(err.level, Level::Error);
        assert_eq!(err.ts, Some(1_792_000_002_500));
        assert_eq!(err.message, r#"{"user":"bob","n":3,"tags":["a",1,true,null]}"#);
        assert_eq!(
            err.exception.as_deref(),
            Some("System.TimeoutException: timed out\n   at Orders.Api.Pay()")
        );
        assert!(!err.fields.contains_key("exception.type"));
        // numeric severity 23 -> Fatal, numeric time, int body
        let fatal = &out.records[3];
        assert_eq!(fatal.level, Level::Fatal);
        assert_eq!(fatal.ts, Some(1_792_000_003_000));
        assert_eq!(fatal.line, 2);
        assert_eq!(fatal.message, "5");
        assert_eq!(out.records[4].level, Level::Verbose);
        assert_eq!(out.records[4].message, "");
        assert_eq!(out.records[4].fields["bytes"], "AAEC");
    }

    #[test]
    fn imports_pretty_traces_with_events() {
        let out = import(TRACES);
        assert_eq!(out.records.len(), 4);
        let root = &out.records[0];
        assert_eq!(root.ts, Some(1_792_000_000_000));
        assert_eq!(root.message, "GET /orders/{id} (250.0 ms)");
        assert_eq!(root.level, Level::Information);
        assert_eq!(root.fields["DurationMs"], 250.0);
        assert_eq!(root.fields["SpanKind"], "Server");
        assert_eq!(root.fields["StatusCode"], "Ok");
        assert!(!root.fields.contains_key("ParentSpanId"));
        assert_eq!(root.fields["http.method"], "GET");
        assert_eq!(root.fields["service.name"], "orders");
        let db = &out.records[1];
        assert_eq!(db.fields["ParentSpanId"], "00f067aa0ba902b7");
        assert_eq!(db.level, Level::Error);
        assert_eq!(db.fields["StatusCode"], "Error");
        assert_eq!(db.fields["StatusMessage"], "deadlock");
        assert_eq!(db.fields["SpanKind"], "Client");
        assert_eq!(db.fields["DurationMs"], 12.346);
        assert_eq!(db.message, "SELECT orders (12.3 ms)");
        let ex = &out.records[2];
        assert_eq!(ex.message, "exception");
        assert_eq!(ex.level, Level::Error);
        assert_eq!(ex.ts, Some(1_792_000_000_100));
        assert_eq!(ex.fields["TraceId"], db.fields["TraceId"]);
        assert_eq!(ex.fields["SpanId"], db.fields["SpanId"]);
        assert!(!ex.fields.contains_key("ParentSpanId"));
        assert_eq!(
            ex.exception.as_deref(),
            Some("Npgsql.PostgresException: deadlock detected\nat Db.Exec()")
        );
        let plain = &out.records[3];
        assert_eq!(plain.level, Level::Information);
        assert_eq!(plain.message, "retry");
        assert_eq!(plain.fields["attempt"], 2);
    }

    #[test]
    fn span_edge_cases() {
        let doc = json!({"resourceSpans":[{"scopeSpans":[{"spans":[
            {"startTimeUnixNano":"1792000000000000000","kind":"SPAN_KIND_PRODUCER","status":{"code":"STATUS_CODE_ERROR"}},
            {"name":"b","kind":9,"status":{"code":"STATUS_CODE_OK"},"events":[{"timeUnixNano":"1792000000000000000"}]},
            {"name":"c","kind":"SPAN_KIND_BOGUS","status":{"code":"junk"}},
            {"name":"d","kind":"consumer","status":{"code":1.5}},
            {"name":"e","kind":5,"startTimeUnixNano":10,"endTimeUnixNano":"5"},
            {"name":"f","kind":1,"status":{"code":2},"events":[{"name":"exception","attributes":[{"key":"exception.message","value":{"stringValue":"m"}}]},{"name":"e2","attributes":[{"key":"exception.stacktrace","value":{"stringValue":"s"}},{"key":"exception.type","value":{"stringValue":"T"}}]},{"name":"exception"}]},
            {"name":"g","kind":2,"status":{"code":"STATUS_CODE_UNSET"}},
            {"name":"h","kind":3,"status":{"code":"1"}},
            {"name":"i","kind":4}
        ]}]}]});
        let out = import(&doc.to_string());
        let kinds: Vec<_> = out.records.iter().map(|r| r.fields["SpanKind"].clone()).take(1).collect();
        assert_eq!(kinds, [json!("Producer")]);
        assert_eq!(out.records[0].message, "(unnamed span)");
        assert_eq!(out.records[0].level, Level::Error);
        assert_eq!(out.records[1].fields["StatusCode"], "Ok");
        assert_eq!(out.records[1].fields["SpanKind"], "Unspecified");
        assert_eq!(out.records[2].fields["EventName"], "event");
        assert_eq!(out.records[2].ts, Some(1_792_000_000_000));
        assert_eq!(out.records[3].fields["SpanKind"], "Unspecified");
        assert_eq!(out.records[3].fields["StatusCode"], "Unset");
        assert_eq!(out.records[4].fields["SpanKind"], "Consumer");
        assert_eq!(out.records[5].ts, None);
        assert_eq!(out.records[5].fields["DurationMs"], 0.0);
        assert_eq!(out.records[7].exception.as_deref(), Some("m"));
        assert_eq!(out.records[8].exception.as_deref(), Some("T\ns"));
        assert_eq!(out.records[9].level, Level::Error);
        assert_eq!(out.records[9].exception, None);
        assert_eq!(out.records[10].fields["StatusCode"], "Unset");
        assert_eq!(out.records[11].fields["StatusCode"], "Ok");
        assert_eq!(out.records[12].fields["StatusCode"], "Unset");
    }

    #[test]
    fn severity_and_values() {
        assert_eq!(severity_level(Some(&json!("17")), None), Level::Error);
        assert_eq!(severity_level(Some(&json!("SEVERITY_NUMBER_WARN2")), None), Level::Warning);
        assert_eq!(severity_level(Some(&json!(0)), Some("debug")), Level::Debug);
        assert_eq!(severity_level(Some(&json!(99)), Some("nonsense")), Level::Information);
        assert_eq!(severity_level(Some(&json!(true)), None), Level::Information);
        assert_eq!(severity_level(None, Some("CRITICAL")), Level::Fatal);
        assert_eq!(any_value(&json!({"intValue": 7})), 7);
        assert_eq!(any_value(&json!({"intValue": "x"})), "x");
        assert_eq!(any_value(&json!({"unknown": 1})), Value::Null);
        assert_eq!(any_value(&json!("raw")), Value::Null);
        assert_eq!(value_text(&Value::Null), "");
        assert_eq!(take_exception(&mut vec![]), None);
        assert_eq!(nanos(Some(&json!(1.5e18))), Some(1_500_000_000_000_000_000));
        assert_eq!(nanos(Some(&json!(-1.0))), None);
        assert_eq!(nanos(Some(&json!(true))), None);
        assert_eq!(nanos(Some(&json!("0"))), None);
        assert_eq!(nanos(None), None);
    }

    #[test]
    fn sniffs_otlp_only() {
        assert_eq!(sniff(LOGS), 99);
        assert_eq!(sniff(TRACES), 99);
        assert_eq!(sniff("\u{feff}  {\"resourceSpans\":[]}"), 99);
        assert_eq!(sniff("{\n  \"resourceLogs\"\n  : []"), 99);
        assert_eq!(sniff(r#"{"other":1,"resourceLogs":[]}"#), 99);
        assert_eq!(sniff(r#"{"@t":"2026-10-14T09:00:00Z","@mt":"resourceLogs"}"#), 0);
        assert_eq!(sniff("{\n  \"resourceLogsX\": 1"), 0);
        assert_eq!(sniff("{\n  \"message\": \"resourceLogs\""), 0);
        assert_eq!(sniff("[{\"resourceLogs\":[]}]"), 0);
        assert_eq!(sniff("2026-10-14 09:00:00 INFO resourceLogs"), 0);
        assert_eq!(sniff(""), 0);
    }

    #[test]
    fn does_not_steal_sample_files() {
        let samples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples");
        let mut checked = 0;
        let mut stack = vec![samples];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let Some(text) = std::fs::read(&path).ok().and_then(|b| imt_core::importers::decode(&b)) else {
                    continue;
                };
                let head: String = text.chars().take(8192).collect();
                assert_eq!(sniff(&head), 0, "{}", path.display());
                checked += 1;
            }
        }
        assert!(checked >= 6);
    }

    #[test]
    fn malformed_input_never_panics() {
        let out = import("{\"resourceLogs\":[{\"scopeLogs\":[{\"logRecords\":[{}]}]}]}\nnot json\n\n{\"foo\":1}\n42\n");
        assert_eq!(out.records.len(), 1);
        assert_eq!(out.records[0].ts, None);
        assert_eq!(out.skipped, 3);
        assert_eq!(import("garbage\nmore").skipped, 2);
        assert_eq!(import("").records.len(), 0);
        assert_eq!(import("[{\"resourceSpans\":[]}, 3, {\"x\":1}]").skipped, 2);
        let garbage = [
            "{", "[", "{\"resourceLogs\":", "{\"resourceLogs\":{}}", "{\"resourceLogs\":[null,1,\"a\"]}",
            "{\"resourceLogs\":[{\"resource\":5,\"scopeLogs\":[{\"scope\":[],\"logRecords\":[1,{\"body\":[],\"attributes\":[1,{\"key\":3}]}]}]}]}",
            "{\"resourceSpans\":[{\"scopeSpans\":[{\"spans\":[{\"events\":[3],\"attributes\":\"x\",\"status\":[]}]}]}]}",
            "\u{0}\u{1}\u{2}", "{\"resourceLogs\":[{\"scopeLogs\":[{\"logRecords\":[{\"timeUnixNano\":{},\"severityNumber\":[]}]}]}]}",
        ];
        for g in garbage {
            let _ = import(g);
            let _ = sniff(g);
            let _ = import(&format!("{g}\n{g}\n"));
        }
        for i in (0..LOGS.len()).step_by(13) {
            if LOGS.is_char_boundary(i) {
                let _ = import(&LOGS[..i]);
                let _ = sniff(&LOGS[..i]);
            }
        }
    }

    #[test]
    fn skips_oversized_lines() {
        let big = format!(
            "{{\"resourceLogs\":[],\"pad\":\"{}\"}}\n{{\"resourceLogs\":[]}}\n",
            "x".repeat(MAX_LINE_LEN)
        );
        assert_eq!(import(&big).skipped, 1);
    }

    #[test]
    fn plugin_registers_the_importer() {
        let mut registry = PluginRegistry::new();
        registry.install(&OtelPlugin).unwrap();
        let info = &registry.plugins()[0];
        assert_eq!(info.id, "net.decryptology.otel");
        assert_eq!(info.contributes, vec!["importer:otlp".to_string()]);
        assert_eq!(OtelPlugin.name(), "OpenTelemetry");
        assert_eq!(OtelPlugin.version(), env!("CARGO_PKG_VERSION"));
    }

    fn workspace() -> imt_core::Workspace {
        let mut registry = PluginRegistry::new();
        registry.install(&OtelPlugin).unwrap();
        imt_core::Workspace::with_registry(Profile::default_profile(), registry).unwrap()
    }

    #[test]
    fn opens_fixtures_through_a_workspace() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let mut ws = workspace();
        ws.open(&[dir.join("logs.jsonl"), dir.join("traces.json")]).unwrap();
        let s = ws.summary();
        assert_eq!(s.sources.len(), 2);
        assert!(s.sources.iter().all(|x| x.importer == "otlp"));
        assert!(s.plugins.iter().any(|p| p.id == "net.decryptology.otel"));
        assert_eq!(ws.records().len(), 9);
        assert!(ws
            .records()
            .iter()
            .any(|r| r.fields.get("service.name") == Some(&json!("orders")) && r.message == "Order 42 accepted"));
    }
}
