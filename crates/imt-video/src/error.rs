//! The one error type of video work. Every variant's message says what happened and, where there is one, what to do.

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum VideoError {
    /// An argument from the webview or a file IMT wrote is not acceptable.
    #[error("{0}")]
    Invalid(String),
    #[error("{0} was not found")]
    NotFound(String),
    /// No FFmpeg runtime is installed or chosen (or its files are gone).
    #[error("{0}")]
    NoRuntime(String),
    /// A downloaded file is not the pinned one.
    #[error("{0}")]
    Integrity(String),
    /// A runtime failed its self-test at `step`.
    #[error("the FFmpeg self-test failed while {step}: {message}")]
    SelfTest { step: String, message: String },
    /// FFmpeg ran but failed (its last error lines).
    #[error("{0}")]
    Tool(String),
    /// A process ran longer than its limit and was stopped.
    #[error("{0}")]
    Timeout(String),
    /// A process used more memory than its cap and was stopped.
    #[error("{0}")]
    OverMemory(String),
    /// A limit (frames, length, size) was reached.
    #[error("{0}")]
    Limit(String),
    #[error("cancelled")]
    Cancelled,
    #[error("{0}")]
    Io(String),
    /// Something in the AI part failed (its own message).
    #[error("{0}")]
    Ai(String),
}

impl From<std::io::Error> for VideoError {
    fn from(e: std::io::Error) -> Self {
        if e.kind() == std::io::ErrorKind::StorageFull {
            VideoError::Io("the disk is full: free some space and try again".into())
        } else {
            VideoError::Io(e.to_string())
        }
    }
}

impl From<imt_ai::AiError> for VideoError {
    fn from(e: imt_ai::AiError) -> Self {
        match e {
            imt_ai::AiError::Cancelled => VideoError::Cancelled,
            imt_ai::AiError::Integrity(m) => VideoError::Integrity(m),
            other => VideoError::Ai(other.to_string()),
        }
    }
}

pub type Result<T> = std::result::Result<T, VideoError>;
