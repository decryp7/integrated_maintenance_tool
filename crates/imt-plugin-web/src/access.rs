//! Apache / Nginx common and combined access logs.

use crate::level_for_status;
use anyhow::Result;
use chrono::DateTime;
use imt_core::importers::{ImportCtx, Imported, Importer};
use imt_core::model::Record;
use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;

fn line_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"^(?P<ip>\S+) (?P<ident>\S+) (?P<user>\S+) \[(?P<ts>[^\]]+)\] "(?P<req>[^"]*)" (?P<status>\d{3}|-) (?P<bytes>\d+|-)(?: "(?P<ref>[^"]*)" "(?P<ua>[^"]*)")?(?: (?P<rt>\d+(?:\.\d+)?))?\s*$"#,
        )
        .expect("static regex")
    })
}

pub struct AccessLogImporter;

impl Importer for AccessLogImporter {
    fn id(&self) -> &'static str {
        "access-log"
    }

    fn sniff(&self, _ctx: &ImportCtx, head: &str) -> u8 {
        let first = head.trim_start_matches('\u{feff}').lines().find(|l| !l.trim().is_empty());
        if first.is_some_and(|l| line_re().is_match(l)) {
            90
        } else {
            0
        }
    }

    fn import(&self, _ctx: &ImportCtx, content: &str) -> Result<Imported> {
        let mut out = Imported::default();
        for (i, line) in content.trim_start_matches('\u{feff}').lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            match line_re().captures(line) {
                Some(c) => out.records.push(record(i as u32 + 1, &c)),
                None => out.skipped += 1,
            }
        }
        Ok(out)
    }
}

fn record(line: u32, c: &regex::Captures) -> Record {
    let mut r = Record::new(line);
    r.ts = DateTime::parse_from_str(&c["ts"], "%d/%b/%Y:%H:%M:%S %z")
        .ok()
        .map(|d| d.timestamp_millis());
    r.fields.insert("ClientIp".into(), Value::from(&c["ip"]));
    if &c["user"] != "-" {
        r.fields.insert("User".into(), Value::from(&c["user"]));
    }
    let req = &c["req"];
    let mut parts = req.splitn(3, ' ');
    let (method, path, proto) = (parts.next().unwrap_or(""), parts.next(), parts.next());
    if let Some(path) = path {
        r.fields.insert("Method".into(), Value::from(method));
        r.fields.insert("Path".into(), Value::from(path));
        if let Some(p) = proto {
            r.fields.insert("Protocol".into(), Value::from(p));
        }
    }
    let status = c["status"].parse::<i64>().ok();
    if let Some(s) = status {
        r.fields.insert("Status".into(), Value::from(s));
        r.level = level_for_status(s);
    }
    if let Ok(b) = c["bytes"].parse::<u64>() {
        r.fields.insert("Bytes".into(), Value::from(b));
    }
    for (group, name) in [("ref", "Referer"), ("ua", "UserAgent")] {
        if let Some(m) = c.name(group).filter(|m| m.as_str() != "-") {
            r.fields.insert(name.into(), Value::from(m.as_str()));
        }
    }
    let ms = c.name("rt").and_then(|m| {
        let s = m.as_str();
        if s.contains('.') {
            s.parse::<f64>().ok().map(|secs| (secs * 1000.0 * 1000.0).round() / 1000.0)
        } else {
            s.parse::<f64>().ok().map(|us| us / 1000.0)
        }
    });
    if let Some(ms) = ms {
        r.fields.insert("RequestTimeMs".into(), Value::from(ms));
    }
    r.message = match path {
        Some(p) => format!("{method} {p}"),
        None => req.to_string(),
    };
    if let Some(s) = status {
        r.message.push_str(&format!(" \u{2192} {s}"));
    }
    if let Some(ms) = ms {
        r.message.push_str(&format!(" in {ms} ms"));
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{garbage, with_ctx};
    use imt_core::model::Level;
    use serde_json::json;

    const LOG: &str = include_str!("../tests/fixtures/access.log");

    #[test]
    fn parses_combined_and_common() {
        let out = with_ctx(|c| {
            assert_eq!(AccessLogImporter.sniff(c, LOG), 90);
            AccessLogImporter.import(c, LOG).unwrap()
        });
        assert_eq!(AccessLogImporter.id(), "access-log");
        assert_eq!((out.records.len(), out.skipped), (5, 1));
        let a = &out.records[0];
        assert_eq!(a.ts, Some(1_791_665_736_000));
        assert_eq!(a.fields["ClientIp"], "127.0.0.1");
        assert_eq!(a.fields["User"], "frank");
        assert_eq!(a.fields["Method"], "GET");
        assert_eq!(a.fields["Path"], "/apache_pb.gif?x=1");
        assert_eq!(a.fields["Protocol"], "HTTP/1.0");
        assert_eq!(a.fields["Status"], 200);
        assert_eq!(a.fields["Bytes"], 2326);
        assert_eq!(a.fields["Referer"], "http://example.com/start.html");
        assert_eq!(a.fields["UserAgent"], "Mozilla/4.08 [en] (Win98; I ;Nav)");
        assert_eq!(a.fields["RequestTimeMs"], json!(32.0));
        assert_eq!(a.message, "GET /apache_pb.gif?x=1 \u{2192} 200 in 32 ms");
        let b = &out.records[1];
        assert!(!b.fields.contains_key("User") && !b.fields.contains_key("Bytes") && !b.fields.contains_key("Referer"));
        assert_eq!(b.fields["RequestTimeMs"], json!(12.345));
        let common = &out.records[2];
        assert_eq!((common.level, common.fields["User"].clone()), (Level::Warning, json!("bob")));
        assert!(!common.fields.contains_key("UserAgent"));
        assert_eq!(out.records[3].level, Level::Error);
        let odd = &out.records[4];
        assert_eq!(odd.message, "- \u{2192} 408");
        assert!(!odd.fields.contains_key("Method"));
    }

    #[test]
    fn sniff_negatives_and_odd_values() {
        with_ctx(|c| {
            assert_eq!(AccessLogImporter.sniff(c, "\n\u{feff}2026-10-14 10:00:00 INF hello"), 0);
            assert_eq!(AccessLogImporter.sniff(c, ""), 0);
            let out = AccessLogImporter.import(c, "h - - [bad date] \"GET / HTTP/1.1\" - -\n\n").unwrap();
            let r = &out.records[0];
            assert!(r.ts.is_none() && !r.fields.contains_key("Status") && !r.fields.contains_key("Bytes"));
            assert_eq!(r.level, Level::Information);
        });
    }

    #[test]
    fn garbage_never_panics() {
        with_ctx(|c| {
            for g in garbage(
                &[
                    "1.2.3.4",
                    " - ",
                    "[10/Oct/2026:13:55:36 +0000]",
                    "\"",
                    "GET /",
                    " 200 ",
                    "12",
                    "\n",
                    "x",
                    "]",
                    "[",
                ],
                300,
            ) {
                let _ = AccessLogImporter.sniff(c, &g);
                let _ = AccessLogImporter.import(c, &g);
            }
        });
    }
}
