//! Field conditions shared by profile rules (`when = [...]`) and the grid's field filters.
//!
//! A [`ConditionCfg`] (TOML or JSON) is parsed once into a typed [`Check`], one variant per operator,
//! and lowered into the query language's [`Expr`] (the one predicate language, redesign R2), which is
//! what evaluates it and what the store compiles to SQL.

use crate::model::Record;
use crate::ql::{Expr, NumOp};
use crate::row::{FieldKeys, RecordRow, RowRef};
use anyhow::{anyhow, Result};
use regex::Regex;

pub use imt_plugin_api::condition::{CondValue, ConditionCfg};

/// What a condition checks: one variant per operator, with operands of the right type.
#[derive(Debug, Clone)]
pub enum Check {
    /// Text equal (level names ignore case).
    Eq {
        field: String,
        value: String,
    },
    /// Not [`Check::Eq`] (a missing field is not equal).
    Ne {
        field: String,
        value: String,
    },
    /// Contains, ignoring case (`needle` is lower-case).
    Contains {
        field: String,
        needle: String,
    },
    NotContains {
        field: String,
        needle: String,
    },
    Matches {
        field: String,
        re: Regex,
    },
    /// Present and not empty.
    Exists {
        field: String,
    },
    /// Absent or empty.
    Missing {
        field: String,
    },
    /// Numerically greater (levels by severity rank).
    Gt {
        field: String,
        value: f64,
    },
    Lt {
        field: String,
        value: f64,
    },
    /// Text equal to one of the values.
    In {
        field: String,
        values: Vec<String>,
    },
}

/// The operators, as written in profiles and filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

impl Op {
    fn parse(op: &str) -> Result<Op> {
        Ok(match op {
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
        })
    }
}

/// The operand of a condition before it is checked against its operator.
enum Operand<'a> {
    None,
    Text(&'a str),
    Number(f64),
    List(&'a [String]),
}

impl Check {
    fn build(field: &str, op: Op, value: Operand<'_>) -> Result<Check> {
        let text = || -> Result<String> {
            match &value {
                Operand::None => Ok(String::new()),
                Operand::Text(t) => Ok(t.to_string()),
                Operand::Number(n) => Ok(n.to_string()),
                Operand::List(_) => Err(anyhow!("condition on \"{field}\": a list of values needs op = \"in\"")),
            }
        };
        let needs_value = |v: String| -> Result<String> {
            if v.is_empty() {
                Err(anyhow!("condition on \"{field}\" needs a value"))
            } else {
                Ok(v)
            }
        };
        let number = |name: &str| -> Result<f64> {
            match &value {
                Operand::Number(n) => Ok(*n),
                Operand::Text(t) => t.trim().parse::<f64>().map_err(|_| anyhow!("\"{field}\" {name} needs a number")),
                Operand::None => Err(anyhow!("condition on \"{field}\" needs a value")),
                Operand::List(_) => Err(anyhow!("\"{field}\" {name} needs a number")),
            }
        };
        let owned = field.to_string();
        Ok(match op {
            Op::Eq => Check::Eq {
                value: text()?,
                field: owned,
            },
            Op::Ne => Check::Ne {
                value: text()?,
                field: owned,
            },
            Op::Contains => Check::Contains {
                needle: needs_value(text()?)?.to_lowercase(),
                field: owned,
            },
            Op::NotContains => Check::NotContains {
                needle: needs_value(text()?)?.to_lowercase(),
                field: owned,
            },
            Op::Matches => {
                let pattern = needs_value(text()?)?;
                let re = crate::user_regex::compile(&pattern, false).map_err(|e| anyhow!("invalid regex for \"{field}\": {e}"))?;
                Check::Matches { field: owned, re }
            }
            Op::Exists => Check::Exists { field: owned },
            Op::Missing => Check::Missing { field: owned },
            Op::Gt => Check::Gt {
                value: number("Gt")?,
                field: owned,
            },
            Op::Lt => Check::Lt {
                value: number("Lt")?,
                field: owned,
            },
            Op::In => {
                let values = match value {
                    // A list is taken as it is (its values may contain `|`).
                    Operand::List(l) => l.iter().map(|s| s.trim().to_string()).collect(),
                    // Text (the grid's filter box) separates values with `|`.
                    _ => needs_value(text()?)?.split('|').map(|s| s.trim().to_string()).collect(),
                };
                Check::In { field: owned, values }
            }
        })
    }

    pub fn field(&self) -> &str {
        match self {
            Check::Eq { field, .. }
            | Check::Ne { field, .. }
            | Check::Contains { field, .. }
            | Check::NotContains { field, .. }
            | Check::Matches { field, .. }
            | Check::Exists { field }
            | Check::Missing { field }
            | Check::Gt { field, .. }
            | Check::Lt { field, .. }
            | Check::In { field, .. } => field,
        }
    }

    /// The check as a query-language expression, with field names interned in `keys`.
    pub fn lower(&self, keys: &mut FieldKeys) -> Expr {
        let field = keys.key(self.field());
        let level = self.field() == "@level";
        match self {
            Check::Eq { value, .. } => Expr::TextEq {
                field,
                value: value.clone(),
                ignore_case: level,
            },
            Check::Ne { value, .. } => Expr::Not(Box::new(Expr::TextEq {
                field,
                value: value.clone(),
                ignore_case: level,
            })),
            Check::Contains { needle, .. } => Expr::Contains {
                field,
                needle: needle.clone(),
                negate: false,
            },
            Check::NotContains { needle, .. } => Expr::Contains {
                field,
                needle: needle.clone(),
                negate: true,
            },
            Check::Matches { re, .. } => Expr::Regex {
                field,
                re: re.clone(),
                negate: false,
            },
            Check::Exists { .. } => Expr::Exists(field),
            Check::Missing { .. } => Expr::Missing(field),
            Check::Gt { value, .. } => Expr::NumCmp {
                field,
                op: NumOp::Gt,
                value: *value,
            },
            Check::Lt { value, .. } => Expr::NumCmp {
                field,
                op: NumOp::Lt,
                value: *value,
            },
            Check::In { values, .. } => Expr::TextIn {
                field,
                set: values.clone(),
            },
        }
    }
}

/// A compiled condition: its typed [`Check`] and the expression that evaluates it.
#[derive(Debug, Clone)]
pub struct Condition {
    check: Check,
    expr: Expr,
}

impl Condition {
    pub fn compile(cfg: &ConditionCfg) -> Result<Condition> {
        let op = Op::parse(&cfg.op)?;
        let bool_text;
        let value = match &cfg.value {
            None => Operand::None,
            Some(CondValue::Text(t)) => Operand::Text(t),
            Some(CondValue::Number(n)) => Operand::Number(*n),
            Some(CondValue::Bool(b)) => {
                bool_text = b.to_string();
                Operand::Text(&bool_text)
            }
            Some(CondValue::List(l)) => Operand::List(l),
        };
        Ok(Condition::of(Check::build(&cfg.field, op, value)?))
    }

    /// A condition from text (the grid's field filters): `value` separates `in` values with `|`.
    pub fn new(field: &str, op: &str, value: Option<&str>) -> Result<Condition> {
        let op = Op::parse(op)?;
        let value = value.map_or(Operand::None, Operand::Text);
        Ok(Condition::of(Check::build(field, op, value)?))
    }

    pub fn of(check: Check) -> Condition {
        let expr = check.lower(&mut FieldKeys::new());
        Condition { check, expr }
    }

    pub fn check(&self) -> &Check {
        &self.check
    }

    /// The condition as a query-language expression (for combining and compiling to SQL).
    pub fn expr(&self) -> &Expr {
        &self.expr
    }

    pub fn eval(&self, rec: &Record, source_name: &str) -> bool {
        self.eval_row(&RecordRow { rec, source: source_name })
    }

    pub fn eval_row(&self, row: &dyn RowRef) -> bool {
        self.expr.matches(row)
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
        // `ne` is the negation of `eq`, also for level names.
        assert!(!Condition::new("@level", "ne", Some("WARNING")).unwrap().eval(&r, "f"));
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
        let cond = Condition::new("Tag", "exists", None).unwrap();
        assert_eq!(cond.check().field(), "Tag");
        assert!(matches!(cond.expr(), Expr::Exists(_)));
    }

    #[test]
    fn rejects_bad_conditions() {
        assert!(Condition::new("x", "like", Some("a")).is_err());
        assert!(Condition::new("x", "contains", None).is_err());
        assert!(Condition::new("x", "matches", Some("(")).is_err());
        assert!(Condition::new("x", "gt", Some("ten")).is_err());
        assert!(Condition::new("x", "gt", None).is_err());
        assert!(Condition::new("x", "in", None).is_err());
        assert!(Condition::new("x", "eq", None).is_ok());
        let list = |op: &str| ConditionCfg {
            field: "x".into(),
            op: op.into(),
            value: Some(CondValue::List(vec!["a".into()])),
        };
        let err = Condition::compile(&list("eq")).unwrap_err().to_string();
        assert!(err.contains("needs op = \"in\""), "{err}");
        assert!(Condition::compile(&list("gt")).is_err());
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

    #[test]
    fn list_values_keep_their_bars() {
        // Regression (engine review finding 7): lists were joined with `|` and split again.
        let cfg = ConditionCfg {
            field: "Path".into(),
            op: "in".into(),
            value: Some(CondValue::List(vec!["a|b".into(), "c".into()])),
        };
        let c = Condition::compile(&cfg).unwrap();
        let mut r = Record::new(1);
        r.fields.insert("Path".into(), "a|b".into());
        assert!(c.eval(&r, "f"));
        r.fields.insert("Path".into(), "a".into());
        assert!(!c.eval(&r, "f"));
    }
}
