//! W3C extended log format (IIS, Windows Firewall), and its comma-separated variant with quoted
//! values and a `date-time` column (Exchange message tracking and protocol logs).

use crate::level_for_status;
use anyhow::Result;
use chrono::NaiveDateTime;
use imt_core::importers::{ImportCtx, Imported, Importer};
use imt_core::model::{auto_value, Record};
use serde_json::Value;

pub struct W3cImporter;

impl Importer for W3cImporter {
    fn id(&self) -> &'static str {
        "w3c"
    }

    fn sniff(&self, _ctx: &ImportCtx, head: &str) -> u8 {
        let header = head
            .trim_start_matches('\u{feff}')
            .lines()
            .take(20)
            .take_while(|l| l.starts_with('#') || l.trim().is_empty());
        if header.into_iter().any(|l| l.starts_with("#Fields:")) {
            100
        } else {
            0
        }
    }

    fn import(&self, _ctx: &ImportCtx, content: &str) -> Result<Imported> {
        let mut out = Imported::default();
        let mut columns: Vec<&str> = Vec::new();
        let mut commas = false;
        for (i, line) in content.trim_start_matches('\u{feff}').lines().enumerate() {
            let line = line.trim_end();
            if line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix('#') {
                if let Some(fields) = rest.strip_prefix("Fields:") {
                    commas = fields.contains(',');
                    columns = if commas {
                        fields.split(',').map(str::trim).collect()
                    } else {
                        fields.split_whitespace().collect()
                    };
                }
                continue;
            }
            let quoted;
            let values: Vec<&str> = if commas {
                quoted = split_quoted(line);
                quoted.iter().map(String::as_str).collect()
            } else {
                line.split(' ').collect()
            };
            if columns.is_empty() || values.len() != columns.len() {
                out.skipped += 1;
                continue;
            }
            out.records.push(record(i as u32 + 1, &columns, &values));
        }
        Ok(out)
    }
}

/// One comma-separated row; values may be quoted, with `""` for a quote.
fn split_quoted(line: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut value = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                chars.next();
                value.push('"');
            }
            '"' => quoted = !quoted,
            ',' if !quoted => values.push(std::mem::take(&mut value)),
            _ => value.push(c),
        }
    }
    values.push(value);
    values
}

fn record(line: u32, columns: &[&str], values: &[&str]) -> Record {
    let mut r = Record::new(line);
    let (mut date, mut time, mut date_time) = ("", "", "");
    for (name, value) in columns.iter().zip(values) {
        match *name {
            "date" => date = value,
            "time" => time = value,
            "date-time" => date_time = value,
            _ if value.is_empty() || *value == "-" => {}
            _ => {
                r.fields.insert((*name).into(), auto_value(value));
            }
        }
    }
    r.ts = match date_time {
        "" => NaiveDateTime::parse_from_str(&format!("{date} {time}"), "%Y-%m-%d %H:%M:%S%.f")
            .ok()
            .map(|d| d.and_utc().timestamp_millis()),
        dt => chrono::DateTime::parse_from_rfc3339(dt).ok().map(|d| d.timestamp_millis()),
    };
    let text = |k: &str| r.fields.get(k).map(|v| imt_core::model::value_text(v).into_owned());
    let status = r.fields.get("sc-status").and_then(Value::as_i64);
    let mut msg = format!(
        "{} {}",
        text("cs-method").unwrap_or_default(),
        text("cs-uri-stem").unwrap_or_default()
    );
    if let Some(q) = text("cs-uri-query") {
        msg.push('?');
        msg.push_str(&q);
    }
    if let Some(s) = status {
        msg.push_str(&format!(" \u{2192} {s}"));
    }
    if let Some(t) = text("time-taken") {
        msg.push_str(&format!(" in {t} ms"));
    }
    r.message = msg.trim().to_string();
    if r.message.is_empty() {
        // Not a web request (Exchange, firewall): the row's values.
        let parts: Vec<String> = r.fields.values().map(|v| imt_core::model::value_text(v).into_owned()).collect();
        r.message = parts.join(", ");
    }
    if let Some(s) = status {
        r.level = level_for_status(s);
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{garbage, with_ctx};
    use imt_core::model::Level;

    const IIS: &str = include_str!("../tests/fixtures/iis.log");

    #[test]
    fn parses_iis_and_follows_field_changes() {
        let out = with_ctx(|c| {
            assert_eq!(W3cImporter.sniff(c, IIS), 100);
            W3cImporter.import(c, IIS).unwrap()
        });
        assert_eq!(W3cImporter.id(), "w3c");
        assert_eq!((out.records.len(), out.skipped), (4, 1));
        let a = &out.records[0];
        assert_eq!(a.ts, Some(1_791_972_000_000));
        assert_eq!(a.message, "GET /api/orders?id=42 \u{2192} 200 in 32 ms");
        assert_eq!(a.fields["sc-status"], 200);
        assert_eq!(a.fields["time-taken"], 32);
        assert_eq!(a.fields["cs(User-Agent)"], "Mozilla/5.0+(Windows+NT+10.0)");
        assert!(!a.fields.contains_key("cs-username"));
        assert_eq!(a.level, Level::Information);
        assert_eq!(out.records[1].level, Level::Warning);
        assert_eq!(out.records[1].fields["cs-username"], "alice");
        assert_eq!(out.records[2].level, Level::Error);
        let last = &out.records[3];
        assert_eq!(last.message, "GET /health \u{2192} 200 in 3 ms");
        assert!(!last.fields.contains_key("s-ip"));
        assert_eq!(last.line, 10);
    }

    #[test]
    fn comma_separated_with_date_time() {
        // The Exchange message tracking layout (synthetic values).
        let log = concat!(
            "\u{feff}#Software: Microsoft Exchange Server\r\n#Version: 15.0.0.0\r\n#Log-type: Message Tracking Log\r\n",
            "#Fields: date-time,client-ip,source,event-id,message-subject,recipient-count\r\n",
            "2026-10-14T10:00:02.052Z,,STOREDRIVER,NOTIFYMAPI,\"Quota, \"\"warning\"\"\",1\r\n",
            "2026-10-14T10:00:03.000Z,10.0.0.5,SMTP,RECEIVE,Hello,2\r\n",
            "2026-10-14T10:00:04.000Z,too,few\r\n",
        );
        let out = with_ctx(|c| {
            assert_eq!(W3cImporter.sniff(c, log), 100);
            W3cImporter.import(c, log).unwrap()
        });
        assert_eq!((out.records.len(), out.skipped), (2, 1));
        let r = &out.records[0];
        assert_eq!(r.ts, Some(1_791_972_002_052));
        assert_eq!(r.fields["message-subject"], "Quota, \"warning\"");
        assert!(!r.fields.contains_key("client-ip"));
        assert_eq!(r.message, "STOREDRIVER, NOTIFYMAPI, Quota, \"warning\", 1");
        assert_eq!(out.records[1].fields["recipient-count"], 2);
    }

    #[test]
    fn sniff_and_edge_cases() {
        with_ctx(|c| {
            assert_eq!(W3cImporter.sniff(c, "\u{feff}#Software: x\n#Fields: date time\n"), 100);
            assert_eq!(W3cImporter.sniff(c, "#Software: x\n2026-01-01 00:00:00 a\n#Fields: a"), 0);
            assert_eq!(W3cImporter.sniff(c, "2026-10-14 10:00:00 INF hello"), 0);
            assert_eq!(W3cImporter.sniff(c, ""), 0);
            // rows before any #Fields, bad dates, no status or method
            let out = W3cImporter
                .import(
                    c,
                    "2026-01-01 00:00:00 a\n#Fields: date time x\nbad bad 1\n\n2026-01-01 00:00:00.5 -\n",
                )
                .unwrap();
            assert_eq!(out.skipped, 1);
            assert!(out.records[0].ts.is_none());
            assert_eq!(out.records[0].message, "1", "not a request: the row's values");
            assert_eq!(out.records[1].message, "");
            assert_eq!(out.records[1].ts, Some(1_767_225_600_500));
        });
    }

    #[test]
    fn garbage_never_panics() {
        with_ctx(|c| {
            for g in garbage(
                &[
                    "#Fields: ",
                    "date time ",
                    "sc-status ",
                    " ",
                    "-",
                    "\n",
                    "200",
                    "x",
                    "\u{e9}",
                    "#",
                    "2026-01-01",
                ],
                300,
            ) {
                let _ = W3cImporter.sniff(c, &g);
                let _ = W3cImporter.import(c, &g);
            }
        });
    }
}
