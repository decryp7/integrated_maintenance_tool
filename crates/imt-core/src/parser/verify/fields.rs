//! Checks of proposed identity fields and metric extractors against the events a parser read (ADR 0032).

use crate::api::ParsedPreview;
use crate::profile::{ExtractCfg, FieldSpec, FieldType};
use crate::user_regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};

/// Most distinct values an identity field may have.
pub const MAX_IDENTITY_VALUES: usize = 256;
/// Most metrics one parser proposal carries.
pub const MAX_METRICS: usize = 12;

/// How a proposed identity field fared.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldCheck {
    pub field: String,
    /// Share of the events that carry it (0..1).
    pub coverage: f64,
    pub distinct: usize,
    pub accepted: bool,
    pub reason: String,
}

fn text(v: &Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::String(s) if s.trim().is_empty() => None,
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

/// Identity fields must be on (nearly) every event and stable: few values, each seen again and again.
pub fn check_identity(fields: &[String], records: &[ParsedPreview]) -> Vec<FieldCheck> {
    let events = records.len().max(1);
    fields
        .iter()
        .map(|field| {
            let values: Vec<String> = records.iter().filter_map(|r| r.fields.get(field).and_then(text)).collect();
            let distinct = values.iter().collect::<HashSet<_>>().len();
            let coverage = values.len() as f64 / events as f64;
            let (accepted, reason) = if values.is_empty() {
                (false, "no event has this field".to_string())
            } else if coverage < 0.9 {
                (false, format!("only {:.0}% of the events have it", coverage * 100.0))
            } else if distinct > MAX_IDENTITY_VALUES {
                (
                    false,
                    format!("{distinct} different values: it changes with every event, it is not an identity"),
                )
            } else if records.len() >= 9 && distinct * 3 > records.len() {
                (
                    false,
                    format!("{distinct} values in {} events: too many to tell processes apart", records.len()),
                )
            } else {
                (true, format!("{distinct} value(s) on {:.0}% of the events", coverage * 100.0))
            };
            FieldCheck {
                field: field.clone(),
                coverage,
                distinct,
                accepted,
                reason,
            }
        })
        .collect()
}

/// The unit of a metric, which decides what values are plausible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MetricUnit {
    Bytes,
    Kb,
    Mb,
    Gb,
    Percent,
    Ms,
    S,
    Count,
}

impl MetricUnit {
    /// The unit a model or user names (`MB`, `%`, `ms`, `threads`...); unknown names are counts.
    pub fn named(name: &str) -> MetricUnit {
        match name.trim().to_ascii_lowercase().as_str() {
            "b" | "byte" | "bytes" => MetricUnit::Bytes,
            "kb" | "kib" | "kilobytes" => MetricUnit::Kb,
            "mb" | "mib" | "megabytes" => MetricUnit::Mb,
            "gb" | "gib" | "gigabytes" => MetricUnit::Gb,
            "%" | "pct" | "percent" => MetricUnit::Percent,
            "ms" | "millisecond" | "milliseconds" => MetricUnit::Ms,
            "s" | "sec" | "second" | "seconds" => MetricUnit::S,
            _ => MetricUnit::Count,
        }
    }

    /// The unit label of the extracted field and chart.
    pub fn label(self) -> &'static str {
        match self {
            MetricUnit::Bytes => "B",
            MetricUnit::Kb => "KB",
            MetricUnit::Mb => "MB",
            MetricUnit::Gb => "GB",
            MetricUnit::Percent => "%",
            MetricUnit::Ms => "ms",
            MetricUnit::S => "s",
            MetricUnit::Count => "",
        }
    }

    fn plausible(self, v: f64) -> bool {
        v.is_finite()
            && v >= 0.0
            && match self {
                // A process's CPU share can pass 100% on several cores.
                MetricUnit::Percent => v <= 6400.0,
                MetricUnit::Ms => v < 1e10,
                MetricUnit::S => v < 1e7,
                MetricUnit::Count => v.fract() == 0.0 && v < 1e12,
                _ => v < 1e15,
            }
    }
}

/// A metric a model proposes: a regex over the message with a `(?P<value>...)` group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricRule {
    pub name: String,
    pub regex: String,
    #[serde(default)]
    pub unit: String,
}

/// How a proposed metric fared.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricCheck {
    /// The field name (letters, digits and `_`).
    pub name: String,
    pub unit: MetricUnit,
    /// Events whose message the regex matches.
    pub matches: usize,
    /// Share of the matches whose value is a plausible number (0..1).
    pub plausible: f64,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub accepted: bool,
    pub reason: String,
}

/// A field name made of letters, digits and `_` that starts with a letter.
pub fn field_name(name: &str) -> String {
    let mut out: String = name
        .trim()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .take(40)
        .collect();
    if !out.starts_with(|c: char| c.is_ascii_alphabetic()) {
        out.insert(0, 'm');
    }
    out
}

/// A number in `s` (`1,234.5`, `600`).
fn number(s: &str) -> Option<f64> {
    s.trim().replace([',', '_'], "").parse().ok()
}

/// Checks each rule on the events' messages: it compiles, matches some events, and its values are plausible
/// numbers for its unit.
pub fn check_metrics(rules: &[MetricRule], records: &[ParsedPreview]) -> Vec<MetricCheck> {
    let mut names = HashSet::new();
    rules
        .iter()
        .take(MAX_METRICS)
        .map(|rule| {
            let name = field_name(&rule.name);
            let unit = MetricUnit::named(&rule.unit);
            let mut check = MetricCheck {
                name: name.clone(),
                unit,
                matches: 0,
                plausible: 0.0,
                min: None,
                max: None,
                accepted: false,
                reason: String::new(),
            };
            if !names.insert(name.clone()) {
                check.reason = "another metric has this name".into();
                return check;
            }
            let re = match user_regex::compile(&rule.regex, false) {
                Ok(re) if re.capture_names().flatten().any(|n| n == "value") => re,
                Ok(_) => {
                    check.reason = "the regex needs a (?P<value>...) group around the number".into();
                    return check;
                }
                Err(e) => {
                    check.reason = format!("the regex does not compile: {e}");
                    return check;
                }
            };
            let values: Vec<Option<f64>> = records
                .iter()
                .filter_map(|r| re.captures(&r.message))
                .map(|c| c.name("value").and_then(|m| number(m.as_str())))
                .collect();
            check.matches = values.len();
            let good: Vec<f64> = values.iter().flatten().copied().filter(|v| unit.plausible(*v)).collect();
            check.plausible = good.len() as f64 / values.len().max(1) as f64;
            check.min = good.iter().copied().reduce(f64::min);
            check.max = good.iter().copied().reduce(f64::max);
            (check.accepted, check.reason) = if values.is_empty() {
                (false, "it matches no event".into())
            } else if check.plausible < 0.95 {
                (
                    false,
                    format!(
                        "only {:.0}% of its {} values are plausible numbers in {}",
                        check.plausible * 100.0,
                        values.len(),
                        unit_text(unit)
                    ),
                )
            } else {
                (
                    true,
                    format!("{} events, {}..{} {}", values.len(), fmt(check.min), fmt(check.max), unit.label()),
                )
            };
            check
        })
        .collect()
}

fn unit_text(u: MetricUnit) -> &'static str {
    match u {
        MetricUnit::Count => "counts (whole numbers)",
        MetricUnit::Percent => "percent",
        _ => u.label(),
    }
}

fn fmt(v: Option<f64>) -> String {
    v.map_or_else(String::new, |v| format!("{v}"))
}

/// The parser's `[[extract]]` for the accepted metrics: one extractor per metric, its value a number with a unit.
pub fn metric_extractors(rules: &[MetricRule], checks: &[MetricCheck]) -> Vec<ExtractCfg> {
    rules
        .iter()
        .zip(checks)
        .filter(|(_, c)| c.accepted)
        .map(|(rule, c)| {
            let unit = c.unit.label();
            let spec = FieldSpec {
                kind: if c.unit == MetricUnit::Count {
                    FieldType::Integer
                } else {
                    FieldType::Number
                },
                unit: (!unit.is_empty()).then(|| unit.to_string()),
                unit_from: None,
                to: None,
            };
            ExtractCfg {
                name: format!("metric-{}", c.name.to_ascii_lowercase().replace('_', "-")),
                from: "@message".into(),
                regex: rule.regex.replacen("(?P<value>", &format!("(?P<{}>", c.name), 1),
                when: Vec::new(),
                fields: BTreeMap::from([(c.name.clone(), spec)]),
            }
        })
        .collect()
}

/// Profile `[[chart]]` sections charting each accepted metric per instance (Metrics view).
pub fn metric_charts_toml(checks: &[MetricCheck]) -> String {
    let mut out = String::new();
    for c in checks.iter().filter(|c| c.accepted) {
        out.push_str(&format!(
            "[[chart]]\nid = \"metric-{}\"\ntitle = \"{} per instance\"\ny = \"avg({})\"\nbucket = \"auto\"\nsplit_by = [\"@group\"]\ntype = \"line\"\nunit = \"{}\"\n\n",
            c.name.to_ascii_lowercase().replace('_', "-"),
            c.name,
            c.name,
            c.unit.label()
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Level;
    use serde_json::{json, Map};

    fn rec(message: &str, fields: Value) -> ParsedPreview {
        ParsedPreview {
            line: 1,
            ts: Some(0),
            level: Level::Information,
            message: message.into(),
            exception: None,
            fields: match fields {
                Value::Object(m) => m,
                _ => Map::new(),
            },
            markers: Vec::new(),
        }
    }

    #[test]
    fn identity_fields_must_be_present_and_stable() {
        let records: Vec<ParsedPreview> = (0..30)
            .map(|i| {
                rec(
                    "x",
                    json!({"Machine": "WEB01", "Pid": 4100 + i % 3, "Request": format!("r{i}"), "Rare": if i == 0 { "a" } else { "" }}),
                )
            })
            .collect();
        let checks = check_identity(
            &["Machine".into(), "Pid".into(), "Request".into(), "Rare".into(), "Nope".into()],
            &records,
        );
        let verdicts: Vec<(&str, bool, usize)> = checks.iter().map(|c| (c.field.as_str(), c.accepted, c.distinct)).collect();
        assert_eq!(
            verdicts,
            [
                ("Machine", true, 1),
                ("Pid", true, 3),
                ("Request", false, 30),
                ("Rare", false, 1),
                ("Nope", false, 0)
            ]
        );
        assert!(checks[2].reason.contains("too many"), "{}", checks[2].reason);
        assert!(checks[3].reason.contains("%"));
        let many: Vec<ParsedPreview> = (0..1000).map(|i| rec("x", json!({"T": i % 300}))).collect();
        assert!(check_identity(&["T".into()], &many)[0].reason.contains("300 different values"));
    }

    #[test]
    fn metrics_are_numbers_plausible_for_their_unit() {
        // Invented process statistics, as a monitor might write them.
        let records: Vec<ParsedPreview> = (0..20)
            .map(|i| {
                rec(
                    &format!("WorkingSet {} MB · CPU {}.{}% · Threads {}", 600 + i, 20 + i, i % 10, 60 + i),
                    Value::Null,
                )
            })
            .chain([rec("unrelated line", Value::Null)])
            .collect();
        let rules = [
            MetricRule {
                name: "WorkingSet".into(),
                regex: r"WorkingSet (?P<value>[\d.]+) MB".into(),
                unit: "MB".into(),
            },
            MetricRule {
                name: "cpu %".into(),
                regex: r"CPU (?P<value>[\d.]+)%".into(),
                unit: "%".into(),
            },
            MetricRule {
                name: "Threads".into(),
                regex: r"Threads (?P<value>\d+)".into(),
                unit: "threads".into(),
            },
            MetricRule {
                name: "Cpu as count".into(),
                regex: r"CPU (?P<value>[\d.]+)%".into(),
                unit: "count".into(),
            },
            MetricRule {
                name: "nothing".into(),
                regex: r"Handles (?P<value>\d+)".into(),
                unit: "".into(),
            },
            MetricRule {
                name: "nogroup".into(),
                regex: r"Threads (\d+)".into(),
                unit: "".into(),
            },
            MetricRule {
                name: "bad".into(),
                regex: r"(?P<value>[".into(),
                unit: "".into(),
            },
            MetricRule {
                name: "Threads".into(),
                regex: r"Threads (?P<value>\d+)".into(),
                unit: "".into(),
            },
        ];
        let checks = check_metrics(&rules, &records);
        let accepted: Vec<(&str, bool)> = checks.iter().map(|c| (c.name.as_str(), c.accepted)).collect();
        assert_eq!(
            accepted,
            [
                ("WorkingSet", true),
                ("cpu__", true),
                ("Threads", true),
                ("Cpu_as_count", false),
                ("nothing", false),
                ("nogroup", false),
                ("bad", false),
                ("Threads", false)
            ]
        );
        assert_eq!((checks[0].matches, checks[0].min, checks[0].max), (20, Some(600.0), Some(619.0)));
        assert_eq!(checks[2].unit, MetricUnit::Count);
        assert!(checks[3].reason.contains("whole numbers"), "{}", checks[3].reason);
        assert!(checks[7].reason.contains("another metric"));
        let extract = metric_extractors(&rules, &checks);
        assert_eq!(extract.len(), 3);
        assert_eq!(extract[0].regex, r"WorkingSet (?P<WorkingSet>[\d.]+) MB");
        assert_eq!(extract[0].fields["WorkingSet"].unit.as_deref(), Some("MB"));
        assert_eq!(extract[2].fields["Threads"].kind, FieldType::Integer);
        let charts = metric_charts_toml(&checks);
        assert_eq!(charts.matches("[[chart]]").count(), 3);
        assert!(charts.contains("y = \"avg(WorkingSet)\"") && charts.contains("split_by = [\"@group\"]"));
        // The charts are a valid profile fragment.
        crate::Profile::from_toml(&charts).unwrap();
        assert_eq!(field_name("9 lives"), "m9_lives");
        for (u, l) in [("b", "B"), ("KB", "KB"), ("gb", "GB"), ("ms", "ms"), ("sec", "s")] {
            assert_eq!(MetricUnit::named(u).label(), l);
        }
        assert!(!MetricUnit::Ms.plausible(-1.0) && !MetricUnit::S.plausible(1e8) && MetricUnit::Gb.plausible(3.5));
    }
}
