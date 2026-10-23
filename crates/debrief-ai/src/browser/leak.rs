//! The leak guard of agent mode: workspace data (names known from the logs, values and text found in the
//! loaded logs) that would go into a URL or a typed field. The approval card shows the text in segments with
//! the risky parts highlighted, and data from the logs always needs the user's approval, even on a site the
//! chat may already visit.

use crate::web::private_spans;
use serde::Serialize;
use std::ops::Range;

/// Most look-ups in the loaded logs per checked text (each is a search of the workspace).
pub const MAX_PROBES: usize = 12;
/// Shortest run of text that is looked up in the logs as a possible log line.
const MIN_PHRASE_CHARS: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LeakKind {
    /// Looks private (an address, a path, a token) but was not found in the logs.
    Private,
    /// A name the logs mark as private (host, user, file names), or a value found in the logs.
    LogValue,
    /// A piece of text that appears in the loaded logs (a log line).
    LogText,
}

/// A piece of the checked text; `leak` marks what to highlight.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Segment {
    pub text: String,
    pub leak: Option<LeakKind>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LeakReport {
    pub segments: Vec<Segment>,
}

impl LeakReport {
    /// Data from the loaded logs is in the text (approval is needed whatever else holds).
    pub fn from_logs(&self) -> bool {
        self.segments
            .iter()
            .any(|s| matches!(s.leak, Some(LeakKind::LogValue | LeakKind::LogText)))
    }

    pub fn any(&self) -> bool {
        self.segments.iter().any(|s| s.leak.is_some())
    }
}

fn is_word(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_alphanumeric() || c == '_')
}

/// Whole-word, case-insensitive occurrences of `term` (byte ranges into `text`).
fn find_term(text: &str, term: &str) -> Vec<Range<usize>> {
    let hay = text.to_lowercase();
    let needle = term.to_lowercase();
    // Lower-casing can change byte lengths of exotic characters; then the ranges would not fit.
    if needle.is_empty() || hay.len() != text.len() {
        return vec![];
    }
    let mut out = vec![];
    let mut from = 0;
    while let Some(i) = hay[from..].find(&needle) {
        let (s, e) = (from + i, from + i + needle.len());
        if !is_word(text[..s].chars().next_back()) && !is_word(text[e..].chars().next()) {
            out.push(s..e);
        }
        from = e;
    }
    out
}

/// Runs of at least [`MIN_PHRASE_CHARS`] characters with three or more words, between URL separators.
fn phrases(text: &str) -> Vec<Range<usize>> {
    let mut out = vec![];
    let mut start = 0;
    let bytes: Vec<(usize, char)> = text.char_indices().collect();
    let push = |s: usize, e: usize, out: &mut Vec<Range<usize>>| {
        let piece = text[s..e].trim();
        if piece.chars().count() >= MIN_PHRASE_CHARS && piece.split_whitespace().count() >= 3 {
            let lead = text[s..e].len() - text[s..e].trim_start().len();
            out.push(s + lead..s + lead + piece.len());
        }
    };
    for (i, c) in &bytes {
        if matches!(c, '?' | '&' | '=' | '/' | '#' | '\n' | '|' | ';') {
            push(start, *i, &mut out);
            start = i + c.len_utf8();
        }
    }
    push(start, text.len(), &mut out);
    out
}

/// Checks `text` for workspace data. `terms` are the names known from the logs; `in_logs` searches the loaded
/// logs for a value (called at most [`MAX_PROBES`] times).
pub fn check(text: &str, terms: &[String], in_logs: &dyn Fn(&str) -> bool) -> LeakReport {
    let mut spans: Vec<(Range<usize>, LeakKind)> = Vec::new();
    for t in terms.iter().filter(|t| t.chars().count() >= 3) {
        spans.extend(find_term(text, t).into_iter().map(|r| (r, LeakKind::LogValue)));
    }
    let mut probes = 0;
    let mut probe = |s: &str| {
        if probes >= MAX_PROBES {
            return false;
        }
        probes += 1;
        in_logs(s)
    };
    for (r, _) in private_spans(text) {
        let kind = if probe(&text[r.clone()]) {
            LeakKind::LogValue
        } else {
            LeakKind::Private
        };
        spans.push((r, kind));
    }
    for r in phrases(text) {
        if !spans.iter().any(|(s, _)| s.start <= r.start && r.end <= s.end) && probe(&text[r.clone()]) {
            spans.push((r, LeakKind::LogText));
        }
    }
    segments(text, spans)
}

/// Splits `text` at the spans; overlapping spans merge and keep the strongest kind.
fn segments(text: &str, mut spans: Vec<(Range<usize>, LeakKind)>) -> LeakReport {
    spans.sort_by_key(|(r, k)| (r.start, std::cmp::Reverse(*k)));
    let mut merged: Vec<(Range<usize>, LeakKind)> = Vec::new();
    for (r, k) in spans {
        match merged.last_mut() {
            Some((last, lk)) if r.start < last.end => {
                last.end = last.end.max(r.end);
                *lk = (*lk).max(k);
            }
            _ => merged.push((r, k)),
        }
    }
    let mut out = Vec::new();
    let mut at = 0;
    for (r, k) in merged {
        if r.start > at {
            out.push(Segment {
                text: text[at..r.start].to_string(),
                leak: None,
            });
        }
        out.push(Segment {
            text: text[r.clone()].to_string(),
            leak: Some(k),
        });
        at = r.end;
    }
    if at < text.len() || out.is_empty() {
        out.push(Segment {
            text: text[at..].to_string(),
            leak: None,
        });
    }
    LeakReport { segments: out }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn marked(r: &LeakReport) -> Vec<(String, LeakKind)> {
        r.segments.iter().filter_map(|s| s.leak.map(|k| (s.text.clone(), k))).collect()
    }

    #[test]
    fn names_values_and_log_lines_are_found() {
        let logs = |s: &str| ["10.0.0.5", "Payment gateway call exceeded 30000 ms"].contains(&s);
        let text = "/collect?host=WEB01&ip=10.0.0.5&other=192.168.9.9&line=Payment gateway call exceeded 30000 ms";
        let r = check(text, &["WEB01".to_string(), "ab".to_string()], &logs);
        assert_eq!(
            marked(&r),
            vec![
                ("WEB01".to_string(), LeakKind::LogValue),
                ("10.0.0.5".to_string(), LeakKind::LogValue),
                ("192.168.9.9".to_string(), LeakKind::Private),
                ("Payment gateway call exceeded 30000 ms".to_string(), LeakKind::LogText),
            ]
        );
        assert!(r.from_logs() && r.any());
        assert_eq!(
            r.segments.iter().map(|s| s.text.as_str()).collect::<String>(),
            text,
            "segments cover the text"
        );
    }

    #[test]
    fn harmless_text_has_no_leaks_and_whole_words_only() {
        let r = check("TimeoutException in HttpClient", &["web01".into()], &|_| false);
        assert!(!r.any() && !r.from_logs());
        assert_eq!(r.segments.len(), 1);
        let r = check("WEB01x and xWEB01 and web01", &["WEB01".into()], &|_| false);
        assert_eq!(marked(&r), vec![("web01".to_string(), LeakKind::LogValue)]);
        assert_eq!(check("", &[], &|_| false).segments.len(), 1);
    }

    #[test]
    fn look_ups_in_the_logs_are_bounded() {
        let calls = Cell::new(0);
        let text: String = (0..40).map(|i| format!("10.0.0.{i} ")).collect();
        check(&text, &[], &|_| {
            calls.set(calls.get() + 1);
            false
        });
        assert_eq!(calls.get(), MAX_PROBES);
    }

    #[test]
    fn overlapping_spans_merge_into_the_strongest_kind() {
        let r = segments(
            "abcdef",
            vec![(1..3, LeakKind::Private), (2..5, LeakKind::LogText), (0..1, LeakKind::Private)],
        );
        assert_eq!(
            r.segments,
            vec![
                Segment {
                    text: "a".into(),
                    leak: Some(LeakKind::Private)
                },
                Segment {
                    text: "bcde".into(),
                    leak: Some(LeakKind::LogText)
                },
                Segment {
                    text: "f".into(),
                    leak: None
                },
            ]
        );
    }
}
