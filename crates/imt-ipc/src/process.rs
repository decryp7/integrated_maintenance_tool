//! Starting a child process the supervisor owns: below-normal priority, no console window, killed with
//! IMT (a job object with KILL_ON_JOB_CLOSE on Windows; on Unix the child exits when its stdin, the
//! lifeline, closes), told its private address and token on stdin, and connected to with retries.

use crate::client::{authenticated, connect_with_retry, AuthChannel};
use crate::endpoint::{new_token, Handshake, PrivateEndpoint};
use crate::IpcError;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::Mutex;
use std::time::Duration;

/// Environment variables a child started with [`SpawnSpec::clear_env`] still gets: what the OS and the
/// runtime need, and the coverage profile path so instrumented test builds can report.
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
    /// Start from an empty environment (plus [`KEPT_ENV`] and `env`), so the child learns nothing ambient.
    pub clear_env: bool,
    /// Working directory (default: inherited).
    pub current_dir: Option<PathBuf>,
    /// Names the process in messages: "the AI engine", "plugin \"x\"".
    pub label: String,
    /// Prefix of the private address (see [`PrivateEndpoint::create_named`]).
    pub endpoint_prefix: String,
    /// How long the child may take to start listening.
    pub connect_window: Duration,
    /// Run at below-normal priority so the analysis stays responsive.
    pub below_normal_priority: bool,
}

impl SpawnSpec {
    pub fn new(exe: impl Into<PathBuf>, label: impl Into<String>) -> SpawnSpec {
        SpawnSpec {
            exe: exe.into(),
            args: Vec::new(),
            env: Vec::new(),
            clear_env: false,
            current_dir: None,
            label: label.into(),
            endpoint_prefix: "imt-ipc".into(),
            connect_window: Duration::from_secs(15),
            below_normal_priority: true,
        }
    }
}

/// A started child. Killed (and reaped) on drop.
pub struct ChildProcess {
    child: Mutex<Option<Child>>,
    stdin: Mutex<Option<ChildStdin>>,
    pid: u32,
    label: String,
    #[cfg(windows)]
    _job: Option<crate::job::JobGuard>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl ChildProcess {
    /// Starts the program with a piped stdin (its lifeline) and no stdout/stderr.
    pub fn spawn(spec: &SpawnSpec) -> Result<ChildProcess, IpcError> {
        let mut cmd = Command::new(&spec.exe);
        cmd.args(&spec.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if spec.clear_env {
            cmd.env_clear();
            for k in KEPT_ENV {
                if let Some(v) = std::env::var_os(k) {
                    cmd.env(k, v);
                }
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
        let mut child = cmd
            .spawn()
            .map_err(|e| IpcError::Spawn(format!("cannot start {} ({}): {e}", spec.label, spec.exe.display())))?;
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
        let job = crate::job::JobGuard::assign(&child);
        let stdin = child.stdin.take();
        Ok(ChildProcess {
            child: Mutex::new(Some(child)),
            stdin: Mutex::new(stdin),
            pid,
            label: spec.label.clone(),
            #[cfg(windows)]
            _job: job,
        })
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Writes the handshake line to the child's stdin.
    pub fn send_handshake(&self, hs: &Handshake) -> Result<(), IpcError> {
        let line = serde_json::to_string(hs).map_err(|e| IpcError::Protocol(e.to_string()))?;
        let wrote = lock(&self.stdin)
            .as_mut()
            .map(|s| s.write_all(format!("{line}\n").as_bytes()).and_then(|_| s.flush()));
        match wrote {
            Some(Ok(())) => Ok(()),
            _ => Err(IpcError::Died(self.exit_text())),
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
        for _ in 0..20 {
            if let Some(s) = self.try_exit() {
                return format!("{} exited ({s})", self.label);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        format!("{} closed the connection", self.label)
    }

    /// Closes the child's stdin: a well-behaved child exits on its own (this is what happens when IMT dies).
    pub fn close_lifeline(&self) {
        *lock(&self.stdin) = None;
    }

    /// Stops the child and reaps it: its lifeline is closed first and a well-behaved child exits at once
    /// (see [`crate::server::child_main`]); one that has not after [`KILL_GRACE`] is killed.
    pub fn kill(&self) {
        *lock(&self.stdin) = None;
        if let Some(mut c) = lock(&self.child).take() {
            let deadline = std::time::Instant::now() + KILL_GRACE;
            while std::time::Instant::now() < deadline {
                if matches!(c.try_wait(), Ok(Some(_))) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            let _ = c.kill();
            let _ = c.wait();
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

/// A started, listening child and an authenticated channel to it.
pub struct Launched {
    pub process: ChildProcess,
    pub channel: AuthChannel,
    /// Kept so the private directory lives as long as the connection.
    pub endpoint: PrivateEndpoint,
}

/// Starts the child, hands it a fresh private address and token, and connects.
pub async fn launch(spec: &SpawnSpec) -> Result<Launched, IpcError> {
    let endpoint = PrivateEndpoint::create_named(&spec.endpoint_prefix)
        .map_err(|e| IpcError::Spawn(format!("cannot prepare a private socket: {e}")))?;
    let token = new_token().map_err(|e| IpcError::Spawn(e.to_string()))?;
    let process = ChildProcess::spawn(spec)?;
    if let Err(e) = process.send_handshake(&Handshake {
        address: endpoint.address.clone(),
        token: token.clone(),
    }) {
        process.kill();
        return Err(e);
    }
    let label = spec.label.clone();
    let channel = connect_with_retry(&endpoint.address, spec.connect_window, || {
        process.try_exit().map(|s| format!("{label} exited while starting ({s})"))
    })
    .await;
    let channel = match channel {
        Ok(c) => c,
        Err(e) => {
            process.kill();
            return Err(e);
        }
    };
    Ok(Launched {
        channel: authenticated(channel, &token)?,
        process,
        endpoint,
    })
}
