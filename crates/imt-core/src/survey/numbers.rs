//! Numbers written inside messages ("took 23 ms", "CPU 23.3%", "queue=17"): candidates for extractors.
//! Events that carry a message template are skipped: their numbers are properties already.

use super::dto::{NumberCandidate, NumberKind};
use super::sample::Sample;
use regex::Regex;
use std::collections::{BTreeMap, HashSet};
use std::sync::OnceLock;

const MAX_PER_MESSAGE: usize = 8;
const MAX_CANDIDATES: usize = 12;
const MIN_COUNT: usize = 5;
/// Characters of a message scanned.
const SCAN_CHARS: usize = 1024;

fn number_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"\b(?P<label>[A-Za-z][A-Za-z_]{1,30})(?P<sep>[ \t]*[=:][ \t]*|[ \t]+)(?P<num>-?\d+(?:\.\d+)?)(?:[ \t]?(?P<unit>ms|%|s|sec|KB|MB|GB|bytes))?")
            .unwrap_or_else(|_| unreachable!("constant regex"))
    })
}

/// Words that never name a measurement on their own ("in 5" is a position, "port 80" a setting).
const NOT_A_METRIC: &[&str] = &[
    "in", "at", "of", "to", "on", "for", "by", "is", "was", "the", "a", "an", "id", "no", "port", "version", "v", "line", "code", "order",
    "user", "pid", "thread", "attempt", "step", "page", "item", "row", "http", "status", "and", "or", "from", "with", "year", "day",
];
/// Words naming a code or an identifier, never a measurement, whatever follows them.
const IDENTIFIERS: &[&str] = &[
    "error", "code", "id", "state", "severity", "pid", "spid", "port", "line", "version", "event", "eventid", "client", "errno",
];
/// Words before a duration that mean "how long it took".
const ELAPSED: &[&str] = &["in", "took", "after", "elapsed", "duration", "within", "time"];

#[derive(Default)]
struct Acc {
    label: String,
    sep_eq: bool,
    unit: Option<String>,
    count: usize,
    min: f64,
    max: f64,
    values: HashSet<u64>,
    example: String,
}

/// The field name for a label and unit: `CPU` + `%` is `CpuPercent`, `took` + `ms` is `ElapsedMs`.
fn field_name(label: &str, unit: Option<&str>) -> (NumberKind, String) {
    let (kind, suffix) = unit_kind(unit);
    let low = label.to_ascii_lowercase();
    let base = if kind == NumberKind::Duration && ELAPSED.contains(&low.as_str()) {
        "Elapsed".to_string()
    } else {
        pascal(label)
    };
    (kind, format!("{base}{suffix}"))
}

/// Text around a number that makes it part of something else: a path or file position (`x.go:250`),
/// an address (`10.20.1.15`), a range or a time (`659:3745`).
fn embedded(msg: &str, label_start: usize, end: usize) -> bool {
    let before = msg[..label_start].chars().next_back();
    let mut after = msg[end..].chars();
    let next = after.next();
    before.is_some_and(|c| matches!(c, '.' | '/' | '\\' | '_' | '-'))
        || next.is_some_and(|c| c.is_alphanumeric() || c == ':')
        || (next == Some('.') && after.next().is_some_and(|c| c.is_ascii_digit()))
}

/// Measurements written in the sampled messages, most frequent first. A number is skipped on events that
/// already carry a field of the name it would get (structured logs have it as a property).
pub fn number_candidates(events: &[Sample]) -> Vec<NumberCandidate> {
    let re = number_re();
    let mut accs: BTreeMap<(String, Option<String>), Acc> = BTreeMap::new();
    for e in events.iter().filter(|e| e.rec.template.as_deref().is_none_or(|t| !t.contains('{'))) {
        let msg: String = e.rec.message.chars().take(SCAN_CHARS).collect();
        for c in re.captures_iter(&msg).take(MAX_PER_MESSAGE) {
            let (Some(whole), Some(label), Some(num), Some(sep)) = (c.get(0), c.name("label"), c.name("num"), c.name("sep")) else {
                continue;
            };
            if embedded(&msg, label.start(), whole.end()) {
                continue;
            }
            let unit = c.name("unit").map(|u| u.as_str().to_string());
            let sep_eq = sep.as_str().contains(['=', ':']);
            let l = label.as_str();
            let lower = l.to_ascii_lowercase();
            if IDENTIFIERS.contains(&lower.as_str()) || (unit.is_none() && !sep_eq && NOT_A_METRIC.contains(&lower.as_str())) {
                continue;
            }
            let (_, name) = field_name(l, unit.as_deref());
            if e.rec
                .fields
                .keys()
                .any(|k| k.eq_ignore_ascii_case(&name) || k.eq_ignore_ascii_case(l))
            {
                continue;
            }
            let Ok(value) = num.as_str().parse::<f64>() else { continue };
            let acc = accs.entry((l.to_string(), unit.clone())).or_insert_with(|| Acc {
                label: l.to_string(),
                sep_eq,
                unit: unit.clone(),
                min: value,
                max: value,
                example: msg.chars().take(240).collect(),
                ..Acc::default()
            });
            acc.count += 1;
            acc.min = acc.min.min(value);
            acc.max = acc.max.max(value);
            if acc.values.len() < 1024 {
                acc.values.insert(value.to_bits());
            }
        }
    }
    let mut names: HashSet<String> = HashSet::new();
    let mut out: Vec<NumberCandidate> = accs
        .into_values()
        .filter(|a| a.count >= MIN_COUNT)
        // A constant is a setting, not a measurement.
        .filter(|a| a.min < a.max)
        // Unique values on every event are identifiers, not measurements (unless they carry a unit).
        .filter(|a| a.unit.is_some() || a.sep_eq || a.values.len() * 10 < a.count * 9)
        .filter_map(|a| {
            let (kind, name) = field_name(&a.label, a.unit.as_deref());
            if !names.insert(name.clone()) {
                return None;
            }
            let sep = if a.sep_eq { r"\s*[=:]\s*" } else { r"\s+" };
            let unit = a.unit.as_deref().map(|u| format!(r"\s?{}", regex::escape(u))).unwrap_or_default();
            Some(NumberCandidate {
                regex: format!(r"\b{}{sep}(?P<{name}>-?\d+(?:\.\d+)?){unit}", regex::escape(&a.label)),
                name,
                label: a.label,
                unit: a.unit,
                number_kind: kind,
                count: a.count,
                min: a.min,
                max: a.max,
                example: a.example,
            })
        })
        .collect();
    out.sort_by(|a, b| b.count.cmp(&a.count).then(a.name.cmp(&b.name)));
    out.truncate(MAX_CANDIDATES);
    out
}

fn unit_kind(unit: Option<&str>) -> (NumberKind, &'static str) {
    match unit {
        Some("ms") => (NumberKind::Duration, "Ms"),
        Some("s" | "sec") => (NumberKind::Duration, "Ms"),
        Some("%") => (NumberKind::Number, "Percent"),
        Some("KB" | "MB" | "GB" | "bytes") => (NumberKind::Bytes, "Mb"),
        _ => (NumberKind::Number, ""),
    }
}

/// `WorkingSet` stays, `CPU` becomes `Cpu`, `queue_depth` becomes `QueueDepth`.
fn pascal(label: &str) -> String {
    label
        .split('_')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let all_caps = p.chars().all(|c| c.is_ascii_uppercase());
            let mut cs = p.chars();
            match cs.next() {
                Some(f) => {
                    let rest: String = cs.collect();
                    let rest = if all_caps { rest.to_ascii_lowercase() } else { rest };
                    format!("{}{rest}", f.to_ascii_uppercase())
                }
                None => String::new(),
            }
        })
        .collect()
}

#[cfg(test)]
pub(super) fn pascal_for_tests(label: &str) -> String {
    pascal(label)
}
