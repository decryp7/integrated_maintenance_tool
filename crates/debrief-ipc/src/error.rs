//! Errors of a call to another process: a typed [`RemoteError`] the other side answers with (it crosses as a
//! `Fail` frame), and [`IpcError`] for every failed call: a typed error passes through as [`IpcError::Remote`]
//! (the call failed, the process is fine); a broken connection is a crash; a missed deadline is a hang.

use serde::{Deserialize, Serialize};

/// A typed error a child returns for a call (not a failure of the child itself).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteError {
    pub code: i32,
    pub message: String,
    /// Structured details (JSON), e.g. `{"reason": "no_runtime"}`.
    #[serde(default)]
    pub data: Option<crate::wire::Json>,
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
    /// The process needs rights the user did not give (an administrator prompt was declined, or elevation is
    /// not possible here). Not a failure of the process.
    Denied(String),
}

impl IpcError {
    /// Whether the process itself failed (crash, hang, garbage), so it must be restarted.
    pub fn is_process_failure(&self) -> bool {
        !matches!(self, IpcError::Remote(_) | IpcError::Denied(_))
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
            IpcError::Denied(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for IpcError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_errors_roundtrip() {
        let mut e = RemoteError::new(-32001, "no such model — café");
        e.data = Some(serde_json::json!({"limit": 5}).into());
        assert_eq!(e.to_string(), "no such model — café");
        assert_eq!(serde_json::from_str::<RemoteError>(&serde_json::to_string(&e).unwrap()).unwrap(), e);
        assert_eq!(crate::wire::decode::<RemoteError>(&crate::wire::encode(&e).unwrap()).unwrap(), e);
        assert_eq!(
            serde_json::from_str::<RemoteError>(r#"{"code":1,"message":"m"}"#).unwrap(),
            RemoteError::new(1, "m")
        );
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
