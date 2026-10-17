//! Web search for error codes and messages: the one exception to "everything stays local", so it is
//! private by design. The engine (not the model) builds a redacted query from an intent, every search and
//! page fetch is approved by the user and logged, and web text reaches the model only as quoted data.

mod approval;
mod engines;
mod html;
mod http;
mod privacy;
mod secrets;
mod toolkit;
mod types;

pub use approval::{
    now_ms, ApprovalDecision, ApprovalKind, ApprovalRequest, Approver, AuditEntry, AuditSink, DenyAll, LogAudit, MemoryAudit,
};
pub use engines::{BraveSearch, HttpPageFetcher, SearxngSearch, BRAVE_ENDPOINT, BRAVE_SECRET_NAME};
pub use html::{decode_entities, html_to_text};
pub use http::{is_loopback_host, is_private_host, WebHttp, WebHttpConfig};
pub use privacy::{
    compose_query, public_type_name, redact, sanitize_edited, strip_terms, valid_error_code, QueryIntent, SafeQuery, MAX_QUERY_CHARS,
};
pub use secrets::{KeyringSecretStore, MemorySecretStore, SecretStore};
pub use toolkit::{quote_untrusted, web_specs, SharedState, WebProvider, WebSearchSettings, WebToolkit, WebTools};
pub use types::{PageFetcher, PageText, SearchHit, WebSearch};
