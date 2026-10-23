//! Error outliers (ADR 0045): fields and values that are over-represented in a subset of the events (the errors by
//! default, or any selection) compared with the rest of the same window, ranked by lift and significance.
//!
//! The statistics are pure ([`lift`], [`g_statistic`], [`p_value`]); [`error_outliers`] drives them through the
//! [`Query`] port only (facets and counts), so every store gives the same answer from its own aggregates. Everything
//! is bounded: fields checked, values per field, outliers returned and the evidence per outlier.

use crate::api::FacetRequest;
use crate::ports::{Analyzed, Coverage, PageQuery, Query as QueryPort, StoreError, StoreResult};
use crate::query::Query;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The subset when the request names none: error and fatal events.
pub const DEFAULT_SUBSET: &str = "@level >= error";
/// Fields examined at most (the most populated first).
pub const MAX_FIELDS: usize = 60;
/// Values taken from the subset's facet per field.
pub const SUBSET_VALUES: usize = 100;
/// Values of the whole window's facet per field (the rest of the window is derived from it).
pub const WINDOW_VALUES: usize = 1000;
/// Outliers returned at most.
pub const MAX_OUTLIERS: usize = 200;
/// Outliers returned when the request asks for none.
pub const DEFAULT_OUTLIERS: usize = 30;
/// Events of the subset that must carry the value (smaller counts prove nothing).
pub const MIN_SUPPORT: usize = 3;
/// The smallest lift reported.
pub const MIN_LIFT: f64 = 1.5;
/// The G statistic of p = 0.001 (one degree of freedom): weaker associations are noise.
pub const MIN_SIGNIFICANCE: f64 = 10.83;
/// Evidence events listed per outlier.
pub const EVIDENCE_EVENTS: usize = 5;
/// Fields with more distinct values than this (identifiers) are not examined.
const MAX_DISTINCT: usize = 5000;
/// Longest value text kept.
const MAX_VALUE_CHARS: usize = 200;

/// What to compare.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OutlierRequest {
    /// The window: the events compared (a filter without `|` stages in its query-language text).
    pub query: Query,
    /// The selected subset as a query-language filter ([`DEFAULT_SUBSET`] when empty).
    pub subset: String,
    /// Examine only these fields (empty: every field with a handful of values).
    pub fields: Vec<String>,
    /// Outliers returned (1 to [`MAX_OUTLIERS`]; [`DEFAULT_OUTLIERS`] when 0).
    pub limit: usize,
}

/// A value over-represented in the subset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Outlier {
    pub field: String,
    pub value: String,
    /// Subset events with the value, out of [`OutlierReport::subset_total`].
    pub subset_count: usize,
    /// The rest of the window's events with the value, out of [`OutlierReport::rest_total`].
    pub rest_count: usize,
    /// How many times more likely the value is in the subset than in the rest (smoothed, so never infinite).
    pub lift: f64,
    /// The G statistic of the 2x2 table (larger is stronger).
    pub significance: f64,
    /// Approximate p-value of `significance` (one degree of freedom).
    pub p_value: f64,
    /// The filter that selects the subset events with this value (the UI opens it as the view).
    pub query: String,
    /// Up to [`EVIDENCE_EVENTS`] events of the subset with the value.
    pub sample_ids: Vec<u32>,
}

/// The outliers of a window.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutlierReport {
    /// The subset used, as a filter.
    pub subset: String,
    pub subset_total: usize,
    pub rest_total: usize,
    pub fields_checked: usize,
    pub outliers: Vec<Outlier>,
}

/// How much more likely the value is in the subset than in the rest: `(a / n_s) / (c / n_r)` with half an event added
/// to each count so that a value absent from the rest has a large but finite lift.
pub fn lift(a: usize, subset_total: usize, c: usize, rest_total: usize) -> f64 {
    let p_subset = (a as f64 + 0.5) / (subset_total as f64 + 1.0);
    let p_rest = (c as f64 + 0.5) / (rest_total as f64 + 1.0);
    p_subset / p_rest
}

/// The G statistic (log-likelihood ratio) of the 2x2 table: subset/rest by with/without the value.
pub fn g_statistic(a: usize, subset_total: usize, c: usize, rest_total: usize) -> f64 {
    let (a, c) = (a as f64, c as f64);
    let (n_s, n_r) = (subset_total as f64, rest_total as f64);
    let cells = [
        (a, n_s, a + c),
        (n_s - a, n_s, n_s + n_r - a - c),
        (c, n_r, a + c),
        (n_r - c, n_r, n_s + n_r - a - c),
    ];
    let n = n_s + n_r;
    let mut g = 0.0;
    for (observed, row, column) in cells {
        if observed > 0.0 && row > 0.0 && column > 0.0 {
            let expected = row * column / n;
            g += observed * (observed / expected).ln();
        }
    }
    (2.0 * g).max(0.0)
}

/// `erfc` (Numerical Recipes' Chebyshev fit, relative error below 1.2e-7 everywhere).
fn erfc(x: f64) -> f64 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.5 * z);
    let r = t
        * (-z * z - 1.265_512_23
            + t * (1.000_023_68
                + t * (0.374_091_96
                    + t * (0.096_784_18
                        + t * (-0.186_288_06
                            + t * (0.278_868_07 + t * (-1.135_203_98 + t * (1.488_515_87 + t * (-0.822_152_23 + t * 0.170_872_77)))))))))
            .exp();
    if x >= 0.0 {
        r
    } else {
        2.0 - r
    }
}

/// The p-value of a G statistic with one degree of freedom.
pub fn p_value(g: f64) -> f64 {
    if g <= 0.0 {
        1.0
    } else {
        erfc((g / 2.0).sqrt()).clamp(1e-300, 1.0)
    }
}

/// `"` and `\` escaped for a query-language string.
fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            '\n' | '\r' | '\t' => out.push(' '),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Whether the query language can address the field by its bare name.
fn addressable(field: &str) -> bool {
    let mut chars = field.chars();
    let first = chars.next();
    first.is_some_and(|c| c == '@' || c == '_' || c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// `base` narrowed by a filter (a `where` stage after the base program).
fn narrowed(base: &Query, filter: &str) -> Query {
    let mut q = base.clone();
    let existing = q.ql.trim().to_string();
    q.ql = if existing.is_empty() {
        filter.to_string()
    } else {
        format!("{existing} | where {filter}")
    };
    q
}

fn count_of(store: &dyn QueryPort, q: &Query) -> StoreResult<usize> {
    let page = store.page(&PageQuery {
        query: q.clone(),
        offset: 0,
        limit: 1,
        descending: false,
        anchor: None,
    })?;
    Ok(page.total)
}

fn cancelled() -> StoreResult<()> {
    match crate::ql::QueryControl::current() {
        Some(c) if c.is_cancelled() => Err(StoreError::Cancelled),
        _ => Ok(()),
    }
}

/// Finds the values over-represented in `req.subset` against the rest of `req.query`'s window.
pub fn error_outliers(store: &dyn QueryPort, req: &OutlierRequest) -> StoreResult<Analyzed<OutlierReport>> {
    if req.query.ql.contains('|') {
        return Err(StoreError::Invalid(
            "error outliers compare events of a window: use a filter without | stages as the window".into(),
        ));
    }
    let subset = match req.subset.trim() {
        "" => DEFAULT_SUBSET.to_string(),
        s if s.contains('|') => return Err(StoreError::Invalid("the subset is a filter: it cannot contain | stages".into())),
        s => s.to_string(),
    };
    if let Some(e) = store.check_query(&subset)? {
        return Err(StoreError::Invalid(format!("the subset does not compile: {}", e.message)));
    }
    let limit = if req.limit == 0 {
        DEFAULT_OUTLIERS
    } else {
        req.limit.min(MAX_OUTLIERS)
    };
    let subset_q = narrowed(&req.query, &format!("({subset})"));
    let window_total = count_of(store, &req.query)?;
    let subset_total = count_of(store, &subset_q)?;
    let rest_total = window_total.saturating_sub(subset_total);
    let mut coverage = Coverage {
        matched: window_total as u64,
        analysed: window_total as u64,
        limits: Vec::new(),
    };
    let mut report = OutlierReport {
        subset: subset.clone(),
        subset_total,
        rest_total,
        ..OutlierReport::default()
    };
    if subset_total == 0 || rest_total == 0 {
        return Ok(Analyzed { result: report, coverage });
    }
    let candidates = candidate_fields(store, req, &mut coverage)?;
    report.fields_checked = candidates.len();
    let mut found: Vec<Outlier> = Vec::new();
    for field in &candidates {
        cancelled()?;
        let in_subset = store.facet(&FacetRequest {
            query: subset_q.clone(),
            field: field.clone(),
            limit: SUBSET_VALUES,
        })?;
        if in_subset.values.is_empty() {
            continue;
        }
        let in_window = store.facet(&FacetRequest {
            query: req.query.clone(),
            field: field.clone(),
            limit: WINDOW_VALUES,
        })?;
        if in_window.distinct > MAX_DISTINCT {
            continue;
        }
        let window_counts: HashMap<&str, usize> = in_window.values.iter().map(|v| (v.value.as_str(), v.count)).collect();
        for v in &in_subset.values {
            let a = v.count;
            // A value beyond the window facet's top values has an unknown rest count: it cannot be judged.
            let Some(&all) = window_counts.get(v.value.as_str()) else {
                continue;
            };
            let c = all.saturating_sub(a);
            if a < MIN_SUPPORT || v.value.chars().count() > MAX_VALUE_CHARS {
                continue;
            }
            let l = lift(a, subset_total, c, rest_total);
            if l < MIN_LIFT {
                continue;
            }
            let g = g_statistic(a, subset_total, c, rest_total);
            if g < MIN_SIGNIFICANCE {
                continue;
            }
            found.push(Outlier {
                field: field.clone(),
                value: v.value.clone(),
                subset_count: a,
                rest_count: c,
                lift: l,
                significance: g,
                p_value: p_value(g),
                query: format!("{field} = {} and ({subset})", quote(&v.value)),
                sample_ids: Vec::new(),
            });
        }
        if in_subset.distinct > SUBSET_VALUES {
            coverage.limits.push(format!(
                "{field}: the {SUBSET_VALUES} most frequent of {} values in the subset were examined",
                in_subset.distinct
            ));
        }
    }
    found.sort_by(|x, y| {
        y.significance
            .total_cmp(&x.significance)
            .then_with(|| x.field.cmp(&y.field))
            .then_with(|| x.value.cmp(&y.value))
    });
    if found.len() > limit {
        coverage
            .limits
            .push(format!("{} outliers found; the {limit} strongest are shown", found.len()));
        found.truncate(limit);
    }
    for o in &mut found {
        cancelled()?;
        o.sample_ids = evidence(store, &req.query, &o.query);
    }
    report.outliers = found;
    Ok(Analyzed { result: report, coverage })
}

/// The first few events of the subset that carry the value; empty when the store cannot answer the query.
fn evidence(store: &dyn QueryPort, window: &Query, filter: &str) -> Vec<u32> {
    let q = narrowed(window, filter);
    match store.page(&PageQuery {
        query: q,
        offset: 0,
        limit: EVIDENCE_EVENTS,
        descending: false,
        anchor: None,
    }) {
        Ok(p) => p.rows.iter().map(|r| crate::analysis::anomaly::sample_id(r.id.0)).collect(),
        Err(_) => Vec::new(),
    }
}

fn candidate_fields(store: &dyn QueryPort, req: &OutlierRequest, coverage: &mut Coverage) -> StoreResult<Vec<String>> {
    if !req.fields.is_empty() {
        let mut fields: Vec<String> = Vec::new();
        for f in &req.fields {
            if !addressable(f) {
                return Err(StoreError::Invalid(format!(
                    "\"{f}\" is not a field name the query language can address"
                )));
            }
            if !fields.contains(f) {
                fields.push(f.clone());
            }
        }
        fields.truncate(MAX_FIELDS);
        return Ok(fields);
    }
    let summary = store.summary()?;
    let mut fields: Vec<_> = summary
        .fields
        .iter()
        .filter(|f| f.distinct >= 2 && f.distinct <= MAX_DISTINCT && f.count > 0 && addressable(&f.name))
        .collect();
    fields.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
    let mut names: Vec<String> = fields.iter().map(|f| f.name.clone()).collect();
    names.insert(0, "@source".into());
    if names.len() > MAX_FIELDS {
        coverage.limits.push(format!(
            "{} fields have a handful of values; the {MAX_FIELDS} most populated were examined",
            names.len()
        ));
        names.truncate(MAX_FIELDS);
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lift_is_finite_and_orders_by_concentration() {
        assert!(lift(10, 10, 0, 1000) > 100.0);
        assert!(lift(10, 10, 0, 1000).is_finite());
        assert!((lift(5, 100, 50, 1000) - 1.0).abs() < 0.2, "5% vs 5% is about 1");
        assert!(lift(20, 100, 10, 1000) > lift(10, 100, 10, 1000));
    }

    #[test]
    fn g_statistic_is_zero_without_association_and_grows_with_it() {
        assert!(g_statistic(5, 100, 50, 1000) < 0.5);
        let strong = g_statistic(40, 100, 20, 1000);
        let weak = g_statistic(8, 100, 50, 1000);
        assert!(strong > weak && strong > MIN_SIGNIFICANCE, "{strong} {weak}");
        assert_eq!(g_statistic(0, 0, 0, 0), 0.0, "an empty table is not an association");
        assert!(g_statistic(3, 3, 0, 10).is_finite());
    }

    #[test]
    fn p_values_follow_the_chi_square_table() {
        // chi-square (1 df): 3.841 -> 0.05, 6.635 -> 0.01, 10.828 -> 0.001
        assert!((p_value(3.841) - 0.05).abs() < 0.001);
        assert!((p_value(6.635) - 0.01).abs() < 0.0005);
        assert!((p_value(10.828) - 0.001).abs() < 0.0001);
        assert_eq!(p_value(0.0), 1.0);
        assert!(p_value(1e6) >= 1e-300);
        assert!((erfc(-1.0) - 1.8427).abs() < 0.001);
    }

    #[test]
    fn values_are_quoted_for_the_query_language() {
        assert_eq!(quote(r#"a"b\c"#), r#""a\"b\\c""#);
        assert_eq!(quote("x\ny"), "\"x y\"");
        assert!(addressable("SourceContext") && addressable("@source") && addressable("a.b-c"));
        assert!(!addressable("has space") && !addressable("") && !addressable("a|b"));
    }
}
