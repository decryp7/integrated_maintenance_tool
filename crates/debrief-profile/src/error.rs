//! Typed errors of the profiling engine. Every message tells the user what
//! went wrong and, where there is one, what to do about it.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// What kind of failure an error is, for callers (the UI) that react to it
/// without reading the message. Serialised camelCase (`notACapture`, ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCode {
    /// The file is not a profile or heap snapshot any importer recognises.
    NotACapture,
    /// The file could not be read.
    Io,
    /// A bounded resource ran out (size, count or time).
    Limit,
    /// A recognised format or view feature that is not supported (or data the capture lacks).
    Unsupported,
    /// The input (file content or request) is invalid.
    Invalid,
    /// The capture or file does not exist (any more).
    NotFound,
}

/// Errors from importing, building and querying captures.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ProfileError {
    /// A bounded resource ran out; the capture is too large for one view.
    #[error("{what} limit exceeded ({limit}); {hint}")]
    LimitExceeded {
        what: &'static str,
        limit: u64,
        hint: &'static str,
    },

    /// The file is not in the format the importer expected, or is damaged.
    #[error("{format}: {message}")]
    Malformed { format: &'static str, message: String },

    /// The file is in a recognised format but uses a feature that is not supported.
    #[error("{format}: {message}")]
    Unsupported { format: &'static str, message: String },

    /// No importer recognised the file.
    #[error("{0} is not a recognised profile or heap snapshot (supported: {1})")]
    Unrecognised(String, String),

    /// An importer contributed by a plugin failed (crashed, quarantined, timed out).
    #[error("importer \"{importer}\" failed: {message}")]
    Importer { importer: String, message: String },

    /// No capture with this id is open.
    #[error("no capture {0} is open (it may have been closed)")]
    UnknownCapture(u32),

    /// The capture exists but is of the other kind (e.g. a heap snapshot passed to a CPU view).
    #[error("capture {id} is a {actual}, not a {expected}")]
    WrongCapture {
        id: u32,
        expected: &'static str,
        actual: &'static str,
    },

    /// A request argument is invalid (bad pattern, unknown path, empty range, ...).
    #[error("{0}")]
    InvalidRequest(String),

    /// The view needs data the capture does not have (e.g. timestamps).
    #[error("{0}")]
    NotAvailable(String),

    /// An error of an importer in the profiling plugin's process, with its code (ADR 0017).
    #[error("{message}")]
    Remote { code: ErrorCode, message: String },
}

impl ProfileError {
    pub fn malformed(format: &'static str, message: impl Into<String>) -> ProfileError {
        ProfileError::Malformed {
            format,
            message: message.into(),
        }
    }

    pub fn unsupported(format: &'static str, message: impl Into<String>) -> ProfileError {
        ProfileError::Unsupported {
            format,
            message: message.into(),
        }
    }

    pub fn invalid(message: impl Into<String>) -> ProfileError {
        ProfileError::InvalidRequest(message.into())
    }

    /// The error's [`ErrorCode`].
    pub fn code(&self) -> ErrorCode {
        match self {
            ProfileError::LimitExceeded { .. } => ErrorCode::Limit,
            ProfileError::Unsupported { .. } | ProfileError::NotAvailable(_) => ErrorCode::Unsupported,
            ProfileError::Unrecognised(..) => ErrorCode::NotACapture,
            ProfileError::UnknownCapture(_) => ErrorCode::NotFound,
            ProfileError::Malformed { .. }
            | ProfileError::Importer { .. }
            | ProfileError::WrongCapture { .. }
            | ProfileError::InvalidRequest(_) => ErrorCode::Invalid,
            ProfileError::Remote { code, .. } => *code,
        }
    }
}

pub type Result<T, E = ProfileError> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_error_has_a_code_that_serialises_camel_case() {
        let cases = [
            (
                ProfileError::LimitExceeded {
                    what: "x",
                    limit: 1,
                    hint: "y",
                },
                ErrorCode::Limit,
            ),
            (ProfileError::malformed("f", "m"), ErrorCode::Invalid),
            (ProfileError::unsupported("f", "m"), ErrorCode::Unsupported),
            (ProfileError::Unrecognised("a".into(), "b".into()), ErrorCode::NotACapture),
            (
                ProfileError::Importer {
                    importer: "i".into(),
                    message: "m".into(),
                },
                ErrorCode::Invalid,
            ),
            (ProfileError::UnknownCapture(3), ErrorCode::NotFound),
            (
                ProfileError::WrongCapture {
                    id: 1,
                    expected: "a",
                    actual: "b",
                },
                ErrorCode::Invalid,
            ),
            (ProfileError::invalid("m"), ErrorCode::Invalid),
            (ProfileError::NotAvailable("m".into()), ErrorCode::Unsupported),
            (
                ProfileError::Remote {
                    code: ErrorCode::Io,
                    message: "m".into(),
                },
                ErrorCode::Io,
            ),
        ];
        for (e, code) in cases {
            assert_eq!(e.code(), code, "{e}");
        }
        assert_eq!(serde_json::to_value(ErrorCode::NotACapture).unwrap(), "notACapture");
        assert_eq!(serde_json::to_value(ErrorCode::NotFound).unwrap(), "notFound");
        assert_eq!(serde_json::to_value(ErrorCode::Io).unwrap(), "io");
    }
}
