//! Annotators: contributions that derive time ranges (spans) and points from the events, e.g. a reboot window
//! or a service's uptime.
//!
//! An [`Annotator`] reads the events through the read-only, bounded [`AnnotationView`] (in a plugin process,
//! each call goes to the host) and returns its complete set of [`Annotation`]s, which replaces what it
//! returned before. It runs after every import and after live-tail appends (`since` then points at the first
//! new record, so it can limit its work; it still returns its whole set).

use crate::condition::ConditionCfg;
use crate::model::Record;
use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Most annotations kept per annotator.
pub const MAX_ANNOTATIONS: usize = 10_000;
/// Most records one [`AnnotationView`] call returns.
pub const MAX_VIEW_RECORDS: usize = 100_000;

/// The first record that is new since the previous run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordCursor {
    /// Id of the record.
    pub id: u32,
    /// Its timestamp (milliseconds since the Unix epoch), when it has one.
    pub ts: Option<i64>,
}

/// Which records a [`AnnotationView::select`] returns.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ViewFilter {
    /// All must match (the `[[marker]] when` condition language).
    pub when: Vec<ConditionCfg>,
    /// Inclusive time bounds (records without a time never match bounds).
    pub from: Option<i64>,
    /// Inclusive upper time bound in milliseconds since the Unix epoch.
    pub to: Option<i64>,
    /// Newest first instead of oldest first.
    pub newest_first: bool,
}

/// A record as an annotator sees it.
#[derive(Debug, Clone)]
pub struct ViewRecord {
    /// The event itself.
    pub record: Record,
    /// Display name of its source file.
    pub source: String,
    /// Its instance (timeline lane) key.
    pub instance: String,
}

/// Read-only, bounded queries over the events for annotators.
pub trait AnnotationView {
    /// Records matching `filter`, in time order, at most `limit` (and [`MAX_VIEW_RECORDS`]). Fails on an
    /// invalid condition.
    fn select(&self, filter: &ViewFilter, limit: usize) -> Result<Vec<ViewRecord>>;
    /// Instance keys, in first-seen order.
    fn instances(&self) -> Vec<String>;
    /// Time of the oldest and newest record.
    fn time_range(&self) -> Option<(i64, i64)>;
}

/// Records with `field == value` (e.g. `Computer = WEB01`): a machine-wide range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldMatch {
    /// The field to compare (a profile field or a pseudo-field such as `@level`).
    pub field: String,
    /// The value it must equal.
    pub value: String,
}

/// How urgent an annotation is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    /// Needs attention now.
    Critical,
    /// Worth a look.
    Warning,
    /// For information only.
    Info,
}

/// One filter of an [`Evidence`] query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceFilter {
    /// The field to compare.
    pub field: String,
    /// `eq`, `ne`, `contains`, ...
    pub op: String,
    /// The value to compare with; absent for operators that take none.
    pub value: Option<String>,
}

/// The query that shows an annotation's evidence (the host turns it into its query).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Evidence {
    /// Free text.
    pub text: String,
    /// Start of the time window in milliseconds since the Unix epoch.
    pub from: Option<i64>,
    /// End of the time window in milliseconds since the Unix epoch.
    pub to: Option<i64>,
    /// Further field filters, all of which must match.
    pub filters: Vec<EvidenceFilter>,
}

/// A point (`from == to`) or range an annotator found.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Annotation {
    /// Short kind (namespaced `<plugin-id>/<kind>` when stored), ideally one of the plugin's marker kinds so it
    /// shares their style.
    pub kind: String,
    /// Start of the range in milliseconds since the Unix epoch.
    pub from: i64,
    /// End of the range in milliseconds since the Unix epoch (equal to `from` for a point).
    pub to: i64,
    /// The instance it applies to; `None` = every instance (or those matching `on`).
    pub instance: Option<String>,
    /// Restricts the annotation to records matching a field value, e.g. a whole machine.
    pub on: Option<FieldMatch>,
    /// Text shown for the annotation.
    pub label: String,
    /// How urgent it is.
    pub severity: Severity,
    /// The query that shows the records behind it.
    pub evidence: Evidence,
}

/// Contributes annotations; see the module docs.
pub trait Annotator: Send + Sync {
    /// Stable id, unique across plugins.
    fn id(&self) -> &str;
    /// The complete set of annotations for the events as they are now.
    fn annotate(&self, view: &dyn AnnotationView, since: Option<RecordCursor>) -> Result<Vec<Annotation>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn annotations_and_filters_roundtrip_as_json() {
        let a = Annotation {
            kind: "downtime".into(),
            from: 1,
            to: 2,
            instance: None,
            on: Some(FieldMatch {
                field: "Computer".into(),
                value: "WEB01".into(),
            }),
            label: "Reboot".into(),
            severity: Severity::Warning,
            evidence: Evidence {
                text: "x".into(),
                from: Some(1),
                to: Some(2),
                filters: vec![EvidenceFilter {
                    field: "Computer".into(),
                    op: "eq".into(),
                    value: Some("WEB01".into()),
                }],
            },
        };
        let back: Annotation = serde_json::from_value(serde_json::to_value(&a).unwrap()).unwrap();
        assert_eq!(back, a);
        let f: ViewFilter = serde_json::from_str(r#"{"newestFirst":true}"#).unwrap();
        assert!(f.newest_first && f.when.is_empty());
    }
}
