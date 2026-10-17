//! Every web search and page fetch needs the user's approval. The UI shows the exact text (or the domain)
//! with Send / Edit / Deny and an "allow searches for this chat" toggle. Every outbound request is logged.

use serde::Serialize;
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ApprovalKind {
    Search,
    Fetch,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequest {
    pub request_id: u64,
    pub kind: ApprovalKind,
    /// The exact query, or the exact URL.
    pub text: String,
    /// For fetches: the site that will be contacted.
    pub domain: Option<String>,
    /// Which search service receives the query.
    pub provider: String,
    /// What the privacy guard removed from the query.
    pub removed: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ApprovalDecision {
    Allow,
    /// Send this text instead (it is redacted again before sending). Ignored for fetches.
    AllowEdited(String),
    /// Allow this search and every further search in this chat (fetches still ask).
    AllowSearchesInChat,
    Deny,
}

/// Contract: blocks until the user decides. A closed dialog or a timeout is `Deny`.
pub trait Approver {
    fn approve(&self, req: &ApprovalRequest) -> ApprovalDecision;
}

/// The safe default: nothing is ever sent.
pub struct DenyAll;

impl Approver for DenyAll {
    fn approve(&self, _: &ApprovalRequest) -> ApprovalDecision {
        ApprovalDecision::Deny
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEntry {
    pub kind: ApprovalKind,
    pub provider: String,
    /// The approved query or the fetched URL: nothing else about the logs.
    pub text: String,
    pub timestamp_ms: i64,
}

pub trait AuditSink: Send + Sync {
    fn record(&self, entry: &AuditEntry);
}

/// Writes to the application log at Info level (target `imt_ai::web`).
pub struct LogAudit;

impl AuditSink for LogAudit {
    fn record(&self, e: &AuditEntry) {
        log::info!(target: "imt_ai::web", "outbound {:?} via {} at {}: {}", e.kind, e.provider, e.timestamp_ms, e.text);
    }
}

/// Keeps entries in memory (tests, and the "what was sent" list in the UI).
#[derive(Default)]
pub struct MemoryAudit {
    pub entries: Mutex<Vec<AuditEntry>>,
}

impl AuditSink for MemoryAudit {
    fn record(&self, e: &AuditEntry) {
        self.entries.lock().unwrap_or_else(|p| p.into_inner()).push(e.clone());
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
