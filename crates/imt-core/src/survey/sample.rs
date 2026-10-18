//! Bounded sampling through the [`LogStore`]: sources are grouped by importer and logical log, and each
//! group contributes events spread over its whole time range (not only its first lines).

use super::dto::SourceGroup;
use super::{SurveyControl, SurveyError, SurveyLimits, SurveyStage};
use crate::api::{PageRequest, WorkspaceSummary};
use crate::model::Record;
use crate::query::Query;
use crate::LogStore;
use std::collections::BTreeMap;
use std::time::Instant;

/// Slices read per large group, so the sample covers its beginning, middle and end.
const SLICES: usize = 4;
/// Largest page the store serves.
const PAGE: usize = 5_000;

/// One sampled event with what the survey needs besides the record.
#[derive(Debug, Clone)]
pub struct Sample {
    pub rec: Record,
    pub source_name: String,
    /// Marker kinds the active profile gave the event.
    pub markers: Vec<String>,
    /// The event's timeline lane under the active profile.
    pub lane: String,
    /// Index into [`Sampled::groups`].
    pub group: usize,
}

#[derive(Debug, Clone, Default)]
pub struct Sampled {
    pub events: Vec<Sample>,
    pub groups: Vec<SourceGroup>,
    pub truncated: bool,
}

struct GroupAcc {
    importer: String,
    name: String,
    ids: Vec<u32>,
    events: usize,
    from: Option<i64>,
    to: Option<i64>,
}

pub(super) fn collect<S: LogStore + ?Sized>(
    store: &S,
    summary: &WorkspaceSummary,
    limits: &SurveyLimits,
    control: &SurveyControl,
    started: Instant,
) -> Result<Sampled, SurveyError> {
    let mut by_key: BTreeMap<(String, String), GroupAcc> = BTreeMap::new();
    for s in summary.sources.iter().filter(|s| s.records > 0) {
        let g = by_key.entry((s.importer.clone(), s.group.clone())).or_insert_with(|| GroupAcc {
            importer: s.importer.clone(),
            name: s.group.clone(),
            ids: Vec::new(),
            events: 0,
            from: None,
            to: None,
        });
        g.ids.push(s.id);
        g.events += s.records;
        g.from = min_opt(g.from, s.from);
        g.to = max_opt(g.to, s.to);
    }
    let mut accs: Vec<GroupAcc> = by_key.into_values().collect();
    accs.sort_by(|a, b| b.events.cmp(&a.events).then(a.name.cmp(&b.name)));
    let mut out = Sampled::default();
    if accs.len() > limits.groups {
        accs.truncate(limits.groups);
        out.truncated = true;
    }
    control.begin(SurveyStage::Sampling, accs.len());
    for acc in accs {
        control.check()?;
        let index = out.groups.len();
        let remaining = limits.total.saturating_sub(out.events.len());
        if remaining == 0 || started.elapsed() > limits.time {
            out.truncated = true;
        }
        let want = if out.truncated { 0 } else { limits.per_group.min(remaining) };
        let before = out.events.len();
        if want > 0 {
            read_group(store, &acc.ids, acc.events, want, index, &mut out.events, control)?;
        }
        let sampled = out.events.len() - before;
        out.groups.push(SourceGroup {
            id: format!("{}:{}", acc.importer, acc.name),
            importer: acc.importer,
            name: acc.name,
            files: acc.ids.len(),
            events: acc.events,
            sampled,
            from: acc.from,
            to: acc.to,
        });
        control.advance(1);
    }
    Ok(out)
}

fn read_group<S: LogStore + ?Sized>(
    store: &S,
    ids: &[u32],
    events: usize,
    want: usize,
    group: usize,
    out: &mut Vec<Sample>,
    control: &SurveyControl,
) -> Result<(), SurveyError> {
    let query = Query {
        sources: ids.iter().copied().take(crate::query::MAX_QUERY_SOURCES).collect(),
        ..Query::default()
    };
    let slices = if events <= want { 1 } else { SLICES };
    let base = want / slices;
    for i in 0..slices {
        control.check()?;
        let offset = if slices == 1 { 0 } else { i * events / slices };
        let mut taken = 0;
        let per_slice = base + usize::from(i < want % slices);
        while taken < per_slice {
            let req = PageRequest {
                query: query.clone(),
                offset: offset + taken,
                limit: (per_slice - taken).min(PAGE),
                descending: false,
                anchor: None,
            };
            let page = store.page(&req).map_err(|e| SurveyError::Invalid(format!("{e:#}")))?;
            if page.rows.is_empty() {
                break;
            }
            taken += page.rows.len();
            for v in page.rows {
                let mut rec = Record::new(v.line);
                rec.id = v.id;
                rec.ts = v.ts;
                rec.level = v.level;
                rec.message = v.message.into_owned();
                rec.template = v.template.map(|t| t.into_owned());
                rec.exception = v.exception.map(|x| x.into_owned());
                rec.fields = v.fields.into_owned();
                rec.source = v.source;
                out.push(Sample {
                    rec,
                    source_name: v.source_name.to_string(),
                    markers: v.markers.iter().map(|m| m.to_string()).collect(),
                    lane: v.group,
                    group,
                });
            }
        }
    }
    Ok(())
}

fn min_opt(a: Option<i64>, b: Option<i64>) -> Option<i64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (x, y) => x.or(y),
    }
}

fn max_opt(a: Option<i64>, b: Option<i64>) -> Option<i64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.max(y)),
        (x, y) => x.or(y),
    }
}
