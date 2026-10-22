//! What of a file the engine checks a parser on, and the much smaller part a local model may see (ADR 0032).
//!
//! Both are cut from the file's text at line boundaries and keep each line's number in the file, so failures name
//! real lines. The model only ever gets [`model_sample`]: the start, the middle, the end and a few odd lines (stack
//! traces, continuations, lines of another shape), never the whole file.

use serde::{Deserialize, Serialize};

/// Most lines the engine checks a parser on.
pub const MAX_VERIFY_LINES: usize = 4000;
/// Most characters of the text a model sees.
pub const MAX_MODEL_CHARS: usize = 6000;
/// Longest line given to the model (longer lines are cut).
const MODEL_LINE_CHARS: usize = 300;

/// Lines of a file with their 1-based line numbers in the file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifySample {
    /// The lines joined with `\n` (what the parser reads).
    pub text: String,
    /// The file line number of each line of `text`.
    pub lines: Vec<u32>,
}

impl VerifySample {
    /// The file line number of line `n` (1-based) of `text`.
    pub fn file_line(&self, n: u32) -> u32 {
        n.checked_sub(1).and_then(|i| self.lines.get(i as usize)).copied().unwrap_or(n)
    }

    /// Line `n` (1-based) of `text`.
    pub fn line(&self, n: u32) -> Option<&str> {
        self.text.lines().nth(n.checked_sub(1)? as usize)
    }
}

/// A line's shape: its first characters with digits as `9`, letters as `a`, blanks as ` ` (other characters kept).
pub fn signature(line: &str) -> String {
    line.chars()
        .take(8)
        .map(|c| match c {
            c if c.is_ascii_digit() => '9',
            c if c.is_alphabetic() => 'a',
            c if c.is_whitespace() => ' ',
            c => c,
        })
        .collect()
}

/// Whether a line can start an event: not empty and not indented.
fn starts_entry(line: &str) -> bool {
    !line.trim().is_empty() && !line.starts_with(char::is_whitespace)
}

/// The engine's sample: the whole file when it is short, else its start, middle and end (each piece begins at a
/// line that can start an event).
pub fn verification_sample(content: &str, max_lines: usize) -> VerifySample {
    let lines: Vec<&str> = content.lines().map(|l| l.trim_end_matches('\r')).collect();
    let max_lines = max_lines.max(3);
    let ranges = if lines.len() <= max_lines {
        vec![(0, lines.len())]
    } else {
        let head = max_lines / 2;
        let part = (max_lines - head) / 2;
        let mid = lines.len() / 2 - part / 2;
        let tail = lines.len() - part;
        vec![(0, head), (mid, mid + part), (tail, lines.len())]
    };
    let mut out = VerifySample::default();
    for (start, end) in ranges {
        // A piece starts at an event, not in the middle of a stack trace.
        let start = if start == 0 {
            0
        } else {
            (start..end.min(start + 50)).find(|&i| starts_entry(lines[i])).unwrap_or(start)
        };
        for (i, l) in lines.iter().enumerate().take(end).skip(start) {
            if !out.text.is_empty() {
                out.text.push('\n');
            }
            out.text.push_str(l);
            out.lines.push(i as u32 + 1);
        }
    }
    out
}

/// What a model sees of a file: the start, the middle, the end and a few lines of other shapes, as sections headed
/// `--- lines a-b (where) ---`, at most `max_chars` characters.
pub fn model_sample(content: &str, max_chars: usize) -> String {
    let lines: Vec<&str> = content.lines().map(|l| l.trim_end_matches('\r')).collect();
    if lines.is_empty() {
        return String::new();
    }
    let n = lines.len();
    let mut picked: Vec<(usize, usize, &str)> = Vec::new();
    let add = |from: usize, to: usize, what: &'static str, picked: &mut Vec<(usize, usize, &str)>| {
        let (from, to) = (from.min(n), to.min(n));
        if from < to && !picked.iter().any(|(a, b, _)| from < *b && *a < to) {
            picked.push((from, to, what));
        }
    };
    add(0, 30, "start of the file", &mut picked);
    if n > 60 {
        add(n / 2, n / 2 + 10, "middle", &mut picked);
        add(n - 10, n, "end", &mut picked);
    }
    // Odd lines: a line of each shape that the start does not show (continuations, stack traces, another layout).
    let mut seen: Vec<String> = lines.iter().take(30).map(|l| signature(l)).collect();
    let mut odd = 0;
    for (i, l) in lines.iter().enumerate().skip(30) {
        if odd >= 12 {
            break;
        }
        let sig = signature(l);
        if l.trim().is_empty() || seen.contains(&sig) {
            continue;
        }
        seen.push(sig);
        // With the line before it, so a continuation shows what it continues.
        add(i.saturating_sub(1), i + 1, "a line of another shape", &mut picked);
        odd += 1;
    }
    picked.sort_by_key(|(a, _, _)| *a);
    let mut out = String::new();
    for (from, to, what) in picked {
        let head = format!("--- lines {}-{} ({what}) ---\n", from + 1, to);
        if out.len() + head.len() > max_chars {
            break;
        }
        out.push_str(&head);
        for l in &lines[from..to] {
            let l: String = l.chars().take(MODEL_LINE_CHARS).collect();
            if out.len() + l.len() + 1 > max_chars {
                return out;
            }
            out.push_str(&l);
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(n: usize) -> String {
        let mut s = String::new();
        for i in 1..=n {
            s.push_str(&format!("2026-03-01 10:00:{:02} INFO step {i}\n", i % 60));
            if i % 97 == 0 {
                s.push_str("System.Exception: boom\n   at Job.Run()\n");
            }
        }
        s
    }

    #[test]
    fn short_files_are_checked_whole_and_long_ones_in_three_pieces() {
        let s = verification_sample("a\nb\r\nc", 10);
        assert_eq!((s.text.as_str(), s.lines.as_slice()), ("a\nb\nc", &[1, 2, 3][..]));
        assert_eq!((s.file_line(2), s.line(3)), (2, Some("c")));
        assert_eq!((s.file_line(0), s.line(0)), (0, None));
        let content = file(10_000);
        let total = content.lines().count();
        let s = verification_sample(&content, 400);
        assert!(s.lines.len() <= 400 && s.lines.len() >= 390, "{}", s.lines.len());
        assert_eq!(s.lines[0], 1);
        assert_eq!(*s.lines.last().unwrap() as usize, total);
        assert!(s.lines.windows(2).all(|w| w[0] < w[1]));
        // Every piece starts with an event line.
        for (i, w) in s.lines.windows(2).enumerate() {
            if w[1] != w[0] + 1 {
                assert!(s.line(i as u32 + 2).unwrap().starts_with("2026-"), "{:?}", s.line(i as u32 + 2));
            }
        }
    }

    #[test]
    fn the_model_sees_the_start_middle_end_and_odd_lines_only() {
        let content = file(10_000);
        let m = model_sample(&content, MAX_MODEL_CHARS);
        assert!(m.len() <= MAX_MODEL_CHARS);
        assert!(m.starts_with("--- lines 1-30 (start of the file) ---\n2026-03-01 10:00:01 INFO step 1\n"));
        assert!(m.contains("(middle)") && m.contains("(end)"));
        assert!(m.contains("System.Exception: boom") && m.contains("   at Job.Run()"), "{m}");
        assert!(m.lines().count() < 100);
        assert_eq!(model_sample("", 100), "");
        // Long lines are cut, and the budget holds.
        let long = format!("{}\n", "x".repeat(5000)).repeat(5);
        let m = model_sample(&long, 1000);
        assert!(m.len() <= 1000 && m.lines().all(|l| l.chars().count() <= MODEL_LINE_CHARS));
        assert_eq!(signature("2026-03 x"), "9999-99 ");
    }

    #[test]
    fn at_most_twelve_odd_lines_are_shown_and_a_section_head_that_does_not_fit_is_left_out() {
        let mut content = String::new();
        for i in 0..40 {
            content.push_str(&format!("2026-03-01 10:00:{:02} INFO step {i}\n", i % 60));
        }
        // Twenty lines that each have a shape of their own (another leading symbol), apart by a blank line.
        for c in "!#$%&*+,-./:;<=>?@^_".chars() {
            content.push_str(&format!("{c}{c}{c} x\n\n"));
        }
        for i in 40..240 {
            content.push_str(&format!("2026-03-01 10:01:00 INFO step {i}\n"));
        }
        let m = model_sample(&content, MAX_MODEL_CHARS);
        assert_eq!(m.matches("(a line of another shape)").count(), 12, "{m}");
        // Not even the first section's head fits.
        assert_eq!(model_sample(&content, 10), "");
    }
}
