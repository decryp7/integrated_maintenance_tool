//! Debrief plugin `net.decryptology.dotnet`: event-based .NET profiling (ADR 0019).
//!
//! Nothing is loaded into the target process. Captures come only from what the
//! runtime and the OS publish:
//!
//! - **ETW** ([`etw`], Windows): .NET Framework 4.6.2+ (and .NET Core)
//!   processes, x86 or x64, recorded by a real-time session with kernel CPU
//!   samples, context switches and the CLR providers, on `debrief-etw`'s sessions
//!   and converter (shared with the native profiler, ADR 0022). Needs
//!   administrator rights.
//! - **EventPipe** ([`eventpipe`], every platform): .NET Core 3+/5+ processes
//!   through the diagnostics IPC. Needs no elevation.
//!
//! Both write NetTrace files ([`nettrace`]) that the nettrace importer of
//! `debrief-plugin-profiling` opens. The [`Collector`] / [`Recording`] interface
//! hides which backend records a process.
//!
//! With feature `plugin` (default) the crate is also a plugin on `debrief-plugin-api` (ADR 0017): the
//! `dotnet-profiler` tool, served in its own process by `debrief-plugins`; ETW recordings run in the plugin's
//! elevated process. `.etl` traces open through the native profiling plugin.

pub mod clr;
pub mod etw;
pub mod eventpipe;
#[doc(hidden)]
pub mod fuzz;
/// The streaming NetTrace writer (`debrief-etw`'s).
pub use debrief_etw::nettrace;
pub mod process;
pub mod recording;

#[cfg(feature = "plugin")]
mod plugin;
#[cfg(feature = "plugin")]
pub use plugin::{DotnetPlugin, TASK_KIND, TOOL_ID};

pub use clr::Preset;
pub use process::{ProcessInfo, Runtime};

use std::path::{Path, PathBuf};
use std::time::Duration;

/// The plugin id.
pub const PLUGIN_ID: &str = "net.decryptology.dotnet";

/// Bounds of one recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The file is closed (and marked truncated) at this size.
    pub max_bytes: u64,
    /// The recording stops by itself after this long.
    pub max_duration: Duration,
    /// Connecting to the target (EventPipe) or starting the session (ETW).
    pub connect_timeout: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_bytes: 1 << 30,
            max_duration: Duration::from_secs(30 * 60),
            connect_timeout: Duration::from_secs(5),
        }
    }
}

/// Longest heap snapshot recording (the heap walk normally ends within seconds).
pub const HEAP_SNAPSHOT_CAP: Duration = Duration::from_secs(120);

/// A recording that ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finished {
    pub path: PathBuf,
    pub bytes: u64,
    /// It stopped at [`Limits::max_bytes`].
    pub truncated: bool,
    /// What the user should know (target exited, rundown incomplete, ...).
    pub notes: Vec<String>,
}

/// Why a capture could not start or finish.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DotnetError {
    #[error("process {pid} is not running (or is not a .NET process)")]
    NotFound { pid: u32 },
    #[error("{why}")]
    Unsupported { why: String },
    /// Missing rights: the administrator prompt for the plugin's elevated process was declined (or there is none).
    #[error("recording this process needs administrator rights (ETW); allow Debrief's administrator prompt when you start recording")]
    Denied,
    #[error("the target did not answer in time")]
    Timeout,
    #[error("the recording failed: {0}")]
    Protocol(String),
    #[error("the recording was cancelled")]
    Cancelled,
    #[error("a limit was reached: {0}")]
    Limit(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl From<etw::EtwError> for DotnetError {
    fn from(e: etw::EtwError) -> Self {
        match e {
            etw::EtwError::Denied => DotnetError::Denied,
            etw::EtwError::Unsupported(why) => DotnetError::Unsupported { why },
            etw::EtwError::Io(e) => DotnetError::Io(e),
            other => DotnetError::Protocol(other.to_string()),
        }
    }
}

/// A recording in progress (on its own thread).
pub trait Recording: Send {
    /// Bytes written so far.
    fn bytes(&self) -> u64;
    fn elapsed(&self) -> Duration;
    /// It ended by itself (duration, size cap, heap walk done, target exited, failure).
    fn is_finished(&self) -> bool;
    /// Stops (collecting method names first) and waits for the file to be complete.
    fn stop(self: Box<Self>) -> Result<Finished, DotnetError>;
    /// Stops at once and deletes the file.
    fn cancel(self: Box<Self>);
}

/// Finds .NET processes and records them.
pub trait Collector: Send + Sync {
    fn list_processes(&self) -> Vec<ProcessInfo>;
    fn start(&self, pid: u32, preset: Preset, out: &Path, limits: Limits) -> Result<Box<dyn Recording>, DotnetError>;
}

/// The real collector: ETW for .NET Framework, EventPipe for .NET Core.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemCollector;

impl Collector for SystemCollector {
    fn list_processes(&self) -> Vec<ProcessInfo> {
        process::list()
    }

    fn start(&self, pid: u32, preset: Preset, out: &Path, limits: Limits) -> Result<Box<dyn Recording>, DotnetError> {
        let info = process::info(pid).ok_or(DotnetError::NotFound { pid })?;
        match info.runtime {
            Runtime::Core => Ok(Box::new(eventpipe::record(pid, preset, out, limits)?)),
            Runtime::Framework => start_etw(&info, preset, out, limits),
        }
    }
}

#[cfg(windows)]
fn start_etw(info: &ProcessInfo, preset: Preset, out: &Path, limits: Limits) -> Result<Box<dyn Recording>, DotnetError> {
    let sink = recording::FileSink::create(out)?;
    Ok(Box::new(etw::session::record(info, preset, Box::new(sink), out, limits)?))
}

#[cfg(not(windows))]
fn start_etw(_: &ProcessInfo, _: Preset, _: &Path, _: Limits) -> Result<Box<dyn Recording>, DotnetError> {
    Err(DotnetError::Unsupported {
        why: ".NET Framework processes can only be recorded on Windows (ETW)".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_say_what_to_do() {
        assert!(DotnetError::Denied.to_string().contains("administrator"));
        assert!(DotnetError::from(etw::EtwError::Denied).to_string().contains("administrator"));
        assert!(matches!(
            DotnetError::from(etw::EtwError::Unsupported("x".into())),
            DotnetError::Unsupported { .. }
        ));
        assert!(matches!(
            DotnetError::from(etw::win32_error("StartTraceW", 1450)),
            DotnetError::Protocol(m) if m.contains("StartTraceW")
        ));
        assert!(matches!(
            DotnetError::from(etw::EtwError::Io(std::io::Error::other("x"))),
            DotnetError::Io(_)
        ));
        assert!(DotnetError::NotFound { pid: 3 }.to_string().contains('3'));
        assert_eq!(Limits::default().max_bytes, 1 << 30);
        assert!(SystemCollector
            .start(u32::MAX - 7, Preset::Cpu, Path::new("x"), Limits::default())
            .is_err());
    }
}
