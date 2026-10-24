//! Starting a child process the supervisor owns: below-normal priority, no console window, a cleared
//! environment, killed with Debrief (a job object with KILL_ON_JOB_CLOSE on Windows, which also carries the
//! sandbox's limits; on Unix the child exits when its stdin, the lifeline, closes). [`crate::launch`] writes its
//! bootstrap to stdin and accepts its connection.

//!
//! With [`SpawnSpec::log`] the child's standard error is captured into its [`crate::child_log`] (with start
//! and exit lines); without it, standard error is discarded.

use crate::child_log::{self, ChildLog, ChildLogSpec, Exit};
use crate::memory_watch::{MemoryEvent, MemoryListener, MemoryWatch, Verdict};
use crate::IpcError;
use std::io::{BufRead, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Longest piece of a child's standard error read as one line.
const MAX_STDERR_LINE: u64 = 8 * 1024;
/// How long the standard error reader waits for the OS to publish an exit status after end of output.
const EXIT_WAIT: Duration = Duration::from_secs(2);

/// Environment variables every child gets by default ([`SpawnSpec::keep_env`]): what the OS and the
/// runtime need, and the coverage profile path so instrumented test builds can report. Nothing else of
/// Debrief's environment (tokens, proxies, user variables) reaches a child unless it is named.
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
    /// The only variables of Debrief's environment the child inherits (default [`KEPT_ENV`]); every child
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
    /// Hears what the memory watcher sees of a capped child (the warning at 80%, the stop, the peak at the end; see
    /// [`crate::memory_watch`]). Ignored without a memory cap.
    pub memory_listener: Option<MemoryListener>,
    /// The private socket the child connects to (Unix; set by [`crate::launch::launch`]): the one socket a contained
    /// macOS child's sandbox profile lets it reach.
    pub ipc_socket: Option<PathBuf>,
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
            endpoint_prefix: "debrief-ipc".into(),
            connect_window: Duration::from_secs(15),
            below_normal_priority: true,
            log: None,
            sandbox: crate::sandbox::SandboxSpec::unrestricted(),
            memory_listener: None,
            ipc_socket: None,
        }
    }
}

/// The exit line of one child, written once (by whoever sees the exit first).
struct ExitNote {
    log: Arc<ChildLog>,
    pid: u32,
    written: AtomicBool,
    /// Debrief asked the child to stop: whatever its status, it was stopped by Debrief.
    stopping: AtomicBool,
}

impl ExitNote {
    fn write(&self, status: Option<ExitStatus>) {
        if self.written.swap(true, Ordering::SeqCst) {
            return;
        }
        let exit = if self.stopping.load(Ordering::SeqCst) {
            Exit::StoppedByDebrief
        } else {
            status.as_ref().map(Exit::of).unwrap_or(Exit::Unknown)
        };
        self.log.exited(Some(self.pid), exit);
    }
}

/// The OS process behind a [`ChildProcess`]: started by the standard library, or (a contained child on Windows) in
/// an AppContainer by [`crate::contain`].
enum OsChild {
    Std(Child),
    #[cfg(windows)]
    Contained(crate::contain::Contained),
}

impl OsChild {
    fn id(&self) -> u32 {
        match self {
            OsChild::Std(c) => c.id(),
            #[cfg(windows)]
            OsChild::Contained(c) => c.id(),
        }
    }

    fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        match self {
            OsChild::Std(c) => c.try_wait(),
            #[cfg(windows)]
            OsChild::Contained(c) => c.try_wait(),
        }
    }

    /// Ends the child's whole process group (Unix: every child is the leader of its own group, see
    /// [`ChildProcess::spawn`]), so what it started (llama-server under the AI worker) ends with it even when its
    /// own clean-up never runs. Windows children are in a job object that does the same.
    fn kill_group(&self) {
        #[cfg(unix)]
        {
            // Unix children are all plain ones (`Contained` is Windows only).
            let OsChild::Std(c) = self;
            // SAFETY: killpg takes plain integers; the group is the one this child leads (it is not reaped yet, or
            // its group still has members, so the id is not reused).
            unsafe {
                libc::killpg(c.id() as libc::pid_t, libc::SIGKILL);
            }
        }
    }

    fn kill(&mut self) -> std::io::Result<()> {
        match self {
            OsChild::Std(c) => c.kill(),
            #[cfg(windows)]
            OsChild::Contained(c) => c.kill(),
        }
    }

    fn wait(&mut self) -> std::io::Result<ExitStatus> {
        match self {
            OsChild::Std(c) => c.wait(),
            #[cfg(windows)]
            OsChild::Contained(c) => c.wait(),
        }
    }
}

/// A job object on Windows; nothing elsewhere.
#[cfg(windows)]
type Job = Option<crate::job::JobGuard>;
#[cfg(not(windows))]
type Job = ();

/// A started child. Killed (and reaped) on drop.
pub struct ChildProcess {
    child: Arc<Mutex<Option<OsChild>>>,
    stdin: Mutex<Option<Box<dyn Write + Send>>>,
    pid: u32,
    label: String,
    exit_note: Option<Arc<ExitNote>>,
    /// The memory cap the host stopped the child over, once it did.
    over_memory: Arc<Mutex<Option<u64>>>,
    /// The most memory the watcher saw the child use (bytes; 0 when it is not watched).
    memory_peak: Arc<AtomicU64>,
    /// What the launcher could not enforce of its sandbox.
    sandbox_notes: Vec<crate::sandbox::Note>,
    _job: Job,
}

/// How often the host looks at the memory of a capped child.
pub const MEMORY_LOOK: Duration = Duration::from_millis(200);

/// "512 MB": a memory cap for messages.
fn megabytes(bytes: u64) -> String {
    format!("{} MB", bytes.div_ceil(1 << 20))
}

/// What the memory watcher of one child shares with its [`ChildProcess`].
struct Watched {
    pid: u32,
    child: Arc<Mutex<Option<OsChild>>>,
    over: Arc<Mutex<Option<u64>>>,
    peak: Arc<AtomicU64>,
    log: Option<Arc<ChildLog>>,
    label: String,
    listener: Option<MemoryListener>,
}

/// Watches the resident memory of the child `pid` and stops it once it uses more than `limit` (the memory cap of its
/// sandbox), the same way on every platform: rlimits and job commit limits count reservations and commitments that
/// allocators make up front (see `SandboxSpec::memory_limit`). Tells the listener at 80% of the cap, at the stop and
/// when the child ends ([`crate::memory_watch`]). Ends with the child.
fn watch_memory(limit: u64, w: Watched) {
    let spawned = std::thread::Builder::new().name("debrief-child-memory".into()).spawn(move || {
        let id = sysinfo::Pid::from_u32(w.pid);
        let mut sys = sysinfo::System::new();
        let mut watch = MemoryWatch::new(limit);
        let mut last = 0;
        let tell = |e: MemoryEvent| {
            if let Some(l) = &w.listener {
                l.tell(&w.label, e);
            }
        };
        loop {
            std::thread::sleep(MEMORY_LOOK);
            let ended = lock(&w.child).as_mut().is_none_or(|c| matches!(c.try_wait(), Ok(Some(_))));
            if ended {
                return tell(MemoryEvent::Ended(watch.ended(last)));
            }
            sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[id]), true);
            last = sys.process(id).map(|p| p.memory()).unwrap_or(0);
            let verdict = watch.observe(last);
            w.peak.store(watch.peak(), Ordering::Relaxed);
            match verdict {
                Verdict::Fine => {}
                Verdict::Warn(u) => tell(MemoryEvent::Warning(u)),
                Verdict::Stop(u) => {
                    *lock(&w.over) = Some(limit);
                    if let Some(l) = &w.log {
                        l.record(&child_log::ChildEvent::Note(format!(
                            "Stopped: it used {} of memory, more than its cap of {}",
                            megabytes(u.used),
                            megabytes(limit)
                        )));
                    }
                    if let Some(c) = lock(&w.child).as_mut() {
                        let _ = c.kill();
                    }
                    return tell(MemoryEvent::Stopped(u));
                }
            }
        }
    });
    drop(spawned);
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Reads a child's standard error into its log until end of output, then records how it exited (unless
/// Debrief is already stopping it, which records that itself).
fn capture_stderr(stderr: impl Read + Send + 'static, log: Arc<ChildLog>, child: Arc<Mutex<Option<OsChild>>>, note: Arc<ExitNote>) {
    let spawned = std::thread::Builder::new().name("debrief-child-stderr".into()).spawn(move || {
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
                // Taken: Debrief is stopping it and records the exit.
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
    /// log when the spec names one (discarded otherwise). A spec whose sandbox is
    /// [`crate::sandbox::Containment::Launcher`] is contained before it runs any code.
    pub fn spawn(spec: &SpawnSpec) -> Result<ChildProcess, IpcError> {
        let fail = |e: String| {
            let msg = format!("cannot start {} ({}): {e}", spec.label, spec.exe.display());
            if let Some(l) = &spec.log {
                child_log::get(l).record(&child_log::ChildEvent::Note(format!("Could not start: {msg}")));
            }
            IpcError::Spawn(msg)
        };
        #[cfg(windows)]
        if let crate::sandbox::Containment::Launcher { name, read, write } = &spec.sandbox.containment {
            let container = crate::contain::Container::open(name).map_err(|e| fail(e.to_string()))?;
            for (paths, w) in [(read, false), (write, true)] {
                for p in paths {
                    container.grant(p, w).map_err(|e| fail(e.to_string()))?;
                }
            }
            let mut contained = spec.clone();
            // The container can only enter folders granted to it.
            if !read.iter().chain(write).any(|r| Some(r) == contained.current_dir.as_ref()) {
                contained.current_dir = read.first().cloned();
            }
            let limits = crate::job::JobLimits::of(spec);
            let started = crate::contain::spawn(&contained, &container, spec.sandbox.network, &limits, spec.log.is_some())
                .map_err(|e| fail(e.to_string()))?;
            let stderr = started.stderr.map(|f| Box::new(f) as Box<dyn Read + Send>);
            return Ok(ChildProcess::assemble(
                spec,
                OsChild::Contained(started.child),
                Some(Box::new(started.stdin)),
                stderr,
                Some(started.job),
            ));
        }
        let (exe, args) = launcher_command(spec);
        // The one spawner outside debrief-core (this crate stands alone); it sets CREATE_NO_WINDOW below.
        #[allow(clippy::disallowed_methods)]
        let mut cmd = Command::new(&exe);
        cmd.args(&args)
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
        // Its own process group, so a kill reaches what it starts too (posix_spawn supports it; no `pre_exec`).
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // No console window; below-normal priority when asked.
            let priority = if spec.below_normal_priority { 0x0000_4000 } else { 0 };
            cmd.creation_flags(0x0800_0000 | priority);
        }
        #[cfg(target_os = "linux")]
        let report = match spec.sandbox.contained() {
            true => crate::sandbox::contain_before_exec(&mut cmd, &spec.sandbox, &spec.exe).map_err(fail)?,
            false => None,
        };
        let mut child = cmd.spawn().map_err(|e| fail(e.to_string()))?;
        #[cfg(target_os = "linux")]
        let sandbox_notes = report.map(|r| r.notes()).unwrap_or_default();
        #[cfg(not(target_os = "linux"))]
        let sandbox_notes: Vec<crate::sandbox::Note> = Vec::new();
        // Lowered from here rather than in a `pre_exec` hook: a hook forces fork+exec instead of
        // posix_spawn, and on macOS a concurrent fork can then inherit another spawn's exec-status pipe
        // and keep that spawn waiting forever.
        #[cfg(unix)]
        if spec.below_normal_priority {
            // SAFETY: setpriority takes plain integers and has no memory effects; it only lowers the
            // priority of our own child.
            unsafe {
                libc::setpriority(libc::PRIO_PROCESS, child.id() as libc::id_t, unix_nice(&spec.sandbox));
            }
        }
        #[cfg(windows)]
        let job = crate::job::JobGuard::assign(&child, &crate::job::JobLimits::of(spec));
        #[cfg(not(windows))]
        let job = ();
        let stdin = child.stdin.take().map(|s| Box::new(s) as Box<dyn Write + Send>);
        let stderr = child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>);
        let mut process = ChildProcess::assemble(spec, OsChild::Std(child), stdin, stderr, job);
        if let Some(log) = process.log() {
            for n in &sandbox_notes {
                log.record(&child_log::ChildEvent::Note(n.log_line()));
            }
        }
        process.sandbox_notes = sandbox_notes;
        Ok(process)
    }

    /// The bookkeeping of a started child: its log, its exit line and its standard error reader.
    fn assemble(
        spec: &SpawnSpec,
        child: OsChild,
        stdin: Option<Box<dyn Write + Send>>,
        stderr: Option<Box<dyn Read + Send>>,
        job: Job,
    ) -> ChildProcess {
        let pid = child.id();
        let child = Arc::new(Mutex::new(Some(child)));
        let over_memory = Arc::new(Mutex::new(None));
        let memory_peak = Arc::new(AtomicU64::new(0));
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
        if let Some(limit) = spec.sandbox.memory_limit {
            watch_memory(
                limit,
                Watched {
                    pid,
                    child: child.clone(),
                    over: over_memory.clone(),
                    peak: memory_peak.clone(),
                    log: exit_note.as_ref().map(|n| n.log.clone()),
                    label: spec.label.clone(),
                    listener: spec.memory_listener.clone(),
                },
            );
        }
        ChildProcess {
            child,
            stdin: Mutex::new(stdin),
            pid,
            label: spec.label.clone(),
            exit_note,
            over_memory,
            memory_peak,
            sandbox_notes: Vec::new(),
            _job: job,
        }
    }

    /// The child's log, when its output is captured.
    pub fn log(&self) -> Option<&Arc<ChildLog>> {
        self.exit_note.as_ref().map(|n| &n.log)
    }

    /// From now on, however the child ends, Debrief stopped it.
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

    /// The most memory the host's watcher saw the child use, in bytes (0 when it has no memory cap).
    pub fn memory_peak(&self) -> u64 {
        self.memory_peak.load(Ordering::Relaxed)
    }

    /// What the launcher could not enforce of the child's sandbox (Linux: the landlock status the child reported).
    pub fn sandbox_notes(&self) -> &[crate::sandbox::Note] {
        &self.sandbox_notes
    }

    /// Writes `value` as one JSON line to the child's stdin (its bootstrap).
    pub fn send_line<T: serde::Serialize>(&self, value: &T) -> Result<(), IpcError> {
        let line = serde_json::to_string(value).map_err(|e| IpcError::Protocol(e.to_string()))?;
        let wrote = lock(&self.stdin)
            .as_mut()
            .map(|s| s.write_all(format!("{line}\n").as_bytes()).and_then(|_| s.flush()));
        // The handshake is the first thing a child reads, so a failed write means it died while starting (one
        // that exits at once can break the pipe before Debrief writes).
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
        if let Some(limit) = *lock(&self.over_memory) {
            return format!(
                "{} used more than its memory cap of {} and was stopped",
                self.label,
                megabytes(limit)
            );
        }
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

    /// Closes the child's stdin: a well-behaved child exits on its own (this is what happens when Debrief dies).
    pub fn close_lifeline(&self) {
        self.note_own_exit();
        self.mark_stopping();
        *lock(&self.stdin) = None;
    }

    /// A child that already ended on its own is recorded as such (crash or normal), before Debrief stops it.
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
            // Whatever it started and left behind ends now too.
            c.kill_group();
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

/// The program and arguments that start `spec`: the program itself, or on macOS for a contained child,
/// `sandbox-exec` with the sandbox profile in front of it. The contained program is named by its resolved path: the
/// profile cannot read the symlinks on the way (macOS' `/var` → `/private/var`), so exec through them is denied.
fn launcher_command(spec: &SpawnSpec) -> (PathBuf, Vec<String>) {
    #[cfg(target_os = "macos")]
    if let Some(profile) = crate::sandbox::launcher_profile(&spec.sandbox, &spec.exe, spec.ipc_socket.as_deref()) {
        let exe = std::fs::canonicalize(&spec.exe).unwrap_or_else(|_| spec.exe.clone());
        let mut args = vec!["-p".to_string(), profile, exe.to_string_lossy().into_owned()];
        args.extend(spec.args.iter().cloned());
        return (PathBuf::from("/usr/bin/sandbox-exec"), args);
    }
    (spec.exe.clone(), spec.args.clone())
}

/// The nice level of a below-normal child on Unix: 10, or 19 (the lowest) when its sandbox asks for a CPU share below
/// 100% (Unix has no hard CPU cap for one process, so a share only lowers the priority; ADR 0031 §1).
#[cfg_attr(not(unix), allow(dead_code))]
fn unix_nice(sandbox: &crate::sandbox::SandboxSpec) -> i32 {
    match sandbox.cpu_rate_percent {
        Some(p) if p < 100 => 19,
        _ => 10,
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

    /// Regression: a contained plugin in a folder reached through a symlink (macOS' temp folder) exited with
    /// `sandbox-exec`'s status 71, as its profile cannot read the symlink to exec the program through it.
    #[test]
    #[cfg(target_os = "macos")]
    fn a_contained_program_is_started_by_its_resolved_path() {
        let t = tempfile::tempdir().unwrap();
        let real = t.path().canonicalize().unwrap().join("real");
        std::fs::create_dir_all(real.join("bin")).unwrap();
        std::fs::write(real.join("bin/x"), b"").unwrap();
        let link = t.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let mut spec = super::SpawnSpec::new(link.join("bin/x"), "x");
        spec.sandbox = crate::sandbox::SandboxSpec {
            containment: crate::sandbox::Containment::Launcher {
                name: "debrief.plugin.x".into(),
                read: vec![link.clone()],
                write: Vec::new(),
            },
            ..crate::sandbox::SandboxSpec::locked()
        };
        let (program, args) = super::launcher_command(&spec);
        assert_eq!(program, std::path::Path::new("/usr/bin/sandbox-exec"));
        assert_eq!(args[2], real.join("bin/x").to_string_lossy());
    }

    #[test]
    fn a_cpu_share_below_100_lowers_the_unix_priority_further() {
        let mut s = crate::sandbox::SandboxSpec::unrestricted();
        assert_eq!(super::unix_nice(&s), 10);
        s.cpu_rate_percent = Some(100);
        assert_eq!(super::unix_nice(&s), 10);
        s.cpu_rate_percent = Some(50);
        assert_eq!(super::unix_nice(&s), 19);
    }

    /// A child killed after its grace period takes what it started with it (the AI worker's llama-server, which on
    /// macOS has no parent-death signal and whose exit hook never runs under SIGKILL).
    #[cfg(unix)]
    #[test]
    fn a_child_killed_after_its_grace_takes_its_own_children_with_it() {
        let d = tempfile::tempdir().unwrap();
        let pid_file = d.path().join("grandchild.pid");
        let script = d.path().join("worker.sh");
        // A fake worker: starts a grandchild (the fake llama-server), ignores its lifeline and SIGTERM, and stays.
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\ntrap '' TERM\nsleep 60 &\necho $! > '{}'\nwhile :; do sleep 1; done\n",
                pid_file.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let child = super::ChildProcess::spawn(&super::SpawnSpec::new(&script, "fake worker")).unwrap();
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let grandchild: i32 = loop {
            if let Some(p) = std::fs::read_to_string(&pid_file).ok().and_then(|s| s.trim().parse().ok()) {
                break p;
            }
            assert!(std::time::Instant::now() < end, "the fake worker started its child");
            std::thread::yield_now();
        };
        child.kill();
        let alive = |pid: i32| unsafe { libc::kill(pid, 0) == 0 };
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while alive(grandchild) {
            if std::time::Instant::now() >= end {
                unsafe { libc::kill(grandchild, libc::SIGKILL) };
                panic!("the grandchild was left behind");
            }
            std::thread::yield_now();
        }
    }
}
