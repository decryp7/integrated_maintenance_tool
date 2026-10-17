//! One run of a schedule at a time, even across processes (a service and a
//! manual `imt-agent once`): a lock file per schedule, created exclusively,
//! holding the owner's PID and process start time. A lock whose owner is no
//! longer running (or whose PID was reused by another process) is stale and
//! taken over; the file is removed when the [`RunLock`] is dropped.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
use thiserror::Error;

/// Tells whether a process is running.
pub trait ProcessProbe: Send + Sync {
    /// The process's start time (seconds since the epoch), or `None` when no such process runs.
    fn start_time(&self, pid: u32) -> Option<u64>;
}

/// [`ProcessProbe`] backed by `sysinfo` (all platforms).
pub struct SystemProbe;

impl ProcessProbe for SystemProbe {
    fn start_time(&self, pid: u32) -> Option<u64> {
        let mut sys = System::new();
        let p = Pid::from_u32(pid);
        sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[p]), true, ProcessRefreshKind::nothing());
        sys.process(p).map(|proc| proc.start_time())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LockInfo {
    pub pid: u32,
    /// Start time of the owning process, to detect PID reuse.
    #[serde(default)]
    pub process_start: Option<u64>,
    pub started_ms: i64,
    pub schedule: String,
}

#[derive(Debug, Error, PartialEq)]
pub enum LockError {
    #[error("schedule \"{schedule}\" is already running (process {pid})")]
    Held { schedule: String, pid: u32, started_ms: i64 },
    #[error("cannot create the lock file {0}: {1}")]
    Io(String, String),
}

/// A held lock; released (file deleted) on drop.
#[derive(Debug)]
pub struct RunLock {
    path: PathBuf,
}

impl RunLock {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Takes the lock of `schedule` in `dir`, or reports who holds it.
    pub fn acquire(dir: &Path, schedule: &str, now_ms: i64, probe: &dyn ProcessProbe) -> Result<RunLock, LockError> {
        std::fs::create_dir_all(dir).map_err(|e| LockError::Io(dir.display().to_string(), e.to_string()))?;
        let path = dir.join(format!("{}.lock", imt_collect::safe_segment(schedule)));
        let me = std::process::id();
        let info = LockInfo {
            pid: me,
            process_start: probe.start_time(me),
            started_ms: now_ms,
            schedule: schedule.to_string(),
        };
        // Second attempt after removing a stale lock.
        for _ in 0..2 {
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut f) => {
                    let text = serde_json::to_string(&info).unwrap_or_default();
                    f.write_all(text.as_bytes())
                        .map_err(|e| LockError::Io(path.display().to_string(), e.to_string()))?;
                    return Ok(RunLock { path });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let holder = std::fs::read_to_string(&path)
                        .ok()
                        .and_then(|t| serde_json::from_str::<LockInfo>(&t).ok());
                    match holder {
                        Some(h) if is_alive(&h, probe) => {
                            return Err(LockError::Held {
                                schedule: schedule.to_string(),
                                pid: h.pid,
                                started_ms: h.started_ms,
                            })
                        }
                        // Stale (owner gone) or unreadable (crash while writing): take over.
                        _ => {
                            let _ = std::fs::remove_file(&path);
                        }
                    }
                }
                Err(e) => return Err(LockError::Io(path.display().to_string(), e.to_string())),
            }
        }
        Err(LockError::Io(
            path.display().to_string(),
            "another process keeps recreating it".into(),
        ))
    }
}

fn is_alive(h: &LockInfo, probe: &dyn ProcessProbe) -> bool {
    match (probe.start_time(h.pid), h.process_start) {
        (None, _) => false,
        (Some(now), Some(then)) => now == then,
        (Some(_), None) => true,
    }
}

impl Drop for RunLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Processes by PID with their start times.
    struct FakeProbe(HashMap<u32, u64>);

    impl ProcessProbe for FakeProbe {
        fn start_time(&self, pid: u32) -> Option<u64> {
            self.0.get(&pid).copied()
        }
    }

    fn write_lock(dir: &Path, info: &LockInfo) {
        std::fs::write(dir.join("nightly.lock"), serde_json::to_string(info).unwrap()).unwrap();
    }

    #[test]
    fn exclusive_while_held_and_released_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let probe = FakeProbe([(std::process::id(), 100)].into());
        let lock = RunLock::acquire(dir.path(), "nightly", 1, &probe).unwrap();
        assert!(lock.path().exists());
        let held = RunLock::acquire(dir.path(), "nightly", 2, &probe).unwrap_err();
        assert_eq!(
            held,
            LockError::Held {
                schedule: "nightly".into(),
                pid: std::process::id(),
                started_ms: 1
            }
        );
        assert!(held.to_string().contains("already running"));
        // Other schedules are independent.
        let other = RunLock::acquire(dir.path(), "hourly", 3, &probe).unwrap();
        drop(lock);
        assert!(!dir.path().join("nightly.lock").exists());
        RunLock::acquire(dir.path(), "nightly", 4, &probe).unwrap();
        drop(other);
    }

    #[test]
    fn stale_locks_are_taken_over() {
        let dir = tempfile::tempdir().unwrap();
        let probe = FakeProbe([(std::process::id(), 100), (42, 7)].into());
        let info = |pid, start| LockInfo {
            pid,
            process_start: start,
            started_ms: 0,
            schedule: "nightly".into(),
        };
        // Owner gone.
        write_lock(dir.path(), &info(999_999, Some(1)));
        drop(RunLock::acquire(dir.path(), "nightly", 1, &probe).unwrap());
        // PID reused by a process that started later.
        write_lock(dir.path(), &info(42, Some(6)));
        drop(RunLock::acquire(dir.path(), "nightly", 1, &probe).unwrap());
        // Same PID and start time: alive. Without a start time, the PID decides.
        write_lock(dir.path(), &info(42, Some(7)));
        assert!(RunLock::acquire(dir.path(), "nightly", 1, &probe).is_err());
        write_lock(dir.path(), &info(42, None));
        assert!(RunLock::acquire(dir.path(), "nightly", 1, &probe).is_err());
        // Garbage (a crash while writing): taken over.
        std::fs::write(dir.path().join("nightly.lock"), "{not json").unwrap();
        drop(RunLock::acquire(dir.path(), "nightly", 1, &probe).unwrap());
    }

    #[test]
    fn real_probe_sees_this_process() {
        let probe = SystemProbe;
        assert!(probe.start_time(std::process::id()).is_some());
        assert!(probe.start_time(u32::MAX - 7).is_none());
        let dir = tempfile::tempdir().unwrap();
        let _held = RunLock::acquire(dir.path(), "x", 0, &probe).unwrap();
        assert!(matches!(RunLock::acquire(dir.path(), "x", 0, &probe), Err(LockError::Held { .. })));
    }

    #[test]
    fn io_errors_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f");
        std::fs::write(&file, "x").unwrap();
        let probe = FakeProbe(HashMap::new());
        assert!(matches!(RunLock::acquire(&file, "x", 0, &probe), Err(LockError::Io(..))));
        // The lock path is a folder: cannot be created nor removed.
        std::fs::create_dir_all(dir.path().join("d").join("x.lock")).unwrap();
        let err = RunLock::acquire(&dir.path().join("d"), "x", 0, &probe).unwrap_err();
        assert!(matches!(err, LockError::Io(..)), "{err:?}");
    }
}
