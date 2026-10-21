//! The typed error every AI command returns: a stable `code` the UI can act on (hide AI, offer set-up,
//! retry) and a message that tells the user what to do next. Never carries file contents or secrets.

use imt_ai::net::NetKind;
use imt_ai::AiError;

/// The AI's error codes are part of the one [`crate::error::ErrorCode`].
pub type AiErrorCode = crate::error::ErrorCode;

/// AI commands reject with the one [`crate::error::CmdError`].
pub type AiCmdError = crate::error::CmdError;

pub type AiResult<T> = Result<T, AiCmdError>;

impl From<AiError> for AiCmdError {
    fn from(e: AiError) -> Self {
        let code = match &e {
            AiError::Cancelled => AiErrorCode::Cancelled,
            AiError::Invalid(_) => AiErrorCode::Invalid,
            AiError::NoModel => AiErrorCode::NoModel,
            AiError::NoRuntime => AiErrorCode::NoRuntime,
            AiError::NoVision => AiErrorCode::NoVision,
            AiError::Engine(_) => AiErrorCode::Engine,
            AiError::Browser(_) => AiErrorCode::Browser,
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
            (AiError::NoVision, AiErrorCode::NoVision),
            (AiError::Engine("x".into()), AiErrorCode::Engine),
            (AiError::Browser("The browser could not finish: x".into()), AiErrorCode::Browser),
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
        assert_eq!(serde_json::to_value(AiErrorCode::Browser).unwrap(), "browser");
        assert!(AiErrorCode::Browser.retryable(), "a page that did not answer may answer next time");
        assert_eq!(AiCmdError::busy("b").to_string(), "b");
        let fe = AiCmdError::from(imt_core::overwrite::FileExists("/x/a.md".into()));
        assert_eq!(fe.code, AiErrorCode::FileExists);
        assert!(fe.message.ends_with(imt_core::overwrite::FILE_EXISTS_SUFFIX));
    }
}
