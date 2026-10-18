//! The internal web browser, as the engine sees it (ADR 0011). The user browses; the AI gets web content
//! only through this module:
//!
//! 1. **Hand-off:** "Add page to AI context" captures the visible page ([`capture`]), redacted and quoted as
//!    untrusted web content.
//! 2. **Read-along** (opt-in per chat): the model may read the page the user has open ([`tools`]).
//! 3. **Suggested links:** the model proposes a search or a page as a card; nothing loads until the user
//!    clicks.
//! 4. **Agent mode** (opt-in per chat): the model drives the visible browser, one approved step at a time
//!    ([`policy`], [`leak`], [`pace`]), and stops for CAPTCHAs, logins and payments ([`snapshot`]).
//!
//! The app reaches the page through the [`port`] traits by evaluating [`scripts`] with a callback, so the page
//! never gets IPC ([`protocol`]). Everything here is pure or behind those traits, and tested without a
//! webview.

pub mod capture;
pub mod control;
pub mod engines;
pub mod leak;
pub mod pace;
pub mod policy;
pub mod port;
pub mod protocol;
pub mod query;
pub mod research;
pub mod scripts;
pub mod snapshot;
pub mod tools;
pub mod url;

pub use capture::{capture_from, PageCapture};
pub use control::{small_for_agent, AgentLimits, ChatBrowser, ChatBrowserStatus};
pub use engines::{SearchEngine, SearchError, SearchSettings, SearchTemplate};
pub use leak::{LeakKind, LeakReport, Segment};
pub use policy::{AgentAction, Ask, AskKind, ScrollDirection};
pub use port::{
    ActionEntry, ActionLog, ActionOutcome, AgentApprover, ApprovalRequest, BrowserError, Decision, DenyAll, MemoryLog, PageDriver,
    PageReader,
};
pub use protocol::{Nonce, ReplyError, ReplyGate};
pub use query::{event_query, EventText};
pub use research::{check_store_query, clamp_results, CheckedQuery, ResearchReport, ResearchRequest, ResearchSettings, Researcher};
pub use snapshot::{snapshot_from, DomSnapshot, Element, ElementKind, HandOff};
pub use tools::ResearchTool;
pub use tools::{browser_specs, BrowserEnv, BrowserTools, PauseReason};
pub use url::{check_navigation, Navigation, Site, UrlError, WebUrl};

use crate::web::{quote_untrusted, UNTRUSTED_REMINDER};

/// At most `max` characters of `s`; a cut ends with an ellipsis (within `max`).
pub(crate) fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// `body` quoted as untrusted web content from `source`, with the reminder. The body is cut so the whole fits
/// `max_chars` (when that leaves room for any body at all); the closing tag is never cut.
pub(crate) fn quote_capped(source: &str, body: &str, max_chars: usize) -> String {
    let frame = format!("{UNTRUSTED_REMINDER}\n{}", quote_untrusted(source, "")).chars().count();
    format!(
        "{UNTRUSTED_REMINDER}\n{}",
        quote_untrusted(source, &clip(body, max_chars.saturating_sub(frame)))
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web::CLOSE_TAG;

    #[test]
    fn clipping_stays_within_the_limit() {
        assert_eq!(clip("abc", 3), "abc");
        assert_eq!(clip("abcdef", 4), "abc…");
        assert_eq!(clip("abc", 0), "");
        let q = quote_capped("https://e.org/", &"x".repeat(5000), 600);
        assert_eq!(q.chars().count(), 600);
        assert!(q.ends_with(CLOSE_TAG));
        assert!(quote_capped("https://e.org/", "short", 600).contains("\nshort\n"));
    }
}
