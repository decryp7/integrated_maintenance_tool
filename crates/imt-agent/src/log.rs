//! The agent's own log: CLEF lines (Serilog compact JSON, which IMT opens
//! directly) in rolling files with a size, count and age limit, as the app's
//! diagnostics logs. Logging never fails the caller: write errors go to stderr.

use crate::config::{LogLevel, LogLimits};
use imt_core::export::{ClefFileSink, EventSink};
use imt_core::model::{Level, Record};
use serde_json::Value;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

/// File name stem of the log files (`imt-agent.clef`, `imt-agent-1.clef`, ...).
pub const LOG_STEM: &str = "imt-agent";

pub struct AgentLog {
    sink: Box<dyn EventSink>,
    min: Level,
    /// Also print to stderr (interactive CLI use).
    echo: bool,
    /// Lines that could not be written (shown once on stderr).
    failed: Mutex<bool>,
}

fn level_of(l: LogLevel) -> Level {
    match l {
        LogLevel::Debug => Level::Debug,
        LogLevel::Information => Level::Information,
        LogLevel::Warning => Level::Warning,
        LogLevel::Error => Level::Error,
    }
}

impl AgentLog {
    /// Logs to rolling files in `dir`; deletes log files older than the limit.
    pub fn open(dir: &Path, limits: &LogLimits, echo: bool) -> anyhow::Result<AgentLog> {
        let sink = ClefFileSink::new(dir, LOG_STEM, limits.max_file_bytes, limits.keep_files)?;
        prune_old(dir, Duration::from_secs(u64::from(limits.max_age_days) * 86_400), SystemTime::now());
        Ok(AgentLog::with_sink(Box::new(sink), limits.level, echo))
    }

    pub fn with_sink(sink: Box<dyn EventSink>, level: LogLevel, echo: bool) -> AgentLog {
        AgentLog {
            sink,
            min: level_of(level),
            echo,
            failed: Mutex::new(false),
        }
    }

    /// Writes one event; `props` become CLEF properties.
    pub fn event(&self, level: Level, message: &str, props: &[(&str, Value)]) {
        if level < self.min {
            return;
        }
        let mut r = Record::new(0);
        r.ts = Some(chrono::Utc::now().timestamp_millis());
        r.level = level;
        r.message = message.to_string();
        r.fields.insert("SourceContext".into(), Value::from("IMT.Agent"));
        r.fields.insert("Application".into(), Value::from("IMT Agent"));
        r.fields.insert("Version".into(), Value::from(env!("CARGO_PKG_VERSION")));
        r.fields.insert("ProcessId".into(), Value::from(std::process::id()));
        for (k, v) in props {
            r.fields.insert((*k).to_string(), v.clone());
        }
        if self.echo {
            eprintln!("[{}] {message}", level.as_str());
        }
        if let Err(e) = self.sink.write(std::slice::from_ref(&r)) {
            let mut failed = self.failed.lock().unwrap_or_else(|p| p.into_inner());
            if !*failed {
                eprintln!("imt-agent: cannot write the log: {e:#}");
                *failed = true;
            }
        }
    }

    pub fn info(&self, message: &str, props: &[(&str, Value)]) {
        self.event(Level::Information, message, props)
    }
    pub fn warn(&self, message: &str, props: &[(&str, Value)]) {
        self.event(Level::Warning, message, props)
    }
    pub fn error(&self, message: &str, props: &[(&str, Value)]) {
        self.event(Level::Error, message, props)
    }
    pub fn debug(&self, message: &str, props: &[(&str, Value)]) {
        self.event(Level::Debug, message, props)
    }
}

/// Deletes `imt-agent*.clef` files in `dir` last modified before `now - max_age`.
pub fn prune_old(dir: &Path, max_age: Duration, now: SystemTime) -> usize {
    let Some(cutoff) = now.checked_sub(max_age) else { return 0 };
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    let mut removed = 0;
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !(name.starts_with(LOG_STEM) && name.ends_with(".clef")) {
            continue;
        }
        let old = e.metadata().and_then(|m| m.modified()).is_ok_and(|t| t < cutoff);
        if old && std::fs::remove_file(e.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::MemorySink;

    fn limits(dir_bytes: u64, keep: usize) -> LogLimits {
        LogLimits {
            level: LogLevel::Debug,
            max_file_bytes: dir_bytes,
            keep_files: keep,
            max_age_days: 30,
        }
    }

    #[test]
    fn writes_clef_lines_and_rolls_files() {
        let dir = tempfile::tempdir().unwrap();
        let log = AgentLog::open(dir.path(), &limits(400, 3), false).unwrap();
        for i in 0..20 {
            log.info("Run finished", &[("Schedule", Value::from(format!("s{i}")))]);
        }
        let current = std::fs::read_to_string(dir.path().join("imt-agent.clef")).unwrap();
        let line: Value = serde_json::from_str(current.lines().next().unwrap()).unwrap();
        assert_eq!(line["@m"], "Run finished");
        assert_eq!(line["SourceContext"], "IMT.Agent");
        assert!(line.get("@l").is_none(), "information has no @l in CLEF");
        assert!(line["@t"].as_str().unwrap().ends_with('Z'));
        assert!(dir.path().join("imt-agent-1.clef").exists());
        assert!(dir.path().join("imt-agent-2.clef").exists());
        assert!(!dir.path().join("imt-agent-3.clef").exists(), "keepFiles = 3 in total");
    }

    #[test]
    fn levels_filter_and_echo() {
        let sink = MemorySink::default();
        let log = AgentLog::with_sink(Box::new(sink.clone()), LogLevel::Warning, true);
        log.debug("d", &[]);
        log.info("i", &[]);
        log.warn("w", &[]);
        log.error("e", &[("Path", Value::from("/x"))]);
        let lines = sink.lines();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["@l"], "Warning");
        assert_eq!(lines[1]["Path"], "/x");
    }

    #[test]
    fn write_failures_do_not_propagate() {
        let sink = MemorySink::failing();
        let log = AgentLog::with_sink(Box::new(sink), LogLevel::Debug, false);
        log.info("a", &[]);
        log.info("b", &[]);
        assert!(*log.failed.lock().unwrap());
    }

    #[test]
    fn old_log_files_are_pruned() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("imt-agent-4.clef"), "x").unwrap();
        std::fs::write(dir.path().join("other.txt"), "x").unwrap();
        let later = SystemTime::now() + Duration::from_secs(40 * 86_400);
        assert_eq!(prune_old(dir.path(), Duration::from_secs(30 * 86_400), later), 1);
        assert!(dir.path().join("other.txt").exists());
        assert_eq!(prune_old(dir.path(), Duration::from_secs(u64::MAX / 2), SystemTime::UNIX_EPOCH), 0);
        assert_eq!(prune_old(&dir.path().join("none"), Duration::from_secs(1), later), 0);
        // A folder that cannot be created is an error at open.
        let file = dir.path().join("file");
        std::fs::write(&file, "x").unwrap();
        assert!(AgentLog::open(&file.join("sub"), &limits(10, 1), false).is_err());
    }
}
