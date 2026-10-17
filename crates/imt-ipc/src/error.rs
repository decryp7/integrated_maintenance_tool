//! Errors of a call to a child process, and the typed error that travels in a binary gRPC trailer.
//!
//! A child answers a failed call with a gRPC status whose `x-imt-error-bin` trailer holds a
//! [`RemoteError`] as JSON. The caller turns any status into an [`IpcError`]: a typed error passes through
//! as [`IpcError::Remote`] (the call failed, the process is fine); a broken connection is a crash; a missed
//! deadline is a hang.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tonic::metadata::MetadataValue;
use tonic::{Code, Status};

/// Binary trailer carrying the typed error (JSON).
pub const ERROR_TRAILER: &str = "x-imt-error-bin";

/// A typed error a child returns for a call (not a failure of the child itself).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteError {
    pub code: i32,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl RemoteError {
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        RemoteError {
            code,
            message: message.into(),
            data: None,
        }
    }
}

impl std::fmt::Display for RemoteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// What went wrong with a call to a child process.
#[derive(Debug, Clone, PartialEq)]
pub enum IpcError {
    /// The process could not be started (or is not started any more: see the message).
    Spawn(String),
    /// The process exited or closed the connection (a crash).
    Died(String),
    /// No answer within the deadline, or it ignored a cancel (a hang).
    Timeout,
    /// The process sent something that is not valid protocol (or refused the token).
    Protocol(String),
    /// The process answered with an error (not a failure of the process).
    Remote(RemoteError),
}

impl IpcError {
    /// Whether the process itself failed (crash, hang, garbage), so it must be restarted.
    pub fn is_process_failure(&self) -> bool {
        !matches!(self, IpcError::Remote(_))
    }
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IpcError::Spawn(m) => write!(f, "{m}"),
            IpcError::Died(m) => write!(f, "it crashed ({m})"),
            IpcError::Timeout => write!(f, "it stopped responding"),
            IpcError::Protocol(m) => write!(f, "it sent an invalid response ({m})"),
            IpcError::Remote(r) => write!(f, "{}", r.message),
        }
    }
}

impl std::error::Error for IpcError {}

/// `e` as a gRPC status with gRPC code `code`, the typed error in the binary trailer.
pub fn to_status(code: Code, e: &RemoteError) -> Status {
    let mut s = Status::new(code, e.message.clone());
    if let Ok(json) = serde_json::to_vec(e) {
        s.metadata_mut().insert_bin(ERROR_TRAILER, MetadataValue::from_bytes(&json));
    }
    s
}

/// The typed error in a status' trailer, if it has one.
pub fn remote_error(s: &Status) -> Option<RemoteError> {
    let bin = s.metadata().get_bin(ERROR_TRAILER)?;
    serde_json::from_slice(&bin.to_bytes().ok()?).ok()
}

/// A failed call as an [`IpcError`]. `died` describes the process when the connection broke (e.g. its exit
/// status); `label` names it in messages ("the engine").
pub fn from_status(s: &Status, label: &str, died: impl FnOnce() -> String) -> IpcError {
    if let Some(e) = remote_error(s) {
        return IpcError::Remote(e);
    }
    match s.code() {
        Code::Unauthenticated | Code::PermissionDenied => IpcError::Protocol(format!("{label} rejected the session token")),
        Code::DeadlineExceeded => IpcError::Timeout,
        // The child's server enforces the call's deadline (`grpc-timeout`) and reports it this way.
        Code::Cancelled if s.message() == tonic::TimeoutExpired(()).to_string() => IpcError::Timeout,
        Code::Unavailable | Code::Cancelled | Code::Unknown | Code::Aborted => IpcError::Died(died()),
        _ => IpcError::Protocol(s.message().to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_errors_travel_in_the_trailer() {
        let mut e = RemoteError::new(-32001, "no such model — café");
        e.data = Some(serde_json::json!({"limit": 5}));
        let s = to_status(Code::FailedPrecondition, &e);
        assert_eq!(remote_error(&s), Some(e.clone()));
        assert_eq!(from_status(&s, "x", || unreachable!()), IpcError::Remote(e.clone()));
        assert_eq!(e.to_string(), "no such model — café");
        assert_eq!(serde_json::from_str::<RemoteError>(&serde_json::to_string(&e).unwrap()).unwrap(), e);
        assert!(!serde_json::to_string(&RemoteError::new(1, "m")).unwrap().contains("data"));
    }

    #[test]
    fn statuses_without_a_typed_error_are_classified() {
        let f = |c: Code| from_status(&Status::new(c, "msg"), "the engine", || "exited (1)".into());
        assert_eq!(
            f(Code::Unauthenticated),
            IpcError::Protocol("the engine rejected the session token".into())
        );
        assert_eq!(
            f(Code::PermissionDenied),
            IpcError::Protocol("the engine rejected the session token".into())
        );
        assert_eq!(f(Code::DeadlineExceeded), IpcError::Timeout);
        for c in [Code::Unavailable, Code::Cancelled, Code::Unknown, Code::Aborted] {
            assert_eq!(f(c), IpcError::Died("exited (1)".into()));
        }
        assert_eq!(
            from_status(&Status::cancelled("Timeout expired"), "x", String::new),
            IpcError::Timeout
        );
        assert_eq!(f(Code::InvalidArgument), IpcError::Protocol("msg".into()));
        // A trailer that is not a typed error is ignored.
        let mut s = Status::new(Code::Internal, "m");
        s.metadata_mut().insert_bin(ERROR_TRAILER, MetadataValue::from_bytes(b"not json"));
        assert_eq!(from_status(&s, "x", String::new), IpcError::Protocol("m".into()));
    }

    #[test]
    fn errors_describe_themselves() {
        assert_eq!(IpcError::Spawn("cannot".into()).to_string(), "cannot");
        assert_eq!(IpcError::Died("x".into()).to_string(), "it crashed (x)");
        assert_eq!(IpcError::Timeout.to_string(), "it stopped responding");
        assert_eq!(IpcError::Protocol("p".into()).to_string(), "it sent an invalid response (p)");
        assert_eq!(IpcError::Remote(RemoteError::new(1, "r")).to_string(), "r");
        assert!(IpcError::Timeout.is_process_failure());
        assert!(!IpcError::Remote(RemoteError::new(1, "r")).is_process_failure());
    }
}
