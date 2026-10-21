//! JSON lines (one object per line) or a whole-file JSON array. Handles
//! Serilog CLEF, Serilog JsonFormatter, rendered compact JSON and most other
//! JSON loggers through the key lists in `[parse.json]`.

use super::{first_lines, ImportCtx, Imported, Importer, MAX_LINE_LEN};
use crate::model::{Level, Record};
use crate::profile::JsonCfg;
use crate::template;
use crate::time::{epoch_from_number, TimestampParser};
use anyhow::{Context, Result};
use serde_json::{Map, Value};

pub struct JsonImporter;

impl Importer for JsonImporter {
    fn id(&self) -> &'static str {
        "json"
    }

    fn sniff(&self, ctx: &ImportCtx, head: &str) -> u8 {
        let Some(first) = first_lines(head, 1).next() else { return 0 };
        let first = first.trim_start();
        if first.starts_with('{') && serde_json::from_str::<Map<String, Value>>(first).is_ok() {
            // JSON lines mixed with text events: below the text importer (70), which reads both (ADR 0020).
            if super::lines::has_text_events(ctx.profile, head) {
                65
            } else {
                95
            }
        } else if (first.starts_with('[') || first.starts_with('{')) && starts_with_json(head) {
            // Pretty-printed array or object (possibly cut off at the end of the head).
            60
        } else {
            0
        }
    }

    fn import(&self, ctx: &ImportCtx, content: &str) -> Result<Imported> {
        let cfg = &ctx.profile.profile.parse.json;
        let ts = &ctx.profile.timestamps;
        let default_level = Level::parse(&cfg.default_level).unwrap_or(Level::Information);
        let mut out = Imported::default();
        let trimmed = content.trim_start();

        if trimmed.starts_with('[') {
            let items: Vec<Value> = serde_json::from_str(trimmed).context("file starts with '[' but is not a JSON array")?;
            for (i, item) in items.into_iter().enumerate() {
                match item {
                    Value::Object(obj) => out.records.push(to_record(obj, cfg, ts, default_level, i as u32 + 1)),
                    _ => out.skipped += 1,
                }
            }
            return Ok(out);
        }

        // JSON lines; tolerate a pretty-printed single object as well.
        if !first_lines(content, 1)
            .next()
            .is_some_and(|l| serde_json::from_str::<Value>(l.trim()).is_ok())
        {
            if let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(trimmed) {
                out.records.push(to_record(obj, cfg, ts, default_level, 1));
                return Ok(out);
            }
        }
        json_lines(ctx, content, &mut out);
        Ok(out)
    }

    fn import_tail(&self, ctx: &ImportCtx, head: &str, tail: &str) -> Option<Result<Imported>> {
        // Only JSON lines can grow by appending; an array or a pretty-printed
        // object is re-imported whole.
        let first = first_lines(head, 1).next()?.trim();
        if !first.starts_with('{') || serde_json::from_str::<Value>(first).is_err() {
            return None;
        }
        let mut out = Imported::default();
        json_lines(ctx, tail, &mut out);
        Some(Ok(out))
    }

    fn multiline(&self) -> bool {
        false
    }
}

/// Whether `head` (the start of a file) begins with a JSON value, possibly cut off where the head
/// ends. Text such as `[Sun Dec 04 04:47:44 2005] [error] ...` or `[2026-10-14 10:00:00] INFO` starts
/// with a bracket too but is not JSON.
pub(crate) fn starts_with_json(head: &str) -> bool {
    match serde_json::Deserializer::from_str(head.trim_start()).into_iter::<Value>().next() {
        Some(Err(e)) => e.is_eof(),
        other => other.is_some(),
    }
}

/// JSON lines. An object may also span several lines (pretty-printed objects one after another, as
/// `journalctl -o json-pretty` writes them, or Elasticsearch's JSON logs with the stack trace on the
/// lines below); such an object is at most [`MAX_LINE_LEN`] bytes. Looking ahead for the end of such
/// objects reads at most about twice the content in all, so broken files stay linear.
fn json_lines(ctx: &ImportCtx, content: &str, out: &mut Imported) {
    let cfg = &ctx.profile.profile.parse.json;
    let ts = &ctx.profile.timestamps;
    let default_level = Level::parse(&cfg.default_level).unwrap_or(Level::Information);
    let mut lines = content.lines().enumerate();
    let mut budget = content.len().saturating_mul(2).saturating_add(MAX_LINE_LEN);
    while let Some((i, line)) = lines.next() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line.len() > MAX_LINE_LEN {
            out.skipped += 1;
            continue;
        }
        let value = match serde_json::from_str::<Value>(line) {
            Err(e) if e.is_eof() && line.starts_with('{') => multiline_object(line, &mut lines, &mut budget),
            other => other.ok(),
        };
        match value {
            Some(Value::Object(obj)) => out.records.push(to_record(obj, cfg, ts, default_level, i as u32 + 1)),
            _ => out.skipped += 1,
        }
    }
}

/// The object that starts with `first` and continues on the next lines, which are consumed when
/// it is complete and valid. When it is not (broken, cut off, or larger than [`MAX_LINE_LEN`]),
/// nothing is consumed and `None` is returned, so the next lines are read on their own. Every byte
/// looked at is taken from `budget`; once it is spent, objects no longer span lines.
fn multiline_object(first: &str, lines: &mut std::iter::Enumerate<std::str::Lines<'_>>, budget: &mut usize) -> Option<Value> {
    let mut depth = Depth::default();
    depth.feed(first);
    let mut buf = first.to_string();
    let mut ahead = lines.clone();
    while let Some((_, line)) = ahead.next() {
        if buf.len() + line.len() >= MAX_LINE_LEN || line.len() >= *budget {
            return None;
        }
        *budget -= line.len() + 1;
        buf.push('\n');
        buf.push_str(line);
        if depth.feed(line) {
            let value = serde_json::from_str::<Value>(&buf).ok()?;
            *lines = ahead;
            return Some(value);
        }
    }
    None
}

/// Nesting depth of JSON text fed line by line, so a multi-line object is parsed once, when it closes.
#[derive(Default)]
pub(crate) struct Depth {
    depth: usize,
    in_string: bool,
    escaped: bool,
}

impl Depth {
    /// Feeds one line; true when the outermost object or array closed on it.
    pub(crate) fn feed(&mut self, text: &str) -> bool {
        let mut closed = false;
        for b in text.bytes() {
            if self.in_string {
                match b {
                    _ if self.escaped => self.escaped = false,
                    b'\\' => self.escaped = true,
                    b'"' => self.in_string = false,
                    _ => {}
                }
                continue;
            }
            match b {
                b'"' => self.in_string = true,
                b'{' | b'[' => self.depth += 1,
                b'}' | b']' => {
                    self.depth = self.depth.saturating_sub(1);
                    closed |= self.depth == 0;
                }
                _ => {}
            }
        }
        closed
    }
}

pub(crate) fn to_record(mut obj: Map<String, Value>, cfg: &JsonCfg, ts: &TimestampParser, default_level: Level, line: u32) -> Record {
    let mut rec = Record::new(line);

    if let Some(v) = take_first(&mut obj, &cfg.timestamp) {
        rec.ts = match &v {
            Value::String(s) => ts.parse(s),
            Value::Number(n) => n.as_f64().and_then(epoch_from_number),
            _ => None,
        };
    }
    rec.level = match take_first(&mut obj, &cfg.level) {
        Some(Value::String(s)) => Level::parse(&s).unwrap_or(default_level),
        Some(Value::Number(n)) => Level::from_number(n.as_f64().unwrap_or(30.0)),
        _ => default_level,
    };
    let message = take_first(&mut obj, &cfg.message);
    rec.template = take_first(&mut obj, &cfg.template).map(into_text);
    rec.exception = take_first(&mut obj, &cfg.exception).map(|v| match v {
        Value::String(s) => s,
        // A stack trace as one string per frame (Elasticsearch's "stacktrace").
        Value::Array(frames) if !frames.is_empty() && frames.iter().all(Value::is_string) => {
            frames.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n")
        }
        other => serde_json::to_string_pretty(&other).unwrap_or_default(),
    });

    // CLEF / Serilog bookkeeping keys.
    obj.remove("@r");
    obj.remove("Renderings");
    // The fields: these first, then the properties objects' entries, then the other keys (renamed, nested
    // objects flattened), a later key replacing the value of an earlier one in its place.
    let mut front: Vec<(String, Value)> = Vec::new();
    for (from, to) in [("@i", "EventId"), ("@tr", "TraceId"), ("@sp", "SpanId")] {
        if let Some(v) = obj.remove(from) {
            front.push((to.to_string(), v));
        }
    }
    for key in &cfg.properties {
        if let Some(Value::Object(props)) = obj.remove(key) {
            front.extend(props);
        }
    }
    rec.fields = fields_of(front, obj);

    rec.message = match message {
        Some(v) => into_text(v),
        None => rec
            .template
            .as_deref()
            .map(|t| template::render(t, &rec.fields))
            .unwrap_or_default(),
    };
    rec
}

/// The fields of an event: `front` (in order), then the keys of `rest`, as inserting them one by one into
/// a new map would give them. Most events (no nested objects, no `@@` keys, no key twice) keep `rest`'s
/// map as it is, so the fields are not hashed into a second map.
fn fields_of(front: Vec<(String, Value)>, rest: Map<String, Value>) -> Map<String, Value> {
    let plain = rest.iter().all(|(k, v)| !k.starts_with("@@") && !flattens(v))
        && front
            .iter()
            .enumerate()
            .all(|(i, (k, _))| !rest.contains_key(k) && front[..i].iter().all(|(e, _)| e != k));
    if plain {
        let mut fields = rest;
        for (i, (k, v)) in front.into_iter().enumerate() {
            fields.shift_insert(i, k, v);
        }
        return fields;
    }
    let mut fields = Map::with_capacity(front.len() + rest.len());
    for (k, v) in front {
        fields.insert(k, v);
    }
    for (k, v) in rest {
        let k = match k.strip_prefix("@@") {
            Some(rest) => format!("@{rest}"),
            None => k,
        };
        flatten_into(&mut fields, k, v);
    }
    fields
}

/// Whether [`flatten_into`] spreads the value over several fields.
fn flattens(v: &Value) -> bool {
    matches!(v, Value::Object(inner) if flattens_inner(inner))
}

fn flattens_inner(inner: &Map<String, Value>) -> bool {
    !inner.is_empty() && inner.len() <= 32
}

/// The text of a value: a string as it is (moved, not copied), anything else as JSON.
fn into_text(v: Value) -> String {
    match v {
        Value::String(s) => s,
        other => other.to_string(),
    }
}

/// Removes and returns the first key present. Dotted keys address nested
/// objects ("log.level"), unless the dotted name itself is a key.
fn take_first(obj: &mut Map<String, Value>, keys: &[String]) -> Option<Value> {
    for key in keys {
        if let Some(v) = obj.remove(key) {
            if !v.is_null() {
                return Some(v);
            }
        }
        if let Some((head, rest)) = key.split_once('.') {
            if let Some(Value::Object(inner)) = obj.get_mut(head) {
                if let Some(v) = take_first(inner, &[rest.to_string()]) {
                    if inner.is_empty() {
                        obj.remove(head);
                    }
                    return Some(v);
                }
            }
        }
    }
    None
}

fn flatten_into(fields: &mut Map<String, Value>, key: String, v: Value) {
    match v {
        Value::Object(inner) if flattens_inner(&inner) => {
            for (k, v) in inner {
                flatten_into(fields, format!("{key}.{k}"), v);
            }
        }
        other => {
            fields.insert(key, other);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{CompiledProfile, Profile};
    use std::path::Path;

    fn import(content: &str) -> Imported {
        let p = CompiledProfile::compile(Profile::default_profile()).unwrap();
        let ctx = ImportCtx {
            profile: &p,
            path: Path::new("x.clef"),
            file_date: None,
        };
        JsonImporter.import(&ctx, content).unwrap()
    }

    #[test]
    fn clef() {
        let out = import(concat!(
            r#"{"@t":"2026-10-14T10:41:22.318Z","@mt":"Order {OrderId} failed","@l":"Error","@x":"System.Exception: boom","OrderId":88213,"MachineName":"WEB02"}"#,
            "\n",
            r#"{"@t":"2026-10-14T10:41:23.000Z","@mt":"Hello {Name}","Name":"x","@i":"a1b2"}"#,
            "\nnot json\n"
        ));
        assert_eq!(out.records.len(), 2);
        assert_eq!(out.skipped, 1);
        let r = &out.records[0];
        assert_eq!(r.level, Level::Error);
        assert_eq!(r.message, "Order 88213 failed");
        assert_eq!(r.exception.as_deref(), Some("System.Exception: boom"));
        assert_eq!(r.fields["MachineName"], "WEB02");
        assert_eq!(out.records[1].level, Level::Information);
        assert_eq!(out.records[1].fields["EventId"], "a1b2");
    }

    /// CLEF with a rendered `@m` (no template), a .NET round-trip timestamp (7-digit fraction, offset), an exception
    /// with an inner exception and a nested `ExceptionDetail` object (Serilog.Exceptions). Invented values.
    #[test]
    fn clef_with_rendered_message_and_exception_detail() {
        let out = import(concat!(
            r#"{"@t":"2021-04-03T10:20:30.1234567+08:00","@m":"Error during Sync.Fetch","@i":"0badc0de","@l":"Error","@x":"Acme.Sync.FetchException: Client timed out(5000ms)!  ---> System.TimeoutException: The operation has timed out.\r\n   --- End of inner exception stack trace ---","Arguments":"queue A","ProcessId":4100,"ProcessName":"Acme.Agent","ThreadId":9,"PrivateBytes":"120.50MB","MachineName":"NODE7","ExceptionDetail":{"HResult":-2146233088,"Message":"Client timed out(5000ms)! ","InnerException":{"Type":"System.TimeoutException","HResult":-2146233083},"Type":"Acme.Sync.FetchException"}}"#,
            "\n"
        ));
        assert_eq!(out.records.len(), 1);
        let r = &out.records[0];
        // 10:20:30.1234567 at +08:00 is 02:20:30.123 UTC (milliseconds kept).
        let want = chrono::NaiveDate::from_ymd_opt(2021, 4, 3)
            .unwrap()
            .and_hms_milli_opt(2, 20, 30, 123)
            .unwrap()
            .and_utc()
            .timestamp_millis();
        assert_eq!(r.ts, Some(want));
        assert_eq!(r.level, Level::Error);
        assert_eq!(r.message, "Error during Sync.Fetch");
        assert!(r
            .exception
            .as_deref()
            .is_some_and(|x| x.contains("System.TimeoutException") && x.contains('\n')));
        assert_eq!(r.fields["ProcessId"], 4100);
        assert_eq!(r.fields["MachineName"], "NODE7");
        assert_eq!(r.fields["EventId"], "0badc0de");
        assert_eq!(r.fields["ExceptionDetail.InnerException.Type"], "System.TimeoutException");
        assert_eq!(r.fields["ExceptionDetail.HResult"], -2146233088);
    }

    #[test]
    fn serilog_json_formatter_and_generic() {
        let out = import(concat!(
            r#"{"Timestamp":"2026-10-14T10:41:22.318+00:00","Level":"Warning","MessageTemplate":"Slow {Ms}","RenderedMessage":"Slow 5000","Properties":{"Ms":5000,"SourceContext":"Pay"}}"#,
            "\n",
            r#"{"time":1792000000123,"level":50,"msg":"pino style","req":{"id":7}}"#,
            "\n",
            r#"{"@timestamp":"2026-10-14T10:00:00Z","log":{"level":"warn"},"message":"ecs"}"#,
        ));
        assert_eq!(out.records[0].message, "Slow 5000");
        assert_eq!(out.records[0].fields["SourceContext"], "Pay");
        assert_eq!(out.records[1].level, Level::Error);
        assert_eq!(out.records[1].ts, Some(1_792_000_000_123));
        assert_eq!(out.records[1].fields["req.id"], 7);
        assert_eq!(out.records[2].level, Level::Warning);
        assert!(!out.records[2].fields.contains_key("log"));
    }

    #[test]
    fn json_array() {
        let out = import("[\n {\"ts\": \"2026-10-14 10:00:00\", \"msg\": \"a\"},\n {\"ts\": \"2026-10-14 10:00:01\", \"msg\": \"b\"}\n]");
        assert_eq!(out.records.len(), 2);
        assert_eq!(out.records[1].message, "b");
    }

    #[test]
    fn sniffing() {
        let p = CompiledProfile::compile(Profile::default_profile()).unwrap();
        let ctx = ImportCtx {
            profile: &p,
            path: Path::new("x"),
            file_date: None,
        };
        assert_eq!(JsonImporter.sniff(&ctx, "{\"a\":1}\n"), 95);
        assert_eq!(JsonImporter.sniff(&ctx, "[\n"), 60);
        assert_eq!(JsonImporter.sniff(&ctx, "{\n  \"a\": 1\n}"), 60);
        assert_eq!(JsonImporter.sniff(&ctx, "hello"), 0);
        assert_eq!(JsonImporter.sniff(&ctx, ""), 0);
        // Cut off where the head ends: still JSON.
        assert_eq!(JsonImporter.sniff(&ctx, "[\n {\"msg\": \"a\"},\n {\"msg\": \"b"), 60);
        // Text logs that start with a bracket are left to the text importers.
        for text in [
            "[Sun Dec 04 04:47:44 2005] [error] mod_jk child workerEnv in error state 6\n",
            "[2026-10-14T10:00:00,123][INFO ][o.e.n.Node] [node-1] started\n",
            "[2026-10-14 10:00:00,123] WARN [Controller id=1] slow (kafka.controller.KafkaController)\n",
            "[10:00:00 INF] Started\n",
            "{ not json }\n",
        ] {
            assert_eq!(JsonImporter.sniff(&ctx, text), 0, "{text}");
        }
    }

    #[test]
    fn objects_spanning_lines() {
        // Pretty-printed objects one after another (journalctl -o json-pretty).
        let out = import("{\n  \"time\": \"2026-10-14T10:00:00Z\",\n  \"msg\": \"a\"\n}\n{\n  \"msg\": \"b {\\\"x\\\": [\"\n}\n");
        assert_eq!(out.records.len(), 2);
        assert_eq!((out.records[1].message.as_str(), out.records[1].line), ("b {\"x\": [", 5));
        // Elasticsearch JSON logs: the stack trace array continues on the next lines.
        let out = import(concat!(
            "{\"timestamp\": \"2026-10-14T10:00:00,123+0100\", \"level\": \"ERROR\", \"message\": \"boom\",\n",
            "\"stacktrace\": [\"java.lang.Exception: boom\",\n",
            "\"at org.example.Foo.bar(Foo.java:12) [app.jar]\"] }\n",
            "{\"timestamp\": \"2026-10-14T10:00:01,000+0100\", \"level\": \"INFO\", \"message\": \"next\"}\n",
        ));
        assert_eq!((out.records.len(), out.skipped), (2, 0));
        assert_eq!(out.records[0].level, Level::Error);
        assert_eq!(
            out.records[0].exception.as_deref(),
            Some("java.lang.Exception: boom\nat org.example.Foo.bar(Foo.java:12) [app.jar]")
        );
        assert_eq!(out.records[1].line, 4);
        // A broken object consumes only its first line; the lines after it are read on their own.
        let out = import("{\"msg\": \"a\",\n\"x\": nope }\n{\"msg\": \"c\"}\n");
        assert_eq!((out.records.len(), out.skipped), (1, 2));
        assert_eq!(out.records[0].message, "c");
        // Cut off at the end of the file, or larger than a line may be.
        let out = import("{\"msg\": \"ok\"}\n{\"msg\": \"a\",\n\"more\": 1,\n");
        assert_eq!((out.records.len(), out.skipped), (1, 2));
        let big = format!("{{\"msg\": \"a\",\n\"x\": \"{}\"\n}}\n", "y".repeat(MAX_LINE_LEN));
        let out = import(&format!("{{\"msg\": \"ok\"}}\n{big}"));
        assert_eq!(out.records.len(), 1);
        // Many unclosed objects: the look-ahead budget runs out and lines are read one by one.
        let unclosed = "{\"a\": [\n".repeat(3);
        let mut lines = unclosed.lines().enumerate();
        let mut budget = 5;
        assert!(multiline_object("{", &mut lines, &mut budget).is_none());
        assert_eq!((budget, lines.next().map(|(i, _)| i)), (5, Some(0)));
        let out = import(&"{\"a\": [\n".repeat(2000));
        assert_eq!((out.records.len(), out.skipped), (0, 2000));
        // Empty or mixed arrays as the exception stay JSON text.
        let out = import("{\"msg\": \"m\", \"stacktrace\": []}\n{\"msg\": \"m\", \"stacktrace\": [\"a\", 1]}\n");
        assert_eq!(out.records[0].exception.as_deref(), Some("[]"));
        assert!(out.records[1].exception.as_deref().unwrap().contains('1'));
    }

    #[test]
    fn odd_shapes() {
        // Pretty-printed single object.
        let out = import("{\n  \"time\": \"2026-10-14T10:00:00Z\",\n  \"message\": \"hi\"\n}");
        assert_eq!(out.records.len(), 1);
        assert_eq!(out.records[0].message, "hi");
        // Arrays skip non-objects.
        let out = import("[1, {\"msg\": \"x\"}, \"y\"]");
        assert_eq!((out.records.len(), out.skipped), (1, 2));
        // Blank lines, odd value types, escaped keys, nested exception objects, nulls.
        let out = import(concat!(
            r#"{"time": true, "level": ["x"], "msg": {"text": 1}, "error": {"type": "E", "stack_trace": "s"}, "@@weird": 1, "@t": null, "deep": {"a": {"b": 2}}, "empty": {}}"#,
            "\n\n",
        ));
        let r = &out.records[0];
        assert_eq!(r.ts, None);
        assert_eq!(r.level, Level::Information);
        assert_eq!(r.message, "{\"text\":1}");
        assert_eq!(r.exception.as_deref(), Some("s"));
        assert_eq!(r.fields["error.type"], "E");
        assert_eq!(r.fields["@weird"], 1);
        assert_eq!(r.fields["deep.a.b"], 2);
        assert_eq!(r.fields["empty"], serde_json::json!({}));
        // A whole object as the exception.
        let out = import(r#"{"msg": "m", "exception": {"type": "E"}}"#);
        assert!(out.records[0].exception.as_deref().unwrap().contains("\"type\""));
        // A nested object without the dotted key stays a field; a partly used one keeps the rest.
        let out = import(concat!(
            r#"{"msg": "m", "log": {"logger": "app"}}"#,
            "\n",
            r#"{"msg": "m", "log": {"level": "error", "logger": "app"}}"#,
        ));
        assert_eq!(out.records[0].level, Level::Information);
        assert_eq!(out.records[0].fields["log.logger"], "app");
        assert_eq!(out.records[1].level, Level::Error);
        assert_eq!(out.records[1].fields["log.logger"], "app");
        assert!(!out.records[1].fields.contains_key("log.level"));
        // No message and no template.
        let out = import("{\"time\": \"2026-10-14T10:00:00Z\"}");
        assert_eq!(out.records[0].message, "");
        assert!(JsonImporter
            .import(
                &ImportCtx {
                    profile: &CompiledProfile::compile(Profile::default_profile()).unwrap(),
                    path: Path::new("x"),
                    file_date: None
                },
                "[not json"
            )
            .is_err());
    }

    /// The fields as the importer built them before [`fields_of`]: inserted one by one into a new map.
    fn reference_fields(front: Vec<(String, Value)>, rest: Map<String, Value>) -> Map<String, Value> {
        let mut fields = Map::new();
        for (k, v) in front {
            fields.insert(k, v);
        }
        for (k, v) in rest {
            let k = match k.strip_prefix("@@") {
                Some(rest) => format!("@{rest}"),
                None => k,
            };
            flatten_into(&mut fields, k, v);
        }
        fields
    }

    fn pairs(m: &Map<String, Value>) -> Vec<(String, Value)> {
        m.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    fn same_fields(front: &[(&str, Value)], rest: &str) {
        let front: Vec<(String, Value)> = front.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
        let rest: Map<String, Value> = serde_json::from_str(rest).unwrap();
        let want = reference_fields(front.clone(), rest.clone());
        assert_eq!(pairs(&fields_of(front, rest)), pairs(&want), "keys, values and order");
    }

    #[test]
    fn fields_keep_the_order_and_values_of_inserting_them_one_by_one() {
        use serde_json::json;
        // The usual event: the map is kept, the front goes first.
        same_fields(
            &[("EventId", json!(7)), ("TraceId", json!("t"))],
            r#"{"b": 1, "a": "x", "c": null}"#,
        );
        same_fields(&[], r#"{"b": 1, "a": [1, {"x": 2}], "e": {}}"#);
        // A front key again in the rest, or twice in the front: the later value in the first place.
        same_fields(&[("EventId", json!(7))], r#"{"b": 1, "EventId": 8}"#);
        same_fields(&[("EventId", json!(7)), ("X", json!(1)), ("EventId", json!(9))], r#"{"b": 1}"#);
        // Renamed and flattened keys.
        same_fields(&[("EventId", json!(1))], r#"{"@@x": 1, "b": 2}"#);
        same_fields(&[], r#"{"o": {"k": 1, "j": {"z": 2}}, "o.k": 3, "p": 4}"#);
        let big: Map<String, Value> = (0..33).map(|i| (format!("k{i}"), json!(i))).collect();
        same_fields(&[], &serde_json::to_string(&json!({"big": big, "z": 1})).unwrap());
    }

    #[test]
    fn fields_match_the_one_by_one_insertion_on_every_json_sample() {
        let samples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples");
        let cfg = CompiledProfile::compile(Profile::default_profile())
            .unwrap()
            .profile
            .parse
            .json
            .clone();
        let mut files = vec![samples];
        let mut lines = 0;
        while let Some(p) = files.pop() {
            if p.is_dir() {
                files.extend(std::fs::read_dir(&p).unwrap().map(|e| e.unwrap().path()));
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&p) else { continue };
            for line in text.lines() {
                let Ok(mut obj) = serde_json::from_str::<Map<String, Value>>(line.trim()) else {
                    continue;
                };
                obj.remove("@r");
                obj.remove("Renderings");
                let mut front = Vec::new();
                for (from, to) in [("@i", "EventId"), ("@tr", "TraceId"), ("@sp", "SpanId")] {
                    if let Some(v) = obj.remove(from) {
                        front.push((to.to_string(), v));
                    }
                }
                for key in &cfg.properties {
                    if let Some(Value::Object(props)) = obj.remove(key) {
                        front.extend(props);
                    }
                }
                let want = reference_fields(front.clone(), obj.clone());
                assert_eq!(pairs(&fields_of(front, obj)), pairs(&want), "{}: {line}", p.display());
                lines += 1;
            }
        }
        assert!(lines > 1000, "the samples have JSON events ({lines})");
    }

    #[test]
    fn skips_oversized_lines() {
        let huge = format!("{{\"msg\": \"{}\"}}", "x".repeat(MAX_LINE_LEN));
        let out = import(&format!("{{\"msg\": \"ok\"}}\n{huge}\n"));
        assert_eq!((out.records.len(), out.skipped), (1, 1));
    }
}
