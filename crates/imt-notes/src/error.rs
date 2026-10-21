use thiserror::Error;

/// Why a note was refused. The messages are for the user and never contain note text.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum NotesError {
    /// A value that fails its checks (an empty text, a region outside the image, ...).
    #[error("{0}")]
    Invalid(String),
    /// The note (or what it names) does not exist any more.
    #[error("{0}")]
    NotFound(String),
    /// A bound ran out (too many notes, a text too long).
    #[error("{0}")]
    Limit(String),
}

pub type Result<T> = std::result::Result<T, NotesError>;

pub(crate) fn invalid(msg: impl Into<String>) -> NotesError {
    NotesError::Invalid(msg.into())
}
