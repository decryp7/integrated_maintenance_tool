//! Writing events out: the [`Exporter`] extension point and the built-in
//! CSV and CLEF (Serilog compact JSON) exporters.

use crate::model::{Level, Record};
use anyhow::Result;
use chrono::{SecondsFormat, TimeZone, Utc};
use serde_json::{Map, Value};
use std::io::Write;

/// One event handed to an exporter, with what the workspace knows about it.
pub struct ExportRow<'a> {
    pub record: &'a Record,
    pub source_name: &'a str,
    /// Instance key under the current group-by.
    pub group: String,
}

/// An output format. Exporters stream: they must write rows as they come and
/// never collect them all in memory.
pub trait Exporter: Send + Sync {
    /// Stable id used by the UI and in export requests (`"csv"`).
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    /// File extension without the dot.
    fn extension(&self) -> &'static str;
    /// Writes every row and returns how many were written.
    fn export(&self, rows: &mut dyn Iterator<Item = ExportRow<'_>>, out: &mut dyn Write) -> Result<usize>;
}

fn iso(ts: Option<i64>) -> String {
    ts.and_then(|ms| Utc.timestamp_millis_opt(ms).single())
        .map(|t| t.to_rfc3339_opts(SecondsFormat::Millis, true))
        .unwrap_or_default()
}

/// CSV with fixed columns; event properties go into one JSON column so no
/// information is lost and the file can be written in a single pass.
pub struct CsvExporter;

impl Exporter for CsvExporter {
    fn id(&self) -> &'static str {
        "csv"
    }

    fn name(&self) -> &'static str {
        "CSV (spreadsheet)"
    }

    fn extension(&self) -> &'static str {
        "csv"
    }

    fn export(&self, rows: &mut dyn Iterator<Item = ExportRow<'_>>, out: &mut dyn Write) -> Result<usize> {
        let mut w = csv::Writer::from_writer(out);
        w.write_record([
            "Timestamp",
            "Level",
            "Instance",
            "Source",
            "Line",
            "Message",
            "Exception",
            "Properties",
        ])?;
        let mut n = 0;
        for row in rows {
            let r = row.record;
            w.write_record([
                iso(r.ts).as_str(),
                r.level.as_str(),
                &row.group,
                row.source_name,
                &r.line.to_string(),
                &r.message,
                r.exception.as_deref().unwrap_or(""),
                &if r.fields.is_empty() {
                    String::new()
                } else {
                    Value::Object(r.fields.clone()).to_string()
                },
            ])?;
            n += 1;
        }
        w.flush()?;
        Ok(n)
    }
}

/// One record as a CLEF JSON object.
pub fn clef_value(r: &Record) -> Value {
    let mut e = Map::new();
    if r.ts.is_some() {
        e.insert("@t".into(), Value::from(iso(r.ts)));
    }
    if r.level != Level::Information {
        e.insert("@l".into(), Value::from(r.level.as_str()));
    }
    e.insert("@m".into(), Value::from(r.message.as_str()));
    if let Some(t) = &r.template {
        e.insert("@mt".into(), Value::from(t.as_str()));
    }
    if let Some(x) = &r.exception {
        e.insert("@x".into(), Value::from(x.as_str()));
    }
    for (k, v) in &r.fields {
        let k = if k.starts_with('@') { format!("@{k}") } else { k.clone() };
        e.insert(k, v.clone());
    }
    Value::Object(e)
}

/// Where collectors write the events they produce.
pub trait EventSink: Send + Sync {
    fn write(&self, records: &[Record]) -> Result<()>;
}

/// Appends events as CLEF lines to `<dir>/<stem>.clef`, rolling over to
/// `<stem>-1.clef`, `<stem>-2.clef`, ... when a file exceeds `max_bytes`, and
/// keeping at most `keep` files.
pub struct ClefFileSink {
    dir: std::path::PathBuf,
    stem: String,
    max_bytes: u64,
    keep: usize,
    lock: std::sync::Mutex<()>,
}

impl ClefFileSink {
    pub fn new(dir: impl Into<std::path::PathBuf>, stem: &str, max_bytes: u64, keep: usize) -> Result<ClefFileSink> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)?;
        Ok(ClefFileSink {
            dir,
            stem: stem.to_string(),
            max_bytes: max_bytes.max(1),
            keep: keep.max(1),
            lock: std::sync::Mutex::new(()),
        })
    }

    /// The file currently written to.
    pub fn current(&self) -> std::path::PathBuf {
        self.dir.join(format!("{}.clef", self.stem))
    }

    fn roll(&self) -> Result<()> {
        let numbered = |i: usize| self.dir.join(format!("{}-{i}.clef", self.stem));
        let oldest = numbered(self.keep - 1);
        if self.keep == 1 {
            return Ok(std::fs::remove_file(self.current())?);
        }
        let _ = std::fs::remove_file(&oldest);
        for i in (1..self.keep - 1).rev() {
            let from = numbered(i);
            if from.exists() {
                std::fs::rename(&from, numbered(i + 1))?;
            }
        }
        std::fs::rename(self.current(), numbered(1))?;
        Ok(())
    }
}

impl EventSink for ClefFileSink {
    fn write(&self, records: &[Record]) -> Result<()> {
        let _guard = self.lock.lock().map_err(|_| anyhow::anyhow!("sink lock poisoned"))?;
        if std::fs::metadata(self.current())
            .map(|m| m.len() >= self.max_bytes)
            .unwrap_or(false)
        {
            self.roll()?;
        }
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(self.current())?;
        let mut buf = Vec::new();
        for r in records {
            serde_json::to_writer(&mut buf, &clef_value(r))?;
            buf.push(b'\n');
        }
        f.write_all(&buf)?;
        Ok(())
    }
}

/// Serilog compact JSON, one event per line; IMT, Seq and Serilog tooling
/// read it back with all properties.
pub struct ClefExporter;

impl Exporter for ClefExporter {
    fn id(&self) -> &'static str {
        "clef"
    }

    fn name(&self) -> &'static str {
        "Serilog CLEF (JSON lines)"
    }

    fn extension(&self) -> &'static str {
        "clef"
    }

    fn export(&self, rows: &mut dyn Iterator<Item = ExportRow<'_>>, out: &mut dyn Write) -> Result<usize> {
        let mut n = 0;
        for row in rows {
            serde_json::to_writer(&mut *out, &clef_value(row.record))?;
            out.write_all(b"\n")?;
            n += 1;
        }
        out.flush()?;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::importers::Importer;
    use serde_json::json;

    fn rec() -> Record {
        let mut r = Record::new(7);
        r.ts = Some(1_791_972_000_123);
        r.level = Level::Error;
        r.message = "Order \"88\", failed".into();
        r.template = Some("Order {Id} failed".into());
        r.exception = Some("System.Exception: boom\n   at X()".into());
        r.fields.insert("Id".into(), json!(88));
        r.fields.insert("@odd".into(), json!("x"));
        r
    }

    fn run(e: &dyn Exporter, records: &[Record]) -> (usize, String) {
        let mut out = Vec::new();
        let mut rows = records.iter().map(|r| ExportRow {
            record: r,
            source_name: "a/b.clef",
            group: "WEB01".into(),
        });
        let n = e.export(&mut rows, &mut out).unwrap();
        (n, String::from_utf8(out).unwrap())
    }

    #[test]
    fn csv_quotes_and_keeps_properties() {
        let mut info = Record::new(1);
        info.message = "plain".into();
        let (n, text) = run(&CsvExporter, &[rec(), info]);
        assert_eq!(n, 2);
        let mut reader = csv::Reader::from_reader(text.as_bytes());
        let rows: Vec<csv::StringRecord> = reader.records().map(|r| r.unwrap()).collect();
        assert_eq!(&rows[0][0], "2026-10-14T10:00:00.123Z");
        assert_eq!(&rows[0][1], "Error");
        assert_eq!(&rows[0][2], "WEB01");
        assert_eq!(&rows[0][5], "Order \"88\", failed");
        assert!(rows[0][6].contains("   at X()"));
        assert_eq!(serde_json::from_str::<Value>(&rows[0][7]).unwrap()["Id"], 88);
        assert_eq!((&rows[1][0], &rows[1][7]), ("", ""));
        assert_eq!((CsvExporter.id(), CsvExporter.extension()), ("csv", "csv"));
        assert!(CsvExporter.name().contains("CSV"));
    }

    #[test]
    fn clef_round_trips_through_the_json_importer() {
        let mut info = Record::new(2);
        info.message = "no time".into();
        let (n, text) = run(&ClefExporter, &[rec(), info]);
        assert_eq!(n, 2);
        let lines: Vec<Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines[0]["@t"], "2026-10-14T10:00:00.123Z");
        assert_eq!(lines[0]["@l"], "Error");
        assert_eq!(lines[0]["@mt"], "Order {Id} failed");
        assert_eq!(lines[0]["@@odd"], "x");
        assert!(lines[1].get("@t").is_none() && lines[1].get("@l").is_none());

        let p = crate::profile::CompiledProfile::compile(crate::profile::Profile::default_profile()).unwrap();
        let ctx = crate::importers::ImportCtx {
            profile: &p,
            path: std::path::Path::new("x.clef"),
            file_date: None,
        };
        let back = crate::importers::JsonImporter.import(&ctx, &text).unwrap();
        let r = &back.records[0];
        assert_eq!(
            (r.ts, r.level, r.message.as_str()),
            (Some(1_791_972_000_123), Level::Error, "Order \"88\", failed")
        );
        assert_eq!(r.fields["Id"], 88);
        assert_eq!(r.fields["@odd"], "x");
        assert_eq!((ClefExporter.id(), ClefExporter.extension()), ("clef", "clef"));
        assert!(ClefExporter.name().contains("CLEF"));
    }

    #[test]
    fn clef_file_sink_appends_and_rolls() {
        let dir = std::env::temp_dir().join(format!("imt-sink-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let sink = ClefFileSink::new(&dir, "monitor", 200, 3).unwrap();
        for _ in 0..6 {
            sink.write(&[rec()]).unwrap();
        }
        let mut files: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        files.sort();
        assert_eq!(files, ["monitor-1.clef", "monitor-2.clef", "monitor.clef"]);
        let line = std::fs::read_to_string(sink.current()).unwrap();
        assert_eq!(serde_json::from_str::<Value>(line.lines().next().unwrap()).unwrap()["Id"], 88);

        let single = ClefFileSink::new(dir.join("one"), "m", 1, 1).unwrap();
        single.write(&[rec()]).unwrap();
        single.write(&[rec()]).unwrap();
        assert_eq!(std::fs::read_to_string(single.current()).unwrap().lines().count(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
