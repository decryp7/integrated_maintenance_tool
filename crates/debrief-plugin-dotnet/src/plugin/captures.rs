//! Running and finished captures (app-wide within a process): start, stop,
//! cancel, status, and the watcher that finalises a capture that ended by
//! itself. Wire DTOs mirror `DotnetCaptureStatus` in the UI.

use crate::{Collector, DotnetError, Finished, Limits, Preset, ProcessInfo, Recording};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Finished captures kept in the list.
const KEEP: usize = 50;
/// Longest recording a request may ask for.
pub const MAX_DURATION_SEC: u64 = 3600;

/// `DotnetCaptureState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum State {
    Recording,
    Stopping,
    Done,
    Failed,
    Cancelled,
}

/// `DotnetCaptureRequest`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub pid: u32,
    pub kind: Preset,
    /// `None`: until stopped (a heap snapshot ends by itself).
    pub duration_sec: Option<u64>,
}

/// `DotnetCaptureStatus`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub id: String,
    pub pid: u32,
    pub process_name: String,
    pub kind: Preset,
    pub state: State,
    pub elapsed_ms: u64,
    pub bytes: u64,
    pub path: Option<String>,
    pub error: Option<String>,
    /// Whether the error is missing rights (the UI offers "Restart as administrator").
    pub needs_elevation: bool,
    pub truncated: bool,
    pub notes: Vec<String>,
}

struct Entry {
    status: Status,
    started: Instant,
    recording: Option<Box<dyn Recording>>,
}

/// The capture list.
pub struct Captures {
    collector: Arc<dyn Collector>,
    entries: Mutex<VecDeque<Entry>>,
    next: Mutex<u64>,
}

impl Captures {
    pub fn new(collector: Arc<dyn Collector>) -> Arc<Captures> {
        Arc::new(Captures {
            collector,
            entries: Mutex::new(VecDeque::new()),
            next: Mutex::new(1),
        })
    }

    pub fn collector(&self) -> &Arc<dyn Collector> {
        &self.collector
    }

    fn lock(&self) -> MutexGuard<'_, VecDeque<Entry>> {
        self.entries.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Every capture, newest first, with live sizes and times.
    pub fn list(&self) -> Vec<Status> {
        let mut g = self.lock();
        g.iter_mut().rev().map(refresh).collect()
    }

    /// One capture.
    pub fn get(&self, id: &str) -> Option<Status> {
        self.lock().iter_mut().find(|e| e.status.id == id).map(refresh)
    }

    /// Starts a capture into `dir`; refuses a second capture of one process.
    pub fn start(&self, req: Request, dir: &Path, limits: Limits) -> Result<Status, String> {
        if req.duration_sec.is_some_and(|d| d == 0 || d > MAX_DURATION_SEC) {
            return Err(format!("the duration must be between 1 and {MAX_DURATION_SEC} seconds"));
        }
        if self
            .lock()
            .iter()
            .any(|e| e.status.pid == req.pid && matches!(e.status.state, State::Recording | State::Stopping))
        {
            return Err(format!("process {} is already being recorded; stop that capture first", req.pid));
        }
        let info = self
            .collector
            .list_processes()
            .into_iter()
            .find(|p| p.pid == req.pid)
            .ok_or_else(|| format!("process {} is not a running .NET process (refresh the list)", req.pid))?;
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot create the captures folder {}: {e}", dir.display()))?;
        let id = {
            let mut n = self.next.lock().unwrap_or_else(|p| p.into_inner());
            *n += 1;
            format!("dotnet-{}", *n - 1)
        };
        let path = dir.join(file_name(&info, req.kind, unix_secs()));
        let mut limits = limits;
        if let Some(d) = req.duration_sec {
            limits.max_duration = Duration::from_secs(d);
        }
        let mut status = Status {
            id: id.clone(),
            pid: info.pid,
            process_name: info.name.clone(),
            kind: req.kind,
            state: State::Recording,
            elapsed_ms: 0,
            bytes: 0,
            path: None,
            error: None,
            needs_elevation: false,
            truncated: false,
            notes: Vec::new(),
        };
        let recording = match self.collector.start(info.pid, req.kind, &path, limits) {
            Ok(r) => Some(r),
            Err(e) => {
                status.state = State::Failed;
                status.needs_elevation = matches!(e, DotnetError::Denied);
                status.error = Some(e.to_string());
                None
            }
        };
        let mut g = self.lock();
        g.push_back(Entry {
            status: status.clone(),
            started: Instant::now(),
            recording,
        });
        trim(&mut g);
        Ok(status)
    }

    /// Takes the recording of `id` and marks it stopping (or `None` when it is not running).
    fn take(&self, id: &str, state: State) -> Option<Box<dyn Recording>> {
        let mut g = self.lock();
        let e = g.iter_mut().find(|e| e.status.id == id)?;
        let r = e.recording.take()?;
        e.status.state = state;
        Some(r)
    }

    fn finish(&self, id: &str, result: Result<Finished, DotnetError>) -> Option<Status> {
        let mut g = self.lock();
        let e = g.iter_mut().find(|e| e.status.id == id)?;
        e.status.elapsed_ms = e.started.elapsed().as_millis() as u64;
        match result {
            Ok(f) => {
                e.status.state = State::Done;
                e.status.bytes = f.bytes;
                e.status.path = Some(f.path.to_string_lossy().into_owned());
                e.status.truncated = f.truncated;
                e.status.notes = f.notes;
            }
            Err(DotnetError::Cancelled) => e.status.state = State::Cancelled,
            Err(err) => {
                e.status.state = State::Failed;
                e.status.needs_elevation = matches!(err, DotnetError::Denied);
                e.status.error = Some(err.to_string());
            }
        }
        Some(e.status.clone())
    }

    /// Stops `id` (the file is kept and finished: rundown first). Unknown or finished: its status.
    pub fn stop(&self, id: &str) -> Result<Status, String> {
        match self.take(id, State::Stopping) {
            Some(r) => self.finish(id, r.stop()).ok_or_else(|| unknown(id)),
            None => self.get(id).ok_or_else(|| unknown(id)),
        }
    }

    /// Cancels `id`: stops at once and deletes the file.
    pub fn cancel(&self, id: &str) -> Result<Status, String> {
        match self.take(id, State::Stopping) {
            Some(r) => {
                r.cancel();
                self.finish(id, Err(DotnetError::Cancelled)).ok_or_else(|| unknown(id))
            }
            None => self.get(id).ok_or_else(|| unknown(id)),
        }
    }

    /// Finalises captures that ended by themselves (duration, heap walk done, target exited, size cap).
    /// Returns the ids it finished.
    pub fn reap(&self) -> Vec<String> {
        let ended: Vec<String> = self
            .lock()
            .iter()
            .filter(|e| e.recording.as_ref().is_some_and(|r| r.is_finished()))
            .map(|e| e.status.id.clone())
            .collect();
        for id in &ended {
            if let Some(r) = self.take(id, State::Stopping) {
                self.finish(id, r.stop());
            }
        }
        ended
    }

    /// Whether `id` still records.
    pub fn is_running(&self, id: &str) -> bool {
        self.lock().iter().any(|e| e.status.id == id && e.recording.is_some())
    }
}

fn unknown(id: &str) -> String {
    format!("there is no capture \"{id}\"")
}

fn refresh(e: &mut Entry) -> Status {
    if let Some(r) = &e.recording {
        e.status.bytes = r.bytes();
        e.status.elapsed_ms = r.elapsed().as_millis() as u64;
    }
    e.status.clone()
}

/// Drops the oldest finished captures beyond [`KEEP`].
fn trim(entries: &mut VecDeque<Entry>) {
    while entries.len() > KEEP {
        match entries.iter().position(|e| e.recording.is_none()) {
            Some(i) => {
                entries.remove(i);
            }
            None => break,
        }
    }
}

fn unix_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// `Shop-1234-cpu-1792000000.nettrace` (names cleaned for the file system).
pub fn file_name(info: &ProcessInfo, kind: Preset, secs: u64) -> PathBuf {
    let stem: String = info
        .name
        .trim_end_matches(".exe")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(60)
        .collect();
    let kind = match kind {
        Preset::Cpu => "cpu",
        Preset::Allocations => "allocations",
        Preset::HeapSnapshot => "heap",
    };
    PathBuf::from(format!("{stem}-{}-{kind}-{secs}.nettrace", info.pid))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::process::Runtime;
    use crate::recording::ThreadRecording;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// A collector with one process whose recordings write a few bytes until stopped.
    pub(crate) struct FakeCollector {
        pub deny: AtomicBool,
        /// Recordings end by themselves after this long.
        pub self_ending: Option<Duration>,
    }

    pub(crate) fn proc(pid: u32) -> ProcessInfo {
        ProcessInfo {
            pid,
            name: "Shop.exe".into(),
            command_line: None,
            runtime_version: Some(".NET 10.0.1".into()),
            arch: Some("x64".into()),
            runtime: Runtime::Core,
            needs_elevation: false,
        }
    }

    impl Collector for FakeCollector {
        fn list_processes(&self) -> Vec<ProcessInfo> {
            vec![proc(10), proc(11)]
        }

        fn start(&self, _pid: u32, _preset: Preset, out: &Path, limits: Limits) -> Result<Box<dyn Recording>, DotnetError> {
            if self.deny.load(Ordering::SeqCst) {
                return Err(DotnetError::Denied);
            }
            let out = out.to_path_buf();
            let o = out.clone();
            let ends = self.self_ending;
            let r = ThreadRecording::spawn("fake", out, move |ctl| {
                std::fs::write(&o, b"Nettrace").map_err(DotnetError::Io)?;
                ctl.set_bytes(8);
                let t = Instant::now();
                while !ctl.stopping() && t.elapsed() < limits.max_duration && ends.is_none_or(|d| t.elapsed() < d) {
                    std::thread::sleep(Duration::from_millis(5));
                }
                if ctl.cancelled() {
                    return Err(DotnetError::Cancelled);
                }
                Ok(Finished {
                    path: o,
                    bytes: 8,
                    truncated: false,
                    notes: vec!["n".into()],
                })
            })?;
            Ok(Box::new(r))
        }
    }

    pub(crate) fn fake(self_ending: Option<Duration>) -> Arc<FakeCollector> {
        Arc::new(FakeCollector {
            deny: AtomicBool::new(false),
            self_ending,
        })
    }

    fn req(pid: u32) -> Request {
        Request {
            pid,
            kind: Preset::Cpu,
            duration_sec: None,
        }
    }

    #[test]
    fn start_stop_cancel_and_list() {
        let dir = tempfile::tempdir().unwrap();
        let c = Captures::new(fake(None));
        let s = c.start(req(10), dir.path(), Limits::default()).unwrap();
        assert_eq!(s.state, State::Recording);
        assert_eq!(s.process_name, "Shop.exe");
        assert!(c
            .start(req(10), dir.path(), Limits::default())
            .unwrap_err()
            .contains("already being recorded"));
        assert!(c
            .start(req(99), dir.path(), Limits::default())
            .unwrap_err()
            .contains("not a running"));
        let bad = Request {
            duration_sec: Some(0),
            ..req(11)
        };
        assert!(c.start(bad, dir.path(), Limits::default()).is_err());
        assert!(c.is_running(&s.id));
        let done = c.stop(&s.id).unwrap();
        assert_eq!(done.state, State::Done);
        assert!(done.path.as_deref().is_some_and(|p| p.ends_with(".nettrace")));
        assert_eq!(done.notes, ["n"]);
        assert_eq!(c.stop(&s.id).unwrap().state, State::Done, "stopping twice is harmless");

        let s2 = c.start(req(11), dir.path(), Limits::default()).unwrap();
        let cancelled = c.cancel(&s2.id).unwrap();
        assert_eq!(cancelled.state, State::Cancelled);
        assert_eq!(c.list().len(), 2);
        assert_eq!(c.list()[0].id, s2.id, "newest first");
        assert!(c.stop("nope").is_err());
        assert!(c.cancel("nope").is_err());
        assert!(c.get("nope").is_none());
    }

    #[test]
    fn denied_starts_are_failed_captures_that_ask_for_elevation() {
        let dir = tempfile::tempdir().unwrap();
        let f = fake(None);
        f.deny.store(true, Ordering::SeqCst);
        let c = Captures::new(f);
        let s = c.start(req(10), dir.path(), Limits::default()).unwrap();
        assert_eq!(s.state, State::Failed);
        assert!(s.needs_elevation);
        assert!(s.error.unwrap().contains("administrator"));
        // A failed capture does not block a new one.
        assert!(c.start(req(10), dir.path(), Limits::default()).is_ok());
    }

    #[test]
    fn captures_that_end_by_themselves_are_reaped() {
        let dir = tempfile::tempdir().unwrap();
        let c = Captures::new(fake(Some(Duration::from_millis(20))));
        let s = c.start(req(10), dir.path(), Limits::default()).unwrap();
        let t = Instant::now();
        let mut reaped = Vec::new();
        while reaped.is_empty() && t.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(10));
            reaped = c.reap();
        }
        assert_eq!(reaped, std::slice::from_ref(&s.id));
        assert_eq!(c.get(&s.id).unwrap().state, State::Done);
        assert!(!c.is_running(&s.id));
    }

    #[test]
    fn the_list_is_bounded_and_names_are_safe() {
        let dir = tempfile::tempdir().unwrap();
        let f = fake(None);
        f.deny.store(true, Ordering::SeqCst);
        let c = Captures::new(f);
        for _ in 0..(KEEP + 10) {
            c.start(req(10), dir.path(), Limits::default()).unwrap();
        }
        assert_eq!(c.list().len(), KEEP);
        let mut p = proc(7);
        p.name = "My App/..\\x.exe".into();
        let n = file_name(&p, Preset::HeapSnapshot, 5);
        assert_eq!(n.to_str().unwrap(), "My_App____x-7-heap-5.nettrace");
        assert!(!n.to_str().unwrap().contains('/'));
        let json = serde_json::to_value(c.list()[0].clone()).unwrap();
        assert_eq!(json["state"], "failed");
        assert_eq!(json["kind"], "cpu");
        assert_eq!(json["needsElevation"], true);
        let r: Request = serde_json::from_value(serde_json::json!({"pid": 3, "kind": "heap", "durationSec": null})).unwrap();
        assert_eq!(r.kind, Preset::HeapSnapshot);
        assert!(serde_json::from_value::<Request>(serde_json::json!({"pid": 3, "kind": "x"})).is_err());
    }
}
