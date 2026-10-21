//! Windows event logs (`.evtx`).

use anyhow::{Context, Result};
use evtx::{EvtxParser, ParserSettings};
use imt_plugin_api::importer::{BinaryImporter, ImportCtx, Imported};
use imt_plugin_api::model::{Level, Record};
use serde_json::Value;

const MAGIC: &[u8] = b"ElfFile\0";
const MAX_MESSAGE: usize = 1000;

/// True when the bytes start with the EVTX file magic.
pub fn is_evtx(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}

pub struct EvtxImporter;

impl BinaryImporter for EvtxImporter {
    fn id(&self) -> &'static str {
        "evtx"
    }

    /// The computer and the process that logged the event.
    fn identity_fields(&self) -> Vec<String> {
        ["Computer", "ProcessId"].map(String::from).to_vec()
    }

    fn sniff(&self, _ctx: &ImportCtx, head: &[u8]) -> u8 {
        if is_evtx(head) {
            100
        } else {
            0
        }
    }

    fn import(&self, _ctx: &ImportCtx, bytes: &[u8]) -> Result<Imported> {
        let (records, skipped) = read_evtx_counted(bytes)?;
        Ok(Imported {
            records,
            skipped,
            ..Default::default()
        })
    }
}

/// Reads every event of an EVTX file. Unreadable events are dropped.
pub fn read_evtx(bytes: &[u8]) -> Result<Vec<Record>> {
    read_evtx_counted(bytes).map(|(r, _)| r)
}

fn read_evtx_counted(bytes: &[u8]) -> Result<(Vec<Record>, usize)> {
    if !is_evtx(bytes) {
        anyhow::bail!("not an EVTX event log (missing ElfFile signature)");
    }
    let settings = ParserSettings::new().validate_checksums(false);
    let mut parser = EvtxParser::from_buffer(bytes.to_vec())
        .context("unreadable EVTX header")?
        .with_configuration(settings);
    let mut records = Vec::new();
    let mut skipped = 0;
    for rec in parser.records_json_value() {
        match rec {
            Ok(r) => records.push(to_record(&r.data, r.event_record_id, records.len() as u32 + 1)),
            Err(_) => skipped += 1,
        }
    }
    Ok((records, skipped))
}

/// `#text` of a node such as `{"#attributes": {...}, "#text": 4624}`, or the node itself.
fn text_of(v: &Value) -> Option<&Value> {
    match v {
        Value::Object(o) => o.get("#text"),
        Value::Null => None,
        other => Some(other),
    }
}

fn attr<'a>(node: &'a Value, name: &str) -> Option<&'a Value> {
    node.get("#attributes")?.get(name)
}

fn show(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().map(show).collect::<Vec<_>>().join(","),
        other => other.to_string(),
    }
}

fn level_of(n: i64) -> Level {
    match n {
        1 => Level::Fatal,
        2 => Level::Error,
        3 => Level::Warning,
        5 => Level::Verbose,
        _ => Level::Information,
    }
}

/// 1601-01-01T00:00:00Z, the FILETIME epoch, in Unix milliseconds.
const FILETIME_ZERO_MS: i64 = -11_644_473_600_000;

pub fn to_record(data: &Value, record_id: u64, line: u32) -> Record {
    let mut r = Record::new(line);
    let event = data.get("Event").unwrap_or(data);
    let system = event.get("System").unwrap_or(&Value::Null);
    let mut put = |name: &str, v: Option<&Value>| {
        if let Some(v) = v {
            r.fields.insert(name.into(), v.clone());
        }
    };
    let provider = system.get("Provider").and_then(|p| attr(p, "Name")).map(show).unwrap_or_default();
    let event_id = system.get("EventID").and_then(text_of).cloned();
    put("Channel", system.get("Channel").and_then(text_of));
    put("Provider", Some(&Value::from(provider.clone())));
    put("EventId", event_id.as_ref());
    put("RecordId", Some(&Value::from(record_id)));
    put("Computer", system.get("Computer").and_then(text_of));
    let exec = system.get("Execution").unwrap_or(&Value::Null);
    put("ProcessId", attr(exec, "ProcessID"));
    put("ThreadId", attr(exec, "ThreadID"));
    put("Task", system.get("Task").and_then(text_of));
    put("Keywords", system.get("Keywords").and_then(text_of));
    put("UserId", system.get("Security").and_then(|s| attr(s, "UserID")));

    let ts = system
        .get("TimeCreated")
        .and_then(|t| attr(t, "SystemTime"))
        .and_then(Value::as_str)
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.timestamp_millis())
        // A zero FILETIME (1601-01-01) means "not recorded", not an event 400 years ago.
        .filter(|ms| *ms > FILETIME_ZERO_MS);
    r.ts = ts;
    r.level = system
        .get("Level")
        .and_then(text_of)
        .and_then(Value::as_i64)
        .map_or(Level::Information, level_of);

    let mut pairs: Vec<(String, String)> = Vec::new();
    for section in ["EventData", "UserData"] {
        if let Some(Value::Object(o)) = event.get(section) {
            collect(o, &mut pairs);
        }
    }
    for (k, v) in &pairs {
        let key = if r.fields.contains_key(k) {
            format!("EventData.{k}")
        } else {
            k.clone()
        };
        r.fields.insert(key, v_auto(v));
    }
    let mut msg = format!("{provider} {}", event_id.as_ref().map(show).unwrap_or_default());
    if !pairs.is_empty() {
        msg.push_str(": ");
        msg.push_str(&pairs.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(", "));
    }
    r.message = truncate(msg);
    r
}

fn v_auto(s: &str) -> Value {
    imt_plugin_api::model::auto_value(s)
}

/// Flattens EventData / UserData: named entries become key=value, anonymous
/// `Data` entries are numbered `Data1..`, nested elements are walked.
fn collect(o: &serde_json::Map<String, Value>, out: &mut Vec<(String, String)>) {
    for (k, v) in o {
        match (k.as_str(), v) {
            ("#attributes", _) => {}
            ("Data" | "#text", Value::Array(a)) => {
                for (i, x) in a.iter().enumerate() {
                    out.push((format!("{k}{}", i + 1), show(x)));
                }
            }
            (_, Value::Object(inner)) if inner.keys().any(|key| key != "#attributes" && key != "#text") => collect(inner, out),
            (_, Value::Object(inner)) => {
                if let Some(t) = inner.get("#text") {
                    out.push((k.clone(), show(t)));
                }
            }
            (_, Value::Null) => {}
            (_, other) => out.push((k.clone(), show(other))),
        }
    }
}

fn truncate(mut s: String) -> String {
    if s.chars().count() > MAX_MESSAGE {
        s = s.chars().take(MAX_MESSAGE - 1).collect();
        s.push('\u{2026}');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_plugin_api::time::TimestampParser;
    use serde_json::json;
    use std::path::Path;

    const SAMPLE: &[u8] = include_bytes!("../tests/fixtures/security.evtx");

    fn ctx_run<T>(f: impl FnOnce(&ImportCtx) -> T) -> T {
        let ts = TimestampParser::default();
        let mut ctx = ImportCtx::new(Path::new("x.evtx"), &ts);
        ctx.file_date = None;
        f(&ctx)
    }

    #[test]
    fn declares_identity_fields() {
        assert_eq!(
            imt_plugin_api::BinaryImporter::identity_fields(&EvtxImporter),
            ["Computer", "ProcessId"]
        );
    }

    #[test]
    fn reads_the_sample_log() {
        let out = ctx_run(|c| {
            assert_eq!(EvtxImporter.sniff(c, SAMPLE), 100);
            assert_eq!(EvtxImporter.sniff(c, b"hello"), 0);
            EvtxImporter.import(c, SAMPLE).unwrap()
        });
        assert_eq!(EvtxImporter.id(), "evtx");
        assert!(!out.records.is_empty());
        assert_eq!(out.skipped, 0);
        for r in &out.records {
            assert!(r.ts.is_some());
            for f in ["Channel", "Provider", "EventId", "RecordId", "Computer"] {
                assert!(r.fields.contains_key(f), "{f} missing in {:?}", r.fields);
            }
            assert!(r.message.starts_with(&format!("{} ", r.fields["Provider"].as_str().unwrap())));
        }
        assert!(read_evtx(SAMPLE).unwrap().len() == out.records.len());
    }

    #[test]
    fn rejects_non_evtx_and_survives_corruption() {
        assert!(read_evtx(b"not evtx").is_err());
        assert!(read_evtx(b"ElfFile\0short").is_err());
        assert!(!is_evtx(b"ElfFil"));
        // Corrupt the first chunk body: events are skipped or dropped, never a panic.
        let mut bad = SAMPLE.to_vec();
        for i in (4096 + 600..bad.len().min(4096 + 65536)).step_by(7) {
            bad[i] = bad[i].wrapping_add(0x55);
        }
        let _ = ctx_run(|c| EvtxImporter.import(c, &bad));
        let mut x: u32 = 7;
        for _ in 0..30 {
            let mut junk = MAGIC.to_vec();
            for _ in 0..8000 {
                x = x.wrapping_mul(1664525).wrapping_add(1013904223);
                junk.push((x >> 24) as u8);
            }
            let _ = read_evtx(&junk);
        }
        let _ = read_evtx(&SAMPLE[..SAMPLE.len() / 2]);
    }

    #[test]
    fn maps_event_json() {
        let data = json!({"Event": {
            "System": {
                "Provider": {"#attributes": {"Name": "Contoso"}},
                "EventID": {"#attributes": {"Qualifiers": 0}, "#text": 7036},
                "Level": 2, "Task": 0, "Keywords": "0x80000000000000", "Channel": "System", "Computer": "pc1",
                "TimeCreated": {"#attributes": {"SystemTime": "2026-10-14T10:00:00.250000Z"}},
                "Execution": {"#attributes": {"ProcessID": 4, "ThreadID": 8}},
                "Security": {"#attributes": {"UserID": "S-1-5-18"}}
            },
            "EventData": {"Name": "Svc", "State": "stopped", "Channel": "dup", "Empty": null,
                "Data": ["a", "b"], "Nested": {"#attributes": {"x": 1}, "#text": "t"},
                "Group": {"Inner": 5}},
            "UserData": {"Odd": {"#attributes": {"x": 1}}}
        }});
        let r = to_record(&data, 9, 3);
        assert_eq!(r.level, Level::Error);
        assert_eq!(r.ts, Some(1_791_972_000_250));
        assert_eq!(r.fields["EventId"], 7036);
        assert_eq!(r.fields["RecordId"], 9);
        assert_eq!(r.fields["ProcessId"], 4);
        assert_eq!(r.fields["UserId"], "S-1-5-18");
        assert_eq!(r.fields["Name"], "Svc");
        assert_eq!(r.fields["EventData.Channel"], "dup");
        assert_eq!(r.fields["Data2"], "b");
        assert_eq!(r.fields["Nested"], "t");
        assert_eq!(r.fields["Inner"], 5);
        assert!(r.message.starts_with("Contoso 7036: Name=Svc, State=stopped"));
        for (n, l) in [
            (1, Level::Fatal),
            (3, Level::Warning),
            (4, Level::Information),
            (0, Level::Information),
            (5, Level::Verbose),
        ] {
            assert_eq!(level_of(n), l);
        }
        // Minimal / odd shapes and message truncation.
        let r = to_record(&json!({"Event": {"System": {}, "EventData": {"Big": "x".repeat(2000)}}}), 1, 1);
        assert!(r.ts.is_none());
        assert_eq!(r.message.chars().count(), MAX_MESSAGE);
        let r = to_record(&json!({}), 1, 1);
        assert_eq!(r.message, " ");
        // A zero FILETIME is no time.
        let zero = json!({"Event": {"System": {"TimeCreated": {"#attributes": {"SystemTime": "1601-01-01T00:00:00.000000Z"}}}}});
        assert_eq!(to_record(&zero, 1, 1).ts, None);
    }
}
