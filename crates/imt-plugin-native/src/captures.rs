//! Captures in progress and finished, as the tools report them (`status`), and the thread each one records on.
//!
//! A capture runs its body on its own thread ([`spawn`]) and watches a [`Control`] (stop keeps the file, cancel
//! discards it). The registry is per process: the normal plugin process holds sampling captures, the elevated
//! one its ETW captures.

use crate::{Finished, NativeError};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Finished captures kept in the list.
const KEEP_FINISHED: usize = 32;
/// Captures recording at once (per process).
pub const MAX_RUNNING: usize = 4;

/// Shared between a capture's thread and the registry.
#[derive(Debug, Default)]
pub struct Control {
    stop: AtomicBool,
    cancel: AtomicBool,
    bytes: AtomicU64,
    samples: AtomicU64,
    done: AtomicBool,
}

impl Control {
    /// Whether the capture should end (stop or cancel).
    pub fn stopping(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }

    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::Release);
    }

    pub fn request_cancel(&self) {
        self.cancel.store(true, Ordering::Release);
        self.request_stop();
    }

    pub fn set_bytes(&self, n: u64) {
        self.bytes.store(n, Ordering::Release);
    }

    pub fn set_samples(&self, n: u64) {
        self.samples.store(n, Ordering::Release);
    }
}

/// A capture's state as the UI shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CaptureState {
    Recording,
    Stopping,
    Done,
    Failed,
    Cancelled,
}

/// A capture (the `status` DTO, `NativeCaptureStatus` in api.types.ts).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureStatus {
    pub id: String,
    pub pid: u32,
    pub process_name: String,
    /// `sampling` or `etw`.
    pub mode: String,
    pub state: CaptureState,
    pub elapsed_ms: u64,
    pub bytes: u64,
    /// Samples taken so far (sampling mode).
    pub samples: u64,
    /// The NetTrace file once done; open it with `profiling_open`.
    pub path: Option<String>,
    pub error: Option<String>,
    /// The error is missing rights.
    pub needs_elevation: bool,
    pub truncated: bool,
    pub notes: Vec<String>,
}

struct Entry {
    status: CaptureStatus,
    ctl: Arc<Control>,
    started: Instant,
    thread: Option<JoinHandle<Result<Finished, NativeError>>>,
}

/// The captures of this process.
#[derive(Default)]
pub struct Captures {
    entries: Mutex<BTreeMap<u64, Entry>>,
    next: AtomicU64,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn key(id: &str) -> Option<u64> {
    id.rsplit('-').next()?.parse().ok()
}

impl Captures {
    /// Starts `body` on its own thread as capture `<prefix>-<n>` of `pid`.
    pub fn spawn(
        &self,
        prefix: &str,
        pid: u32,
        process_name: &str,
        mode: &str,
        body: impl FnOnce(Arc<Control>) -> Result<Finished, NativeError> + Send + 'static,
    ) -> Result<CaptureStatus, NativeError> {
        self.reap();
        if self.running() >= MAX_RUNNING {
            return Err(NativeError::Unsupported(format!(
                "{MAX_RUNNING} captures are already recording; stop one first"
            )));
        }
        let n = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let ctl = Arc::new(Control::default());
        let c = ctl.clone();
        let thread = std::thread::Builder::new()
            .name(format!("imt-native-{prefix}-{n}"))
            .spawn(move || {
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| body(c.clone())))
                    .unwrap_or_else(|_| Err(NativeError::Failed("the capture failed unexpectedly (a bug in IMT)".into())));
                c.done.store(true, Ordering::Release);
                r
            })?;
        let status = CaptureStatus {
            id: format!("{prefix}-{n}"),
            pid,
            process_name: process_name.to_string(),
            mode: mode.to_string(),
            state: CaptureState::Recording,
            elapsed_ms: 0,
            bytes: 0,
            samples: 0,
            path: None,
            error: None,
            needs_elevation: false,
            truncated: false,
            notes: Vec::new(),
        };
        lock(&self.entries).insert(
            n,
            Entry {
                status: status.clone(),
                ctl,
                started: Instant::now(),
                thread: Some(thread),
            },
        );
        Ok(status)
    }

    fn running(&self) -> usize {
        lock(&self.entries).values().filter(|e| e.thread.is_some()).count()
    }

    /// Whether capture `id` is still recording.
    pub fn is_running(&self, id: &str) -> bool {
        key(id).is_some_and(|k| lock(&self.entries).get(&k).is_some_and(|e| e.thread.is_some()))
    }

    /// Collects the results of captures whose thread ended.
    pub fn reap(&self) {
        let mut entries = lock(&self.entries);
        for e in entries.values_mut() {
            let finished = e.ctl.done.load(Ordering::Acquire);
            if !finished {
                continue;
            }
            if let Some(t) = e.thread.take() {
                let r = t
                    .join()
                    .unwrap_or_else(|_| Err(NativeError::Failed("the capture thread failed".into())));
                apply(&mut e.status, r, e.ctl.cancelled());
            }
        }
        // Keep the newest finished ones.
        let finished: Vec<u64> = entries.iter().filter(|(_, e)| e.thread.is_none()).map(|(k, _)| *k).collect();
        for k in finished.iter().take(finished.len().saturating_sub(KEEP_FINISHED)) {
            entries.remove(k);
        }
    }

    /// Every capture, newest first.
    pub fn list(&self) -> Vec<CaptureStatus> {
        self.reap();
        let mut entries = lock(&self.entries);
        entries
            .values_mut()
            .rev()
            .map(|e| {
                if e.thread.is_some() {
                    e.status.elapsed_ms = e.started.elapsed().as_millis() as u64;
                    e.status.bytes = e.ctl.bytes.load(Ordering::Acquire);
                    e.status.samples = e.ctl.samples.load(Ordering::Acquire);
                    if e.ctl.stopping() {
                        e.status.state = CaptureState::Stopping;
                    }
                }
                e.status.clone()
            })
            .collect()
    }

    /// One capture.
    pub fn get(&self, id: &str) -> Option<CaptureStatus> {
        self.list().into_iter().find(|s| s.id == id)
    }

    /// Stops capture `id`, waits for its file (at most `wait`) and returns its status.
    pub fn stop(&self, id: &str, wait: Duration) -> Result<CaptureStatus, NativeError> {
        self.end(id, false, wait)
    }

    /// Cancels capture `id` (its file is deleted).
    pub fn cancel(&self, id: &str, wait: Duration) -> Result<CaptureStatus, NativeError> {
        self.end(id, true, wait)
    }

    fn end(&self, id: &str, cancel: bool, wait: Duration) -> Result<CaptureStatus, NativeError> {
        let k = key(id).ok_or_else(|| NativeError::Unsupported(format!("no capture \"{id}\"")))?;
        let ctl = lock(&self.entries)
            .get(&k)
            .map(|e| e.ctl.clone())
            .ok_or_else(|| NativeError::Unsupported(format!("no capture \"{id}\"")))?;
        if cancel {
            ctl.request_cancel();
        } else {
            ctl.request_stop();
        }
        let until = Instant::now() + wait;
        while !ctl.done.load(Ordering::Acquire) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(20));
        }
        self.get(id).ok_or_else(|| NativeError::Unsupported(format!("no capture \"{id}\"")))
    }

    /// Stops every capture (the process is shutting down).
    pub fn stop_all(&self) {
        for e in lock(&self.entries).values() {
            e.ctl.request_stop();
        }
    }
}

fn apply(s: &mut CaptureStatus, r: Result<Finished, NativeError>, cancelled: bool) {
    match r {
        Ok(f) => {
            s.state = CaptureState::Done;
            s.bytes = f.bytes;
            s.path = Some(f.path.to_string_lossy().into_owned());
            s.truncated = f.truncated;
            s.notes = f.notes;
        }
        Err(NativeError::Cancelled) => s.state = CaptureState::Cancelled,
        Err(_) if cancelled => s.state = CaptureState::Cancelled,
        Err(e) => {
            s.state = CaptureState::Failed;
            s.needs_elevation = matches!(e, NativeError::Denied(_));
            s.error = Some(e.to_string());
        }
    }
}

/// A unique capture file `<dir>/<name>` (see [`capture_name`]).
pub fn capture_file(dir: &std::path::Path, process_name: &str, pid: u32) -> PathBuf {
    dir.join(capture_name(process_name, pid))
}

/// A unique capture file name `<stem>-<pid>-<stamp>-<n>.nettrace` (the stem is the process name, made safe: only
/// letters, digits, `-` and `_`, so it is also a valid output name for the host to save, ADR 0017 §7).
pub fn capture_name(process_name: &str, pid: u32) -> String {
    static N: AtomicU64 = AtomicU64::new(1);
    let stem: String = process_name
        .trim_end_matches(".exe")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    let stem = if stem.is_empty() { "process".to_string() } else { stem };
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    format!("{stem}-{pid}-{stamp}-{}.nettrace", N.fetch_add(1, Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finished(path: &str) -> Finished {
        Finished {
            path: PathBuf::from(path),
            bytes: 10,
            truncated: false,
            notes: vec!["n".into()],
        }
    }

    #[test]
    fn captures_stop_cancel_fail_and_are_listed_newest_first() {
        let c = Captures::default();
        let s = c
            .spawn("s", 7, "busy.exe", "sampling", |ctl| {
                while !ctl.stopping() {
                    ctl.set_bytes(5);
                    ctl.set_samples(3);
                    std::thread::sleep(Duration::from_millis(5));
                }
                if ctl.cancelled() {
                    return Err(NativeError::Cancelled);
                }
                Ok(finished("a.nettrace"))
            })
            .unwrap();
        assert_eq!(s.state, CaptureState::Recording);
        assert!(c.is_running(&s.id));
        std::thread::sleep(Duration::from_millis(30));
        let live = c.get(&s.id).unwrap();
        assert_eq!((live.bytes, live.samples), (5, 3));
        let done = c.stop(&s.id, Duration::from_secs(5)).unwrap();
        assert_eq!(done.state, CaptureState::Done);
        assert_eq!(done.path.as_deref(), Some("a.nettrace"));
        assert!(!c.is_running(&s.id));

        let x = c
            .spawn("s", 7, "busy.exe", "sampling", |ctl| {
                while !ctl.stopping() {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(NativeError::Cancelled)
            })
            .unwrap();
        assert_eq!(c.cancel(&x.id, Duration::from_secs(5)).unwrap().state, CaptureState::Cancelled);

        let d = c
            .spawn("e", 9, "svc.exe", "etw", |_| Err(NativeError::Denied("needs administrator".into())))
            .unwrap();
        let p = c.spawn("e", 9, "svc.exe", "etw", |_| panic!("boom")).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        let denied = c.get(&d.id).unwrap();
        assert_eq!(denied.state, CaptureState::Failed);
        assert!(denied.needs_elevation);
        let crashed = c.get(&p.id).unwrap();
        assert!(crashed.error.unwrap().contains("bug in IMT"));
        let ids: Vec<String> = c.list().into_iter().map(|s| s.id).collect();
        assert_eq!(ids, [p.id.clone(), d.id.clone(), x.id.clone(), s.id.clone()]);
        assert!(c.stop("nope", Duration::ZERO).is_err());
        assert!(c.stop("s-999", Duration::ZERO).is_err());
        c.stop_all();
    }

    #[test]
    fn running_captures_are_bounded_and_files_are_named_safely() {
        let c = Captures::default();
        let block = Arc::new(AtomicBool::new(false));
        for _ in 0..MAX_RUNNING {
            let b = block.clone();
            c.spawn("s", 1, "a", "sampling", move |_| {
                while !b.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Ok(finished("x"))
            })
            .unwrap();
        }
        assert!(c.spawn("s", 1, "a", "sampling", |_| Ok(finished("x"))).is_err());
        block.store(true, Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(100));
        c.reap();
        assert!(c.spawn("s", 1, "a", "sampling", |_| Ok(finished("x"))).is_ok());
        let f = capture_file(std::path::Path::new("d"), "my app/..\\x.exe", 42);
        let name = f.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("my_app____x-42-") && name.ends_with(".nettrace"), "{name}");
        assert!(capture_file(std::path::Path::new("d"), "", 1)
            .to_string_lossy()
            .contains("process-1-"));
        // The host saves the elevated recorder's capture under this name, so it must pass the host's name check
        // whatever the process is called, and stay unique.
        for process in ["my app/..\\x.exe", "", "CON.exe", &"é".repeat(300), "a.b.c"] {
            let name = capture_name(process, 7);
            imt_plugin_api::protocol::check_output_name("file name", &name).unwrap_or_else(|e| panic!("{process:?} -> {name}: {e}"));
        }
        assert_ne!(capture_name("a", 1), capture_name("a", 1));
    }
}
