//! ETW for the .NET profiler. The decoders, the ETW -> NetTrace converter, the `.etl` reader and the session
//! plumbing are `debrief-etw`'s, shared with the native profiler (ADR 0022 §6); this module adds only the .NET
//! recording control ([`session`], Windows): presets, the CLR providers, rundown and heap-walk milestones.

pub use debrief_etw::{convert, kernel, scan, win32_error, EtwError, RawEvent, ERROR_ACCESS_DENIED};

#[cfg(windows)]
pub use debrief_etw::etl;
#[cfg(windows)]
pub mod session;
