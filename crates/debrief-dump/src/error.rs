//! The engine's typed errors. The app converts them once, at the command edge, into its `CmdError`.

use serde::{Deserialize, Serialize};

/// What kind of failure an error is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCode {
    /// The file is not a dump or UMDH log any importer recognises.
    NotADump,
    /// The file or document does not exist (any more).
    NotFound,
    /// The file could not be read.
    Io,
    /// A size, count or time bound was hit.
    Limit,
    /// What was asked for is not in this document or not supported (here).
    Unsupported,
    /// The file's content or the request is invalid.
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct DumpError {
    pub code: ErrorCode,
    pub message: String,
}

impl DumpError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> DumpError {
        DumpError {
            code,
            message: message.into(),
        }
    }

    pub fn invalid(message: impl Into<String>) -> DumpError {
        DumpError::new(ErrorCode::Invalid, message)
    }

    pub fn limit(message: impl Into<String>) -> DumpError {
        DumpError::new(ErrorCode::Limit, message)
    }

    pub fn unsupported(message: impl Into<String>) -> DumpError {
        DumpError::new(ErrorCode::Unsupported, message)
    }

    pub fn not_found(message: impl Into<String>) -> DumpError {
        DumpError::new(ErrorCode::NotFound, message)
    }
}

pub type Result<T> = std::result::Result<T, DumpError>;
