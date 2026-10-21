//! `[[route]]`: splitting one physical file into logical sources by the fields of its events (ADR 0020), e.g.
//! a log server's file holding several products or hosts. A route has `when` conditions (the condition
//! language of extractors and markers) and a `source` label template such as `"{App}@{Host}"`; the first
//! route whose conditions hold and whose template fields are all present names the event's logical source.
//! Events no route names stay in the file's own source.

use crate::condition::{Condition, ConditionCfg};
use crate::ingest::{ProblemKind, Problems};
use crate::model::Record;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

/// Most logical sources one file is split into; events of further labels stay in the file's own source and
/// are reported ([`ProblemKind::RouteLimit`]).
pub const MAX_LOGICAL_SOURCES: usize = 256;
/// Longest label, in characters; longer labels are cut.
pub const MAX_LABEL_CHARS: usize = 120;
/// Most routes a profile may have.
pub const MAX_ROUTES: usize = 64;

/// `[[route]]` as written in a profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteCfg {
    /// All must hold (empty: every event).
    #[serde(default)]
    pub when: Vec<ConditionCfg>,
    /// Label of the logical source: text with `{Field}` holes (fields or pseudo-fields such as `@level`);
    /// `{{` and `}}` are literal braces.
    pub source: String,
}

/// A piece of a label template.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    Text(String),
    Field(String),
}

/// A compiled route.
#[derive(Debug, Clone)]
pub struct Route {
    when: Vec<Condition>,
    label: Vec<Part>,
}

impl Route {
    pub fn compile(cfg: &RouteCfg) -> Result<Route> {
        let when = crate::profile::compile_conditions(&cfg.when)?;
        let label = parse_label(&cfg.source)?;
        Ok(Route { when, label })
    }

    /// The label of `rec`'s logical source under this route, if the route applies: its conditions hold and
    /// every field of the template is present and not empty.
    pub fn label(&self, rec: &Record, source_name: &str) -> Option<String> {
        if !self.when.iter().all(|c| c.eval(rec, source_name)) {
            return None;
        }
        let mut out = String::new();
        for part in &self.label {
            match part {
                Part::Text(t) => out.push_str(t),
                Part::Field(f) => out.push_str(&rec.field_text(f, source_name).filter(|v| !v.trim().is_empty())?),
            }
        }
        let label: String = out
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .take(MAX_LABEL_CHARS)
            .collect();
        let label = label.trim();
        (!label.is_empty()).then(|| label.to_string())
    }
}

/// Compiles every route; fails with the number of the route that is wrong.
pub fn compile_all(cfgs: &[RouteCfg]) -> Result<Vec<Route>> {
    if cfgs.len() > MAX_ROUTES {
        return Err(anyhow!("at most {MAX_ROUTES} [[route]] entries are supported"));
    }
    cfgs.iter()
        .enumerate()
        .map(|(i, c)| Route::compile(c).map_err(|e| anyhow!("[[route]] {}: {e:#}", i + 1)))
        .collect()
}

fn parse_label(template: &str) -> Result<Vec<Part>> {
    let mut parts = Vec::new();
    let mut text = String::new();
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                text.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                text.push('}');
            }
            '{' => {
                let mut name = String::new();
                let mut closed = false;
                for c in chars.by_ref() {
                    if c == '}' {
                        closed = true;
                        break;
                    }
                    name.push(c);
                }
                if !closed {
                    return Err(anyhow!("source \"{template}\": unclosed '{{'"));
                }
                let name = name.trim();
                if name.is_empty() {
                    return Err(anyhow!("source \"{template}\": a {{}} hole needs a field name"));
                }
                if !text.is_empty() {
                    parts.push(Part::Text(std::mem::take(&mut text)));
                }
                parts.push(Part::Field(name.to_string()));
            }
            '}' => return Err(anyhow!("source \"{template}\": unmatched '}}' (write '}}}}' for a brace)")),
            c => text.push(c),
        }
    }
    if !text.is_empty() {
        parts.push(Part::Text(text));
    }
    if parts.is_empty() {
        return Err(anyhow!("source must not be empty"));
    }
    Ok(parts)
}

/// The label of `rec`'s logical source: the first route that applies.
pub fn route(routes: &[Route], rec: &Record, source_name: &str) -> Option<String> {
    routes.iter().find_map(|r| r.label(rec, source_name))
}

/// The events of one file, split by route.
#[derive(Debug, Default)]
pub struct Split {
    /// Events no route names (or past [`MAX_LOGICAL_SOURCES`]): they stay in the file's own source.
    pub unrouted: Vec<Record>,
    /// Logical sources in order of first appearance, with their events.
    pub logical: Vec<(String, Vec<Record>)>,
    pub problems: Problems,
}

/// Splits the events of the file named `source_name` by `routes`.
pub fn split(routes: &[Route], records: Vec<Record>, source_name: &str) -> Split {
    let mut out = Split::default();
    if routes.is_empty() {
        out.unrouted = records;
        return out;
    }
    let mut index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for rec in records {
        let Some(label) = route(routes, &rec, source_name) else {
            out.unrouted.push(rec);
            continue;
        };
        let at = match index.get(&label) {
            Some(&i) => i,
            None if out.logical.len() < MAX_LOGICAL_SOURCES => {
                index.insert(label.clone(), out.logical.len());
                out.logical.push((label, Vec::new()));
                out.logical.len() - 1
            }
            None => {
                out.problems.add(ProblemKind::RouteLimit, rec.line);
                out.unrouted.push(rec);
                continue;
            }
        };
        out.logical[at].1.push(rec);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rec(line: u32, fields: serde_json::Value) -> Record {
        let mut r = Record::new(line);
        if let serde_json::Value::Object(m) = fields {
            r.fields = m;
        }
        r
    }

    fn cfg(when: &[(&str, &str, &str)], source: &str) -> RouteCfg {
        RouteCfg {
            when: when
                .iter()
                .map(|(f, op, v)| ConditionCfg {
                    field: f.to_string(),
                    op: op.to_string(),
                    value: Some(crate::condition::CondValue::Text(v.to_string())),
                })
                .collect(),
            source: source.into(),
        }
    }

    #[test]
    fn labels_from_fields() {
        let r = Route::compile(&cfg(&[], "{App}@{Host} {{x}}")).unwrap();
        let ev = rec(1, json!({"App": "billing", "Host": "srv1"}));
        assert_eq!(r.label(&ev, "f.log").as_deref(), Some("billing@srv1 {x}"));
        assert_eq!(r.label(&rec(1, json!({"App": "billing"})), "f.log"), None, "a hole without a value");
        assert_eq!(r.label(&rec(1, json!({"App": " ", "Host": "h"})), "f.log"), None, "an empty value");
        let pseudo = Route::compile(&cfg(&[], "{ @source }:{@level}")).unwrap();
        assert_eq!(pseudo.label(&rec(1, json!({})), "f.log").as_deref(), Some("f.log:Information"));
        let ctrl = Route::compile(&cfg(&[], "{App}")).unwrap();
        let long = "y".repeat(500);
        assert_eq!(ctrl.label(&rec(1, json!({"App": "a\u{1}b"})), "s").as_deref(), Some("a b"));
        assert_eq!(
            ctrl.label(&rec(1, json!({"App": long})), "s").unwrap().chars().count(),
            MAX_LABEL_CHARS
        );
        let constant = Route::compile(&cfg(&[("App", "eq", "billing")], "Billing")).unwrap();
        assert_eq!(constant.label(&ev, "s").as_deref(), Some("Billing"));
        assert_eq!(constant.label(&rec(1, json!({"App": "web"})), "s"), None);
    }

    #[test]
    fn invalid_templates_and_limits() {
        for bad in ["", "{}", "{ }", "a}b", "{App"] {
            assert!(Route::compile(&cfg(&[], bad)).is_err(), "{bad}");
        }
        assert!(Route::compile(&cfg(&[("App", "bogus", "x")], "x")).is_err());
        let many = vec![cfg(&[], "x"); MAX_ROUTES + 1];
        assert!(compile_all(&many).unwrap_err().to_string().contains("at most"));
        let e = compile_all(&[cfg(&[], "x"), cfg(&[], "")]).unwrap_err();
        assert!(e.to_string().starts_with("[[route]] 2:"), "{e}");
    }

    #[test]
    fn splits_by_first_matching_route_with_a_cap() {
        let routes = compile_all(&[cfg(&[("App", "eq", "web")], "web"), cfg(&[], "{App}@{Host}")]).unwrap();
        let records = vec![
            rec(1, json!({"App": "billing", "Host": "srv1"})),
            rec(2, json!({"App": "web", "Host": "srv2"})),
            rec(3, json!({"Other": 1})),
            rec(4, json!({"App": "billing", "Host": "srv1"})),
        ];
        let s = split(&routes, records, "f.log");
        let labels: Vec<(&str, Vec<u32>)> = s
            .logical
            .iter()
            .map(|(l, r)| (l.as_str(), r.iter().map(|r| r.line).collect()))
            .collect();
        assert_eq!(labels, [("billing@srv1", vec![1, 4]), ("web", vec![2])]);
        assert_eq!(s.unrouted.iter().map(|r| r.line).collect::<Vec<_>>(), [3]);
        assert!(s.problems.is_empty());
        let none = split(&[], vec![rec(1, json!({}))], "f");
        assert_eq!((none.unrouted.len(), none.logical.len()), (1, 0));

        let one = compile_all(&[cfg(&[], "{N}")]).unwrap();
        let records: Vec<Record> = (0..MAX_LOGICAL_SOURCES as u32 + 3).map(|i| rec(i + 1, json!({ "N": i }))).collect();
        let s = split(&one, records, "f");
        assert_eq!(s.logical.len(), MAX_LOGICAL_SOURCES);
        assert_eq!(s.unrouted.len(), 3);
        assert_eq!(s.problems.count(ProblemKind::RouteLimit), 3);
        assert_eq!(route(&one, &rec(1, json!({"N": 7})), "f").as_deref(), Some("7"));
    }
}
