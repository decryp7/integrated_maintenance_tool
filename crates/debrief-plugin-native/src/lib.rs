//! Debrief plugin `net.decryptology.native`: native (C/C++) profiling (ADR 0022).
//!
//! Nothing is loaded into the target. Two capture modes:
//!
//! - **Suspend-and-sample** ([`sampling`], no administrator rights): a sampler thread suspends one target thread
//!   at a time, copies its registers and stack, resumes it, and walks the copy with DbgHelp; threads whose CPU
//!   time did not advance count as waiting.
//! - **ETW** ([`etw`], administrator, the plugin's elevated process): kernel CPU samples, context switches and
//!   image loads with stack walks, optionally the heap provider and the CLR providers (mixed stacks).
//!
//! Both end in a NetTrace file (via `debrief-etw`'s writer) with `NativeSymbol` events from `debrief-symbols`, so the
//! profiling views and findings open it. `.etl` traces (WPR/xperf) convert through the same code ([`importer`]).
//!
//! | Tool | Process | Operations |
//! |---|---|---|
//! | [`TOOL_ID`] | normal | `defaults`, `processes`, `start`, `status` (pollable), `stop`, `etl-processes`, `etl-convert` |
//! | [`ETW_TOOL_ID`] | elevated | `start`, `status` (pollable), `stop` |

pub mod captures;
pub mod etw;
pub mod importer;
pub mod paths;
pub mod plugin;
pub mod process;
pub mod request;
pub mod sampling;
pub mod settings;
pub mod symbolize;
pub mod tool;

pub use plugin::NativePlugin;

use std::path::PathBuf;
use std::time::Duration;

/// The plugin id.
pub const PLUGIN_ID: &str = "net.decryptology.native";
/// The tool of the normal process (sampling, listing, `.etl`).
pub const TOOL_ID: &str = "native-profiler";
/// The tool of the elevated process (ETW).
pub const ETW_TOOL_ID: &str = "native-profiler.etw";
/// Task kind hint: the shell lists captures as Jobs of kind Capture.
pub const TASK_KIND: &str = "capture";

/// Bounds of one capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The file is closed (and marked truncated) at this size.
    pub max_bytes: u64,
    /// The capture stops by itself after this long.
    pub max_duration: Duration,
    /// Most samples (sampling mode) kept in memory.
    pub max_samples: usize,
    /// Most distinct code addresses symbolised.
    pub max_addresses: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_bytes: 1 << 30,
            max_duration: Duration::from_secs(60 * 60),
            max_samples: 4_000_000,
            max_addresses: 200_000,
        }
    }
}

/// A capture that ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finished {
    pub path: PathBuf,
    pub bytes: u64,
    /// It stopped at a limit (size or samples).
    pub truncated: bool,
    /// What the user should know (target exited, symbols missing, ...).
    pub notes: Vec<String>,
}

/// Why a capture could not start or finish.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum NativeError {
    #[error("process {pid} is not running")]
    NotFound { pid: u32 },
    #[error("{0}")]
    Unsupported(String),
    /// Missing rights: the UI offers "Restart as administrator" (or the ETW mode).
    #[error("{0}")]
    Denied(String),
    #[error("the capture failed: {0}")]
    Failed(String),
    #[error("the capture was cancelled")]
    Cancelled,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl From<debrief_etw::EtwError> for NativeError {
    fn from(e: debrief_etw::EtwError) -> Self {
        match e {
            debrief_etw::EtwError::Denied => NativeError::Denied(
                "recording with ETW needs administrator rights; allow the administrator prompt, or use the sampling mode".into(),
            ),
            debrief_etw::EtwError::Unsupported(why) => NativeError::Unsupported(why),
            debrief_etw::EtwError::Io(e) => NativeError::Io(e),
            other => NativeError::Failed(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_say_what_to_do() {
        let d = NativeError::from(debrief_etw::EtwError::Denied);
        assert!(matches!(d, NativeError::Denied(ref m) if m.contains("administrator")));
        assert!(matches!(
            NativeError::from(debrief_etw::EtwError::Unsupported("x".into())),
            NativeError::Unsupported(_)
        ));
        assert!(matches!(
            NativeError::from(debrief_etw::win32_error("StartTraceW", 1450)),
            NativeError::Failed(m) if m.contains("StartTraceW")
        ));
        assert!(matches!(
            NativeError::from(debrief_etw::EtwError::Io(std::io::Error::other("x"))),
            NativeError::Io(_)
        ));
        assert!(NativeError::NotFound { pid: 3 }.to_string().contains('3'));
        assert_eq!(Limits::default().max_bytes, 1 << 30);
    }
}
