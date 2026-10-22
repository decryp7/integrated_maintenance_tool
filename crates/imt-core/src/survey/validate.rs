//! Validation and preview of every proposal, heuristic or AI: the candidate profile (the user's profile
//! with the proposal and what it needs) must compile against the store's plugins, and the proposal must do
//! something sensible on the sampled events. Proposals that fail are returned as skipped, with the reason.

use super::dto::*;
use super::fields::MAX_LANES;
use super::fragment::{compose_toml, fragment_toml};
use super::sample::{Sample, Sampled};
use super::SurveyStore;
use super::{SurveyControl, SurveyError, SurveyStage};
use crate::api::ParsePreviewRequest;
use crate::condition::Condition;
use crate::model::Record;
use crate::pipeline::Pipeline;
use crate::ports::PageQuery;
use crate::profile::{compile_conditions, CompiledProfile};
use crate::query::Query;
use crate::spans::{Edge, Pairer, Per, RunStatus, SpanCfg};
use crate::Profile;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};

const EXAMPLES: usize = 3;
const LANE_PREVIEW: usize = 12;
/// Share of a sample's lines a format must read (with a timestamp) to be proposed.
const MIN_PARSED: f64 = 0.6;

/// Validates `candidates` in order (a proposal may only require earlier, valid ones).
pub fn validate<S: SurveyStore + ?Sized>(
    store: &S,
    base_toml: &str,
    survey: &FolderSurvey,
    sampled: &Sampled,
    candidates: Vec<Candidate>,
    control: &SurveyControl,
) -> Result<(Vec<SetupProposal>, Vec<SkippedProposal>), SurveyError> {
    control.begin(SurveyStage::Validating, candidates.len());
    let mut accepted: Vec<SetupProposal> = Vec::new();
    let mut skipped: Vec<SkippedProposal> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for c in candidates {
        control.check()?;
        control.advance(1);
        if !seen.insert(c.id.clone()) {
            continue;
        }
        let skip = |c: &Candidate, reason: String| SkippedProposal {
            id: c.id.clone(),
            group: c.group,
            origin: c.origin,
            title: c.title.clone(),
            reason,
        };
        let mut needed: Vec<Fragment> = Vec::new();
        let mut missing = None;
        for r in &c.requires {
            match accepted.iter().find(|p| &p.id == r) {
                Some(p) => needed.extend(p.fragments.iter().cloned()),
                None => missing = Some(r.clone()),
            }
        }
        if let Some(r) = missing {
            skipped.push(skip(&c, format!("it needs the proposal “{r}”, which was not valid")));
            continue;
        }
        match check(store, base_toml, survey, sampled, &needed, &c) {
            Ok((preview, toml)) => accepted.push(SetupProposal {
                id: c.id,
                group: c.group,
                origin: c.origin,
                title: c.title,
                summary: c.summary,
                fragments: c.fragments,
                requires: c.requires,
                toml,
                preview,
                recommended: c.recommended,
            }),
            Err(reason) => skipped.push(skip(&c, reason)),
        }
    }
    best_format_only(&mut accepted, &mut skipped);
    Ok((accepted, skipped))
}

/// Of several formats that read one group of files, keeps the one that parses most lines.
fn best_format_only(accepted: &mut Vec<SetupProposal>, skipped: &mut Vec<SkippedProposal>) {
    let mut best: HashMap<String, (usize, String)> = HashMap::new();
    for p in accepted.iter().filter(|p| p.group == ProposalGroup::Formats) {
        let parsed = match p.preview {
            ProposalPreview::Parse { with_timestamp, .. } => with_timestamp,
            _ => 0,
        };
        let ext = format_group(&p.id);
        if best.get(&ext).is_none_or(|(n, _)| parsed > *n) {
            best.insert(ext, (parsed, p.id.clone()));
        }
    }
    accepted.retain(|p| {
        if p.group != ProposalGroup::Formats || p.origin == ProposalOrigin::Ai {
            return true;
        }
        let keep = best.get(&format_group(&p.id)).is_some_and(|(_, id)| id == &p.id);
        if !keep {
            skipped.push(SkippedProposal {
                id: p.id.clone(),
                group: p.group,
                origin: p.origin,
                title: p.title.clone(),
                reason: "another format reads more of these files".into(),
            });
        }
        keep
    });
}

/// `format:<ext>:<n>` -> `<ext>`.
fn format_group(id: &str) -> String {
    id.split(':').nth(1).unwrap_or_default().to_string()
}

type Checked = std::result::Result<(ProposalPreview, String), String>;

fn check<S: SurveyStore + ?Sized>(
    store: &S,
    base_toml: &str,
    survey: &FolderSurvey,
    sampled: &Sampled,
    needed: &[Fragment],
    c: &Candidate,
) -> Checked {
    if c.fragments.is_empty() {
        return Err("it changes nothing".into());
    }
    let snippet = fragment_toml(&c.fragments).map_err(|e| format!("{e:#}"))?;
    let all: Vec<Fragment> = needed.iter().chain(&c.fragments).cloned().collect();
    let profile_fragments: Vec<Fragment> = all.iter().filter(|f| !matches!(f, Fragment::SavedQuery { .. })).cloned().collect();
    let mut candidate_toml = None;
    if !profile_fragments.is_empty() {
        let (_, merged) = compose_toml(base_toml, &profile_fragments).map_err(|e| format!("{e:#}"))?;
        let profile = Profile::from_toml(&merged).map_err(|e| format!("the profile would not be valid: {e}"))?;
        store
            .validate_profile(&profile)
            .map_err(|e| format!("the profile would not compile: {e:#}"))?;
        candidate_toml = Some(merged);
    }
    let eval = Eval::new(&all)?;
    let events = &sampled.events;
    let preview = if let Some(key) = c.fragments.iter().find_map(|f| match f {
        Fragment::InstanceKey { key } => Some(key.clone()),
        _ => None,
    }) {
        lanes(&eval, events, &key)?
    } else {
        match c.fragments.first() {
            Some(Fragment::ParseRegex { .. }) => parse(store, survey, c, candidate_toml.as_deref().unwrap_or(base_toml))?,
            Some(Fragment::Marker { marker }) => matches(&eval, events, &marker.kind)?,
            Some(Fragment::Span { span }) => runs(&eval, events, span)?,
            Some(Fragment::Extractor { extractor }) => extracted(&eval, events, &extractor.name)?,
            Some(Fragment::Pipeline { step }) => pipeline(&eval, events, step)?,
            Some(Fragment::Chart { chart }) => chart_shape(&eval, events, chart)?,
            Some(Fragment::Dashboard { dashboard }) => ProposalPreview::Dashboard {
                tiles: dashboard.tile.len(),
            },
            Some(Fragment::SavedQuery { ql, .. }) => query(store, ql)?,
            Some(Fragment::InstanceKey { .. }) | None => return Err("it changes nothing".into()),
        }
    };
    Ok((preview, snippet))
}

/// What the proposals' extractors and markers do to a sampled event, compiled once per proposal.
struct Eval {
    compiled: CompiledProfile,
    markers: Vec<(String, Vec<Condition>)>,
}

impl Eval {
    fn new(fragments: &[Fragment]) -> std::result::Result<Eval, String> {
        let mut profile = Profile::default_profile();
        profile.extract = fragments
            .iter()
            .filter_map(|f| match f {
                Fragment::Extractor { extractor } => Some(extractor.clone()),
                _ => None,
            })
            .collect();
        let compiled = CompiledProfile::compile(profile).map_err(|e| format!("{e:#}"))?;
        let mut markers = Vec::new();
        for f in fragments {
            if let Fragment::Marker { marker } = f {
                if marker.when.is_empty() {
                    return Err(format!("marker “{}” has no conditions", marker.kind));
                }
                let when = compile_conditions(&marker.when).map_err(|e| format!("marker “{}”: {e:#}", marker.kind))?;
                markers.push((marker.kind.clone(), when));
            }
        }
        Ok(Eval { compiled, markers })
    }

    /// The event after the proposals' extractors.
    fn record(&self, e: &Sample) -> Record {
        let mut r = e.rec.clone();
        self.compiled.apply_extractors(&mut r, &e.source_name);
        r
    }

    /// Marker kinds of the event: the active profile's plus the proposed ones.
    fn kinds<'a>(&'a self, e: &'a Sample, r: &Record) -> Vec<&'a str> {
        let mut k: Vec<&str> = e.markers.iter().map(String::as_str).collect();
        for (kind, when) in &self.markers {
            if when.iter().all(|c| c.eval(r, &e.source_name)) {
                k.push(kind);
            }
        }
        k
    }
}

fn message_example(r: &Record) -> String {
    r.message.chars().take(200).collect()
}

fn lanes(eval: &Eval, events: &[Sample], key: &[String]) -> Previewed {
    let mut lanes: BTreeMap<String, LanePreview> = BTreeMap::new();
    let mut unassigned = 0;
    for e in events {
        let r = eval.record(e);
        let parts: Vec<String> = key
            .iter()
            .filter_map(|k| r.field_text(k, &e.source_name).map(|v| v.into_owned()))
            .collect();
        if parts.len() != key.len() {
            unassigned += 1;
            continue;
        }
        let name = parts.join(" · ");
        let lane = lanes.entry(name.clone()).or_insert(LanePreview {
            name,
            count: 0,
            from: None,
            to: None,
        });
        lane.count += 1;
        if let Some(ts) = r.ts {
            lane.from = Some(lane.from.map_or(ts, |f| f.min(ts)));
            lane.to = Some(lane.to.map_or(ts, |t| t.max(ts)));
        }
    }
    let total = lanes.len();
    if total < 2 {
        return Err(format!("it gives {total} lane(s) on the sample; a key needs at least 2"));
    }
    if total > MAX_LANES {
        return Err(format!(
            "it gives {total} lanes on the sample, more than {MAX_LANES} are unreadable"
        ));
    }
    let mut list: Vec<LanePreview> = lanes.into_values().collect();
    list.sort_by(|a, b| b.count.cmp(&a.count).then(a.name.cmp(&b.name)));
    list.truncate(LANE_PREVIEW);
    Ok(ProposalPreview::Lanes {
        lanes: list,
        total_lanes: total,
        unassigned,
    })
}

type Previewed = std::result::Result<ProposalPreview, String>;

fn matches(eval: &Eval, events: &[Sample], kind: &str) -> Previewed {
    let mut n = 0;
    let mut examples = Vec::new();
    for e in events {
        let r = eval.record(e);
        if eval
            .markers
            .iter()
            .any(|(k, when)| k == kind && when.iter().all(|c| c.eval(&r, &e.source_name)))
        {
            n += 1;
            if examples.len() < EXAMPLES {
                examples.push(message_example(&r));
            }
        }
    }
    if n == 0 {
        return Err(format!("marker “{kind}” matches none of the {} sampled events", events.len()));
    }
    if n == events.len() && n > 3 {
        return Err(format!("marker “{kind}” matches every sampled event"));
    }
    Ok(ProposalPreview::Matches {
        matches: n,
        sampled: events.len(),
        examples,
    })
}

fn extracted(eval: &Eval, events: &[Sample], name: &str) -> Previewed {
    let Some(ex) = eval.compiled.extractors.iter().find(|x| x.name == name) else {
        return Err(format!("extractor “{name}” is missing"));
    };
    let mut n = 0;
    let mut examples: Vec<Map<String, Value>> = Vec::new();
    for e in events {
        if let Some(values) = ex.extract(&e.rec, &e.source_name).filter(|v| !v.is_empty()) {
            n += 1;
            if examples.len() < EXAMPLES {
                examples.push(values.into_iter().collect());
            }
        }
    }
    if n == 0 {
        return Err(format!(
            "extractor “{name}” extracts nothing from the {} sampled events",
            events.len()
        ));
    }
    Ok(ProposalPreview::Extracted {
        matches: n,
        sampled: events.len(),
        examples,
    })
}

/// Runs of a `[[span]]` proposal: the engine's pairing ([`Pairer`]) over the sample with the proposed markers
/// applied (the same pairing as the Runs table).
fn runs(eval: &Eval, events: &[Sample], span: &SpanCfg) -> Previewed {
    if let Some(issue) = crate::spans::config::check(span).into_iter().next() {
        return Err(format!("the run rule is not valid: {}", issue.message));
    }
    let (starts, stops) = (span.start.list(), span.stop.list());
    let per = Per::parse(&span.per).map_err(|e| format!("the run rule is not valid: {e:#}"))?;
    let mut order: Vec<usize> = (0..events.len()).filter(|i| events[*i].rec.ts.is_some()).collect();
    order.sort_by_key(|i| (events[*i].rec.ts, *i));
    let mut pairer = Pairer::new(span.restart_on_start);
    for i in order {
        let e = &events[i];
        let r = eval.record(e);
        let kinds = eval.kinds(e, &r);
        let edge = if kinds.iter().any(|k| starts.iter().any(|s| s == k)) {
            Edge::Start
        } else if kinds.iter().any(|k| stops.iter().any(|s| s == k)) {
            Edge::Stop
        } else {
            Edge::Other
        };
        let value = match &per {
            Per::Instance => None,
            Per::Field(f) => r.field_text(f, &e.source_name).map(|v| v.into_owned()),
        };
        pairer.feed(&e.lane, value.as_deref(), edge, r.id, r.ts.unwrap_or_default(), || {
            message_example(&r)
        });
    }
    let all = pairer.runs();
    let count = |st: RunStatus| all.iter().filter(|r| r.status == st).count();
    let (closed, open, no_stop) = (count(RunStatus::Closed), count(RunStatus::Open), count(RunStatus::NoStop));
    let runs = closed + open + no_stop;
    if runs == 0 {
        return Err("no run starts in the sampled events".into());
    }
    let lanes = all.iter().map(|r| r.lane.as_str()).collect::<HashSet<_>>().len();
    Ok(ProposalPreview::Runs {
        runs,
        closed,
        open,
        no_stop,
        lanes,
    })
}

fn pipeline(eval: &Eval, events: &[Sample], step: &crate::pipeline::StepCfg) -> Previewed {
    let compiled =
        Pipeline::compile(std::slice::from_ref(step), &HashMap::new()).map_err(|e| format!("the step does not compile: {e:#}"))?;
    let mut by_source: BTreeMap<&str, Vec<Record>> = BTreeMap::new();
    let mut originals: HashMap<u32, Record> = HashMap::new();
    for e in events {
        let r = eval.record(e);
        originals.insert(r.id, r.clone());
        by_source.entry(e.source_name.as_str()).or_default().push(r);
    }
    let (mut kept, mut changed) = (0, 0);
    let mut examples = Vec::new();
    let mut survivors: HashSet<u32> = HashSet::new();
    for (source, mut recs) in by_source {
        recs.sort_by_key(|r| (r.ts, r.line));
        let (out, _) = compiled.run_with_stats(recs, source);
        for r in out {
            kept += 1;
            survivors.insert(r.id);
            if let Some(before) = originals.get(&r.id) {
                if before.message != r.message || before.fields != r.fields || before.level != r.level {
                    changed += 1;
                    if examples.len() < EXAMPLES {
                        let added: Vec<String> = r
                            .fields
                            .iter()
                            .filter(|(k, v)| before.fields.get(*k) != Some(*v))
                            .map(|(k, v)| format!("{k}={}", crate::model::value_text(v)))
                            .collect();
                        let after = if added.is_empty() {
                            message_example(&r)
                        } else {
                            format!("{} [{}]", message_example(&r), added.join(", "))
                        };
                        examples.push(ChangeExample {
                            before: message_example(before),
                            after,
                        });
                    }
                }
            }
        }
    }
    let dropped = events.len() - kept;
    if dropped == 0 && changed == 0 {
        return Err("the step changes none of the sampled events".into());
    }
    if kept == 0 {
        return Err("the step would drop every sampled event".into());
    }
    for e in events
        .iter()
        .filter(|e| !survivors.contains(&e.rec.id))
        .take(EXAMPLES.saturating_sub(examples.len()))
    {
        examples.push(ChangeExample {
            before: message_example(&e.rec),
            after: String::new(),
        });
    }
    Ok(ProposalPreview::Pipeline {
        sampled: events.len(),
        kept,
        dropped,
        changed,
        examples,
    })
}

fn chart_shape(eval: &Eval, events: &[Sample], chart: &crate::profile::ChartCfg) -> Previewed {
    let (_, field) = crate::chart::parse_y(&chart.y).map_err(|e| format!("the chart's value is not valid: {e:#}"))?;
    let when = compile_conditions(&chart.when).map_err(|e| format!("the chart's conditions are not valid: {e:#}"))?;
    let mut series: BTreeMap<String, usize> = BTreeMap::new();
    let mut points = 0;
    for e in events.iter().filter(|e| e.rec.ts.is_some()) {
        let r = eval.record(e);
        if !when.iter().all(|c| c.eval(&r, &e.source_name)) {
            continue;
        }
        if let Some(f) = &field {
            if r.field_number(f).is_none() {
                continue;
            }
        }
        points += 1;
        let key: Vec<String> = chart
            .split_by
            .iter()
            .map(|k| {
                r.field_text(k, &e.source_name)
                    .map(|v| v.into_owned())
                    .unwrap_or_else(|| "(none)".into())
            })
            .collect();
        *series.entry(key.join(" · ")).or_default() += 1;
    }
    if points == 0 {
        return Err(match field {
            Some(f) => format!("no sampled event has a number in “{f}”"),
            None => "no sampled event matches the chart".into(),
        });
    }
    let mut list: Vec<(String, usize)> = series.into_iter().collect();
    list.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    Ok(ProposalPreview::Chart {
        series: list.into_iter().take(8).map(|s| s.0).collect(),
        points,
    })
}

fn query<S: SurveyStore + ?Sized>(store: &S, ql: &str) -> Previewed {
    match store.check_query(ql) {
        Ok(Some(e)) => return Err(format!("the query is not valid: {}", e.message)),
        Ok(None) => {}
        Err(e) => return Err(format!("the query cannot be checked: {e}")),
    }
    let req = PageQuery {
        query: Query {
            ql: ql.to_string(),
            ..Query::default()
        },
        offset: 0,
        limit: 1,
        descending: false,
        anchor: None,
    };
    let matches = store.page(&req).map_err(|e| format!("the query cannot run: {e:#}"))?.total;
    if matches == 0 {
        return Err("the query finds no events".into());
    }
    Ok(ProposalPreview::Query { matches })
}

fn parse<S: SurveyStore + ?Sized>(store: &S, survey: &FolderSurvey, c: &Candidate, toml: &str) -> Previewed {
    let ext = format_group(&c.id);
    let group = survey
        .unrecognised
        .iter()
        .find(|g| g.extension == ext)
        .filter(|g| !g.sample_lines.is_empty())
        .ok_or("there are no sample lines to try it on")?;
    let parser = super::fragment::parser_drafts(&c.fragments)
        .map_err(|e| format!("{e:#}"))?
        .into_iter()
        .next()
        .ok_or("it adds no format")?;
    let req = ParsePreviewRequest {
        toml: parser,
        profile: toml.to_string(),
        sample: group.sample_lines.join("\n"),
        file_name: group.sample_file.clone().unwrap_or_default(),
        limit: 200,
    };
    let p = store
        .preview_parse(&req)
        .map_err(|e| format!("the files cannot be previewed: {e}"))?;
    if let Some(e) = p.error {
        return Err(format!("the files still cannot be read: {e}"));
    }
    let lines = group.sample_lines.len();
    let with_timestamp = p.records.iter().filter(|r| r.ts.is_some()).count();
    if (with_timestamp as f64) < lines as f64 * MIN_PARSED {
        return Err(format!(
            "it reads {with_timestamp} of {lines} sample lines with a timestamp ({}%)",
            with_timestamp * 100 / lines.max(1)
        ));
    }
    Ok(ProposalPreview::Parse {
        parsed: p.records.len(),
        lines,
        with_timestamp,
        examples: p
            .records
            .iter()
            .take(EXAMPLES)
            .map(|r| r.message.chars().take(200).collect())
            .collect(),
    })
}
