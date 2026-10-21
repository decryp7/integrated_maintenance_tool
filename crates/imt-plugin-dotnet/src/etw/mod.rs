//! ETW: recording a .NET process with a real-time session ([`session`],
//! Windows) and reading `.etl` files ([`etl`], Windows), both converted to
//! NetTrace by the pure, cross-platform [`convert::Converter`].
//!
//! The session and the file reader only copy each `EVENT_RECORD` into a
//! [`RawEvent`] (provider, id, version, opcode, process, thread, timestamp,
//! the header's pointer size, payload and extended-data stack); everything
//! else is pure and tested on every platform with recorded payloads.

pub mod convert;
pub mod kernel;
pub mod scan;

#[cfg(windows)]
pub mod etl;
#[cfg(windows)]
pub mod session;
#[cfg(windows)]
mod sys;

/// One ETW event, as the consumer callback receives it (borrowed for the callback's duration).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawEvent<'a> {
    /// Provider GUID (for classic kernel events: the event class GUID).
    pub provider: u128,
    pub id: u16,
    pub version: u8,
    pub opcode: u8,
    pub process_id: u32,
    pub thread_id: u32,
    /// Raw timestamp (the session's clock: QPC ticks for IMT's sessions).
    pub timestamp: i64,
    /// Pointer size of the process that logged it (4 or 8), from the header flags.
    pub pointer_size: u8,
    pub payload: &'a [u8],
    /// The stack the session attached (extended data), leaf first; empty when none.
    pub stack: &'a [u64],
}

impl<'a> RawEvent<'a> {
    /// The pointer size as a byte count the decoders take.
    pub fn ptr(&self) -> usize {
        if self.pointer_size == 4 {
            4
        } else {
            8
        }
    }
}

/// Errors of the ETW collector and reader.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EtwError {
    /// `ERROR_ACCESS_DENIED`: ETW sessions need an administrator or a member of Performance Log Users.
    #[error("recording with ETW needs administrator rights (or membership of Performance Log Users); restart IMT as administrator")]
    Denied,
    /// ETW is not available here.
    #[error("{0}")]
    Unsupported(String),
    /// A Win32 call failed.
    #[error("{call} failed: {message} (error {code})")]
    Win32 { call: &'static str, code: u32, message: String },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// `ERROR_ACCESS_DENIED`.
pub const ERROR_ACCESS_DENIED: u32 = 5;

/// Maps a Win32 error code of `call` to an [`EtwError`] (access denied is its own case).
pub fn win32_error(call: &'static str, code: u32) -> EtwError {
    if code == ERROR_ACCESS_DENIED {
        return EtwError::Denied;
    }
    let message = match code {
        2 => "not found".to_string(),
        87 => "invalid parameter".to_string(),
        183 => "a session with this name already exists".to_string(),
        1450 => "insufficient system resources".to_string(),
        4201 => "the instance name was not found (the session is not running)".to_string(),
        _ => std::io::Error::from_raw_os_error(code as i32).to_string(),
    };
    EtwError::Win32 { call, code, message }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_denied_is_its_own_error_with_the_next_step() {
        let e = win32_error("StartTraceW", ERROR_ACCESS_DENIED);
        assert!(matches!(e, EtwError::Denied));
        assert!(e.to_string().contains("administrator"));
        for code in [2, 87, 183, 1450, 4201, 31] {
            let e = win32_error("ControlTraceW", code);
            assert!(matches!(e, EtwError::Win32 { code: c, .. } if c == code));
            assert!(e.to_string().contains("ControlTraceW"));
        }
        let raw = RawEvent {
            provider: 0,
            id: 0,
            version: 0,
            opcode: 0,
            process_id: 0,
            thread_id: 0,
            timestamp: 0,
            pointer_size: 4,
            payload: &[],
            stack: &[],
        };
        assert_eq!(raw.ptr(), 4);
        assert_eq!(RawEvent { pointer_size: 8, ..raw }.ptr(), 8);
    }
}
