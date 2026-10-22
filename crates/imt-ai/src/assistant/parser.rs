//! propose_profile: format, timestamp format, identity fields and parse regex for an unknown log file, as a
//! parser for the library (formats are parsers, ADR 0026). The engine builds the candidate parser, compiles
//! it and parses the sample with it.

use super::proposals::ProfileProposal;
use super::tools::truncate_chars;
use super::Assistant;
use crate::access::StoreRef;
use crate::error::{AiError, Result};
use crate::llm::LlmProvider;
use imt_core::api::ParsePreviewRequest;
use imt_core::parser::Parser;
use imt_core::{regex_assist, user_regex};
use serde::Deserialize;
use serde_json::json;

const SYSTEM: &str = "You configure a log parser. Look at the sample lines and describe how to parse them. \
For text logs give ONE Rust regex with named groups: (?P<timestamp>...), (?P<level>...), (?P<message>...); other named groups become fields. \
Match only the first line of each entry. Rust regex syntax only: no lookahead, no lookbehind, no backreferences; write named groups as (?P<name>...) and escape [ ] ( ) | . + * ? when they are literal. Answer with JSON only.";

#[derive(Deserialize)]
struct Answer {
    #[serde(default)]
    name: String,
    format: String,
    #[serde(default)]
    regex: String,
    #[serde(default)]
    timestamp_formats: Vec<String>,
    #[serde(default)]
    instance_key: Vec<String>,
    #[serde(default)]
    rationale: String,
}

const EXAMPLE: &str = r#"Rules: the timestamp group captures ONLY the time text. Match the whole first line (^...$). Escape [ ] ( ) | . when they are literal. Put (?P<message>.*) last. Use \s+ between columns.
Example 1, lines like "2026-10-14 10:42:01,123 [worker-3] ERROR Billing.Gateway - timeout after 30000 ms":
{"name": "Billing log4net", "format": "text", "regex": "^(?P<timestamp>\\d{4}-\\d{2}-\\d{2} \\d{2}:\\d{2}:\\d{2},\\d{3})\\s+\\[(?P<Thread>[^\\]]+)\\]\\s+(?P<level>[A-Za-z]+)\\s+(?P<Logger>\\S+)\\s+-\\s(?P<message>.*)$", "timestamp_formats": ["yyyy-MM-dd HH:mm:ss,fff"], "instance_key": [], "rationale": "log4net pattern layout."}
Example 2, lines like "2026-10-14 10:42:01.123 +00:00 [WRN] Outbox timed out":
{"name": "Serilog text", "format": "text", "regex": "^(?P<timestamp>\\d{4}-\\d{2}-\\d{2} \\d{2}:\\d{2}:\\d{2}\\.\\d{3} [+-]\\d{2}:\\d{2}) \\[(?P<level>[A-Z]{3})\\] (?P<message>.*)$", "timestamp_formats": ["yyyy-MM-dd HH:mm:ss.fff zzz"], "instance_key": [], "rationale": "Serilog output template."}"#;

/// How a line begins: a digit, a letter, or the punctuation character itself.
fn shape(l: &str) -> char {
    match l.chars().next() {
        Some(c) if c.is_ascii_digit() => 'd',
        Some(c) if c.is_alphabetic() => 'a',
        Some(c) => c,
        None => ' ',
    }
}

/// The first line of every entry, with its 1-based line number: lines that are not indented and begin the way the
/// first line does (so an unindented "java.net.SocketTimeoutException: ..." under a dated line is a continuation).
fn entry_lines(sample: &str, limit: usize) -> Vec<(u32, &str)> {
    let mut first: Option<char> = None;
    let mut out = Vec::new();
    for (i, l) in sample.lines().enumerate().take(limit) {
        if l.trim().is_empty() || l.starts_with(char::is_whitespace) {
            continue;
        }
        let sh = *first.get_or_insert_with(|| shape(l));
        if shape(l) == sh {
            out.push((i as u32 + 1, l));
        }
    }
    out
}

fn record_starts(sample: &str) -> usize {
    entry_lines(sample, 400).len()
}

const LEVEL_WORDS: &[&str] = &[
    "trace",
    "debug",
    "dbg",
    "info",
    "information",
    "inf",
    "notice",
    "warn",
    "warning",
    "wrn",
    "error",
    "err",
    "fatal",
    "ftl",
    "critical",
    "crit",
    "verbose",
    "vrb",
];

/// A regex built by the engine from the sample's entries, with its groups given the roles the parser needs:
/// the timestamp group, the level group (the word group whose values are all level names) and the message
/// (the last free-text group). `None` when the entries do not have that shape.
pub(crate) fn assemble_regex(entries: &[&str]) -> Option<String> {
    let owned: Vec<String> = entries.iter().map(|s| s.to_string()).collect();
    let inf = regex_assist::infer(&owned).ok()?;
    let mut pattern = inf.pattern.clone();
    let rename = |pattern: &mut String, from: &str, to: &str| {
        if pattern.contains(&format!("(?P<{to}>")) {
            return false;
        }
        *pattern = pattern.replacen(&format!("(?P<{from}>"), &format!("(?P<{to}>"), 1);
        true
    };
    let ts = inf.groups.iter().find(|g| g.kind == "timestamp")?;
    rename(&mut pattern, &ts.name, "timestamp");
    if let Some(l) = inf.groups.iter().find(|g| {
        g.kind == "word"
            && !g.examples.is_empty()
            && g.examples
                .iter()
                .all(|e| LEVEL_WORDS.contains(&e.trim_matches(|c: char| !c.is_alphabetic()).to_lowercase().as_str()))
    }) {
        rename(&mut pattern, &l.name, "level");
    }
    let msg = inf
        .groups
        .iter()
        .rev()
        .find(|g| (g.kind == "text" || g.kind == "token") && g.name != ts.name)?;
    if !rename(&mut pattern, &msg.name, "message") {
        return None;
    }
    if !pattern.starts_with('^') {
        pattern.insert(0, '^');
    }
    Some(pattern)
}

impl<P: LlmProvider> Assistant<P> {
    pub fn propose_profile<'s>(&self, store: impl Into<StoreRef<'s>>, sample: &str, file_name: &str) -> Result<ProfileProposal> {
        let store: StoreRef<'_> = store.into();
        let budget = self.prompt_chars();
        let mut head = String::new();
        for l in sample.lines().take(60) {
            let l = truncate_chars(l, 300);
            if head.len() + l.len() > budget / 2 {
                break;
            }
            head.push_str(&l);
            head.push('\n');
        }
        if head.trim().is_empty() {
            return Err(AiError::Invalid("the sample is empty".into()));
        }
        let starts: Vec<String> = entry_lines(&head, 400).into_iter().map(|(_, l)| l.to_string()).take(12).collect();
        let hint = regex_assist::infer(&starts)
            .map(|r| {
                format!(
                    "A regex the engine inferred from the first lines (a starting point): {}\n",
                    r.pattern
                )
            })
            .unwrap_or_default();
        let user = format!("File name: {file_name}\nSample:\n{head}\n{hint}\n{EXAMPLE}\nReturn JSON with name, format (text, json or csv), regex (text only, else empty), timestamp_formats, instance_key (field names that identify an instance, usually empty) and rationale.");
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "format": {"type": "string", "enum": ["text", "json", "csv"]},
                "regex": {"type": "string"},
                "timestamp_formats": {"type": "array", "items": {"type": "string"}},
                "instance_key": {"type": "array", "items": {"type": "string"}},
                "rationale": {"type": "string"}
            },
            "required": ["name", "format", "regex", "timestamp_formats", "instance_key", "rationale"]
        });
        let expected = record_starts(sample.lines().take(400).collect::<Vec<_>>().join("\n").as_str());
        // IMT's built-in settings read many formats already: use them when they cope, without asking the model.
        // (Without its last, generic pattern: that one reads any line starting with a date, level and all.)
        let mut builtin = imt_core::parser::builtin::generic();
        builtin.format.regexes.pop();
        {
            let toml = builtin.canonical_toml();
            let req = ParsePreviewRequest {
                toml: toml.clone(),
                profile: String::new(),
                sample: sample.to_string(),
                file_name: file_name.to_string(),
                limit: 200,
            };
            let pv = store
                .with(|s| s.preview_parse(&req))
                .map_err(|e| AiError::Invalid(format!("the engine cannot preview this profile: {e}")))?;
            let with_ts = pv.records.iter().filter(|r| r.ts.is_some()).count();
            // A delimited file is only trusted when its first line looks like a header (no digits).
            let header_ok =
                pv.importer.as_deref() != Some("csv") || !sample.lines().next().unwrap_or("").chars().any(|c| c.is_ascii_digit());
            if pv.error.is_none() && header_ok && pv.total > 0 && pv.total * 100 >= expected * 95 && with_ts * 100 >= pv.records.len() * 95
            {
                // Nothing to add to the library: the proposal carries no parser.
                let _ = toml;
                return Ok(ProfileProposal {
                    name: "Built-in parsers".into(),
                    toml: String::new(),
                    importer: pv.importer,
                    parsed: pv.total,
                    with_timestamp: with_ts,
                    preview: pv.records.into_iter().take(5).collect(),
                    rationale: "IMT's built-in parsers already read this file; no changes are needed.".into(),
                });
            }
        }
        // The first line of every entry, with its 1-based line number (continuation lines are indented).
        let entry_lines = entry_lines(sample, 400);
        let engine_regex = assemble_regex(&entry_lines.iter().map(|(_, l)| *l).take(60).collect::<Vec<_>>());
        let (p, _) = self.generate_validated::<Answer, ProfileProposal>(SYSTEM, &user, schema, |a| {
            let base = format!(
                "schema = 1\nid = \"{}\"\nname = {}\nversion = \"1.0.0\"\ncategory = \"Custom\"\ndescription = \"Proposed by the local AI model from a sample of {}.\"\n",
                imt_core::parser::draft::id_for(file_name),
                toml::Value::String(if a.name.trim().is_empty() { "Proposed parser".into() } else { truncate_chars(a.name.trim(), 60) }),
                truncate_chars(file_name, 80).replace(['"', '\\'], "")
            );
            let mut parser = Parser::from_toml(&base).map_err(|e| format!("the parser cannot be written: {e}"))?;
            parser.format.kind = a.format.clone();
            if a.format == "text" {
                // The model's regex is checked against the sample here, so the feedback can quote a line it misses. If it
                // does not work but the engine could assemble a working one from the sample, that one is used.
                let check = |text: &str| -> std::result::Result<(), String> {
                    if text.trim().is_empty() {
                        return Err("format \"text\" needs a regex with named groups".into());
                    }
                    let re = user_regex::compile(text.trim(), false).map_err(|e| format!("the regex does not compile: {e}"))?;
                    let names: Vec<&str> = re.capture_names().flatten().collect();
                    if !names.contains(&"message") {
                        return Err("the regex needs a (?P<message>...) group holding the main text of the entry (for an access log, the request line, for example (?P<message>\"[A-Z]+ [^\"]*\"))".into());
                    }
                    if !names.contains(&"timestamp") {
                        return Err("the regex needs a (?P<timestamp>...) group".into());
                    }
                    let missed: Vec<&(u32, &str)> = entry_lines.iter().filter(|(_, l)| !re.is_match(l)).collect();
                    if missed.len() * 20 > entry_lines.len().max(1) {
                        return Err(format!(
                            "your regex does not match {} of {} entries, for example this line: `{}`. Make it match the whole first line of every entry.",
                            missed.len(),
                            entry_lines.len(),
                            truncate_chars(missed[0].1, 200)
                        ));
                    }
                    Ok(())
                };
                let regex_text = match check(&a.regex) {
                    Ok(()) => a.regex.trim().to_string(),
                    Err(why) => match engine_regex.as_deref().filter(|r| check(r).is_ok()) {
                        Some(r) => r.to_string(),
                        None => return Err(why),
                    },
                };
                parser.format.output_templates = vec![];
                parser.format.regexes = vec![regex_text];
            }
            parser.format.timestamp_formats = a.timestamp_formats.iter().map(|f| f.trim().to_string()).filter(|f| !f.is_empty()).collect();
            // Fields that identify an instance are the parser's identity fields (instance-key candidates).
            parser.format.identity = a.instance_key.iter().map(|f| f.trim().to_string()).filter(|f| !f.is_empty()).collect();
            let toml = parser.canonical_toml();
            let preview = store
                .with(|s| {
                    s.preview_parse(&ParsePreviewRequest {
                        toml: toml.clone(),
                        profile: String::new(),
                        sample: sample.to_string(),
                        file_name: file_name.to_string(),
                        limit: 200,
                    })
                })
                .map_err(|e| format!("the engine cannot preview this parser: {e}"))?;
            if let Some(e) = preview.error {
                return Err(format!("the engine cannot use this parser: {e}"));
            }
            let parsed = preview.total;
            if parsed == 0 || (a.format != "csv" && parsed * 100 < expected * 95) {
                let parsed_lines: Vec<u32> = preview.records.iter().map(|r| r.line).collect();
                let example = entry_lines.iter().find(|(n, _)| !parsed_lines.contains(n)).map(|(_, l)| format!(", for example this line: `{}`", truncate_chars(l, 200))).unwrap_or_default();
                return Err(format!("only {parsed} of about {expected} entries were parsed{example}. Every first line of an entry must be parsed."));
            }
            let with_timestamp = preview.records.iter().filter(|r| r.ts.is_some()).count();
            if with_timestamp * 100 < preview.records.len() * 95 {
                let bad = preview.records.iter().find(|r| r.ts.is_none()).and_then(|r| sample.lines().nth(r.line as usize - 1)).map(|l| format!(" (for example in `{}`)", truncate_chars(l, 160))).unwrap_or_default();
                return Err(format!(
                    "timestamps were not understood in {} of {} entries{bad}; the `timestamp` group must capture only the time text, and timestamp_formats must describe it, for example yyyy-MM-dd HH:mm:ss",
                    preview.records.len() - with_timestamp,
                    preview.records.len()
                ));
            }
            Ok(ProfileProposal {
                name: parser.name.clone(),
                toml,
                importer: preview.importer,
                parsed,
                with_timestamp,
                preview: preview.records.into_iter().take(5).collect(),
                rationale: a.rationale.trim().to_string(),
            })
        })?;
        Ok(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_are_lines_that_begin_like_the_first_one() {
        let sample = "2026-10-14 10:00:00 ERROR boom\njava.net.SocketTimeoutException: Read timed out\n\tat Foo.bar(Foo.java:1)\n2026-10-14 10:00:01 INFO ok\n   indented\n";
        let e = entry_lines(sample, 100);
        assert_eq!(e.iter().map(|(n, _)| *n).collect::<Vec<_>>(), [1, 4]);
        assert_eq!(record_starts(sample), 2);
        assert_eq!(record_starts("a=1\nb=2\n"), 2);
        assert_eq!(record_starts("[x] 1\n[y] 2\nz 3"), 2);
        assert_eq!(shape(""), ' ');
        assert!(entry_lines("", 10).is_empty());
    }

    #[test]
    fn the_engine_assembles_a_regex_with_the_roles_the_parser_needs() {
        let lines = [
            "2026-10-14 10:41:24,101 INFO  [http-nio-8080-exec-3] c.example.orders.OrderController - Order 91713 created",
            "2026-10-14 10:41:25,230 WARN  [http-nio-8080-exec-5] c.example.orders.PaymentClient - Gateway slow: 2171 ms",
            "2026-10-14 10:41:26,415 ERROR [http-nio-8080-exec-5] c.example.orders.PaymentClient - Capture failed for order 91713",
            "2026-10-14 10:41:27,002 INFO  [http-nio-8080-exec-1] c.example.orders.OrderController - Order 91714 created",
        ];
        let r = assemble_regex(&lines).expect("a regex");
        let re = regex::Regex::new(&r).unwrap();
        let names: Vec<&str> = re.capture_names().flatten().collect();
        assert!(names.contains(&"timestamp") && names.contains(&"message"), "{r}");
        assert!(lines.iter().all(|l| re.is_match(l)));
        // no timestamp, no regex
        assert!(assemble_regex(&["plain words only", "more plain words"]).is_none());
        assert!(assemble_regex(&[]).is_none());
    }
}
