//! Writing events as CLEF (Serilog compact JSON, one event per line): what collectors (the process monitor, the
//! event-log follower) write and IMT opens like any log.

use crate::model::{Level, Record};
use anyhow::Result;
use chrono::{SecondsFormat, TimeZone, Utc};
use serde_json::{Map, Value};
use std::io::Write;

/// ISO 8601 UTC with milliseconds, or "" without a time.
pub fn iso(ts: Option<i64>) -> String {
    ts.and_then(|ms| Utc.timestamp_millis_opt(ms).single())
        .map(|t| t.to_rfc3339_opts(SecondsFormat::Millis, true))
        .unwrap_or_default()
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

/// Appends events as CLEF lines to `<dir>/<stem>.clef`, rolling over to `<stem>-1.clef`, `<stem>-2.clef`, ...
/// when a file exceeds `max_bytes`, and keeping at most `keep` files.
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

/// Runs `f`, turning a panic into an error ("{what} crashed: ..."), for plugin code that runs work of its own
/// (a collector's writer thread).
pub fn catch<T>(what: &str, f: impl FnOnce() -> Result<T>) -> Result<T> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(result) => result,
        Err(payload) => Err(anyhow::anyhow!("{what} crashed: {}", imt_ipc::panic_message(payload.as_ref()))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn records_become_clef_objects() {
        let v = clef_value(&rec());
        assert_eq!(v["@t"], "2026-10-14T10:00:00.123Z");
        assert_eq!(v["@l"], "Error");
        assert_eq!(v["@mt"], "Order {Id} failed");
        assert_eq!(v["@@odd"], "x");
        assert!(clef_value(&Record::new(1)).get("@t").is_none());
        assert_eq!(iso(None), "");
    }

    #[test]
    fn clef_file_sink_appends_and_rolls() {
        let dir = std::env::temp_dir().join(format!("imt-api-sink-{}", std::process::id()));
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
        let single = ClefFileSink::new(dir.join("one"), "m", 1, 1).unwrap();
        single.write(&[rec()]).unwrap();
        single.write(&[rec()]).unwrap();
        assert_eq!(std::fs::read_to_string(single.current()).unwrap().lines().count(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn panics_become_errors() {
        let e = catch::<()>("the writer", || panic!("boom")).unwrap_err();
        assert_eq!(e.to_string(), "the writer crashed: boom");
        assert_eq!(catch("x", || Ok(3)).unwrap(), 3);
    }
}
