//! Words of the search box (smart search): how text is split into words for the full-text index and how a
//! search is read, as pure functions so the index (the store's Tantivy tokenizer) and the in-memory oracle
//! agree exactly.
//!
//! - A *run* is a stretch of letters and digits; everything else separates runs. A run is a word, and when
//!   it has inner boundaries (`camelCase`, `HTTPServer`, `v2beta`) its parts are words too:
//!   `TimeoutException` gives `timeoutexception`, `timeout`, `exception`. Words are lower-cased.
//! - A search is every word of its text (all required). A run of the search with parts asks for its parts,
//!   so `TimeoutException` also finds `Timeout.Exception`; a run without parts asks for itself.
//! - The last run is a prefix unless the text ends in a space or a quote: `timeout exc` finds
//!   `TimeoutException`.
//! - Quoted phrases (`"builder failed"`) must also appear, ignoring case and separators: any run of
//!   non-alphanumeric characters (spaces, `.`, `_`, `-`, `/`, `\`, `::`) and every `camelCase` boundary count as
//!   the same gap, so `"graphic builder"` finds `Graphic.Builder`, `GraphicBuilder` and `graphic_builder`
//!   ([`phrase_in`]).
//!
//! Runs longer than [`MAX_RUN`] characters (hashes, base64) are not words; their parts still are.

use std::collections::HashSet;

/// Longest run that is a word (in characters).
pub const MAX_RUN: usize = 64;

/// Words of `text`, in order (with repeats): every run and, for a run with inner boundaries, its parts.
pub fn words(text: &str, out: &mut dyn FnMut(String)) {
    for run in runs(text) {
        let parts = parts(run);
        if run.chars().count() <= MAX_RUN {
            out(run.to_lowercase());
        }
        if parts.len() > 1 {
            for p in parts {
                if p.chars().count() <= MAX_RUN {
                    out(p.to_lowercase());
                }
            }
        }
    }
}

/// The runs of letters and digits.
fn runs(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !c.is_alphanumeric()).filter(|r| !r.is_empty())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Lower,
    Upper,
    Digit,
    Other,
}

fn class(c: char) -> Class {
    if c.is_numeric() {
        Class::Digit
    } else if c.is_uppercase() {
        Class::Upper
    } else if c.is_lowercase() {
        Class::Lower
    } else {
        Class::Other
    }
}

/// A run split at its inner boundaries: lower→upper (`aB`), the last capital of a capital run before a
/// lower-case letter (`HTTPServer` → `HTTP`, `Server`), and letter↔digit.
fn parts(run: &str) -> Vec<&str> {
    let chars: Vec<(usize, char)> = run.char_indices().collect();
    let mut out = Vec::new();
    let mut start = 0;
    for i in 1..chars.len() {
        let (prev, cur) = (class(chars[i - 1].1), class(chars[i].1));
        let next = chars.get(i + 1).map(|c| class(c.1));
        let boundary = match (prev, cur) {
            (Class::Lower, Class::Upper) => true,
            (Class::Upper, Class::Upper) => next == Some(Class::Lower),
            (Class::Digit, Class::Lower | Class::Upper | Class::Other) | (Class::Lower | Class::Upper | Class::Other, Class::Digit) => true,
            _ => false,
        };
        if boundary {
            out.push(&run[start..chars[i].0]);
            start = chars[i].0;
        }
    }
    out.push(&run[start..]);
    out
}

/// A search read as words: what an event must contain.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WordQuery {
    /// Words that must appear (lower-cased, deduplicated).
    pub terms: Vec<String>,
    /// Words that must start a word (the last run's, lower-cased).
    pub prefixes: Vec<String>,
    /// Phrases that must appear as written, ignoring case (lower-cased).
    pub phrases: Vec<String>,
}

impl WordQuery {
    /// Reads a search box text. No words (only punctuation) gives an empty query.
    pub fn parse(text: &str) -> WordQuery {
        let (words_text, phrases) = split_phrases(text);
        let trimmed = text.trim_start();
        let open_end = !trimmed.is_empty() && !trimmed.ends_with(|c: char| c.is_whitespace() || c == '"');
        let all: Vec<&str> = runs(&words_text).collect();
        let mut terms = Vec::new();
        let mut prefixes = Vec::new();
        for (i, run) in all.iter().enumerate() {
            let parts = parts(run);
            let wanted: Vec<String> = if parts.len() > 1 {
                parts.iter().map(|p| p.to_lowercase()).collect()
            } else {
                vec![run.to_lowercase()]
            };
            let last = i + 1 == all.len();
            for (j, w) in wanted.iter().enumerate() {
                if w.chars().count() > MAX_RUN {
                    continue;
                }
                let list = if last && open_end && j + 1 == wanted.len() {
                    &mut prefixes
                } else {
                    &mut terms
                };
                if !list.contains(w) {
                    list.push(w.clone());
                }
            }
        }
        WordQuery { terms, prefixes, phrases }
    }

    /// Whether the search has no words (then it cannot be a word search).
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty() && self.prefixes.is_empty()
    }

    /// Whether texts (an event's message, exception and template) hold every word and phrase.
    pub fn matches(&self, texts: &[&str]) -> bool {
        if self.is_empty() {
            return false;
        }
        let mut have = HashSet::new();
        for t in texts {
            words(t, &mut |w| {
                have.insert(w);
            });
        }
        self.terms.iter().all(|t| have.contains(t))
            && self.prefixes.iter().all(|p| have.iter().any(|w| w.starts_with(p.as_str())))
            && (self.phrases.is_empty() || { self.phrases.iter().all(|p| texts.iter().any(|t| phrase_in(p, t))) })
    }
}

/// A text as its words between single spaces and at both ends: its parts (`CompanyA.Graphic` gives
/// ` company a graphic `) or its whole runs (` companya graphic `).
fn gapped(text: &str, parts_form: bool) -> String {
    let mut out = String::from(" ");
    for run in runs(text) {
        if parts_form {
            for p in parts(run) {
                out.push_str(&p.to_lowercase());
                out.push(' ');
            }
        } else {
            out.push_str(&run.to_lowercase());
            out.push(' ');
        }
    }
    out
}

/// Whether `text` holds `phrase` as consecutive words, ignoring case and what separates them: any run of
/// non-alphanumeric characters and every `camelCase`/digit boundary is one gap. `graphic builder` is in
/// `CompanyA.Graphic.Builder.exe`, `GraphicBuilder` and `graphic_builder`; a phrase written as one run
/// (`GraphicBuilder`) is also found as that run. A phrase without words (only punctuation) is a plain
/// case-insensitive substring.
pub fn phrase_in(phrase: &str, text: &str) -> bool {
    let wanted = gapped(phrase, true);
    if wanted.len() <= 1 {
        return text.to_lowercase().contains(&phrase.to_lowercase());
    }
    gapped(text, true).contains(&wanted) || gapped(text, false).contains(&gapped(phrase, false))
}

/// Phrases (`"..."`) of a search and the text with the quotes removed (the phrases' words stay, so a word
/// search finds a superset the phrases are then checked on). An unbalanced quote is just a character.
pub fn split_phrases(text: &str) -> (String, Vec<String>) {
    let mut words = String::with_capacity(text.len());
    let mut phrases = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('"') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('"') else { break };
        words.push_str(&rest[..open]);
        words.push(' ');
        let phrase = after[..close].trim();
        words.push_str(phrase);
        words.push(' ');
        if !phrase.is_empty() {
            phrases.push(phrase.to_lowercase());
        }
        rest = &after[close + 1..];
    }
    words.push_str(&rest.replace('"', " "));
    (words.trim().to_string(), phrases)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(text: &str) -> Vec<String> {
        let mut v = Vec::new();
        words(text, &mut |w| v.push(w));
        v
    }

    #[test]
    fn runs_split_at_case_digit_and_punctuation() {
        assert_eq!(
            all("System.TimeoutException"),
            ["system", "timeoutexception", "timeout", "exception"]
        );
        assert_eq!(
            all("HTTPServer v2beta"),
            ["httpserver", "http", "server", "v2beta", "v", "2", "beta"]
        );
        assert_eq!(all("snake_case/path\\to"), ["snake", "case", "path", "to"]);
        assert_eq!(all("ALLCAPS lower 42"), ["allcaps", "lower", "42"]);
        assert_eq!(all("Grüße Ärger"), ["grüße", "ärger"]);
        // Letters without case (CJK) are neither upper nor lower: they split from digits only.
        assert_eq!(all("日本語"), ["日本語"]);
        assert_eq!(all("日本2語"), ["日本2語", "日本", "2", "語"]);
        assert!(all("...  --").is_empty());
        let long = "a".repeat(MAX_RUN + 1);
        assert!(all(&long).is_empty(), "an over-long run is not a word");
        assert_eq!(all(&format!("{long}X")), ["x"], "its parts still are");
    }

    #[test]
    fn a_search_asks_for_words_and_a_last_prefix() {
        let q = WordQuery::parse("graphic BUILDER fail");
        assert_eq!(
            (q.terms.as_slice(), q.prefixes.as_slice()),
            (&["graphic".to_string(), "builder".into()][..], &["fail".to_string()][..])
        );
        assert!(WordQuery::parse("fail ").prefixes.is_empty(), "a trailing space ends the word");
        let q = WordQuery::parse("TimeoutExc");
        assert_eq!((q.terms, q.prefixes), (vec!["timeout".to_string()], vec!["exc".to_string()]));
        let q = WordQuery::parse("\"Builder Failed\"");
        assert_eq!(
            (q.terms.len(), q.prefixes.len(), q.phrases),
            (2, 0, vec!["builder failed".to_string()])
        );
        assert!(WordQuery::parse(" -- ").is_empty());
        assert_eq!(WordQuery::parse("a a").terms, ["a"], "deduplicated");
    }

    #[test]
    fn matching_follows_the_words() {
        let q = WordQuery::parse("timeout exc");
        assert!(q.matches(&["System.TimeoutException at Foo"]));
        assert!(q.matches(&["timeout", "SomeException"]), "across message and exception");
        assert!(!q.matches(&["timeouts happen"]), "timeout is a whole word");
        assert!(WordQuery::parse("TimeoutException").matches(&["Timeout.Exception"]));
        assert!(WordQuery::parse("timeoutexception").matches(&["TimeoutException"]));
        assert!(
            !WordQuery::parse("imeout").matches(&["TimeoutException"]),
            "a partial match, not a word"
        );
        let q = WordQuery::parse("\"builder failed\" x");
        assert!(q.matches(&["GRAPHIC Builder Failed: x1"]));
        assert!(!q.matches(&["failed builder x"]), "the phrase is checked");
        assert!(!WordQuery::parse("--").matches(&["--"]), "an empty query matches nothing");
    }

    #[test]
    fn phrases_ignore_separators_case_and_camel_case() {
        let hit = |p: &str, t: &str| phrase_in(p, t);
        for t in [
            "CompanyA.Graphic.Builder.exe failed",
            "GraphicBuilder",
            "graphic_builder",
            "Graphic-Builder",
            "C:\\tools\\Graphic\\Builder.exe",
            "Graphic::Builder",
            "GRAPHIC   BUILDER",
        ] {
            assert!(hit("graphic builder", t), "{t}");
        }
        assert!(
            hit("GraphicBuilder", "the graphic.builder ran"),
            "a camel phrase finds separated words"
        );
        assert!(hit("graphicbuilder", "GraphicBuilder"), "a one-run phrase finds the run");
        assert!(!hit("graphic builder", "builder graphic"), "order matters");
        assert!(!hit("graphic builder", "graphic the builder"), "the words are consecutive");
        assert!(!hit("graphic build", "graphic builder"), "whole words");
        assert!(hit("::", "a::b") && !hit("--", "a b"), "no words: a substring");
        assert!(WordQuery::parse("\"graphic builder\"").matches(&["CompanyA.Graphic.Builder.exe"]));
    }

    #[test]
    fn phrases_are_split_from_words() {
        assert_eq!(split_phrases("a b"), ("a b".to_string(), vec![]));
        let (w, p) = split_phrases("x \"Builder  Failed\" y \"\"");
        assert_eq!(p, ["builder  failed"]);
        assert!(w.contains("Builder  Failed") && w.starts_with('x') && w.ends_with('y'), "{w}");
        assert_eq!(split_phrases("unbalanced \"quote"), ("unbalanced  quote".to_string(), vec![]));
    }
}
