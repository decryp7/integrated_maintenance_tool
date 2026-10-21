//! "Set up analysis for this folder": a bounded survey of what is loaded, heuristic proposals built from
//! it, and the engine's validation and preview of every proposal (heuristic or AI) before the user sees it.
//!
//! - [`run`] samples each source group through the [`SurveyStore`] (capped per group, in total and in time;
//!   cancellable through a [`SurveyControl`]), computes the [`FolderSurvey`] facts, turns them into
//!   [`Candidate`]s and validates them ([`validate()`]). Nothing is changed.
//! - [`compose`] merges selected [`Fragment`]s into the active profile's TOML, validates the result
//!   against the store's plugins and returns it with a line diff, ready for the normal profile save.
//!
//! Pure analysis lives in the submodules (fields, templates, numbers); this module only sequences them.

mod diff;
mod dto;
mod fields;
mod fragment;
mod numbers;
mod propose;
mod sample;
mod templates;
mod unrecognised;
mod validate;

#[cfg(test)]
mod tests;

pub use diff::line_diff;
pub use dto::*;
pub use fields::{field_summaries, instance_candidates};
pub use fragment::{compose_toml, fragment_toml};
pub use numbers::number_candidates;
pub use propose::propose;
pub use sample::{Sample, Sampled};
pub use templates::{classify_lifecycle, noise_reason, template_regex};
pub use validate::validate;

/// What the survey reads: the store's queries and profile operations (any store adapter).
pub trait SurveyStore: crate::ports::Query + crate::ports::Profiles {}
impl<T: crate::ports::Query + crate::ports::Profiles + ?Sized> SurveyStore for T {}

fn store_failed(e: crate::ports::StoreError) -> SurveyError {
    SurveyError::Invalid(e.to_string())
}
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Why a survey produced nothing.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SurveyError {
    #[error("the survey was cancelled")]
    Cancelled,
    #[error("nothing is open: open a folder first")]
    NothingOpened,
    #[error("{0}")]
    Invalid(String),
}

/// Bounds of one survey. Defaults keep a survey of a large folder to a few seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurveyLimits {
    pub per_group: usize,
    pub total: usize,
    pub groups: usize,
    pub time: Duration,
}

impl SurveyLimits {
    pub const MAX_PER_GROUP: usize = 20_000;
    pub const MAX_TOTAL: usize = 100_000;
    pub const MAX_MILLIS: u64 = 60_000;
    pub const GROUPS: usize = 64;

    pub fn from_request(req: &SurveyRequest) -> SurveyLimits {
        SurveyLimits {
            per_group: req.max_events_per_group.unwrap_or(2_000).clamp(10, Self::MAX_PER_GROUP),
            total: req.max_events.unwrap_or(20_000).clamp(10, Self::MAX_TOTAL),
            groups: Self::GROUPS,
            time: Duration::from_millis(req.max_millis.unwrap_or(5_000).clamp(100, Self::MAX_MILLIS)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SurveyStage {
    Sampling,
    Analysing,
    Proposing,
    Validating,
    Done,
}

impl SurveyStage {
    fn from_u8(v: u8) -> SurveyStage {
        match v {
            0 => SurveyStage::Sampling,
            1 => SurveyStage::Analysing,
            2 => SurveyStage::Proposing,
            3 => SurveyStage::Validating,
            _ => SurveyStage::Done,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SurveyProgress {
    pub stage: SurveyStage,
    pub done: usize,
    pub total: usize,
}

#[derive(Debug, Default)]
struct ControlInner {
    cancel: AtomicBool,
    stage: AtomicU8,
    done: AtomicUsize,
    total: AtomicUsize,
}

/// Cancellation and progress of a survey, shared with the caller (cheap to clone).
#[derive(Debug, Clone, Default)]
pub struct SurveyControl(Arc<ControlInner>);

impl SurveyControl {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.cancel.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.cancel.load(Ordering::SeqCst)
    }

    /// `Err(Cancelled)` once cancelled.
    pub fn check(&self) -> Result<(), SurveyError> {
        if self.is_cancelled() {
            Err(SurveyError::Cancelled)
        } else {
            Ok(())
        }
    }

    pub fn begin(&self, stage: SurveyStage, total: usize) {
        self.0.stage.store(stage as u8, Ordering::SeqCst);
        self.0.total.store(total, Ordering::SeqCst);
        self.0.done.store(0, Ordering::SeqCst);
    }

    pub fn advance(&self, n: usize) {
        self.0.done.fetch_add(n, Ordering::SeqCst);
    }

    pub fn progress(&self) -> SurveyProgress {
        SurveyProgress {
            stage: SurveyStage::from_u8(self.0.stage.load(Ordering::SeqCst)),
            done: self.0.done.load(Ordering::SeqCst),
            total: self.0.total.load(Ordering::SeqCst),
        }
    }
}

/// Surveys what `store` has loaded and returns validated heuristic proposals. `base_toml` is the active
/// profile as the user wrote it (proposals are merged over it).
pub fn run<S: SurveyStore + ?Sized>(
    store: &S,
    base_toml: &str,
    req: &SurveyRequest,
    control: &SurveyControl,
) -> Result<FolderSetup, SurveyError> {
    let started = Instant::now();
    let limits = SurveyLimits::from_request(req);
    let summary = store.summary().map_err(store_failed)?;
    if summary.sources.is_empty() && summary.unrecognized.is_empty() {
        return Err(SurveyError::NothingOpened);
    }
    let profile = store.active_profile().map_err(store_failed)?;
    let sampled = sample::collect(store, &summary, &limits, control, started)?;
    let survey = analyse(&sampled, &summary, &profile, control, started)?;
    control.check()?;
    control.begin(SurveyStage::Proposing, 1);
    let candidates = propose(&survey, &profile);
    control.advance(1);
    let (proposals, skipped) = validate(store, base_toml, &survey, &sampled, candidates, control)?;
    control.begin(SurveyStage::Done, 0);
    let mut survey = survey;
    survey.elapsed_ms = started.elapsed().as_millis() as u64;
    Ok(FolderSetup {
        survey,
        proposals,
        skipped,
    })
}

/// Surveys `store` without proposing anything (the facts the AI refinement is grounded in) and returns
/// the samples too, for validating further proposals.
pub fn survey_only<S: SurveyStore + ?Sized>(
    store: &S,
    req: &SurveyRequest,
    control: &SurveyControl,
) -> Result<(FolderSurvey, Sampled), SurveyError> {
    let started = Instant::now();
    let limits = SurveyLimits::from_request(req);
    let summary = store.summary().map_err(store_failed)?;
    if summary.sources.is_empty() && summary.unrecognized.is_empty() {
        return Err(SurveyError::NothingOpened);
    }
    let profile = store.active_profile().map_err(store_failed)?;
    let sampled = sample::collect(store, &summary, &limits, control, started)?;
    let mut survey = analyse(&sampled, &summary, &profile, control, started)?;
    survey.elapsed_ms = started.elapsed().as_millis() as u64;
    Ok((survey, sampled))
}

fn analyse(
    sampled: &Sampled,
    summary: &crate::ports::SummaryReply,
    profile: &crate::Profile,
    control: &SurveyControl,
    started: Instant,
) -> Result<FolderSurvey, SurveyError> {
    control.check()?;
    control.begin(SurveyStage::Analysing, 5);
    let fields = field_summaries(&sampled.events);
    control.advance(1);
    let instance_candidates = instance_candidates(&sampled.events, &fields, &profile.instance.candidates);
    control.advance(1);
    control.check()?;
    let mined = templates::mine(&sampled.events);
    let lifecycle = templates::lifecycle(&mined, &sampled.events);
    let errors = templates::errors(&mined, &sampled.events);
    let noise = templates::noise(&mined, &sampled.events);
    control.advance(1);
    control.check()?;
    let numbers = number_candidates(&sampled.events);
    control.advance(1);
    let markers = templates::coverage(&summary.marker_kinds, profile, &sampled.events);
    let unrecognised = unrecognised::groups(&summary.unrecognized);
    control.advance(1);
    Ok(FolderSurvey {
        total_events: summary.total,
        sampled_events: sampled.events.len(),
        truncated: sampled.truncated,
        elapsed_ms: started.elapsed().as_millis() as u64,
        groups: sampled.groups.clone(),
        unrecognised,
        fields,
        instance_candidates,
        lifecycle,
        errors,
        noise,
        numbers,
        markers,
    })
}

/// Merges `fragments` into `base_toml`, validates the result against `store` (its plugins) and returns it
/// with a diff. Saved queries are checked with the query language and returned separately.
pub fn compose<S: SurveyStore + ?Sized>(store: &S, base_toml: &str, fragments: &[Fragment]) -> anyhow::Result<ComposedProfile> {
    const MAX_FRAGMENTS: usize = 200;
    if fragments.is_empty() {
        anyhow::bail!("select at least one proposal");
    }
    if fragments.len() > MAX_FRAGMENTS {
        anyhow::bail!("at most {MAX_FRAGMENTS} changes can be applied at once");
    }
    let mut saved_queries = Vec::new();
    for f in fragments {
        if let Fragment::SavedQuery { name, ql } = f {
            if name.trim().is_empty() || name.chars().count() > 120 {
                anyhow::bail!("a saved query needs a name of 1 to 120 characters");
            }
            if let Some(e) = store.check_query(ql).map_err(|e| anyhow::anyhow!("{e}"))? {
                anyhow::bail!("the saved query \"{name}\" is not valid: {}", e.message);
            }
            saved_queries.push(SavedQueryFragment {
                name: name.clone(),
                ql: ql.clone(),
            });
        }
    }
    let (before, after) = compose_toml(base_toml, fragments)?;
    let profile = crate::Profile::from_toml(&after).map_err(|e| anyhow::anyhow!("the combined profile is not valid: {e}"))?;
    store
        .validate_profile(&profile)
        .map_err(|e| anyhow::anyhow!("the combined profile does not compile: {e:#}"))?;
    let diff = line_diff(&before, &after);
    let added = diff.iter().filter(|d| d.op == DiffOp::Add).count();
    let removed = diff.iter().filter(|d| d.op == DiffOp::Remove).count();
    Ok(ComposedProfile {
        toml: after,
        diff,
        added,
        removed,
        saved_queries,
    })
}
