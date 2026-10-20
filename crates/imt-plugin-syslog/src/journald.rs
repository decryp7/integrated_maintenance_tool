//! `journalctl -o json` (one object per line).

use super::{facility_name, severity_level, severity_name};
use anyhow::Result;
use imt_core::importers::{ImportCtx, Imported, Importer, MAX_LINE_LEN};
use imt_core::model::{auto_value, Level, Record};
use serde_json::{Map, Value};

pub struct JournaldImporter;

impl Importer for JournaldImporter {
    fn id(&self) -> &'static str {
        "journald"
    }

    /// The journal fields mapped to the syslog names: host, identifier and process id.
    fn identity_fields(&self) -> Vec<String> {
        ["Host", "App", "ProcId"].map(String::from).to_vec()
    }

    fn sniff(&self, _ctx: &ImportCtx, head: &str) -> u8 {
        let first = head.trim_start_matches('\u{feff}').lines().map(str::trim).find(|l| !l.is_empty());
        match first {
            Some(l) if l.starts_with('{') && l.contains("\"__REALTIME_TIMESTAMP\"") => 97,
            _ => 0,
        }
    }

    fn import(&self, _ctx: &ImportCtx, content: &str) -> Result<Imported> {
        let mut out = Imported::default();
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
                Ok(Value::Object(obj)) => out.records.push(to_record(obj, i as u32 + 1)),
                _ => out.skipped += 1,
            }
        }
        Ok(out)
    }
}

/// Journal binary fields are JSON arrays of byte values.
fn decode_bytes(items: &[Value]) -> Option<String> {
    let bytes: Option<Vec<u8>> = items.iter().map(|v| v.as_u64().and_then(|n| u8::try_from(n).ok())).collect();
    bytes.filter(|b| !b.is_empty()).map(|b| String::from_utf8_lossy(&b).into_owned())
}

fn text_of(v: Value) -> String {
    match v {
        Value::String(s) => s,
        Value::Null => String::new(),
        Value::Array(items) => decode_bytes(&items).unwrap_or_else(|| Value::Array(items).to_string()),
        other => other.to_string(),
    }
}

fn to_record(mut obj: Map<String, Value>, line: u32) -> Record {
    let mut rec = Record::new(line);
    rec.ts = obj
        .remove("__REALTIME_TIMESTAMP")
        .and_then(|v| match v {
            Value::String(s) => s.trim().parse::<i64>().ok(),
            Value::Number(n) => n.as_i64(),
            _ => None,
        })
        .map(|us| us / 1000)
        .filter(|ms| *ms > 0);
    rec.message = obj.remove("MESSAGE").map(text_of).unwrap_or_default();
    let priority = obj.get("PRIORITY").and_then(|v| match v {
        Value::String(s) => s.trim().parse::<u8>().ok(),
        other => other.as_u64().and_then(|n| u8::try_from(n).ok()),
    });
    if let Some(p) = priority.filter(|p| *p <= 7) {
        rec.level = severity_level(p);
        rec.fields.insert("Severity".into(), Value::from(severity_name(p)));
    } else {
        rec.level = Level::Information;
    }
    let facility = obj
        .get("SYSLOG_FACILITY")
        .and_then(Value::as_str)
        .and_then(|s| s.trim().parse::<u8>().ok())
        .and_then(facility_name);
    if let Some(f) = facility {
        rec.fields.insert("Facility".into(), Value::from(f));
    }
    for (journal, field) in [("_HOSTNAME", "Host"), ("SYSLOG_IDENTIFIER", "App"), ("_PID", "ProcId")] {
        if let Some(v) = obj.get(journal) {
            rec.fields.insert(field.into(), field_value(v.clone()));
        }
    }
    for (k, v) in obj {
        if k.starts_with("__") {
            continue;
        }
        rec.fields.insert(k, field_value(v));
    }
    rec
}

fn field_value(v: Value) -> Value {
    match v {
        Value::String(s) => auto_value(&s),
        Value::Array(items) => match decode_bytes(&items) {
            Some(s) => Value::String(s),
            None => Value::Array(items.into_iter().map(field_value).collect()),
        },
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_core::profile::{CompiledProfile, Profile};
    use serde_json::json;
    use std::path::Path;

    const JOURNAL: &str = include_str!("../tests/fixtures/journal.json");

    fn with_ctx<T>(f: impl FnOnce(&ImportCtx) -> T) -> T {
        let p = CompiledProfile::compile(Profile::default_profile()).unwrap();
        f(&ImportCtx {
            profile: &p,
            path: Path::new("journal.json"),
            file_date: None,
        })
    }

    fn import(content: &str) -> Imported {
        with_ctx(|ctx| JournaldImporter.import(ctx, content).unwrap())
    }

    fn sniff(content: &str) -> u8 {
        with_ctx(|ctx| JournaldImporter.sniff(ctx, content))
    }

    #[test]
    fn imports_journal_entries() {
        let out = import(JOURNAL);
        assert_eq!(out.skipped, 1);
        assert_eq!(out.records.len(), 4);
        let r = &out.records[0];
        assert_eq!(r.ts, Some(1_792_000_000_123));
        assert_eq!(r.level, Level::Information);
        assert_eq!(r.message, "Started OrderService.");
        assert_eq!(r.fields["Host"], "web01");
        assert_eq!(r.fields["App"], "systemd");
        assert_eq!(r.fields["ProcId"], 1);
        assert_eq!(r.fields["Severity"], "info");
        assert_eq!(r.fields["Facility"], "daemon");
        assert_eq!(r.fields["_SYSTEMD_UNIT"], "orders.service");
        assert_eq!(r.fields["PRIORITY"], 6);
        assert!(!r.fields.contains_key("__CURSOR"));
        assert!(!r.fields.contains_key("__MONOTONIC_TIMESTAMP"));
        assert!(!r.fields.contains_key("__REALTIME_TIMESTAMP"));
        assert!(r.fields.contains_key("_BOOT_ID"));
        let err = &out.records[1];
        assert_eq!(err.level, Level::Error);
        // MESSAGE as a byte array
        assert_eq!(err.message, "caf\u{e9} failed\n  detail");
        let emerg = &out.records[2];
        assert_eq!(emerg.level, Level::Fatal);
        assert_eq!(emerg.fields["BLOB"], json!(["x", 1]));
        assert_eq!(emerg.fields["BYTES"], "hi");
        // numeric timestamp, no priority, odd values
        let odd = &out.records[3];
        assert_eq!(odd.ts, Some(1_792_000_009_000));
        assert_eq!(odd.level, Level::Information);
        assert!(!odd.fields.contains_key("Severity"));
        assert!(!odd.fields.contains_key("Facility"));
        assert_eq!(odd.message, "[1,2,300]");
    }

    #[test]
    fn handles_missing_and_invalid_values() {
        let out = import(concat!(
            "{\"__REALTIME_TIMESTAMP\":\"abc\",\"MESSAGE\":null,\"PRIORITY\":\"9\",\"SYSLOG_FACILITY\":\"99\"}\n",
            "{\"__REALTIME_TIMESTAMP\":true,\"PRIORITY\":3}\n",
            "{\"__REALTIME_TIMESTAMP\":\"0\",\"PRIORITY\":-1,\"MESSAGE\":[]}\n",
            "{\"MESSAGE\":42,\"PRIORITY\":\"7\"}\n",
        ));
        assert_eq!(out.records.len(), 4);
        assert!(out.records.iter().take(3).all(|r| r.ts.is_none()));
        assert_eq!(out.records[0].message, "");
        assert_eq!(out.records[0].level, Level::Information);
        assert_eq!(out.records[1].level, Level::Error);
        assert_eq!(out.records[2].message, "[]");
        assert_eq!(out.records[3].message, "42");
        assert_eq!(out.records[3].level, Level::Debug);
    }

    #[test]
    fn sniffs_journal_json_only() {
        assert_eq!(sniff(JOURNAL), 97);
        assert_eq!(sniff("\u{feff}\n{\"__REALTIME_TIMESTAMP\":\"1\"}"), 97);
        assert_eq!(sniff(r#"{"@t":"2026-10-14T09:00:00Z","@mt":"hello"}"#), 0);
        assert_eq!(sniff("Oct 14 10:00:00 host app: __REALTIME_TIMESTAMP"), 0);
        assert_eq!(sniff(""), 0);
    }

    #[test]
    fn malformed_input_never_panics() {
        let out = import("garbage\n[1]\n\n{\n42\n");
        assert_eq!(out.skipped, 4);
        let big = format!("{{\"MESSAGE\":\"{}\"}}\n", "x".repeat(MAX_LINE_LEN));
        assert_eq!(import(&big).skipped, 1);
        for i in (0..JOURNAL.len()).step_by(7) {
            if JOURNAL.is_char_boundary(i) {
                let _ = import(&JOURNAL[..i]);
                let _ = sniff(&JOURNAL[..i]);
            }
        }
    }
}
