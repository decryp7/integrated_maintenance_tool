//! Regex help for users: infer a pattern with named groups from example lines,
//! and explain any regex in plain English (from the real parser's syntax tree,
//! so the explanation matches what the engine runs).

use anyhow::{anyhow, Result};
use regex::Regex;
use regex_syntax::ast::{self, Ast};
use serde::Serialize;
use std::collections::HashSet;
use std::sync::OnceLock;

/// Most example lines considered, and longest line, to keep inference fast.
const MAX_LINES: usize = 200;
const MAX_LINE_LEN: usize = 4096;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InferredGroup {
    pub name: String,
    /// number, decimal, guid, ipv4, hex, timestamp, word, token, text
    pub kind: String,
    pub examples: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InferredRegex {
    pub pattern: String,
    pub groups: Vec<InferredGroup>,
    /// Whether the pattern matches each input line.
    pub matches: Vec<bool>,
    pub explanation: Vec<ExplainNode>,
}

/// One line of an explanation outline.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExplainNode {
    pub depth: usize,
    pub text: String,
    /// Byte range in the pattern this node describes.
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Guid,
    Ip,
    Timestamp,
    Hex,
    Quoted,
    Number,
    Word,
    Space,
    Punct,
}

#[derive(Debug, Clone)]
struct Token {
    text: String,
    kind: Kind,
}

fn special_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"(?x)
            (?P<guid>\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b)
          | (?P<ts>\b\d{4}-\d{2}-\d{2}[T\ ]\d{2}:\d{2}:\d{2}(?:[.,]\d+)?(?:Z|[+-]\d{2}:?\d{2})?)
          | (?P<ip>\b(?:\d{1,3}\.){3}\d{1,3}\b)
          | (?P<hex>\b0x[0-9a-fA-F]+\b|\b[0-9a-fA-F]*[a-fA-F][0-9a-fA-F]*\d[0-9a-fA-F]*\b|\b[0-9a-fA-F]*\d[0-9a-fA-F]*[a-fA-F][0-9a-fA-F]*\b)
          | (?P<quoted>"[^"]*"|'[^']*')
        "#,
        )
        .unwrap()
    })
}

fn tokenize(line: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut pos = 0;
    for c in special_re().captures_iter(line) {
        let m = c.get(0).unwrap();
        // Hex needs at least 8 characters to avoid treating words like "cafe1" as ids.
        if c.name("hex").is_some() && m.as_str().len() < 8 {
            continue;
        }
        plain_tokens(&line[pos..m.start()], &mut out);
        let kind = if c.name("guid").is_some() {
            Kind::Guid
        } else if c.name("ts").is_some() {
            Kind::Timestamp
        } else if c.name("ip").is_some() {
            Kind::Ip
        } else if c.name("hex").is_some() {
            Kind::Hex
        } else {
            Kind::Quoted
        };
        out.push(Token {
            text: m.as_str().to_string(),
            kind,
        });
        pos = m.end();
    }
    plain_tokens(&line[pos..], &mut out);
    out
}

fn plain_tokens(s: &str, out: &mut Vec<Token>) {
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let class = |c: char| {
            if c.is_ascii_digit() {
                Kind::Number
            } else if c.is_alphanumeric() || c == '_' {
                Kind::Word
            } else if c.is_whitespace() {
                Kind::Space
            } else {
                Kind::Punct
            }
        };
        let kind = class(c);
        let mut end = i + c.len_utf8();
        if kind != Kind::Punct {
            while let Some(&(j, d)) = chars.peek() {
                let same = class(d) == kind || (kind == Kind::Word && d.is_ascii_digit());
                // Keep decimals together: "1.5", "20,25".
                let decimal = kind == Kind::Number
                    && matches!(d, '.' | ',')
                    && s[j + 1..].chars().next().is_some_and(|n| n.is_ascii_digit())
                    && !s[i..j].contains(['.', ',']);
                if !same && !decimal {
                    break;
                }
                end = j + d.len_utf8();
                chars.next();
            }
        }
        out.push(Token {
            text: s[i..end].to_string(),
            kind,
        });
    }
}

/// Longest common subsequence of token texts.
fn lcs(a: &[String], b: &[Token]) -> Vec<String> {
    let (n, m) = (a.len(), b.len());
    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if a[i] == b[j].text {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let (mut i, mut j, mut out) = (0, 0, Vec::new());
    while i < n && j < m {
        if a[i] == b[j].text {
            out.push(a[i].clone());
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

/// Infers a regex with named groups matching every example line.
pub fn infer(lines: &[String]) -> Result<InferredRegex> {
    let lines: Vec<&str> = lines
        .iter()
        .map(|l| l.trim_end_matches(['\r', '\n']))
        .filter(|l| !l.trim().is_empty())
        .take(MAX_LINES)
        .collect();
    if lines.is_empty() {
        return Err(anyhow!("give at least one example line"));
    }
    if lines.iter().any(|l| l.len() > MAX_LINE_LEN) {
        return Err(anyhow!("example lines are limited to {MAX_LINE_LEN} characters"));
    }
    let tokens: Vec<Vec<Token>> = lines.iter().map(|l| tokenize(l)).collect();
    // A single example: keep words/punctuation, generalise the variable-looking tokens.
    let mut skeleton: Vec<String> = tokens[0]
        .iter()
        .filter(|t| lines.len() > 1 || matches!(t.kind, Kind::Word | Kind::Space | Kind::Punct))
        .map(|t| t.text.clone())
        .collect();
    for t in &tokens[1..] {
        skeleton = lcs(&skeleton, t);
    }

    // Embed the skeleton in each line (leftmost); collect each slot's text per line.
    let slots = skeleton.len() + 1;
    let mut samples: Vec<Vec<String>> = vec![Vec::new(); slots];
    let mut slot_kinds: Vec<Vec<Kind>> = vec![Vec::new(); slots];
    for line in &tokens {
        let mut s = 0;
        let mut current = String::new();
        let mut kinds = Vec::new();
        for tok in line {
            if s < skeleton.len() && tok.text == skeleton[s] {
                samples[s].push(std::mem::take(&mut current));
                slot_kinds[s].append(&mut kinds);
                s += 1;
            } else {
                current.push_str(&tok.text);
                kinds.push(tok.kind);
            }
        }
        samples[s].push(current);
        slot_kinds[s].append(&mut kinds);
        for rest in samples.iter_mut().skip(s + 1) {
            rest.push(String::new());
        }
    }

    let mut pattern = String::new();
    let mut groups = Vec::new();
    let mut used = HashSet::new();
    let mut literal_before = String::new();
    for i in 0..slots {
        let values = &samples[i];
        if values.iter().any(|v| !v.is_empty()) {
            let (kind, re) = classify(values, &slot_kinds[i], i == slots - 1);
            let name = unique_name(&literal_before, &mut used);
            let optional = values.iter().any(|v| v.is_empty());
            pattern.push_str(&format!("(?P<{name}>{re}){}", if optional { "?" } else { "" }));
            let mut examples: Vec<String> = Vec::new();
            for v in values.iter().filter(|v| !v.is_empty()) {
                if !examples.contains(v) && examples.len() < 5 {
                    examples.push(v.clone());
                }
            }
            groups.push(InferredGroup {
                name,
                kind: kind.into(),
                examples,
            });
            literal_before.clear();
        }
        if i < skeleton.len() {
            pattern.push_str(&regex::escape(&skeleton[i]));
            literal_before.push_str(&skeleton[i]);
        }
    }
    let re = crate::user_regex::compile(&pattern, false)?;
    let matches = lines.iter().map(|l| re.is_match(l)).collect();
    let explanation = explain(&pattern)?;
    Ok(InferredRegex {
        pattern,
        groups,
        matches,
        explanation,
    })
}

fn classify(values: &[String], kinds: &[Kind], last: bool) -> (&'static str, &'static str) {
    let present: Vec<&String> = values.iter().filter(|v| !v.is_empty()).collect();
    let all = |re: &str| {
        let r = Regex::new(re).unwrap();
        present.iter().all(|v| r.is_match(v))
    };
    let only = |k: Kind| !kinds.is_empty() && kinds.iter().all(|x| *x == k);
    if only(Kind::Guid) {
        (
            "guid",
            r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}",
        )
    } else if only(Kind::Ip) {
        ("ipv4", r"\d{1,3}(?:\.\d{1,3}){3}")
    } else if only(Kind::Timestamp) {
        (
            "timestamp",
            r"\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(?:[.,]\d+)?(?:Z|[+-]\d{2}:?\d{2})?",
        )
    } else if all(r"^\d+$") {
        ("number", r"\d+")
    } else if all(r"^-?\d+(?:[.,]\d+)?$") {
        ("decimal", r"-?\d+(?:[.,]\d+)?")
    } else if only(Kind::Hex) || all(r"^(?:0x)?[0-9a-fA-F]{8,}$") {
        ("hex", r"(?:0x)?[0-9a-fA-F]+")
    } else if only(Kind::Quoted) {
        ("text", r#""[^"]*"|'[^']*'"#)
    } else if all(r"^[A-Za-z]+$") {
        ("word", "[A-Za-z]+")
    } else if all(r"^\w+$") {
        ("word", r"\w+")
    } else if all(r"^\S+$") {
        ("token", r"\S+")
    } else if last {
        ("text", ".*")
    } else {
        ("text", ".+?")
    }
}

/// A group name from the last word before the value ("order 123" → "order").
fn unique_name(before: &str, used: &mut HashSet<String>) -> String {
    let word: String = before
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .rev()
        .find(|w| w.chars().any(|c| c.is_alphabetic()))
        .unwrap_or("value")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    let mut base = if word.is_empty() || word.starts_with(|c: char| c.is_ascii_digit()) {
        format!("v{word}")
    } else {
        word.to_lowercase()
    };
    if base == "v" {
        base = "value".into();
    }
    let mut name = base.clone();
    let mut n = 2;
    while !used.insert(name.clone()) {
        name = format!("{base}{n}");
        n += 1;
    }
    name
}

/// Explains a regex as an indented outline in plain English.
pub fn explain(pattern: &str) -> Result<Vec<ExplainNode>> {
    if pattern.len() > crate::user_regex::MAX_PATTERN_LEN {
        return Err(anyhow!("the pattern is too long to explain"));
    }
    let ast = ast::parse::Parser::new().parse(pattern).map_err(|e| anyhow!("{}", e.kind()))?;
    let mut out = Vec::new();
    describe(&ast, 0, &mut out);
    Ok(out)
}

fn node(out: &mut Vec<ExplainNode>, depth: usize, span: &ast::Span, text: String) {
    out.push(ExplainNode {
        depth,
        text,
        start: span.start.offset,
        end: span.end.offset,
    });
}

fn describe(ast: &Ast, depth: usize, out: &mut Vec<ExplainNode>) {
    match ast {
        Ast::Empty(_) => {}
        Ast::Flags(f) => node(out, depth, &f.span, format!("set flags: {}", flags_text(&f.flags))),
        Ast::Literal(l) => node(out, depth, &l.span, format!("the text \"{}\"", l.c)),
        Ast::Dot(span) => node(out, depth, span, "any character (except a line break)".into()),
        Ast::Assertion(a) => node(out, depth, &a.span, assertion_text(&a.kind).into()),
        Ast::ClassUnicode(c) => node(
            out,
            depth,
            &c.span,
            format!("{}a Unicode {}", neg(c.negated), unicode_text(&c.kind)),
        ),
        Ast::ClassPerl(c) => node(out, depth, &c.span, perl_text(&c.kind, c.negated).into()),
        Ast::ClassBracketed(c) => node(out, depth, &c.span, bracketed_text(c)),
        Ast::Repetition(r) => {
            node(
                out,
                depth,
                &r.span,
                format!(
                    "{}{}",
                    repetition_text(&r.op.kind),
                    if r.greedy { "" } else { " (as few as possible)" }
                ),
            );
            describe(&r.ast, depth + 1, out);
        }
        Ast::Group(g) => {
            let what = match &g.kind {
                ast::GroupKind::CaptureIndex(i) => format!("capture group #{i}:"),
                ast::GroupKind::CaptureName { name, .. } => format!("capture \"{}\" (group #{}):", name.name, name.index),
                ast::GroupKind::NonCapturing(f) if f.items.is_empty() => "group:".into(),
                ast::GroupKind::NonCapturing(f) => format!("group with flags {}:", flags_text(f)),
            };
            node(out, depth, &g.span, what);
            describe(&g.ast, depth + 1, out);
        }
        Ast::Alternation(a) => {
            node(out, depth, &a.span, "one of these alternatives:".into());
            for (i, alt) in a.asts.iter().enumerate() {
                node(out, depth + 1, alt.span(), format!("option {}:", i + 1));
                describe(alt, depth + 2, out);
            }
        }
        Ast::Concat(c) => {
            // Merge runs of plain literals into one "the text ..." line.
            let mut i = 0;
            while i < c.asts.len() {
                if let Ast::Literal(first) = &c.asts[i] {
                    let mut text = String::new();
                    let start = first.span.start.offset;
                    let mut end = first.span.end.offset;
                    while let Some(Ast::Literal(l)) = c.asts.get(i) {
                        text.push(l.c);
                        end = l.span.end.offset;
                        i += 1;
                    }
                    out.push(ExplainNode {
                        depth,
                        text: format!("the text \"{text}\""),
                        start,
                        end,
                    });
                } else {
                    describe(&c.asts[i], depth, out);
                    i += 1;
                }
            }
        }
    }
}

fn neg(negated: bool) -> &'static str {
    if negated {
        "anything except "
    } else {
        ""
    }
}

fn flags_text(f: &ast::Flags) -> String {
    let mut on = true;
    let mut parts = Vec::new();
    for item in &f.items {
        match &item.kind {
            ast::FlagsItemKind::Negation => on = false,
            ast::FlagsItemKind::Flag(flag) => {
                let name = match flag {
                    ast::Flag::CaseInsensitive => "ignore case",
                    ast::Flag::MultiLine => "^ and $ match at line breaks",
                    ast::Flag::DotMatchesNewLine => ". matches line breaks",
                    ast::Flag::SwapGreed => "lazy by default",
                    ast::Flag::Unicode => "Unicode",
                    ast::Flag::CRLF => "CRLF line endings",
                    ast::Flag::IgnoreWhitespace => "ignore whitespace in the pattern",
                };
                parts.push(format!("{}{name}", if on { "" } else { "not " }));
            }
        }
    }
    parts.join(", ")
}

fn assertion_text(k: &ast::AssertionKind) -> &'static str {
    use ast::AssertionKind::*;
    match k {
        StartLine => "start of the line",
        EndLine => "end of the line",
        StartText => "start of the text",
        EndText => "end of the text",
        WordBoundary => "a word boundary",
        NotWordBoundary => "not a word boundary",
        _ => "a position assertion",
    }
}

fn unicode_text(k: &ast::ClassUnicodeKind) -> String {
    match k {
        ast::ClassUnicodeKind::OneLetter(c) => format!("character of class {c}"),
        ast::ClassUnicodeKind::Named(n) => format!("character of class {n}"),
        ast::ClassUnicodeKind::NamedValue { name, value, .. } => format!("character with {name}={value}"),
    }
}

fn perl_text(k: &ast::ClassPerlKind, negated: bool) -> &'static str {
    match (k, negated) {
        (ast::ClassPerlKind::Digit, false) => "a digit (0-9)",
        (ast::ClassPerlKind::Digit, true) => "any character except a digit",
        (ast::ClassPerlKind::Space, false) => "a whitespace character",
        (ast::ClassPerlKind::Space, true) => "any character except whitespace",
        (ast::ClassPerlKind::Word, false) => "a word character (letter, digit or _)",
        (ast::ClassPerlKind::Word, true) => "any character except a word character",
    }
}

fn bracketed_text(c: &ast::ClassBracketed) -> String {
    fn items(set: &ast::ClassSet, out: &mut Vec<String>) {
        match set {
            ast::ClassSet::Item(item) => item_text(item, out),
            ast::ClassSet::BinaryOp(_) => out.push("a set operation".into()),
        }
    }
    fn item_text(item: &ast::ClassSetItem, out: &mut Vec<String>) {
        match item {
            ast::ClassSetItem::Empty(_) => {}
            ast::ClassSetItem::Literal(l) => out.push(format!("\"{}\"", l.c)),
            ast::ClassSetItem::Range(r) => out.push(format!("\"{}\" to \"{}\"", r.start.c, r.end.c)),
            ast::ClassSetItem::Ascii(a) => out.push(format!("ASCII {:?}", a.kind).to_lowercase()),
            ast::ClassSetItem::Unicode(u) => out.push(unicode_text(&u.kind)),
            ast::ClassSetItem::Perl(p) => out.push(perl_text(&p.kind, p.negated).into()),
            ast::ClassSetItem::Bracketed(b) => out.push(bracketed_text(b)),
            ast::ClassSetItem::Union(u) => u.items.iter().for_each(|i| item_text(i, out)),
        }
    }
    let mut parts = Vec::new();
    items(&c.kind, &mut parts);
    format!(
        "{}one of: {}",
        if c.negated { "any character except " } else { "" },
        parts.join(", ")
    )
}

fn repetition_text(k: &ast::RepetitionKind) -> String {
    match k {
        ast::RepetitionKind::ZeroOrOne => "optionally:".into(),
        ast::RepetitionKind::ZeroOrMore => "zero or more times:".into(),
        ast::RepetitionKind::OneOrMore => "one or more times:".into(),
        ast::RepetitionKind::Range(r) => match r {
            ast::RepetitionRange::Exactly(n) => format!("exactly {n} times:"),
            ast::RepetitionRange::AtLeast(n) => format!("at least {n} times:"),
            ast::RepetitionRange::Bounded(a, b) => format!("between {a} and {b} times:"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn infers_named_typed_groups() {
        let r = infer(&lines(&[
            "Payment gateway call exceeded 5000 ms (attempt 3/3) for order 88213",
            "Payment gateway call exceeded 4870 ms (attempt 1/3) for order 88301",
            "Payment gateway call exceeded 6100 ms (attempt 2/3) for order 9",
        ]))
        .unwrap();
        assert_eq!(
            r.pattern,
            r"Payment gateway call exceeded (?P<exceeded>\d+) ms \(attempt (?P<attempt>\d+)/3\) for order (?P<order>\d+)"
        );
        assert!(r.matches.iter().all(|m| *m));
        let names: Vec<&str> = r.groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["exceeded", "attempt", "order"]);
        assert_eq!(r.groups[0].examples, ["5000", "4870", "6100"]);
        assert!(r.explanation.iter().any(|n| n.text.contains("capture \"order\"")));
    }

    #[test]
    fn recognises_ids_addresses_times_and_text() {
        let r = infer(&lines(&[
            "2026-10-14T10:00:00Z user=ann from 10.0.0.1 id 7f3c9e21-04b8-4c1e-9a55-d2b1e0c6a913 took 1.5s msg \"a b\" tail one",
            "2026-10-14T11:22:33.5+02:00 user=bob from 192.168.1.20 id 0b8a2c11-1111-4c1e-9a55-aaaaaaaaaaaa took 20.25s msg \"c\" tail two words",
        ]))
        .unwrap();
        let kinds: Vec<(&str, &str)> = r.groups.iter().map(|g| (g.name.as_str(), g.kind.as_str())).collect();
        assert!(kinds.contains(&("value", "timestamp")), "{kinds:?}");
        assert!(kinds.contains(&("user", "word")), "{kinds:?}");
        assert!(kinds.contains(&("from", "ipv4")), "{kinds:?}");
        assert!(kinds.contains(&("id", "guid")), "{kinds:?}");
        assert!(kinds.contains(&("took", "decimal")), "{kinds:?}");
        assert!(kinds.contains(&("msg", "text")), "{kinds:?}");
        assert!(kinds.iter().any(|(_, k)| *k == "text"), "{kinds:?}");
        assert!(r.matches.iter().all(|m| *m), "{}", r.pattern);
    }

    #[test]
    fn single_example_and_optional_parts() {
        let one = infer(&lines(&["Order 42 shipped to warehouse 0xDEADBEEF"])).unwrap();
        assert!(
            one.pattern.contains(r"(?P<order>\d+)") && one.pattern.contains("(?P<warehouse>"),
            "{}",
            one.pattern
        );
        assert!(one.matches[0]);
        let opt = infer(&lines(&["done", "done 5 items"])).unwrap();
        assert!(opt.pattern.ends_with(")?"), "{}", opt.pattern);
        assert!(opt.matches.iter().all(|m| *m));
        assert!(infer(&lines(&["", "  "])).is_err());
        assert!(infer(&["x".repeat(MAX_LINE_LEN + 1)]).is_err());
    }

    #[test]
    fn group_names_are_unique_and_valid() {
        let mut used = HashSet::new();
        assert_eq!(unique_name("order ", &mut used), "order");
        assert_eq!(unique_name("order ", &mut used), "order2");
        assert_eq!(unique_name("", &mut used), "value");
        assert_eq!(unique_name("((", &mut used), "value2");
        assert_eq!(unique_name("Größe ", &mut used), "gre");
    }

    #[test]
    fn explains_regex_constructs() {
        let e = explain(r"(?i)^GET\s+(?P<path>/api/\w+)(?:\?[^#]*)?.*?$|\d{2,4}|[a-z0-9_]{3}|\p{L}+|\bx\B|(\d)").unwrap();
        let text: Vec<&str> = e.iter().map(|n| n.text.as_str()).collect();
        for want in [
            "set flags: ignore case",
            "one of these alternatives:",
            "start of the line",
            "the text \"GET\"",
            "one or more times:",
            "a whitespace character",
            "capture \"path\" (group #1):",
            "the text \"/api/\"",
            "a word character (letter, digit or _)",
            "optionally:",
            "any character except one of: \"#\"",
            "zero or more times: (as few as possible)",
            "any character (except a line break)",
            "end of the line",
            "between 2 and 4 times:",
            "a digit (0-9)",
            "one of: \"a\" to \"z\", \"0\" to \"9\", \"_\"",
            "exactly 3 times:",
            "a Unicode character of class L",
            "a word boundary",
            "not a word boundary",
            "capture group #2:",
        ] {
            assert!(text.contains(&want), "missing {want:?} in {text:#?}");
        }
        assert!(e.iter().all(|n| n.start <= n.end));
        assert!(explain("(").is_err());
        assert!(explain(&"a".repeat(crate::user_regex::MAX_PATTERN_LEN + 1)).is_err());
        let more = explain(r"(?-i:x)\D\S\W[^\d][[:alpha:]]a{2,}(?s:.)").unwrap();
        assert!(more.iter().any(|n| n.text.contains("not ignore case")));
        assert!(more.iter().any(|n| n.text == "at least 2 times:"));
        assert!(more.iter().any(|n| n.text.contains(". matches line breaks")));
    }
}
