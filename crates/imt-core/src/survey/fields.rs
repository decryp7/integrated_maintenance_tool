//! Field statistics of the sample and the choice of an instance key (the field that splits the data into a
//! sensible number of timeline lanes).

use super::dto::{FieldKind, FieldSummary, InstanceCandidate, KeyOrigin, ValueCount};
use super::sample::Sample;
use crate::model::value_text;
use regex::Regex;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

/// Distinct values counted per field.
const MAX_DISTINCT: usize = 256;
/// Fields tracked (the most common ones are kept).
const MAX_FIELDS: usize = 200;
/// More lanes than this make a timeline unreadable.
pub const MAX_LANES: usize = 64;
const TOP_VALUES: usize = 5;

#[derive(Default)]
struct FieldAcc {
    present: usize,
    numbers: usize,
    bools: usize,
    texts: usize,
    values: HashMap<String, usize>,
    capped: bool,
    min: Option<f64>,
    max: Option<f64>,
}

/// Statistics of every field on the sampled events, most common first.
pub fn field_summaries(events: &[Sample]) -> Vec<FieldSummary> {
    let mut accs: BTreeMap<String, FieldAcc> = BTreeMap::new();
    for e in events {
        for (k, v) in &e.rec.fields {
            if !accs.contains_key(k) && accs.len() >= MAX_FIELDS {
                continue;
            }
            let acc = accs.entry(k.clone()).or_default();
            acc.present += 1;
            match v {
                Value::Number(n) => {
                    acc.numbers += 1;
                    if let Some(f) = n.as_f64() {
                        acc.min = Some(acc.min.map_or(f, |m| m.min(f)));
                        acc.max = Some(acc.max.map_or(f, |m| m.max(f)));
                    }
                }
                Value::Bool(_) => acc.bools += 1,
                _ => acc.texts += 1,
            }
            let text = value_text(v);
            if let Some(c) = acc.values.get_mut(text.as_ref()) {
                *c += 1;
            } else if acc.values.len() < MAX_DISTINCT {
                acc.values.insert(text.into_owned(), 1);
            } else {
                acc.capped = true;
            }
        }
    }
    let total = events.len().max(1);
    let mut out: Vec<FieldSummary> = accs
        .into_iter()
        .map(|(name, a)| {
            let kind = if a.numbers == a.present {
                FieldKind::Number
            } else if a.bools == a.present {
                FieldKind::Bool
            } else if a.texts == a.present {
                FieldKind::Text
            } else {
                FieldKind::Mixed
            };
            let mut top: Vec<ValueCount> = a
                .values
                .iter()
                .map(|(v, c)| ValueCount {
                    value: v.clone(),
                    count: *c,
                })
                .collect();
            top.sort_by(|x, y| y.count.cmp(&x.count).then(x.value.cmp(&y.value)));
            top.truncate(TOP_VALUES);
            FieldSummary {
                name,
                kind,
                present: a.present,
                coverage: a.present as f64 / total as f64,
                distinct: a.values.len(),
                distinct_capped: a.capped,
                top,
                min: a.min,
                max: a.max,
            }
        })
        .collect();
    out.sort_by(|a, b| b.present.cmp(&a.present).then(a.name.cmp(&b.name)));
    out
}

/// How much a field's name suggests an instance (machine, worker, process...); `None` = never a key.
pub fn name_bonus(name: &str, configured: &[String]) -> Option<f64> {
    let n = name.to_ascii_lowercase().replace(['_', '-', '.'], "");
    const NEVER: &[&str] = &[
        "sourcecontext",
        "logger",
        "category",
        "class",
        "thread",
        "threadid",
        "level",
        "message",
        "requestpath",
        "path",
        "method",
        "requestmethod",
        "statuscode",
        "correlationid",
        "traceid",
        "spanid",
        "requestid",
        "user",
        "username",
        "userid",
        "version",
        "address",
        "eventid",
    ];
    if NEVER.contains(&n.as_str()) {
        return None;
    }
    const STRONG: &[&str] = &[
        "machine",
        "host",
        "computer",
        "node",
        "pod",
        "instance",
        "container",
        "server",
        "worker",
    ];
    const MEDIUM: &[&str] = &["application", "app", "service", "process", "pid"];
    let mut bonus = if STRONG.iter().any(|s| n.contains(s)) {
        3.0
    } else if MEDIUM.iter().any(|s| n.contains(s)) {
        2.0
    } else {
        0.0
    };
    if configured.iter().any(|c| c.eq_ignore_ascii_case(name)) {
        bonus += 1.0;
    }
    Some(bonus)
}

fn lanes_fit(lanes: usize) -> f64 {
    match lanes {
        2..=16 => 1.0,
        17..=32 => 0.5,
        _ => 0.2,
    }
}

fn message_key_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\b(worker|node|instance|host|pod|shard|replica|pid)[ #:=_-]?([A-Za-z0-9][\w.-]{0,30})\b")
            .unwrap_or_else(|_| unreachable!("constant regex"))
    })
}

/// Fields (or words in messages) that split the sample into 2..=[`MAX_LANES`] lanes, best first.
pub fn instance_candidates(events: &[Sample], fields: &[FieldSummary], configured: &[String]) -> Vec<InstanceCandidate> {
    let mut out = Vec::new();
    for f in fields {
        let Some(bonus) = name_bonus(&f.name, configured) else { continue };
        let numeric_ok = f.kind != FieldKind::Number || bonus >= 2.0;
        if f.kind == FieldKind::Bool || !numeric_ok || f.distinct_capped || !(2..=MAX_LANES).contains(&f.distinct) {
            continue;
        }
        if f.coverage < 0.3 || (bonus == 0.0 && f.coverage < 0.9) {
            continue;
        }
        let penalty = if bonus == 0.0 { 1.0 } else { 0.0 };
        out.push(InstanceCandidate {
            field: f.name.clone(),
            origin: KeyOrigin::Field,
            lanes: f.distinct,
            coverage: f.coverage,
            score: bonus + f.coverage * 2.0 + lanes_fit(f.distinct) - penalty,
            examples: f.top.iter().map(|v| v.value.clone()).collect(),
        });
    }
    out.extend(message_candidates(events, fields));
    out.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.field.cmp(&b.field)));
    out.truncate(8);
    out
}

/// Instance names written in messages ("worker-3 picked up job", "[node 2] ..."): values must contain a digit.
fn message_candidates(events: &[Sample], fields: &[FieldSummary]) -> Vec<InstanceCandidate> {
    let re = message_key_regex();
    let mut words: BTreeMap<String, (usize, HashMap<String, usize>)> = BTreeMap::new();
    for e in events {
        let msg: String = e.rec.message.chars().take(512).collect();
        if let Some(c) = re.captures(&msg) {
            let (Some(word), Some(value)) = (c.get(1), c.get(2)) else {
                continue;
            };
            if !value.as_str().bytes().any(|b| b.is_ascii_digit()) {
                continue;
            }
            let entry = words.entry(word.as_str().to_ascii_lowercase()).or_default();
            entry.0 += 1;
            if entry.1.len() <= MAX_LANES || entry.1.contains_key(value.as_str()) {
                *entry.1.entry(value.as_str().to_string()).or_default() += 1;
            }
        }
    }
    let total = events.len().max(1);
    words
        .into_iter()
        .filter_map(|(word, (hits, values))| {
            let mut name: String = word.clone();
            if let Some(first) = name.get_mut(0..1) {
                first.make_ascii_uppercase();
            }
            let coverage = hits as f64 / total as f64;
            let exists = fields.iter().any(|f| f.name.eq_ignore_ascii_case(&name));
            if exists || coverage < 0.3 || !(2..=MAX_LANES).contains(&values.len()) {
                return None;
            }
            let mut examples: Vec<(String, usize)> = values.into_iter().collect();
            examples.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            let lanes = examples.len();
            Some(InstanceCandidate {
                field: name.clone(),
                origin: KeyOrigin::Message {
                    regex: format!(r"(?i)\b{word}[ #:=_-]?(?P<{name}>[A-Za-z0-9][\w.-]{{0,30}})\b"),
                },
                lanes,
                coverage,
                // Below a real field of the same quality: the key needs an extractor.
                score: 2.5 + coverage * 2.0 + lanes_fit(lanes) - 0.5,
                examples: examples.into_iter().take(TOP_VALUES).map(|e| e.0).collect(),
            })
        })
        .collect()
}
