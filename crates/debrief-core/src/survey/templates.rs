//! Message templates of the sample (exact Serilog templates, else Drain) and what they suggest: lifecycle
//! events (start, stop, crash), error templates, noisy high-frequency templates and marker coverage.

use super::dto::{LifecycleCandidate, LifecycleRole, MarkerCoverage, NoiseCandidate, NoiseReason, TemplateKind, TemplateSummary};
use super::sample::Sample;
use crate::model::Level;
use crate::patterns::{MinerConfig, PatternMiner, TemplateSource, WILDCARD};
use regex::Regex;
use std::collections::HashMap;
use std::sync::OnceLock;

/// Timestamps kept per template for the interval check.
const MAX_TIMES: usize = 4096;
const MAX_LIFECYCLE: usize = 16;
const MAX_ERRORS: usize = 10;
const MAX_NOISE: usize = 8;
/// Longest example message kept.
const EXAMPLE_CHARS: usize = 240;

/// One template with what the survey needs about its events.
#[derive(Debug, Clone)]
pub struct MinedTemplate {
    pub template: String,
    pub kind: TemplateKind,
    pub count: usize,
    pub levels: [usize; 6],
    pub exceptions: usize,
    /// Index (into the sample) of the first event.
    pub first: usize,
    pub times: Vec<i64>,
    pub markers: HashMap<String, usize>,
}

impl MinedTemplate {
    pub fn level(&self) -> Level {
        let (i, _) = self
            .levels
            .iter()
            .enumerate()
            .max_by_key(|(i, n)| (**n, std::cmp::Reverse(*i)))
            .unwrap_or((2, &0));
        Level::ALL[i]
    }

    fn at_least(&self, level: Level) -> usize {
        self.levels[level.index()..].iter().sum()
    }
}

/// The sample's templates, most frequent first.
#[derive(Debug, Clone, Default)]
pub struct Mined {
    pub templates: Vec<MinedTemplate>,
}

pub fn mine(events: &[Sample]) -> Mined {
    let mut miner = PatternMiner::new(MinerConfig::default());
    let mut accs: Vec<MinedTemplate> = Vec::new();
    for (i, e) in events.iter().enumerate() {
        let id = miner.add(&e.rec);
        if id >= accs.len() {
            accs.resize_with(id + 1, || MinedTemplate {
                template: String::new(),
                kind: TemplateKind::Mined,
                count: 0,
                levels: [0; 6],
                exceptions: 0,
                first: usize::MAX,
                times: Vec::new(),
                markers: HashMap::new(),
            });
        }
        let a = &mut accs[id];
        a.count += 1;
        a.levels[e.rec.level.index()] += 1;
        a.exceptions += usize::from(e.rec.exception.is_some());
        a.first = a.first.min(i);
        if let Some(ts) = e.rec.ts {
            if a.times.len() < MAX_TIMES {
                a.times.push(ts);
            }
        }
        for m in &e.markers {
            *a.markers.entry(m.clone()).or_default() += 1;
        }
    }
    for p in miner.clusters() {
        if let Some(a) = accs.get_mut(p.id) {
            a.template = p.template;
            a.kind = match p.source {
                TemplateSource::Serilog => TemplateKind::Serilog,
                TemplateSource::Mined => TemplateKind::Mined,
            };
        }
    }
    // Most frequent first (ties: first seen first).
    let mut templates = accs;
    templates.sort_by(|a, b| b.count.cmp(&a.count).then(a.first.cmp(&b.first)));
    Mined { templates }
}

fn re(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).unwrap_or_else(|_| unreachable!("constant regex")))
}

fn crash_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    re(
        &R,
        r"(?i)\b(crash(ed|ing)?|unhandled exception|fatal error|segmentation fault|segfault|core dumped|panic(ked)?|terminated unexpectedly|abnormal(ly)? terminat\w*|out of memory|stack ?overflow|was killed)\b",
    )
}

fn start_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    re(
        &R,
        r"(?i)\b(start(ed|ing|up)?|launch(ed|ing)?|(now )?listening on|initiali[sz](ed|ing)|boot(ed|ing)?|ready to accept|running on)\b",
    )
}

fn stop_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    re(
        &R,
        r"(?i)\b(stop(ped|ping)?|shut(ting)? ?down|exit(ed|ing)?|terminat(ed|ing)|quit(ting)?)\b",
    )
}

fn per_operation_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    re(
        &R,
        r"(?i)\b(request|job|task|transaction|connection|session|download|upload|batch|query|import|export|sync|handshake|timer|span|step|phase)s?\b",
    )
}

/// The lifecycle meaning of a template, if any. A template naming both a start and a stop phrase is
/// what it says first ("Application started. Press Ctrl+C to shut down." is a start).
pub fn classify_lifecycle(template: &str) -> Option<LifecycleRole> {
    if crash_re().is_match(template) {
        return Some(LifecycleRole::Crash);
    }
    if per_operation_re().is_match(template) {
        return None;
    }
    let start = start_re().find(template).map(|m| m.start());
    let stop = stop_re().find(template).map(|m| m.start());
    match (start, stop) {
        (Some(a), Some(b)) if b < a => Some(LifecycleRole::Stop),
        (Some(_), _) => Some(LifecycleRole::Start),
        (None, Some(_)) => Some(LifecycleRole::Stop),
        (None, None) => None,
    }
}

fn example(events: &[Sample], i: usize) -> String {
    events
        .get(i)
        .map(|e| e.rec.message.chars().take(EXAMPLE_CHARS).collect())
        .unwrap_or_default()
}

/// The marker kind on at least half of the template's events.
fn covered_by(t: &MinedTemplate) -> Option<String> {
    t.markers
        .iter()
        .filter(|(_, n)| **n * 2 >= t.count)
        .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
        .map(|(k, _)| k.clone())
}

/// Rare templates that read like a start, stop or crash.
pub fn lifecycle(mined: &Mined, events: &[Sample]) -> Vec<LifecycleCandidate> {
    let rare = (events.len() / 50).max(5);
    let mut out: Vec<LifecycleCandidate> = mined
        .templates
        .iter()
        .filter(|t| t.count <= rare)
        .filter_map(|t| {
            let role = classify_lifecycle(&t.template)?;
            // A start or stop logged as an error is a failure report, not a lifecycle event.
            if role != LifecycleRole::Crash && t.at_least(Level::Error) * 2 > t.count {
                return None;
            }
            Some(LifecycleCandidate {
                role,
                template: t.template.clone(),
                template_kind: t.kind,
                count: t.count,
                example: example(events, t.first),
                covered_by: covered_by(t),
            })
        })
        .collect();
    out.truncate(MAX_LIFECYCLE);
    out
}

/// Templates that are mostly errors (or carry exceptions), most frequent first.
pub fn errors(mined: &Mined, events: &[Sample]) -> Vec<TemplateSummary> {
    let total = events.len().max(1) as f64;
    mined
        .templates
        .iter()
        .filter(|t| t.at_least(Level::Error) * 2 > t.count || t.exceptions * 2 > t.count)
        .take(MAX_ERRORS)
        .map(|t| TemplateSummary {
            template: t.template.clone(),
            template_kind: t.kind,
            count: t.count,
            level: t.level(),
            share: t.count as f64 / total,
            example: example(events, t.first),
        })
        .collect()
}

const NOISE_WORDS: &[&str] = &[
    "healthcheck",
    "health check",
    "health",
    "heartbeat",
    "keepalive",
    "keep-alive",
    "liveness",
    "readiness",
    "ping",
    "pong",
    "still alive",
    "polling",
];

/// Why a frequent template looks like noise: a known low-value activity, or a constant message repeated
/// at a near-constant interval. `times` must be the template's timestamps.
pub fn noise_reason(template: &str, times: &[i64]) -> Option<NoiseReason> {
    let low = template.to_ascii_lowercase();
    if let Some(w) = NOISE_WORDS.iter().find(|w| contains_word(&low, w)) {
        return Some(NoiseReason::Keyword { word: (*w).to_string() });
    }
    let constant = !template.contains(WILDCARD) && !template.contains('{');
    if !constant || times.len() < 20 {
        return None;
    }
    let mut ts = times.to_vec();
    ts.sort_unstable();
    let gaps: Vec<f64> = ts.windows(2).map(|w| (w[1] - w[0]) as f64).filter(|g| *g > 0.0).collect();
    if gaps.len() < 10 {
        return None;
    }
    let mean = gaps.iter().sum::<f64>() / gaps.len() as f64;
    let var = gaps.iter().map(|g| (g - mean).powi(2)).sum::<f64>() / gaps.len() as f64;
    (mean > 0.0 && var.sqrt() / mean < 0.2).then(|| NoiseReason::Periodic {
        interval_ms: mean.round() as i64,
    })
}

fn contains_word(haystack: &str, word: &str) -> bool {
    haystack.match_indices(word).any(|(i, _)| {
        let before = haystack[..i].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
        let after = haystack[i + word.len()..].chars().next().is_none_or(|c| !c.is_alphanumeric());
        before && after
    })
}

/// Frequent low-level templates that look like noise.
pub fn noise(mined: &Mined, events: &[Sample]) -> Vec<NoiseCandidate> {
    let total = events.len().max(1) as f64;
    mined
        .templates
        .iter()
        .filter(|t| t.count >= 20 && t.count as f64 / total >= 0.02 && t.at_least(Level::Warning) * 10 < t.count)
        .filter_map(|t| {
            let reason = noise_reason(&t.template, &t.times)?;
            Some(NoiseCandidate {
                template: t.template.clone(),
                template_kind: t.kind,
                count: t.count,
                share: t.count as f64 / total,
                level: t.level(),
                reason,
                example: example(events, t.first),
            })
        })
        .take(MAX_NOISE)
        .collect()
}

/// How many sampled events each existing marker kind matches (`kinds` as the store numbers them; label and
/// lifecycle from the profile's `[[marker]]`, the kind itself for plugin kinds).
pub fn coverage(kinds: &[String], profile: &crate::Profile, events: &[Sample]) -> Vec<MarkerCoverage> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for e in events {
        for m in &e.markers {
            *counts.entry(m.as_str()).or_default() += 1;
        }
    }
    kinds
        .iter()
        .map(|k| {
            let cfg = profile.marker.iter().find(|m| &m.kind == k);
            MarkerCoverage {
                kind: k.clone(),
                label: cfg.and_then(|m| m.label.clone()).unwrap_or_else(|| k.clone()),
                lifecycle: cfg.is_some_and(|m| m.lifecycle),
                matches: counts.get(k.as_str()).copied().unwrap_or(0),
            }
        })
        .collect()
}

/// A regex matching the messages of a mined template: literal tokens escaped, `<*>` as any
/// text, tokens separated by whitespace. Anchored at the start only (long messages are mined truncated).
pub fn template_regex(template: &str) -> String {
    let tokens: Vec<String> = template
        .split_whitespace()
        .map(|tok| tok.split(WILDCARD).map(regex::escape).collect::<Vec<_>>().join(".*?"))
        .collect();
    format!("^{}", tokens.join(r"\s+"))
}
