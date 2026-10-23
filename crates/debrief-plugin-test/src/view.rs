//! An annotator's view in the fake host: time bounds, order, limit and the field conditions of
//! [`debrief_plugin_api::annotate::ViewFilter`]. A test double: Debrief's engine is the reference for how conditions
//! evaluate; this covers the operators annotators use (`eq`, `ne`, `contains`, `not_contains`, `matches`, `exists`,
//! `missing`, `gt`, `lt`, `in`).

use debrief_plugin_api::annotate::{ViewFilter, MAX_VIEW_RECORDS};
use debrief_plugin_api::condition::{CondValue, ConditionCfg};
use debrief_plugin_api::model::Level;
use debrief_plugin_api::protocol::{WireValue, WireViewRecord};

/// The text of `field` in `r` (`@level`, `@message`, ... or a field), `None` when it has none.
fn value_of(r: &WireViewRecord, field: &str) -> Option<String> {
    let e = &r.event;
    match field {
        "@level" => Level::ALL.get(e.level as usize).map(|l| format!("{l:?}")),
        "@message" => Some(e.message.clone()),
        "@exception" => e.exception.clone(),
        "@template" => e.template.clone(),
        "@source" => Some(r.source.clone()),
        "@line" => Some(e.line.to_string()),
        name => e.fields.iter().find(|(k, _)| k == name).and_then(|(_, v)| match v {
            WireValue::Null => None,
            WireValue::Bool(b) => Some(b.to_string()),
            WireValue::Int(i) => Some(i.to_string()),
            WireValue::UInt(u) => Some(u.to_string()),
            WireValue::Float(f) => Some(f.to_string()),
            WireValue::Text(s) | WireValue::Json(s) => Some(s.clone()),
        }),
    }
}

fn matches(c: &ConditionCfg, r: &WireViewRecord) -> Result<bool, String> {
    let v = value_of(r, &c.field);
    let operand = c.value.as_ref().map(CondValue::text).unwrap_or_default();
    let num = |s: &str| s.trim().parse::<f64>().ok();
    Ok(match c.op.as_str() {
        "exists" => v.is_some(),
        "missing" => v.is_none(),
        "eq" => v.is_some_and(|v| v.eq_ignore_ascii_case(&operand)),
        "ne" => !v.is_some_and(|v| v.eq_ignore_ascii_case(&operand)),
        "contains" => v.is_some_and(|v| v.to_lowercase().contains(&operand.to_lowercase())),
        "not_contains" => !v.is_some_and(|v| v.to_lowercase().contains(&operand.to_lowercase())),
        "in" => {
            let alts: Vec<String> = match &c.value {
                Some(CondValue::List(l)) => l.clone(),
                _ => operand.split('|').map(str::to_string).collect(),
            };
            v.is_some_and(|v| alts.iter().any(|a| a.eq_ignore_ascii_case(&v)))
        }
        "gt" | "lt" => match (v.as_deref().and_then(num), num(&operand)) {
            (Some(a), Some(b)) => (c.op == "gt" && a > b) || (c.op == "lt" && a < b),
            _ => false,
        },
        "matches" => {
            let re = debrief_plugin_api::user_regex::compile(&operand, false).map_err(|e| format!("{}: {e}", c.field))?;
            v.is_some_and(|v| re.is_match(&v))
        }
        other => return Err(format!("unknown operator \"{other}\"")),
    })
}

/// The rows of `rows` that `filter` (JSON of a [`ViewFilter`]) selects, at most `limit`.
pub(crate) fn select(rows: &[WireViewRecord], filter: &serde_json::Value, limit: usize) -> Result<Vec<WireViewRecord>, String> {
    let f: ViewFilter = serde_json::from_value(filter.clone()).map_err(|e| format!("invalid filter: {e}"))?;
    let mut out = Vec::new();
    for r in rows {
        let in_time = match (r.event.ts, f.from, f.to) {
            (_, None, None) => true,
            (None, _, _) => false,
            (Some(t), from, to) => from.is_none_or(|a| t >= a) && to.is_none_or(|b| t <= b),
        };
        if !in_time {
            continue;
        }
        let mut ok = true;
        for c in &f.when {
            if !matches(c, r)? {
                ok = false;
                break;
            }
        }
        if ok {
            out.push(r.clone());
        }
    }
    out.sort_by_key(|r| r.event.ts.unwrap_or(i64::MIN));
    if f.newest_first {
        out.reverse();
    }
    out.truncate(limit.min(MAX_VIEW_RECORDS));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use debrief_plugin_api::protocol::WireEvent;
    use debrief_plugin_api::Record;
    use serde_json::json;

    fn row(ts: i64, level: Level, fields: &[(&str, serde_json::Value)]) -> WireViewRecord {
        let mut r = Record::new(1);
        r.ts = Some(ts);
        r.level = level;
        r.message = format!("m{ts}");
        for (k, v) in fields {
            r.fields.insert(k.to_string(), v.clone());
        }
        WireViewRecord {
            event: WireEvent::of(&r),
            source: "a.log".into(),
            instance: "i".into(),
        }
    }

    #[test]
    fn filters_by_time_conditions_order_and_limit() {
        let rows = vec![
            row(3, Level::Error, &[("Code", json!(500))]),
            row(1, Level::Information, &[("Code", json!(200))]),
            row(2, Level::Warning, &[("Host", json!("web01"))]),
        ];
        let ts = |v: Vec<WireViewRecord>| v.iter().map(|r| r.event.ts.unwrap()).collect::<Vec<_>>();
        assert_eq!(ts(select(&rows, &json!({}), 10).unwrap()), [1, 2, 3]);
        assert_eq!(ts(select(&rows, &json!({"newestFirst": true}), 2).unwrap()), [3, 2]);
        assert_eq!(ts(select(&rows, &json!({"from": 2, "to": 2}), 10).unwrap()), [2]);
        let when = |c: serde_json::Value| ts(select(&rows, &json!({ "when": [c] }), 10).unwrap());
        assert_eq!(when(json!({"field": "@level", "op": "eq", "value": "error"})), [3]);
        assert_eq!(when(json!({"field": "Code", "op": "gt", "value": 300})), [3]);
        assert_eq!(when(json!({"field": "Code", "op": "lt", "value": 300})), [1]);
        assert_eq!(when(json!({"field": "Host", "op": "exists"})), [2]);
        assert_eq!(when(json!({"field": "Host", "op": "missing"})), [1, 3]);
        assert_eq!(when(json!({"field": "Code", "op": "in", "value": ["200", "404"]})), [1]);
        assert_eq!(when(json!({"field": "@message", "op": "contains", "value": "M2"})), [2]);
        assert_eq!(when(json!({"field": "@message", "op": "not_contains", "value": "m2"})), [1, 3]);
        assert_eq!(when(json!({"field": "@message", "op": "matches", "value": "^m[13]$"})), [1, 3]);
        assert_eq!(when(json!({"field": "@source", "op": "ne", "value": "a.log"})), Vec::<i64>::new());
        assert!(select(&rows, &json!({"when": [{"field": "x", "op": "nope"}]}), 10).is_err());
        assert!(select(&rows, &json!({"when": 5}), 10).is_err());
    }
}
