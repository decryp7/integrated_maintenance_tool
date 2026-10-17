//! Field conditions shared by profile rules (`when = [...]`) and query filters.

use crate::model::Record;
use anyhow::{anyhow, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConditionCfg {
    pub field: String,
    pub op: String,
    #[serde(default)]
    pub value: Option<CondValue>,
}

/// Lets TOML write `value = 5`, `value = true` or `value = "x"`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CondValue {
    Text(String),
    Number(f64),
    Bool(bool),
    List(Vec<String>),
}

impl CondValue {
    fn text(&self) -> String {
        match self {
            CondValue::Text(s) => s.clone(),
            CondValue::Number(n) => n.to_string(),
            CondValue::Bool(b) => b.to_string(),
            CondValue::List(l) => l.join("|"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Op {
    Eq,
    Ne,
    Contains,
    NotContains,
    Matches,
    Exists,
    Missing,
    Gt,
    Lt,
    In,
}

#[derive(Debug, Clone)]
pub struct Condition {
    field: String,
    op: Op,
    value: String,
    lower: String,
    regex: Option<Regex>,
    number: Option<f64>,
    set: Vec<String>,
}

impl Condition {
    pub fn compile(cfg: &ConditionCfg) -> Result<Condition> {
        Condition::new(&cfg.field, &cfg.op, cfg.value.as_ref().map(|v| v.text()).as_deref())
    }

    pub fn new(field: &str, op: &str, value: Option<&str>) -> Result<Condition> {
        let op = match op {
            "eq" | "=" | "==" => Op::Eq,
            "ne" | "!=" => Op::Ne,
            "contains" | "~" => Op::Contains,
            "notContains" | "not_contains" | "!~" => Op::NotContains,
            "matches" | "regex" => Op::Matches,
            "exists" => Op::Exists,
            "missing" => Op::Missing,
            "gt" | ">" => Op::Gt,
            "lt" | "<" => Op::Lt,
            "in" => Op::In,
            other => {
                return Err(anyhow!(
                    "unknown operator \"{other}\" (use eq ne contains notContains matches exists missing gt lt in)"
                ))
            }
        };
        let value = value.unwrap_or_default().to_string();
        if !matches!(op, Op::Exists | Op::Missing) && value.is_empty() && op != Op::Eq && op != Op::Ne {
            return Err(anyhow!("condition on \"{field}\" needs a value"));
        }
        let regex = match op {
            Op::Matches => Some(crate::user_regex::compile(&value, false).map_err(|e| anyhow!("invalid regex for \"{field}\": {e}"))?),
            _ => None,
        };
        let number = match op {
            Op::Gt | Op::Lt => Some(
                value
                    .trim()
                    .parse::<f64>()
                    .map_err(|_| anyhow!("\"{field}\" {op:?} needs a number"))?,
            ),
            _ => None,
        };
        let set = match op {
            Op::In => value.split('|').map(|s| s.trim().to_string()).collect(),
            _ => Vec::new(),
        };
        Ok(Condition {
            field: field.to_string(),
            lower: value.to_lowercase(),
            value,
            op,
            regex,
            number,
            set,
        })
    }

    pub fn eval(&self, rec: &Record, source_name: &str) -> bool {
        // The record's number (the level's rank for @level) and the threshold.
        let numbers = || {
            let n = match self.field.as_str() {
                "@level" => Some(rec.level.index() as f64),
                f => rec.field_number(f),
            };
            n.zip(self.number)
        };
        let v = || rec.field_text(&self.field, source_name);
        match self.op {
            Op::Gt => numbers().is_some_and(|(n, t)| n > t),
            Op::Lt => numbers().is_some_and(|(n, t)| n < t),
            Op::Exists => v().is_some_and(|s| !s.is_empty()),
            Op::Missing => v().is_none_or(|s| s.is_empty()),
            Op::Eq => v().is_some_and(|s| s == self.value || (self.field == "@level" && s.eq_ignore_ascii_case(&self.value))),
            Op::Ne => v().is_none_or(|s| s != self.value),
            Op::Contains => v().is_some_and(|s| s.to_lowercase().contains(&self.lower)),
            Op::NotContains => v().is_none_or(|s| !s.to_lowercase().contains(&self.lower)),
            Op::Matches => v().is_some_and(|s| self.regex.as_ref().is_some_and(|r| r.is_match(&s))),
            Op::In => v().is_some_and(|s| self.set.iter().any(|x| *x == *s)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Level;

    #[test]
    fn evaluates() {
        let mut r = Record::new(1);
        r.level = Level::Warning;
        r.message = "Application started".into();
        r.fields.insert("ElapsedMs".into(), 1200.into());
        r.fields.insert("Machine".into(), "WEB01".into());
        assert!(Condition::new("@level", "eq", Some("warning")).unwrap().eval(&r, "f"));
        assert!(Condition::new("@level", "gt", Some("2")).unwrap().eval(&r, "f"));
        assert!(Condition::new("ElapsedMs", "gt", Some("1000")).unwrap().eval(&r, "f"));
        assert!(Condition::new("@message", "matches", Some("(?i)start")).unwrap().eval(&r, "f"));
        assert!(Condition::new("Machine", "in", Some("WEB01|WEB02")).unwrap().eval(&r, "f"));
        assert!(Condition::new("Nope", "missing", None).unwrap().eval(&r, "f"));
        assert!(!Condition::new("@exception", "exists", None).unwrap().eval(&r, "f"));
    }

    #[test]
    fn all_operators() {
        let mut r = Record::new(1);
        r.message = "Payment Timed Out".into();
        r.fields.insert("Tag".into(), "".into());
        r.fields.insert("Flag".into(), true.into());
        let c = |f: &str, op: &str, v: Option<&str>| Condition::new(f, op, v).unwrap().eval(&r, "f");
        assert!(c("@message", "contains", Some("timed out")));
        assert!(!c("@message", "notContains", Some("TIMED")));
        assert!(c("@message", "notContains", Some("crash")));
        assert!(c("Nope", "notContains", Some("x")));
        assert!(c("@message", "ne", Some("x")));
        assert!(c("Nope", "ne", Some("x")));
        assert!(c("Tag", "missing", None));
        assert!(!c("Tag", "exists", None));
        assert!(c("Flag", "eq", Some("true")));
        assert!(c("@line", "lt", Some("2")));
        assert!(!c("Nope", "gt", Some("1")));
        assert!(!c("@message", "in", Some("a|b")));
        assert!(c("@level", "eq", Some("information")));
        for op in ["=", "==", "!=", "~", "!~", "regex", ">", "<", "not_contains"] {
            assert!(Condition::new("x", op, Some("1")).is_ok(), "{op}");
        }
    }

    #[test]
    fn rejects_bad_conditions() {
        assert!(Condition::new("x", "like", Some("a")).is_err());
        assert!(Condition::new("x", "contains", None).is_err());
        assert!(Condition::new("x", "matches", Some("(")).is_err());
        assert!(Condition::new("x", "gt", Some("ten")).is_err());
        assert!(Condition::new("x", "eq", None).is_ok());
    }

    #[test]
    fn toml_values() {
        let cfg: Vec<ConditionCfg> = toml::from_str::<toml::Table>(
            "c = [{field='a',op='gt',value=5},{field='b',op='eq',value=true},{field='c',op='in',value=['x','y']},{field='d',op='eq',value='s'}]",
        )
        .unwrap()["c"]
            .clone()
            .try_into()
            .unwrap();
        let mut r = Record::new(1);
        r.fields.insert("a".into(), 6.into());
        r.fields.insert("b".into(), true.into());
        r.fields.insert("c".into(), "y".into());
        r.fields.insert("d".into(), "s".into());
        for c in &cfg {
            assert!(Condition::compile(c).unwrap().eval(&r, "f"), "{c:?}");
        }
    }
}
