//! Snippets: a short window of a text around the first match of the search's words, with every match in it marked.
//! Pure and bounded: at most [`MAX_SCAN_CHARS`] of a text are looked at.

use super::dto::SnippetPart;
use crate::words::WordQuery;

/// Characters of a text looked at for matches (the rest of a huge message is not shown anyway).
pub const MAX_SCAN_CHARS: usize = 64 * 1024;
/// Characters of a snippet (without the ellipses).
pub const SNIPPET_CHARS: usize = 200;
/// Characters shown before the first match.
const LEAD_CHARS: usize = SNIPPET_CHARS / 4;
const ELLIPSIS: &str = "…";

/// The strings to mark for a search: its words, word prefixes and phrases (lower-cased, longest first so a longer
/// match wins where two overlap).
pub fn needles(q: &WordQuery) -> Vec<String> {
    let mut all: Vec<String> = q
        .terms
        .iter()
        .chain(&q.prefixes)
        .chain(&q.phrases)
        .filter(|n| !n.is_empty())
        .cloned()
        .collect();
    all.sort_by(|a, b| b.chars().count().cmp(&a.chars().count()).then_with(|| a.cmp(b)));
    all.dedup();
    all
}

/// A text folded for matching: whitespace runs as one space, each character lower-cased to one character (so
/// positions in the folded text are positions in the shown text).
struct Folded {
    /// The characters shown.
    shown: Vec<char>,
    /// The same characters lower-cased.
    lower: String,
    /// Byte offset in `lower` of each character.
    starts: Vec<usize>,
}

fn fold_char(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

impl Folded {
    fn new(text: &str) -> Folded {
        let mut shown = Vec::new();
        let mut last_space = true;
        for c in text.chars() {
            if shown.len() >= MAX_SCAN_CHARS {
                break;
            }
            if c.is_whitespace() || c.is_control() {
                if !last_space {
                    shown.push(' ');
                }
                last_space = true;
            } else {
                shown.push(c);
                last_space = false;
            }
        }
        while shown.last() == Some(&' ') {
            shown.pop();
        }
        let mut lower = String::with_capacity(shown.len());
        let mut starts = Vec::with_capacity(shown.len());
        for &c in &shown {
            starts.push(lower.len());
            lower.push(fold_char(c));
        }
        Folded { shown, lower, starts }
    }

    /// Character index of a byte offset of `lower` (always a character start).
    fn char_at(&self, byte: usize) -> usize {
        self.starts.partition_point(|&s| s < byte)
    }

    /// Every match of the needles as character ranges, sorted and merged.
    fn matches(&self, needles: &[String]) -> Vec<(usize, usize)> {
        let mut found = Vec::new();
        for n in needles {
            let n: String = n.chars().map(fold_char).collect();
            let len = n.chars().count();
            if len == 0 {
                continue;
            }
            for (at, _) in self.lower.match_indices(n.as_str()) {
                let start = self.char_at(at);
                found.push((start, start + len));
            }
        }
        found.sort_unstable();
        let mut merged: Vec<(usize, usize)> = Vec::with_capacity(found.len());
        for (s, e) in found {
            match merged.last_mut() {
                Some(last) if s <= last.1 => last.1 = last.1.max(e),
                _ => merged.push((s, e)),
            }
        }
        merged
    }
}

/// Whether `text` holds any of the needles (ignoring case).
pub fn contains_any(text: &str, needles: &[String]) -> bool {
    !Folded::new(text).matches(needles).is_empty()
}

/// The snippet of `text` for `needles`: [`SNIPPET_CHARS`] characters from a little before the first match (from the
/// start without one), with an ellipsis where text was cut and every match marked.
pub fn snippet(text: &str, needles: &[String]) -> Vec<SnippetPart> {
    let f = Folded::new(text);
    let marks = f.matches(needles);
    let n = f.shown.len();
    let from = marks.first().map_or(0, |m| m.0.saturating_sub(LEAD_CHARS));
    let to = (from + SNIPPET_CHARS).min(n);
    // Show a full window when the match is near the end.
    let from = to.saturating_sub(SNIPPET_CHARS).min(from);
    let text_of = |a: usize, b: usize| f.shown[a..b].iter().collect::<String>();
    let mut parts: Vec<SnippetPart> = Vec::new();
    let mut push = |text: String, mark: bool| {
        if text.is_empty() {
            return;
        }
        match parts.last_mut() {
            Some(last) if last.mark == mark => last.text.push_str(&text),
            _ => parts.push(SnippetPart { text, mark }),
        }
    };
    if from > 0 {
        push(ELLIPSIS.to_string(), false);
    }
    let mut at = from;
    for &(s, e) in &marks {
        let (s, e) = (s.max(from), e.min(to));
        if s >= e {
            continue;
        }
        push(text_of(at, s), false);
        push(text_of(s, e), true);
        at = e;
    }
    push(text_of(at, to), false);
    if to < n {
        push(ELLIPSIS.to_string(), false);
    }
    parts
}
