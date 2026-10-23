//! The llama-server child process: started with an empty environment (llama-server reads `LLAMA_ARG_*`
//! variables, which could otherwise switch on its agent tools or the web UI), on 127.0.0.1 and a free
//! port, with a random API key passed in its environment rather than on the command line, and tied to
//! the worker's lifetime so it is never left behind:
//!
//! - Windows: a job object with KILL_ON_JOB_CLOSE (closed when the worker exits or crashes);
//! - Linux: `PR_SET_PDEATHSIG` (SIGKILL when the worker dies);
//! - every Unix: killed on drop and from an `atexit` hook (the worker exits through `exit` when Debrief
//!   closes its stdin), which is all macOS offers.

use crate::http::{Call, Client, HttpError};
use std::io::{BufRead, BufReader, Read};
use std::net::{Ipv4Addr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};

use debrief_core::process::{quiet_command_with, BELOW_NORMAL_PRIORITY_CLASS};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Variables llama-server still gets (what the OS, the GPU drivers and their loaders need): the worker's
/// own allow-list, so the worker never has a variable it does not pass on.
pub const KEPT_ENV: &[&str] = debrief_ai::worker::WORKER_ENV;

/// How much of llama-server's error output is kept for messages.
const TAIL_BYTES: usize = 4096;

/// Why a start failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartError {
    /// The executable could not be run at all (missing, not executable, wrong architecture, ...).
    Spawn(String),
    /// It ran but exited before it was ready.
    Exited(String),
    /// It did not become ready in time.
    TimedOut,
    Cancelled,
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StartError::Spawn(m) | StartError::Exited(m) => f.write_str(m),
            StartError::TimedOut => f.write_str("llama-server did not become ready in time"),
            StartError::Cancelled => f.write_str("cancelled"),
        }
    }
}

/// The last bytes of a stream, for error messages.
#[derive(Default)]
struct Tail(Vec<u8>);

impl Tail {
    fn push(&mut self, line: &[u8]) {
        self.0.extend_from_slice(line);
        if self.0.len() > TAIL_BYTES {
            let cut = self.0.len() - TAIL_BYTES;
            self.0.drain(..cut);
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// The last meaningful lines of llama-server's output, condensed to one line.
fn summarize(tail: &[u8]) -> String {
    let text = String::from_utf8_lossy(tail);
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let mut s = lines[lines.len().saturating_sub(3)..].join(" | ");
    if s.len() > 600 {
        let mut cut = s.len() - 600;
        while !s.is_char_boundary(cut) {
            cut += 1;
        }
        s = format!("…{}", &s[cut..]);
    }
    s
}

#[cfg(unix)]
mod reaper {
    //! Kills the children still registered when the worker exits normally (including the `exit` the
    //! worker calls when Debrief closes its stdin).
    use std::sync::{Mutex, Once};

    static PIDS: Mutex<Vec<i32>> = Mutex::new(Vec::new());
    static HOOK: Once = Once::new();

    extern "C" fn kill_all() {
        // try_lock: never block in an exit hook.
        if let Ok(pids) = PIDS.try_lock() {
            for pid in pids.iter() {
                // SAFETY: kill takes plain integers; the pid is a child we started and have not reaped.
                unsafe {
                    libc::kill(*pid, libc::SIGKILL);
                }
            }
        }
    }

    pub fn register(pid: i32) {
        HOOK.call_once(|| {
            // SAFETY: registers a plain extern "C" function without captured state.
            unsafe {
                libc::atexit(kill_all);
            }
        });
        PIDS.lock().unwrap_or_else(|p| p.into_inner()).push(pid);
    }

    pub fn unregister(pid: i32) {
        PIDS.lock().unwrap_or_else(|p| p.into_inner()).retain(|p| *p != pid);
    }
}

#[cfg(windows)]
mod job {
    //! A job object that kills llama-server when the worker's handle closes (the worker exits or crashes).
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    pub struct JobGuard(HANDLE);

    // SAFETY: a job handle may be used and closed from any thread.
    unsafe impl Send for JobGuard {}

    impl JobGuard {
        /// Best effort: `None` when the job cannot be created (the child is still killed on drop).
        pub fn assign(child: &Child) -> Option<JobGuard> {
            // SAFETY: plain Win32 calls with valid arguments; the handle is closed in `Drop`.
            unsafe {
                let job = CreateJobObjectW(None, None).ok()?;
                let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let ok = SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const std::ffi::c_void,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
                .is_ok()
                    && AssignProcessToJobObject(job, HANDLE(child.as_raw_handle())).is_ok();
                if !ok {
                    let _ = CloseHandle(job);
                    return None;
                }
                Some(JobGuard(job))
            }
        }
    }

    impl Drop for JobGuard {
        fn drop(&mut self) {
            // SAFETY: the handle was created by `assign` and is closed once.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

/// A free port on 127.0.0.1 (another process may still take it before llama-server binds it; the
/// start is retried on a new port in that case).
pub fn free_port() -> std::io::Result<u16> {
    Ok(TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?.local_addr()?.port())
}

/// A random API key (hex), so no other local process can use the server.
pub fn random_key() -> String {
    let mut b = [0u8; 24];
    if getrandom::fill(&mut b).is_err() {
        // Not expected; fall back to the clock so the key is at least not constant.
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        b[..16].copy_from_slice(&t.to_le_bytes());
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// A command for llama-server with the empty environment and no window.
pub fn command(exe: &Path, args: &[String]) -> Command {
    // No console window; below-normal priority on Windows (on Unix the priority is inherited from the worker).
    let mut cmd = quiet_command_with(exe, BELOW_NORMAL_PRIORITY_CLASS);
    cmd.args(args).stdin(Stdio::null()).env_clear();
    for k in KEPT_ENV {
        if let Some(v) = std::env::var_os(k) {
            cmd.env(k, v);
        }
    }
    if let Some(dir) = exe.parent().filter(|d| d.is_dir()) {
        cmd.current_dir(dir);
        // The prebuilt Linux builds load their own libraries from their folder.
        #[cfg(target_os = "linux")]
        {
            let mut paths = vec![dir.to_path_buf()];
            if let Some(old) = std::env::var_os("LD_LIBRARY_PATH") {
                paths.extend(std::env::split_paths(&old));
            }
            if let Ok(joined) = std::env::join_paths(paths) {
                cmd.env("LD_LIBRARY_PATH", joined);
            }
        }
    }
    cmd
}

/// Spawns `cmd`, retrying for up to a second while the executable is busy (`ETXTBSY`).
///
/// On Linux, exec fails with "Text file busy" while any process still holds the file open for writing. A
/// runtime that was just unpacked (or, in tests, copied) can hit this when another thread forks at the
/// same moment and the child briefly inherits the write handle; it clears as soon as that child execs.
pub fn spawn_retrying(cmd: &mut Command) -> std::io::Result<Child> {
    retry_while_busy(|| cmd.spawn(), Duration::from_millis(20), 50)
}

fn retry_while_busy<T>(mut f: impl FnMut() -> std::io::Result<T>, pause: Duration, attempts: u32) -> std::io::Result<T> {
    let mut left = attempts;
    loop {
        match f() {
            Err(e) if left > 1 && is_busy(&e) => {
                left -= 1;
                std::thread::sleep(pause);
            }
            other => return other,
        }
    }
}

#[cfg(unix)]
fn is_busy(e: &std::io::Error) -> bool {
    e.raw_os_error() == Some(libc::ETXTBSY)
}

#[cfg(not(unix))]
fn is_busy(_: &std::io::Error) -> bool {
    false
}

/// Runs a short llama-server command (e.g. `--list-devices`) and returns its standard output, killing it
/// after `timeout`.
pub fn run_briefly(exe: &Path, args: &[String], timeout: Duration) -> Result<String, String> {
    let mut cmd = command(exe, args);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = spawn_retrying(&mut cmd).map_err(|e| format!("cannot run {}: {e}", exe.display()))?;
    let out = child.stdout.take().map(|s| std::thread::spawn(move || read_capped(s, 256 * 1024)));
    let err = child.stderr.take().map(|s| std::thread::spawn(move || read_capped(s, 64 * 1024)));
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let out = out.and_then(|h| h.join().ok()).unwrap_or_default();
    let err = err.and_then(|h| h.join().ok()).unwrap_or_default();
    match status {
        None => Err(format!("{} did not answer within {} s", exe.display(), timeout.as_secs())),
        Some(s) if s.success() => Ok(String::from_utf8_lossy(&out).into_owned()),
        Some(s) => Err(format!("{} failed ({s}): {}", exe.display(), summarize(&err))),
    }
}

/// llama-server's output goes on to Debrief through the worker's standard error, tagged so Debrief keeps it in a log
/// of its own (`debrief_ipc::child_log`: `[llama-server] text`, and `[llama-server] @debrief start|exit ...` for its
/// lifecycle). Debrief discards the worker's standard error unless it captures it.
fn forward_line(line: &[u8]) {
    let text = String::from_utf8_lossy(line);
    let text = text.trim_end();
    if !text.is_empty() {
        eprintln!("[llama-server] {text}");
    }
}

fn forward_event(words: &str) {
    eprintln!("[llama-server] @debrief {words}");
}

/// The exit as `debrief_ipc::child_log::Exit::wire` writes it: `code 3`, `signal 11`, `stopped`, `unknown`.
fn exit_words(status: Option<&ExitStatus>) -> String {
    match status {
        Some(s) => match s.code() {
            Some(c) => format!("code {c}"),
            None => {
                #[cfg(unix)]
                {
                    use std::os::unix::process::ExitStatusExt;
                    if let Some(sig) = s.signal() {
                        return format!("signal {sig}");
                    }
                }
                "unknown".into()
            }
        },
        None => "stopped".into(),
    }
}

fn read_capped(r: impl std::io::Read, cap: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let _ = std::io::Read::read_to_end(&mut r.take(cap as u64), &mut out);
    out
}

/// A running llama-server. Stopped (killed and reaped) on drop.
pub struct ServerChild {
    child: Option<Child>,
    pub exe: PathBuf,
    client: Client,
    tail: Arc<Mutex<Tail>>,
    /// Its exit was forwarded to Debrief (see [`forward_event`]).
    exit_reported: bool,
    #[cfg(windows)]
    _job: Option<job::JobGuard>,
}

impl ServerChild {
    /// Starts llama-server with `args` (the host, port and key are added here) and waits until `/health`
    /// answers 200. A start that loses its port to another process is retried on a new port.
    pub fn start(exe: &Path, args: &[String], ready_timeout: Duration, cancelled: &dyn Fn() -> bool) -> Result<ServerChild, StartError> {
        let mut last = StartError::TimedOut;
        for _ in 0..3 {
            let port = free_port().map_err(|e| StartError::Spawn(format!("no free local port: {e}")))?;
            let mut child = Self::spawn(exe, args, port)?;
            match child.wait_ready(ready_timeout, cancelled) {
                Ok(()) => return Ok(child),
                Err(StartError::Exited(m)) if m.contains("bind") || m.contains("address already in use") => last = StartError::Exited(m),
                Err(e) => return Err(e),
            }
        }
        Err(last)
    }

    fn spawn(exe: &Path, args: &[String], port: u16) -> Result<ServerChild, StartError> {
        if !exe.is_file() {
            return Err(StartError::Spawn(format!("llama-server was not found at {}", exe.display())));
        }
        let key = random_key();
        let mut full = args.to_vec();
        full.extend(["--host".into(), "127.0.0.1".into(), "--port".into(), port.to_string()]);
        let mut cmd = command(exe, &full);
        cmd.env("LLAMA_API_KEY", &key).stdout(Stdio::null()).stderr(Stdio::piped());
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::process::CommandExt;
            // SAFETY: only async-signal-safe calls between fork and exec. The worker starts llama-server
            // from one long-lived thread, so the "parent thread exits" case of PDEATHSIG does not apply.
            unsafe {
                cmd.pre_exec(|| {
                    libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL as libc::c_ulong);
                    Ok(())
                });
            }
        }
        let mut child = spawn_retrying(&mut cmd).map_err(|e| {
            StartError::Spawn(match e.kind() {
                std::io::ErrorKind::NotFound => format!("llama-server was not found at {}", exe.display()),
                std::io::ErrorKind::PermissionDenied => format!("llama-server at {} is not executable", exe.display()),
                _ => format!("cannot start llama-server ({}): {e}", exe.display()),
            })
        })?;
        #[cfg(unix)]
        reaper::register(child.id() as i32);
        #[cfg(windows)]
        let job = job::JobGuard::assign(&child);
        forward_event(&format!("start {}", child.id()));
        let tail = Arc::new(Mutex::new(Tail::default()));
        if let Some(err) = child.stderr.take() {
            let tail = tail.clone();
            std::thread::spawn(move || {
                let mut r = BufReader::new(err);
                let mut line = Vec::new();
                loop {
                    line.clear();
                    match r.by_ref().take(8192).read_until(b'\n', &mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            lock(&tail).push(&line);
                            forward_line(&line);
                        }
                    }
                }
            });
        }
        Ok(ServerChild {
            child: Some(child),
            exe: exe.to_path_buf(),
            client: Client { port, key },
            tail,
            exit_reported: false,
            #[cfg(windows)]
            _job: job,
        })
    }

    fn wait_ready(&mut self, timeout: Duration, cancelled: &dyn Fn() -> bool) -> Result<(), StartError> {
        let deadline = Instant::now() + timeout;
        loop {
            if cancelled() {
                self.stop();
                return Err(StartError::Cancelled);
            }
            if let Some(status) = self.exit_status() {
                return Err(StartError::Exited(self.describe_exit(Some(status))));
            }
            let call = Call {
                method: "GET",
                path: "/health",
                body: None,
                idle: Duration::from_secs(2),
                cancelled,
            };
            match self.client.send(&call, 64 * 1024) {
                Ok(r) if r.status == 200 => return Ok(()),
                Ok(_) | Err(HttpError::Connect(_) | HttpError::Io(_) | HttpError::TimedOut) => {}
                Err(HttpError::Cancelled) => continue,
                Err(e) => {
                    self.stop();
                    return Err(StartError::Exited(format!("llama-server is not healthy: {e}")));
                }
            }
            if Instant::now() >= deadline {
                self.stop();
                return Err(StartError::TimedOut);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn port(&self) -> u16 {
        self.client.port
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().map(Child::id)
    }

    fn exit_status(&mut self) -> Option<ExitStatus> {
        match self.child.as_mut() {
            Some(c) => c.try_wait().ok().flatten(),
            None => None,
        }
    }

    /// Whether the process is still running.
    pub fn alive(&mut self) -> bool {
        self.child.is_some() && self.exit_status().is_none()
    }

    /// "llama-server exited (exit code: 3): <its last error lines>".
    pub fn describe_exit(&mut self, status: Option<ExitStatus>) -> String {
        // Give the stderr reader a moment to collect the last lines.
        std::thread::sleep(Duration::from_millis(50));
        let status = status.or_else(|| self.exit_status());
        if let Some(s) = &status {
            self.report_exit(Some(s));
        }
        let how = match status {
            Some(s) => format!("llama-server exited ({s})"),
            None => "llama-server stopped answering".to_string(),
        };
        match summarize(&lock(&self.tail).0) {
            t if t.is_empty() => how,
            t => format!("{how}: {t}"),
        }
    }

    /// Kills and reaps the process.
    pub fn stop(&mut self) {
        // One that already ended on its own is reported as such (crash or normal), not as stopped.
        if let Some(s) = self.exit_status() {
            self.report_exit(Some(&s));
        }
        if let Some(mut c) = self.child.take() {
            #[cfg(unix)]
            reaper::unregister(c.id() as i32);
            let pid = c.id();
            let _ = c.kill();
            let _ = c.wait();
            self.report_exit_of(pid, None);
        }
    }

    fn report_exit(&mut self, status: Option<&ExitStatus>) {
        if let Some(pid) = self.pid() {
            self.report_exit_of(pid, status);
        }
    }

    /// Forwards how llama-server ended, once (`None`: the worker stopped it).
    fn report_exit_of(&mut self, pid: u32, status: Option<&ExitStatus>) {
        if self.exit_reported {
            return;
        }
        self.exit_reported = true;
        forward_event(&format!("exit {pid} {}", exit_words(status)));
    }
}

impl Drop for ServerChild {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    #[test]
    fn a_busy_executable_is_retried_until_it_can_run() {
        let mut calls = 0;
        let r = super::retry_while_busy(
            || {
                calls += 1;
                if calls < 3 {
                    Err(std::io::Error::from_raw_os_error(libc::ETXTBSY))
                } else {
                    Ok(calls)
                }
            },
            std::time::Duration::ZERO,
            5,
        );
        assert_eq!(r.unwrap(), 3);
        // It gives up after the last attempt, and other errors are returned at once.
        let mut calls = 0;
        let r: std::io::Result<()> = super::retry_while_busy(
            || {
                calls += 1;
                Err(std::io::Error::from_raw_os_error(libc::ETXTBSY))
            },
            std::time::Duration::ZERO,
            4,
        );
        assert_eq!((r.unwrap_err().raw_os_error(), calls), (Some(libc::ETXTBSY), 4));
        let mut calls = 0;
        let r: std::io::Result<()> = super::retry_while_busy(
            || {
                calls += 1;
                Err(std::io::Error::from(std::io::ErrorKind::NotFound))
            },
            std::time::Duration::ZERO,
            4,
        );
        assert_eq!((r.unwrap_err().kind(), calls), (std::io::ErrorKind::NotFound, 1));
    }

    use super::*;

    #[test]
    fn tails_and_summaries_are_bounded() {
        let mut t = Tail::default();
        for i in 0..1000 {
            t.push(format!("line {i}\n").as_bytes());
        }
        assert!(t.0.len() <= TAIL_BYTES);
        assert_eq!(summarize(&t.0), "line 997 | line 998 | line 999");
        assert_eq!(summarize(b""), "");
        let long = "é".repeat(800);
        let s = summarize(long.as_bytes());
        assert!(s.starts_with('…') && s.len() <= 610);
    }

    #[test]
    fn exits_are_forwarded_in_the_child_log_wire_form() {
        assert_eq!(exit_words(None), "stopped");
        let ok = debrief_core::process::quiet_command(std::env::current_exe().unwrap())
            .arg("--list")
            .stdout(Stdio::null())
            .status()
            .unwrap();
        assert_eq!(exit_words(Some(&ok)), "code 0");
    }

    #[test]
    fn keys_and_ports() {
        let (a, b) = (random_key(), random_key());
        assert_eq!(a.len(), 48);
        assert_ne!(a, b);
        assert!(free_port().unwrap() > 0);
        for e in [
            StartError::Spawn("x".into()),
            StartError::Exited("y".into()),
            StartError::TimedOut,
            StartError::Cancelled,
        ] {
            assert!(!e.to_string().is_empty());
        }
    }

    #[test]
    fn missing_executables_fail_cleanly() {
        let e = ServerChild::start(Path::new("/definitely/not/llama-server"), &[], Duration::from_secs(1), &|| false)
            .err()
            .unwrap();
        assert!(matches!(&e, StartError::Spawn(m) if m.contains("not found")), "{e:?}");
        assert!(run_briefly(Path::new("/definitely/not/llama-server"), &[], Duration::from_secs(1)).is_err());
    }
}
