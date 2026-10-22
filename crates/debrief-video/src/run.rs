//! Running `ffmpeg` and `ffprobe` (ADR 0033 §2): a separate process through Debrief's one launcher (`debrief_ipc::process`),
//! contained before its first instruction (`Containment::Launcher`): no network, reading only the runtime's folders
//! and the chosen video, writing only a fresh work folder, no child processes, below-normal priority, an environment
//! of a few variables. The host's memory watcher stops it over its cap; a wall-clock limit and Cancel kill it.
//! Arguments are a list (never a shell); output comes from files in the work folder (FFmpeg's report through
//! `FFREPORT`, `ffprobe -o`, the `metadata` filter's file, `-progress`), never from a pipe that could fill up.

use crate::error::{Result, VideoError};
use crate::parse;
use debrief_ipc::memory_watch::{MemoryEvent, MemoryListener};
use debrief_ipc::process::{ChildProcess, SpawnSpec};
use debrief_ipc::sandbox::{Containment, FsAccess, SandboxSpec};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The name of the AppContainer FFmpeg runs in (Windows).
pub const CONTAINER: &str = "video-ffmpeg";
/// FFmpeg's report in the work folder.
pub const REPORT_FILE: &str = "ffreport.log";
/// `-progress` output in the work folder.
pub const PROGRESS_FILE: &str = "progress.txt";
/// The variables of Debrief's environment an FFmpeg process inherits (the OS needs them; nothing else).
const KEEP_ENV: &[&str] = &["SystemRoot", "SYSTEMROOT", "windir", "LANG", "LC_ALL", "LLVM_PROFILE_FILE"];
/// Largest report read back.
const MAX_REPORT_BYTES: u64 = 32 << 20;
/// How often a running process is looked at.
const POLL: Duration = Duration::from_millis(50);

/// An FFmpeg installation: the two programs and the folders they need to read (their libraries).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tools {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    /// Folders the processes may read (the runtime folder; for a user-provided FFmpeg its installation prefix).
    pub read: Vec<PathBuf>,
    /// Where the shared libraries are, when the loader must be told (Linux builds).
    pub lib_dir: Option<PathBuf>,
}

impl Tools {
    /// `ffmpeg` and `ffprobe` in `dir` (with their libraries there too).
    pub fn in_dir(dir: &Path) -> Tools {
        Tools {
            ffmpeg: dir.join(exe("ffmpeg")),
            ffprobe: dir.join(exe("ffprobe")),
            read: vec![dir.to_path_buf()],
            lib_dir: cfg!(target_os = "linux").then(|| dir.to_path_buf()),
        }
    }

    /// Both programs exist.
    pub fn present(&self) -> Result<()> {
        for p in [&self.ffmpeg, &self.ffprobe] {
            if !p.is_file() {
                return Err(VideoError::NoRuntime(format!(
                    "{} is missing: install the FFmpeg runtime again in Settings ▸ Video",
                    p.display()
                )));
            }
        }
        Ok(())
    }
}

/// `name` with the platform's executable extension.
pub fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// The caps of one run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    pub memory_bytes: u64,
    pub timeout: Duration,
}

impl Default for Caps {
    fn default() -> Self {
        Caps {
            memory_bytes: 1 << 30,
            timeout: Duration::from_secs(30),
        }
    }
}

/// The wall-clock limit of a pass over a video of `duration_ms`: twice its length plus a minute, 1–30 minutes.
pub fn pass_timeout(duration_ms: u64) -> Duration {
    Duration::from_millis(duration_ms.saturating_mul(2).saturating_add(60_000).clamp(60_000, 30 * 60_000))
}

/// One process to run.
pub struct Run<'a> {
    pub exe: &'a Path,
    pub args: Vec<String>,
    /// A fresh folder the process may write (and where its outputs are read from).
    pub work: &'a Path,
    /// The video it may read.
    pub input: Option<&'a Path>,
    pub caps: Caps,
    pub cancel: &'a AtomicBool,
    /// "ffprobe", "the frame analysis": names the process in messages.
    pub label: &'a str,
    /// Called about every 50 ms while it runs (progress).
    pub on_tick: Option<&'a mut dyn FnMut()>,
}

/// How a run ended (only successful runs return; every failure is an error).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ran {
    /// FFmpeg's report (its log at info level).
    pub report: String,
    /// The process could not be contained (its folder's permissions cannot be extended) and ran with the job limits
    /// only.
    pub uncontained: bool,
}

/// `p` with symbolic links resolved: sandbox profiles match real paths (macOS `/var` is a link to `/private/var`).
fn real(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

fn spec(tools: &Tools, run: &Run, contained: bool, stopped: Arc<AtomicBool>) -> SpawnSpec {
    let mut s = SpawnSpec::new(real(run.exe), format!("FFmpeg ({})", run.label));
    s.args = run.args.clone();
    s.keep_env = KEEP_ENV;
    s.env = vec![
        ("FFREPORT".into(), format!("file={REPORT_FILE}:level=32")),
        ("AV_LOG_FORCE_NOCOLOR".into(), "1".into()),
    ];
    if let Some(lib) = &tools.lib_dir {
        s.env.push(("LD_LIBRARY_PATH".into(), lib.to_string_lossy().into_owned()));
    }
    s.current_dir = Some(real(run.work));
    s.below_normal_priority = true;
    let mut read: Vec<PathBuf> = tools.read.iter().map(|p| real(p)).collect();
    if let Some(i) = run.input {
        read.push(real(i));
    }
    let write = vec![real(run.work)];
    s.sandbox = SandboxSpec {
        memory_limit: Some(run.caps.memory_bytes),
        cpu_rate_percent: None,
        cpu_seconds: None,
        children: false,
        low_integrity: false,
        network: false,
        fs: FsAccess::Paths {
            read: read.clone(),
            write: write.clone(),
        },
        containment: if contained {
            Containment::Launcher {
                name: CONTAINER.into(),
                read,
                write,
            }
        } else {
            Containment::SelfApplied
        },
    };
    s.memory_listener = Some(MemoryListener::new(move |_, e| {
        if matches!(e, MemoryEvent::Stopped(_)) {
            stopped.store(true, Ordering::SeqCst);
        }
    }));
    s
}

fn read_capped(path: &Path) -> String {
    use std::io::Read;
    let Ok(f) = std::fs::File::open(path) else { return String::new() };
    let mut buf = Vec::new();
    let _ = f.take(MAX_REPORT_BYTES).read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).into_owned()
}

/// Reads a text output of a run (bounded; empty when it is missing).
pub fn output(work: &Path, name: &str) -> String {
    read_capped(&work.join(name))
}

/// Runs one process to its end (see the module docs). A non-zero exit is [`VideoError::Tool`] with FFmpeg's last error
/// lines; over the memory cap [`VideoError::OverMemory`]; past the timeout [`VideoError::Timeout`]; cancelled
/// [`VideoError::Cancelled`].
pub fn run(tools: &Tools, mut run: Run) -> Result<Ran> {
    if run.cancel.load(Ordering::SeqCst) {
        return Err(VideoError::Cancelled);
    }
    if !run.exe.is_file() {
        return Err(VideoError::NoRuntime(format!(
            "{} is missing: install the FFmpeg runtime again in Settings ▸ Video",
            run.exe.display()
        )));
    }
    std::fs::create_dir_all(run.work)?;
    let stopped = Arc::new(AtomicBool::new(false));
    let (child, uncontained) = match ChildProcess::spawn(&spec(tools, &run, true, stopped.clone())) {
        Ok(c) => (c, false),
        // The AppContainer needs the video's permissions extended; where that is refused (a share, a file the user
        // does not own), the job object's limits still apply.
        Err(e) if cfg!(windows) => {
            log::warn!("FFmpeg could not be contained ({e}); it runs with its job limits only");
            let c = ChildProcess::spawn(&spec(tools, &run, false, stopped.clone()))
                .map_err(|e| VideoError::Tool(format!("{} could not start: {e}", run.label)))?;
            (c, true)
        }
        Err(e) => return Err(VideoError::Tool(format!("{} could not start: {e}", run.label))),
    };
    let deadline = Instant::now() + run.caps.timeout;
    let status = loop {
        if let Some(s) = child.try_exit() {
            break s;
        }
        if stopped.load(Ordering::SeqCst) {
            child.kill();
            return Err(VideoError::OverMemory(format!(
                "{} used more than its memory cap of {} MB and was stopped: the video may be too large for this PC",
                run.label,
                run.caps.memory_bytes >> 20
            )));
        }
        if run.cancel.load(Ordering::SeqCst) {
            child.kill();
            return Err(VideoError::Cancelled);
        }
        if Instant::now() >= deadline {
            child.kill();
            return Err(VideoError::Timeout(format!(
                "{} took longer than {} s and was stopped",
                run.label,
                run.caps.timeout.as_secs()
            )));
        }
        if let Some(t) = run.on_tick.as_mut() {
            t();
        }
        std::thread::sleep(POLL);
    };
    // The watcher may see the cap a moment before the kill lands.
    if stopped.load(Ordering::SeqCst) {
        return Err(VideoError::OverMemory(format!(
            "{} used more than its memory cap of {} MB and was stopped",
            run.label,
            run.caps.memory_bytes >> 20
        )));
    }
    let report = read_capped(&run.work.join(REPORT_FILE));
    if !status.success() {
        let lines = parse::report_errors(&report, 3);
        let why = if lines.is_empty() {
            format!("it exited with {status}")
        } else {
            lines.join(" · ")
        };
        return Err(VideoError::Tool(format!("{} failed: {why}", run.label)));
    }
    Ok(Ran { report, uncontained })
}

/// The progress of a running pass from its `-progress` file: the share of `duration_ms` done (0–1).
pub fn progress_of(work: &Path, duration_ms: u64) -> Option<f64> {
    let text = std::fs::read_to_string(work.join(PROGRESS_FILE)).ok()?;
    let us: u64 = text
        .lines()
        .rev()
        .find_map(|l| l.strip_prefix("out_time_us=").and_then(|v| v.trim().parse().ok()))?;
    (duration_ms > 0).then(|| (us as f64 / 1000.0 / duration_ms as f64).clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeouts_scale_with_the_video() {
        assert_eq!(pass_timeout(0), Duration::from_secs(60));
        assert_eq!(pass_timeout(120_000), Duration::from_secs(300));
        assert_eq!(pass_timeout(u64::MAX), Duration::from_secs(1800));
    }

    #[test]
    fn progress_is_read_from_the_last_line() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(progress_of(dir.path(), 10_000), None);
        std::fs::write(
            dir.path().join(PROGRESS_FILE),
            "out_time_us=1000000\nprogress=continue\nout_time_us=2500000\n",
        )
        .unwrap();
        assert_eq!(progress_of(dir.path(), 10_000), Some(0.25));
        assert_eq!(progress_of(dir.path(), 0), None);
        std::fs::write(dir.path().join(PROGRESS_FILE), "out_time_us=99000000\n").unwrap();
        assert_eq!(progress_of(dir.path(), 10_000), Some(1.0));
    }

    #[test]
    fn a_missing_program_is_a_clear_message() {
        let dir = tempfile::tempdir().unwrap();
        let tools = Tools::in_dir(&dir.path().join("none"));
        let e = tools.present().unwrap_err();
        assert!(matches!(e, VideoError::NoRuntime(_)) && e.to_string().contains("install the FFmpeg runtime again"));
        let cancel = AtomicBool::new(false);
        let e = run(
            &tools,
            Run {
                exe: &tools.ffmpeg,
                args: vec![],
                work: dir.path(),
                input: None,
                caps: Caps::default(),
                cancel: &cancel,
                label: "ffmpeg",
                on_tick: None,
            },
        )
        .unwrap_err();
        assert!(matches!(e, VideoError::NoRuntime(_)));
        cancel.store(true, Ordering::SeqCst);
        let e = run(
            &tools,
            Run {
                exe: &tools.ffmpeg,
                args: vec![],
                work: dir.path(),
                input: None,
                caps: Caps::default(),
                cancel: &cancel,
                label: "ffmpeg",
                on_tick: None,
            },
        )
        .unwrap_err();
        assert_eq!(e, VideoError::Cancelled);
        assert_eq!(output(dir.path(), "nothing"), "");
    }
}
