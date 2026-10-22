//! Known issues (ADR 0039): knowledge files (ADR 0029) can declare errors the user already understands, with their fix.
//! The engine reads the declarations, matches them against the open data and returns each match with evidence
//! (the number of events and sample event ids). No AI is involved in the rules; an AI suggestion is only ever a
//! [`Matcher`] that the engine verifies the same way.
//!
//! A declaration is a fenced block in the Markdown file:
//!
//! ````text
//! ```known-issue
//! title: Connection pool exhausted
//! pattern: (?i)timeout expired.*max pool size
//! fix: Raise `Max Pool Size` in the connection string.
//!   Then restart the service.
//! ```
//! ````
//!
//! Keys: `title` (required), `fix` (required), and exactly one of `pattern` (a regular expression matched against
//! the message and the exception) or `query` (a query-language filter); `id` is optional (stable name for notes
//! and reports). A line indented by two or more spaces continues the previous value. Everything is bounded.

use crate::ports::{Store, StoreResult};
use crate::ql::QlResult;
use crate::query::Query;
use crate::user_regex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Most declarations read from one knowledge file.
pub const MAX_ISSUES_PER_FILE: usize = 100;
/// Most declarations the engine evaluates in one call (the rest is reported as `truncated`).
pub const MAX_ISSUES_CHECKED: usize = 200;
/// Sample event ids returned per match.
pub const MAX_SAMPLES: usize = 5;
const MAX_TITLE: usize = 120;
const MAX_FIX: usize = 2_000;
const MAX_QUERY: usize = 2_000;
const MAX_ID: usize = 60;

/// How an issue is recognised in the data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Matcher {
    /// A regular expression over the message and the exception text.
    Pattern { pattern: String },
    /// A query-language filter.
    Query { ql: String },
}

impl Matcher {
    /// The query-language filter that selects the matching events (a pattern is quoted into one).
    pub fn to_ql(&self) -> String {
        match self {
            Matcher::Query { ql } => ql.trim().to_string(),
            Matcher::Pattern { pattern } => {
                let q = pattern.replace('\\', "\\\\").replace('"', "\\\"");
                format!("@message ~ \"{q}\" or @exception ~ \"{q}\"")
            }
        }
    }
}

/// One declared (or AI-suggested and verified) known issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownIssue {
    pub id: String,
    pub title: String,
    pub fix: String,
    pub matcher: Matcher,
    /// The knowledge file that declares it.
    pub file: String,
}

/// Who found the match: the user's declared rule, or an AI suggestion the engine verified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Origin {
    Rule,
    Ai,
}

/// A known issue present in the open data, with the evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueMatch {
    pub issue: KnownIssue,
    pub origin: Origin,
    /// The filter that selects the events (the UI opens it as the view; reports cite it).
    pub query: String,
    pub count: usize,
    /// Up to [`MAX_SAMPLES`] events that show it.
    pub sample_ids: Vec<u32>,
}

/// The result of matching: what is present, what could not be read or checked.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueReport {
    pub matches: Vec<IssueMatch>,
    /// Declarations evaluated.
    pub checked: usize,
    /// Declarations skipped because more than [`MAX_ISSUES_CHECKED`] exist.
    pub truncated: usize,
    /// What the user should fix in their knowledge files (each names the file and the issue).
    pub warnings: Vec<String>,
}

/// A knowledge file as the matcher reads it.
#[derive(Debug, Clone, Copy)]
pub struct KnowledgeText<'a> {
    pub name: &'a str,
    pub text: &'a str,
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').chars().take(MAX_ID).collect()
}

fn is_fence(line: &str) -> Option<(&str, &str)> {
    let t = line.trim_start();
    let ticks = if t.starts_with("```") {
        "```"
    } else if t.starts_with("~~~") {
        "~~~"
    } else {
        return None;
    };
    Some((ticks, t[3..].trim()))
}

/// Reads the declarations of one knowledge file. Damaged declarations are skipped with a warning that says what to
/// fix; this never fails and never panics on any text.
pub fn parse_known_issues(file: &str, markdown: &str) -> (Vec<KnownIssue>, Vec<String>) {
    let mut issues = Vec::new();
    let mut warnings = Vec::new();
    let mut lines = markdown.lines();
    let mut number = 0usize;
    while let Some(line) = lines.next() {
        let Some((ticks, info)) = is_fence(line) else { continue };
        let is_issue = info.eq_ignore_ascii_case("known-issue");
        let mut body = Vec::new();
        for l in lines.by_ref() {
            if is_fence(l).is_some_and(|(t, rest)| t == ticks && rest.is_empty()) {
                break;
            }
            if is_issue {
                body.push(l);
            }
        }
        if !is_issue {
            continue;
        }
        number += 1;
        if number > MAX_ISSUES_PER_FILE {
            warnings.push(format!("{file}: only the first {MAX_ISSUES_PER_FILE} known issues are read"));
            break;
        }
        match parse_block(file, number, &body) {
            Ok(i) => issues.push(i),
            Err(e) => warnings.push(format!("{file}, known issue {number}: {e}")),
        }
    }
    // Ids are unique within what is returned: a repeated one gets a suffix.
    let mut seen = BTreeSet::new();
    for i in &mut issues {
        let base = i.id.clone();
        let mut n = 1;
        while !seen.insert(i.id.clone()) {
            n += 1;
            i.id = format!("{base}-{n}");
        }
    }
    (issues, warnings)
}

fn parse_block(file: &str, number: usize, body: &[&str]) -> Result<KnownIssue, String> {
    let mut fields: Vec<(String, String)> = Vec::new();
    for line in body {
        if line.trim().is_empty() {
            continue;
        }
        if line.starts_with("  ") || line.starts_with('\t') {
            match fields.last_mut() {
                Some((_, v)) => {
                    v.push('\n');
                    v.push_str(line.trim());
                }
                None => return Err("a continuation line comes before any key".into()),
            }
            continue;
        }
        let Some((k, v)) = line.split_once(':') else {
            return Err(format!("`{}` is not `key: value`", line.trim().chars().take(40).collect::<String>()));
        };
        fields.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
    }
    let get = |k: &str| fields.iter().find(|(n, _)| n == k).map(|(_, v)| v.trim().to_string()).filter(|v| !v.is_empty());
    if let Some((k, _)) = fields.iter().find(|(k, _)| !["title", "fix", "pattern", "query", "id"].contains(&k.as_str())) {
        return Err(format!("unknown key `{k}` (use title, pattern or query, fix, id)"));
    }
    let title = get("title").ok_or("it needs a `title:`")?;
    let fix = get("fix").ok_or("it needs a `fix:`")?;
    if title.chars().count() > MAX_TITLE {
        return Err(format!("the title is longer than {MAX_TITLE} characters"));
    }
    if fix.chars().count() > MAX_FIX {
        return Err(format!("the fix is longer than {MAX_FIX} characters"));
    }
    let matcher = match (get("pattern"), get("query")) {
        (Some(_), Some(_)) => return Err("give either `pattern:` or `query:`, not both".into()),
        (None, None) => return Err("it needs a `pattern:` or a `query:`".into()),
        (Some(p), None) => {
            user_regex::compile(&p, false).map_err(|e| format!("the pattern is not a valid regex: {e}"))?;
            Matcher::Pattern { pattern: p }
        }
        (None, Some(q)) => {
            if q.len() > MAX_QUERY {
                return Err(format!("the query is longer than {MAX_QUERY} characters"));
            }
            Matcher::Query { ql: q }
        }
    };
    let id = get("id").map(|i| slug(&i)).filter(|i| !i.is_empty()).unwrap_or_else(|| {
        let s = slug(&title);
        if s.is_empty() {
            format!("issue-{number}")
        } else {
            s
        }
    });
    Ok(KnownIssue {
        id,
        title,
        fix,
        matcher,
        file: file.to_string(),
    })
}

/// Counts the events a query selects and keeps a few ids; `None` when the query is invalid or selects nothing.
/// The outcome of a store failure is an `Err` the caller reports.
pub fn evidence(store: &dyn Store, ql: &str) -> StoreResult<Result<(usize, Vec<u32>), String>> {
    if let Some(e) = store.check_query(ql)? {
        return Ok(Err(e.message));
    }
    let req = crate::api::QueryRequest {
        query: Query {
            ql: ql.to_string(),
            ..Query::default()
        },
    };
    match store.run_query(&req) {
        Ok(QlResult::Events { ids, .. }) => Ok(Ok((ids.len(), ids.into_iter().take(MAX_SAMPLES).collect()))),
        Ok(QlResult::Table { rows, .. }) => Ok(Ok((rows.len(), Vec::new()))),
        Err(crate::ports::StoreError::Query(e)) => Ok(Err(e.message)),
        Err(e) => Err(e),
    }
}

/// Matches the declarations of `files` against the open data, most frequent first. A declaration whose query does
/// not fit the data (unknown field, bad syntax) is reported in `warnings` and skipped; a store failure is returned.
pub fn match_known_issues(store: &dyn Store, files: &[KnowledgeText<'_>]) -> StoreResult<IssueReport> {
    let mut report = IssueReport::default();
    let mut all = Vec::new();
    for f in files {
        let (issues, warnings) = parse_known_issues(f.name, f.text);
        report.warnings.extend(warnings);
        all.extend(issues);
    }
    if all.len() > MAX_ISSUES_CHECKED {
        report.truncated = all.len() - MAX_ISSUES_CHECKED;
        all.truncate(MAX_ISSUES_CHECKED);
    }
    for issue in all {
        let query = issue.matcher.to_ql();
        report.checked += 1;
        match evidence(store, &query)? {
            Ok((0, _)) => {}
            Ok((count, sample_ids)) => report.matches.push(IssueMatch {
                issue,
                origin: Origin::Rule,
                query,
                count,
                sample_ids,
            }),
            Err(e) => report.warnings.push(format!("{}, known issue \"{}\": the query does not work here: {e}", issue.file, issue.title)),
        }
    }
    report.matches.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.issue.id.cmp(&b.issue.id)));
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "# Notes\n\nSome text.\n\n```known-issue\ntitle: Pool exhausted\npattern: (?i)timeout expired\nfix: Raise Max Pool Size.\n  Restart.\n```\n\n```rust\nfn main() {}\n```\n\n~~~Known-Issue\nid: Disk Full!\ntitle: Disk full\nquery: level >= error and message contains \"disk\"\nfix: Free space.\n~~~\n";

    #[test]
    fn reads_declarations_and_ignores_other_fences() {
        let (issues, warnings) = parse_known_issues("ops.md", FILE);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0].id, "pool-exhausted");
        assert_eq!(issues[0].fix, "Raise Max Pool Size.\nRestart.");
        assert_eq!(issues[0].file, "ops.md");
        assert_eq!(issues[1].id, "disk-full");
        assert!(matches!(&issues[1].matcher, Matcher::Query { ql } if ql.starts_with("level >= error")));
    }

    #[test]
    fn damaged_declarations_are_skipped_with_what_to_fix() {
        let cases = [
            ("title: A\nfix: B", "needs a `pattern:` or a `query:`"),
            ("title: A\npattern: x\nquery: y\nfix: B", "not both"),
            ("pattern: x\nfix: B", "needs a `title:`"),
            ("title: A\npattern: x", "needs a `fix:`"),
            ("title: A\npattern: (\nfix: B", "not a valid regex"),
            ("title: A\npattern: x\nfix: B\ncolour: red", "unknown key `colour`"),
            ("just words", "is not `key: value`"),
            ("  indented first", "continuation"),
        ];
        for (body, want) in cases {
            let (issues, warnings) = parse_known_issues("f.md", &format!("```known-issue\n{body}\n```"));
            assert!(issues.is_empty(), "{body}");
            assert!(warnings[0].starts_with("f.md, known issue 1:") && warnings[0].contains(want), "{body}: {warnings:?}");
        }
        let long = format!("```known-issue\ntitle: {}\npattern: x\nfix: y\n```", "t".repeat(MAX_TITLE + 1));
        assert!(parse_known_issues("f", &long).1[0].contains("title is longer"));
        let long = format!("```known-issue\ntitle: t\npattern: x\nfix: {}\n```", "y".repeat(MAX_FIX + 1));
        assert!(parse_known_issues("f", &long).1[0].contains("fix is longer"));
        let long = format!("```known-issue\ntitle: t\nquery: {}\nfix: y\n```", "q".repeat(MAX_QUERY + 1));
        assert!(parse_known_issues("f", &long).1[0].contains("query is longer"));
    }

    #[test]
    fn an_unclosed_fence_and_garbage_never_panic() {
        assert_eq!(parse_known_issues("f", "```known-issue\ntitle: A\npattern: x\nfix: y").0.len(), 1);
        assert!(parse_known_issues("f", "```").0.is_empty());
        assert!(parse_known_issues("f", "\u{0}\u{ffff}```known-issue```").0.is_empty());
    }

    #[test]
    fn the_number_of_declarations_per_file_is_bounded() {
        let one = "```known-issue\ntitle: T\npattern: x\nfix: y\n```\n";
        let (issues, warnings) = parse_known_issues("big.md", &one.repeat(MAX_ISSUES_PER_FILE + 5));
        assert_eq!(issues.len(), MAX_ISSUES_PER_FILE);
        assert!(warnings[0].contains("only the first 100"));
        // Repeated titles get distinct ids.
        assert_eq!(issues[0].id, "t");
        assert_eq!(issues[1].id, "t-2");
        assert_eq!(issues[2].id, "t-3");
    }

    #[test]
    fn titles_without_letters_get_a_numbered_id() {
        let (issues, _) = parse_known_issues("f", "```known-issue\ntitle: !!!\npattern: x\nfix: y\n```");
        assert_eq!(issues[0].id, "issue-1");
    }

    #[test]
    fn a_pattern_is_quoted_into_a_query() {
        let m = Matcher::Pattern {
            pattern: r#"say "hi" \d+"#.into(),
        };
        assert_eq!(m.to_ql(), r#"@message ~ "say \"hi\" \\d+" or @exception ~ "say \"hi\" \\d+""#);
        assert_eq!(Matcher::Query { ql: " level = error ".into() }.to_ql(), "level = error");
    }
}
