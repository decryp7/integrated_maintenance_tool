//! One typed error for the whole crate. Messages tell the user what to do next and never contain
//! file contents or tokens.

use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq)]
pub enum AiError {
    #[error("cancelled")]
    Cancelled,
    #[error("{0}")]
    Invalid(String),
    #[error("no AI model is set up yet: choose \"Set up AI assistant\" first")]
    NoModel,
    #[error("the model engine failed: {0}")]
    Engine(String),
    #[error("the conversation does not fit the model's context window ({prompt} of {limit} tokens)")]
    ContextOverflow { prompt: usize, limit: usize },
    #[error("the model produced an unusable answer: {0}")]
    Malformed(String),
    #[error("the assistant stopped after {0} tool rounds without an answer")]
    ToolLoopLimit(usize),
    #[error("not a usable model file: {0}")]
    ModelFile(String),
    #[error("not enough memory: {0}")]
    Memory(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("this model needs a Hugging Face account: open {url}, accept the licence, then provide an access token")]
    Gated { url: String },
    #[error("not found: {0}")]
    NotFound(String),
    #[error("the download is corrupt: {0}")]
    Integrity(String),
    #[error("not enough disk space: {0}")]
    Disk(String),
    #[error("refused: {0}")]
    Refused(String),
    #[error("i/o error: {0}")]
    Io(String),
}

pub type Result<T> = std::result::Result<T, AiError>;

impl From<std::io::Error> for AiError {
    fn from(e: std::io::Error) -> Self {
        AiError::Io(e.to_string())
    }
}

impl AiError {
    /// Worth retrying after a pause (flaky network), as opposed to a definite answer.
    pub fn is_transient(&self) -> bool {
        matches!(self, AiError::Network(_))
    }
}
