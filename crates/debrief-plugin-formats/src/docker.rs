//! Docker `json-file` logs: one `{"log":"...","stream":"stdout","time":"..."}`
//! object per line. A `log` value without a trailing newline is a partial line
//! (Docker splits at 16 KiB) and is joined with the chunks that follow it on
//! the same stream. Labels and env added with `--log-opt` (`attrs`) become fields.

use crate::container::{path_fields, Joiner, Stream};
use crate::first_line;
use anyhow::Result;
use chrono::DateTime;
use debrief_plugin_api::importer::{ImportCtx, Imported, TextImporter, MAX_LINE_LEN};
use debrief_plugin_api::model::Fields;
use serde_json::{Map, Value};

pub struct DockerJsonImporter;

impl TextImporter for DockerJsonImporter {
    fn id(&self) -> &'static str {
        "docker-json"
    }

    /// Where the container runs, from the log path (`Namespace`/`Pod` for Kubernetes) and its name or id.
    fn identity_fields(&self) -> Vec<String> {
        ["Namespace", "Pod", "Container", "ContainerId"].map(String::from).to_vec()
    }

    /// Above the generic JSON importer (95), which would otherwise claim these lines.
    fn sniff(&self, _ctx: &ImportCtx, head: &str) -> u8 {
        let entry = first_line(head)
            .filter(|l| l.trim_start().starts_with('{'))
            .and_then(|l| serde_json::from_str::<Map<String, Value>>(l).ok());
        if entry.as_ref().and_then(Entry::parse).is_some() {
            98
        } else {
            0
        }
    }

    fn import(&self, ctx: &ImportCtx, content: &str) -> Result<Imported> {
        Ok(import(ctx, content))
    }

    fn import_tail(&self, ctx: &ImportCtx, _head: &str, tail: &str) -> Option<Result<Imported>> {
        Some(Ok(import(ctx, tail)))
    }
}

/// One parsed line.
struct Entry<'a> {
    log: &'a str,
    stream: Stream,
    time: &'a str,
}

impl<'a> Entry<'a> {
    fn parse(obj: &'a Map<String, Value>) -> Option<Entry<'a>> {
        Some(Entry {
            log: obj.get("log")?.as_str()?,
            stream: Stream::parse(obj.get("stream")?.as_str()?)?,
            time: obj.get("time")?.as_str()?,
        })
    }
}

fn import(ctx: &ImportCtx, content: &str) -> Imported {
    let mut skipped = 0;
    let mut joiner = Joiner::new(path_fields(ctx.path));
    for (i, line) in content.trim_start_matches('\u{feff}').lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line.len() > MAX_LINE_LEN {
            skipped += 1;
            continue;
        }
        let Ok(obj) = serde_json::from_str::<Map<String, Value>>(line) else {
            skipped += 1;
            continue;
        };
        let Some(e) = Entry::parse(&obj) else {
            skipped += 1;
            continue;
        };
        let ts = DateTime::parse_from_rfc3339(e.time)
            .ok()
            .map(|d| d.timestamp_millis())
            .or_else(|| ctx.timestamps.parse(e.time));
        let (text, last) = match e.log.strip_suffix('\n') {
            Some(t) => (t.strip_suffix('\r').unwrap_or(t), true),
            None => (e.log, false),
        };
        joiner.push(e.stream, i as u32 + 1, ts, text, last, attrs(&obj));
    }
    Imported {
        records: joiner.finish(),
        skipped,
        ..Default::default()
    }
}

/// `attrs` (labels, env, tag) as fields; non-string values are kept as they are.
fn attrs(obj: &Map<String, Value>) -> Fields {
    let mut f = Fields::new();
    if let Some(Value::Object(attrs)) = obj.get("attrs") {
        for (k, v) in attrs.iter().take(crate::MAX_FIELDS) {
            f.insert(k.clone(), v.clone());
        }
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{garbage, ms, with_ctx, with_path};
    use debrief_plugin_api::model::Level;

    const LOG: &str = concat!(
        r#"{"log":"Listening on :8080\n","stream":"stdout","time":"2026-10-17T12:00:00.123456789Z"}"#,
        "\n",
        r#"{"log":"part one, ","stream":"stdout","time":"2026-10-17T12:00:01.000000001Z"}"#,
        "\n",
        r#"{"log":"retrying\r\n","stream":"stderr","time":"2026-10-17T12:00:01.5+02:00","attrs":{"tag":"orders","n":3}}"#,
        "\n",
        r#"{"log":"part two\n","stream":"stdout","time":"2026-10-17T12:00:01.000000002Z"}"#,
        "\n",
        "not json\n",
        r#"{"log":"x","stream":"stdin","time":"2026-10-17T12:00:00Z"}"#,
        "\n\n",
        r#"{"log":"{\"level\":\"error\",\"msg\":\"db down\"}\n","stream":"stdout","time":"yesterday"}"#,
        "\n",
    );

    #[test]
    fn declares_identity_fields() {
        assert_eq!(
            debrief_plugin_api::TextImporter::identity_fields(&DockerJsonImporter),
            ["Namespace", "Pod", "Container", "ContainerId"]
        );
    }

    #[test]
    fn parses_joins_partials_and_types_fields() {
        let out = with_ctx(|c| {
            assert_eq!(DockerJsonImporter.sniff(c, LOG), 98);
            DockerJsonImporter.import(c, LOG).unwrap()
        });
        assert_eq!(DockerJsonImporter.id(), "docker-json");
        assert_eq!((out.records.len(), out.skipped), (4, 2));
        let r = &out.records;
        assert_eq!(
            (r[0].message.as_str(), r[0].ts, r[0].level),
            ("Listening on :8080", Some(ms("2026-10-17T12:00:00.123Z")), Level::Information)
        );
        assert_eq!(r[0].fields["Stream"], "stdout");
        assert_eq!((r[1].line, r[1].message.as_str()), (2, "part one, part two"));
        assert_eq!(r[1].ts, Some(ms("2026-10-17T12:00:01Z")));
        assert_eq!((r[2].message.as_str(), r[2].level), ("retrying", Level::Warning));
        assert_eq!(r[2].ts, Some(ms("2026-10-17T10:00:01.5Z")));
        assert_eq!((r[2].fields["tag"].as_str(), r[2].fields["n"].as_i64()), (Some("orders"), Some(3)));
        assert_eq!((r[3].level, r[3].ts), (Level::Error, None));
    }

    #[test]
    fn sniff_negatives_and_path_metadata() {
        with_ctx(|c| {
            for head in [
                "",
                "{\"@t\":\"2026-10-14T10:00:00Z\",\"@mt\":\"x\"}",
                "{\"log\":\"x\",\"stream\":\"stdout\"}",
                "{\"log\":1,\"stream\":\"stdout\",\"time\":\"t\"}",
                "{ broken",
                "2026-10-17 12:00:00 x",
            ] {
                assert_eq!(DockerJsonImporter.sniff(c, head), 0, "{head}");
            }
            let tail = DockerJsonImporter.import_tail(c, LOG, "").unwrap().unwrap();
            assert!(tail.records.is_empty());
            let long = format!("{{\"log\":\"{}\"}}\n", "x".repeat(MAX_LINE_LEN));
            assert_eq!(DockerJsonImporter.import(c, &long).unwrap().skipped, 1);
        });
        let id = "0123456789abcdef".repeat(4);
        let path = format!("/var/lib/docker/containers/{id}/{id}-json.log");
        let out = with_path(&path, |c| DockerJsonImporter.import(c, LOG).unwrap());
        assert_eq!(out.records[0].fields["ContainerId"], id.as_str());
    }

    #[test]
    fn garbage_never_panics() {
        with_ctx(|c| {
            for g in garbage(
                &[
                    "{\"log\":\"",
                    "x",
                    "\\n",
                    "\",\"stream\":\"stdout\"",
                    ",\"stream\":\"stderr\"",
                    ",\"time\":\"2026-10-17T12:00:00Z\"}",
                    ",\"attrs\":{\"a\":1}",
                    "}",
                    "\n",
                    "\"",
                    "\\",
                ],
                300,
            ) {
                let _ = DockerJsonImporter.sniff(c, &g);
                let _ = DockerJsonImporter.import(c, &g);
            }
        });
    }
}
