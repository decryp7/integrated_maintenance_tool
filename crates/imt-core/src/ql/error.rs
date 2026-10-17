//! The error type of the query language: a message plus the byte span it refers to,
//! so the editor can draw a squiggle.

use serde::Serialize;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QlError {
    pub message: String,
    /// Byte offset into the query text.
    pub offset: usize,
    /// Length in bytes (0 = a position, e.g. "unexpected end of query").
    pub len: usize,
    /// "Did you mean": closest known names for an unknown field, best first.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub suggestions: Vec<String>,
}

impl QlError {
    pub fn new(message: impl Into<String>, offset: usize, len: usize) -> Self {
        QlError {
            message: message.into(),
            offset,
            len,
            suggestions: Vec::new(),
        }
    }
}

impl fmt::Display for QlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (at byte {})", self.message, self.offset)
    }
}

impl std::error::Error for QlError {}
