//! Compares two sets of events (typically a healthy and a bad time window).

use super::field_stats::ValueCount;
use super::util::{fmt_num, percentile_sorted, sorted};
use crate::model::{Level, Record};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CompareConfig {
    /// Values listed per changed field.
    pub top_values: usize,
    /// Fields with more distinct values than this are not compared as categories.
    pub max_distinct: usize,
    /// Smallest total variation distance (0..1) worth reporting.
    pub min_distance: f64,
    /// Relative median/p95 change that counts for numeric fields (0.25 = 25%).
    pub min_change: f64,
    /// A value must occur this often to be reported as new or gone.
    pub min_new_count: u64,
    /// Fields listed per section.
    pub max_fields: usize,
}

impl Default for CompareConfig {
    fn default() -> Self {
        CompareConfig {
            top_values: 5,
            max_distinct: 50,
            min_distance: 0.1,
            min_change: 0.25,
            min_new_count: 3,
            max_fields: 10,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LevelCount {
    pub level: Level,
    pub count: u64,
    pub per_minute: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SideStats {
    pub count: u64,
    pub from: Option<i64>,
    pub to: Option<i64>,
    /// Span of the timestamps, at least one second.
    pub duration_ms: i64,
    pub per_minute: f64,
    pub levels: Vec<LevelCount>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValueShare {
    pub value: String,
    pub count_a: u64,
    pub count_b: u64,
    pub share_a: f64,
    pub share_b: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldShift {
    pub field: String,
    /// Total variation distance between the two value distributions (0..1).
    pub distance: f64,
    pub top: Vec<ValueShare>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NumberSummary {
    pub count: usize,
    pub median: f64,
    pub p95: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NumericShift {
    pub field: String,
    pub a: NumberSummary,
    pub b: NumberSummary,
    /// `None` when the baseline is zero.
    pub median_change_pct: Option<f64>,
    pub p95_change_pct: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValueAppearance {
    pub field: String,
    pub values: Vec<ValueCount>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comparison {
    pub a: SideStats,
    pub b: SideStats,
    pub changed_fields: Vec<FieldShift>,
    pub numeric_changes: Vec<NumericShift>,
    /// Values seen only in B, per field.
    pub new_values: Vec<ValueAppearance>,
    /// Values seen only in A, per field.
    pub gone_values: Vec<ValueAppearance>,
    pub summary: Vec<String>,
}

#[derive(Default)]
struct Side {
    /// field -> value -> count (categorical view).
    values: BTreeMap<String, BTreeMap<String, u64>>,
    /// field -> numeric values.
    numbers: BTreeMap<String, Vec<f64>>,
}

fn scan<'a>(records: impl IntoIterator<Item = &'a Record>) -> (Side, SideStats) {
    let mut side = Side::default();
    let mut counts = [0u64; 6];
    let (mut n, mut from, mut to): (u64, Option<i64>, Option<i64>) = (0, None, None);
    for r in records {
        n += 1;
        counts[r.level.index()] += 1;
        if let Some(ts) = r.ts {
            from = Some(from.map_or(ts, |f| f.min(ts)));
            to = Some(to.map_or(ts, |t| t.max(ts)));
        }
        if let Some(t) = &r.template {
            *side.values.entry("@template".into()).or_default().entry(t.clone()).or_insert(0) += 1;
        }
        for (name, v) in &r.fields {
            let text = crate::model::value_text(v).into_owned();
            *side.values.entry(name.clone()).or_default().entry(text).or_insert(0) += 1;
            if let (Value::Number(_), Some(f)) = (v, v.as_f64()) {
                side.numbers.entry(name.clone()).or_default().push(f);
            }
        }
    }
    let duration_ms = from.zip(to).map_or(0, |(f, t)| t - f).max(1000);
    let per_min = |c: u64| c as f64 * 60_000.0 / duration_ms as f64;
    let stats = SideStats {
        count: n,
        from,
        to,
        duration_ms,
        per_minute: per_min(n),
        levels: Level::ALL
            .iter()
            .map(|l| LevelCount {
                level: *l,
                count: counts[l.index()],
                per_minute: per_min(counts[l.index()]),
            })
            .collect(),
    };
    (side, stats)
}

fn errors(s: &SideStats) -> f64 {
    s.levels.iter().filter(|l| l.level >= Level::Error).map(|l| l.per_minute).sum()
}

fn change_pct(a: f64, b: f64) -> Option<f64> {
    (a != 0.0).then(|| (b - a) / a.abs() * 100.0)
}

fn changed(a: f64, b: f64, min: f64) -> bool {
    if a == 0.0 {
        b != 0.0
    } else {
        (b - a).abs() / a.abs() >= min
    }
}

fn summarize(a: &SideStats, b: &SideStats, cmp: &Comparison) -> Vec<String> {
    let mut out = Vec::new();
    if a.count == 0 || b.count == 0 {
        out.push(format!("One side is empty (A has {} events, B has {})", a.count, b.count));
        return out;
    }
    let ratio = b.per_minute / a.per_minute;
    out.push(if !(1.0 / 1.5..1.5).contains(&ratio) {
        format!(
            "B logs {}x the events per minute of A ({}/min vs {}/min)",
            fmt_num(ratio),
            fmt_num(b.per_minute),
            fmt_num(a.per_minute)
        )
    } else {
        format!(
            "Event rate is similar ({}/min in A, {}/min in B)",
            fmt_num(a.per_minute),
            fmt_num(b.per_minute)
        )
    });
    let (ea, eb) = (errors(a), errors(b));
    if ea > 0.0 || eb > 0.0 {
        let verb = if eb > ea { "rose" } else { "fell" };
        out.push(format!("Errors {verb} from {}/min to {}/min", fmt_num(ea), fmt_num(eb)));
    }
    for f in cmp.changed_fields.iter().take(3) {
        if let Some(t) = f
            .top
            .iter()
            .max_by(|x, y| (x.share_b - x.share_a).abs().total_cmp(&(y.share_b - y.share_a).abs()))
        {
            out.push(format!(
                "{} changed: \"{}\" went from {}% to {}% of events",
                f.field,
                t.value,
                fmt_num(t.share_a * 100.0),
                fmt_num(t.share_b * 100.0)
            ));
        }
    }
    for n in cmp.numeric_changes.iter().take(3) {
        let verb = if n.b.median > n.a.median || n.b.p95 > n.a.p95 {
            "rose"
        } else {
            "fell"
        };
        out.push(format!(
            "{} {verb}: median {} to {}, p95 {} to {}",
            n.field,
            fmt_num(n.a.median),
            fmt_num(n.b.median),
            fmt_num(n.a.p95),
            fmt_num(n.b.p95)
        ));
    }
    for (label, list) in [("New in B", &cmp.new_values), ("Only in A", &cmp.gone_values)] {
        for v in list.iter().take(2) {
            let names: Vec<String> = v.values.iter().take(3).map(|c| format!("\"{}\" ({})", c.value, c.count)).collect();
            out.push(format!("{label}: {} = {}", v.field, names.join(", ")));
        }
    }
    out
}

/// Compares window A (the baseline) with window B.
pub fn compare_windows<'a>(
    a: impl IntoIterator<Item = &'a Record>,
    b: impl IntoIterator<Item = &'a Record>,
    cfg: &CompareConfig,
) -> Comparison {
    let ((sa, stats_a), (sb, stats_b)) = (scan(a), scan(b));
    let fields: BTreeSet<&String> = sa.values.keys().chain(sb.values.keys()).collect();
    let empty_map = BTreeMap::new();
    let mut changed_fields = Vec::new();
    let (mut new_values, mut gone_values) = (Vec::new(), Vec::new());
    for field in fields {
        let (va, vb) = (
            sa.values.get(field).unwrap_or(&empty_map),
            sb.values.get(field).unwrap_or(&empty_map),
        );
        let distinct: BTreeSet<&String> = va.keys().chain(vb.keys()).collect();
        if distinct.len() > cfg.max_distinct {
            continue;
        }
        let (ta, tb) = (va.values().sum::<u64>() as f64, vb.values().sum::<u64>() as f64);
        let share = |m: &BTreeMap<String, u64>, total: f64, v: &String| m.get(v).map_or(0.0, |c| *c as f64 / total);
        let mut rows: Vec<ValueShare> = distinct
            .iter()
            .map(|v| ValueShare {
                value: (*v).clone(),
                count_a: va.get(*v).copied().unwrap_or(0),
                count_b: vb.get(*v).copied().unwrap_or(0),
                share_a: share(va, ta, v),
                share_b: share(vb, tb, v),
            })
            .collect();
        let distance = rows.iter().map(|r| (r.share_a - r.share_b).abs()).sum::<f64>() / 2.0;
        if distance >= cfg.min_distance {
            rows.sort_by(|x, y| {
                (y.share_b - y.share_a)
                    .abs()
                    .total_cmp(&(x.share_b - x.share_a).abs())
                    .then_with(|| x.value.cmp(&y.value))
            });
            rows.truncate(cfg.top_values);
            changed_fields.push(FieldShift {
                field: field.clone(),
                distance,
                top: rows,
            });
        }
        // Appearing/disappearing values only make sense when both sides logged the field.
        if !va.is_empty() && !vb.is_empty() {
            let only = |from: &BTreeMap<String, u64>, other: &BTreeMap<String, u64>| {
                let mut v: Vec<ValueCount> = from
                    .iter()
                    .filter(|(k, c)| !other.contains_key(*k) && **c >= cfg.min_new_count)
                    .map(|(k, c)| ValueCount {
                        value: k.clone(),
                        count: *c,
                    })
                    .collect();
                v.sort_by(|x, y| y.count.cmp(&x.count).then_with(|| x.value.cmp(&y.value)));
                v.truncate(cfg.top_values);
                v
            };
            for (target, values) in [(&mut new_values, only(vb, va)), (&mut gone_values, only(va, vb))] {
                if !values.is_empty() {
                    target.push(ValueAppearance {
                        field: field.clone(),
                        values,
                    });
                }
            }
        }
    }
    changed_fields.sort_by(|x, y| y.distance.total_cmp(&x.distance).then_with(|| x.field.cmp(&y.field)));
    changed_fields.truncate(cfg.max_fields);
    new_values.truncate(cfg.max_fields);
    gone_values.truncate(cfg.max_fields);

    let mut numeric_changes = Vec::new();
    for (field, na) in &sa.numbers {
        let Some(nb) = sb.numbers.get(field) else { continue };
        if na.len() < 5 || nb.len() < 5 {
            continue;
        }
        let (sum_a, sum_b) = (summary(na), summary(nb));
        if changed(sum_a.median, sum_b.median, cfg.min_change) || changed(sum_a.p95, sum_b.p95, cfg.min_change) {
            numeric_changes.push(NumericShift {
                field: field.clone(),
                median_change_pct: change_pct(sum_a.median, sum_b.median),
                p95_change_pct: change_pct(sum_a.p95, sum_b.p95),
                a: sum_a,
                b: sum_b,
            });
        }
    }
    let magnitude = |n: &NumericShift| {
        let rel = |a: f64, b: f64| (b - a).abs() / a.abs().max(b.abs()).max(f64::MIN_POSITIVE);
        rel(n.a.median, n.b.median).max(rel(n.a.p95, n.b.p95))
    };
    numeric_changes.sort_by(|x, y| magnitude(y).total_cmp(&magnitude(x)).then_with(|| x.field.cmp(&y.field)));
    numeric_changes.truncate(cfg.max_fields);

    let mut cmp = Comparison {
        a: stats_a,
        b: stats_b,
        changed_fields,
        numeric_changes,
        new_values,
        gone_values,
        summary: Vec::new(),
    };
    cmp.summary = summarize(&cmp.a, &cmp.b, &cmp);
    cmp
}

fn summary(v: &[f64]) -> NumberSummary {
    let s = sorted(v);
    NumberSummary {
        count: s.len(),
        median: percentile_sorted(&s, 50.0),
        p95: percentile_sorted(&s, 95.0),
    }
}
