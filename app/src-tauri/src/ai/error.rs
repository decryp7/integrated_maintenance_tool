//! The typed error every AI command returns: a stable `code` the UI can act on (hide AI, offer set-up,
//! retry) and a message that tells the user what to do next. Never carries file contents or secrets.

use imt_ai::net::NetKind;
use imt_ai::AiError;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AiErrorCode {
    /// AI is off in this run (safe mode, no worker binary, switched off after repeated crashes).
    Unavailable,
    /// No model is set up yet.
    NoModel,
    /// No inference runtime (llama-server build) is installed: offer the download.
    NoRuntime,
    /// Something of the same kind is already running (a set-up, a turn of this chat).
    Busy,
    Cancelled,
    Invalid,
    Engine,
    ContextOverflow,
    Malformed,
    ToolLoopLimit,
    ModelFile,
    Memory,
    Network,
    /// The server certificate could not be verified (usually a TLS-inspecting proxy such as Zscaler whose
    /// root certificate is not in the OS store): offer the manual download.
    Tls,
    /// The proxy asks for a sign-in (HTTP 407).
    ProxyAuth,
    /// The configured download proxy cannot be used.
    Proxy,
    /// No network, or the host cannot be resolved or reached.
    Offline,
    Timeout,
    /// The server (or a proxy in between) answered with an unexpected HTTP status.
    HttpStatus,
    Gated,
    NotFound,
    Integrity,
    Disk,
    Refused,
    Io,
    /// The export target exists and replacing it was not confirmed (see `imt_core::overwrite`).
    FileExists,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiCmdError {
    pub code: AiErrorCode,
    pub message: String,
}

pub type AiResult<T> = Result<T, AiCmdError>;

impl AiCmdError {
    pub fn new(code: AiErrorCode, message: impl Into<String>) -> Self {
        AiCmdError {
            code,
            message: message.into(),
        }
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(AiErrorCode::Unavailable, message)
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(AiErrorCode::Invalid, message)
    }

    pub fn busy(message: impl Into<String>) -> Self {
        Self::new(AiErrorCode::Busy, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(AiErrorCode::NotFound, message)
    }
}

impl std::fmt::Display for AiCmdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<AiError> for AiCmdError {
    fn from(e: AiError) -> Self {
        let code = match &e {
            AiError::Cancelled => AiErrorCode::Cancelled,
            AiError::Invalid(_) => AiErrorCode::Invalid,
            AiError::NoModel => AiErrorCode::NoModel,
            AiError::NoRuntime => AiErrorCode::NoRuntime,
            AiError::Engine(_) => AiErrorCode::Engine,
            AiError::ContextOverflow { .. } => AiErrorCode::ContextOverflow,
            AiError::Malformed(_) => AiErrorCode::Malformed,
            AiError::ToolLoopLimit(_) => AiErrorCode::ToolLoopLimit,
            AiError::ModelFile(_) => AiErrorCode::ModelFile,
            AiError::Memory(_) => AiErrorCode::Memory,
            AiError::Network(_) => AiErrorCode::Network,
            AiError::Connection(n) => match n.kind {
                NetKind::Tls => AiErrorCode::Tls,
                NetKind::ProxyAuth => AiErrorCode::ProxyAuth,
                NetKind::Proxy => AiErrorCode::Proxy,
                NetKind::Offline => AiErrorCode::Offline,
                NetKind::Timeout => AiErrorCode::Timeout,
                NetKind::Http(_) => AiErrorCode::HttpStatus,
            },
            AiError::Gated { .. } => AiErrorCode::Gated,
            AiError::NotFound(_) => AiErrorCode::NotFound,
            AiError::Integrity(_) => AiErrorCode::Integrity,
            AiError::Disk(_) => AiErrorCode::Disk,
            AiError::Refused(_) => AiErrorCode::Refused,
            AiError::Io(_) => AiErrorCode::Io,
        };
        AiCmdError::new(code, e.to_string())
    }
}

/// AI requests run as jobs: a cancelled request cancels its job instead of failing it.
impl crate::jobs::JobFailure for AiCmdError {
    fn cancelled(&self) -> bool {
        self.code == AiErrorCode::Cancelled
    }
}

/// A request whose job never ran: the queue was full (busy), it was cancelled while waiting, or IMT closed.
impl From<crate::jobs::Stopped> for AiCmdError {
    fn from(s: crate::jobs::Stopped) -> Self {
        use crate::jobs::Stopped;
        match s {
            Stopped::Refused(m) => AiCmdError::busy(m),
            Stopped::Cancelled => AiCmdError::new(AiErrorCode::Cancelled, "cancelled"),
            Stopped::Interrupted(m) => AiCmdError::new(AiErrorCode::Cancelled, m),
            Stopped::Failed(m) => AiCmdError::new(AiErrorCode::Engine, m),
        }
    }
}

impl From<imt_core::overwrite::FileExists> for AiCmdError {
    fn from(e: imt_core::overwrite::FileExists) -> Self {
        AiCmdError::new(AiErrorCode::FileExists, e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn net(kind: NetKind) -> AiError {
        AiError::Connection(imt_ai::net::NetError::new(kind, "huggingface.co", "x"))
    }

    #[test]
    fn library_errors_keep_their_kind_and_message() {
        let cases = [
            (AiError::Cancelled, AiErrorCode::Cancelled),
            (AiError::Invalid("x".into()), AiErrorCode::Invalid),
            (AiError::NoModel, AiErrorCode::NoModel),
            (AiError::NoRuntime, AiErrorCode::NoRuntime),
            (AiError::Engine("x".into()), AiErrorCode::Engine),
            (AiError::ContextOverflow { prompt: 2, limit: 1 }, AiErrorCode::ContextOverflow),
            (AiError::Malformed("x".into()), AiErrorCode::Malformed),
            (AiError::ToolLoopLimit(3), AiErrorCode::ToolLoopLimit),
            (AiError::ModelFile("x".into()), AiErrorCode::ModelFile),
            (AiError::Memory("x".into()), AiErrorCode::Memory),
            (AiError::Network("x".into()), AiErrorCode::Network),
            (net(NetKind::Tls), AiErrorCode::Tls),
            (net(NetKind::ProxyAuth), AiErrorCode::ProxyAuth),
            (net(NetKind::Proxy), AiErrorCode::Proxy),
            (net(NetKind::Offline), AiErrorCode::Offline),
            (net(NetKind::Timeout), AiErrorCode::Timeout),
            (net(NetKind::Http(502)), AiErrorCode::HttpStatus),
            (AiError::Gated { url: "u".into() }, AiErrorCode::Gated),
            (AiError::NotFound("x".into()), AiErrorCode::NotFound),
            (AiError::Integrity("x".into()), AiErrorCode::Integrity),
            (AiError::Disk("x".into()), AiErrorCode::Disk),
            (AiError::Refused("x".into()), AiErrorCode::Refused),
            (AiError::Io("x".into()), AiErrorCode::Io),
        ];
        for (e, code) in cases {
            let msg = e.to_string();
            let c = AiCmdError::from(e);
            assert_eq!((c.code, c.message.as_str()), (code, msg.as_str()));
        }
        let json = serde_json::to_value(AiCmdError::from(AiError::NoModel)).unwrap();
        assert_eq!(json["code"], "noModel");
        assert_eq!(serde_json::to_value(AiErrorCode::NoRuntime).unwrap(), "noRuntime");
        assert_eq!(serde_json::to_value(AiErrorCode::ProxyAuth).unwrap(), "proxyAuth");
        assert_eq!(serde_json::to_value(AiErrorCode::HttpStatus).unwrap(), "httpStatus");
        assert_eq!(AiCmdError::busy("b").to_string(), "b");
        let fe = AiCmdError::from(imt_core::overwrite::FileExists("/x/a.md".into()));
        assert_eq!(fe.code, AiErrorCode::FileExists);
        assert!(fe.message.ends_with(imt_core::overwrite::FILE_EXISTS_SUFFIX));
    }
}
