//! A parser's `[match]` rules: which files it is for. Every rule given must hold; each adds to the parser's
//! score for a file (see [`super::set`] for the whole score and the choice).

use anyhow::{anyhow, Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use regex::Regex;
use serde::{Deserialize, Serialize};

/// Lines `sniff` looks at by default, and at most.
pub const DEFAULT_SNIFF_LINES: usize = 20;
pub const MAX_SNIFF_LINES: usize = 200;
/// Range of `priority`.
pub const MAX_PRIORITY: i32 = 1000;
/// Most file globs and JSON keys (each).
pub const MAX_LIST: usize = 64;

/// Points a rule adds when it holds.
const FILES_POINTS: u32 = 20;
const SNIFF_POINTS: f64 = 40.0;
const JSON_KEYS_POINTS: u32 = 30;
const CSV_COLUMNS_POINTS: u32 = 30;

/// `[match]` of a parser file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MatchCfg {
    /// Globs on the file name or the path relative to the opened folder (`*.clef`, `logs/**/*.txt`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    /// A regex some of the first `lines` non-empty lines must match.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sniff: Option<String>,
    /// How many lines `sniff` and `csv_columns` look at (default 20, at most 200).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lines: Option<usize>,
    /// Keys the first JSON object must have (nested keys with dots).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub json_keys: Vec<String>,
    /// Columns every one of the first lines has (with `[format.csv] delimiter`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub csv_columns: Option<usize>,
    /// Breaks ties between parsers with the same score (-1000..=1000).
    #[serde(skip_serializing_if = "is_zero")]
    pub priority: i32,
}

fn is_zero(n: &i32) -> bool {
    *n == 0
}

/// A rule that is out of range, and the key it is on.
pub(crate) struct RuleError {
    pub key: &'static str,
    pub message: String,
}

impl MatchCfg {
    /// Range checks (regexes and globs are checked when compiled).
    pub(crate) fn check(&self) -> std::result::Result<(), RuleError> {
        let err = |key, message: String| Err(RuleError { key, message });
        if let Some(n) = self.lines.filter(|n| !(1..=MAX_SNIFF_LINES).contains(n)) {
            return err("lines", format!("match.lines is {n}; use 1 to {MAX_SNIFF_LINES}"));
        }
        if !(-MAX_PRIORITY..=MAX_PRIORITY).contains(&self.priority) {
            return err(
                "priority",
                format!("match.priority is {}; use -{MAX_PRIORITY} to {MAX_PRIORITY}", self.priority),
            );
        }
        if let Some(n) = self.csv_columns.filter(|n| !(1..=1024).contains(n)) {
            return err("csv_columns", format!("match.csv_columns is {n}; use 1 to 1024"));
        }
        if self.files.len() > MAX_LIST || self.json_keys.len() > MAX_LIST {
            return err("files", format!("match.files and match.json_keys hold at most {MAX_LIST} entries"));
        }
        Ok(())
    }
}

/// The start of a file as the rules see it.
pub struct FileHead<'a> {
    /// The file's name (`app.log`).
    pub name: &'a str,
    /// The path relative to the opened folder (`logs/app.log`), or the name.
    pub rel: &'a str,
    /// The decoded first lines; `None` for a binary file.
    pub text: Option<&'a str>,
}

/// `[match]` compiled.
pub struct CompiledMatch {
    files: Option<GlobSet>,
    sniff: Option<Regex>,
    lines: usize,
    json_keys: Vec<String>,
    csv_columns: Option<usize>,
    pub priority: i32,
}

impl CompiledMatch {
    pub fn new(cfg: &MatchCfg) -> Result<CompiledMatch> {
        let files = if cfg.files.is_empty() {
            None
        } else {
            let mut b = GlobSetBuilder::new();
            for p in &cfg.files {
                b.add(Glob::new(p).with_context(|| format!("match.files: invalid glob \"{p}\""))?);
            }
            Some(b.build().context("match.files")?)
        };
        let sniff = match &cfg.sniff {
            Some(s) => Some(crate::user_regex::compile(s, false).with_context(|| format!("match.sniff: {s}"))?),
            None => None,
        };
        if cfg.json_keys.iter().any(|k| k.trim().is_empty()) {
            return Err(anyhow!("match.json_keys: keys must not be empty"));
        }
        Ok(CompiledMatch {
            files,
            sniff,
            lines: cfg.lines.unwrap_or(DEFAULT_SNIFF_LINES).clamp(1, MAX_SNIFF_LINES),
            json_keys: cfg.json_keys.clone(),
            csv_columns: cfg.csv_columns,
            priority: cfg.priority,
        })
    }

    /// Whether the parser declares a rule (a file glob or a content rule) besides its priority.
    pub fn has_rules(&self) -> bool {
        self.files.is_some() || self.sniff.is_some() || !self.json_keys.is_empty() || self.csv_columns.is_some()
    }

    /// The points of the rules for `file`, or `None` when a rule does not hold. `delimiter` is the parser's
    /// `[format.csv] delimiter` (for `csv_columns`).
    pub fn score(&self, file: &FileHead, delimiter: &str) -> Option<u32> {
        let mut points = 0u32;
        if let Some(globs) = &self.files {
            if !(globs.is_match(file.name) || globs.is_match(file.rel)) {
                return None;
            }
            points += FILES_POINTS;
        }
        let lines = || crate::importers::first_lines(file.text.unwrap_or(""), self.lines);
        if let Some(sniff) = &self.sniff {
            let all: Vec<&str> = lines().collect();
            let hits = all.iter().filter(|l| sniff.is_match(l)).count();
            if file.text.is_none() || hits == 0 {
                return None;
            }
            points += (SNIFF_POINTS * hits as f64 / all.len() as f64).round() as u32;
        }
        if !self.json_keys.is_empty() {
            let first = lines().next()?;
            let obj: serde_json::Map<String, serde_json::Value> = serde_json::from_str(first.trim()).ok()?;
            if !self.json_keys.iter().all(|k| has_key(&obj, k)) {
                return None;
            }
            // A file that also holds text events is not a JSON file (ADR 0020: the text importer reads both).
            if lines().any(|l| !l.trim_start().starts_with('{') && crate::importers::lines::looks_like_event_start(l)) {
                return None;
            }
            points += JSON_KEYS_POINTS;
        }
        if let Some(n) = self.csv_columns {
            let text = file.text?;
            let d = crate::importers::csv_delimiter(delimiter, text)?;
            if !lines().all(|l| crate::importers::csv_count_outside_quotes(l, d) + 1 == n) {
                return None;
            }
            points += CSV_COLUMNS_POINTS;
        }
        Some(points)
    }
}

/// Whether `obj` has `key` (a dotted key looks into nested objects; a key with a dot is also tried whole).
fn has_key(obj: &serde_json::Map<String, serde_json::Value>, key: &str) -> bool {
    if obj.get(key).is_some_and(|v| !v.is_null()) {
        return true;
    }
    let mut parts = key.split('.');
    let Some(first) = parts.next() else { return false };
    let mut value = obj.get(first);
    for p in parts {
        value = value.and_then(|v| v.get(p));
    }
    value.is_some_and(|v| !v.is_null())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn head<'a>(name: &'a str, text: &'a str) -> FileHead<'a> {
        FileHead {
            name,
            rel: name,
            text: Some(text),
        }
    }

    fn rules(toml_text: &str) -> CompiledMatch {
        CompiledMatch::new(&toml::from_str::<MatchCfg>(toml_text).unwrap()).unwrap()
    }

    #[test]
    fn no_rules_hold_everywhere() {
        let any = rules("");
        assert_eq!(any.score(&head("a.log", "x"), "auto"), Some(0));
        assert_eq!(
            any.score(
                &FileHead {
                    name: "a.evtx",
                    rel: "a.evtx",
                    text: None
                },
                "auto"
            ),
            Some(0)
        );
    }

    #[test]
    fn globs_match_the_name_or_the_relative_path() {
        let r = rules("files = ['*.clef', 'logs/**/*.txt']");
        assert_eq!(r.score(&head("a.clef", ""), "auto"), Some(20));
        let nested = FileHead {
            name: "b.txt",
            rel: "logs/x/b.txt",
            text: Some(""),
        };
        assert_eq!(r.score(&nested, "auto"), Some(20));
        assert_eq!(r.score(&head("b.log", ""), "auto"), None);
    }

    #[test]
    fn sniff_scales_with_the_share_of_lines() {
        let r = rules("sniff = '^\\d{4}/'\nlines = 4");
        assert_eq!(r.score(&head("a", "2025/1\n2025/2\n  at x\n  at y\n2025/3\n"), "auto"), Some(20));
        assert_eq!(r.score(&head("a", "2025/1\n2025/2\n"), "auto"), Some(40));
        assert_eq!(r.score(&head("a", "nope\n"), "auto"), None);
        assert_eq!(
            r.score(
                &FileHead {
                    name: "a",
                    rel: "a",
                    text: None
                },
                "auto"
            ),
            None
        );
    }

    #[test]
    fn json_keys_look_at_the_first_object() {
        let r = rules("json_keys = ['@t', 'log.level']");
        assert_eq!(
            r.score(&head("a", "{\"@t\":\"x\",\"log\":{\"level\":\"info\"}}\n"), "auto"),
            Some(30)
        );
        assert_eq!(r.score(&head("a", "{\"@t\":\"x\",\"log.level\":\"info\"}\n"), "auto"), Some(30));
        assert_eq!(r.score(&head("a", "{\"@t\":\"x\"}\n"), "auto"), None);
        assert_eq!(r.score(&head("a", "{\"@t\":null,\"log.level\":1}\n"), "auto"), None);
        assert_eq!(r.score(&head("a", "not json\n"), "auto"), None);
        assert_eq!(r.score(&head("a", ""), "auto"), None);
        // JSON lines mixed with text events are not a JSON file.
        let mixed = "{\"@t\":\"x\",\"log\":{\"level\":\"i\"}}\n2026-10-14 10:00:00 INFO text\n";
        assert_eq!(r.score(&head("a", mixed), "auto"), None);
    }

    #[test]
    fn rules_are_declared_or_not() {
        assert!(!rules("priority = 3").has_rules());
        assert!(rules("files = ['*.x']").has_rules() && rules("csv_columns = 2").has_rules());
    }

    #[test]
    fn csv_columns_count_every_line() {
        let r = rules("csv_columns = 3");
        assert_eq!(r.score(&head("a", "a,b,c\n1,\"2,x\",3\n"), "auto"), Some(30));
        assert_eq!(r.score(&head("a", "a,b,c\n1,2\n"), "auto"), None);
        assert_eq!(r.score(&head("a", "a;b;c\n"), ";"), Some(30));
        // Consistent columns, but not the number the rule asks for.
        assert_eq!(r.score(&head("a", "a,b\n1,2\n"), "auto"), None);
        assert_eq!(
            r.score(
                &FileHead {
                    name: "a",
                    rel: "a",
                    text: None
                },
                ","
            ),
            None
        );
    }

    #[test]
    fn ranges_and_bad_rules_are_refused() {
        let check = |t: &str| toml::from_str::<MatchCfg>(t).unwrap().check().err().map(|e| (e.key, e.message));
        assert!(check("lines = 0").is_some_and(|(k, _)| k == "lines"));
        assert!(check("lines = 201").is_some());
        assert!(check("priority = 1001").is_some_and(|(k, _)| k == "priority"));
        assert!(check("csv_columns = 0").is_some_and(|(k, _)| k == "csv_columns"));
        let many: Vec<String> = (0..=MAX_LIST).map(|i| format!("'*.{i}'")).collect();
        assert!(check(&format!("files = [{}]", many.join(","))).is_some());
        assert!(check("priority = -5\nlines = 200").is_none());
        assert!(CompiledMatch::new(&toml::from_str::<MatchCfg>("files = ['a[']").unwrap()).is_err());
        assert!(CompiledMatch::new(&toml::from_str::<MatchCfg>("sniff = '('").unwrap()).is_err());
        assert!(CompiledMatch::new(&toml::from_str::<MatchCfg>("json_keys = [' ']").unwrap()).is_err());
        assert!(toml::from_str::<MatchCfg>("sniffs = 'x'").is_err());
    }
}
