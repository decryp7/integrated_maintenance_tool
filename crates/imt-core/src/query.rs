//! Query model shared by the grid, timeline, facets and charts.

use crate::condition::Condition;
use crate::instance::KeyScopes;
use crate::model::{Level, MarkerMask, Record};
use crate::ql::{Expr, TextSearch};
use crate::row::{FieldKey, FieldKeys, RecordRow, RowRef};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::sync::Arc;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Query {
    pub from: Option<i64>,
    pub to: Option<i64>,
    pub levels: Vec<Level>,
    pub text: String,
    pub regex: bool,
    pub case_sensitive: bool,
    /// How a plain search box text matches (smart search; a regex or case-sensitive search is always a
    /// substring search).
    #[serde(skip_serializing_if = "TextMode::is_substring")]
    pub text_mode: TextMode,
    pub filters: Vec<FieldFilter>,
    pub group_by: Vec<String>,
    pub groups: Vec<String>,
    pub markers: Vec<String>,
    /// A query-language expression (see [`crate::ql`]); its filter and any
    /// `where` stages before `stats` apply like the other criteria.
    pub ql: String,
    /// A sliding time window; the workspace resolves it into `from` before
    /// evaluating (live tail).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window: Option<TimeWindow>,
    /// Only records from these source ids (`SourceInfo::id`); empty = all
    /// sources. Unknown ids simply match nothing; at most [`MAX_QUERY_SOURCES`].
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<u32>,
}

/// How the search box text matches events.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextMode {
    /// The text anywhere in the message, exception or template, ignoring case.
    #[default]
    Substring,
    /// Every word of the text (see [`crate::words`]): answered from the full-text index.
    Words,
    /// Word matches and substring matches (after the background scan found partial matches).
    WordsOrSubstring,
}

impl TextMode {
    pub fn is_substring(&self) -> bool {
        *self == TextMode::Substring
    }
}

/// The query with its sliding window resolved into `from` for "now" = `now` (ms): the last `last_ms` before
/// it, narrowing a `from` the query already has. A query without a window is returned as it is.
pub fn resolve_live_window(q: &Query, now: i64) -> std::borrow::Cow<'_, Query> {
    match q.window {
        None => std::borrow::Cow::Borrowed(q),
        Some(TimeWindow::Live { last_ms }) => {
            let since = now.saturating_sub(last_ms.max(0));
            std::borrow::Cow::Owned(Query {
                from: Some(q.from.map_or(since, |f| f.max(since))),
                window: None,
                ..q.clone()
            })
        }
    }
}

/// Whether "now" for live windows is the system clock (`[live] clock = "wall"`) rather than the newest event.
pub fn live_clock_is_wall(profile: &crate::Profile) -> bool {
    profile.live.clock.eq_ignore_ascii_case("wall")
}

/// The system clock in ms since the epoch.
pub fn wall_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// Upper bound on [`Query::sources`], so a hostile request cannot make every
/// evaluation allocate an unbounded set.
pub const MAX_QUERY_SOURCES: usize = 10_000;

/// A time window relative to "now": the newest record (default) or the wall
/// clock (`[live] clock = "wall"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum TimeWindow {
    /// The last `last_ms` milliseconds, sliding as new records arrive.
    Live { last_ms: i64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldFilter {
    pub field: String,
    pub op: String,
    #[serde(default)]
    pub value: Option<String>,
}

pub const GROUP_SEPARATOR: &str = " · ";
const MISSING: &str = "–";

/// A query ready to evaluate: every criterion lowered into the one predicate language ([`Expr`]).
/// Split in two parts so the timeline can show instance lifetimes (structural) separately from what
/// matches (content).
pub struct CompiledQuery {
    from: Option<i64>,
    to: Option<i64>,
    pub group_by: Vec<String>,
    keys: FieldKeys,
    group_keys: Vec<FieldKey>,
    /// Where the identity fields of the instance key apply (only when the query uses the default key).
    scopes: Arc<KeyScopes>,
    /// Sources, time range, field filters and instance selection, in that order (all must hold).
    structural: Vec<Expr>,
    /// Levels, markers, the search box and the query language (all must hold).
    content: Vec<Expr>,
}

impl CompiledQuery {
    pub fn new(q: &Query, default_group_by: &[String], marker_kinds: &[String]) -> Result<CompiledQuery> {
        Self::new_scoped(q, default_group_by, &Arc::default(), marker_kinds)
    }

    /// [`new`](Self::new) with the scopes of the default instance key's identity fields
    /// ([`KeyScopes`]); a query that names its own `group_by` applies its fields everywhere.
    pub fn new_scoped(q: &Query, default_group_by: &[String], scopes: &Arc<KeyScopes>, marker_kinds: &[String]) -> Result<CompiledQuery> {
        if q.sources.len() > MAX_QUERY_SOURCES {
            anyhow::bail!(
                "the query restricts to {} sources, more than the maximum of {MAX_QUERY_SOURCES}; clear the source selection",
                q.sources.len()
            );
        }
        let mut keys = FieldKeys::new();
        let mut content = Vec::new();
        if !q.levels.is_empty() {
            content.push(Expr::Levels(q.levels.iter().fold(0u8, |m, l| m | (1 << l.index()))));
        }
        if !q.markers.is_empty() {
            let mut mask: MarkerMask = 0;
            for m in &q.markers {
                if let Some(i) = marker_kinds.iter().position(|k| k == m) {
                    mask |= 1 << i;
                }
            }
            // Asking only for marker kinds that do not exist matches nothing.
            content.push(if mask == 0 { Expr::Const(false) } else { Expr::Markers(mask) });
        }
        if !q.text.trim().is_empty() {
            let literal = (!q.regex).then(|| q.text.trim().to_string());
            let pattern = literal.as_deref().map_or_else(|| q.text.clone(), regex::escape);
            let re = crate::user_regex::compile(&pattern, !q.case_sensitive).context("invalid search regex")?;
            let words = (!q.regex && !q.case_sensitive && !q.text_mode.is_substring())
                .then(|| crate::words::WordQuery::parse(&q.text))
                .filter(|w| !w.is_empty())
                .map(Arc::new);
            content.push(Expr::Search(TextSearch {
                substring: words.is_none() || q.text_mode == TextMode::WordsOrSubstring,
                words,
                re,
                literal,
                case_sensitive: q.case_sensitive,
            }));
        }
        if !q.ql.trim().is_empty() {
            let program = crate::ql::parse(&q.ql).map_err(|e| anyhow::anyhow!("query: {e}"))?;
            // Stages after sort/head/fields/stats shape the output, not which records match.
            let wheres = program.stages.iter().map_while(|s| match s {
                crate::ql::Stage::Where(f) => Some(f.clone()),
                _ => None,
            });
            content.extend(std::iter::once(program.filter.clone()).chain(wheres).filter_map(|f| f.expr));
        }
        let mut structural = Vec::new();
        if !q.sources.is_empty() {
            structural.push(Expr::Sources(Arc::new(q.sources.iter().copied().collect())));
        }
        if q.from.is_some() || q.to.is_some() {
            structural.push(Expr::Time { from: q.from, to: q.to });
        }
        for f in &q.filters {
            structural.push(Condition::new(&f.field, &f.op, f.value.as_deref())?.check().lower(&mut keys));
        }
        let (group_by, scopes) = if q.group_by.is_empty() {
            (default_group_by.to_vec(), scopes.clone())
        } else {
            (q.group_by.clone(), Arc::default())
        };
        let group_keys: Vec<FieldKey> = group_by.iter().map(|f| keys.key(f)).collect();
        if !q.groups.is_empty() {
            structural.push(Expr::Group {
                by: group_keys.clone(),
                set: Arc::new(q.groups.iter().cloned().collect()),
                scopes: scopes.clone(),
            });
        }
        Ok(CompiledQuery {
            from: q.from,
            to: q.to,
            group_by,
            keys,
            group_keys,
            scopes,
            structural,
            content,
        })
    }

    /// Where the identity fields of the instance key apply.
    pub fn key_scopes(&self) -> &KeyScopes {
        &self.scopes
    }

    pub fn has_range(&self) -> bool {
        self.from.is_some() || self.to.is_some()
    }

    pub fn in_range(&self, rec: &Record) -> bool {
        in_range(self.from, self.to, rec.ts)
    }

    /// The structural criteria (sources, time range, field filters, instance selection), all of which must hold.
    pub fn structural_exprs(&self) -> &[Expr] {
        &self.structural
    }

    /// The content criteria (levels, markers, search box, query language), all of which must hold.
    pub fn content_exprs(&self) -> &[Expr] {
        &self.content
    }

    /// The names the criteria refer to (resolve [`crate::row::BatchSlots`] with these).
    pub fn keys(&self) -> &FieldKeys {
        &self.keys
    }

    /// The instance key fields, interned.
    pub fn group_keys(&self) -> &[FieldKey] {
        &self.group_keys
    }

    /// Source selection, time range, field filters and instance selection.
    pub fn structural(&self, rec: &Record, source_name: &str, key: Option<&str>) -> bool {
        self.structural_row(&RecordRow { rec, source: source_name }, key)
    }

    /// [`structural`](Self::structural) of any event; `key` is its instance key when already known.
    pub fn structural_row(&self, row: &dyn RowRef, key: Option<&str>) -> bool {
        self.structural.iter().all(|e| match (e, key) {
            (Expr::Group { set, .. }, Some(k)) => set.contains(k),
            _ => e.matches(row),
        })
    }

    /// Levels, text search, marker kinds and the query language.
    pub fn content(&self, rec: &Record, source_name: &str) -> bool {
        self.content_row(&RecordRow { rec, source: source_name })
    }

    pub fn content_row(&self, row: &dyn RowRef) -> bool {
        self.content.iter().all(|e| e.matches(row))
    }

    pub fn matches(&self, rec: &Record, source_name: &str) -> bool {
        self.matches_row(&RecordRow { rec, source: source_name })
    }

    pub fn matches_row(&self, row: &dyn RowRef) -> bool {
        self.content_row(row) && self.structural_row(row, None)
    }

    pub fn group_key(&self, rec: &Record, source_name: &str) -> String {
        group_key_row_in(&self.group_keys, &self.scopes, &RecordRow { rec, source: source_name })
    }

    pub fn group_key_of(&self, row: &dyn RowRef) -> String {
        group_key_row_in(&self.group_keys, &self.scopes, row)
    }
}

fn in_range(from: Option<i64>, to: Option<i64>, ts: Option<i64>) -> bool {
    match ts {
        Some(ts) => from.is_none_or(|f| ts >= f) && to.is_none_or(|t| ts <= t),
        None => from.is_none() && to.is_none(),
    }
}

/// The instance key: the values of `fields` from the first present one to the last present one, joined
/// with " · " ("–" for a missing one in between). When the first field is missing, the source name comes
/// first, so keys built from different fields (a key chosen for several products lists one product's fields
/// after another's, ADR 0020) never collide. Records that carry none of the fields (e.g. a log4net file
/// among Serilog logs) are keyed by their source instead of collapsing into one anonymous instance.
pub fn group_key(fields: &[String], rec: &Record, source_name: &str) -> String {
    group_key_with(fields.len(), |i| rec.field_text(&fields[i], source_name), source_name)
}

/// [`group_key`] where an identity field counts only for the sources it applies to ([`KeyScopes`]).
pub fn group_key_in(fields: &[String], scopes: &KeyScopes, rec: &Record, source_name: &str) -> String {
    group_key_with(
        fields.len(),
        |i| {
            scopes
                .applies(&fields[i], source_name)
                .then(|| rec.field_text(&fields[i], source_name))
                .flatten()
        },
        source_name,
    )
}

/// [`group_key`] of any event, with interned fields.
pub fn group_key_row(fields: &[FieldKey], row: &dyn RowRef) -> String {
    group_key_with(fields.len(), |i| row.text(&fields[i]), row.source_name())
}

/// [`group_key_in`] of any event, with interned fields.
pub fn group_key_row_in(fields: &[FieldKey], scopes: &KeyScopes, row: &dyn RowRef) -> String {
    let source = row.source_name();
    group_key_with(
        fields.len(),
        |i| scopes.applies(fields[i].name(), source).then(|| row.text(&fields[i])).flatten(),
        source,
    )
}

fn group_key_with<'a>(n: usize, text: impl Fn(usize) -> Option<Cow<'a, str>>, source_name: &str) -> String {
    if n == 0 {
        return "all".into();
    }
    let values: Vec<Option<Cow<'a, str>>> = (0..n).map(|i| text(i).filter(|v| !v.is_empty())).collect();
    let (Some(first), Some(last)) = (values.iter().position(Option::is_some), values.iter().rposition(Option::is_some)) else {
        return source_name.to_string();
    };
    let mut key = String::new();
    if first > 0 {
        key.push_str(source_name);
        key.push_str(GROUP_SEPARATOR);
    }
    for (i, v) in values[first..=last].iter().enumerate() {
        if i > 0 {
            key.push_str(GROUP_SEPARATOR);
        }
        key.push_str(v.as_deref().unwrap_or(MISSING));
    }
    key
}

/// The instance key when the query names none, from field totals over all events (see
/// [`crate::instance::instance_key_from_totals`]; the workspace uses per-source counts,
/// [`crate::instance::instance_key`]).
pub fn default_group_by(cfg: &crate::profile::InstanceCfg, fields: &[crate::api::FieldInfo], total: usize, sources: usize) -> Vec<String> {
    crate::instance::instance_key_from_totals(cfg, fields, total, sources)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_compiled_query_exposes_its_structural_and_content_criteria_and_instance_key() {
        let q = Query {
            from: Some(10),
            to: Some(20),
            levels: vec![crate::model::Level::Error],
            text: "boom".into(),
            sources: vec![3],
            ..Default::default()
        };
        let c = CompiledQuery::new(&q, &["Host".into()], &[]).unwrap();
        // The time range and the source selection are structural; levels and text are content.
        assert!(c.structural_exprs().len() >= 2, "{}", c.structural_exprs().len());
        assert!(c.content_exprs().len() >= 2, "{}", c.content_exprs().len());
        assert_eq!(c.group_keys().len(), 1);
        assert_eq!(c.group_keys()[0].name(), "Host");
        assert!(!c.keys().names().is_empty());
        let mut r = Record::new(1);
        r.ts = Some(15);
        assert!(c.in_range(&r));
        r.ts = Some(25);
        assert!(!c.in_range(&r), "after the end");
        r.ts = None;
        assert!(!c.in_range(&r), "an event without a time is outside any bound");
        // Instance keys read through rows: the field value, or the source name when the field is missing.
        r.fields.insert("Host".into(), "WEB01".into());
        let row = crate::row::RecordRow {
            rec: &r,
            source: "app.log",
        };
        assert_eq!(c.group_key_of(&row), "WEB01");
        assert_eq!(group_key_row(c.group_keys(), &row), "WEB01");
        r.fields.clear();
        let row = crate::row::RecordRow {
            rec: &r,
            source: "app.log",
        };
        assert_eq!(group_key_row(c.group_keys(), &row), "app.log");
        // No key fields at all: one lane for everything.
        assert_eq!(group_key_row(&[], &row), "all");
    }

    #[test]
    fn unknown_marker_kind_matches_nothing() {
        let q = Query {
            markers: vec!["nope".into()],
            ..Default::default()
        };
        let c = CompiledQuery::new(&q, &[], &["start".into()]).unwrap();
        assert!(!c.content(&Record::new(1), "s"));
    }

    #[test]
    fn marker_filter() {
        let q = Query {
            markers: vec!["start".into()],
            ..Default::default()
        };
        let c = CompiledQuery::new(&q, &[], &["stop".into(), "start".into()]).unwrap();
        let mut r = Record::new(1);
        assert!(!c.content(&r, "s"));
        r.markers = 0b10;
        assert!(c.content(&r, "s"));
    }

    fn rec(ts: i64, msg: &str) -> Record {
        let mut r = Record::new(1);
        r.ts = Some(ts);
        r.message = msg.into();
        r
    }

    #[test]
    fn text_search_modes() {
        let mut r = rec(10, "Order failed");
        r.exception = Some("System.TimeoutException".into());
        r.template = Some("Order {Id} failed".into());
        let c = |q: Query| CompiledQuery::new(&q, &[], &[]).unwrap().content(&r, "s");
        assert!(c(Query {
            text: "ORDER".into(),
            ..Default::default()
        }));
        assert!(!c(Query {
            text: "ORDER".into(),
            case_sensitive: true,
            ..Default::default()
        }));
        assert!(c(Query {
            text: "timeoutexc".into(),
            ..Default::default()
        }));
        assert!(c(Query {
            text: r"\{Id\}".into(),
            regex: true,
            ..Default::default()
        }));
        assert!(!c(Query {
            text: "nothing".into(),
            ..Default::default()
        }));
        assert!(CompiledQuery::new(
            &Query {
                text: "(".into(),
                regex: true,
                ..Default::default()
            },
            &[],
            &[]
        )
        .is_err());
        assert!(CompiledQuery::new(
            &Query {
                filters: vec![FieldFilter {
                    field: "a".into(),
                    op: "bad".into(),
                    value: None
                }],
                ..Default::default()
            },
            &[],
            &[]
        )
        .is_err());
    }

    #[test]
    fn live_windows_resolve_into_from() {
        let q = Query {
            window: Some(TimeWindow::Live { last_ms: 1_000 }),
            ..Default::default()
        };
        let r = resolve_live_window(&q, 10_000);
        assert_eq!((r.from, r.window), (Some(9_000), None));
        let narrower = Query {
            from: Some(9_500),
            ..q.clone()
        };
        assert_eq!(
            resolve_live_window(&narrower, 10_000).from,
            Some(9_500),
            "an existing from that is later stays"
        );
        let negative = Query {
            window: Some(TimeWindow::Live { last_ms: -5 }),
            ..Default::default()
        };
        assert_eq!(resolve_live_window(&negative, 10_000).from, Some(10_000));
        assert!(matches!(resolve_live_window(&Query::default(), 1), std::borrow::Cow::Borrowed(_)));
        assert!(wall_ms() > 1_700_000_000_000);
        let mut p = crate::Profile::default_profile();
        assert!(!live_clock_is_wall(&p));
        p.live.clock = "Wall".into();
        assert!(live_clock_is_wall(&p));
    }

    #[test]
    fn smart_search_matches_words_then_substrings() {
        let mut r = rec(10, "Order failed -- retry");
        r.exception = Some("System.TimeoutException".into());
        let c = |text: &str, text_mode: TextMode| {
            let q = Query {
                text: text.into(),
                text_mode,
                ..Default::default()
            };
            CompiledQuery::new(&q, &[], &[]).unwrap().content(&r, "s")
        };
        assert!(c("timeout ord", TextMode::Words), "words in any order, the last a prefix");
        assert!(!c("imeoutExc", TextMode::Words), "a partial match is not a word");
        assert!(c("imeoutExc", TextMode::WordsOrSubstring), "until partial matches are added");
        assert!(!c("timeout ord", TextMode::Substring));
        assert!(
            c("--", TextMode::Words) && !c("++", TextMode::Words),
            "no words: a substring search"
        );
        // A regex or case-sensitive search is always a substring search.
        let q = Query {
            text: "Order f".into(),
            case_sensitive: true,
            text_mode: TextMode::Words,
            ..Default::default()
        };
        assert!(CompiledQuery::new(&q, &[], &[]).unwrap().content(&r, "s"));
        assert_eq!(serde_json::to_value(Query::default()).unwrap().get("textMode"), None);
        let q: Query = serde_json::from_str(r#"{"text": "x", "textMode": "wordsOrSubstring"}"#).unwrap();
        assert_eq!(q.text_mode, TextMode::WordsOrSubstring);
    }

    #[test]
    fn levels_range_and_groups() {
        let mut r = rec(10, "x");
        r.level = Level::Warning;
        r.fields.insert("M".into(), "WEB01".into());
        let warn = CompiledQuery::new(
            &Query {
                levels: vec![Level::Warning],
                ..Default::default()
            },
            &[],
            &[],
        )
        .unwrap();
        assert!(warn.matches(&r, "s"));
        let err = CompiledQuery::new(
            &Query {
                levels: vec![Level::Error],
                ..Default::default()
            },
            &[],
            &[],
        )
        .unwrap();
        assert!(!err.content(&r, "s"));

        let range = CompiledQuery::new(
            &Query {
                from: Some(5),
                to: Some(9),
                ..Default::default()
            },
            &[],
            &[],
        )
        .unwrap();
        assert!(range.has_range());
        assert!(!range.in_range(&r));
        assert!(!range.in_range(&Record::new(1)));
        assert!(CompiledQuery::new(&Query::default(), &[], &[]).unwrap().in_range(&Record::new(1)));

        let groups = Query {
            group_by: vec!["M".into(), "Missing".into()],
            groups: vec!["WEB01".into()],
            ..Default::default()
        };
        let g = CompiledQuery::new(&groups, &["@source".into()], &[]).unwrap();
        assert_eq!(g.group_key(&r, "s"), "WEB01", "missing values at the end are left out");
        assert!(g.structural(&r, "s", None));
        assert!(g.structural(&r, "s", Some("WEB01")));
        let mut both = r.clone();
        both.fields.insert("N".into(), "7".into());
        let fields = ["Missing".into(), "M".into(), "Missing".into(), "N".into(), "Missing".into()];
        assert_eq!(
            group_key(&fields, &both, "s"),
            "s · WEB01 · – · 7",
            "the source first when the first field is missing"
        );
        assert!(!g.structural(&r, "s", Some("other")));
        // A scoped identity field is part of the key only in the sources that declare it, and only for the default key.
        let declares = vec!["M".to_string()];
        let scopes = Arc::new(KeyScopes::new(
            &crate::profile::InstanceCfg {
                key: Vec::new(),
                candidates: Vec::new(),
            },
            [("a.log", declares.as_slice()), ("b.log", &[][..])],
        ));
        assert_eq!(group_key_in(&["M".into()], &scopes, &r, "a.log"), "WEB01");
        assert_eq!(group_key_in(&["M".into()], &scopes, &r, "b.log"), "b.log");
        let pick = Query {
            groups: vec!["b.log".into()],
            ..Default::default()
        };
        let scoped = CompiledQuery::new_scoped(&pick, &["M".into()], &scopes, &[]).unwrap();
        assert_eq!(
            (scoped.group_key(&r, "a.log"), scoped.group_key(&r, "b.log")),
            ("WEB01".into(), "b.log".into())
        );
        assert!(
            scoped.matches(&r, "b.log") && !scoped.matches(&r, "a.log"),
            "the instance selection uses the scopes"
        );
        let own = Query {
            group_by: vec!["M".into()],
            ..Default::default()
        };
        let own = CompiledQuery::new_scoped(&own, &["M".into()], &scopes, &[]).unwrap();
        assert_eq!(own.group_key(&r, "b.log"), "WEB01", "a key the query names applies everywhere");
        assert!(own.key_scopes().is_empty());
        let d = CompiledQuery::new(&Query::default(), &["@source".into()], &[]).unwrap();
        assert_eq!(d.group_key(&r, "file.log"), "file.log");
        assert_eq!(group_key(&[], &r, "s"), "all");
        assert_eq!(
            group_key(&["Nope".into(), "Missing".into()], &r, "legacy/billing.log"),
            "legacy/billing.log"
        );

        let filtered = Query {
            filters: vec![FieldFilter {
                field: "M".into(),
                op: "eq".into(),
                value: Some("WEB02".into()),
            }],
            ..Default::default()
        };
        assert!(!CompiledQuery::new(&filtered, &[], &[]).unwrap().structural(&r, "s", None));
    }

    #[test]
    fn ql_filters_apply_up_to_the_first_non_where_stage() {
        let mut r = rec(10, "slow call");
        r.level = Level::Warning;
        r.fields.insert("ms".into(), 500.into());
        let matches = |ql: &str| {
            let q = Query {
                ql: ql.into(),
                ..Default::default()
            };
            CompiledQuery::new(&q, &[], &[]).unwrap().content(&r, "s")
        };
        assert!(matches("level >= warning | where ms > 100"));
        assert!(!matches("level >= warning | where ms > 1000"));
        // Stages after sort/head/fields/stats shape the output, not which records match.
        assert!(matches("| where ms > 100 | sort ms | where ms > 1000"));
        assert!(matches("| head 5 | where ms > 1000"));
        assert!(matches("| fields ms | where ms > 1000"));
        assert!(matches("| where ms > 100 | stats count() | where ms > 1000"));
        assert!(!matches("| where ms > 1000 | stats count()"));
        let bad = Query {
            ql: "level >=".into(),
            ..Default::default()
        };
        assert!(CompiledQuery::new(&bad, &[], &[]).err().unwrap().to_string().starts_with("query: "));
    }

    #[test]
    fn source_selection_is_structural_and_bounded() {
        let mut r = rec(10, "x");
        r.source = 3;
        let only = |sources: Vec<u32>| {
            CompiledQuery::new(
                &Query {
                    sources,
                    ..Default::default()
                },
                &[],
                &[],
            )
            .unwrap()
        };
        assert!(only(vec![]).matches(&r, "s"));
        assert!(only(vec![1, 3]).structural(&r, "s", None));
        assert!(only(vec![1, 3]).matches(&r, "s"));
        assert!(!only(vec![1]).structural(&r, "s", Some("any")));
        // Unknown ids match nothing rather than failing.
        assert!(!only(vec![999]).matches(&r, "s"));
        let too_many = Query {
            sources: (0..=MAX_QUERY_SOURCES as u32).collect(),
            ..Default::default()
        };
        let err = CompiledQuery::new(&too_many, &[], &[]).err().unwrap().to_string();
        assert!(err.contains("maximum of 10000"), "{err}");
        // Absent and empty are the same, and empty is not serialized.
        let parsed: Query = serde_json::from_str(r#"{"sources":[2]}"#).unwrap();
        assert_eq!(parsed.sources, vec![2]);
        assert!(!serde_json::to_string(&Query::default()).unwrap().contains("sources"));
    }
}
