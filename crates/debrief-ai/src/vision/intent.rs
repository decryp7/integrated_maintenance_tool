//! Whether a chat question that comes with an image asks for the logs related to it ("find the logs related to
//! this screenshot", "/related"), so the engine's search runs instead of a plain answer. Pure.

/// Words that ask for a search, and words that name what is searched.
const ASK: &[&str] = &[
    "related",
    "relate",
    "correlat",
    "matching",
    "match",
    "find",
    "search",
    "look for",
    "show",
    "which",
    "where",
    "corresponding",
];
const LOGS: &[&str] = &["log", "event", "line", "entr", "record", "trace"];

/// True for a request for the related logs. A bare `/related` is the explicit command.
pub fn asks_for_related_logs(text: &str) -> bool {
    let t = text.trim().to_lowercase();
    if t == "/related" || t.starts_with("/related ") {
        return true;
    }
    if t.chars().count() > 300 {
        return false;
    }
    ASK.iter().any(|a| t.contains(a)) && LOGS.iter().any(|l| t.contains(l))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn related_requests_are_recognised() {
        for yes in [
            "find the logs related to this screenshot",
            "Which log lines match this error?",
            "/related",
            "/related please",
            "show me the events around this dialog",
            "Find related entries",
            "are there matching traces?",
        ] {
            assert!(asks_for_related_logs(yes), "{yes}");
        }
        for no in [
            "what does this dialog mean?",
            "explain this screenshot",
            "/relatedness",
            "",
            &format!("find the logs {}", "x".repeat(400)),
        ] {
            assert!(!asks_for_related_logs(no), "{no}");
        }
    }
}
