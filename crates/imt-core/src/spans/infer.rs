//! Span rule inference: from start/end example events the user picked, the engine proposes the rule
//! that pairs them (an extractor for the key, start and stop markers, a `[[span]]` per key) and
//! evaluates any rule (its own or one an AI proposed) on the data.
//!
//! The words that tell starts from ends are the tokens only the start (or only the end) lines have, at
//! either end of the line ("open X" / "close X", "X started" / "X ended", "View X loaded" / "View X
//! closed"); the key is a token both lines of a pair share, captured by a bounded regex anchored on a
//! verb or on the token next to it (`[Tab:42]`), or a field both events carry with the same value
//! (JSON logs). Several keys are offered when the examples allow more than one.

use super::draft::Classifier;
use super::pairing::{Edge, Pairer, RunStatus};
use super::{SpanCfg, SpanRule};
use crate::condition::{CondValue, ConditionCfg};
use crate::model::Record;
use crate::profile::{CompiledProfile, ExtractCfg, FieldSpec, FieldType, MarkerCfg, Profile};
use crate::row::RowRef;
use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Most example pairs looked at.
pub const MAX_EXAMPLES: usize = 20;
/// Most key candidates proposed.
pub const MAX_CANDIDATES: usize = 3;
/// Sample pairs in an evaluation.
pub const MAX_SAMPLES: usize = 20;
/// Characters of an example message looked at.
const MAX_MESSAGE: usize = 2000;
/// The class of characters a key consists of (bounded by the regex size limits of `user_regex`).
const KEY_CLASS: &str = r"[A-Za-z0-9_.\-]+";

/// What the user tells the inference besides the examples.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SpanHints {
    /// Span kind (an id); `view`, `tab`... (default `span`).
    pub name: Option<String>,
    /// Pair by this field instead of a token of the message.
    pub key_field: Option<String>,
}

/// How a candidate finds the key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum KeySource {
    /// A field the events already carry.
    Field { name: String },
    /// A token of the message, extracted by this regex into the field `name`.
    Pattern { name: String, regex: String },
}

impl KeySource {
    pub fn field(&self) -> &str {
        match self {
            KeySource::Field { name } | KeySource::Pattern { name, .. } => name,
        }
    }
}

/// One proposed rule.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpanCandidate {
    /// In words: what pairs the events and what starts and ends them.
    pub description: String,
    pub key: KeySource,
    /// The example keys, one per pair.
    pub example_keys: Vec<String>,
    pub start: ConditionCfg,
    pub stop: ConditionCfg,
    pub kind: String,
    /// The profile override that adds the rule (extractor, markers, span), ready for evaluation and for
    /// applying through the profile-override path.
    pub profile_override: String,
}

/// What a rule does on the data.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpanEvaluation {
    /// Runs with a start and a stop.
    pub spans: usize,
    /// Distinct keys of the runs.
    pub distinct_keys: usize,
    /// Starts not stopped by the end of the data.
    pub open: usize,
    /// Starts followed by another start of the same key before a stop.
    pub unmatched_starts: usize,
    /// Stops without a start.
    pub unmatched_ends: usize,
    pub longest_ms: Option<i64>,
    pub shortest_ms: Option<i64>,
    /// Most runs open at the same time in one lane (overlapping keys).
    pub max_concurrent: usize,
    pub samples: Vec<SpanSample>,
    /// Events looked at.
    pub events: u64,
    /// Runs beyond the per-lane limit (the oldest are dropped from the figures above) and events ignored
    /// beyond the key limit, as a `[[span]]` rule keeps them.
    pub dropped: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpanSample {
    pub key: String,
    pub lane: String,
    pub start_id: Option<u32>,
    pub stop_id: Option<u32>,
    pub from: i64,
    pub to: i64,
    pub status: RunStatus,
}

/// A start and the end event that belongs to it, picked by the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpanExample {
    pub start: crate::batch::EventId,
    pub end: crate::batch::EventId,
}

/// `propose_span_rule`: example pairs and hints.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SpanRuleRequest {
    pub examples: Vec<SpanExample>,
    pub hints: SpanHints,
}

/// `evaluate_span_rule`: a rule as a profile override (the engine's or an AI's) and the span kind it adds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpanRuleCheck {
    pub profile_override: String,
    pub kind: String,
}

/// A candidate with its evaluation.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluatedCandidate {
    #[serde(flatten)]
    pub candidate: SpanCandidate,
    pub evaluation: SpanEvaluation,
}

/// The proposal: candidates, best first, each evaluated on the data.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpanProposal {
    pub candidates: Vec<EvaluatedCandidate>,
    pub notes: Vec<String>,
}

/// A token of a message: its text and byte span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Token<'a> {
    text: &'a str,
    start: usize,
    end: usize,
}

fn is_token_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '.' | '-')
}

fn tokens(s: &str) -> Vec<Token<'_>> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in s.char_indices() {
        match (is_token_char(c), start) {
            (true, None) => start = Some(i),
            (false, Some(b)) => {
                out.push(Token {
                    text: &s[b..i],
                    start: b,
                    end: i,
                });
                start = None;
            }
            _ => {}
        }
    }
    if let Some(b) = start {
        out.push(Token {
            text: &s[b..],
            start: b,
            end: s.len(),
        });
    }
    // Trailing dots of a sentence are not part of a word.
    for t in &mut out {
        let trimmed = t.text.trim_end_matches('.');
        if !trimmed.is_empty() {
            t.end = t.start + trimmed.len();
            t.text = trimmed;
        }
    }
    out
}

fn message(r: &dyn RowRef) -> &str {
    let m = r.message();
    m.char_indices().nth(MAX_MESSAGE).map_or(m, |(i, _)| &m[..i])
}

fn word(text: &str) -> bool {
    text.chars().any(char::is_alphabetic) && !text.chars().any(|c| c.is_ascii_digit())
}

/// Tokens every start (or every end) line has and no line of the other side has, in the order of the
/// first example.
fn distinguishing(pairs: &[(&dyn RowRef, &dyn RowRef)], start: bool) -> Vec<String> {
    fn pick<'a>(p: &(&'a dyn RowRef, &'a dyn RowRef), first: bool) -> &'a dyn RowRef {
        if first {
            p.0
        } else {
            p.1
        }
    }
    let side = |p: &(&dyn RowRef, &dyn RowRef)| message(pick(p, start)).to_string();
    let other = |p: &(&dyn RowRef, &dyn RowRef)| message(pick(p, !start)).to_string();
    let others: BTreeSet<String> = pairs
        .iter()
        .flat_map(|p| tokens(&other(p)).into_iter().map(|t| t.text.to_lowercase()).collect::<Vec<_>>())
        .collect();
    let mut out: Vec<String> = Vec::new();
    let first = side(&pairs[0]);
    for t in tokens(&first) {
        let lower = t.text.to_lowercase();
        if !word(t.text) || others.contains(&lower) || out.contains(&lower) {
            continue;
        }
        if pairs
            .iter()
            .all(|p| tokens(&side(p)).iter().any(|u| u.text.eq_ignore_ascii_case(t.text)))
        {
            out.push(lower);
        }
    }
    out
}

/// A field whose value tells starts from ends: the same value on every start, another on every end.
fn distinguishing_field(pairs: &[(&dyn RowRef, &dyn RowRef)]) -> Option<(String, String, String)> {
    let mut found = None;
    pairs[0].0.for_each_field(&mut |name, v| {
        if found.is_some() {
            return;
        }
        let s = crate::model::value_text(&v).into_owned();
        let end = pairs[0].1.text_of(name).map(|t| t.into_owned());
        let Some(end) = end.filter(|e| *e != s) else { return };
        let same = pairs
            .iter()
            .all(|(a, b)| a.text_of(name).as_deref() == Some(s.as_str()) && b.text_of(name).as_deref() == Some(end.as_str()));
        if same && !s.is_empty() && !end.is_empty() {
            found = Some((name.to_string(), s, end));
        }
    });
    found
}

/// The case-insensitive whole-word condition on the message.
fn word_condition(w: &str) -> ConditionCfg {
    ConditionCfg {
        field: "@message".into(),
        op: "matches".into(),
        value: Some(CondValue::Text(format!(r"(?i)\b{}\b", regex::escape(w)))),
    }
}

/// Key regexes for a token both lines share: anchored on the verbs on either side, or on the shared token
/// next to it.
fn key_patterns(start: &str, key: &Token<'_>, verbs: &[String]) -> Vec<String> {
    let toks = tokens(start);
    let i = toks.iter().position(|t| t == key).unwrap_or(0);
    let mut out = Vec::new();
    let alternation = format!("(?:{})", verbs.iter().map(|v| regex::escape(v)).collect::<Vec<_>>().join("|"));
    let sep = |a: usize, b: usize| {
        let s = &start[a..b];
        if s.chars().all(char::is_whitespace) {
            r"\s+".to_string()
        } else {
            regex::escape(s)
        }
    };
    if let Some(prev) = i.checked_sub(1).map(|j| &toks[j]) {
        if verbs.iter().any(|v| v.eq_ignore_ascii_case(prev.text)) {
            out.push(format!(r"(?i)\b{alternation}{}(?P<SpanKey>{KEY_CLASS})", sep(prev.end, key.start)));
        } else {
            out.push(format!(
                r"\b{}{}(?P<SpanKey>{KEY_CLASS})",
                regex::escape(prev.text),
                sep(prev.end, key.start)
            ));
        }
    }
    if let Some(next) = toks.get(i + 1) {
        if verbs.iter().any(|v| v.eq_ignore_ascii_case(next.text)) {
            out.push(format!(
                r"(?i)\b(?P<SpanKey>{KEY_CLASS}){}{alternation}\b",
                sep(key.end, next.start)
            ));
        } else {
            out.push(format!(
                r"\b(?P<SpanKey>{KEY_CLASS}){}{}\b",
                sep(key.end, next.start),
                regex::escape(next.text)
            ));
        }
    }
    if toks.len() == 1 {
        out.push(format!("^(?P<SpanKey>{KEY_CLASS})$"));
    }
    out
}

fn capture(re: &regex::Regex, text: &str) -> Option<String> {
    re.captures(text).and_then(|c| c.name("SpanKey")).map(|m| m.as_str().to_string())
}

/// A span kind not used by the profile yet (and the marker kinds it needs).
fn free_kind(profile: &Profile, wanted: &str) -> String {
    let base: String = wanted
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .take(40)
        .collect();
    let base = if base.is_empty() { "span".to_string() } else { base };
    let taken = |k: &str| {
        profile.span.iter().any(|s| s.kind == k)
            || profile
                .marker
                .iter()
                .any(|m| m.kind == format!("{k}-start") || m.kind == format!("{k}-stop"))
    };
    (1..100)
        .map(|n| if n == 1 { base.clone() } else { format!("{base}{n}") })
        .find(|k| !taken(k))
        .unwrap_or(base)
}

/// The TOML override that adds the rule to `profile` (lists replace in an override, so the profile's own
/// extractors, markers and spans are repeated).
fn override_toml(profile: &Profile, kind: &str, key: &KeySource, start: &ConditionCfg, stop: &ConditionCfg) -> Result<String> {
    let mut extract = profile.extract.clone();
    let key_field = key.field().to_string();
    let exists = ConditionCfg {
        field: key_field.clone(),
        op: "exists".into(),
        value: None,
    };
    if let KeySource::Pattern { name, regex } = key {
        extract.push(ExtractCfg {
            name: format!("{kind}-key"),
            from: "@message".into(),
            regex: regex.clone(),
            when: Vec::new(),
            fields: BTreeMap::from([(
                name.clone(),
                // Untyped: the store keeps it with the other undeclared fields (no new column).
                FieldSpec {
                    kind: FieldType::Auto,
                    unit: None,
                    unit_from: None,
                    to: None,
                },
            )]),
        });
    }
    let marker = |suffix: &str, label: &str, c: &ConditionCfg| -> Result<MarkerCfg> {
        Ok(serde_json::from_value(serde_json::json!({
            "kind": format!("{kind}-{suffix}"),
            "label": format!("{kind} {label}"),
            "when": [c, exists],
        }))?)
    };
    let mut markers = profile.marker.clone();
    markers.push(marker("start", "start", start)?);
    markers.push(marker("stop", "stop", stop)?);
    let mut spans = profile.span.clone();
    spans.push(SpanCfg {
        kind: kind.to_string(),
        label: Some(kind.to_string()),
        start: super::OneOrMany::One(format!("{kind}-start")),
        stop: super::OneOrMany::One(format!("{kind}-stop")),
        per: format!("field:{key_field}"),
        color: None,
        max_duration: None,
        restart_on_start: true,
        enabled: true,
    });
    let mut table = toml::Table::new();
    table.insert("extract".into(), toml::Value::try_from(&extract)?);
    table.insert("marker".into(), toml::Value::try_from(&markers)?);
    table.insert("span".into(), toml::Value::try_from(&spans)?);
    Ok(toml::to_string(&table)?)
}

/// Proposes rules that pair the example (start, end) events, best first (not yet evaluated).
pub fn propose(pairs: &[(&dyn RowRef, &dyn RowRef)], hints: &SpanHints, profile: &Profile) -> Result<Vec<SpanCandidate>> {
    if pairs.is_empty() {
        bail!("pick at least one start event and the end event that belongs to it");
    }
    let pairs = &pairs[..pairs.len().min(MAX_EXAMPLES)];
    let kind = free_kind(profile, hints.name.as_deref().unwrap_or("span"));
    // What starts and what ends: words of the message, else a field's value.
    let (starts, stops) = (distinguishing(pairs, true), distinguishing(pairs, false));
    let (start, stop, verbs, edges) = match (starts.first(), stops.first()) {
        (Some(a), Some(b)) => (
            word_condition(a),
            word_condition(b),
            vec![a.clone(), b.clone()],
            format!("\"{a}\" starts, \"{b}\" ends"),
        ),
        _ => match distinguishing_field(pairs) {
            Some((field, a, b)) => (
                ConditionCfg {
                    field: field.clone(),
                    op: "eq".into(),
                    value: Some(CondValue::Text(a.clone())),
                },
                ConditionCfg {
                    field: field.clone(),
                    op: "eq".into(),
                    value: Some(CondValue::Text(b.clone())),
                },
                Vec::new(),
                format!("{field} = {a} starts, {field} = {b} ends"),
            ),
            None => bail!(
                "the start and end examples do not differ by a word or a field; pick lines that say what happens (e.g. \"opened\" and \"closed\")"
            ),
        },
    };
    let mut keys: Vec<(KeySource, Vec<String>, u8)> = Vec::new();
    let key_name = "SpanKey".to_string();
    // A field both events of each pair carry with the same value.
    let mut field_keys = Vec::new();
    pairs[0].0.for_each_field(&mut |name, _| field_keys.push(name.to_string()));
    if let Some(f) = &hints.key_field {
        field_keys.retain(|n| n == f);
        if field_keys.is_empty() {
            field_keys.push(f.clone());
        }
    }
    for name in field_keys {
        let values: Option<Vec<String>> = pairs
            .iter()
            .map(|(a, b)| {
                let (x, y) = (a.text_of(&name)?, b.text_of(&name)?);
                (x == y && !x.is_empty()).then(|| x.into_owned())
            })
            .collect();
        let Some(values) = values else { continue };
        let distinct: BTreeSet<&String> = values.iter().collect();
        if pairs.len() > 1 && distinct.len() < pairs.len() && hints.key_field.is_none() {
            continue;
        }
        let rank = if hints.key_field.is_some() { 0 } else { 2 };
        keys.push((KeySource::Field { name: name.clone() }, values, rank));
    }
    // A token both lines of each pair share.
    if hints.key_field.is_none() {
        let first = message(pairs[0].0);
        let end_tokens: BTreeSet<&str> = tokens(message(pairs[0].1)).iter().map(|t| t.text).collect();
        for t in tokens(first) {
            if !end_tokens.contains(t.text) || verbs.iter().any(|v| v.eq_ignore_ascii_case(t.text)) {
                continue;
            }
            for pattern in key_patterns(first, &t, &verbs) {
                let Ok(re) = crate::user_regex::compile(&pattern, false) else {
                    continue;
                };
                let found: Option<Vec<String>> = pairs
                    .iter()
                    .map(|(a, b)| {
                        let (x, y) = (capture(&re, message(*a))?, capture(&re, message(*b))?);
                        (x == y).then_some(x)
                    })
                    .collect();
                let Some(found) = found else { continue };
                let distinct: BTreeSet<&String> = found.iter().collect();
                if pairs.len() > 1 && distinct.len() < pairs.len() {
                    continue;
                }
                // Keys with digits, anchored on a verb, are the most likely.
                let digits = found.iter().all(|k| k.chars().any(|c| c.is_ascii_digit()));
                let on_verb = pattern.contains("(?:");
                let rank = 1 + u8::from(!digits) * 2 + u8::from(!on_verb);
                if !keys
                    .iter()
                    .any(|(k, _, _)| matches!(k, KeySource::Pattern { regex, .. } if *regex == pattern))
                {
                    keys.push((
                        KeySource::Pattern {
                            name: key_name.clone(),
                            regex: pattern,
                        },
                        found,
                        rank,
                    ));
                }
            }
        }
    }
    if keys.is_empty() {
        bail!("no word or field is shared by each start and its end; name the field that pairs them, or pick other examples");
    }
    keys.sort_by_key(|(_, _, rank)| *rank);
    let mut out = Vec::new();
    for (key, example_keys, _) in keys.into_iter().take(MAX_CANDIDATES) {
        let by = match &key {
            KeySource::Field { name } => format!("the field {name}"),
            KeySource::Pattern { regex, .. } => format!("the text matched by {regex}"),
        };
        out.push(SpanCandidate {
            description: format!("{edges}; paired by {by}"),
            profile_override: override_toml(profile, &kind, &key, &start, &stop)?,
            key,
            example_keys,
            start: start.clone(),
            stop: stop.clone(),
            kind: kind.clone(),
        });
    }
    Ok(out)
}

/// Evaluates the `[[span]]` rule `kind` of a compiled profile over events in time order: the profile's
/// extractors and markers are applied to each event as an import would, and its runs are paired.
pub struct SpanEvaluator<'p> {
    profile: &'p CompiledProfile,
    rule: &'p SpanRule,
    classifier: Classifier,
    pairer: Pairer,
    /// Conditions an event must pass to be a start or stop at all (those on fields no extractor makes),
    /// per start/stop marker: a cheap test before extracting.
    gates: Vec<Vec<&'p crate::condition::Condition>>,
    events: u64,
}

impl<'p> SpanEvaluator<'p> {
    pub fn new(profile: &'p CompiledProfile, kind: &str) -> Result<SpanEvaluator<'p>> {
        let rule = profile
            .spans
            .iter()
            .find(|r| r.kind == kind)
            .ok_or_else(|| anyhow!("the profile has no [[span]] \"{kind}\""))?;
        if !rule.missing.is_empty() {
            bail!(
                "span \"{kind}\" names marker kinds the profile does not define: {}",
                rule.missing.join(", ")
            );
        }
        let made: BTreeSet<&str> = profile
            .extractors
            .iter()
            .flat_map(|e| e.fields.iter().map(|(n, _)| n.as_str()))
            .collect();
        let mask = rule.start_mask | rule.stop_mask;
        let gates = profile
            .markers
            .iter()
            .enumerate()
            .filter(|(i, _)| mask & (1 << i) != 0)
            .map(|(_, m)| m.when.iter().filter(|c| !made.contains(c.check().field())).collect())
            .collect();
        Ok(SpanEvaluator {
            profile,
            rule,
            classifier: Classifier::of_rule(rule),
            pairer: Pairer::new(rule.restart_on_start),
            gates,
            events: 0,
        })
    }

    /// Per start or stop marker of the rule, the conditions an event must pass to be one (those on fields
    /// no extractor makes): a store prefilters with them.
    pub fn gates(&self) -> &[Vec<&'p crate::condition::Condition>] {
        &self.gates
    }

    /// One event (in time order), `lane` its instance key.
    pub fn add(&mut self, rec: &Record, source: &str, lane: &str) {
        self.events += 1;
        let Some(ts) = rec.ts else { return };
        let row = crate::row::RecordRow { rec, source };
        if !self.gates.iter().any(|g| g.iter().all(|c| c.eval_row(&row))) {
            return;
        }
        let mut r = rec.clone();
        self.profile.apply_extractors(&mut r, source);
        r.markers |= self.profile.compute_markers(&r, source);
        let edge = self.classifier.edge(&r);
        if edge == Edge::Other {
            return;
        }
        let value = match &self.rule.per {
            super::Per::Instance => None,
            super::Per::Field(f) => r.field_text(f, source).map(|v| v.into_owned()),
        };
        self.pairer.feed(lane, value.as_deref(), edge, r.id, ts, || r.message.clone());
    }

    pub fn finish(self) -> SpanEvaluation {
        let runs = self.pairer.runs();
        let mut e = SpanEvaluation {
            events: self.events,
            dropped: self.pairer.dropped_runs + self.pairer.ignored_events,
            ..SpanEvaluation::default()
        };
        let mut keys = BTreeSet::new();
        let mut by_lane: HashMap<&str, Vec<(i64, i64)>> = HashMap::new();
        for r in &runs {
            match r.status {
                RunStatus::Closed => {
                    e.spans += 1;
                    let d = r.to - r.from;
                    e.longest_ms = Some(e.longest_ms.map_or(d, |m| m.max(d)));
                    e.shortest_ms = Some(e.shortest_ms.map_or(d, |m| m.min(d)));
                }
                RunStatus::Open => e.open += 1,
                RunStatus::NoStop => e.unmatched_starts += 1,
                RunStatus::NoStart => e.unmatched_ends += 1,
            }
            if r.status != RunStatus::NoStart {
                keys.insert(r.group.clone().unwrap_or_default());
                by_lane.entry(r.lane.as_str()).or_default().push((r.from, r.to));
            }
            if e.samples.len() < MAX_SAMPLES {
                e.samples.push(SpanSample {
                    key: r.group.clone().unwrap_or_default(),
                    lane: r.lane.clone(),
                    start_id: r.start.as_ref().map(|s| s.id),
                    stop_id: r.stop.as_ref().map(|s| s.id),
                    from: r.from,
                    to: r.to,
                    status: r.status,
                });
            }
        }
        e.distinct_keys = keys.len();
        // Most runs open at once per lane: a sweep over starts (+1) and ends (-1), ends first at a tie.
        for intervals in by_lane.values() {
            let mut points: Vec<(i64, i32)> = intervals.iter().flat_map(|(a, b)| [(*a, 1), (*b, -1)]).collect();
            points.sort_by_key(|(t, d)| (*t, *d));
            let mut open = 0i32;
            for (_, d) in points {
                open += d;
                e.max_concurrent = e.max_concurrent.max(open.max(0) as usize);
            }
        }
        e
    }
}

#[cfg(test)]
mod tests;
