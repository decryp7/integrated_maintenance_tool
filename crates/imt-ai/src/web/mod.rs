//! Web content for the assistant. The AI never searches the web itself: the user browses in IMT's internal
//! browser and hands pages over (see `crate::browser`). This module holds what that needs: the privacy guard
//! that builds redacted queries and redacts page text, names known from the logs, host classification, a
//! bounded HTML to text extractor, and the quoting that marks web text as untrusted data.

mod hosts;
mod html;
mod privacy;
mod terms;
mod untrusted;

pub use hosts::{is_loopback_host, is_private_host};
pub use html::{decode_entities, html_to_text};
pub use privacy::{
    compose_query, public_type_name, redact, sanitize_edited, strip_terms, valid_error_code, QueryIntent, SafeQuery, MAX_QUERY_CHARS,
};
pub use terms::private_terms_in;
pub use untrusted::{quote_untrusted, CLOSE_TAG, OPEN_TAG, UNTRUSTED_REMINDER};
