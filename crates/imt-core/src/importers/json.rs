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

    fn sniff(&self, _ctx: &ImportCtx, head: &str) -> u8 {
        let Some(first) = first_lines(head, 1).next() else { return 0 };
        let first = first.trim_start();
        if first.starts_with('{') && serde_json::from_str::<Map<String, Value>>(first).is_ok() {
            95
        } else if first.starts_with('[') || first.starts_with('{') {
            // Pretty-printed array or object; confirmed during import.
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
        for (i, line) in content.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if line.len() > MAX_LINE_LEN {
                out.skipped += 1;
                continue;
            }
            match serde_json::from_str::<Value>(line) {
                Ok(Value::Object(obj)) => out.records.push(to_record(obj, cfg, ts, default_level, i as u32 + 1)),
                _ => out.skipped += 1,
            }
        }
        Ok(out)
    }
}

fn to_record(mut obj: Map<String, Value>, cfg: &JsonCfg, ts: &TimestampParser, default_level: Level, line: u32) -> Record {
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
    rec.template = take_first(&mut obj, &cfg.template).map(|v| as_text(&v));
    rec.exception = take_first(&mut obj, &cfg.exception).map(|v| match v {
        Value::String(s) => s,
        other => serde_json::to_string_pretty(&other).unwrap_or_default(),
    });

    // CLEF / Serilog bookkeeping keys.
    obj.remove("@r");
    obj.remove("Renderings");
    for (from, to) in [("@i", "EventId"), ("@tr", "TraceId"), ("@sp", "SpanId")] {
        if let Some(v) = obj.remove(from) {
            rec.fields.insert(to.to_string(), v);
        }
    }

    for key in &cfg.properties {
        if let Some(Value::Object(props)) = obj.remove(key) {
            for (k, v) in props {
                rec.fields.insert(k, v);
            }
        }
    }
    for (k, v) in obj {
        let k = match k.strip_prefix("@@") {
            Some(rest) => format!("@{rest}"),
            None => k,
        };
        flatten_into(&mut rec.fields, k, v);
    }

    rec.message = match message {
        Some(v) => as_text(&v),
        None => rec
            .template
            .as_deref()
            .map(|t| template::render(t, &rec.fields))
            .unwrap_or_default(),
    };
    rec
}

fn as_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
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
        Value::Object(inner) if !inner.is_empty() && inner.len() <= 32 => {
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

    #[test]
    fn skips_oversized_lines() {
        let huge = format!("{{\"msg\": \"{}\"}}", "x".repeat(MAX_LINE_LEN));
        let out = import(&format!("{{\"msg\": \"ok\"}}\n{huge}\n"));
        assert_eq!((out.records.len(), out.skipped), (1, 1));
    }
}
