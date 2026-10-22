//! Command line of the headless monitor:
//! `debrief-monitor --config monitor.json --out ./monitor-logs [--duration 10m]`.

use crate::ProcessMonitor;
use anyhow::{anyhow, bail, Context, Result};
use debrief_core::export::ClefFileSink;
use debrief_core::plugin::Collector;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub const USAGE: &str = "usage: debrief-monitor --config <monitor.json> --out <dir> [--duration <10m|30s|2h|1500ms>]";
const ROTATE_BYTES: u64 = 10 * 1024 * 1024;
const KEEP_FILES: usize = 10;

/// Parses `500ms`, `30s`, `10m`, `2h` (a bare number means seconds).
pub fn parse_duration(s: &str) -> Result<Duration> {
    let s = s.trim();
    let (num, unit_ms) = if let Some(n) = s.strip_suffix("ms") {
        (n, 1)
    } else if let Some(n) = s.strip_suffix('s') {
        (n, 1000)
    } else if let Some(n) = s.strip_suffix('m') {
        (n, 60_000)
    } else if let Some(n) = s.strip_suffix('h') {
        (n, 3_600_000)
    } else {
        (s, 1000)
    };
    let n: u64 = num
        .trim()
        .parse()
        .map_err(|_| anyhow!("invalid duration \"{s}\"; use e.g. 30s, 10m or 2h"))?;
    Ok(Duration::from_millis(n.saturating_mul(unit_ms)))
}

/// Parsed command line.
#[derive(Debug, PartialEq, Eq)]
pub struct Args {
    pub config: PathBuf,
    pub out: PathBuf,
    pub duration: Option<Duration>,
}

pub fn parse_args(args: &[String]) -> Result<Args> {
    let (mut config, mut out, mut duration) = (None, None, None);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| anyhow!("{name} needs a value\n{USAGE}"));
        match a.as_str() {
            "--config" => config = Some(PathBuf::from(value("--config")?)),
            "--out" => out = Some(PathBuf::from(value("--out")?)),
            "--duration" => duration = Some(parse_duration(value("--duration")?)?),
            other => bail!("unknown argument \"{other}\"\n{USAGE}"),
        }
    }
    Ok(Args {
        config: config.ok_or_else(|| anyhow!("--config is required\n{USAGE}"))?,
        out: out.ok_or_else(|| anyhow!("--out is required\n{USAGE}"))?,
        duration,
    })
}

/// Runs the monitor until `stop` is set or the optional duration elapses.
pub fn run(args: &[String], stop: &AtomicBool) -> Result<()> {
    let args = parse_args(args)?;
    let text = std::fs::read_to_string(&args.config).with_context(|| format!("cannot read {}", args.config.display()))?;
    let config: serde_json::Value = serde_json::from_str(&text).with_context(|| format!("{} is not valid JSON", args.config.display()))?;
    let sink = ClefFileSink::new(&args.out, "process-monitor", ROTATE_BYTES, KEEP_FILES)?;
    let local_stop = AtomicBool::new(false);
    std::thread::scope(|s| {
        s.spawn(|| {
            let start = Instant::now();
            while !stop.load(Ordering::Relaxed) && !local_stop.load(Ordering::Relaxed) {
                if args.duration.is_some_and(|d| start.elapsed() >= d) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            local_stop.store(true, Ordering::Relaxed);
        });
        let result = ProcessMonitor.run(&config, &sink, &local_stop);
        local_stop.store(true, Ordering::Relaxed);
        result
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("500ms").unwrap(), Duration::from_millis(500));
        assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_duration("10m").unwrap(), Duration::from_secs(600));
        assert_eq!(parse_duration("2h").unwrap(), Duration::from_secs(7200));
        assert_eq!(parse_duration("5").unwrap(), Duration::from_secs(5));
        assert!(parse_duration("soon").is_err());
    }

    #[test]
    fn arguments() {
        let a = parse_args(&v(&["--config", "c.json", "--out", "o", "--duration", "1s"])).unwrap();
        assert_eq!(
            a,
            Args {
                config: "c.json".into(),
                out: "o".into(),
                duration: Some(Duration::from_secs(1))
            }
        );
        for bad in [
            v(&[]),
            v(&["--config", "c"]),
            v(&["--out", "o"]),
            v(&["--config"]),
            v(&["--bogus"]),
            v(&["--config", "c", "--out", "o", "--duration", "x"]),
        ] {
            assert!(parse_args(&bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn run_reports_unreadable_and_invalid_config() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out").to_string_lossy().into_owned();
        let stop = AtomicBool::new(false);
        let missing = dir.path().join("none.json").to_string_lossy().into_owned();
        assert!(run(&v(&["--config", &missing, "--out", &out]), &stop)
            .unwrap_err()
            .to_string()
            .contains("cannot read"));
        let bad = dir.path().join("bad.json");
        std::fs::write(&bad, "{nope").unwrap();
        let bad = bad.to_string_lossy().into_owned();
        assert!(run(&v(&["--config", &bad, "--out", &out]), &stop)
            .unwrap_err()
            .to_string()
            .contains("not valid JSON"));
    }

    #[test]
    fn run_stops_after_duration_and_writes_events() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("m.json");
        std::fs::write(
            &cfg,
            format!(r#"{{"targets":[{{"pid":{}}}],"intervalMs":250}}"#, std::process::id()),
        )
        .unwrap();
        let out = dir.path().join("logs");
        let stop = AtomicBool::new(false);
        run(
            &v(&[
                "--config",
                cfg.to_str().unwrap(),
                "--out",
                out.to_str().unwrap(),
                "--duration",
                "700ms",
            ]),
            &stop,
        )
        .unwrap();
        let text = std::fs::read_to_string(out.join("process-monitor.clef")).unwrap();
        assert!(text.contains("Process stats"), "{text}");
        // An external stop also ends the run.
        let stop = AtomicBool::new(true);
        run(&v(&["--config", cfg.to_str().unwrap(), "--out", out.to_str().unwrap()]), &stop).unwrap();
    }

    #[test]
    fn invalid_monitor_config_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("m.json");
        std::fs::write(&cfg, r#"{"targets":[]}"#).unwrap();
        let stop = AtomicBool::new(false);
        let err = run(
            &v(&["--config", cfg.to_str().unwrap(), "--out", dir.path().to_str().unwrap()]),
            &stop,
        )
        .unwrap_err();
        assert!(err.to_string().contains("at least one"));
    }
}
