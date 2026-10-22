//! Parser generation (ADR 0032): a self-correcting loop in which the local model drafts a parser for an unknown log
//! file and the engine verifies it. The model only sees a varied sample of the file (start, middle, end, odd lines);
//! the engine checks each draft on a larger sample and answers with the exact failures (line, fragment, rule),
//! never more than needed. Up to [`MAX_ROUNDS`] rounds, until the engine's target is reached or a round brings no
//! improvement. Only engine-verified rules reach the result: rejected identity fields and metrics are dropped, the
//! time zone and an uptime boot time are the engine's.

use super::proposals::{ParserProposal, ParserRound, ParserStatus};
use super::structured::extract_json;
use super::tools::truncate_chars;
use super::Assistant;
use crate::access::StoreRef;
use crate::error::{AiError, Result};
use crate::llm::{ChatMessage, LlmProvider};
use imt_core::parser::verify::{self, FileFacts, MetricRule, ParserVerdict, VerifySample, ZoneSource};
use imt_core::parser::Parser;
use imt_core::{regex_assist, user_regex};
use serde::Deserialize;
use serde_json::json;

/// Most rounds of drafting and verification.
pub const MAX_ROUNDS: usize = 5;

const SYSTEM: &str = "You configure a log parser. Look at the sample lines and describe how to parse them. \
For text logs give ONE Rust regex with named groups: (?P<timestamp>...), (?P<level>...), (?P<message>...); other named groups become fields. \
Match only the first line of each entry. Rust regex syntax only: no lookahead, no lookbehind, no backreferences; write named groups as (?P<name>...) and escape [ ] ( ) | . + * ? when they are literal. \
timestamp_formats may use .NET or strftime formats, or one of: asctime, epoch_s, epoch_ms, epoch_us, epoch_ns, dotnet_ticks, filetime, uptime. \
identity lists the fields (groups) that tell writers apart: machine, process or PID, thread, component. \
metrics lists numbers inside messages worth charting, each with a regex over the message holding a (?P<value>...) group and a unit (bytes, KB, MB, GB, %, ms, s, count). \
Answer with JSON only. The engine tests every answer on more of the file and tells you exactly which lines fail.";

#[derive(Deserialize, Default)]
#[serde(default)]
struct Answer {
    name: String,
    format: String,
    regex: String,
    timestamp_formats: Vec<String>,
    #[serde(alias = "instance_key")]
    identity: Vec<String>,
    metrics: Vec<MetricRule>,
    rationale: String,
}

const EXAMPLE: &str = r#"Rules: the timestamp group captures ONLY the time text. Match the whole first line (^...$). Escape [ ] ( ) | . when they are literal. Put (?P<message>.*) last. Use \s+ between columns.
Example 1, lines like "2026-10-14 10:42:01,123 [worker-3] ERROR Billing.Gateway - timeout after 30000 ms":
{"name": "Billing log4net", "format": "text", "regex": "^(?P<timestamp>\\d{4}-\\d{2}-\\d{2} \\d{2}:\\d{2}:\\d{2},\\d{3})\\s+\\[(?P<Thread>[^\\]]+)\\]\\s+(?P<level>[A-Za-z]+)\\s+(?P<Logger>\\S+)\\s+-\\s(?P<message>.*)$", "timestamp_formats": ["yyyy-MM-dd HH:mm:ss,fff"], "identity": ["Thread"], "metrics": [], "rationale": "log4net pattern layout."}
Example 2, lines like "2026-10-14 10:42:01.123 +00:00 [INF] WorkingSet 600 MB, CPU 29.3%":
{"name": "Serilog text", "format": "text", "regex": "^(?P<timestamp>\\d{4}-\\d{2}-\\d{2} \\d{2}:\\d{2}:\\d{2}\\.\\d{3} [+-]\\d{2}:\\d{2}) \\[(?P<level>[A-Z]{3})\\] (?P<message>.*)$", "timestamp_formats": ["yyyy-MM-dd HH:mm:ss.fff zzz"], "identity": [], "metrics": [{"name": "WorkingSet", "regex": "WorkingSet (?P<value>\\d+) MB", "unit": "MB"}, {"name": "Cpu", "regex": "CPU (?P<value>[\\d.]+)%", "unit": "%"}], "rationale": "Serilog output template."}"#;

/// What a parser is generated from: the file's text (the engine cuts the samples) and facts about the file.
#[derive(Debug, Clone, Default)]
pub struct ParserJob<'a> {
    pub file_name: &'a str,
    /// The file's text (or its first part).
    pub content: &'a str,
    pub facts: FileFacts,
}

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

#[cfg(test)]
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

/// A verified draft.
#[derive(Clone)]
struct Draft {
    toml: String,
    rationale: String,
    metrics: Vec<MetricRule>,
    verdict: ParserVerdict,
}

impl Draft {
    fn usable(&self) -> bool {
        self.verdict.error.is_none() && self.verdict.events > 0
    }

    /// Better than `other`: done before not done, then the score, then an accepted timestamp rule.
    fn better_than(&self, other: &Draft) -> bool {
        let key = |d: &Draft| (d.verdict.done(), d.verdict.score, d.verdict.timestamps.accepted);
        let (a, b) = (key(self), key(other));
        (a.0 && !b.0) || (a.0 == b.0 && (a.1 > b.1 + 1e-9 || ((a.1 - b.1).abs() <= 1e-9 && a.2 && !b.2)))
    }
}

fn schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "name": {"type": "string"},
            "format": {"type": "string", "enum": ["text", "json", "csv"]},
            "regex": {"type": "string"},
            "timestamp_formats": {"type": "array", "items": {"type": "string"}},
            "identity": {"type": "array", "items": {"type": "string"}},
            "metrics": {"type": "array", "items": {"type": "object", "properties": {
                "name": {"type": "string"}, "regex": {"type": "string"}, "unit": {"type": "string"}
            }, "required": ["name", "regex", "unit"]}},
            "rationale": {"type": "string"}
        },
        "required": ["name", "format", "regex", "timestamp_formats", "identity", "metrics", "rationale"]
    })
}

/// The timestamp text the regexes capture on the last line they read of `text`.
fn last_timestamp_text(regexes: &[String], text: &str) -> Option<String> {
    let res: Vec<regex::Regex> = regexes.iter().filter_map(|r| user_regex::compile(r, false).ok()).collect();
    text.lines().rev().find_map(|l| {
        res.iter()
            .find_map(|re| re.captures(l)?.name("timestamp").map(|m| m.as_str().to_string()))
    })
}

/// The importer that reads the sample with `toml`.
fn judge_importer(
    toml: &str,
    check: &VerifySample,
    job: &ParserJob<'_>,
    preview: &dyn Fn(&imt_core::api::ParsePreviewRequest) -> imt_core::api::ParsePreview,
) -> Option<String> {
    preview(&imt_core::api::ParsePreviewRequest {
        toml: toml.to_string(),
        profile: String::new(),
        sample: check.text.clone(),
        file_name: job.file_name.to_string(),
        limit: 1,
    })
    .importer
}

/// Sets `boot_time` when the parser reads uptimes and the file's modification time anchors them.
fn anchor_uptime(parser: &mut Parser, facts: &FileFacts, text: &str) {
    if !parser
        .format
        .timestamp_formats
        .iter()
        .any(|f| f.trim().eq_ignore_ascii_case("uptime"))
    {
        return;
    }
    let (Some(modified), Some(last)) = (facts.modified_ms, last_timestamp_text(&parser.format.regexes, text)) else {
        return;
    };
    if let Some(boot) = verify::timestamps::uptime_boot_time(&[last], modified) {
        parser.format.boot_time = boot;
    }
}

impl<P: LlmProvider> Assistant<P> {
    /// [`generate_parser`](Self::generate_parser) for a sample alone (no file facts).
    pub fn propose_parser<'s>(&self, store: impl Into<StoreRef<'s>>, sample: &str, file_name: &str) -> Result<ParserProposal> {
        self.generate_parser(
            store,
            &ParserJob {
                file_name,
                content: sample,
                facts: FileFacts::default(),
            },
        )
    }

    /// Generates a parser for `job` (see the module documentation).
    pub fn generate_parser<'s>(&self, store: impl Into<StoreRef<'s>>, job: &ParserJob<'_>) -> Result<ParserProposal> {
        let store: StoreRef<'_> = store.into();
        if job.content.trim().is_empty() {
            return Err(AiError::Invalid("the sample is empty".into()));
        }
        let check = verify::verification_sample(job.content, verify::MAX_VERIFY_LINES);
        let preview = |req: &imt_core::api::ParsePreviewRequest| {
            store
                .with(|s| s.preview_parse(req))
                .unwrap_or_else(|e| imt_core::api::ParsePreview {
                    error: Some(format!("the engine cannot preview this parser: {e}")),
                    ..Default::default()
                })
        };
        let judge = |toml: &str, metrics: &[MetricRule]| verify::verify_text(toml, &check, job.file_name, &job.facts, metrics, preview);

        // IMT's built-in parsers read many formats already: use them when the engine accepts them, without the model.
        // (The generic parser without its last pattern: that one reads any line starting with a date.)
        let mut builtin = imt_core::parser::builtin::generic();
        builtin.format.regexes.pop();
        let builtin_toml = builtin.canonical_toml();
        let verdict = judge(&builtin_toml, &[]);
        // A delimited file is only trusted when its first line looks like a header (no digits): else columns are guessed.
        let delimited = verdict.events > 0 && judge_importer(&builtin_toml, &check, job, &preview).as_deref() == Some("csv");
        let header_ok = !delimited || !check.text.lines().next().unwrap_or("").chars().any(|c| c.is_ascii_digit());
        if verdict.done() && header_ok {
            return Ok(self.finish_builtin(verdict, &check, job, &builtin_toml, &preview));
        }

        let entries: Vec<(u32, &str)> = entry_lines(&check.text, 400);
        let engine_regex = assemble_regex(&entries.iter().map(|(_, l)| *l).take(60).collect::<Vec<_>>());
        let budget = (self.prompt_chars() / 2).clamp(1000, verify::MAX_MODEL_CHARS);
        let shown = verify::model_sample(job.content, budget);
        let starts: Vec<String> = entry_lines(&shown, 400)
            .into_iter()
            .filter(|(_, l)| !l.starts_with("--- lines "))
            .map(|(_, l)| l.to_string())
            .take(12)
            .collect();
        let hint = regex_assist::infer(&starts)
            .map(|r| {
                format!(
                    "A regex the engine inferred from the first lines (a starting point): {}\n",
                    r.pattern
                )
            })
            .unwrap_or_default();
        let first = format!(
            "File name: {}\nParts of the file (the engine checks your answer on much more of it):\n{shown}\n{hint}\n{EXAMPLE}\nReturn JSON with name, format (text, json or csv), regex (text only, else empty), timestamp_formats, identity, metrics and rationale.",
            truncate_chars(job.file_name, 120)
        );
        let mut messages = vec![ChatMessage::user(first)];
        let mut best: Option<Draft> = None;
        let mut rounds: Vec<ParserRound> = Vec::new();
        let mut last_feedback = String::new();
        for round in 1..=MAX_ROUNDS {
            let mut req = self.request(SYSTEM, messages.clone());
            req.json_schema = Some(schema());
            let resp = self.complete(&req)?;
            let (draft, feedback, note) = match serde_json::from_str::<Answer>(extract_json(&resp.text)) {
                Err(e) => (
                    None,
                    format!("the answer is not valid JSON of the requested shape ({e})"),
                    "the answer was not JSON".to_string(),
                ),
                Ok(a) => match self.draft(&a, job, &check, &entries, engine_regex.as_deref()) {
                    Err(why) => (None, why.clone(), why),
                    Ok((toml, mut note)) => {
                        let mut d = Draft {
                            verdict: judge(&toml, &a.metrics),
                            toml,
                            rationale: a.rationale.trim().to_string(),
                            metrics: a.metrics.clone(),
                        };
                        // Timestamps first: a format the engine found for the timestamp texts replaces the model's.
                        if let Some(fixed) = self.engine_timestamps(&d, job, &check, &judge) {
                            note = format!("the engine read the timestamps with \"{}\"", fixed.1);
                            d = fixed.0;
                        }
                        let feedback = d.verdict.feedback();
                        (Some(d), feedback, note)
                    }
                },
            };
            rounds.push(ParserRound {
                round,
                score: draft.as_ref().map_or(0.0, |d| d.verdict.score),
                timestamps_accepted: draft.as_ref().is_some_and(|d| d.verdict.timestamps.accepted),
                failures: draft.as_ref().map_or(1, |d| d.verdict.failures.len()),
                note,
            });
            let improved = draft
                .as_ref()
                .is_some_and(|d| d.usable() && best.as_ref().is_none_or(|b| d.better_than(b)));
            if improved {
                best = draft;
            }
            last_feedback = feedback.clone();
            if best.as_ref().is_some_and(|b| b.verdict.done()) || (round >= 2 && !improved) {
                break;
            }
            messages.push(ChatMessage::assistant(resp.text));
            messages.push(ChatMessage::user(format!(
                "Your answer was checked by the engine and rejected: {feedback}\nReply again with corrected JSON only."
            )));
        }
        let best = best.ok_or(AiError::Malformed(last_feedback))?;
        self.finish(best, rounds, &check, job, &judge, &preview)
    }

    /// Builds the parser TOML of an answer; `Err` explains to the model what is structurally wrong.
    fn draft(
        &self,
        a: &Answer,
        job: &ParserJob<'_>,
        check: &VerifySample,
        entries: &[(u32, &str)],
        engine_regex: Option<&str>,
    ) -> std::result::Result<(String, String), String> {
        let base = format!(
            "schema = 1\nid = \"{}\"\nname = {}\nversion = \"1.0.0\"\ncategory = \"Custom\"\ndescription = \"Generated with the local AI model and verified by the engine on {}.\"\n",
            imt_core::parser::draft::id_for(job.file_name),
            toml::Value::String(if a.name.trim().is_empty() {
                "Proposed parser".into()
            } else {
                truncate_chars(a.name.trim(), 60)
            }),
            truncate_chars(job.file_name, 80).replace(['"', '\\'], "")
        );
        let mut parser = Parser::from_toml(&base).map_err(|e| format!("the parser cannot be written: {e}"))?;
        let format = if a.format.trim().is_empty() { "text" } else { a.format.trim() };
        if !matches!(format, "text" | "json" | "csv") {
            return Err(format!("format must be text, json or csv, not \"{format}\""));
        }
        parser.format.kind = format.to_string();
        let mut note = String::new();
        if format == "text" {
            // The model's regex is checked against the entries first, so the feedback can quote a line it misses. If it
            // does not work but the engine could assemble a working one from the sample, that one is used.
            let structural = |text: &str| -> std::result::Result<(), String> {
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
                let missed: Vec<&(u32, &str)> = entries.iter().filter(|(_, l)| !re.is_match(l)).collect();
                // A regex that reads some entries is verified (the engine names the lines it misses); one that reads none is not.
                if !entries.is_empty() && missed.len() == entries.len() {
                    return Err(format!(
                        "your regex does not match {} of {} entries, for example line {}: `{}`. Make it match the whole first line of every entry.",
                        missed.len(),
                        entries.len(),
                        check.file_line(missed[0].0),
                        truncate_chars(missed[0].1, 200)
                    ));
                }
                Ok(())
            };
            let regex = match structural(&a.regex) {
                Ok(()) => a.regex.trim().to_string(),
                Err(why) => match engine_regex.filter(|r| structural(r).is_ok()) {
                    Some(r) => {
                        note = "the engine's regex was used".into();
                        r.to_string()
                    }
                    None => return Err(why),
                },
            };
            parser.format.output_templates = vec![];
            parser.format.regexes = vec![regex];
        }
        parser.format.timestamp_formats = a
            .timestamp_formats
            .iter()
            .map(|f| f.trim().to_string())
            .filter(|f| !f.is_empty())
            .take(8)
            .collect();
        parser.format.identity = a
            .identity
            .iter()
            .map(|f| f.trim().to_string())
            .filter(|f| !f.is_empty())
            .take(8)
            .collect();
        anchor_uptime(&mut parser, &job.facts, &check.text);
        Ok((parser.canonical_toml(), note))
    }

    /// The draft with the timestamp format the engine suggests, when it fixes the timestamp rule without a loss.
    fn engine_timestamps(
        &self,
        d: &Draft,
        job: &ParserJob<'_>,
        check: &VerifySample,
        judge: &dyn Fn(&str, &[MetricRule]) -> ParserVerdict,
    ) -> Option<(Draft, String)> {
        let format = d.verdict.timestamps.suggested_format.clone()?;
        let mut parser = Parser::from_toml(&d.toml).ok()?;
        parser.format.timestamp_formats.insert(0, format.clone());
        anchor_uptime(&mut parser, &job.facts, &check.text);
        let toml = parser.canonical_toml();
        let verdict = judge(&toml, &d.metrics);
        (verdict.timestamps.accepted && verdict.score + 1e-9 >= d.verdict.score).then(|| {
            (
                Draft {
                    toml,
                    verdict,
                    ..d.clone()
                },
                format,
            )
        })
    }

    /// The proposal of the best draft: only verified rules, the engine's zone, verified again.
    fn finish(
        &self,
        best: Draft,
        rounds: Vec<ParserRound>,
        check: &VerifySample,
        job: &ParserJob<'_>,
        judge: &dyn Fn(&str, &[MetricRule]) -> ParserVerdict,
        preview: &dyn Fn(&imt_core::api::ParsePreviewRequest) -> imt_core::api::ParsePreview,
    ) -> Result<ParserProposal> {
        let mut parser = Parser::from_toml(&best.toml).map_err(|e| AiError::Engine(format!("the verified parser does not load: {e}")))?;
        let v = &best.verdict;
        parser
            .format
            .identity
            .retain(|f| v.identity.iter().any(|c| &c.field == f && c.accepted));
        parser.extract.extend(verify::fields::metric_extractors(&best.metrics, &v.metrics));
        let zone = &v.zone;
        if zone.source != ZoneSource::Timestamps {
            parser.format.assume_timezone = zone.offset.clone();
        }
        parser.format.timezone_source = zone.source.as_str().to_string();
        parser.format.timezone_confidence = Some(zone.confidence);
        let toml = parser.canonical_toml();
        let mut verdict = judge(&toml, &best.metrics);
        // The rejected identity fields and metrics stay listed, so the user sees what was dropped and why.
        verdict.identity = v.identity.clone();
        verdict.metrics = v.metrics.clone();
        let status = if verdict.done() {
            ParserStatus::Done
        } else {
            ParserStatus::BestEffort
        };
        let pv = preview(&imt_core::api::ParsePreviewRequest {
            toml: toml.clone(),
            profile: String::new(),
            sample: check.text.clone(),
            file_name: job.file_name.to_string(),
            limit: 5,
        });
        Ok(ParserProposal {
            name: parser.name.clone(),
            toml,
            importer: pv.importer,
            parsed: verdict.events,
            with_timestamp: verdict.with_timestamp,
            preview: pv.records.into_iter().take(5).collect(),
            rationale: best.rationale,
            status,
            charts_toml: verify::fields::metric_charts_toml(&verdict.metrics),
            verdict: Some(Box::new(verdict)),
            rounds,
        })
    }

    fn finish_builtin(
        &self,
        verdict: ParserVerdict,
        check: &VerifySample,
        job: &ParserJob<'_>,
        toml: &str,
        preview: &dyn Fn(&imt_core::api::ParsePreviewRequest) -> imt_core::api::ParsePreview,
    ) -> ParserProposal {
        let pv = preview(&imt_core::api::ParsePreviewRequest {
            toml: toml.to_string(),
            profile: String::new(),
            sample: check.text.clone(),
            file_name: job.file_name.to_string(),
            limit: 5,
        });
        ParserProposal {
            name: "Built-in parsers".into(),
            toml: String::new(),
            importer: pv.importer,
            parsed: verdict.events,
            with_timestamp: verdict.with_timestamp,
            preview: pv.records,
            rationale: "IMT's built-in parsers already read this file; no changes are needed.".into(),
            status: ParserStatus::Builtin,
            verdict: Some(Box::new(verdict)),
            rounds: Vec::new(),
            charts_toml: String::new(),
        }
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
