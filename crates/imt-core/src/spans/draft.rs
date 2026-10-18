//! The guided "Runs" editor: a candidate rule ([`SpanDraft`]) is validated
//! field by field, turned into the profile changes saving it needs
//! ([`SpanPlan`]) and classified over the loaded records for the preview.

use super::config::{self, check_kinds, mask_of, SpanCfg, SpanField, SpanIssue, SpanRule};
use super::dto::{SpanDraft, SpanPlan, Trigger};
use super::pairing::Edge;
use crate::model::{MarkerMask, Record, MAX_MARKER_KINDS};
use crate::profile::{Marker, Profile};
use regex::Regex;
use serde_json::{json, Value};

/// Most `[[marker]]` rules a profile may have (see `CompiledProfile::compile_with`).
const MAX_PROFILE_MARKERS: usize = 32;
/// Colours of generated start and stop markers (those of the built-in start/stop).
const START_COLOR: &str = "#3FBF7F";
const STOP_COLOR: &str = "#A7B0C2";

/// Decides whether a record starts or stops a run of the draft.
#[derive(Debug, Clone)]
enum Matcher {
    Mask(MarkerMask),
    Regex(Regex),
}

impl Matcher {
    fn hit(&self, r: &Record) -> bool {
        match self {
            Matcher::Mask(m) => r.markers & m != 0,
            Matcher::Regex(re) => re.is_match(&r.message),
        }
    }
}

/// Classifies records for one draft.
#[derive(Debug, Clone)]
pub struct Classifier {
    start: Matcher,
    stop: Matcher,
}

impl Classifier {
    /// A record matching both a start and a stop counts as a start: messages like
    /// "Application started. Press Ctrl+C to shut down." name both.
    pub fn edge(&self, r: &Record) -> Edge {
        match (self.start.hit(r), self.stop.hit(r)) {
            (true, _) => Edge::Start,
            (false, true) => Edge::Stop,
            (false, false) => Edge::Other,
        }
    }

    /// From a compiled rule's marker bits.
    pub fn of_rule(rule: &SpanRule) -> Classifier {
        Classifier {
            start: Matcher::Mask(rule.start_mask),
            stop: Matcher::Mask(rule.stop_mask),
        }
    }
}

/// A checked draft: the rule, how to classify records for it, and what saving writes.
#[derive(Debug, Clone)]
pub struct Checked {
    pub rule: SpanRule,
    pub classifier: Classifier,
    pub plan: SpanPlan,
}

/// Validates a draft against the profile and its effective markers. Errors
/// block saving; warnings do not.
pub fn check(draft: &SpanDraft, profile: &Profile, markers: &[Marker]) -> (Vec<SpanIssue>, Vec<String>, Option<Checked>) {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let kind = draft.kind.trim().to_string();
    let label = draft.label.trim().to_string();
    let mut new_markers: Vec<Value> = Vec::new();

    let mut side = |trigger: &Trigger, field: SpanField, role: &str, color: &str| -> (Vec<String>, Option<Matcher>) {
        match trigger {
            Trigger::Markers { kinds } => {
                let kinds: Vec<String> = kinds.iter().map(|k| k.trim().to_string()).collect();
                let before = errors.len();
                check_kinds(&kinds, field, &mut errors);
                if errors.len() > before {
                    return (kinds, None);
                }
                let (mask, missing) = mask_of(&kinds, markers);
                if !missing.is_empty() {
                    errors.push(SpanIssue {
                        field,
                        message: format!("marker \"{}\" is not defined (choose one from the list)", missing.join("\", \"")),
                    });
                    return (kinds, None);
                }
                (kinds, Some(Matcher::Mask(mask)))
            }
            Trigger::Pattern { pattern, case_sensitive } => {
                let marker_kind = format!("{kind}-{role}");
                if pattern.trim().is_empty() {
                    errors.push(SpanIssue {
                        field,
                        message: format!("enter the text that marks a {role}, or choose a marker"),
                    });
                    return (vec![marker_kind], None);
                }
                match crate::user_regex::compile(pattern, !case_sensitive) {
                    Ok(re) => {
                        let value = if *case_sensitive {
                            pattern.clone()
                        } else {
                            format!("(?i){pattern}")
                        };
                        let name = if label.is_empty() { kind.clone() } else { label.clone() };
                        new_markers.push(json!({
                            "kind": marker_kind,
                            "label": format!("{name}: {role}"),
                            "lifecycle": true,
                            "color": color,
                            "description": format!("The {role} of a \"{name}\" run (created by the Runs editor)"),
                            "when": [{ "field": "@message", "op": "matches", "value": value }],
                        }));
                        (vec![marker_kind], Some(Matcher::Regex(re)))
                    }
                    Err(e) => {
                        errors.push(SpanIssue {
                            field,
                            message: format!("the {role} pattern is not a valid regex: {e}"),
                        });
                        (vec![marker_kind], None)
                    }
                }
            }
        }
    };
    let (start_kinds, start) = side(&draft.start, SpanField::Start, "start", START_COLOR);
    let (stop_kinds, stop) = side(&draft.stop, SpanField::Stop, "stop", STOP_COLOR);

    let cfg = SpanCfg {
        kind: kind.clone(),
        label: (!label.is_empty()).then(|| label.clone()),
        start: one_or_many(&start_kinds),
        stop: one_or_many(&stop_kinds),
        per: draft.per.trim().to_string(),
        color: draft.color.clone().filter(|c| !c.trim().is_empty()),
        max_duration: draft.max_duration.clone().filter(|d| !d.trim().is_empty()),
        restart_on_start: draft.restart_on_start.unwrap_or(true),
        enabled: true,
    };
    // Trigger problems are reported above, with friendlier wording.
    errors.extend(
        config::check(&cfg)
            .into_iter()
            .filter(|i| !matches!(i.field, SpanField::Start | SpanField::Stop)),
    );
    errors.sort_by_key(|i| i.field as u8);

    let new_kinds: Vec<String> = new_markers.iter().filter_map(|m| m["kind"].as_str().map(str::to_string)).collect();
    let added = new_kinds.iter().filter(|k| !profile.marker.iter().any(|m| m.kind == **k)).count();
    for k in new_kinds.iter().filter(|k| profile.marker.iter().any(|m| m.kind == **k)) {
        warnings.push(format!("saving updates the marker rule \"{k}\""));
    }
    if added > 0 && profile.marker.len() + added > MAX_PROFILE_MARKERS.min(MAX_MARKER_KINDS) {
        errors.push(SpanIssue {
            field: SpanField::General,
            message: format!(
                "the profile has {} marker rules; the text patterns need {added} more and at most {MAX_PROFILE_MARKERS} are supported — choose existing markers instead",
                profile.marker.len()
            ),
        });
    }
    if profile.span.iter().any(|s| s.kind == kind) {
        warnings.push(format!("saving replaces the existing run rule \"{kind}\""));
    } else if profile.span.len() >= config::MAX_SPAN_RULES {
        errors.push(SpanIssue {
            field: SpanField::General,
            message: format!("at most {} run rules are supported; remove one first", config::MAX_SPAN_RULES),
        });
    }

    let (Some(start), Some(stop), true) = (start, stop, errors.is_empty()) else {
        return (errors, warnings, None);
    };
    let rule = match config::compile(std::slice::from_ref(&cfg)) {
        Ok(mut rules) => rules.remove(0),
        Err(e) => {
            errors.push(SpanIssue {
                field: SpanField::General,
                message: e.to_string(),
            });
            return (errors, warnings, None);
        }
    };
    let span = span_table(&cfg);
    let mut spans: Vec<Value> = profile
        .span
        .iter()
        .map(|s| if s.kind == cfg.kind { span.clone() } else { json!(s) })
        .collect();
    if !profile.span.iter().any(|s| s.kind == cfg.kind) {
        spans.push(span.clone());
    }
    let plan = SpanPlan {
        span,
        spans,
        markers: new_markers,
    };
    (
        errors,
        warnings,
        Some(Checked {
            rule,
            classifier: Classifier { start, stop },
            plan,
        }),
    )
}

fn one_or_many(kinds: &[String]) -> config::OneOrMany {
    match kinds {
        [one] => config::OneOrMany::One(one.clone()),
        _ => config::OneOrMany::Many(kinds.to_vec()),
    }
}

/// The `[[span]]` table as the profile writes it: defaults left out.
fn span_table(cfg: &SpanCfg) -> Value {
    let mut t = serde_json::Map::new();
    t.insert("kind".into(), json!(cfg.kind));
    if let Some(l) = &cfg.label {
        t.insert("label".into(), json!(l));
    }
    t.insert("start".into(), json!(cfg.start));
    t.insert("stop".into(), json!(cfg.stop));
    t.insert("per".into(), json!(cfg.per));
    if let Some(c) = &cfg.color {
        t.insert("color".into(), json!(c));
    }
    if let Some(d) = &cfg.max_duration {
        t.insert("max_duration".into(), json!(d));
    }
    if !cfg.restart_on_start {
        t.insert("restart_on_start".into(), json!(false));
    }
    Value::Object(t)
}
