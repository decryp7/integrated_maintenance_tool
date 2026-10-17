//! Compiled filter expressions and the row abstraction they are evaluated against
//! (event records before `stats`, result-table rows after it).

use super::lexer::CmpOp;
use crate::model::{value_number, value_text, Level, Record};
use regex::Regex;
use serde_json::Value;
use std::borrow::Cow;
use std::cmp::Ordering;

/// Something a filter can be evaluated against.
pub trait Row {
    fn text(&self, field: &str) -> Option<Cow<'_, str>>;
    fn number(&self, field: &str) -> Option<f64>;
    fn level(&self) -> Option<Level>;
    /// Case-insensitive full-text search; `needle` is already lower-case.
    fn full_text(&self, needle: &str) -> bool;
    fn value(&self, field: &str) -> Option<Value>;
}

pub struct EventRow<'a> {
    pub rec: &'a Record,
    pub source: &'a str,
}

impl Row for EventRow<'_> {
    fn text(&self, field: &str) -> Option<Cow<'_, str>> {
        self.rec.field_text(field, self.source)
    }

    fn number(&self, field: &str) -> Option<f64> {
        self.rec.field_number(field)
    }

    fn level(&self) -> Option<Level> {
        Some(self.rec.level)
    }

    fn full_text(&self, needle: &str) -> bool {
        let has = |s: &str| s.to_lowercase().contains(needle);
        has(&self.rec.message) || self.rec.exception.as_deref().is_some_and(has) || self.rec.template.as_deref().is_some_and(has)
    }

    fn value(&self, field: &str) -> Option<Value> {
        match field {
            "@line" => Some(Value::from(self.rec.line)),
            "@level" | "@message" | "@source" | "@exception" | "@template" => self.text(field).map(|t| Value::String(t.into_owned())),
            _ => self.rec.fields.get(field).cloned(),
        }
    }
}

pub struct TableRow<'a> {
    pub columns: &'a [String],
    pub cells: &'a [Value],
}

impl TableRow<'_> {
    /// Index of a column; a pseudo-field such as `@level` also finds a column named `level`.
    pub fn column(columns: &[String], name: &str) -> Option<usize> {
        let find = |n: &str| columns.iter().position(|c| c == n);
        find(name).or_else(|| name.strip_prefix('@').and_then(find))
    }

    fn cell(&self, field: &str) -> Option<&Value> {
        TableRow::column(self.columns, field)
            .and_then(|i| self.cells.get(i))
            .filter(|v| !v.is_null())
    }
}

impl Row for TableRow<'_> {
    fn text(&self, field: &str) -> Option<Cow<'_, str>> {
        self.cell(field).map(value_text)
    }

    fn number(&self, field: &str) -> Option<f64> {
        self.cell(field).and_then(value_number)
    }

    fn level(&self) -> Option<Level> {
        self.text("@level").and_then(|t| Level::parse(&t))
    }

    fn full_text(&self, needle: &str) -> bool {
        self.cells
            .iter()
            .any(|c| matches!(c, Value::String(s) if s.to_lowercase().contains(needle)))
    }

    fn value(&self, field: &str) -> Option<Value> {
        self.cell(field).cloned()
    }
}

/// A literal from the query: bare numbers keep their numeric value, quoted text never does.
#[derive(Debug, Clone)]
pub struct Literal {
    pub text: String,
    pub number: Option<f64>,
}

/// A literal plus, for the level field, the severity it names.
#[derive(Debug, Clone)]
pub struct Operand {
    pub lit: Literal,
    pub level: Option<Level>,
}

#[derive(Debug, Clone)]
pub enum Expr {
    And(Vec<Expr>),
    Or(Vec<Expr>),
    Not(Box<Expr>),
    Cmp { field: String, op: CmpOp, rhs: Operand },
    Contains { field: String, needle: String, negate: bool },
    Regex { field: String, re: Regex, negate: bool },
    In { field: String, set: Vec<Operand> },
    Exists(String),
    Missing(String),
    Text(String),
}

fn compare(row: &dyn Row, field: &str, rhs: &Operand) -> Option<Ordering> {
    if let Some(want) = rhs.level {
        return row.level().map(|l| l.index().cmp(&want.index()));
    }
    let text = row.text(field)?;
    if let (Some(a), Some(b)) = (row.number(field), rhs.lit.number) {
        return Some(a.total_cmp(&b));
    }
    Some((*text).cmp(rhs.lit.text.as_str()))
}

impl Expr {
    pub fn eval(&self, row: &dyn Row) -> bool {
        match self {
            Expr::And(v) => v.iter().all(|e| e.eval(row)),
            Expr::Or(v) => v.iter().any(|e| e.eval(row)),
            Expr::Not(e) => !e.eval(row),
            Expr::Cmp { field, op, rhs } => {
                let ord = compare(row, field, rhs);
                match op {
                    CmpOp::Ne => ord != Some(Ordering::Equal),
                    CmpOp::Gt => ord == Some(Ordering::Greater),
                    CmpOp::Ge => matches!(ord, Some(Ordering::Greater | Ordering::Equal)),
                    CmpOp::Lt => ord == Some(Ordering::Less),
                    CmpOp::Le => matches!(ord, Some(Ordering::Less | Ordering::Equal)),
                    // Eq (the regex operators become Expr::Regex and never reach here).
                    _ => ord == Some(Ordering::Equal),
                }
            }
            Expr::Contains { field, needle, negate } => row.text(field).is_some_and(|t| t.to_lowercase().contains(needle)) != *negate,
            Expr::Regex { field, re, negate } => row.text(field).is_some_and(|t| re.is_match(&t)) != *negate,
            Expr::In { field, set } => set.iter().any(|o| compare(row, field, o) == Some(Ordering::Equal)),
            Expr::Exists(f) => row.text(f).is_some_and(|t| !t.is_empty()),
            Expr::Missing(f) => row.text(f).is_none_or(|t| t.is_empty()),
            Expr::Text(needle) => row.full_text(needle),
        }
    }
}

/// A compiled `where` expression. An empty filter matches everything.
#[derive(Debug, Clone)]
pub struct Filter {
    pub(crate) expr: Option<Expr>,
}

impl Filter {
    pub fn matches(&self, rec: &Record, source_name: &str) -> bool {
        self.matches_row(&EventRow { rec, source: source_name })
    }

    pub(crate) fn matches_row(&self, row: &dyn Row) -> bool {
        self.expr.as_ref().is_none_or(|e| e.eval(row))
    }
}
