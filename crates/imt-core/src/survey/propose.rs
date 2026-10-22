//! Heuristic proposals (no AI): each survey fact that suggests a profile change becomes a [`Candidate`]
//! with typed fragments. Nothing here checks that a candidate works; [`super::validate()`] does.

use super::dto::*;
use crate::condition::{CondValue, ConditionCfg};
use crate::dashboard::{DashboardCfg, TileCfg, TileKind};
use crate::patterns::WILDCARD;
use crate::pipeline::StepCfg;
use crate::profile::{ChartCfg, ExtractCfg, FieldSpec, FieldType, MarkerCfg};
use crate::spans::{OneOrMany, SpanCfg};
use crate::Profile;
use std::collections::{BTreeMap, HashSet};

const MAX_INSTANCE: usize = 3;
const MAX_METRIC_EXTRACTORS: usize = 6;
const RECOMMENDED_EXTRACTORS: usize = 3;
const MAX_QUERIES: usize = 3;
/// Text formats tried on unrecognised files (named groups as in a parser's `[format] regexes`).
const FORMATS: &[(&str, &str)] = &[
    (
        "day-first or US date",
        r"^\[?(?P<timestamp>\d{1,2}[-/.]\d{1,2}[-/.]\d{2,4}[ T,]\d{1,2}:\d{2}:\d{2}(?:[.,]\d{1,9})?)\]?\s*(?:\[?(?P<level>(?i:TRACE|DEBUG|INFO|WARN|WARNING|ERROR|FATAL|CRITICAL))\]?\s*[:|-]?\s*)?(?P<message>.*)$",
    ),
    (
        "ISO timestamp",
        r"^\[?(?P<timestamp>\d{4}[-/.]\d{2}[-/.]\d{2}[ T,]\d{2}:\d{2}:\d{2}(?:[.,]\d{1,9})?(?:Z|[+-]\d{2}:?\d{2})?)\]?\s*(?:\[?(?P<level>(?i:TRACE|DEBUG|INFO|WARN|WARNING|ERROR|FATAL|CRITICAL))\]?\s*[:|-]?\s*)?(?P<message>.*)$",
    ),
    (
        "time of day",
        r"^\[?(?P<timestamp>\d{2}:\d{2}:\d{2}(?:[.,]\d{1,9})?)\]?\s+(?:\[?(?P<level>(?i:TRACE|DEBUG|INFO|WARN|WARNING|ERROR|FATAL|CRITICAL))\]?\s*[:|-]?\s*)?(?P<message>.*)$",
    ),
];

fn short(s: &str, n: usize) -> String {
    let mut out: String = s.chars().take(n).collect();
    if s.chars().count() > n {
        out.push('…');
    }
    out
}

/// A name not yet in `taken` (`base`, `base-2`, ...), reserved.
fn unique(base: &str, taken: &mut HashSet<String>) -> String {
    let mut name = base.to_string();
    let mut n = 2;
    while taken.contains(&name) {
        name = format!("{base}-{n}");
        n += 1;
    }
    taken.insert(name.clone());
    name
}

/// All heuristic candidates for a survey.
pub fn propose(survey: &FolderSurvey, profile: &Profile) -> Vec<Candidate> {
    let mut out = Vec::new();
    out.extend(formats(survey));
    let key = instances(survey, profile, &mut out);
    markers_and_runs(survey, profile, &mut out);
    let metrics = extractors(survey, profile, &mut out);
    noise(survey, &mut out);
    charts(survey, profile, &key, &metrics, &mut out);
    queries(survey, &mut out);
    out
}

fn formats(survey: &FolderSurvey) -> Vec<Candidate> {
    survey
        .unrecognised
        .iter()
        .filter(|g| !g.sample_lines.is_empty())
        .flat_map(|g| {
            FORMATS.iter().enumerate().map(move |(i, (label, regex))| Candidate {
                id: format!("format:{}:{i}", g.extension),
                group: ProposalGroup::Formats,
                origin: ProposalOrigin::Heuristic,
                title: format!("Read .{} files as text with a {label}", g.extension),
                summary: format!(
                    "{} file(s) like {} are not read now ({}). This format takes a {label} at the start of each line, an optional level and the rest as the message.",
                    g.file_count,
                    g.sample_file.as_deref().unwrap_or("?"),
                    g.reason
                ),
                fragments: vec![Fragment::ParseRegex {
                    regex: (*regex).to_string(),
                    file_name: g.sample_file.clone().unwrap_or_else(|| format!("sample.{}", g.extension)),
                }],
                requires: vec![],
                recommended: true,
            })
        })
        .collect()
}

/// Instance-key proposals; returns the key field the best one sets (for charts), if any.
fn instances(survey: &FolderSurvey, profile: &Profile, out: &mut Vec<Candidate>) -> Option<String> {
    let mut best = None;
    for (i, c) in survey.instance_candidates.iter().take(MAX_INSTANCE).enumerate() {
        if profile.instance.key.len() == 1 && profile.instance.key[0] == c.field {
            best.get_or_insert_with(|| c.field.clone());
            continue;
        }
        let mut fragments = Vec::new();
        let from = match &c.origin {
            KeyOrigin::Field => "the field".to_string(),
            KeyOrigin::Message { regex } => {
                fragments.push(Fragment::Extractor {
                    extractor: ExtractCfg {
                        name: format!("instance-{}", c.field.to_ascii_lowercase()),
                        from: "@message".into(),
                        regex: regex.clone(),
                        when: vec![],
                        fields: BTreeMap::from([(
                            c.field.clone(),
                            FieldSpec {
                                kind: FieldType::String,
                                unit: None,
                                unit_from: None,
                                to: None,
                            },
                        )]),
                    },
                });
                "text in the messages (extracted as".to_string() + " a field)"
            }
        };
        fragments.push(Fragment::InstanceKey {
            key: vec![c.field.clone()],
        });
        if i == 0 {
            best = Some(c.field.clone());
        }
        out.push(Candidate {
            id: format!("instance:{}", c.field),
            group: ProposalGroup::Instances,
            origin: ProposalOrigin::Heuristic,
            title: format!("One timeline lane per {}", c.field),
            summary: format!(
                "Split the events into {} lanes by {from} {} ({}% of the events have it), e.g. {}.",
                c.lanes,
                c.field,
                (c.coverage * 100.0).round(),
                c.examples.iter().take(3).cloned().collect::<Vec<_>>().join(", ")
            ),
            fragments,
            requires: vec![],
            recommended: i == 0,
        });
    }
    best
}

fn marker_kinds(survey: &FolderSurvey, profile: &Profile) -> HashSet<String> {
    survey
        .markers
        .iter()
        .map(|m| m.kind.clone())
        .chain(profile.marker.iter().map(|m| m.kind.clone()))
        .chain(profile.imt_disabled_markers.iter().map(|m| m.rule.kind.clone()))
        .collect()
}

fn markers_and_runs(survey: &FolderSurvey, profile: &Profile, out: &mut Vec<Candidate>) {
    let mut taken = marker_kinds(survey, profile);
    let mut starts: Vec<String> = Vec::new();
    let mut stops: Vec<String> = Vec::new();
    let mut requires: Vec<String> = Vec::new();
    let add = |list: &mut Vec<String>, k: &str| {
        if !list.iter().any(|x| x == k) {
            list.push(k.to_string());
        }
    };
    for c in &survey.lifecycle {
        if let Some(kind) = &c.covered_by {
            match c.role {
                LifecycleRole::Start => add(&mut starts, kind),
                LifecycleRole::Stop => add(&mut stops, kind),
                LifecycleRole::Crash => {}
            }
            continue;
        }
        let (base, label, color, icon) = match c.role {
            LifecycleRole::Start => ("svc-start", "Service start", "#3FBF7F", "rocket"),
            LifecycleRole::Stop => ("svc-stop", "Service stop", "#A7B0C2", "power"),
            LifecycleRole::Crash => ("crash", "Crash", "#FF4FA3", "fire"),
        };
        let kind = unique(base, &mut taken);
        let id = format!("marker:{kind}");
        match c.role {
            LifecycleRole::Start => add(&mut starts, &kind),
            LifecycleRole::Stop => add(&mut stops, &kind),
            LifecycleRole::Crash => {}
        }
        if c.role != LifecycleRole::Crash {
            requires.push(id.clone());
        }
        out.push(Candidate {
            id,
            group: ProposalGroup::Markers,
            origin: ProposalOrigin::Heuristic,
            title: format!("Mark “{}” as {}", short(&c.template, 60), label.to_lowercase()),
            summary: format!(
                "{} event(s) read like a {} and no marker shows them yet. Example: {}",
                c.count,
                label.to_lowercase(),
                short(&c.example, 120)
            ),
            fragments: vec![Fragment::Marker {
                marker: MarkerCfg {
                    kind,
                    label: Some(label.into()),
                    lifecycle: true,
                    color: Some(color.into()),
                    icon: Some(icon.into()),
                    badge: None,
                    shape: None,
                    description: Some(format!("Found by the folder survey: {}", short(&c.template, 120))),
                    when: vec![template_condition(&c.template, c.template_kind)],
                },
            }],
            requires: vec![],
            recommended: true,
        });
    }
    if let Some(c) = run_rule(profile, starts, stops, requires) {
        out.push(c);
    }
}

/// The `[[span]]` proposal: the profile's run rule extended with the new lifecycle markers (same kind, so
/// it replaces that rule, as the Runs editor does), or a new "run" rule when the profile has none. Nothing
/// when the existing rule already covers what was found.
fn run_rule(profile: &Profile, starts: Vec<String>, stops: Vec<String>, requires: Vec<String>) -> Option<Candidate> {
    let new_kinds: Vec<String> = requires
        .iter()
        .filter_map(|id| id.strip_prefix("marker:").map(str::to_string))
        .collect();
    let base = profile
        .span
        .iter()
        .find(|s| s.enabled && s.kind == "run")
        .or_else(|| profile.span.iter().find(|s| s.enabled));
    let mut span = match base {
        Some(_) if new_kinds.is_empty() => return None,
        Some(b) => b.clone(),
        None if starts.is_empty() || stops.is_empty() => return None,
        None => SpanCfg {
            kind: "run".into(),
            label: Some("Running".into()),
            start: OneOrMany::Many(vec![]),
            stop: OneOrMany::Many(vec![]),
            per: "instance".into(),
            color: None,
            max_duration: None,
            restart_on_start: true,
            enabled: true,
        },
    };
    let merge = |list: &OneOrMany, add: &[String]| {
        let mut l = list.list();
        for k in add {
            if !l.contains(k) {
                l.push(k.clone());
            }
        }
        OneOrMany::Many(l)
    };
    span.start = merge(&span.start, &starts);
    span.stop = merge(&span.stop, &stops);
    if span.start.list().is_empty() || span.stop.list().is_empty() {
        return None;
    }
    let (title, summary) = if base.is_some() {
        (
            format!(
                "Let the “{}” runs start and stop on the new markers",
                span.label.clone().unwrap_or_else(|| span.kind.clone())
            ),
            format!(
                "Runs now start at {} and stop at {}; a start without a stop is flagged as a likely crash or restart.",
                span.start.list().join(", "),
                span.stop.list().join(", ")
            ),
        )
    } else {
        (
            "Show runs from start to stop".to_string(),
            format!(
                "Draw a band on each lane from a start ({}) to the next stop ({}); a start without a stop is flagged as a likely crash or restart.",
                span.start.list().join(", "),
                span.stop.list().join(", ")
            ),
        )
    };
    Some(Candidate {
        id: format!("span:{}", span.kind),
        group: ProposalGroup::Markers,
        origin: ProposalOrigin::Heuristic,
        title,
        summary,
        fragments: vec![Fragment::Span { span }],
        requires,
        recommended: true,
    })
}

/// Extractor proposals for embedded numbers; returns (proposal id, field) of each.
fn extractors(survey: &FolderSurvey, profile: &Profile, out: &mut Vec<Candidate>) -> Vec<(String, String, Option<String>)> {
    let mut taken: HashSet<String> = profile.extract.iter().map(|e| e.name.clone()).collect();
    let mut metrics = Vec::new();
    for (i, n) in survey.numbers.iter().take(MAX_METRIC_EXTRACTORS).enumerate() {
        let name = unique(&format!("metric-{}", n.name.to_ascii_lowercase()), &mut taken);
        let spec = match n.number_kind {
            NumberKind::Duration => FieldSpec {
                kind: FieldType::Duration,
                unit: n.unit.as_deref().map(|u| if u == "sec" { "s".to_string() } else { u.to_string() }),
                unit_from: None,
                to: Some("ms".into()),
            },
            NumberKind::Bytes => FieldSpec {
                kind: FieldType::Bytes,
                unit: n
                    .unit
                    .as_deref()
                    .map(|u| if u == "bytes" { "B".to_string() } else { u.to_string() }),
                unit_from: None,
                to: Some("MB".into()),
            },
            NumberKind::Number => FieldSpec {
                kind: FieldType::Number,
                unit: n.unit.clone(),
                unit_from: None,
                to: None,
            },
        };
        let id = format!("extract:{}", n.name);
        let unit = match n.number_kind {
            NumberKind::Duration => Some("ms".to_string()),
            NumberKind::Bytes => Some("MB".to_string()),
            NumberKind::Number => n.unit.clone(),
        };
        metrics.push((id.clone(), n.name.clone(), unit));
        out.push(Candidate {
            id,
            group: ProposalGroup::Extractors,
            origin: ProposalOrigin::Heuristic,
            title: format!("Extract {} as a number", n.name),
            summary: format!(
                "“{}” is followed by a number in {} sampled messages (from {} to {}{}). Example: {}",
                n.label,
                n.count,
                n.min,
                n.max,
                n.unit.as_deref().map(|u| format!(" {u}")).unwrap_or_default(),
                short(&n.example, 120)
            ),
            fragments: vec![Fragment::Extractor {
                extractor: ExtractCfg {
                    name,
                    from: "@message".into(),
                    regex: n.regex.clone(),
                    when: vec![],
                    fields: BTreeMap::from([(n.name.clone(), spec)]),
                },
            }],
            requires: vec![],
            recommended: i < RECOMMENDED_EXTRACTORS,
        });
    }
    metrics
}

fn window_text(ms: i64) -> String {
    format!("{}s", (ms.saturating_mul(10) / 1000).clamp(1, 3600))
}

fn noise(survey: &FolderSurvey, out: &mut Vec<Candidate>) {
    for (i, n) in survey.noise.iter().enumerate() {
        let when = vec![template_condition(&n.template, n.template_kind)];
        let (step, title, why, recommended) = match &n.reason {
            NoiseReason::Keyword { word } => (
                StepCfg {
                    step: "drop".into(),
                    when,
                    ..StepCfg::default()
                },
                format!("Drop “{}”", short(&n.template, 60)),
                format!("it looks like routine {word} traffic"),
                false,
            ),
            NoiseReason::Periodic { interval_ms } => (
                StepCfg {
                    step: "dedupe".into(),
                    when,
                    fields: vec!["@message".into()],
                    window: Some(window_text(*interval_ms)),
                    ..StepCfg::default()
                },
                format!("Collapse repeats of “{}”", short(&n.template, 60)),
                format!(
                    "it repeats about every {} s with the same text",
                    (*interval_ms as f64 / 1000.0).max(0.001)
                ),
                true,
            ),
        };
        out.push(Candidate {
            id: format!("noise:{i}"),
            group: ProposalGroup::Noise,
            origin: ProposalOrigin::Heuristic,
            title,
            summary: format!(
                "{} of the sampled events ({}%) are this message and {why}.",
                n.count,
                (n.share * 100.0).round()
            ),
            fragments: vec![Fragment::Pipeline { step }],
            requires: vec![],
            recommended,
        });
    }
}

const METRIC_HINTS: &[&str] = &["elapsed", "duration", "latency", "time", "cpu", "memory", "workingset", "ms"];

fn charts(
    survey: &FolderSurvey,
    profile: &Profile,
    key: &Option<String>,
    metrics: &[(String, String, Option<String>)],
    out: &mut Vec<Candidate>,
) {
    let mut ids: HashSet<String> = profile.chart.iter().filter_map(|c| c.id.clone()).collect();
    let split = key.clone().unwrap_or_else(|| "@source".into());
    let key_req: Vec<String> = out
        .iter()
        .filter(|c| c.group == ProposalGroup::Instances && c.recommended)
        .map(|c| c.id.clone())
        .collect();
    let id = unique("errors-by-instance", &mut ids);
    out.push(Candidate {
        id: format!("chart:{id}"),
        group: ProposalGroup::Charts,
        origin: ProposalOrigin::Heuristic,
        title: format!("Chart errors per {split}"),
        summary: format!("Errors and fatal events per interval, one series per {split}, to see which instance fails when."),
        fragments: vec![Fragment::Chart {
            chart: ChartCfg {
                id: Some(id),
                title: format!("Errors per {split}"),
                y: "count()".into(),
                bucket: "auto".into(),
                split_by: vec![split.clone()],
                kind: "stacked".into(),
                unit: "events".into(),
                when: vec![ConditionCfg {
                    field: "@level".into(),
                    op: "in".into(),
                    value: Some(CondValue::List(vec!["Error".into(), "Fatal".into()])),
                }],
            },
        }],
        requires: key_req.clone(),
        recommended: true,
    });
    // The top metric: an existing numeric field that reads like a measurement, else the first extracted one.
    let field_metric = survey.fields.iter().find(|f| {
        f.kind == FieldKind::Number && f.coverage >= 0.05 && METRIC_HINTS.iter().any(|h| f.name.to_ascii_lowercase().contains(h))
    });
    let metric = match (field_metric, metrics.first()) {
        (Some(f), _) => Some((f.name.clone(), None, None)),
        (None, Some((req, name, unit))) => Some((name.clone(), Some(req.clone()), unit.clone())),
        _ => None,
    };
    if let Some((name, req, unit)) = metric {
        let id = unique(&format!("{}-by-instance", name.to_ascii_lowercase()), &mut ids);
        let mut requires = key_req;
        requires.extend(req);
        out.push(Candidate {
            id: format!("chart:{id}"),
            group: ProposalGroup::Charts,
            origin: ProposalOrigin::Heuristic,
            title: format!("Chart average {name} per {split}"),
            summary: format!("The average of {name} per interval, one line per {split}."),
            fragments: vec![Fragment::Chart {
                chart: ChartCfg {
                    id: Some(id),
                    title: format!("{name} per {split}"),
                    y: format!("avg({name})"),
                    bucket: "auto".into(),
                    split_by: vec![split.clone()],
                    kind: "line".into(),
                    unit: unit.unwrap_or_default(),
                    when: vec![],
                },
            }],
            requires,
            recommended: true,
        });
    }
    let tile = |id: &str, title: &str, kind: TileKind, (x, y, w, h): (u32, u32, u32, u32)| TileCfg {
        id: id.into(),
        title: title.into(),
        kind,
        x,
        y,
        w,
        h,
        query: None,
        chart: None,
        value: None,
        by: vec![],
        style: None,
        field: None,
        limit: None,
        unit: None,
        window: None,
    };
    let tiles = vec![
        TileCfg {
            query: Some("level >= error | stats count()".into()),
            ..tile("errors", "Errors", TileKind::Stat, (0, 0, 3, 2))
        },
        TileCfg {
            query: Some("level = warning | stats count()".into()),
            ..tile("warnings", "Warnings", TileKind::Stat, (3, 0, 3, 2))
        },
        TileCfg {
            value: Some("count()".into()),
            by: vec!["@level".into()],
            style: Some("stacked".into()),
            ..tile("events", "Events by level", TileKind::Chart, (0, 2, 12, 4))
        },
        TileCfg {
            field: Some(split.clone()),
            limit: Some(10),
            ..tile("busiest", &format!("Busiest by {split}"), TileKind::Top, (0, 6, 6, 4))
        },
        TileCfg {
            query: Some("level >= error | stats count() as n by @template | sort n desc | head 10".into()),
            limit: Some(10),
            ..tile("top-errors", "Top error messages", TileKind::Table, (6, 6, 6, 4))
        },
    ];
    let taken: HashSet<String> = profile.dashboard.iter().map(|d| d.id.clone()).collect();
    if !taken.contains("folder-overview") {
        out.push(Candidate {
            id: "dashboard:folder-overview".into(),
            group: ProposalGroup::Charts,
            origin: ProposalOrigin::Heuristic,
            title: "Add a starter dashboard".into(),
            summary: format!(
                "Error and warning counts, events by level, the busiest {split} values and the most frequent errors on one page."
            ),
            fragments: vec![Fragment::Dashboard {
                dashboard: DashboardCfg {
                    id: "folder-overview".into(),
                    title: "Folder overview".into(),
                    columns: 12,
                    tile: tiles,
                },
            }],
            requires: vec![],
            recommended: false,
        });
    }
}

/// `"..."` for the query language.
pub fn ql_quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The longest run of literal words of a mined template (what a `contains` filter can use).
fn literal_run(template: &str) -> String {
    let mut best: Vec<&str> = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    for tok in template.split_whitespace() {
        if tok.contains(WILDCARD) {
            if cur.len() > best.len() {
                best = std::mem::take(&mut cur);
            }
            cur.clear();
        } else {
            cur.push(tok);
        }
    }
    if cur.len() > best.len() {
        best = cur;
    }
    best.join(" ")
}

fn queries(survey: &FolderSurvey, out: &mut Vec<Candidate>) {
    for (i, t) in survey.errors.iter().take(MAX_QUERIES).enumerate() {
        let ql = match t.template_kind {
            TemplateKind::Serilog => format!("@template = {}", ql_quote(&t.template)),
            TemplateKind::Mined => {
                let lit = literal_run(&t.template);
                if lit.chars().count() < 4 {
                    continue;
                }
                format!("level >= warning and @message contains {}", ql_quote(&lit))
            }
        };
        out.push(Candidate {
            id: format!("query:error:{i}"),
            group: ProposalGroup::Queries,
            origin: ProposalOrigin::Heuristic,
            title: format!("Save a query for “{}”", short(&t.template, 60)),
            summary: format!(
                "{} sampled {} events like: {}",
                t.count,
                t.level.as_str().to_lowercase(),
                short(&t.example, 120)
            ),
            fragments: vec![Fragment::SavedQuery {
                name: format!("Errors: {}", short(&t.template, 50)),
                ql,
            }],
            requires: vec![],
            recommended: i == 0,
        });
    }
}
