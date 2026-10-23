//! "Search the web for this error": the engine (not the UI, not the model) turns an event into a search
//! text with the privacy guard. It takes the exception type, an error code and the message template (or the
//! first line of the message), then removes names known from the logs and anything private. The user sees
//! the result, with what was removed, and can edit it before the browser opens.

use crate::web::{compose_query, private_terms_in, QueryIntent, SafeQuery};
use debrief_core::ports::Store;
use regex::Regex;
use std::sync::OnceLock;

/// The parts of an event a search is built from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventText {
    pub message: String,
    /// The message with its variable parts as `<*>` (from the pattern miner), when known.
    pub template: Option<String>,
    pub exception: Option<String>,
}

fn re(p: &str) -> Regex {
    Regex::new(p).unwrap_or_else(|e| unreachable!("static pattern {p}: {e}"))
}

/// `System.Net.Http.HttpRequestException: ...` at the start of an exception: the type.
fn exception_type(exception: &str) -> Option<String> {
    static R: OnceLock<Regex> = OnceLock::new();
    let first = exception.lines().find(|l| !l.trim().is_empty())?.trim();
    R.get_or_init(|| re(r"^([A-Za-z_][\w`+]*(?:\.[A-Za-z_][\w`+]*)*(?:Exception|Error|Fault|Failure))\b"))
        .captures(first)
        .map(|c| c[1].to_string())
}

/// An HRESULT, Win32/errno-like code or a compiler-style code mentioned in the text.
fn error_code(text: &str) -> Option<String> {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| re(r"\b(0[xX][0-9A-Fa-f]{8}|ERROR_[A-Z0-9_]{3,40}|E[A-Z]{3,12}(?:\b|$)|[A-Z]{2,4}\d{4,5})\b"))
        .captures(text)
        .map(|c| c[1].to_string())
}

fn first_line(s: &str) -> &str {
    s.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim()
}

/// The search text for an event; `private_terms` are the names known from the logs.
pub fn event_query(e: &EventText, private_terms: &[String]) -> Result<SafeQuery, String> {
    let exc = e.exception.as_deref().unwrap_or("");
    let exception_type = exception_type(exc);
    let code = error_code(&e.message).or_else(|| error_code(first_line(exc)));
    let text = e.template.as_deref().filter(|t| !t.trim().is_empty()).unwrap_or(&e.message);
    // Message templates name their holes ({OrderId}, {0}): they are not words to search for.
    static HOLES: OnceLock<Regex> = OnceLock::new();
    let holes = HOLES.get_or_init(|| re(r"\{[@$]?[A-Za-z_0-9][\w.:,-]*\}|<\*>"));
    let mut free = holes.replace_all(first_line(text), " ").into_owned();
    // The exception's message tells more than a generic "Request failed" when there is one.
    if let Some(t) = &exception_type {
        let after = first_line(exc)
            .strip_prefix(t.as_str())
            .unwrap_or("")
            .trim_start_matches([':', ' ']);
        if !after.is_empty() {
            free = after.to_string();
        }
    }
    if let Some(c) = &code {
        free = free.replace(c.as_str(), " ");
    }
    let intent = QueryIntent {
        exception_type,
        error_code: code,
        free_text: Some(free),
        ..Default::default()
    };
    compose_query(&intent, &|_| None, private_terms)
}

/// The text parts of event `id` in the loaded logs (`None` when there is no such event).
pub fn event_text_of(store: &dyn Store, id: u32) -> Option<EventText> {
    let req = debrief_core::ports::EventsRequest {
        ids: vec![debrief_core::batch::EventId(u64::from(id))],
        group_by: Vec::new(),
    };
    let row = store.events(&req).ok()?.into_iter().next()?;
    Some(EventText {
        message: row.message,
        template: row.template,
        exception: row.exception,
    })
}

/// "Search the web for this error" for event `id`: the redacted search text, or why there is none.
pub fn search_for_event(store: &dyn Store, id: u32) -> Result<SafeQuery, String> {
    let e = event_text_of(store, id).ok_or_else(|| format!("there is no event #{id} in the loaded logs"))?;
    event_query(&e, &private_terms_in(store))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(message: &str, template: Option<&str>, exception: Option<&str>) -> EventText {
        EventText {
            message: message.into(),
            template: template.map(str::to_string),
            exception: exception.map(str::to_string),
        }
    }

    #[test]
    fn the_exception_type_code_and_message_make_the_query() {
        let q = event_query(
            &ev(
                "Payment for order 4711 failed on WEB02 (alice@contoso.com)",
                None,
                Some("System.TimeoutException: The operation has timed out.\n   at Gateway.Call()"),
            ),
            &["WEB02".to_string()],
        )
        .unwrap();
        assert_eq!(q.query, "System.TimeoutException The operation has timed out.");
        let q = event_query(
            &ev("Login failed for alice@contoso.com on WEB02 with 0x80070005", None, None),
            &["WEB02".into()],
        )
        .unwrap();
        assert!(q.query.contains("0x80070005") && q.query.contains("Login failed"), "{q:?}");
        assert!(!q.query.contains("alice") && !q.query.contains("WEB02"), "{q:?}");
        assert!(
            q.removed.len() >= 2 && q.removed.iter().any(|r| r.contains("names from your logs")),
            "{q:?}"
        );
    }

    #[test]
    fn templates_drop_the_variable_parts() {
        let q = event_query(
            &ev("Slow request /orders/4711 took 5300 ms", Some("Slow request <*> took <*> ms"), None),
            &[],
        )
        .unwrap();
        assert_eq!(q.query, "Slow request took ms");
    }

    #[test]
    fn codes_and_types_are_recognised() {
        assert_eq!(error_code("failed with ERROR_ACCESS_DENIED"), Some("ERROR_ACCESS_DENIED".into()));
        assert_eq!(error_code("errno ECONNREFUSED here"), Some("ECONNREFUSED".into()));
        assert_eq!(error_code("CS0103 the name does not exist"), Some("CS0103".into()));
        assert_eq!(error_code("nothing here"), None);
        assert_eq!(
            exception_type("java.net.SocketTimeoutException: Read timed out"),
            Some("java.net.SocketTimeoutException".into())
        );
        assert_eq!(exception_type("\n  KeyError: 'x'"), Some("KeyError".into()));
        assert_eq!(exception_type("Something went wrong"), None);
    }

    #[test]
    fn nothing_searchable_is_an_error() {
        assert!(event_query(&ev("10.0.0.5 alice@contoso.com", None, None), &[]).is_err());
    }
}
