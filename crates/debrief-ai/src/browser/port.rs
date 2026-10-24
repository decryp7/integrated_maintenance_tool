//! The browser ports. The chat only ever reads and drives the visible browser through these small traits;
//! the app implements them on its browser window (script evaluation with a callback, see `protocol`), tests
//! use fakes. Page content is untrusted data, whatever an adapter returns.

use super::capture::PageCapture;
use super::leak::Segment;
use super::policy::{Ask, ScrollDirection};
use super::protocol::ReplyError;
use super::snapshot::{DomSnapshot, Element};
use super::url::WebUrl;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BrowserError {
    #[error("the browser is not open; open it from the Browser panel first")]
    NotOpen,
    #[error("the page did not answer in time")]
    Timeout,
    #[error("{0}")]
    Reply(#[from] ReplyError),
    #[error("the page changed: {0}; read it again")]
    PageChanged(String),
    #[error("{0}")]
    Failed(String),
    /// The browser window was closed (by the user) or the browser ended: this page is gone for good. A later call
    /// opens a new browser; work that was using this one stops instead of carrying on in another.
    #[error("the browser window was closed")]
    Closed,
}

/// Reads the page the user sees.
///
/// Contract: returns within a few seconds (a timeout is [`BrowserError::Timeout`]), never panics, and caps
/// what it returns ([`PageCapture`] and [`DomSnapshot`] limits).
pub trait PageReader: Send + Sync {
    fn capture(&self) -> Result<PageCapture, BrowserError>;
    /// Numbers the interactive elements; a ref stays the same for an element as long as the page lives.
    fn snapshot(&self) -> Result<DomSnapshot, BrowserError>;
}

/// Drives the visible browser (agent mode).
///
/// Contract: each call returns once the action ran and any page load it started finished (or a timeout
/// passed); an element is checked against the snapshot it came from before it is touched.
pub trait PageDriver: Send + Sync {
    fn navigate(&self, url: &WebUrl) -> Result<(), BrowserError>;
    /// Loads `url`, a search engine's own redirect to a result, and lets it go on to the one other site it redirects
    /// to (see `ChatBrowser::expect_redirect`). Drivers that do not restrict navigation just navigate.
    fn navigate_redirect(&self, url: &WebUrl) -> Result<(), BrowserError> {
        self.navigate(url)
    }
    fn click(&self, element: &Element) -> Result<(), BrowserError>;
    fn type_text(&self, element: &Element, text: &str, submit: bool) -> Result<(), BrowserError>;
    fn scroll(&self, direction: ScrollDirection) -> Result<(), BrowserError>;
    fn back(&self) -> Result<(), BrowserError>;
}

/// The user's answer to an approval card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow,
    /// Allow, and let this chat visit the site again without asking.
    AllowSite,
    Deny,
}

/// What the approval card shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequest {
    /// Numbered from 1 in every turn.
    pub request_id: u64,
    pub ask: Ask,
    /// The address, split for highlighting what the leak guard found.
    pub url_segments: Vec<Segment>,
    /// The typed text, split the same way.
    pub text_segments: Vec<Segment>,
}

/// Contract: blocks until the user decides. A closed card, a timeout, Stop or a cancelled turn is `Deny`.
pub trait AgentApprover {
    fn approve(&self, request: &ApprovalRequest) -> Decision;
}

/// The safe default: nothing is ever approved.
pub struct DenyAll;

impl AgentApprover for DenyAll {
    fn approve(&self, _: &ApprovalRequest) -> Decision {
        Decision::Deny
    }
}

/// How an agent action ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ActionOutcome {
    /// Ran without needing approval.
    Done,
    /// Ran after the user approved it.
    Approved,
    Denied,
    /// The rules forbid it (a private network, not a text field, ...).
    Refused,
    /// Stopped for a CAPTCHA, a login or a payment page.
    HandedBack,
    /// The step budget is spent.
    StepCap,
    /// Agent mode was off or stopped.
    Stopped,
    Failed,
}

impl ActionOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            ActionOutcome::Done => "done",
            ActionOutcome::Approved => "approved",
            ActionOutcome::Denied => "denied",
            ActionOutcome::Refused => "refused",
            ActionOutcome::HandedBack => "handedBack",
            ActionOutcome::StepCap => "stepCap",
            ActionOutcome::Stopped => "stopped",
            ActionOutcome::Failed => "failed",
        }
    }
}

/// One line of the activity log: what the AI did in the browser. Never page text or typed text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionEntry {
    pub chat: String,
    pub action: &'static str,
    /// The site acted on or loaded (host only).
    pub site: Option<String>,
    pub outcome: ActionOutcome,
    /// The step number in the task (0 when no step was taken).
    pub step: u32,
}

/// Where every agent action is recorded (the activity log in the app, so diagnostics reports show it).
pub trait ActionLog: Send + Sync {
    fn record(&self, entry: &ActionEntry);
}

/// Keeps entries in memory (tests).
#[derive(Default)]
pub struct MemoryLog {
    pub entries: std::sync::Mutex<Vec<ActionEntry>>,
}

impl ActionLog for MemoryLog {
    fn record(&self, entry: &ActionEntry) {
        self.entries.lock().unwrap_or_else(|p| p.into_inner()).push(entry.clone());
    }
}
