//! Live system performance counters for the [`imt_api_perf::CounterSource`]
//! service. On Windows any counter path is read with PDH
//! ([`crate::path_counters`]); elsewhere the counters below are sampled with
//! `sysinfo` (the same library the process monitor samples processes with).
//! Counters carry their Performance Monitor names and meanings on every
//! platform, so consumers need no platform switch.
//!
//! | Counter | Value |
//! |---|---|
//! | `\Processor(_Total)\% Processor Time` | CPU use of the whole machine, 0-100 (needs a previous sample) |
//! | `\Memory\Available Bytes` / `\Memory\Available MBytes` | memory available to start new programs |
//! | `\Memory\% Committed Bytes In Use` | used share of physical memory, 0-100 |
//! | `\Paging File(_Total)\% Usage` | used share of swap; omitted when there is no swap |

use anyhow::{bail, Result};
use imt_api_perf::{CounterSample, CounterSource};
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use sysinfo::System;

const CPU: &str = "\\Processor(_Total)\\% Processor Time";
const AVAILABLE_BYTES: &str = "\\Memory\\Available Bytes";
const AVAILABLE_MB: &str = "\\Memory\\Available MBytes";
const MEMORY_IN_USE: &str = "\\Memory\\% Committed Bytes In Use";
const SWAP_USAGE: &str = "\\Paging File(_Total)\\% Usage";
const ALL: [&str; 5] = [CPU, AVAILABLE_BYTES, AVAILABLE_MB, MEMORY_IN_USE, SWAP_USAGE];

/// The platform's counter source: PDH for any counter path on Windows, `sysinfo` for the
/// counters above elsewhere. Both list the counters above.
pub fn system() -> std::sync::Arc<dyn CounterSource> {
    #[cfg(windows)]
    {
        let opener: crate::path_counters::Opener = Box::new(imt_monitor::counters::open);
        std::sync::Arc::new(crate::path_counters::PathCounters::new(opener, &ALL))
    }
    #[cfg(not(windows))]
    {
        std::sync::Arc::new(SystemCounters::new())
    }
}

/// One reading of the machine (the port tests replace).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Reading {
    /// `None` until a previous CPU sample exists (CPU use is a rate).
    pub cpu_percent: Option<f64>,
    pub available_bytes: u64,
    pub total_memory: u64,
    pub used_swap: u64,
    pub total_swap: u64,
}

/// Where readings come from.
pub trait Probe: Send {
    fn read(&mut self, want_cpu: bool) -> Reading;
}

/// The real probe.
pub struct SysinfoProbe {
    sys: System,
    /// When CPU usage was last refreshed: usage is only meaningful after
    /// `sysinfo::MINIMUM_CPU_UPDATE_INTERVAL` has passed since.
    cpu_since: Option<Instant>,
}

impl Default for SysinfoProbe {
    fn default() -> Self {
        SysinfoProbe {
            sys: System::new(),
            cpu_since: None,
        }
    }
}

impl Probe for SysinfoProbe {
    fn read(&mut self, want_cpu: bool) -> Reading {
        self.sys.refresh_memory();
        let mut cpu_percent = None;
        if want_cpu {
            let ready = self.cpu_since.is_some_and(|t| t.elapsed() >= sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
            if ready || self.cpu_since.is_none() {
                self.sys.refresh_cpu_usage();
                if ready {
                    cpu_percent = Some(f64::from(self.sys.global_cpu_usage()));
                }
                self.cpu_since = Some(Instant::now());
            }
        }
        Reading {
            cpu_percent,
            available_bytes: self.sys.available_memory(),
            total_memory: self.sys.total_memory(),
            used_swap: self.sys.used_swap(),
            total_swap: self.sys.total_swap(),
        }
    }
}

/// The [`CounterSource`] the tools plugin provides off Windows.
pub struct SystemCounters {
    probe: Mutex<Box<dyn Probe>>,
}

impl Default for SystemCounters {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemCounters {
    pub fn new() -> Self {
        Self::with_probe(Box::<SysinfoProbe>::default())
    }

    pub fn with_probe(probe: Box<dyn Probe>) -> Self {
        SystemCounters { probe: Mutex::new(probe) }
    }
}

fn percent(part: u64, whole: u64) -> Option<f64> {
    (whole > 0).then(|| (part as f64 / whole as f64 * 100.0).clamp(0.0, 100.0))
}

fn value(path: &str, r: &Reading) -> Option<f64> {
    match path {
        CPU => r.cpu_percent.map(|c| c.clamp(0.0, 100.0)),
        AVAILABLE_BYTES => Some(r.available_bytes as f64),
        AVAILABLE_MB => Some((r.available_bytes / (1024 * 1024)) as f64),
        MEMORY_IN_USE => percent(r.total_memory.saturating_sub(r.available_bytes), r.total_memory),
        _ => percent(r.used_swap, r.total_swap),
    }
    .filter(|v| v.is_finite())
}

impl CounterSource for SystemCounters {
    fn counters(&self) -> Result<Vec<String>> {
        Ok(ALL.iter().map(|c| c.to_string()).collect())
    }

    fn sample(&self, paths: &[String]) -> Result<Vec<CounterSample>> {
        if let Some(unknown) = paths.iter().find(|p| !ALL.contains(&p.as_str())) {
            bail!("unknown counter \"{unknown}\" (available: {})", ALL.join(", "));
        }
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        let reading = self
            .probe
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .read(paths.iter().any(|p| p == CPU));
        let at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(1)
            .max(1);
        Ok(paths
            .iter()
            .filter_map(|p| {
                value(p, &reading).map(|value| CounterSample {
                    path: p.clone(),
                    value,
                    at_ms,
                })
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scripted(Vec<Reading>);
    impl Probe for Scripted {
        fn read(&mut self, _: bool) -> Reading {
            if self.0.len() > 1 {
                self.0.remove(0)
            } else {
                self.0[0]
            }
        }
    }

    #[test]
    fn the_real_source_passes_the_contract() {
        let source = SystemCounters::new();
        imt_api_perf::contract_tests(&source).unwrap();
        // The platform's source (PDH on Windows) passes it too and lists the same counters.
        let platform = system();
        imt_api_perf::contract_tests(&*platform).unwrap();
        assert_eq!(platform.counters().unwrap(), source.counters().unwrap());
        // Memory is always there; CPU follows once enough time has passed.
        let all: Vec<String> = ALL.iter().map(|c| c.to_string()).collect();
        let first = source.sample(&all).unwrap();
        assert!(first.iter().any(|s| s.path == AVAILABLE_BYTES && s.value > 0.0));
        std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL + std::time::Duration::from_millis(50));
        let later = source.sample(&[CPU.to_string()]).unwrap();
        assert_eq!(later.len(), 1);
        assert!((0.0..=100.0).contains(&later[0].value));
    }

    #[test]
    fn values_are_derived_and_bounded() {
        let mb = 1024 * 1024;
        let r = Reading {
            cpu_percent: Some(140.0),
            available_bytes: 3 * mb,
            total_memory: 4 * mb,
            used_swap: 1,
            total_swap: 4,
        };
        let source = SystemCounters::with_probe(Box::new(Scripted(vec![r, Reading::default()])));
        let all: Vec<String> = ALL.iter().map(|c| c.to_string()).collect();
        let got: Vec<(String, f64)> = source.sample(&all).unwrap().into_iter().map(|s| (s.path, s.value)).collect();
        assert_eq!(
            got,
            [
                (CPU.to_string(), 100.0),
                (AVAILABLE_BYTES.to_string(), (3 * mb) as f64),
                (AVAILABLE_MB.to_string(), 3.0),
                (MEMORY_IN_USE.to_string(), 25.0),
                (SWAP_USAGE.to_string(), 25.0),
            ]
        );
        // No CPU baseline, no memory and no swap: only what can be computed.
        let got: Vec<String> = source.sample(&all).unwrap().into_iter().map(|s| s.path).collect();
        assert_eq!(got, [AVAILABLE_BYTES, AVAILABLE_MB]);
        let e = source.sample(&["\\Nope\\X".to_string()]).unwrap_err().to_string();
        assert!(e.starts_with("unknown counter \"\\Nope\\X\""), "{e}");
        assert!(imt_api_perf::contract_tests(&SystemCounters::default()).is_ok());
    }

    #[test]
    fn a_poisoned_probe_still_samples() {
        let source = std::sync::Arc::new(SystemCounters::new());
        let s = source.clone();
        let _ = std::thread::spawn(move || {
            let _guard = s.probe.lock().unwrap();
            panic!("poison");
        })
        .join();
        assert!(!source.sample(&[AVAILABLE_BYTES.to_string()]).unwrap().is_empty());
    }
}
