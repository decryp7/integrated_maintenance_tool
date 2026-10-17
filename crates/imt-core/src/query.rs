//! Query model shared by the grid, timeline, facets and charts.

use crate::condition::Condition;
use crate::model::{Level, Record};
use anyhow::{Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

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
}

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

/// A query ready to evaluate. Split in two parts so the timeline can show
/// instance lifetimes (structural) separately from what matches (content).
pub struct CompiledQuery {
    from: Option<i64>,
    to: Option<i64>,
    level_mask: u8,
    text: Option<Regex>,
    filters: Vec<Condition>,
    pub group_by: Vec<String>,
    groups: Option<HashSet<String>>,
    marker_mask: crate::model::MarkerMask,
    match_nothing: bool,
    ql: Vec<crate::ql::Filter>,
}

impl CompiledQuery {
    pub fn new(q: &Query, default_group_by: &[String], marker_kinds: &[String]) -> Result<CompiledQuery> {
        let text = if q.text.trim().is_empty() {
            None
        } else {
            let pattern = if q.regex { q.text.clone() } else { regex::escape(q.text.trim()) };
            Some(crate::user_regex::compile(&pattern, !q.case_sensitive).context("invalid search regex")?)
        };
        let filters = q
            .filters
            .iter()
            .map(|f| Condition::new(&f.field, &f.op, f.value.as_deref()))
            .collect::<Result<Vec<_>>>()?;
        let mut marker_mask: crate::model::MarkerMask = 0;
        for m in &q.markers {
            if let Some(i) = marker_kinds.iter().position(|k| k == m) {
                marker_mask |= 1 << i;
            }
        }
        let ql = if q.ql.trim().is_empty() {
            Vec::new()
        } else {
            let program = crate::ql::parse(&q.ql).map_err(|e| anyhow::anyhow!("query: {e}"))?;
            let wheres = program.stages.iter().map_while(|s| match s {
                crate::ql::Stage::Where(f) => Some(f.clone()),
                crate::ql::Stage::Sort(_) | crate::ql::Stage::Head(_) | crate::ql::Stage::Fields(_) => None,
                crate::ql::Stage::Stats { .. } => None,
            });
            std::iter::once(program.filter).chain(wheres).collect()
        };
        Ok(CompiledQuery {
            ql,
            from: q.from,
            to: q.to,
            level_mask: q.levels.iter().fold(0u8, |m, l| m | (1 << l.index())),
            text,
            filters,
            group_by: if q.group_by.is_empty() {
                default_group_by.to_vec()
            } else {
                q.group_by.clone()
            },
            groups: (!q.groups.is_empty()).then(|| q.groups.iter().cloned().collect()),
            marker_mask,
            // Asking only for marker kinds that do not exist matches nothing.
            match_nothing: !q.markers.is_empty() && marker_mask == 0,
        })
    }

    pub fn has_range(&self) -> bool {
        self.from.is_some() || self.to.is_some()
    }

    pub fn in_range(&self, rec: &Record) -> bool {
        match rec.ts {
            Some(ts) => self.from.is_none_or(|f| ts >= f) && self.to.is_none_or(|t| ts <= t),
            None => !self.has_range(),
        }
    }

    /// Time range, field filters and instance selection.
    pub fn structural(&self, rec: &Record, source_name: &str, key: Option<&str>) -> bool {
        if !self.in_range(rec) || !self.filters.iter().all(|f| f.eval(rec, source_name)) {
            return false;
        }
        match (&self.groups, key) {
            (Some(groups), Some(k)) => groups.contains(k),
            (Some(groups), None) => groups.contains(&self.group_key(rec, source_name)),
            (None, _) => true,
        }
    }

    /// Levels, text search and marker kinds.
    pub fn content(&self, rec: &Record, source_name: &str) -> bool {
        if self.match_nothing {
            return false;
        }
        if self.level_mask != 0 && self.level_mask & (1 << rec.level.index()) == 0 {
            return false;
        }
        if self.marker_mask != 0 && rec.markers & self.marker_mask == 0 {
            return false;
        }
        if let Some(re) = &self.text {
            let hit = re.is_match(&rec.message)
                || rec.exception.as_deref().is_some_and(|x| re.is_match(x))
                || rec.template.as_deref().is_some_and(|t| re.is_match(t));
            if !hit {
                return false;
            }
        }
        self.ql.iter().all(|f| f.matches(rec, source_name))
    }

    pub fn matches(&self, rec: &Record, source_name: &str) -> bool {
        self.content(rec, source_name) && self.structural(rec, source_name, None)
    }

    pub fn group_key(&self, rec: &Record, source_name: &str) -> String {
        group_key(&self.group_by, rec, source_name)
    }
}

/// The instance key: the values of `fields` joined with " · ". Records that
/// carry none of the fields (e.g. a log4net file among Serilog logs) are keyed
/// by their source file instead of collapsing into one anonymous instance.
pub fn group_key(fields: &[String], rec: &Record, source_name: &str) -> String {
    if fields.is_empty() {
        return "all".into();
    }
    let mut key = String::new();
    let mut found = false;
    for (i, f) in fields.iter().enumerate() {
        if i > 0 {
            key.push_str(GROUP_SEPARATOR);
        }
        match rec.field_text(f, source_name) {
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
}
