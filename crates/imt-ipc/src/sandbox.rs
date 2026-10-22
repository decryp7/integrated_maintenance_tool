//! What a child may do (ADR 0017 §6). The launcher applies the limits that must come from outside (a job
//! object on Windows, the environment, the working directory); the child applies the rest to itself with
//! [`lockdown`] right after it connected and before it handles any request.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// File-system access of a child.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "mode")]
pub enum FsAccess {
    /// No restriction beyond the OS user's rights.
    #[default]
    Unrestricted,
    /// No file access at all: files reach the child through the host (host files, ADR 0017 §4).
    Nothing,
    /// Only these paths (and what is below them).
    Paths {
        #[serde(default)]
        read: Vec<PathBuf>,
        #[serde(default)]
        write: Vec<PathBuf>,
    },
}

/// Who enforces a child's sandbox.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "mode")]
pub enum Containment {
    /// The child locks itself down after connecting ([`lockdown`]): IMT's own code, where the threat is an exploited
    /// parser, not a malicious binary.
    #[default]
    SelfApplied,
    /// The launcher contains the child before its first instruction, so a binary that never calls [`lockdown`]
    /// is still confined (third-party plugins, ADR 0027). The child still locks itself down on top.
    ///
    /// - Windows: an AppContainer named `name` (no access to the user's files or objects; network only with
    ///   [`SandboxSpec::network`], through the `internetClient` capability), inside a job object with the
    ///   sandbox's limits assigned while the process is still suspended; `readable` and `writable` are granted to
    ///   the container.
    /// - Linux: resource limits, `no_new_privs`, landlock (read and execute only below `readable` and the system
    ///   library folders, writes only below `writable`) and the seccomp network filter, applied between `fork` and
    ///   `exec`.
    /// - macOS: started through `sandbox-exec` with the sandbox profile.
    Launcher {
        /// A stable name of at most 64 characters from `A–Z a–z 0–9 . -` (the AppContainer profile's name).
        name: String,
        /// Folders the child may read (its own program folder).
        #[serde(default)]
        read: Vec<PathBuf>,
        /// Folders the child may write (its private data folder).
        #[serde(default)]
        write: Vec<PathBuf>,
    },
}

impl Containment {
    /// Whether `name` is a valid [`Containment::Launcher`] name.
    pub fn valid_name(name: &str) -> bool {
        !name.is_empty() && name.len() <= 64 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    }
}

/// The sandbox of one child. The default restricts nothing (workers that need the user's rights).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SandboxSpec {
    /// Memory cap in bytes: the memory the child uses (resident), watched by the host on every platform, which stops a
    /// child over it with a message that says so (`process::ChildProcess`). Never an rlimit: `RLIMIT_AS` and
    /// `RLIMIT_DATA` count reservations, which allocators such as mimalloc make up front, so capped plugins failed
    /// their first allocations and closed the connection. An elevated child, which the host cannot watch, caps itself
    /// with a job object (Windows).
    pub memory_limit: Option<u64>,
    /// CPU share cap in percent of the machine: exact on Windows (job object hard cap); on Unix a share below 100 only
    /// lowers the child's priority to the lowest (nice 19), as there is no per-process cap.
    pub cpu_rate_percent: Option<u8>,
    /// Total CPU seconds (`RLIMIT_CPU` on Unix; job per-process user time on Windows).
    pub cpu_seconds: Option<u64>,
    /// May start other programs.
    pub children: bool,
    /// Windows: run at low integrity (no writes to the user's files, no access to the app's objects).
    pub low_integrity: bool,
    /// May open network sockets (Linux: seccomp; macOS: the sandbox profile; Windows: not enforced).
    pub network: bool,
    pub fs: FsAccess,
    /// Who enforces the rest: the child itself (the default) or the launcher, before the child runs.
    pub containment: Containment,
}

impl SandboxSpec {
    /// No restrictions (the user's rights): `children` and `network` allowed.
    pub fn unrestricted() -> SandboxSpec {
        SandboxSpec {
            children: true,
            network: true,
            ..SandboxSpec::default()
        }
    }

    /// For code that parses untrusted input and needs nothing else: 512 MiB, no children, no network, no
    /// files, low integrity.
    pub fn locked() -> SandboxSpec {
        SandboxSpec {
            memory_limit: Some(512 << 20),
            cpu_rate_percent: None,
            cpu_seconds: None,
            children: false,
            low_integrity: true,
            network: false,
            fs: FsAccess::Nothing,
            containment: Containment::SelfApplied,
        }
    }

    /// Whether the launcher contains the child ([`Containment::Launcher`]).
    pub fn contained(&self) -> bool {
        matches!(self.containment, Containment::Launcher { .. })
    }

    /// Whether the child must lock itself down at all.
    pub fn restricts_child(&self) -> bool {
        !self.children || self.low_integrity || !self.network || self.fs != FsAccess::Unrestricted || self.memory_limit.is_some()
    }
}

/// What part of a [`SandboxSpec`] this system could not enforce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Gap {
    /// File-system restrictions (Linux: landlock missing, e.g. an old kernel or a container that filters its
    /// system calls).
    Files,
    /// System-call restrictions (Linux: seccomp unavailable): network sockets and program starts are allowed.
    Syscalls,
    /// Resource limits (memory, CPU) or the job of an elevated process.
    Limits,
    /// Not a gap: how this platform enforces the spec (e.g. Windows does not filter network access).
    Info,
}

/// A note about the lockdown of this process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    pub gap: Gap,
    pub message: String,
}

impl Note {
    pub fn new(gap: Gap, message: impl Into<String>) -> Note {
        Note {
            gap,
            message: message.into(),
        }
    }

    /// The line for the child's log: gaps are warnings ("sandbox degraded"), so they are never silent.
    pub fn log_line(&self) -> String {
        match self.gap {
            Gap::Info => format!("sandbox: {}", self.message),
            _ => format!("warning: sandbox degraded: {}", self.message),
        }
    }
}

/// A child's sandbox status when the kernel cannot confine its files (Linux without landlock). User decision
/// (2026-10-22): no setting and no refusal, the plugin runs and its status says this one line.
pub const FILES_NOT_CONFINED: &str = "files are not confined on this system (landlock is not enabled in the kernel)";

impl SandboxSpec {
    /// Whether the child's files are restricted (by the child's lockdown or by the launcher).
    pub fn restricts_files(&self) -> bool {
        self.fs != FsAccess::Unrestricted || self.contained()
    }
}

/// What this system cannot enforce of `spec`, known before the child starts (the host shows it as the child's
/// sandbox status): on Linux without landlock, [`FILES_NOT_CONFINED`] for a child whose files are restricted.
pub fn host_notes(spec: &SandboxSpec) -> Vec<Note> {
    files_note(spec.restricts_files(), files_confinable()).into_iter().collect()
}

fn files_note(restricts_files: bool, confinable: bool) -> Option<Note> {
    (restricts_files && !confinable).then(|| Note::new(Gap::Files, FILES_NOT_CONFINED))
}

/// Whether this system can confine a child's files: on Linux, whether the kernel enforces landlock; elsewhere the
/// platform's own mechanism does (AppContainer, low integrity, `sandbox-exec`). Asked once per process.
pub fn files_confinable() -> bool {
    #[cfg(target_os = "linux")]
    {
        static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ENABLED.get_or_init(platform::landlock_enabled)
    }
    #[cfg(not(target_os = "linux"))]
    {
        true
    }
}

static APPLIED: std::sync::OnceLock<Vec<Note>> = std::sync::OnceLock::new();

/// The notes of this process' lockdown (empty before it, or when nothing was restricted).
pub fn applied_notes() -> &'static [Note] {
    APPLIED.get().map_or(&[], Vec::as_slice)
}

/// Applies `spec` to the current process (the child side; irreversible). `elevated`: the child was started
/// through the administrator prompt, so IMT could neither clear its environment nor put it into a job: it does
/// both itself. Returns notes about what this system could not enforce (a kernel without landlock, a container
/// without seccomp): the restriction degrades and the note says so, so a plugin still runs; fails only when a
/// restriction that must hold could not be applied (lowering the integrity level on Windows, resource limits).
pub fn lockdown(spec: &SandboxSpec, elevated: bool) -> Result<Vec<Note>, String> {
    let mut notes = Vec::new();
    if elevated {
        clear_env(crate::process::KEPT_ENV);
    }
    platform::lockdown(spec, elevated, &mut notes)?;
    let _ = APPLIED.set(notes.clone());
    Ok(notes)
}

/// System folders a contained child may read and execute (the dynamic loader and the C library), Linux.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const SYSTEM_READ: &[&str] = &[
    "/usr",
    "/lib",
    "/lib64",
    "/lib32",
    "/etc/ld.so.cache",
    "/etc/ld.so.conf",
    "/etc/ld.so.conf.d",
];

/// Linux: contains `cmd` before it runs (see [`Containment::Launcher`]): resource limits, `no_new_privs`, landlock
/// (read and execute below `read`, the program's folder and [`SYSTEM_READ`]; writes below `write`) and the network
/// filter, applied in the forked child before `exec`. Everything is prepared before the fork.
#[cfg(target_os = "linux")]
pub(crate) fn contain_before_exec(
    cmd: &mut std::process::Command,
    spec: &SandboxSpec,
    exe: &std::path::Path,
) -> Result<Option<LauncherReport>, String> {
    use landlock::RulesetStatus;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::process::CommandExt;
    let Containment::Launcher { read, write, .. } = &spec.containment else {
        return Ok(None);
    };
    let mut readable: Vec<PathBuf> = read.clone();
    readable.extend(exe.parent().map(PathBuf::from));
    readable.extend(SYSTEM_READ.iter().map(PathBuf::from).filter(|p| p.exists()));
    let ruleset = std::sync::Mutex::new(Some(platform::launcher_ruleset(&readable, write)?));
    let filter = bpf::filter(!spec.network, false);
    let cpu = spec.cpu_seconds;
    // The child reports how far landlock confined it through this pipe (closed in the child by exec; neither end
    // blocks: the one byte is written before exec, and `spawn` returns after it).
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: pipe2 fills two descriptors it opened; each is owned once below.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) } != 0 {
        return Err(format!(
            "cannot create the sandbox status pipe: {}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: both descriptors were just opened by pipe2 and are owned by nothing else.
    let report = unsafe {
        LauncherReport {
            read: OwnedFd::from_raw_fd(fds[0]),
            write: Some(OwnedFd::from_raw_fd(fds[1])),
        }
    };
    let status_fd = report.write.as_ref().map_or(-1, |w| w.as_raw_fd());
    // SAFETY: the hook runs in the forked child before exec; it only makes system calls on data prepared above
    // (the limits, the landlock ruleset's descriptor, the BPF program and the status pipe). The ruleset's mutex is
    // never contended: only this hook takes it, so no lock held at fork time can block it.
    unsafe {
        cmd.pre_exec(move || {
            platform::set_limit(libc::RLIMIT_CPU, cpu).map_err(std::io::Error::other)?;
            if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if let Some(r) = ruleset.lock().ok().and_then(|mut g| g.take()) {
                let status = r.restrict_self().map_err(|e| std::io::Error::other(e.to_string()))?;
                let code: u8 = match status.ruleset {
                    RulesetStatus::FullyEnforced => LANDLOCK_FULL,
                    RulesetStatus::PartiallyEnforced => LANDLOCK_PARTIAL,
                    RulesetStatus::NotEnforced => LANDLOCK_NONE,
                };
                libc::write(status_fd, (&code as *const u8).cast(), 1);
            }
            let fprog = libc::sock_fprog {
                len: filter.len() as u16,
                filter: filter.as_ptr() as *mut libc::sock_filter,
            };
            if libc::prctl(libc::PR_SET_SECCOMP, libc::SECCOMP_MODE_FILTER, &fprog as *const libc::sock_fprog) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok(Some(report))
}

/// Status bytes a contained child writes between `fork` and `exec` (Linux): how far landlock confined it.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const LANDLOCK_FULL: u8 = 0;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const LANDLOCK_PARTIAL: u8 = 1;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const LANDLOCK_NONE: u8 = 2;

/// The notes of a contained child from the status it reported (`None`: it reported nothing).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn launcher_notes(status: Option<u8>) -> Vec<Note> {
    match status {
        Some(LANDLOCK_FULL) => Vec::new(),
        Some(LANDLOCK_PARTIAL) => vec![Note::new(
            Gap::Files,
            "landlock is only partly supported by this kernel: some file operations are not restricted",
        )],
        _ => vec![Note::new(Gap::Files, FILES_NOT_CONFINED)],
    }
}

/// What a contained child reported from between `fork` and `exec` (Linux: how far landlock confined it).
#[cfg(target_os = "linux")]
pub(crate) struct LauncherReport {
    read: std::os::fd::OwnedFd,
    write: Option<std::os::fd::OwnedFd>,
}

#[cfg(target_os = "linux")]
impl LauncherReport {
    /// The child's notes; call once `spawn` returned (the child wrote its status before it ran the program).
    pub(crate) fn notes(mut self) -> Vec<Note> {
        use std::os::fd::AsRawFd;
        drop(self.write.take());
        let mut b = [0u8; 1];
        // SAFETY: reads at most one byte into `b` from a descriptor this report owns.
        let n = unsafe { libc::read(self.read.as_raw_fd(), b.as_mut_ptr().cast(), 1) };
        launcher_notes((n == 1).then_some(b[0]))
    }
}

/// macOS: the profile `sandbox-exec` starts a contained child with (see [`Containment::Launcher`]): the sandbox's
/// profile with file access limited to `read` (and the program's folder) and `write`, plus permission to execute
/// the program itself. `None` for a child that is not contained.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn launcher_profile(spec: &SandboxSpec, exe: &std::path::Path) -> Option<String> {
    let Containment::Launcher { read, write, .. } = &spec.containment else {
        return None;
    };
    let mut s = spec.clone();
    let mut readable = read.clone();
    readable.extend(exe.parent().map(PathBuf::from));
    s.fs = FsAccess::Paths {
        read: readable,
        write: write.clone(),
    };
    let mut p = macos_profile(&s)?;
    p.push_str(&format!("(allow process-exec (literal {}))\n", sbpl_quote(exe)));
    Some(p)
}

/// A path as an SBPL string literal.
fn sbpl_quote(p: &std::path::Path) -> String {
    format!("\"{}\"", p.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\""))
}

/// Removes every environment variable but `keep`.
fn clear_env(keep: &[&str]) {
    let names: Vec<std::ffi::OsString> = std::env::vars_os().map(|(k, _)| k).collect();
    for k in names {
        if !keep.iter().any(|kept| std::ffi::OsStr::new(kept) == k.as_os_str()) {
            // Called before the child starts serving; the IPC runtime reads no environment.
            std::env::remove_var(&k);
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::{Gap, Note, SandboxSpec};
    use windows::core::w;
    use windows::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL};
    use windows::Win32::Security::Authorization::ConvertStringSidToSidW;
    use windows::Win32::Security::{
        GetLengthSid, SetTokenInformation, TokenIntegrityLevel, PSID, SID_AND_ATTRIBUTES, TOKEN_ADJUST_DEFAULT, TOKEN_MANDATORY_LABEL,
        TOKEN_QUERY,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    /// `SE_GROUP_INTEGRITY`.
    const SE_GROUP_INTEGRITY: u32 = 0x20;

    pub fn lockdown(spec: &SandboxSpec, elevated: bool, notes: &mut Vec<Note>) -> Result<(), String> {
        if elevated {
            // IMT could not put an elevated child into a job: it joins one itself (killed with its last handle,
            // which this process holds until it exits).
            match crate::job::JobGuard::assign_self(&crate::job::JobLimits::from_sandbox(spec)) {
                Some(job) => std::mem::forget(job),
                None => notes.push(Note::new(Gap::Limits, "could not apply the job limits")),
            }
        }
        // A contained child already runs in an AppContainer, which is below low integrity.
        if spec.low_integrity && !spec.contained() {
            lower_integrity()?;
        }
        if !spec.network {
            notes.push(Note::new(
                Gap::Info,
                "network access is not restricted on Windows (low integrity still applies)",
            ));
        }
        Ok(())
    }

    /// Lowers this process' token to low integrity (S-1-16-4096): no writes to the user's files and folders,
    /// no access to objects of medium-integrity processes. Handles opened before stay usable.
    fn lower_integrity() -> Result<(), String> {
        // SAFETY: Win32 calls with valid arguments; the SID and the token handle are freed below.
        unsafe {
            let mut sid = PSID::default();
            ConvertStringSidToSidW(w!("S-1-16-4096"), &mut sid).map_err(|e| format!("no low-integrity SID: {e}"))?;
            let mut token = HANDLE::default();
            let opened = OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_DEFAULT | TOKEN_QUERY, &mut token);
            let result = opened.and_then(|()| {
                let label = TOKEN_MANDATORY_LABEL {
                    Label: SID_AND_ATTRIBUTES {
                        Sid: sid,
                        Attributes: SE_GROUP_INTEGRITY,
                    },
                };
                let size = std::mem::size_of::<TOKEN_MANDATORY_LABEL>() as u32 + GetLengthSid(sid);
                SetTokenInformation(token, TokenIntegrityLevel, &label as *const _ as *const std::ffi::c_void, size)
            });
            if !token.is_invalid() {
                let _ = CloseHandle(token);
            }
            let _ = LocalFree(Some(HLOCAL(sid.0)));
            result.map_err(|e| format!("cannot lower the integrity level: {e}"))
        }
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::{FsAccess, Gap, Note, SandboxSpec};

    pub fn lockdown(spec: &SandboxSpec, _elevated: bool, notes: &mut Vec<Note>) -> Result<(), String> {
        // No memory rlimit: the host watches the memory the child uses (see `SandboxSpec::memory_limit`).
        set_limit(libc::RLIMIT_CPU, spec.cpu_seconds)?;
        landlock(&spec.fs, notes)?;
        if let Err(e) = seccomp(!spec.network, !spec.children) {
            notes.push(Note::new(
                Gap::Syscalls,
                format!("seccomp is not available ({e}): network sockets and program starts are not restricted"),
            ));
        }
        Ok(())
    }

    pub(super) fn set_limit(resource: libc::__rlimit_resource_t, value: Option<u64>) -> Result<(), String> {
        let Some(v) = value else { return Ok(()) };
        let lim = libc::rlimit {
            rlim_cur: v as libc::rlim_t,
            rlim_max: v as libc::rlim_t,
        };
        // SAFETY: a valid rlimit structure for this process.
        if unsafe { libc::setrlimit(resource, &lim) } != 0 {
            return Err(format!("setrlimit failed: {}", std::io::Error::last_os_error()));
        }
        Ok(())
    }

    fn landlock(fs: &FsAccess, notes: &mut Vec<Note>) -> Result<(), String> {
        use landlock::{Access, AccessFs, PathBeneath, PathFd, Ruleset, RulesetAttr, RulesetCreatedAttr, RulesetStatus, ABI};
        let (read, write) = match fs {
            FsAccess::Unrestricted => return Ok(()),
            FsAccess::Nothing => (Vec::new(), Vec::new()),
            FsAccess::Paths { read, write } => (read.clone(), write.clone()),
        };
        let abi = ABI::V3;
        let fail = |e: landlock::RulesetError| format!("landlock: {e}");
        // The same status line the host shows (`host_notes`), with the kernel's reason for the child's log.
        let unavailable =
            |notes: &mut Vec<Note>, why: String| notes.push(Note::new(Gap::Files, format!("{} ({why})", super::FILES_NOT_CONFINED)));
        let mut ruleset = match Ruleset::default().handle_access(AccessFs::from_all(abi)).and_then(|r| r.create()) {
            Ok(r) => r,
            Err(e) => {
                unavailable(notes, fail(e));
                return Ok(());
            }
        };
        for (paths, access) in [(read, AccessFs::from_read(abi)), (write, AccessFs::from_all(abi))] {
            for p in paths {
                match PathFd::new(&p) {
                    Ok(fd) => ruleset = ruleset.add_rule(PathBeneath::new(fd, access)).map_err(fail)?,
                    // A path that does not exist grants nothing: the restriction only gets stricter.
                    Err(e) => notes.push(Note::new(Gap::Info, format!("{}: {e}", p.display()))),
                }
            }
        }
        // Landlock restricts this thread and the threads it starts from now on, not those already running.
        let earlier = std::fs::read_dir("/proc/self/task")
            .map(|d| d.count().saturating_sub(1))
            .unwrap_or(0);
        notes.extend(super::earlier_threads_note(earlier));
        match ruleset.restrict_self() {
            Ok(status) => match status.ruleset {
                RulesetStatus::FullyEnforced => {}
                RulesetStatus::PartiallyEnforced => notes.push(Note::new(
                    Gap::Files,
                    "landlock is only partly supported by this kernel: some file operations are not restricted",
                )),
                RulesetStatus::NotEnforced => unavailable(notes, "not supported by this kernel or filtered by its container".into()),
            },
            Err(e) => unavailable(notes, fail(e)),
        }
        Ok(())
    }

    /// Whether the kernel enforces landlock (its first ABI at least): a ruleset that requires it can be created.
    pub(super) fn landlock_enabled() -> bool {
        use landlock::{Access, AccessFs, CompatLevel, Compatible, Ruleset, RulesetAttr, ABI};
        Ruleset::default()
            .set_compatibility(CompatLevel::HardRequirement)
            .handle_access(AccessFs::from_all(ABI::V1))
            .and_then(|r| r.create())
            .is_ok()
    }

    /// The landlock ruleset of a contained child (see [`super::contain_before_exec`]), created but not applied.
    pub(super) fn launcher_ruleset(read: &[std::path::PathBuf], write: &[std::path::PathBuf]) -> Result<landlock::RulesetCreated, String> {
        use landlock::{Access, AccessFs, PathBeneath, PathFd, Ruleset, RulesetAttr, RulesetCreatedAttr, ABI};
        let abi = ABI::V3;
        let fail = |e: landlock::RulesetError| format!("landlock: {e}");
        let mut ruleset = Ruleset::default()
            .handle_access(AccessFs::from_all(abi))
            .map_err(fail)?
            .create()
            .map_err(fail)?;
        for (paths, access) in [(read, AccessFs::from_read(abi)), (write, AccessFs::from_all(abi))] {
            for p in paths {
                if let Ok(fd) = PathFd::new(p) {
                    ruleset = ruleset.add_rule(PathBeneath::new(fd, access)).map_err(fail)?;
                }
            }
        }
        Ok(ruleset)
    }

    /// Installs [`super::bpf::filter`] on every thread of the process (`SECCOMP_FILTER_FLAG_TSYNC`; `prctl` would
    /// filter only the calling thread).
    fn seccomp(deny_network: bool, deny_exec: bool) -> Result<(), String> {
        if !deny_network && !deny_exec {
            return Ok(());
        }
        let prog = super::bpf::filter(deny_network, deny_exec);
        let fprog = libc::sock_fprog {
            len: prog.len() as u16,
            filter: prog.as_ptr() as *mut libc::sock_filter,
        };
        // SAFETY: prctl with documented arguments; `fprog` points at `prog`, alive for the call; `Insn` has the
        // layout of `struct sock_filter`.
        unsafe {
            if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                return Err(format!("no_new_privs: {}", std::io::Error::last_os_error()));
            }
            let r = libc::syscall(
                libc::SYS_seccomp,
                libc::SECCOMP_SET_MODE_FILTER,
                libc::SECCOMP_FILTER_FLAG_TSYNC,
                &fprog as *const libc::sock_fprog,
            );
            if r < 0 {
                return Err(format!("seccomp: {}", std::io::Error::last_os_error()));
            }
            if r > 0 {
                return Err(format!("seccomp: thread {r} could not be synchronised"));
            }
        }
        Ok(())
    }
}

/// The gap when `earlier` other threads were running when landlock was applied: they keep their file access.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn earlier_threads_note(earlier: usize) -> Option<Note> {
    (earlier > 0).then(|| {
        Note::new(
            Gap::Files,
            format!("{earlier} thread(s) started before the lockdown: landlock does not restrict their file access"),
        )
    })
}

/// The seccomp program (pure, so it is unit-tested on every platform): `socket(AF_INET | AF_INET6, ..)` fails
/// with `EACCES` when `deny_network`; `execve` and `execveat` fail with `EACCES` when `deny_exec`; a system call
/// of another architecture kills the process.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod bpf {
    /// One BPF instruction (the layout of `struct sock_filter`).
    #[repr(C)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Insn {
        pub code: u16,
        pub jt: u8,
        pub jf: u8,
        pub k: u32,
    }

    const LD_W_ABS: u16 = 0x20; // BPF_LD | BPF_W | BPF_ABS
    const JEQ_K: u16 = 0x15; // BPF_JMP | BPF_JEQ | BPF_K
    const RET_K: u16 = 0x06; // BPF_RET | BPF_K
    const RET_ALLOW: u32 = 0x7fff_0000;
    const RET_KILL: u32 = 0x8000_0000; // SECCOMP_RET_KILL_PROCESS
    const RET_ERRNO: u32 = 0x0005_0000;
    const EACCES: u32 = 13;
    // Offsets in `struct seccomp_data`.
    const NR: u32 = 0;
    const ARCH: u32 = 4;
    const ARG0: u32 = 16;
    const AF_INET: u32 = 2;
    const AF_INET6: u32 = 10;

    #[cfg(target_arch = "aarch64")]
    const ARCH_ID: u32 = 0xC000_00B7; // AUDIT_ARCH_AARCH64
    #[cfg(target_arch = "aarch64")]
    const SYS: (u32, u32, u32) = (198, 221, 281); // socket, execve, execveat
    #[cfg(not(target_arch = "aarch64"))]
    const ARCH_ID: u32 = 0xC000_003E; // AUDIT_ARCH_X86_64
    #[cfg(not(target_arch = "aarch64"))]
    const SYS: (u32, u32, u32) = (41, 59, 322);

    fn stmt(code: u16, k: u32) -> Insn {
        Insn { code, jt: 0, jf: 0, k }
    }

    fn jeq(k: u32, jt: u8, jf: u8) -> Insn {
        Insn { code: JEQ_K, jt, jf, k }
    }

    pub fn filter(deny_network: bool, deny_exec: bool) -> Vec<Insn> {
        let (socket, execve, execveat) = SYS;
        let deny = RET_ERRNO | EACCES;
        let mut p = vec![stmt(LD_W_ABS, ARCH), jeq(ARCH_ID, 1, 0), stmt(RET_K, RET_KILL), stmt(LD_W_ABS, NR)];
        if deny_exec {
            // nr == execve || nr == execveat -> deny
            p.push(jeq(execve, 1, 0));
            p.push(jeq(execveat, 0, 1));
            p.push(stmt(RET_K, deny));
        }
        if deny_network {
            // nr != socket -> allow; else arg0 == AF_INET || AF_INET6 -> deny, else allow
            p.push(jeq(socket, 0, 4));
            p.push(stmt(LD_W_ABS, ARG0));
            p.push(jeq(AF_INET, 1, 0));
            p.push(jeq(AF_INET6, 0, 1));
            p.push(stmt(RET_K, deny));
        }
        p.push(stmt(RET_K, RET_ALLOW));
        p
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// Runs the program on (arch, nr, arg0), as the kernel would.
        fn run(p: &[Insn], arch: u32, nr: u32, arg0: u32) -> u32 {
            let mut acc = 0u32;
            let mut pc = 0usize;
            loop {
                let i = p[pc];
                match i.code {
                    LD_W_ABS => {
                        acc = match i.k {
                            ARCH => arch,
                            NR => nr,
                            ARG0 => arg0,
                            k => panic!("offset {k}"),
                        };
                        pc += 1;
                    }
                    JEQ_K => pc += 1 + if acc == i.k { i.jt as usize } else { i.jf as usize },
                    RET_K => return i.k,
                    c => panic!("code {c}"),
                }
            }
        }

        #[test]
        fn the_filter_denies_exactly_what_it_should() {
            let (socket, execve, execveat) = SYS;
            let deny = RET_ERRNO | EACCES;
            let both = filter(true, true);
            assert_eq!(run(&both, ARCH_ID, socket, AF_INET), deny);
            assert_eq!(run(&both, ARCH_ID, socket, AF_INET6), deny);
            assert_eq!(run(&both, ARCH_ID, socket, 1), RET_ALLOW, "AF_UNIX stays allowed");
            assert_eq!(run(&both, ARCH_ID, execve, 0), deny);
            assert_eq!(run(&both, ARCH_ID, execveat, 0), deny);
            assert_eq!(run(&both, ARCH_ID, 0, AF_INET), RET_ALLOW, "another call with the same argument");
            assert_eq!(run(&both, ARCH_ID ^ 1, socket, 1), RET_KILL, "another architecture");
            let net = filter(true, false);
            assert_eq!(run(&net, ARCH_ID, execve, 0), RET_ALLOW);
            assert_eq!(run(&net, ARCH_ID, socket, AF_INET), deny);
            let exec = filter(false, true);
            assert_eq!(run(&exec, ARCH_ID, socket, AF_INET), RET_ALLOW);
            assert_eq!(run(&exec, ARCH_ID, execve, 0), deny);
            assert_eq!(std::mem::size_of::<Insn>(), 8);
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::SandboxSpec;
    use std::ffi::{c_char, c_int, CStr, CString};

    extern "C" {
        fn sandbox_init(profile: *const c_char, flags: u64, errorbuf: *mut *mut c_char) -> c_int;
        fn sandbox_free_error(errorbuf: *mut c_char);
    }

    pub fn lockdown(spec: &SandboxSpec, _elevated: bool, _notes: &mut Vec<super::Note>) -> Result<(), String> {
        set_limit(libc::RLIMIT_CPU, spec.cpu_seconds)?;
        // A contained child already runs under the profile sandbox-exec applied.
        let Some(profile) = super::macos_profile(spec).filter(|_| !spec.contained()) else {
            return Ok(());
        };
        let profile = CString::new(profile).map_err(|e| e.to_string())?;
        let mut err: *mut c_char = std::ptr::null_mut();
        // SAFETY: `profile` is a valid C string; `err` receives an allocated message, freed below.
        let rc = unsafe { sandbox_init(profile.as_ptr(), 0, &mut err) };
        if rc == 0 {
            return Ok(());
        }
        // SAFETY: on failure `err` is null or a C string from sandbox_init, freed once.
        let msg = unsafe {
            if err.is_null() {
                String::from("unknown error")
            } else {
                let m = CStr::from_ptr(err).to_string_lossy().into_owned();
                sandbox_free_error(err);
                m
            }
        };
        Err(format!("sandbox_init: {msg}"))
    }

    fn set_limit(resource: libc::c_int, value: Option<u64>) -> Result<(), String> {
        let Some(v) = value else { return Ok(()) };
        let lim = libc::rlimit {
            rlim_cur: v as libc::rlim_t,
            rlim_max: v as libc::rlim_t,
        };
        // SAFETY: a valid rlimit structure for this process.
        if unsafe { libc::setrlimit(resource, &lim) } != 0 {
            return Err(format!("setrlimit failed: {}", std::io::Error::last_os_error()));
        }
        Ok(())
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
mod platform {
    use super::SandboxSpec;

    pub fn lockdown(_spec: &SandboxSpec, _elevated: bool, notes: &mut Vec<super::Note>) -> Result<(), String> {
        notes.push(super::Note::new(super::Gap::Files, "no sandbox on this platform"));
        Ok(())
    }
}

/// The macOS sandbox profile (SBPL) for `spec`, or `None` when it restricts nothing. Pure, so it is tested on
/// every platform.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn macos_profile(spec: &SandboxSpec) -> Option<String> {
    let quote = |p: &std::path::Path| format!("\"{}\"", p.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\""));
    let mut rules: Vec<String> = Vec::new();
    if !spec.network {
        rules.push("(deny network*)".into());
    }
    if !spec.children {
        rules.push("(deny process-exec*)".into());
    }
    if spec.fs != FsAccess::Unrestricted {
        rules.push("(deny file-read* file-write*)".into());
        // What the dynamic loader and the C library read after start.
        rules.push(r#"(allow file-read* (subpath "/System") (subpath "/usr/lib") (literal "/dev/urandom"))"#.into());
        if let FsAccess::Paths { read, write } = &spec.fs {
            rules.extend(read.iter().map(|p| format!("(allow file-read* (subpath {}))", quote(p))));
            rules.extend(
                write
                    .iter()
                    .map(|p| format!("(allow file-read* file-write* (subpath {}))", quote(p))),
            );
        }
    }
    if rules.is_empty() {
        return None;
    }
    Some(format!("(version 1)\n(allow default)\n{}\n", rules.join("\n")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_and_json() {
        assert!(!SandboxSpec::unrestricted().restricts_child());
        let l = SandboxSpec::locked();
        assert!(l.restricts_child() && l.low_integrity && !l.network && l.fs == FsAccess::Nothing);
        let json = serde_json::to_string(&l).unwrap();
        assert_eq!(serde_json::from_str::<SandboxSpec>(&json).unwrap(), l);
        let p = SandboxSpec {
            fs: FsAccess::Paths {
                read: vec!["/a".into()],
                write: Vec::new(),
            },
            ..SandboxSpec::unrestricted()
        };
        assert_eq!(serde_json::from_str::<SandboxSpec>(&serde_json::to_string(&p).unwrap()).unwrap(), p);
        assert_eq!(serde_json::from_str::<SandboxSpec>("{}").unwrap(), SandboxSpec::default());
    }

    #[test]
    fn no_lockdown_caps_memory_with_an_rlimit() {
        // The host watches the memory a child uses instead (process.rs): rlimits count reservations, which broke
        // Linux (RLIMIT_AS, then RLIMIT_DATA with mimalloc) and macOS. Guard against their return in any lockdown.
        let src = include_str!("sandbox.rs");
        for banned in [["RLIMIT", "_AS"].concat(), ["RLIMIT", "_DATA"].concat()] {
            assert!(
                !src.lines().any(|l| l.contains(&banned) && !l.trim_start().starts_with("//")),
                "{banned} is used in sandbox.rs"
            );
        }
    }

    #[test]
    fn the_macos_profile_follows_the_spec() {
        assert_eq!(macos_profile(&SandboxSpec::unrestricted()), None);
        let locked = macos_profile(&SandboxSpec::locked()).unwrap();
        assert!(locked.starts_with("(version 1)"));
        for rule in ["(deny network*)", "(deny process-exec*)", "(deny file-read* file-write*)"] {
            assert!(locked.contains(rule), "{rule} in {locked}");
        }
        let paths = SandboxSpec {
            fs: FsAccess::Paths {
                read: vec![std::path::PathBuf::from("/models/a \"b\"")],
                write: vec!["/out".into()],
            },
            ..SandboxSpec::unrestricted()
        };
        let p = macos_profile(&paths).unwrap();
        assert!(p.contains(r#"(allow file-read* (subpath "/models/a \"b\""))"#), "{p}");
        assert!(p.contains(r#"(allow file-read* file-write* (subpath "/out"))"#), "{p}");
        assert!(!p.contains("network"));
    }

    #[test]
    fn contained_children_get_a_launcher_profile_with_their_folders_only() {
        assert_eq!(launcher_profile(&SandboxSpec::locked(), std::path::Path::new("/p/bin/x")), None);
        let spec = SandboxSpec {
            containment: Containment::Launcher {
                name: "imt.plugin.x".into(),
                read: vec!["/p".into()],
                write: vec!["/p/data".into()],
            },
            ..SandboxSpec::locked()
        };
        assert!(spec.contained() && !SandboxSpec::locked().contained());
        let p = launcher_profile(&spec, std::path::Path::new("/p/bin/x")).unwrap();
        for rule in [
            "(deny network*)",
            "(deny process-exec*)",
            "(deny file-read* file-write*)",
            r#"(allow file-read* (subpath "/p"))"#,
            r#"(allow file-read* (subpath "/p/bin"))"#,
            r#"(allow file-read* file-write* (subpath "/p/data"))"#,
            r#"(allow process-exec (literal "/p/bin/x"))"#,
        ] {
            assert!(p.contains(rule), "{rule} in {p}");
        }
        let json = serde_json::to_string(&spec).unwrap();
        assert_eq!(serde_json::from_str::<SandboxSpec>(&json).unwrap(), spec);
        assert!(Containment::valid_name("imt.plugin.a-b") && !Containment::valid_name("a b") && !Containment::valid_name(""));
        assert!(!Containment::valid_name(&"x".repeat(65)));
    }

    #[test]
    fn an_unrestricted_lockdown_changes_nothing() {
        assert!(lockdown(&SandboxSpec::unrestricted(), false).unwrap().is_empty());
    }

    #[test]
    fn threads_started_before_the_lockdown_are_a_gap() {
        assert_eq!(earlier_threads_note(0), None);
        let note = earlier_threads_note(2).unwrap();
        assert_eq!(note.gap, Gap::Files);
        assert!(
            note.message.starts_with("2 thread(s) started before the lockdown"),
            "{}",
            note.message
        );
    }

    #[test]
    fn a_kernel_without_landlock_is_one_status_line_for_children_whose_files_are_restricted() {
        assert_eq!(files_note(true, true), None);
        assert_eq!(files_note(false, false), None, "nothing to confine");
        assert_eq!(files_note(true, false), Some(Note::new(Gap::Files, FILES_NOT_CONFINED)));
        assert!(SandboxSpec::locked().restricts_files() && !SandboxSpec::unrestricted().restricts_files());
        let contained = SandboxSpec {
            containment: Containment::Launcher {
                name: "imt.plugin.x".into(),
                read: Vec::new(),
                write: Vec::new(),
            },
            ..SandboxSpec::unrestricted()
        };
        assert!(contained.restricts_files());
        assert!(host_notes(&SandboxSpec::unrestricted()).is_empty());
        assert_eq!(host_notes(&contained).is_empty(), files_confinable());
        // What a contained child reports from between fork and exec.
        assert!(launcher_notes(Some(LANDLOCK_FULL)).is_empty());
        assert!(launcher_notes(Some(LANDLOCK_PARTIAL))[0].message.contains("partly"));
        assert_eq!(launcher_notes(Some(LANDLOCK_NONE))[0].message, FILES_NOT_CONFINED);
        assert_eq!(launcher_notes(None)[0].message, FILES_NOT_CONFINED, "no report: not confined");
    }

    #[test]
    fn gaps_are_logged_as_warnings_and_information_is_not() {
        let gap = Note::new(Gap::Files, "landlock is not available");
        assert_eq!(gap.log_line(), "warning: sandbox degraded: landlock is not available");
        assert!(Note::new(Gap::Info, "x").log_line().starts_with("sandbox: "));
        // The child log reads "warning" as a Warning line.
        assert!(gap.log_line().to_ascii_lowercase().contains("warn"));
    }
}
