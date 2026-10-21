//! The worker protocol over IMT's one IPC (ADR 0017): every call is a [`StoreRequest`] frame body (postcard);
//! the operation is typed ([`StoreOp`]), the port DTOs it carries and answers with cross as JSON text (they are
//! the ports' JSON-shaped types, unchanged). A failed call answers a [`imt_ipc::RemoteError`] whose `data` is a
//! [`WireError`], so the client gets the same [`StoreError`] back; anything else (a crash, a hang, garbage, an
//! oversized frame) is a failure of the worker process.

use imt_core::batch::SourceId;
use imt_core::ports::StoreError;
use imt_core::Profile;
use imt_ipc::RemoteError;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::path::PathBuf;

/// Largest frame either way (a page of 5,000 rows with long messages fits).
pub const MAX_MESSAGE_BYTES: usize = 64 << 20;

/// An operation on one open store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StoreOp {
    Open,
    Reload,
    Refresh,
    RemoveSource,
    Clear,
    AppendTail,
    SessionProgress,
    SessionCancel,
    SessionWait,
    Summary,
    Page,
    Timeline,
    Facet,
    IdsAt,
    PositionOf,
    Positioned,
    ActiveProfile,
    ValidateProfile,
    Problems,
    Search,
    IndexStatus,
}

impl StoreOp {
    /// The method name (logs, the worker's dispatch).
    pub fn method(self) -> &'static str {
        match self {
            StoreOp::Open => method::OPEN,
            StoreOp::Reload => method::RELOAD,
            StoreOp::Refresh => method::REFRESH,
            StoreOp::RemoveSource => method::REMOVE_SOURCE,
            StoreOp::Clear => method::CLEAR,
            StoreOp::AppendTail => method::APPEND_TAIL,
            StoreOp::SessionProgress => method::SESSION_PROGRESS,
            StoreOp::SessionCancel => method::SESSION_CANCEL,
            StoreOp::SessionWait => method::SESSION_WAIT,
            StoreOp::Summary => method::SUMMARY,
            StoreOp::Page => method::PAGE,
            StoreOp::Timeline => method::TIMELINE,
            StoreOp::Facet => method::FACET,
            StoreOp::IdsAt => method::IDS_AT,
            StoreOp::PositionOf => method::POSITION_OF,
            StoreOp::Positioned => method::POSITIONED,
            StoreOp::ActiveProfile => method::ACTIVE_PROFILE,
            StoreOp::ValidateProfile => method::VALIDATE_PROFILE,
            StoreOp::Problems => method::PROBLEMS,
            StoreOp::Search => method::SEARCH,
            StoreOp::IndexStatus => method::INDEX_STATUS,
        }
    }
}

/// What the app asks the worker; the reply is the operation's DTO as JSON ([`imt_ipc::Json`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum StoreRequest {
    /// An [`OpenStore`] → [`Handle`].
    OpenStore(imt_ipc::Json),
    CloseStore {
        store: u64,
    },
    /// → [`WorkerStatus`].
    Status,
    /// `op` on the open store `store`, with its request DTO.
    Call {
        store: u64,
        op: StoreOp,
        body: imt_ipc::Json,
    },
}

/// Error code of every store error (the kind is in [`WireError`]).
pub const STORE_ERROR: i32 = -32050;

pub mod method {
    pub const OPEN_STORE: &str = "open_store";
    pub const CLOSE_STORE: &str = "close_store";
    // Ingest
    pub const OPEN: &str = "open";
    pub const RELOAD: &str = "reload";
    pub const REFRESH: &str = "refresh";
    pub const REMOVE_SOURCE: &str = "remove_source";
    pub const CLEAR: &str = "clear";
    pub const APPEND_TAIL: &str = "append_tail";
    pub const SESSION_PROGRESS: &str = "session_progress";
    pub const SESSION_CANCEL: &str = "session_cancel";
    pub const SESSION_WAIT: &str = "session_wait";
    // Query
    pub const SUMMARY: &str = "summary";
    pub const PAGE: &str = "page";
    pub const TIMELINE: &str = "timeline";
    pub const FACET: &str = "facet";
    // Positions
    pub const IDS_AT: &str = "ids_at";
    pub const POSITION_OF: &str = "position_of";
    pub const POSITIONED: &str = "positioned";
    // Profiles
    pub const ACTIVE_PROFILE: &str = "active_profile";
    pub const VALIDATE_PROFILE: &str = "validate_profile";
    // Diagnostics
    pub const PROBLEMS: &str = "problems";
    // Search
    pub const SEARCH: &str = "search";
    pub const INDEX_STATUS: &str = "index_status";
    // The worker itself
    pub const STATUS: &str = "worker_status";
}

/// Opens (or creates) the store in a folder.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenStore {
    pub dir: PathBuf,
    pub profile: Profile,
    /// A new, empty store (an existing one there is replaced).
    pub create: bool,
    /// DuckDB memory limit and threads, parse threads (defaults when absent).
    #[serde(default)]
    pub limits: Option<Limits>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Limits {
    pub parse_threads: usize,
    pub duck_threads: usize,
    pub memory_limit: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Handle {
    pub store: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Session {
    pub session: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Paths {
    pub paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Source {
    pub id: SourceId,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct IdsAt {
    pub from: u64,
    pub len: usize,
}

/// What the worker process is doing (tests and the supervisor's view).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkerStatus {
    pub pid: u32,
    pub rss_bytes: u64,
    pub stores: usize,
}

/// A [`StoreError`] on the wire.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum WireError {
    Unsupported {
        operation: String,
        phase: String,
    },
    Invalid {
        message: String,
    },
    /// An invalid query-language text, with the span to underline.
    Query {
        message: String,
        offset: usize,
        len: usize,
        suggestions: Vec<String>,
    },
    NotFound {
        message: String,
    },
    Busy {
        message: String,
    },
    Cancelled,
    Io {
        message: String,
    },
    Engine {
        message: String,
    },
}

impl From<&StoreError> for WireError {
    fn from(e: &StoreError) -> Self {
        match e {
            StoreError::Unsupported { operation, phase } => WireError::Unsupported {
                operation: operation.to_string(),
                phase: phase.to_string(),
            },
            StoreError::Invalid(m) => WireError::Invalid { message: m.clone() },
            StoreError::Query(q) => WireError::Query {
                message: q.message.clone(),
                offset: q.offset,
                len: q.len,
                suggestions: q.suggestions.clone(),
            },
            StoreError::NotFound(m) => WireError::NotFound { message: m.clone() },
            StoreError::Busy(m) => WireError::Busy { message: m.clone() },
            StoreError::Cancelled => WireError::Cancelled,
            StoreError::Io(m) => WireError::Io { message: m.clone() },
            StoreError::Engine(m) => WireError::Engine { message: m.clone() },
        }
    }
}

impl From<WireError> for StoreError {
    fn from(e: WireError) -> Self {
        match e {
            WireError::Unsupported { operation, phase } => StoreError::Unsupported {
                operation: Cow::Owned(operation),
                phase: Cow::Owned(phase),
            },
            WireError::Invalid { message } => StoreError::Invalid(message),
            WireError::Query {
                message,
                offset,
                len,
                suggestions,
            } => StoreError::Query(imt_core::ql::QlError {
                message,
                offset,
                len,
                suggestions,
            }),
            WireError::NotFound { message } => StoreError::NotFound(message),
            WireError::Busy { message } => StoreError::Busy(message),
            WireError::Cancelled => StoreError::Cancelled,
            WireError::Io { message } => StoreError::Io(message),
            WireError::Engine { message } => StoreError::Engine(message),
        }
    }
}

/// A store error as the typed error of a failed call.
pub fn to_remote(e: &StoreError) -> RemoteError {
    let mut r = RemoteError::new(STORE_ERROR, e.to_string());
    r.data = serde_json::to_value(WireError::from(e)).ok().map(Into::into);
    r
}

/// The store error a typed error carries (a malformed one is an engine error with its message).
pub fn from_remote(r: RemoteError) -> StoreError {
    r.data
        .and_then(|d| serde_json::from_value::<WireError>(d.into_inner()).ok())
        .map(StoreError::from)
        .unwrap_or(StoreError::Engine(r.message))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_errors_survive_the_wire() {
        for e in [
            StoreError::unsupported("chart", "R2c"),
            StoreError::Invalid("bad".into()),
            StoreError::NotFound("source 3".into()),
            StoreError::Busy("import".into()),
            StoreError::Cancelled,
            StoreError::Io("disk".into()),
            StoreError::Engine("x".into()),
            StoreError::Query(imt_core::ql::QlError {
                message: "unknown field".into(),
                offset: 3,
                len: 4,
                suggestions: vec!["Host".into()],
            }),
        ] {
            let r = to_remote(&e);
            let back: RemoteError = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
            assert_eq!(from_remote(back), e);
        }
        assert_eq!(from_remote(RemoteError::new(1, "plain")), StoreError::Engine("plain".into()));
    }
}
