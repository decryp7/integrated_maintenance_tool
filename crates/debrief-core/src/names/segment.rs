//! Word segmentation of run-together names: `graphicbuilder` becomes `graphic builder`.
//!
//! A Viterbi search over the letters of one token picks the split of least cost. A word costs more the less common it
//! is: words of the workspace's own [`Vocabulary`] (how the logs spell things elsewhere) are cheaper than the
//! embedded list, so splits follow the logs. A token that does not split into mostly known words is kept whole
//! (`svchost` stays `svchost`). Pure, deterministic, bounded by [`MAX_TOKEN_CHARS`] and [`MAX_SEGMENTS`].

use std::collections::HashMap;
use std::sync::OnceLock;

/// Longest token that is split (longer ones are kept whole).
pub const MAX_TOKEN_CHARS: usize = 64;
/// Most words a token is split into (more: kept whole).
pub const MAX_SEGMENTS: usize = 8;
/// Longest word looked up.
const MAX_WORD_CHARS: usize = 24;
const UNKNOWN_CHAR_COST: f64 = 14.0;
const SEGMENT_COST: f64 = 0.5;
/// Share of the token's letters that must be in known words for a split to be used.
const MIN_KNOWN_SHARE: f64 = 0.85;
/// Fewest letters of a token worth splitting.
const MIN_SPLIT_CHARS: usize = 4;

/// The compact embedded word list (authored for Debrief: common English plus software words, most common first).
static BUILTIN: &str = include_str!("words.txt");

fn builtin() -> &'static HashMap<&'static str, usize> {
    static MAP: OnceLock<HashMap<&'static str, usize>> = OnceLock::new();
    MAP.get_or_init(|| BUILTIN.lines().enumerate().map(|(i, w)| (w, i)).collect())
}

/// What the segmenter knows: the embedded list plus the words the workspace's logs use.
#[derive(Debug, Clone, Default)]
pub struct Vocabulary {
    workspace: HashMap<String, u64>,
}

impl Vocabulary {
    /// Only the embedded list.
    pub fn builtin() -> Vocabulary {
        Vocabulary::default()
    }

    /// Counts `word` (lower-cased) `n` times as used by the workspace. Words under two letters are ignored.
    pub fn add(&mut self, word: &str, n: u64) {
        let w = word.to_lowercase();
        let len = w.chars().count();
        if (2..=MAX_WORD_CHARS).contains(&len) {
            *self.workspace.entry(w).or_default() += n;
        }
    }

    /// Whether `word` is known (workspace or embedded).
    pub fn knows(&self, word: &str) -> bool {
        self.cost(word).is_some()
    }

    /// Whether `word` is in the embedded list of common words (the workspace's own words do not count).
    pub fn is_common(&self, word: &str) -> bool {
        builtin().contains_key(word)
    }

    /// The cost of `word`, if known: workspace words are cheaper the more they occur.
    fn cost(&self, word: &str) -> Option<f64> {
        if let Some(n) = self.workspace.get(word) {
            return Some((3.0 - 0.4 * (*n as f64 + 1.0).ln()).max(0.5));
        }
        builtin().get(word).map(|rank| 4.0 + 0.9 * (*rank as f64 + 1.0).ln())
    }

    /// Number of workspace words.
    pub fn len(&self) -> usize {
        self.workspace.len()
    }

    pub fn is_empty(&self) -> bool {
        self.workspace.is_empty()
    }
}

/// Splits a lower-case alphabetic token into words; the token itself when it does not split well.
pub fn segment(token: &str, vocab: &Vocabulary) -> Vec<String> {
    let chars: Vec<char> = token.chars().flat_map(char::to_lowercase).collect();
    let n = chars.len();
    let all: String = chars.iter().collect();
    if !(MIN_SPLIT_CHARS..=MAX_TOKEN_CHARS).contains(&n) || vocab.knows(&all) {
        return vec![all];
    }
    // best[i]: least cost of the first i letters; back[i]: (start of the last word, whether it is known).
    let mut best = vec![f64::INFINITY; n + 1];
    let mut back = vec![(0usize, false); n + 1];
    best[0] = 0.0;
    for i in 1..=n {
        best[i] = best[i - 1] + UNKNOWN_CHAR_COST;
        back[i] = (i - 1, false);
        for l in 2..=i.min(MAX_WORD_CHARS) {
            let w: String = chars[i - l..i].iter().collect();
            if let Some(c) = vocab.cost(&w) {
                let total = best[i - l] + c + SEGMENT_COST;
                if total < best[i] {
                    best[i] = total;
                    back[i] = (i - l, true);
                }
            }
        }
    }
    let mut parts: Vec<(usize, usize, bool)> = Vec::new();
    let mut i = n;
    while i > 0 {
        let (start, known) = back[i];
        parts.push((start, i, known));
        i = start;
    }
    parts.reverse();
    let known_chars: usize = parts.iter().filter(|p| p.2).map(|p| p.1 - p.0).sum();
    if (known_chars as f64) < MIN_KNOWN_SHARE * n as f64 {
        return vec![all];
    }
    // Neighbouring unknown letters form one word.
    let mut out: Vec<String> = Vec::new();
    let mut prev_unknown = false;
    for (s, e, known) in parts {
        let w: String = chars[s..e].iter().collect();
        match out.last_mut() {
            Some(last) if !known && prev_unknown => last.push_str(&w),
            _ => out.push(w),
        }
        prev_unknown = !known;
    }
    if out.len() > MAX_SEGMENTS {
        return vec![all];
    }
    out
}

/// The pieces of a raw name part: split at separators, at lower-to-upper case changes (`GraphicBuilder`), after an
/// acronym (`XMLParser` is `XML`, `Parser`) and between letters and digits; each lower-cased, alphabetic pieces of
/// four or more letters then segmented. At most `2 * MAX_TOKEN_CHARS` characters are read.
pub fn pieces(raw: &str, vocab: &Vocabulary) -> Vec<String> {
    let mut out = Vec::new();
    for lower in split_raw(raw) {
        if lower.chars().all(char::is_alphabetic) {
            out.extend(segment(&lower, vocab));
        } else {
            out.push(lower);
        }
    }
    out
}

/// The lower-case pieces of `raw` split at separators, case changes, acronyms and letter/digit changes, not yet segmented.
pub fn split_raw(raw: &str) -> Vec<String> {
    let chars: Vec<char> = raw.chars().take(MAX_TOKEN_CHARS * 2).collect();
    let mut raw_pieces: Vec<String> = Vec::new();
    let mut cur = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if !c.is_alphanumeric() {
            flush(&mut cur, &mut raw_pieces);
            continue;
        }
        if let Some(p) = cur.chars().last() {
            let next_lower = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
            let boundary = (p.is_lowercase() && c.is_uppercase())
                || (p.is_uppercase() && c.is_uppercase() && next_lower)
                || (p.is_alphabetic() != c.is_alphabetic());
            if boundary {
                flush(&mut cur, &mut raw_pieces);
            }
        }
        cur.push(c);
    }
    flush(&mut cur, &mut raw_pieces);
    raw_pieces.into_iter().map(|p| p.chars().flat_map(char::to_lowercase).collect()).collect()
}

fn flush(cur: &mut String, out: &mut Vec<String>) {
    if !cur.is_empty() {
        out.push(std::mem::take(cur));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(s: &str) -> String {
        segment(s, &Vocabulary::builtin()).join(" ")
    }

    #[test]
    fn splits_run_together_words() {
        assert_eq!(seg("graphicbuilder"), "graphic builder");
        assert_eq!(seg("imageeditor"), "image editor");
        assert_eq!(seg("printspooler"), "print spooler");
    }

    #[test]
    fn known_words_and_short_tokens_stay_whole() {
        assert_eq!(seg("therapist"), "therapist");
        assert_eq!(seg("exe"), "exe");
        assert_eq!(seg("builder"), "builder");
    }

    #[test]
    fn unknown_tokens_stay_whole() {
        assert_eq!(seg("svchost"), "svchost");
        assert_eq!(seg("zzqxkj"), "zzqxkj");
    }

    #[test]
    fn workspace_vocabulary_wins() {
        // Without the workspace word "thera", "therapist" is one known word; a made-up product word is learned.
        let mut v = Vocabulary::builtin();
        assert_eq!(segment("gfxhost", &v).join(" "), "gfxhost");
        v.add("gfx", 50);
        assert_eq!(segment("gfxhost", &v).join(" "), "gfx host");
        assert_eq!(v.len(), 1);
        assert!(!v.is_empty());
    }

    #[test]
    fn ambiguous_split_prefers_the_common_reading() {
        // "therapistfinder": "therapist" + "finder" beats "the rapist ..." because "rapist" is unknown.
        assert_eq!(seg("therapistfinder"), "therapist finder");
        let mut v = Vocabulary::builtin();
        v.add("rapist", 1000);
        // A known whole word is never split.
        assert_eq!(segment("therapist", &v).join(" "), "therapist");
    }

    #[test]
    fn pieces_handle_case_acronyms_digits_and_separators() {
        let v = Vocabulary::builtin();
        assert_eq!(pieces("XMLParser", &v), vec!["xml", "parser"]);
        assert_eq!(pieces("GraphicBuilder", &v), vec!["graphic", "builder"]);
        assert_eq!(pieces("graphic-builder_2", &v), vec!["graphic", "builder", "2"]);
        assert_eq!(pieces("log4j", &v), vec!["log", "4", "j"]);
        assert_eq!(pieces("", &v), Vec::<String>::new());
    }

    #[test]
    fn non_english_letters_do_not_panic() {
        let v = Vocabulary::builtin();
        assert_eq!(segment("müllerservice", &v).join(" "), "müllerservice");
        let mut v = v;
        v.add("Müller", 3);
        assert_eq!(segment("müllerservice", &v).join(" "), "müller service");
        let v = &v;
        assert_eq!(segment("日本語ツール", v).join(""), "日本語ツール");
        assert_eq!(pieces("Ünïcode-Tøol", v).len(), 2);
    }

    #[test]
    fn long_tokens_are_bounded() {
        let long = "graphic".repeat(30);
        assert_eq!(segment(&long, &Vocabulary::builtin()), vec![long.clone()]);
        let many = "ab".repeat(10);
        let mut v = Vocabulary::builtin();
        v.add("ab", 100);
        // Ten words are over MAX_SEGMENTS: kept whole.
        assert_eq!(segment(&many, &v).len(), 1);
        assert!(pieces(&long, &v).len() < 200);
    }
}
