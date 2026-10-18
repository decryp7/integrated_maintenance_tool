//! Shared by IMT's .NET Framework profiler: the agent DLL that attaches to a
//! running CLR (`imt-clr-agent`), the host that drives it (`imt-clr`) and the
//! importer that opens its captures (`imt-plugin-profiling`).
//!
//! - [`record`]: capture files (`.imtclr` profiles, `.imtclrheap` heap
//!   snapshots) as streams of self-delimiting records.
//! - [`protocol`]: the attach configuration and the pipe messages.
//! - [`wire`]: the primitive encoding both use.
//!
//! Decoding never panics on any input and bounds every length.

pub mod protocol;
pub mod record;
pub mod wire;

pub use protocol::{AttachConfig, Channel, Command, Event};
pub use record::{FrameKind, HeapRecord, ProfileRecord, Records, RootKind};

/// Longest string in any record (bytes).
pub const MAX_STRING: usize = 4096;
/// Frames kept per stack.
pub const MAX_STACK: usize = 2048;
/// References kept per object (larger arrays are cut).
pub const MAX_REFS: usize = 4 << 20;
/// Largest record payload.
pub const MAX_RECORD: usize = 64 << 20;
/// Largest data-pipe frame.
pub const MAX_FRAME: usize = MAX_RECORD + 16;

/// Why bytes could not be decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatError {
    /// The input is not this format (wrong magic).
    NotThisFormat,
    /// Written by a newer (or unknown) version.
    UnsupportedVersion(u16),
    /// The input ended in the middle of a value.
    Truncated,
    /// A value is out of range or inconsistent.
    Invalid(&'static str),
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FormatError::NotThisFormat => f.write_str("not an IMT .NET capture"),
            FormatError::UnsupportedVersion(v) => write!(f, "format version {v} is not supported; update IMT"),
            FormatError::Truncated => f.write_str("the data ends in the middle of a record"),
            FormatError::Invalid(why) => write!(f, "damaged data: {why}"),
        }
    }
}

impl std::error::Error for FormatError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_say_what_is_wrong() {
        assert_eq!(FormatError::NotThisFormat.to_string(), "not an IMT .NET capture");
        assert!(FormatError::UnsupportedVersion(3).to_string().contains("update IMT"));
        assert!(FormatError::Truncated.to_string().contains("ends"));
        assert!(FormatError::Invalid("x").to_string().ends_with(": x"));
    }
}
