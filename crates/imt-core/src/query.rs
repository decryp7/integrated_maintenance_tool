//! Query model shared by the grid, timeline, facets and charts.

use crate::condition::Condition;
use crate::model::{Level, MarkerMask, Record};
use crate::ql::{Expr, TextSearch};
use crate::row::{FieldKey, FieldKeys, RecordRow, RowRef};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Sources, time range, field filters and instance selection, in that order (all must hold).
    structural: Vec<Expr>,
    /// Levels, markers, the search box and the query language (all must hold).
    content: Vec<Expr>,
}

impl CompiledQuery {
    pub fn new(q: &Query, default_group_by: &[String], marker_kinds: &[String]) -> Result<CompiledQuery> {
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
            content.push(Expr::Search(TextSearch {
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
        let group_by = if q.group_by.is_empty() {
            default_group_by.to_vec()
        } else {
            q.group_by.clone()
        };
        let group_keys: Vec<FieldKey> = group_by.iter().map(|f| keys.key(f)).collect();
        if !q.groups.is_empty() {
            structural.push(Expr::Group {
                by: group_keys.clone(),
                set: Arc::new(q.groups.iter().cloned().collect()),
            });
        }
        Ok(CompiledQuery {
            from: q.from,
            to: q.to,
            group_by,
            keys,
            group_keys,
            structural,
            content,
        })
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
        group_key_row(&self.group_keys, &RecordRow { rec, source: source_name })
    }

    pub fn group_key_of(&self, row: &dyn RowRef) -> String {
        group_key_row(&self.group_keys, row)
    }
}

fn in_range(from: Option<i64>, to: Option<i64>, ts: Option<i64>) -> bool {
    match ts {
        Some(ts) => from.is_none_or(|f| ts >= f) && to.is_none_or(|t| ts <= t),
        None => from.is_none() && to.is_none(),
    }
}

/// The instance key: the values of `fields` joined with " · ". Records that
/// carry none of the fields (e.g. a log4net file among Serilog logs) are keyed
/// by their source file instead of collapsing into one anonymous instance.
pub fn group_key(fields: &[String], rec: &Record, source_name: &str) -> String {
    group_key_with(fields.len(), |i| rec.field_text(&fields[i], source_name), source_name)
}

/// [`group_key`] of any event, with interned fields.
pub fn group_key_row(fields: &[FieldKey], row: &dyn RowRef) -> String {
    group_key_with(fields.len(), |i| row.text(&fields[i]), row.source_name())
}

fn group_key_with<'a>(n: usize, text: impl Fn(usize) -> Option<std::borrow::Cow<'a, str>>, source_name: &str) -> String {
    if n == 0 {
        return "all".into();
    }
    let mut key = String::new();
    let mut found = false;
    for i in 0..n {
        if i > 0 {
            key.push_str(GROUP_SEPARATOR);
        }
        match text(i) {
            Some(v) if !v.is_empty() => {
                found = true;
                key.push_str(&v);
            }
            _ => key.push_str(MISSING),
        }
    }
    if found {
        key
    } else {
        source_name.to_string()
    }
}

/// The instance key when the query names none: the profile's `[instance] key`, else the candidate
/// fields present on at least half of the `total` events that vary, else (with one source) the first
/// present candidate, else `@source`.
pub fn default_group_by(cfg: &crate::profile::InstanceCfg, fields: &[crate::api::FieldInfo], total: usize, sources: usize) -> Vec<String> {
    if !cfg.key.is_empty() {
        return cfg.key.clone();
    }
    let total = total.max(1);
    let present: Vec<&crate::api::FieldInfo> = cfg
        .candidates
        .iter()
        .filter_map(|c| fields.iter().find(|f| &f.name == c))
        .filter(|f| f.count * 2 >= total)
        .collect();
    // Constant fields (same value everywhere) do not tell instances apart.
    let varying: Vec<String> = present.iter().filter(|f| f.distinct > 1).map(|f| f.name.clone()).collect();
    if !varying.is_empty() {
        return varying;
    }
    if sources <= 1 {
        if let Some(f) = present.first() {
            return vec![f.name.clone()];
        }
    }
    vec!["@source".into()]
}

#[cfg(test)]
mod tests {
    use super::*;

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
            groups: vec!["WEB01 · –".into()],
            ..Default::default()
        };
        let g = CompiledQuery::new(&groups, &["@source".into()], &[]).unwrap();
        assert_eq!(g.group_key(&r, "s"), "WEB01 · –");
        assert!(g.structural(&r, "s", None));
        assert!(g.structural(&r, "s", Some("WEB01 · –")));
        assert!(!g.structural(&r, "s", Some("other")));
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
