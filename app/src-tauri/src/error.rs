//! The one error every command rejects with (ADR 0016): [`CmdError`] `{ code, message, retryable, field?,
//! line?, hint?, detail? }`, camelCase on the wire and mirrored as `CmdError` in api.types.ts. The UI shows
//! `message` (and `hint`, the next step), and reacts to `code` and `retryable`, never to the text.
//!
//! Domain errors keep their own types inside the shell and the libraries (`debrief_ai::Error`,
//! [`debrief_profile::api::ProfilingErrorDto`], [`crate::updater::UpdateError`], [`crate::paths`] refusals) and
//! convert here, once, at the command edge. Plain text errors of older helpers become `failed`.

use serde::Serialize;

/// What kind of failure an error is (camelCase on the wire). One list for every command, so a code means
/// the same everywhere; domain codes (the AI's, profiling's, the updater's) are part of it.
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCode {
    // ---- general ----
    /// The command failed; the message says why.
    Failed,
    /// A request the command refuses (a bad argument, a value out of range).
    Invalid,
    /// What the request names does not exist (any more).
    NotFound,
    /// A path the user did not choose: choose it again in a dialog ([`crate::paths`]).
    NotGranted,
    /// Something of the same kind is already running.
    Busy,
    /// Stopped on request.
    Cancelled,
    /// A file could not be read or written.
    Io,
    /// A bounded resource ran out (size, count, time).
    Limit,
    /// Not supported here (platform, format, feature).
    Unsupported,
    /// Off in this run (safe mode, missing component, quarantined).
    Unavailable,
    /// The target exists and replacing it was not confirmed (see `debrief_core::overwrite`).
    FileExists,
    /// Missing rights (administrator, access denied): offer to restart elevated.
    Denied,
    /// A program a tool runs is not installed.
    Missing,
    /// A bug or a crash inside Debrief (contained).
    Internal,
    // ---- the AI ----
    NoModel,
    NoRuntime,
    /// The active model cannot read images (no projector installed for it; ADR 0025).
    NoVision,
    Engine,
    /// The research or agent browser failed (not open, a page that did not answer or changed): not a Debrief bug.
    Browser,
    ContextOverflow,
    Malformed,
    ToolLoopLimit,
    ModelFile,
    Memory,
    Network,
    Tls,
    ProxyAuth,
    Proxy,
    Offline,
    Timeout,
    HttpStatus,
    Gated,
    Integrity,
    Disk,
    Refused,
    /// A new inference runtime failed its self-test with the active model; the previous one stays in use.
    SelfTestFailed,
    // ---- profiling ----
    NotACapture,
    // ---- updates ----
    NotAvailable,
    Unreachable,
    NoRelease,
    NoPlatform,
    InvalidManifest,
    SignatureInvalid,
    TooLarge,
    DiskFull,
    InstallFailed,
    // ---- plugins ----
    /// A plugin package that is unsigned or signed by a key the user has not trusted (developer mode is off).
    Untrusted,
    /// A plugin built for a plugin protocol this Debrief cannot serve; the message says which side to update.
    Incompatible,
}

impl ErrorCode {
    /// Whether trying again as is may succeed (a passing condition), as opposed to needing a change first.
    pub fn retryable(self) -> bool {
        use ErrorCode::*;
        matches!(
            self,
            Failed | Busy | Io | Network | Offline | Timeout | HttpStatus | Unreachable | Proxy | Engine | Browser | Memory | Internal
        )
    }
}

/// The error of every command.
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "CmdErrorDto"))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CmdError {
    pub code: ErrorCode,
    /// For the user, as is.
    pub message: String,
    /// Trying again as is may succeed.
    pub retryable: bool,
    /// The request field the error is about (a form highlights it).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub field: Option<String>,
    /// The 1-based line of a text the error is about (a profile, a query).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub line: Option<usize>,
    /// What to do next, for the user.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub hint: Option<String>,
    /// Technical detail (shown on demand, logged).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub detail: Option<String>,
    /// The service failure this error is a symptom of (`health:store`): the engine reports that once through service
    /// health, so the UI does not show it again as an error of the request. (A one-byte enum: the error stays small.)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub cause: Option<ServiceCause>,
}

/// A service whose outage an error is a symptom of; serialised as the notification cause key (`health:<service>`). One per service of
/// [`crate::health`]; set on an error only while that service is not working (see [`CmdError::in_outage_of`]).
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ServiceCause {
    #[serde(rename = "health:store")]
    Store,
    #[serde(rename = "health:ai")]
    Ai,
    #[serde(rename = "health:plugins")]
    Plugins,
    #[serde(rename = "health:browser")]
    Browser,
    #[serde(rename = "health:downloads")]
    Downloads,
}

impl ServiceCause {
    /// The cause of a service id of [`crate::health`].
    pub fn of(service: &str) -> Option<ServiceCause> {
        match service {
            crate::health::hub::STORE => Some(ServiceCause::Store),
            crate::health::hub::AI => Some(ServiceCause::Ai),
            crate::health::hub::PLUGINS => Some(ServiceCause::Plugins),
            crate::health::hub::BROWSER => Some(ServiceCause::Browser),
            crate::health::hub::DOWNLOADS => Some(ServiceCause::Downloads),
            _ => None,
        }
    }
}

/// The result of a command.
pub type CmdResult<T> = Result<T, CmdError>;

/// A profiling engine result as a command result.
pub fn lift_dto<T>(r: Result<T, debrief_profile::api::ProfilingErrorDto>) -> CmdResult<T> {
    r.map_err(CmdError::from)
}

/// A result of an older helper with a text error, as a command result (the error becomes `failed`).
pub fn lift<T>(r: Result<T, String>) -> CmdResult<T> {
    r.map_err(CmdError::from)
}

impl CmdError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> CmdError {
        CmdError {
            code,
            message: message.into(),
            retryable: code.retryable(),
            field: None,
            line: None,
            hint: None,
            detail: None,
            cause: None,
        }
    }

    /// Marks the error as a symptom of `service`'s outage, but only while service health says that service is not working
    /// (decided from its state, never from the error text). The UI then waits for the service instead of reporting again.
    pub fn in_outage_of(mut self, service: &str) -> CmdError {
        if crate::health::hub::is_in_outage(service) {
            self.cause = ServiceCause::of(service);
        }
        self
    }

    /// Marks the error as a symptom of a service outage (see [`crate::health`]).
    pub fn caused_by(mut self, cause: ServiceCause) -> CmdError {
        self.cause = Some(cause);
        self
    }

    pub fn failed(message: impl Into<String>) -> CmdError {
        CmdError::new(ErrorCode::Failed, message)
    }

    pub fn invalid(message: impl Into<String>) -> CmdError {
        CmdError::new(ErrorCode::Invalid, message)
    }

    pub fn not_found(message: impl Into<String>) -> CmdError {
        CmdError::new(ErrorCode::NotFound, message)
    }

    pub fn not_granted(message: impl Into<String>) -> CmdError {
        CmdError::new(ErrorCode::NotGranted, message)
    }

    pub fn busy(message: impl Into<String>) -> CmdError {
        CmdError::new(ErrorCode::Busy, message)
    }

    pub fn unavailable(message: impl Into<String>) -> CmdError {
        CmdError::new(ErrorCode::Unavailable, message)
    }

    /// The same error about request field `field`.
    pub fn field(mut self, field: impl Into<String>) -> CmdError {
        self.field = Some(field.into());
        self
    }

    /// The same error about line `line` (1-based) of the text the request carried.
    pub fn line(mut self, line: Option<usize>) -> CmdError {
        self.line = line;
        self
    }

    pub fn hint(mut self, hint: impl Into<String>) -> CmdError {
        self.hint = Some(hint.into());
        self
    }

    pub fn detail(mut self, detail: Option<String>) -> CmdError {
        self.detail = detail;
        self
    }

    /// Whether the message contains `text` (tests only; the UI never matches text).
    #[cfg(test)]
    pub fn contains(&self, text: &str) -> bool {
        self.message.contains(text)
    }
}

/// Tests: the error a confirmed-overwrite check gives for `message`.
#[cfg(test)]
pub fn file_exists(message: String) -> CmdError {
    CmdError::new(ErrorCode::FileExists, message)
}

impl std::fmt::Display for CmdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CmdError {}

impl From<String> for CmdError {
    fn from(message: String) -> CmdError {
        CmdError::failed(message)
    }
}

impl From<&str> for CmdError {
    fn from(message: &str) -> CmdError {
        CmdError::failed(message)
    }
}

impl From<debrief_core::overwrite::FileExists> for CmdError {
    fn from(e: debrief_core::overwrite::FileExists) -> CmdError {
        CmdError::new(ErrorCode::FileExists, e.to_string())
    }
}

impl From<debrief_profile::api::ProfilingErrorDto> for CmdError {
    fn from(e: debrief_profile::api::ProfilingErrorDto) -> CmdError {
        use debrief_profile::error::ErrorCode as P;
        let code = match e.code {
            P::NotACapture => ErrorCode::NotACapture,
            P::Io => ErrorCode::Io,
            P::Limit => ErrorCode::Limit,
            P::Unsupported => ErrorCode::Unsupported,
            P::Invalid => ErrorCode::Invalid,
            P::NotFound => ErrorCode::NotFound,
        };
        CmdError::new(code, e.message)
    }
}

/// A job's work failed, or the job did not run (see [`crate::jobs::Stopped`]).
impl crate::jobs::JobFailure for CmdError {
    fn cancelled(&self) -> bool {
        self.code == ErrorCode::Cancelled
    }
}

/// A request whose job never ran: the queue was full (busy), it was cancelled while waiting, or Debrief closed.
impl From<crate::jobs::Stopped> for CmdError {
    fn from(s: crate::jobs::Stopped) -> CmdError {
        use crate::jobs::Stopped;
        match s {
            Stopped::Refused(m) => CmdError::busy(m),
            Stopped::Cancelled => CmdError::new(ErrorCode::Cancelled, "cancelled"),
            Stopped::Interrupted(m) => CmdError::new(ErrorCode::Cancelled, m),
            Stopped::Failed(m) => CmdError::new(ErrorCode::Internal, m),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wire_form_is_camel_case_with_optional_parts_left_out() {
        let e = CmdError::invalid("the regex does not compile")
            .field("toml")
            .line(Some(3))
            .hint("fix the rule");
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            serde_json::json!({ "code": "invalid", "message": "the regex does not compile", "retryable": false,
                "field": "toml", "line": 3, "hint": "fix the rule" })
        );
        let plain = serde_json::to_value(CmdError::from("x".to_string())).unwrap();
        assert_eq!(plain, serde_json::json!({ "code": "failed", "message": "x", "retryable": true }));
        assert_eq!(serde_json::to_value(ErrorCode::NotACapture).unwrap(), "notACapture");
        assert_eq!(serde_json::to_value(ErrorCode::NotGranted).unwrap(), "notGranted");
        assert_eq!(CmdError::busy("b").to_string(), "b");
        assert!(CmdError::from("abc").contains("b"));
        assert!(CmdError::new(ErrorCode::Offline, "o").retryable && !CmdError::not_found("n").retryable);
        assert_eq!(CmdError::failed("x").detail(Some("d".into())).detail.as_deref(), Some("d"));
    }

    #[test]
    fn domain_errors_and_stopped_jobs_convert_once() {
        use crate::jobs::{JobFailure, Stopped};
        let fe = CmdError::from(debrief_core::overwrite::FileExists("/x/a.md".into()));
        assert_eq!(fe.code, ErrorCode::FileExists);
        let p = CmdError::from(debrief_profile::api::ProfilingErrorDto::new(
            debrief_profile::error::ErrorCode::NotACapture,
            "no",
        ));
        assert_eq!((p.code, p.message.as_str()), (ErrorCode::NotACapture, "no"));
        for (code, want) in [
            (debrief_profile::error::ErrorCode::Io, ErrorCode::Io),
            (debrief_profile::error::ErrorCode::Limit, ErrorCode::Limit),
            (debrief_profile::error::ErrorCode::Unsupported, ErrorCode::Unsupported),
            (debrief_profile::error::ErrorCode::Invalid, ErrorCode::Invalid),
            (debrief_profile::error::ErrorCode::NotFound, ErrorCode::NotFound),
        ] {
            assert_eq!(CmdError::from(debrief_profile::api::ProfilingErrorDto::new(code, "m")).code, want);
        }
        assert_eq!(CmdError::from(Stopped::Refused("full".into())).code, ErrorCode::Busy);
        assert!(CmdError::from(Stopped::Cancelled).cancelled());
        assert!(CmdError::from(Stopped::Interrupted("closed".into())).cancelled());
        assert_eq!(CmdError::from(Stopped::Failed("boom".into())).code, ErrorCode::Internal);
        assert!(!CmdError::failed("x").cancelled());
    }
}
