//! ETW recordings in the plugin's elevated process (ADR 0017 §7, ADR 0019).
//!
//! - [`Router`] is the collector of the normal process: EventPipe records here; ETW goes to the elevated process
//!   through the tool's `etw-*` operations, which the host runs there (starting it with the administrator prompt
//!   on first use). A declined prompt is [`DotnetError::Denied`].
//! - [`EtwHost`] serves those operations in the elevated process. It writes only into the captures folder of the
//!   data folder the *host* gives it, under a file name it checks, and records only .NET processes.

use crate::process::{self, Runtime};
use crate::{Collector, DotnetError, Finished, Limits, Preset, ProcessInfo, Recording};
use anyhow::{anyhow, bail, Context, Result};
use imt_plugin_api::tools::{ElevatedOps, FailureReason, ToolFailure};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
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
/// Largest recording the elevated process writes.
pub const MAX_BYTES: u64 = 8 << 30;

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

/// Starts an ETW recording (the real backend; tests use a fake).
pub type EtwStart = dyn Fn(&ProcessInfo, Preset, &Path, Limits) -> Result<Box<dyn Recording>, DotnetError> + Send + Sync;

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
fn start_etw(info: &ProcessInfo, preset: Preset, out: &Path, limits: Limits) -> Result<Box<dyn Recording>, DotnetError> {
    Ok(Box::new(crate::etw::session::record(info, preset, out, limits)?))
}

#[cfg(not(windows))]
fn start_etw(_: &ProcessInfo, _: Preset, _: &Path, _: Limits) -> Result<Box<dyn Recording>, DotnetError> {
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

    /// Runs one elevated operation; `data_dir` comes from the host.
    pub fn invoke(&self, op: &str, args: Value, data_dir: Option<&Path>) -> Result<Value> {
        let id = || args.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
        match op {
            START => {
                let a: StartArgs = serde_json::from_value(args.clone()).context("invalid ETW request")?;
                check_name(&a.name)?;
                let data = data_dir.ok_or_else(|| anyhow!("IMT has no data folder to write captures to"))?;
                let info = (self.find)(a.pid).ok_or_else(|| anyhow!(DotnetError::NotFound { pid: a.pid }))?;
                if lock(&self.recordings).len() >= MAX_RECORDINGS {
                    bail!("too many ETW recordings at once; stop one first");
                }
                let dir = captures_dir(data);
                std::fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
                let limits = Limits {
                    max_bytes: a.max_bytes.clamp(1, MAX_BYTES),
                    max_duration: Duration::from_millis(a.max_duration_ms.clamp(1, super::captures::MAX_DURATION_SEC * 1000)),
                    ..Limits::default()
                };
                let recording = (self.start)(&info, a.kind, &dir.join(&a.name), limits).map_err(|e| anyhow!(e))?;
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
    use super::super::captures::tests::{fake, proc};
    use super::*;

    /// Elevated operations served in this process by an [`EtwHost`] over the fake collector, as the host would.
    struct Loopback {
        host: EtwHost,
        data: PathBuf,
        deny: bool,
    }

    impl ElevatedOps for Loopback {
        fn invoke(&self, tool: &str, op: &str, args: Value, _: Duration) -> Result<Value> {
            assert_eq!(tool, crate::TOOL_ID);
            if self.deny {
                return Err(anyhow::Error::new(ToolFailure::denied("the administrator prompt was declined")));
            }
            self.host.invoke(op, args, Some(&self.data))
        }
    }

    fn loopback(data: &Path, deny: bool) -> Arc<Loopback> {
        let f = fake(None);
        Arc::new(Loopback {
            host: EtwHost::new(
                Box::new(move |info, preset, out, limits| f.start(info.pid, preset, out, limits)),
                Box::new(|pid| (pid == 10).then(|| proc(10))),
            ),
            data: data.to_path_buf(),
            deny,
        })
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

        let r = RemoteRecording::start(ops.clone(), 10, Preset::Cpu, &out, Limits::default()).unwrap();
        Box::new(r).cancel();
        assert!(matches!(
            RemoteRecording::start(ops, 99, Preset::Cpu, &out, Limits::default()),
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
                None
            )
            .unwrap_err()
            .to_string()
            .contains("data folder"));
        assert!(host.host.invoke(START, json!({"pid": 10}), Some(dir.path())).is_err());
        assert!(host.host.invoke(STOP, json!({"id": "nope"}), Some(dir.path())).is_err());
        assert_eq!(host.host.invoke(CANCEL, json!({"id": "nope"}), None).unwrap(), Value::Null);
        assert!(host.host.invoke("bogus", json!({}), None).is_err());
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
