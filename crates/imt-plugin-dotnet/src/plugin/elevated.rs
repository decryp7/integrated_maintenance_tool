//! ETW recordings in the plugin's elevated process (ADR 0017 §7, ADR 0019).
//!
//! - [`Router`] is the collector of the normal process: EventPipe records here; ETW goes to the elevated process
//!   through the tool's `etw-*` operations, which the host runs there (starting it with the administrator prompt
//!   on first use). A declined prompt is [`DotnetError::Denied`].
//! - [`EtwHost`] serves those operations in the elevated process. It records only .NET processes and writes no file
//!   at a path: the capture is streamed through the call's `ctx.outputs()` (plugin protocol 1.1) to the host, which
//!   saves it as `<data>/captures/dotnet/<name>` under a file name checked on both sides. A failed or cancelled
//!   recording drops its stream unfinished, so no partial file is left.

use crate::process::{self, Runtime};
use crate::recording::CaptureSink;
use crate::{Collector, DotnetError, Finished, Limits, Preset, ProcessInfo, Recording};
use anyhow::{anyhow, bail, Context, Result};
use imt_plugin_api::output::{OutputFile, Outputs};
use imt_plugin_api::tools::{ElevatedOps, FailureReason, ToolFailure};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// The elevated operations of the tool.
pub const START: &str = "etw-start";
pub const POLL: &str = "etw-poll";
pub const STOP: &str = "etw-stop";
pub const CANCEL: &str = "etw-cancel";

/// Time for an ordinary elevated call (the host adds the time of the administrator prompt).
const CALL_TIMEOUT: Duration = Duration::from_secs(30);
/// Time for stopping: the rundown that collects method names.
pub const STOP_TIMEOUT: Duration = Duration::from_secs(120);
/// Status of a remote recording is asked at most this often.
const POLL_EVERY: Duration = Duration::from_millis(200);
/// Longest file name accepted.
const MAX_NAME: usize = 200;
/// Largest recording the elevated process streams.
pub const MAX_BYTES: u64 = 8 << 30;
/// What the recorder may write after the size limit is reached (its closing blocks); the host's stream is cut at
/// the size limit plus this.
const CLOSING_ALLOWANCE: u64 = 64 << 20;
/// The folder below `<data>/captures` the host saves captures in.
const FOLDER: &str = "dotnet";

/// `etw-start` arguments.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartArgs {
    pub pid: u32,
    pub kind: Preset,
    /// The file name in the captures folder.
    pub name: String,
    pub max_bytes: u64,
    pub max_duration_ms: u64,
}

/// `etw-poll` answer.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Poll {
    pub bytes: u64,
    pub finished: bool,
}

/// `etw-stop` answer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Stopped {
    pub path: String,
    pub bytes: u64,
    pub truncated: bool,
    pub notes: Vec<String>,
}

/// The folder the elevated process writes captures into: `<data>/captures/dotnet`, as the normal process does.
pub fn captures_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("captures").join("dotnet")
}

/// A capture file name: one component, no path syntax, `.nettrace`.
fn check_name(name: &str) -> Result<()> {
    let plain = name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if name.is_empty() || name.len() > MAX_NAME || !plain || name.starts_with('.') || !name.ends_with(".nettrace") {
        bail!("invalid capture file name {name:?}");
    }
    Ok(())
}

// ---- the normal process ----

/// The normal process' collector: EventPipe here, ETW in the elevated process.
#[derive(Default)]
pub struct Router {
    /// How to reach the elevated process; set from the context of the tool's calls.
    elevated: Mutex<Option<Arc<dyn ElevatedOps>>>,
}

impl Router {
    /// Remembers how to run elevated operations (from the call that starts a capture).
    pub fn connect(&self, ops: Option<&Arc<dyn ElevatedOps>>) {
        if let Some(ops) = ops {
            *lock(&self.elevated) = Some(ops.clone());
        }
    }

    fn ops(&self) -> Option<Arc<dyn ElevatedOps>> {
        lock(&self.elevated).clone()
    }
}

impl Collector for Router {
    fn list_processes(&self) -> Vec<ProcessInfo> {
        process::list()
    }

    fn start(&self, pid: u32, preset: Preset, out: &Path, limits: Limits) -> Result<Box<dyn Recording>, DotnetError> {
        let info = process::info(pid).ok_or(DotnetError::NotFound { pid })?;
        match info.runtime {
            Runtime::Core => Ok(Box::new(crate::eventpipe::record(pid, preset, out, limits)?)),
            Runtime::Framework => {
                if !cfg!(windows) {
                    return Err(DotnetError::Unsupported {
                        why: ".NET Framework processes can only be recorded on Windows (ETW)".into(),
                    });
                }
                let ops = self.ops().ok_or(DotnetError::Denied)?;
                Ok(Box::new(RemoteRecording::start(ops, pid, preset, out, limits)?))
            }
        }
    }
}

/// An ETW recording in the elevated process, seen from the normal process.
pub struct RemoteRecording {
    ops: Arc<dyn ElevatedOps>,
    id: String,
    started: Instant,
    last: Mutex<(Option<Instant>, Poll)>,
}

fn remote_error(e: anyhow::Error) -> DotnetError {
    match e.chain().find_map(|c| c.downcast_ref::<ToolFailure>()) {
        Some(f) if f.reason == FailureReason::Denied => DotnetError::Denied,
        _ => DotnetError::Protocol(format!("{e:#}")),
    }
}

impl RemoteRecording {
    pub fn start(ops: Arc<dyn ElevatedOps>, pid: u32, preset: Preset, out: &Path, limits: Limits) -> Result<RemoteRecording, DotnetError> {
        let name = out
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| DotnetError::Protocol("the capture has no file name".into()))?;
        let args = StartArgs {
            pid,
            kind: preset,
            name,
            max_bytes: limits.max_bytes,
            max_duration_ms: u64::try_from(limits.max_duration.as_millis()).unwrap_or(u64::MAX),
        };
        let args = serde_json::to_value(args).map_err(|e| DotnetError::Protocol(e.to_string()))?;
        let reply = ops.invoke(super::TOOL_ID, START, args, CALL_TIMEOUT).map_err(remote_error)?;
        let id = reply
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| DotnetError::Protocol("the elevated process sent no recording id".into()))?
            .to_string();
        Ok(RemoteRecording {
            ops,
            id,
            started: Instant::now(),
            last: Mutex::new((None, Poll::default())),
        })
    }

    fn poll(&self) -> Poll {
        let mut last = lock(&self.last);
        if last.0.is_some_and(|t| t.elapsed() < POLL_EVERY) {
            return last.1;
        }
        let p = self
            .ops
            .invoke(super::TOOL_ID, POLL, json!({ "id": self.id }), CALL_TIMEOUT)
            .ok()
            .and_then(|v| serde_json::from_value::<Poll>(v).ok())
            // The elevated process is gone (restarted, quarantined): the recording ended.
            .unwrap_or(Poll {
                bytes: last.1.bytes,
                finished: true,
            });
        *last = (Some(Instant::now()), p);
        p
    }
}

impl Recording for RemoteRecording {
    fn bytes(&self) -> u64 {
        self.poll().bytes
    }

    fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    fn is_finished(&self) -> bool {
        self.poll().finished
    }

    fn stop(self: Box<Self>) -> Result<Finished, DotnetError> {
        let v = self
            .ops
            .invoke(super::TOOL_ID, STOP, json!({ "id": self.id }), STOP_TIMEOUT)
            .map_err(remote_error)?;
        let s: Stopped = serde_json::from_value(v).map_err(|e| DotnetError::Protocol(e.to_string()))?;
        Ok(Finished {
            path: PathBuf::from(s.path),
            bytes: s.bytes,
            truncated: s.truncated,
            notes: s.notes,
        })
    }

    fn cancel(self: Box<Self>) {
        let _ = self.ops.invoke(super::TOOL_ID, CANCEL, json!({ "id": self.id }), CALL_TIMEOUT);
    }
}

// ---- the elevated process ----

/// A host output stream as the recorder's sink.
struct StreamSink(Box<dyn OutputFile>);

impl Write for StreamSink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl CaptureSink for StreamSink {
    fn finish(self: Box<Self>) -> Result<u64, DotnetError> {
        self.0
            .finish()
            .map(|saved| saved.bytes)
            .map_err(|e| DotnetError::Protocol(format!("the capture could not be saved: {e:#}")))
    }
}

/// Starts an ETW recording into a sink; the third argument is where the host will have saved it (the real backend;
/// tests use a fake).
pub type EtwStart =
    dyn Fn(&ProcessInfo, Preset, Box<dyn CaptureSink>, &Path, Limits) -> Result<Box<dyn Recording>, DotnetError> + Send + Sync;

/// The ETW recordings of the elevated process.
pub struct EtwHost {
    start: Box<EtwStart>,
    find: Box<dyn Fn(u32) -> Option<ProcessInfo> + Send + Sync>,
    recordings: Mutex<HashMap<String, Box<dyn Recording>>>,
    next: AtomicU64,
}

impl Default for EtwHost {
    fn default() -> Self {
        EtwHost::new(Box::new(start_etw), Box::new(process::info))
    }
}

#[cfg(windows)]
fn start_etw(
    info: &ProcessInfo,
    preset: Preset,
    sink: Box<dyn CaptureSink>,
    saved_at: &Path,
    limits: Limits,
) -> Result<Box<dyn Recording>, DotnetError> {
    Ok(Box::new(crate::etw::session::record(info, preset, sink, saved_at, limits)?))
}

#[cfg(not(windows))]
fn start_etw(_: &ProcessInfo, _: Preset, _: Box<dyn CaptureSink>, _: &Path, _: Limits) -> Result<Box<dyn Recording>, DotnetError> {
    Err(DotnetError::Unsupported {
        why: "ETW needs Windows".into(),
    })
}

/// Recordings the elevated process keeps at once.
const MAX_RECORDINGS: usize = 16;

impl EtwHost {
    pub fn new(start: Box<EtwStart>, find: Box<dyn Fn(u32) -> Option<ProcessInfo> + Send + Sync>) -> EtwHost {
        EtwHost {
            start,
            find,
            recordings: Mutex::new(HashMap::new()),
            next: AtomicU64::new(1),
        }
    }

    /// Runs one elevated operation; `data_dir` and `outputs` come from the host.
    pub fn invoke(&self, op: &str, args: Value, data_dir: Option<&Path>, outputs: Option<&Arc<dyn Outputs>>) -> Result<Value> {
        let id = || args.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
        match op {
            START => {
                let a: StartArgs = serde_json::from_value(args.clone()).context("invalid ETW request")?;
                check_name(&a.name)?;
                let data = data_dir.ok_or_else(|| anyhow!("IMT has no data folder to write captures to"))?;
                let outputs =
                    outputs.ok_or_else(|| anyhow!("this IMT gave the elevated recorder no way to save its capture; update IMT"))?;
                let info = (self.find)(a.pid).ok_or_else(|| anyhow!(DotnetError::NotFound { pid: a.pid }))?;
                if lock(&self.recordings).len() >= MAX_RECORDINGS {
                    bail!("too many ETW recordings at once; stop one first");
                }
                let saved_at = captures_dir(data).join(&a.name);
                let limits = Limits {
                    max_bytes: a.max_bytes.clamp(1, MAX_BYTES),
                    max_duration: Duration::from_millis(a.max_duration_ms.clamp(1, super::captures::MAX_DURATION_SEC * 1000)),
                    ..Limits::default()
                };
                // The host writes the file; this process never does. The recorder may finish a little past its limit.
                let sink = outputs.create(FOLDER, &a.name, limits.max_bytes.saturating_add(CLOSING_ALLOWANCE))?;
                let recording = (self.start)(&info, a.kind, Box::new(StreamSink(sink)), &saved_at, limits).map_err(|e| anyhow!(e))?;
                let id = format!("etw-{}", self.next.fetch_add(1, Ordering::Relaxed));
                lock(&self.recordings).insert(id.clone(), recording);
                Ok(json!({ "id": id }))
            }
            POLL => {
                let g = lock(&self.recordings);
                let r = g.get(&id()).ok_or_else(|| anyhow!("there is no ETW recording \"{}\"", id()))?;
                Ok(serde_json::to_value(Poll {
                    bytes: r.bytes(),
                    finished: r.is_finished(),
                })?)
            }
            STOP => {
                let r = lock(&self.recordings)
                    .remove(&id())
                    .ok_or_else(|| anyhow!("there is no ETW recording \"{}\"", id()))?;
                let f = r.stop().map_err(|e| anyhow!(e))?;
                Ok(serde_json::to_value(Stopped {
                    path: f.path.to_string_lossy().into_owned(),
                    bytes: f.bytes,
                    truncated: f.truncated,
                    notes: f.notes,
                })?)
            }
            CANCEL => {
                if let Some(r) = lock(&self.recordings).remove(&id()) {
                    r.cancel();
                }
                Ok(Value::Null)
            }
            other => bail!("unknown operation {other}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::captures::tests::proc;
    use super::*;
    use crate::recording::{wait_or_stop, ThreadRecording};
    use imt_plugin_api::output::FolderOutputs;
    use std::sync::atomic::AtomicBool;

    /// An ETW recording that streams "Nettrace" into its sink; `fail` makes it end with an error.
    fn streaming(fail: Arc<AtomicBool>) -> Box<EtwStart> {
        Box::new(move |_info, _preset, sink, saved_at, limits| {
            let (saved, fail) = (saved_at.to_path_buf(), fail.clone());
            Ok(Box::new(ThreadRecording::spawn("fake-etw", saved.clone(), move |ctl| {
                let mut sink = sink;
                sink.write_all(b"Nettrace").map_err(DotnetError::Io)?;
                ctl.set_bytes(8);
                wait_or_stop(&ctl, Instant::now() + limits.max_duration);
                if ctl.cancelled() {
                    return Err(DotnetError::Cancelled);
                }
                if fail.load(Ordering::SeqCst) {
                    return Err(DotnetError::Protocol("the session broke".into()));
                }
                let bytes = sink.finish()?;
                Ok(Finished {
                    path: saved,
                    bytes,
                    truncated: false,
                    notes: Vec::new(),
                })
            })?))
        })
    }

    /// Elevated operations served in this process by an [`EtwHost`] over the fake collector, as the host would.
    struct Loopback {
        host: EtwHost,
        data: PathBuf,
        outputs: Arc<dyn Outputs>,
        deny: bool,
    }

    impl ElevatedOps for Loopback {
        fn invoke(&self, tool: &str, op: &str, args: Value, _: Duration) -> Result<Value> {
            assert_eq!(tool, crate::TOOL_ID);
            if self.deny {
                return Err(anyhow::Error::new(ToolFailure::denied("the administrator prompt was declined")));
            }
            self.host.invoke(op, args, Some(&self.data), Some(&self.outputs))
        }
    }

    fn loopback(data: &Path, deny: bool) -> Arc<Loopback> {
        loopback_failing(data, deny, Arc::new(AtomicBool::new(false)))
    }

    fn loopback_failing(data: &Path, deny: bool, fail: Arc<AtomicBool>) -> Arc<Loopback> {
        Arc::new(Loopback {
            host: EtwHost::new(streaming(fail), Box::new(|pid| (pid == 10).then(|| proc(10)))),
            data: data.to_path_buf(),
            outputs: Arc::new(FolderOutputs(data.to_path_buf())),
            deny,
        })
    }

    /// No partial file is left in the captures folder.
    fn only_files(dir: &Path, names: &[&str]) {
        let mut found: Vec<String> = std::fs::read_dir(captures_dir(dir))
            .map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default();
        found.sort();
        assert_eq!(found, names);
    }

    #[test]
    fn a_failed_or_cancelled_recording_leaves_no_partial_file_and_a_good_one_is_saved_by_the_host() {
        let dir = tempfile::tempdir().unwrap();
        let fail = Arc::new(AtomicBool::new(false));
        let ops: Arc<dyn ElevatedOps> = loopback_failing(dir.path(), false, fail.clone());
        let out = captures_dir(dir.path()).join("Shop-10-cpu-1.nettrace");
        let wait_bytes = |r: &RemoteRecording| {
            let t = Instant::now();
            while r.bytes() == 0 && t.elapsed() < Duration::from_secs(5) {
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        // Cancelled: the stream is dropped unfinished.
        let r = RemoteRecording::start(ops.clone(), 10, Preset::Cpu, &out, Limits::default()).unwrap();
        wait_bytes(&r);
        Box::new(r).cancel();
        only_files(dir.path(), &[]);
        // Failed: the error reaches the caller and nothing is left.
        fail.store(true, Ordering::SeqCst);
        let r = RemoteRecording::start(ops.clone(), 10, Preset::Cpu, &out, Limits::default()).unwrap();
        wait_bytes(&r);
        let e = Box::new(r).stop().expect_err("the recording failed");
        assert!(e.to_string().contains("the session broke"), "{e}");
        only_files(dir.path(), &[]);
        // Good: the host's output folder holds the finished file only.
        fail.store(false, Ordering::SeqCst);
        let r = RemoteRecording::start(ops, 10, Preset::Cpu, &out, Limits::default()).unwrap();
        wait_bytes(&r);
        let f = Box::new(r).stop().unwrap();
        assert_eq!((f.path.as_path(), f.bytes), (out.as_path(), 8));
        only_files(dir.path(), &["Shop-10-cpu-1.nettrace"]);
        assert_eq!(std::fs::read(&out).unwrap(), b"Nettrace");
    }

    #[test]
    fn a_host_without_output_streams_is_told_to_update() {
        let dir = tempfile::tempdir().unwrap();
        let host = loopback(dir.path(), false);
        let e = host
            .host
            .invoke(
                START,
                json!({"pid": 10, "kind": "cpu", "name": "a.nettrace", "maxBytes": 1, "maxDurationMs": 1}),
                Some(dir.path()),
                None,
            )
            .unwrap_err();
        assert!(e.to_string().contains("update IMT"), "{e}");
    }

    #[test]
    fn an_etw_recording_runs_through_the_elevated_operations() {
        let dir = tempfile::tempdir().unwrap();
        let ops: Arc<dyn ElevatedOps> = loopback(dir.path(), false);
        let out = captures_dir(dir.path()).join("Shop-10-cpu-1.nettrace");
        let r = RemoteRecording::start(ops.clone(), 10, Preset::Cpu, &out, Limits::default()).unwrap();
        let t = Instant::now();
        while r.bytes() == 0 && t.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(r.bytes(), 8);
        assert!(!r.is_finished());
        let f = Box::new(r).stop().unwrap();
        assert_eq!(PathBuf::from(&f.path), out);
        assert!(out.exists());
        // Gone after stopping: a poll error means the recording ended.
        assert!(ops.invoke(crate::TOOL_ID, POLL, json!({"id": "etw-1"}), CALL_TIMEOUT).is_err());

        // The host never replaces a file: the same name again is refused, another name works.
        assert!(RemoteRecording::start(ops.clone(), 10, Preset::Cpu, &out, Limits::default()).is_err());
        let again = captures_dir(dir.path()).join("Shop-10-cpu-2.nettrace");
        let r = RemoteRecording::start(ops.clone(), 10, Preset::Cpu, &again, Limits::default()).unwrap();
        Box::new(r).cancel();
        assert!(!again.exists());
        assert!(matches!(
            RemoteRecording::start(ops, 99, Preset::Cpu, &again, Limits::default()),
            Err(DotnetError::Protocol(m)) if m.contains("99")
        ));
    }

    #[test]
    fn a_declined_prompt_is_denied_and_the_elevated_side_checks_its_inputs() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("x.nettrace");
        let declined: Arc<dyn ElevatedOps> = loopback(dir.path(), true);
        assert!(matches!(
            RemoteRecording::start(declined, 10, Preset::Cpu, &out, Limits::default()),
            Err(DotnetError::Denied)
        ));
        let host = loopback(dir.path(), false);
        let start = |name: &str| {
            host.host.invoke(
                START,
                json!({"pid": 10, "kind": "cpu", "name": name, "maxBytes": 10, "maxDurationMs": 10}),
                Some(dir.path()),
                Some(&host.outputs),
            )
        };
        for bad in [
            "../x.nettrace",
            "a/b.nettrace",
            "a\\b.nettrace",
            "x.exe",
            ".nettrace",
            "",
            "c:x.nettrace",
        ] {
            assert!(start(bad).is_err(), "{bad}");
        }
        assert!(host
            .host
            .invoke(
                START,
                json!({"pid": 10, "kind": "cpu", "name": "a.nettrace", "maxBytes": 1, "maxDurationMs": 1}),
                None,
                Some(&host.outputs)
            )
            .unwrap_err()
            .to_string()
            .contains("data folder"));
        assert!(host.host.invoke(START, json!({"pid": 10}), Some(dir.path()), None).is_err());
        assert!(host.host.invoke(STOP, json!({"id": "nope"}), Some(dir.path()), None).is_err());
        assert_eq!(host.host.invoke(CANCEL, json!({"id": "nope"}), None, None).unwrap(), Value::Null);
        assert!(host.host.invoke("bogus", json!({}), None, None).is_err());
        // Nothing was written outside the captures folder.
        assert!(!dir.path().join("x.exe").exists());
    }

    #[test]
    fn without_an_elevated_process_framework_targets_need_rights() {
        let router = Router::default();
        assert!(router.ops().is_none());
        router.connect(None);
        assert!(router.ops().is_none());
        let ops: Arc<dyn ElevatedOps> = loopback(Path::new("."), false);
        router.connect(Some(&ops));
        assert!(router.ops().is_some());
        assert!(matches!(
            Router::default().start(u32::MAX - 3, Preset::Cpu, Path::new("x.nettrace"), Limits::default()),
            Err(DotnetError::NotFound { .. })
        ));
    }
}
