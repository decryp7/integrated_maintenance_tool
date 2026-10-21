//! Annotators: plugin contributions that derive time ranges (spans) and
//! points from the workspace, e.g. a reboot window or a service's uptime.
//!
//! An [`Annotator`] reads the workspace through the read-only, bounded
//! [`AnnotationView`] and returns its complete set of [`Annotation`]s, which
//! replaces what it returned before. It runs after every import and after
//! live-tail appends (`since` then points at the first new record, so it can
//! limit its work; it still returns its whole set). Each run is guarded like
//! every plugin call (panics contained, watchdog budget
//! [`crate::guard::Budgets::annotate`]) and capped at [`MAX_ANNOTATIONS`].

use crate::api::Severity;
use crate::guard::{CallKind, Guard};
use crate::query::Query;
use anyhow::Result;
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// Most annotations kept per annotator.
pub const MAX_ANNOTATIONS: usize = 10_000;
/// Most records one [`AnnotationView`] call returns.
pub const MAX_VIEW_RECORDS: usize = 100_000;
/// Most annotations one [`super::PluginRegistry::annotations`] reply holds.
pub const MAX_REPLY: usize = 20_000;

pub use imt_plugin_api::annotate::{RecordCursor, ViewFilter, ViewRecord};

/// Read-only, bounded queries over the workspace for annotators.
pub trait AnnotationView {
    /// Records matching `filter`, in time order, at most `limit` (and
    /// [`MAX_VIEW_RECORDS`]). Fails on an invalid condition.
    fn select(&self, filter: &ViewFilter, limit: usize) -> Result<Vec<ViewRecord>>;
    /// Records matching a query (text, filters, query language, time range),
    /// oldest first, at most `limit` (and [`MAX_VIEW_RECORDS`]).
    fn query(&self, query: &Query, limit: usize) -> Result<Vec<ViewRecord>>;
    /// Instance keys, in first-seen order.
    fn instances(&self) -> Vec<String>;
    /// Time of the oldest and newest record.
    fn time_range(&self) -> Option<(i64, i64)>;
}

pub use imt_plugin_api::annotate::FieldMatch;

/// A point (`from == to`) or range an annotator found.
#[derive(Debug, Clone)]
pub struct Annotation {
    /// Short kind (namespaced `<plugin-id>/<kind>` when stored), ideally one
    /// of the plugin's marker kinds so it shares their style.
    pub kind: String,
    pub from: i64,
    pub to: i64,
    /// The instance it applies to; `None` = every instance (or those
    /// matching `on`).
    pub instance: Option<String>,
    pub on: Option<FieldMatch>,
    pub label: String,
    pub severity: Severity,
    /// The query that shows the evidence.
    pub evidence_query: Query,
}

/// Contributes annotations; see the module docs.
pub trait Annotator: Send + Sync {
    /// Stable id, unique across plugins.
    fn id(&self) -> &str;
    /// The complete set of annotations for the workspace as it is now.
    fn annotate(&self, view: &dyn AnnotationView, since: Option<RecordCursor>) -> Result<Vec<Annotation>>;
}

/// An annotation as the UI receives it.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationDto {
    pub annotator: String,
    /// Plugin id that contributed it.
    pub owner: String,
    /// `<plugin-id>/<kind>`.
    pub kind: String,
    pub from: i64,
    pub to: i64,
    pub instance: Option<String>,
    pub on: Option<FieldMatch>,
    pub label: String,
    pub severity: Severity,
    pub evidence_query: Query,
    /// Set on runs between lifecycle markers (`[[span]]`, owner `profile`); absent on plugin annotations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run: Option<crate::spans::RunInfo>,
}

/// Which annotations to return.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AnnotationsRequest {
    /// Overlapping `[from, to]` (inclusive); open-ended when absent.
    pub from: Option<i64>,
    pub to: Option<i64>,
    /// Only those for this instance (plus the ones for every instance).
    pub instance: Option<String>,
}

/// An annotator under its plugin's guard (panics become errors and count
/// towards quarantine; runs are watched against the annotate budget). Used
/// when an annotator is called outside [`super::PluginRegistry::run_annotators`],
/// e.g. by the plugin host process.
pub(crate) struct GuardedAnnotator {
    pub inner: Arc<dyn Annotator>,
    pub guard: Guard,
}

impl Annotator for GuardedAnnotator {
    fn id(&self) -> &str {
        self.inner.id()
    }
    fn annotate(&self, view: &dyn AnnotationView, since: Option<RecordCursor>) -> Result<Vec<Annotation>> {
        self.guard.call(CallKind::Annotate, "annotate", || self.inner.annotate(view, since))
    }
}

pub(crate) struct Registered {
    pub owner: String,
    pub guard: Option<Guard>,
    pub annotator: Arc<dyn Annotator>,
}

/// The annotations of every annotator, replaced per annotator on each run.
#[derive(Default)]
pub(crate) struct Store {
    by: Mutex<BTreeMap<String, Vec<AnnotationDto>>>,
}

impl Store {
    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Vec<AnnotationDto>>> {
        self.by.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn clear(&self) {
        self.lock().clear();
    }

    /// Runs every annotator whose plugin is available; returns notes (owner, text).
    pub fn run(
        &self,
        annotators: &[Registered],
        view: &dyn AnnotationView,
        since: Option<RecordCursor>,
        available: &dyn Fn(&str) -> bool,
    ) -> Vec<(String, String)> {
        let mut notes = Vec::new();
        for a in annotators {
            let id = a.annotator.id().to_string();
            if a.guard.is_some() && !available(&a.owner) {
                continue;
            }
            let result = match &a.guard {
                Some(g) => g.call(CallKind::Annotate, "annotate", || a.annotator.annotate(view, since)),
                None => a.annotator.annotate(view, since),
            };
            match result {
                Ok(list) => {
                    if list.len() > MAX_ANNOTATIONS {
                        notes.push((
                            a.owner.clone(),
                            format!(
                                "annotator \"{id}\" returned {} annotations; only the first {MAX_ANNOTATIONS} are kept",
                                list.len()
                            ),
                        ));
                    }
                    let items = list
                        .into_iter()
                        .take(MAX_ANNOTATIONS)
                        .map(|x| AnnotationDto {
                            annotator: id.clone(),
                            owner: a.owner.clone(),
                            kind: if x.kind.contains('/') {
                                x.kind
                            } else {
                                format!("{}/{}", a.owner, x.kind)
                            },
                            from: x.from.min(x.to),
                            to: x.from.max(x.to),
                            instance: x.instance,
                            on: x.on,
                            label: x.label.chars().take(500).collect(),
                            severity: x.severity,
                            evidence_query: x.evidence_query,
                            run: None,
                        })
                        .collect();
                    self.lock().insert(id, items);
                }
                // Keep the previous set; panics are already counted by the guard.
                Err(e) => notes.push((a.owner.clone(), format!("annotator \"{id}\" failed: {e:#}"))),
            }
        }
        notes
    }

    /// Stored annotations overlapping the request, of available plugins, by start time.
    pub fn query(&self, req: &AnnotationsRequest, available: &dyn Fn(&str) -> bool) -> Vec<AnnotationDto> {
        let by = self.lock();
        let mut out: Vec<AnnotationDto> = by
            .values()
            .flatten()
            .filter(|a| available(&a.owner))
            .filter(|a| req.from.is_none_or(|f| a.to >= f) && req.to.is_none_or(|t| a.from <= t))
            .filter(|a| match (&req.instance, &a.instance) {
                (Some(want), Some(have)) => want == have,
                _ => true,
            })
            .take(MAX_REPLY)
            .cloned()
            .collect();
        out.sort_by(|a, b| a.from.cmp(&b.from).then_with(|| a.kind.cmp(&b.kind)));
        out
    }
}

#[cfg(test)]
mod tests;
