//! Kubernetes container logs in the CRI format (containerd, CRI-O):
//! `2026-10-17T12:00:00.123456789Z stdout F message`. The tag `P` marks a
//! partial line that continues on the next line of the same stream; `F` ends it.

use crate::container::{path_fields, Joiner, Stream};
use crate::first_line;
use anyhow::Result;
use chrono::DateTime;
use imt_core::importers::{ImportCtx, Imported, Importer, MAX_LINE_LEN};
use imt_core::model::Fields;

pub struct CriImporter;

impl Importer for CriImporter {
    fn id(&self) -> &'static str {
        "cri"
    }

    /// Where the container runs, from the log path (`Namespace`/`Pod`/`Container`) and its id.
    fn identity_fields(&self) -> Vec<String> {
        ["Namespace", "Pod", "Container", "ContainerId"].map(String::from).to_vec()
    }

    /// Above the generic text importer (70) and syslog (75).
    fn sniff(&self, _ctx: &ImportCtx, head: &str) -> u8 {
        if first_line(head).and_then(parse_line).is_some() {
            95
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

/// One parsed line: time (epoch ms), stream, whether it ends a message, text.
struct Line<'a> {
    ts: i64,
    stream: Stream,
    last: bool,
    text: &'a str,
}

fn parse_line(line: &str) -> Option<Line<'_>> {
    let (time, rest) = line.split_once(' ')?;
    let (stream, rest) = rest.split_once(' ')?;
    let (tag, text) = rest.split_once(' ').unwrap_or((rest, ""));
    // The tag may carry more flags after ':'; the first one is the line type.
    let last = match tag.split(':').next() {
        Some("F") => true,
        Some("P") => false,
        _ => return None,
    };
    // RFC 3339 with a 'T' and a zone, as every CRI runtime writes it.
    if time.len() < 20 || time.as_bytes().get(10) != Some(&b'T') {
        return None;
    }
    Some(Line {
        ts: DateTime::parse_from_rfc3339(time).ok()?.timestamp_millis(),
        stream: Stream::parse(stream)?,
        last,
        text,
    })
}

fn import(ctx: &ImportCtx, content: &str) -> Imported {
    let mut skipped = 0;
    let mut joiner = Joiner::new(path_fields(ctx.path));
    for (i, line) in content.trim_start_matches('\u{feff}').lines().enumerate() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.trim().is_empty() {
            continue;
        }
        if line.len() > MAX_LINE_LEN {
            skipped += 1;
            continue;
        }
        match parse_line(line) {
            Some(l) => joiner.push(l.stream, i as u32 + 1, Some(l.ts), l.text, l.last, Fields::new()),
            None => skipped += 1,
        }
    }
    Imported {
        records: joiner.finish(),
        skipped,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{garbage, ms, with_ctx, with_path};
    use imt_core::model::Level;

    const LOG: &str = "\
2026-10-17T12:00:00.123456789Z stdout F Started orders v3.14.2\r
2026-10-17T12:00:01.000000000Z stdout P {\"level\":\"error\",
2026-10-17T12:00:01.100000000+02:00 stderr F something on stderr
2026-10-17T12:00:01.000000001Z stdout F \"msg\":\"db down\"}
not a cri line
2026-10-17T12:00:02Z stdout X bad tag
2026-10-17T12:00:03.5Z stdout F

2026-10-17T12:00:04Z stderr F:x E1017 12:00:04.000000 1 main.go:1] failed
";

    #[test]
    fn declares_identity_fields() {
        assert_eq!(
            imt_core::importers::Importer::identity_fields(&CriImporter),
            ["Namespace", "Pod", "Container", "ContainerId"]
        );
    }

    #[test]
    fn parses_joins_partials_and_levels() {
        let out = with_ctx(|c| {
            assert_eq!(CriImporter.sniff(c, LOG), 95);
            CriImporter.import(c, LOG).unwrap()
        });
        assert_eq!(CriImporter.id(), "cri");
        assert_eq!((out.records.len(), out.skipped), (5, 2));
        let r = &out.records;
        assert_eq!(
            (r[0].message.as_str(), r[0].ts, r[0].level),
            ("Started orders v3.14.2", Some(ms("2026-10-17T12:00:00.123Z")), Level::Information)
        );
        assert_eq!(
            (r[1].line, r[1].message.as_str(), r[1].level),
            (2, "{\"level\":\"error\",\"msg\":\"db down\"}", Level::Error)
        );
        assert_eq!((r[2].ts, r[2].level), (Some(ms("2026-10-17T10:00:01.1Z")), Level::Warning));
        assert_eq!(r[2].fields["Stream"], "stderr");
        assert_eq!((r[3].message.as_str(), r[3].ts), ("", Some(ms("2026-10-17T12:00:03.5Z"))));
        assert_eq!(r[4].level, Level::Error);
    }

    #[test]
    fn sniff_negatives_and_pod_metadata() {
        with_ctx(|c| {
            for head in [
                "",
                "2026-10-14 09:00:00.000 +00:00 [INF] hello",
                "2026-10-17T12:00:00Z stdin F x",
                "2026-10-17T12:00:00 stdout F no zone",
                "2026-10-17 12:00:00Z stdout F space",
                "17-10-2026T12:00:00Z stdout F x",
                "x stdout F",
                "2026-10-17T12:00:00Z stdout",
                "<14>1 2026-10-17T12:00:00Z host app - - - msg",
            ] {
                assert_eq!(CriImporter.sniff(c, head), 0, "{head}");
            }
            assert!(CriImporter.import_tail(c, LOG, "").unwrap().unwrap().records.is_empty());
            let long = format!("2026-10-17T12:00:00Z stdout F {}", "x".repeat(MAX_LINE_LEN));
            assert_eq!(CriImporter.import(c, &long).unwrap().skipped, 1);
        });
        let out = with_path("/var/log/pods/shop_orders-1_uid-1/orders/0.log", |c| {
            CriImporter.import(c, LOG).unwrap()
        });
        let f = &out.records[0].fields;
        assert_eq!(
            (f["Namespace"].as_str(), f["Pod"].as_str(), f["Container"].as_str()),
            (Some("shop"), Some("orders-1"), Some("orders"))
        );
        assert_eq!(f["Restart"], 0);
    }

    #[test]
    fn garbage_never_panics() {
        with_ctx(|c| {
            for g in garbage(
                &[
                    "2026-10-17T12:00:00.1Z",
                    " ",
                    "stdout",
                    "stderr",
                    "F",
                    "P",
                    "x",
                    "\n",
                    "é",
                    ":",
                    "\r",
                ],
                300,
            ) {
                let _ = CriImporter.sniff(c, &g);
                let _ = CriImporter.import(c, &g);
            }
        });
    }
}
