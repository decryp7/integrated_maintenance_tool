//! The redacted view over any store, and what redaction would change in the events of a query.
//!
//! The view is a [`Redactor`] the app holds for a window while the user has switched redaction on
//! ([`view_redactor`]); the `view_*` methods apply it to every reply that shows event text: pages and events by
//! id, facets, source context, query-language tables and dashboard tiles. [`summary`] counts what the profile's
//! rules would change in the events matching a query (shown before an export, report or bundle is written),
//! scanning at most [`MAX_SCANNED`] events through the [`Query`] port.

use super::{hits_of, Counts, RedactCfg, Redactor, RuleHits, Salt};
use crate::api::{Facet, FacetValue, SourceContext};
use crate::dashboard::TileData;
use crate::model::Record;
use crate::ports::{EventView, Filter, PageReply, Query, StoreError, StoreResult};
use crate::ql::QlResult;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::borrow::Cow;
use std::ops::ControlFlow;

/// Most events a redaction summary scans.
pub const MAX_SCANNED: usize = 200_000;
/// Most before/after samples in a summary.
pub const MAX_SAMPLES: usize = 20;

/// Why the view cannot be switched on.
pub const NO_RULES: &str = "the profile has no redaction rules: add detectors or [[redact.rule]] to its [redact] section";

/// `preview_redaction`: a sample redacted with the profile's rules, or with those of an unsaved profile.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedactionPreviewRequest {
    pub sample: String,
    /// A candidate profile (TOML); absent = the active profile.
    #[serde(default)]
    pub toml: Option<String>,
}

/// `redaction_summary`: what redaction would change in the events of a query.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedactionSummaryRequest {
    #[serde(default)]
    pub query: Filter,
    #[serde(default = "default_samples")]
    pub samples: usize,
}

fn default_samples() -> usize {
    5
}

/// One event before and after redaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedactionSample {
    pub record_id: u32,
    pub before: String,
    pub after: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedactionSummary {
    /// Events looked at (at most [`MAX_SCANNED`]).
    pub scanned: usize,
    /// Of those, events with at least one match.
    pub changed: usize,
    /// More events matched the query than were scanned.
    pub truncated: bool,
    pub hits: Vec<RuleHits>,
    pub samples: Vec<RedactionSample>,
}

/// The redactor of a redacted view: the profile's rules with a fresh salt (hashes are stable while the view is on
/// and differ from those of exports). Fails, saying what to add, when the profile has no rules.
pub fn view_redactor(cfg: &RedactCfg) -> anyhow::Result<Redactor> {
    let r = Redactor::from_config(cfg, Salt::random())?;
    if r.is_empty() {
        anyhow::bail!(NO_RULES);
    }
    Ok(r)
}

/// Replaces `text` with its redacted form when a rule matched.
fn redact_text(text: &mut String, f: impl FnOnce(&str) -> Cow<'_, str>) {
    let changed = match f(text) {
        Cow::Owned(s) => Some(s),
        Cow::Borrowed(_) => None,
    };
    if let Some(s) = changed {
        *text = s;
    }
}

impl Redactor {
    fn view_field(&self, name: &str, text: &mut String) {
        redact_text(text, |t| self.field(name, Cow::Borrowed(t), &mut Counts::new()));
    }

    fn view_fields(&self, fields: &mut Map<String, Value>) {
        let changed = match self.fields(fields, &mut Counts::new()) {
            Cow::Owned(m) => Some(m),
            Cow::Borrowed(_) => None,
        };
        if let Some(m) = changed {
            *fields = m;
        }
    }

    /// An event as the redacted view shows it: message, template, exception and properties in scope.
    pub fn view_event(&self, e: &mut EventView) {
        self.view_field("@message", &mut e.message);
        if let Some(t) = &mut e.template {
            self.view_field("@template", t);
        }
        if let Some(x) = &mut e.exception {
            self.view_field("@exception", x);
        }
        self.view_fields(&mut e.fields);
    }

    pub fn view_events(&self, events: &mut [EventView]) {
        events.iter_mut().for_each(|e| self.view_event(e));
    }

    pub fn view_page(&self, page: &mut PageReply) {
        self.view_events(&mut page.rows);
    }

    /// A facet's values as the view shows them: values that redact to the same text are counted together (among
    /// the values the facet holds), most frequent first.
    pub fn view_facet(&self, facet: &mut Facet) {
        if !self.scope().covers(&facet.field) {
            return;
        }
        let held = facet.values.len();
        let mut merged: Vec<FacetValue> = Vec::with_capacity(held);
        let mut at: std::collections::HashMap<String, usize> = std::collections::HashMap::with_capacity(held);
        for v in facet.values.drain(..) {
            let value = self.field(&facet.field, Cow::Borrowed(v.value.as_str()), &mut Counts::new()).into_owned();
            match at.get(&value) {
                Some(&i) => merged[i].count += v.count,
                None => {
                    at.insert(value.clone(), merged.len());
                    merged.push(FacetValue { value, count: v.count });
                }
            }
        }
        // A stable sort keeps the store's order among equal counts.
        merged.sort_by(|a, b| b.count.cmp(&a.count));
        facet.distinct = facet.distinct.saturating_sub(held - merged.len());
        facet.values = merged;
    }

    /// The original lines around an event (always fully redacted: a line is not split into fields).
    pub fn view_context(&self, context: &mut SourceContext) {
        for l in &mut context.lines {
            redact_text(&mut l.text, |t| self.apply(t, &mut Counts::new()));
        }
    }

    /// A table's cells (strings, and the strings inside lists and objects) in the columns in scope.
    pub fn view_table(&self, columns: &[String], rows: &mut [Vec<Value>]) {
        let covered: Vec<bool> = columns.iter().map(|c| self.scope().covers(c)).collect();
        if !covered.iter().any(|c| *c) {
            return;
        }
        for row in rows {
            for (cell, _) in row.iter_mut().zip(&covered).filter(|(_, c)| **c) {
                if matches!(cell, Value::String(_) | Value::Array(_) | Value::Object(_)) {
                    *cell = self.apply_json(cell, &mut Counts::new());
                }
            }
        }
    }

    /// A query-language result (event lists are ids: the page that shows them is redacted).
    pub fn view_ql(&self, result: &mut QlResult) {
        if let QlResult::Table { columns, rows } = result {
            self.view_table(columns, rows);
        }
    }

    /// A dashboard tile: tables, top values and a stat's text.
    pub fn view_tile(&self, tile: &mut TileData) {
        match tile {
            TileData::Table { columns, rows, .. } => self.view_table(columns, rows),
            TileData::Top { facet } => self.view_facet(facet),
            TileData::Stat { text, label, value: None, .. } => self.view_field(label, text),
            TileData::Stat { .. } | TileData::Chart { .. } | TileData::Alerts { .. } | TileData::Annotations { .. } => {}
        }
    }
}

/// What the rules of `cfg` would change in the events matching `req.query`: hits per rule over at most
/// [`MAX_SCANNED`] of them (scanned in time order), with up to `req.samples` (at most [`MAX_SAMPLES`]) before/after
/// samples. A redaction configuration that does not compile is [`StoreError::Invalid`].
pub fn summary<S: Query + ?Sized>(store: &S, cfg: &RedactCfg, req: &RedactionSummaryRequest) -> StoreResult<RedactionSummary> {
    summary_within(store, cfg, req, MAX_SCANNED)
}

/// [`summary`] scanning at most `max_scanned` events (tests, and callers with a tighter budget).
pub fn summary_within<S: Query + ?Sized>(
    store: &S,
    cfg: &RedactCfg,
    req: &RedactionSummaryRequest,
    max_scanned: usize,
) -> StoreResult<RedactionSummary> {
    let max_scanned = max_scanned.min(MAX_SCANNED);
    let redactor = Redactor::from_config(cfg, Salt::random()).map_err(|e| StoreError::Invalid(format!("{e:#}")))?;
    let wanted = req.samples.min(MAX_SAMPLES);
    let mut counts = Counts::new();
    let (mut scanned, mut changed, mut truncated) = (0usize, 0usize, false);
    let mut samples = Vec::new();
    store.scan(&req.query, &mut |batch| {
        for row in batch.rows() {
            if scanned == max_scanned {
                truncated = true;
                return ControlFlow::Break(());
            }
            scanned += 1;
            let before = crate::row::to_record(&row);
            if let Cow::Owned(after) = redactor.record(&before, &mut counts) {
                changed += 1;
                if samples.len() < wanted {
                    samples.push(sample(&before, &after));
                }
            }
        }
        ControlFlow::Continue(())
    })?;
    Ok(RedactionSummary {
        scanned,
        changed,
        truncated,
        hits: hits_of(&redactor, &counts),
        samples,
    })
}

/// The message, or the properties when only they changed.
pub(crate) fn sample(before: &Record, after: &Record) -> RedactionSample {
    let (b, a) = if before.message != after.message {
        (before.message.clone(), after.message.clone())
    } else if before.exception != after.exception {
        (
            before.exception.clone().unwrap_or_default(),
            after.exception.clone().unwrap_or_default(),
        )
    } else if before.template != after.template {
        (
            before.template.clone().unwrap_or_default(),
            after.template.clone().unwrap_or_default(),
        )
    } else {
        (
            Value::Object(before.fields.clone()).to_string(),
            Value::Object(after.fields.clone()).to_string(),
        )
    };
    RedactionSample {
        record_id: before.id,
        before: b,
        after: a,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::SourceLine;
    use crate::batch::{EventId, SourceId, TsState};
    use crate::model::Level;
    use serde_json::json;

    fn cfg(toml: &str) -> RedactCfg {
        crate::Profile::from_toml(&format!("name = 'p'\n[redact]\n{toml}")).unwrap().redact
    }

    fn redactor(toml: &str) -> Redactor {
        view_redactor(&cfg(toml)).unwrap()
    }

    fn event(message: &str) -> EventView {
        EventView {
            id: EventId(1),
            ts: None,
            ts_state: TsState::Parsed,
            level: Level::Information,
            message: message.into(),
            template: Some("t {U}".into()),
            exception: Some("at a@b.io".into()),
            fields: json!({"User": "x@y.io", "N": 3}).as_object().unwrap().clone(),
            source: SourceId(0),
            source_name: "s".into(),
            line: 1,
            byte_offset: 0,
            group: String::new(),
            markers: Vec::new(),
            search_match: None,
        }
    }

    #[test]
    fn the_view_needs_rules_and_says_what_to_add() {
        let e = view_redactor(&RedactCfg::default()).unwrap_err().to_string();
        assert!(e.contains("[redact]"), "{e}");
        let bad = cfg("[[redact.rule]]\nname = 'x'\nregex = '('");
        assert!(view_redactor(&bad).is_err());
    }

    #[test]
    fn events_and_pages_are_redacted_where_in_scope() {
        let r = redactor("detectors = ['email']");
        let mut page = PageReply {
            total: 2,
            offset: 0,
            rows: vec![event("mail a@b.io"), event("nothing")],
            anchor_index: None,
        };
        r.view_page(&mut page);
        assert_eq!(page.rows[0].message, "mail <email>");
        assert_eq!(page.rows[1].message, "nothing");
        // The default scope is the message and the exception: properties stay.
        assert_eq!(page.rows[0].exception.as_deref(), Some("at <email>"));
        assert_eq!(page.rows[0].fields["User"], "x@y.io");
        let all = redactor("detectors = ['email']\nfields = ['*']");
        let mut e = event("m");
        all.view_event(&mut e);
        assert_eq!(e.fields["User"], "<email>");
        assert_eq!(e.fields["N"], 3, "numbers are never touched");
        assert_eq!(e.template.as_deref(), Some("t {U}"));
    }

    #[test]
    fn facets_merge_values_that_redact_alike_and_out_of_scope_fields_stay() {
        let r = redactor("detectors = ['email']\nfields = ['User']");
        let v = |value: &str, count| FacetValue { value: value.into(), count };
        let mut f = Facet {
            field: "User".into(),
            values: vec![v("a@b.io", 5), v("bob", 4), v("c@d.io", 2)],
            distinct: 3,
            total: 11,
        };
        r.view_facet(&mut f);
        let got: Vec<(&str, usize)> = f.values.iter().map(|v| (v.value.as_str(), v.count)).collect();
        assert_eq!(got, vec![("<email>", 7), ("bob", 4)]);
        assert_eq!((f.distinct, f.total), (2, 11));
        let mut other = Facet {
            field: "Host".into(),
            values: vec![v("a@b.io", 1)],
            distinct: 1,
            total: 1,
        };
        r.view_facet(&mut other);
        assert_eq!(other.values[0].value, "a@b.io");
    }

    #[test]
    fn context_lines_tables_and_tiles_are_redacted() {
        let r = redactor("detectors = ['email', 'ipv4']\nfields = ['@message', 'User']");
        let mut c = SourceContext {
            source_name: "s".into(),
            line: 1,
            lines: vec![
                SourceLine {
                    line: 1,
                    text: "from 10.0.0.1".into(),
                },
                SourceLine { line: 2, text: "plain".into() },
            ],
            note: None,
        };
        r.view_context(&mut c);
        assert_eq!(c.lines[0].text, "from <ipv4>");
        assert_eq!(c.lines[1].text, "plain");
        let columns = vec!["User".to_string(), "Host".to_string(), "count()".to_string()];
        let mut ql = QlResult::Table {
            columns: columns.clone(),
            rows: vec![
                vec![json!("a@b.io"), json!("10.0.0.1"), json!(4)],
                vec![json!(["x@y.io"]), json!(null), json!(1)],
            ],
        };
        r.view_ql(&mut ql);
        let QlResult::Table { rows, .. } = &ql else { unreachable!() };
        assert_eq!(rows[0], vec![json!("<email>"), json!("10.0.0.1"), json!(4)]);
        assert_eq!(rows[1][0], json!(["<email>"]));
        let mut ids = QlResult::Events { ids: vec![1], fields: None };
        r.view_ql(&mut ids);
        assert!(matches!(ids, QlResult::Events { .. }));
        let mut table = TileData::Table {
            columns,
            rows: vec![vec![json!("a@b.io"), json!("h"), json!(1)]],
            truncated: false,
        };
        r.view_tile(&mut table);
        assert!(matches!(&table, TileData::Table { rows, .. } if rows[0][0] == "<email>"));
        let mut stat = TileData::Stat {
            value: None,
            text: "a@b.io".into(),
            label: "User".into(),
            unit: None,
            sparkline: Vec::new(),
        };
        r.view_tile(&mut stat);
        assert!(matches!(&stat, TileData::Stat { text, .. } if text == "<email>"));
        let mut top = TileData::Top {
            facet: Facet {
                field: "User".into(),
                values: vec![FacetValue {
                    value: "a@b.io".into(),
                    count: 1,
                }],
                distinct: 1,
                total: 1,
            },
        };
        r.view_tile(&mut top);
        assert!(matches!(&top, TileData::Top { facet } if facet.values[0].value == "<email>"));
        let mut alerts = TileData::Alerts { items: Vec::new() };
        r.view_tile(&mut alerts);
    }

    #[test]
    fn samples_show_what_changed() {
        let req: RedactionSummaryRequest = serde_json::from_value(json!({})).unwrap();
        assert_eq!(req.samples, 5);
        let mut before = Record::new(1);
        before.message = "m".into();
        before.exception = Some("x".into());
        before.template = Some("t".into());
        before.fields.insert("F".into(), "v".into());
        let changed = |f: &dyn Fn(&mut Record)| {
            let mut after = before.clone();
            f(&mut after);
            let s = sample(&before, &after);
            (s.before, s.after)
        };
        assert_eq!(changed(&|r| r.message = "M".into()), ("m".into(), "M".into()));
        assert_eq!(changed(&|r| r.exception = Some("X".into())), ("x".into(), "X".into()));
        assert_eq!(changed(&|r| r.template = Some("T".into())), ("t".into(), "T".into()));
        let (b, a) = changed(&|r| {
            r.fields.insert("F".into(), "<v>".into());
        });
        assert!(b.contains("\"v\"") && a.contains("<v>"));
    }
}
