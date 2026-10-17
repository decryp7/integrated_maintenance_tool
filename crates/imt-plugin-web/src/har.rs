//! HAR (HTTP Archive) captures.

use crate::level_for_status;
use anyhow::{anyhow, Context, Result};
use chrono::DateTime;
use imt_core::importers::{ImportCtx, Imported, Importer};
use imt_core::model::{Level, Record};
use serde_json::Value;

pub struct HarImporter;

impl Importer for HarImporter {
    fn id(&self) -> &'static str {
        "har"
    }

    fn sniff(&self, _ctx: &ImportCtx, head: &str) -> u8 {
        let head = head.trim_start_matches('\u{feff}').trim_start();
        if head.starts_with('{') && head.contains("\"log\"") && head.contains("\"entries\"") {
            98
        } else {
            0
        }
    }

    fn import(&self, _ctx: &ImportCtx, content: &str) -> Result<Imported> {
        let doc: Value = serde_json::from_str(content.trim_start_matches('\u{feff}')).context("not valid JSON")?;
        let entries = doc
            .pointer("/log/entries")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("no log.entries array: not a HAR file"))?;
        let mut out = Imported::default();
        for (i, entry) in entries.iter().enumerate() {
            if entry.is_object() {
                out.records.push(record(i as u32 + 1, entry));
            } else {
                out.skipped += 1;
            }
        }
        Ok(out)
    }
}

/// Header + body size, ignoring the `-1` "unknown" markers.
fn sized(node: &Value) -> Option<i64> {
    let parts: Vec<i64> = ["headersSize", "bodySize"]
        .iter()
        .filter_map(|k| node.get(*k).and_then(Value::as_i64))
        .filter(|n| *n >= 0)
        .collect();
    (!parts.is_empty()).then(|| parts.iter().sum())
}

fn record(line: u32, e: &Value) -> Record {
    let mut r = Record::new(line);
    r.ts = e
        .get("startedDateTime")
        .and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.timestamp_millis());
    let (req, resp) = (e.get("request").unwrap_or(&Value::Null), e.get("response").unwrap_or(&Value::Null));
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    let url = req.get("url").and_then(Value::as_str).unwrap_or("");
    let status = resp.get("status").and_then(Value::as_i64);
    let mut put = |name: &str, v: Option<Value>| {
        if let Some(v) = v {
            r.fields.insert(name.into(), v);
        }
    };
    put("Method", Some(Value::from(method)));
    put("Url", Some(Value::from(url)));
    put("Status", status.map(Value::from));
    put(
        "StatusText",
        resp.get("statusText")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(Value::from),
    );
    put(
        "MimeType",
        resp.pointer("/content/mimeType")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(Value::from),
    );
    let time = e.get("time").filter(|t| t.as_f64().is_some_and(|n| n >= 0.0)).cloned();
    put("TimeMs", time.clone());
    put("RequestBytes", sized(req).map(Value::from));
    let response_bytes = resp
        .get("_transferSize")
        .and_then(Value::as_i64)
        .filter(|n| *n >= 0)
        .or_else(|| sized(resp));
    put("ResponseBytes", response_bytes.map(Value::from));
    put(
        "ServerIp",
        e.get("serverIPAddress")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(Value::from),
    );
    let failed = status == Some(0);
    r.level = if failed {
        Level::Error
    } else {
        status.map_or(Level::Information, level_for_status)
    };
    r.message = format!("{method} {url} \u{2192} ");
    match status {
        _ if failed => r.message.push_str("failed request"),
        Some(s) => r.message.push_str(&s.to_string()),
        None => r.message.push('?'),
    }
    if let Some(t) = time.and_then(|t| t.as_f64()) {
        r.message.push_str(&format!(" ({} ms)", (t * 10.0).round() / 10.0));
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{garbage, with_ctx};
    use serde_json::json;

    const HAR: &str = include_str!("../tests/fixtures/traffic.har");

    #[test]
    fn parses_entries() {
        let out = with_ctx(|c| {
            assert_eq!(HarImporter.sniff(c, HAR), 98);
            HarImporter.import(c, HAR).unwrap()
        });
        assert_eq!(HarImporter.id(), "har");
        assert_eq!((out.records.len(), out.skipped), (4, 1));
        let a = &out.records[0];
        assert_eq!(a.ts, Some(1_791_972_000_123));
        assert_eq!(a.message, "GET https://example.com/api/items?page=2 \u{2192} 200 (123.5 ms)");
        assert_eq!(a.fields["StatusText"], "OK");
        assert_eq!(a.fields["MimeType"], "application/json");
        assert_eq!(a.fields["TimeMs"], json!(123.5));
        assert_eq!(a.fields["RequestBytes"], 200);
        assert_eq!(a.fields["ResponseBytes"], 1174);
        assert_eq!(a.fields["ServerIp"], "93.184.216.34");
        let nf = &out.records[1];
        assert_eq!(nf.level, Level::Warning);
        assert!(!nf.fields.contains_key("RequestBytes") && !nf.fields.contains_key("ServerIp"));
        let failed = &out.records[2];
        assert_eq!(failed.level, Level::Error);
        assert_eq!(failed.message, "POST https://example.com/down \u{2192} failed request");
        assert!(!failed.fields.contains_key("TimeMs") && !failed.fields.contains_key("MimeType"));
        let bad = &out.records[3];
        assert_eq!((bad.level, bad.fields["ResponseBytes"].clone()), (Level::Error, json!(777)));
    }

    #[test]
    fn sniff_and_errors() {
        with_ctx(|c| {
            assert_eq!(HarImporter.sniff(c, "{\"log\":{\"entries\":[]}}"), 98);
            assert_eq!(HarImporter.sniff(c, "\u{feff}  {\n\"log\": {}, \"entries\""), 98);
            assert_eq!(HarImporter.sniff(c, "{\"log\": 1}"), 0);
            assert_eq!(HarImporter.sniff(c, "[\"log\", \"entries\"]"), 0);
            assert_eq!(HarImporter.sniff(c, "2026 \"log\" \"entries\""), 0);
            assert!(HarImporter.import(c, "{nope").is_err());
            assert!(HarImporter.import(c, "{\"log\": {}}").is_err());
            let out = HarImporter.import(c, "{\"log\":{\"entries\":[{}]}}").unwrap();
            let r = &out.records[0];
            assert_eq!(r.message, "  \u{2192} ?");
            assert!(r.ts.is_none());
        });
    }

    #[test]
    fn garbage_never_panics() {
        with_ctx(|c| {
            for g in garbage(
                &[
                    "{",
                    "}",
                    "\"log\"",
                    "\"entries\"",
                    ":",
                    "[",
                    "]",
                    ",",
                    "\"request\"",
                    "1",
                    "null",
                    "x",
                ],
                300,
            ) {
                let _ = HarImporter.sniff(c, &g);
                let _ = HarImporter.import(c, &g);
            }
        });
    }
}
