//! Starting a child process the supervisor owns: below-normal priority, no console window, a cleared
//! environment, killed with IMT (a job object with KILL_ON_JOB_CLOSE on Windows, which also carries the
//! sandbox's limits; on Unix the child exits when its stdin, the lifeline, closes). [`crate::launch`] writes its
//! bootstrap to stdin and accepts its connection.

//!
//! With [`SpawnSpec::log`] the child's standard error is captured into its [`crate::child_log`] (with start
//! and exit lines); without it, standard error is discarded.

use crate::child_log::{self, ChildLog, ChildLogSpec, Exit};
use crate::IpcError;
use std::io::{BufRead, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Longest piece of a child's standard error read as one line.
const MAX_STDERR_LINE: u64 = 8 * 1024;
/// How long the standard error reader waits for the OS to publish an exit status after end of output.
const EXIT_WAIT: Duration = Duration::from_secs(2);

/// Environment variables every child gets by default ([`SpawnSpec::keep_env`]): what the OS and the
/// runtime need, and the coverage profile path so instrumented test builds can report. Nothing else of
/// IMT's environment (tokens, proxies, user variables) reaches a child unless it is named.
pub const KEPT_ENV: &[&str] = &[
    "PATH",
    "SystemRoot",
    "SYSTEMROOT",
    "windir",
    "TEMP",
    "TMP",
    "TMPDIR",
    "LANG",
    "LC_ALL",
    "RUST_BACKTRACE",
    "LLVM_PROFILE_FILE",
    "CARGO_LLVM_COV",
    "XDG_RUNTIME_DIR",
];

/// How long a child whose lifeline was cut may take to exit before it is killed.
pub const KILL_GRACE: Duration = Duration::from_millis(150);

/// How to start a child.
#[derive(Debug, Clone)]
pub struct SpawnSpec {
    pub exe: PathBuf,
    pub args: Vec<String>,
    /// Extra environment.
    pub env: Vec<(String, String)>,
    /// The only variables of IMT's environment the child inherits (default [`KEPT_ENV`]); every child
    /// starts from an empty environment plus these and `env`, so it learns nothing ambient.
    pub keep_env: &'static [&'static str],
    /// Working directory (default: inherited).
    pub current_dir: Option<PathBuf>,
    /// Names the process in messages: "the AI engine", "plugin \"x\"".
    pub label: String,
    /// Prefix of the private address (see [`PrivateEndpoint::create_named`]).
    pub endpoint_prefix: String,
    /// How long the child may take to connect.
    pub connect_window: Duration,
    /// Run at below-normal priority so the analysis stays responsive.
    pub below_normal_priority: bool,
    /// Capture standard error into this child log (see [`crate::child_log`]); `None` discards it.
    pub log: Option<ChildLogSpec>,
    /// The child's sandbox: the launcher applies its job limits (Windows), the child the rest
    /// ([`crate::sandbox`]).
    pub sandbox: crate::sandbox::SandboxSpec,
}

impl SpawnSpec {
    pub fn new(exe: impl Into<PathBuf>, label: impl Into<String>) -> SpawnSpec {
        SpawnSpec {
            exe: exe.into(),
            args: Vec::new(),
            env: Vec::new(),
            keep_env: KEPT_ENV,
            current_dir: None,
            label: label.into(),
            endpoint_prefix: "imt-ipc".into(),
            connect_window: Duration::from_secs(15),
            below_normal_priority: true,
            log: None,
            sandbox: crate::sandbox::SandboxSpec::unrestricted(),
        }
    }
}

/// The exit line of one child, written once (by whoever sees the exit first).
struct ExitNote {
    log: Arc<ChildLog>,
    pid: u32,
    written: AtomicBool,
    /// IMT asked the child to stop: whatever its status, it was stopped by IMT.
    stopping: AtomicBool,
}

impl ExitNote {
    fn write(&self, status: Option<ExitStatus>) {
        if self.written.swap(true, Ordering::SeqCst) {
            return;
        }
        let exit = if self.stopping.load(Ordering::SeqCst) {
            Exit::StoppedByImt
        } else {
            status.as_ref().map(Exit::of).unwrap_or(Exit::Unknown)
        };
        self.log.exited(Some(self.pid), exit);
    }
}

/// A started child. Killed (and reaped) on drop.
pub struct ChildProcess {
    child: Arc<Mutex<Option<Child>>>,
    stdin: Mutex<Option<ChildStdin>>,
    pid: u32,
    label: String,
    exit_note: Option<Arc<ExitNote>>,
    #[cfg(windows)]
    _job: Option<crate::job::JobGuard>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Reads a child's standard error into its log until end of output, then records how it exited (unless
/// IMT is already stopping it, which records that itself).
fn capture_stderr(stderr: impl Read + Send + 'static, log: Arc<ChildLog>, child: Arc<Mutex<Option<Child>>>, note: Arc<ExitNote>) {
    let spawned = std::thread::Builder::new().name("imt-child-stderr".into()).spawn(move || {
        let mut r = std::io::BufReader::new(stderr);
        let mut line = Vec::new();
        loop {
            line.clear();
            match r.by_ref().take(MAX_STDERR_LINE).read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => log.output(&String::from_utf8_lossy(&line)),
            }
        }
        let deadline = std::time::Instant::now() + EXIT_WAIT;
        while std::time::Instant::now() < deadline {
            let status = match lock(&child).as_mut() {
                // Taken: IMT is stopping it and records the exit.
                None => return,
                Some(c) => c.try_wait().ok().flatten(),
            };
            if let Some(s) = status {
                return note.write(Some(s));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    });
    // Without a thread the pipe is dropped with the closure: the child's writes fail instead of blocking.
    drop(spawned);
}

impl ChildProcess {
    /// Starts the program with a piped stdin (its lifeline), no stdout, and stderr captured into its child
    /// log when the spec names one (discarded otherwise).
    pub fn spawn(spec: &SpawnSpec) -> Result<ChildProcess, IpcError> {
        // The one spawner outside imt-core (this crate stands alone); it sets CREATE_NO_WINDOW below.
        #[allow(clippy::disallowed_methods)]
        let mut cmd = Command::new(&spec.exe);
        cmd.args(&spec.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(if spec.log.is_some() { Stdio::piped() } else { Stdio::null() });
        cmd.env_clear();
        for k in spec.keep_env {
            if let Some(v) = std::env::var_os(k) {
                cmd.env(k, v);
            }
        }
        cmd.envs(spec.env.iter().map(|(k, v)| (k, v)));
        if let Some(d) = &spec.current_dir {
            cmd.current_dir(d);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // No console window; below-normal priority when asked.
            let priority = if spec.below_normal_priority { 0x0000_4000 } else { 0 };
            cmd.creation_flags(0x0800_0000 | priority);
        }
        let mut child = cmd.spawn().map_err(|e| {
            let msg = format!("cannot start {} ({}): {e}", spec.label, spec.exe.display());
            if let Some(l) = &spec.log {
                child_log::get(l).record(&child_log::ChildEvent::Note(format!("Could not start: {msg}")));
            }
            IpcError::Spawn(msg)
        })?;
        let pid = child.id();
        // Lowered from here rather than in a `pre_exec` hook: a hook forces fork+exec instead of
        // posix_spawn, and on macOS a concurrent fork can then inherit another spawn's exec-status pipe
        // and keep that spawn waiting forever.
        #[cfg(unix)]
        if spec.below_normal_priority {
            // SAFETY: setpriority takes plain integers and has no memory effects; it only lowers the
            // priority of our own child.
            unsafe {
                libc::setpriority(libc::PRIO_PROCESS, pid as libc::id_t, 10);
            }
        }
        #[cfg(windows)]
        let job = crate::job::JobGuard::assign(&child, &crate::job::JobLimits::of(spec));
        let stdin = child.stdin.take();
        let stderr = child.stderr.take();
        let child = Arc::new(Mutex::new(Some(child)));
        let exit_note = spec.log.as_ref().map(|l| {
            let log = child_log::get(l);
            log.started(pid);
            let note = Arc::new(ExitNote {
                log: log.clone(),
                pid,
                written: AtomicBool::new(false),
                stopping: AtomicBool::new(false),
            });
            if let Some(err) = stderr {
                capture_stderr(err, log, child.clone(), note.clone());
            }
            note
        });
        Ok(ChildProcess {
            child,
            stdin: Mutex::new(stdin),
            pid,
            label: spec.label.clone(),
            exit_note,
            #[cfg(windows)]
            _job: job,
        })
    }

    /// The child's log, when its output is captured.
    pub fn log(&self) -> Option<&Arc<ChildLog>> {
        self.exit_note.as_ref().map(|n| &n.log)
    }

    /// From now on, however the child ends, IMT stopped it.
    fn mark_stopping(&self) {
        if let Some(n) = &self.exit_note {
            n.stopping.store(true, Ordering::SeqCst);
        }
    }

    fn note_exit(&self, status: Option<ExitStatus>) {
        if let Some(n) = &self.exit_note {
            n.write(status);
        }
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Writes `value` as one JSON line to the child's stdin (its bootstrap).
    pub fn send_line<T: serde::Serialize>(&self, value: &T) -> Result<(), IpcError> {
        let line = serde_json::to_string(value).map_err(|e| IpcError::Protocol(e.to_string()))?;
        let wrote = lock(&self.stdin)
            .as_mut()
            .map(|s| s.write_all(format!("{line}\n").as_bytes()).and_then(|_| s.flush()));
        // The handshake is the first thing a child reads, so a failed write means it died while starting (one
        // that exits at once can break the pipe before IMT writes).
        match wrote {
            Some(Ok(())) => Ok(()),
            _ => Err(IpcError::Died(match self.exit_soon() {
                Some(s) => format!("{} exited while starting ({s})", self.label),
                None => format!("{} closed the connection while starting", self.label),
            })),
        }
    }

    /// The exit status, if the child has exited.
    pub fn try_exit(&self) -> Option<ExitStatus> {
        lock(&self.child).as_mut().and_then(|c| c.try_wait().ok().flatten())
    }

    /// Whether the child has exited (or was killed).
    pub fn exited(&self) -> bool {
        lock(&self.child).as_mut().is_none_or(|c| matches!(c.try_wait(), Ok(Some(_))))
    }

    /// Describes how the child ended, waiting briefly for the OS to publish its exit status.
    pub fn exit_text(&self) -> String {
        match self.exit_soon() {
            Some(s) => format!("{} exited ({s})", self.label),
            None => format!("{} closed the connection", self.label),
        }
    }

    /// The exit status, waiting up to 200 ms for the OS to publish it.
    fn exit_soon(&self) -> Option<ExitStatus> {
        for _ in 0..20 {
            if let Some(s) = self.try_exit() {
                return Some(s);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        None
    }

    /// Closes the child's stdin: a well-behaved child exits on its own (this is what happens when IMT dies).
    pub fn close_lifeline(&self) {
        self.note_own_exit();
        self.mark_stopping();
        *lock(&self.stdin) = None;
    }

    /// A child that already ended on its own is recorded as such (crash or normal), before IMT stops it.
    fn note_own_exit(&self) {
        if let Some(s) = self.try_exit() {
            self.note_exit(Some(s));
        }
    }

    /// Stops the child and reaps it: its lifeline is closed first and a well-behaved child exits at once
    /// (see [`crate::child::child_main`]); one that has not after [`KILL_GRACE`] is killed.
    pub fn kill(&self) {
        self.note_own_exit();
        self.mark_stopping();
        *lock(&self.stdin) = None;
        let taken = lock(&self.child).take();
        if let Some(mut c) = taken {
            let deadline = std::time::Instant::now() + KILL_GRACE;
            let mut ended = false;
            while std::time::Instant::now() < deadline {
                if matches!(c.try_wait(), Ok(Some(_))) {
                    ended = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            if !ended {
                let _ = c.kill();
                let _ = c.wait();
            }
            self.note_exit(None);
        }
    }

    /// Closes the lifeline and waits up to `grace` for the child to exit on its own; kills it after that.
    pub fn close(&self, grace: Duration) {
        self.close_lifeline();
        let deadline = std::time::Instant::now() + grace;
        while std::time::Instant::now() < deadline {
            if self.try_exit().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        self.kill();
    }
}

impl Drop for ChildProcess {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Resident memory of this process, in bytes (0 if unknown).
pub fn current_rss_bytes() -> u64 {
    let Ok(pid) = sysinfo::get_current_pid() else { return 0 };
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
    sys.process(pid).map(|p| p.memory()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    #[test]
    fn rss_is_reported() {
        assert!(super::current_rss_bytes() > 0);
    }
}
