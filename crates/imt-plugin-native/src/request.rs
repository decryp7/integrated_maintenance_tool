//! A capture request as the UI sends it (`start` of either tool), parsed once into typed values.

use imt_symbols::{parse_search_path, ServerConfig, SymbolConfig};
use serde::Deserialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Default sampling rate (per thread).
pub const DEFAULT_RATE_HZ: u32 = 100;
/// Highest sampling rate.
pub const MAX_RATE_HZ: u32 = 1000;
/// Longest capture in seconds.
pub const MAX_DURATION_SEC: u64 = 3600;
/// Longest symbol path.
pub const MAX_SYMBOL_PATH: usize = 4096;

/// How a capture records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureKind {
    /// Suspend-and-sample at `rate_hz` per thread (no rights needed).
    Sampling { rate_hz: u32 },
    /// ETW (administrator); `heap` adds the heap provider's allocations.
    Etw { heap: bool },
}

impl CaptureKind {
    /// `sampling` or `etw` (the DTO's `mode`).
    pub fn mode(&self) -> &'static str {
        match self {
            CaptureKind::Sampling { .. } => "sampling",
            CaptureKind::Etw { .. } => "etw",
        }
    }
}

/// Where symbols come from for one capture.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SymbolOptions {
    pub paths: Vec<PathBuf>,
    /// Ask the Microsoft symbol server for missing PDBs (off unless the user enabled it).
    pub server: bool,
}

impl SymbolOptions {
    /// Parses the user's symbol path text (absolute folders separated by `;`).
    pub fn parse(path: &str, server: bool) -> Result<SymbolOptions, String> {
        if path.chars().count() > MAX_SYMBOL_PATH {
            return Err(format!("the symbol path may have at most {MAX_SYMBOL_PATH} characters"));
        }
        Ok(SymbolOptions {
            paths: parse_search_path(path).map_err(|e| e.to_string())?,
            server,
        })
    }

    /// The symbol configuration, with the cache under `data_dir/symbols`.
    pub fn config(&self, data_dir: Option<&Path>) -> SymbolConfig {
        let cache = data_dir.map(|d| d.join("symbols"));
        SymbolConfig {
            paths: self.paths.clone(),
            server: (self.server && cache.is_some()).then(ServerConfig::microsoft),
            cache,
        }
    }
}

/// A validated capture request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureRequest {
    pub pid: u32,
    /// `None`: until stopped (bounded by [`crate::Limits::max_duration`]).
    pub duration: Option<Duration>,
    pub kind: CaptureKind,
    pub symbols: SymbolOptions,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Raw {
    pid: u32,
    duration_sec: Option<u64>,
    rate_hz: Option<u32>,
    heap: Option<bool>,
    symbol_path: Option<String>,
    symbol_server: Option<bool>,
}

impl CaptureRequest {
    /// Parses `start` arguments for sampling (`etw == false`) or ETW.
    pub fn parse(args: &Value, etw: bool) -> Result<CaptureRequest, String> {
        let raw: Raw = serde_json::from_value(args.clone()).map_err(|e| format!("invalid capture request: {e}"))?;
        if raw.pid == 0 {
            return Err("choose a process".into());
        }
        if raw.pid == std::process::id() {
            return Err("IMT cannot profile its own plugin process".into());
        }
        let duration = match raw.duration_sec {
            None => None,
            Some(s) if (1..=MAX_DURATION_SEC).contains(&s) => Some(Duration::from_secs(s)),
            Some(_) => return Err(format!("the duration must be 1 to {MAX_DURATION_SEC} seconds")),
        };
        let kind = if etw {
            if raw.rate_hz.is_some() {
                return Err("the sampling rate applies to the sampling mode only".into());
            }
            CaptureKind::Etw {
                heap: raw.heap.unwrap_or(false),
            }
        } else {
            if raw.heap == Some(true) {
                return Err("heap allocations need the ETW mode (administrator)".into());
            }
            let rate_hz = raw.rate_hz.unwrap_or(DEFAULT_RATE_HZ);
            if !(1..=MAX_RATE_HZ).contains(&rate_hz) {
                return Err(format!("the sampling rate must be 1 to {MAX_RATE_HZ} per second"));
            }
            CaptureKind::Sampling { rate_hz }
        };
        let symbols = SymbolOptions::parse(raw.symbol_path.as_deref().unwrap_or(""), raw.symbol_server.unwrap_or(false))?;
        Ok(CaptureRequest {
            pid: raw.pid,
            duration,
            kind,
            symbols,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_both_modes_with_defaults() {
        let r = CaptureRequest::parse(&json!({ "pid": 42 }), false).unwrap();
        assert_eq!(r.kind, CaptureKind::Sampling { rate_hz: DEFAULT_RATE_HZ });
        assert_eq!(r.duration, None);
        assert_eq!(r.kind.mode(), "sampling");
        let r = CaptureRequest::parse(
            &json!({ "pid": 42, "durationSec": 30, "heap": true, "symbolPath": "C:\\syms;D:\\more", "symbolServer": true }),
            true,
        )
        .unwrap();
        assert_eq!(r.kind, CaptureKind::Etw { heap: true });
        assert_eq!(r.duration, Some(Duration::from_secs(30)));
        assert_eq!(r.symbols.paths.len(), 2);
        let cfg = r.symbols.config(Some(Path::new("C:\\data")));
        assert!(cfg.server.is_some() && cfg.cache.unwrap().ends_with("symbols"));
        assert!(r.symbols.config(None).server.is_none(), "no cache, no downloads");
        assert_eq!(r.kind.mode(), "etw");
    }

    #[test]
    fn refuses_what_does_not_fit_the_mode() {
        for (args, etw, what) in [
            (json!({ "pid": 0 }), false, "choose a process"),
            (json!({ "pid": std::process::id() }), false, "its own"),
            (json!({ "pid": 1, "durationSec": 0 }), false, "duration"),
            (json!({ "pid": 1, "durationSec": 3601 }), true, "duration"),
            (json!({ "pid": 1, "rateHz": 0 }), false, "sampling rate"),
            (json!({ "pid": 1, "rateHz": 5000 }), false, "sampling rate"),
            (json!({ "pid": 1, "rateHz": 100 }), true, "sampling mode only"),
            (json!({ "pid": 1, "heap": true }), false, "ETW mode"),
            (json!({ "pid": 1, "symbolPath": "relative" }), false, "absolute"),
            (json!({ "pid": 1, "symbolPath": "srv*x" }), false, "symbol server"),
            (json!({ "pid": 1, "symbolPath": "C:\\".repeat(3000) }), false, "at most"),
            (json!({ "pid": 1, "extra": 1 }), false, "invalid capture request"),
        ] {
            let e = CaptureRequest::parse(&args, etw).unwrap_err();
            assert!(e.contains(what), "{args}: {e}");
        }
    }
}
