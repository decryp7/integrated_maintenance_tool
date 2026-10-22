//! The one predicate language (redesign R2): the query language, profile conditions (`when = [...]`),
//! grid field filters, the search box and the query's time, level, marker, source and instance
//! criteria are all lowered into an [`Expr`] tree, evaluated here over any [`Row`] (an event through
//! [`RowRef`], or a result-table row after `stats`) and compiled to SQL by the store.

use super::lexer::CmpOp;
use crate::model::{value_number, value_text, Level, MarkerMask, Record};
use crate::row::{FieldKey, Pseudo, RecordRow, RowRef};
use regex::Regex;
use serde_json::Value;
use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

/// Something a filter can be evaluated against.
pub trait Row {
    fn text(&self, key: &FieldKey) -> Option<Cow<'_, str>>;
    fn number(&self, key: &FieldKey) -> Option<f64>;
    fn level(&self) -> Option<Level>;
    /// Case-insensitive full-text search; `needle` is already lower-case.
    fn full_text(&self, needle: &str) -> bool;
    fn value(&self, key: &FieldKey) -> Option<Value>;
    /// The event this row is, if it is one (time, source, marker and instance criteria need it).
    fn event(&self) -> Option<&dyn RowRef> {
        None
    }
}

/// An event as a filter row.
#[derive(Clone, Copy)]
pub struct EventRow<'a>(pub &'a dyn RowRef);

/// Whether `message`, `exception` or `template` contains `needle` (lower-case), ignoring case.
pub fn full_text_match(row: &dyn RowRef, needle: &str) -> bool {
    let has = |s: &str| s.to_lowercase().contains(needle);
    has(row.message()) || row.exception().is_some_and(has) || row.template().is_some_and(has)
}

impl Row for EventRow<'_> {
    fn text(&self, key: &FieldKey) -> Option<Cow<'_, str>> {
        self.0.text(key)
    }

    fn number(&self, key: &FieldKey) -> Option<f64> {
        self.0.number(key)
    }

    fn level(&self) -> Option<Level> {
        Some(self.0.level())
    }

    fn full_text(&self, needle: &str) -> bool {
        full_text_match(self.0, needle)
    }

    fn value(&self, key: &FieldKey) -> Option<Value> {
        self.0.value(key)
    }

    fn event(&self) -> Option<&dyn RowRef> {
        Some(self.0)
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

    fn cell(&self, key: &FieldKey) -> Option<&Value> {
        TableRow::column(self.columns, key.name())
            .and_then(|i| self.cells.get(i))
            .filter(|v| !v.is_null())
    }
}

impl Row for TableRow<'_> {
    fn text(&self, key: &FieldKey) -> Option<Cow<'_, str>> {
        self.cell(key).map(value_text)
    }

    fn number(&self, key: &FieldKey) -> Option<f64> {
        self.cell(key).and_then(value_number)
    }

    fn level(&self) -> Option<Level> {
        self.text(&FieldKey::Pseudo(Pseudo::Level)).and_then(|t| Level::parse(&t))
    }

    fn full_text(&self, needle: &str) -> bool {
        self.cells
            .iter()
            .any(|c| matches!(c, Value::String(s) if s.to_lowercase().contains(needle)))
    }

    fn value(&self, key: &FieldKey) -> Option<Value> {
        self.cell(key).cloned()
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

/// A strict numeric comparison of conditions (`gt`, `lt`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumOp {
    Gt,
    Lt,
}

/// The search box over message, exception and template: its words (smart search) and/or a regex (escaped
/// text when not in regex mode) found anywhere.
#[derive(Debug, Clone)]
pub struct TextSearch {
    pub re: Regex,
    /// The trimmed text when the search is not a regex (lets SQL use `LIKE`).
    pub literal: Option<String>,
    pub case_sensitive: bool,
    /// The words an event may hold to match (smart search).
    pub words: Option<std::sync::Arc<crate::words::WordQuery>>,
    /// Whether the text found anywhere (`re`) also matches.
    pub substring: bool,
}

impl TextSearch {
    pub fn matches(&self, row: &dyn RowRef) -> bool {
        self.word_match(row) || (self.substring && self.substring_match(row))
    }

    /// Whether the event holds the search's words (false without words).
    pub fn word_match(&self, row: &dyn RowRef) -> bool {
        self.words.as_ref().is_some_and(|w| {
            let texts: Vec<&str> = [Some(row.message()), row.exception(), row.template()]
                .into_iter()
                .flatten()
                .collect();
            w.matches(&texts)
        })
    }

    fn substring_match(&self, row: &dyn RowRef) -> bool {
        self.re.is_match(row.message())
            || row.exception().is_some_and(|x| self.re.is_match(x))
            || row.template().is_some_and(|t| self.re.is_match(t))
    }
}

/// The predicate tree.
#[derive(Debug, Clone)]
pub enum Expr {
    And(Vec<Expr>),
    Or(Vec<Expr>),
    Not(Box<Expr>),
    /// Matches everything (`true`) or nothing.
    Const(bool),
    /// Query-language comparison: by severity for levels, numerically when both sides are numbers,
    /// else as text. A missing field compares unequal to everything.
    Cmp {
        field: FieldKey,
        op: CmpOp,
        rhs: Operand,
    },
    /// Case-insensitive substring (`needle` is lower-case); a missing field does not contain it.
    Contains {
        field: FieldKey,
        needle: String,
        negate: bool,
    },
    Regex {
        field: FieldKey,
        re: Regex,
        negate: bool,
    },
    /// Query-language `in`: equal (as [`Expr::Cmp`]) to one of the operands.
    In {
        field: FieldKey,
        set: Vec<Operand>,
    },
    /// Present and not empty.
    Exists(FieldKey),
    /// Absent or empty.
    Missing(FieldKey),
    /// Query-language full text over message, exception and template (lower-case needle).
    Text(String),
    /// Exact text equality (conditions); `ignore_case` for level names.
    TextEq {
        field: FieldKey,
        value: String,
        ignore_case: bool,
    },
    /// Exact text equal to one of `set` (conditions).
    TextIn {
        field: FieldKey,
        set: Vec<String>,
    },
    /// Strict numeric comparison (conditions): levels by severity rank; a non-numeric field never matches.
    NumCmp {
        field: FieldKey,
        op: NumOp,
        value: f64,
    },
    /// The search box.
    Search(TextSearch),
    /// The event's level is in the mask (bit = `Level::index`).
    Levels(u8),
    /// The event carries one of the marker kinds in the mask.
    Markers(MarkerMask),
    /// The event comes from one of these source ids.
    Sources(Arc<BTreeSet<u32>>),
    /// Timed events within the inclusive bounds; untimed events never match a bound.
    Time {
        from: Option<i64>,
        to: Option<i64>,
    },
    /// The event's instance key (over `by`, see [`crate::query::group_key_row`]) is one of `set`.
    Group {
        by: Vec<FieldKey>,
        set: Arc<HashSet<String>>,
        /// Where `by`'s identity fields apply.
        scopes: Arc<crate::instance::KeyScopes>,
    },
}

fn compare(row: &dyn Row, field: &FieldKey, rhs: &Operand) -> Option<Ordering> {
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
    /// `parts` joined with `and` (`true` when there are none).
    pub fn all(mut parts: Vec<Expr>) -> Expr {
        parts.retain(|e| !matches!(e, Expr::Const(true)));
        match parts.len() {
            0 => Expr::Const(true),
            1 => parts.remove(0),
            _ => Expr::And(parts),
        }
    }

    pub fn eval(&self, row: &dyn Row) -> bool {
        match self {
            Expr::And(v) => v.iter().all(|e| e.eval(row)),
            Expr::Or(v) => v.iter().any(|e| e.eval(row)),
            Expr::Not(e) => !e.eval(row),
            Expr::Const(b) => *b,
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
            Expr::TextEq { field, value, ignore_case } => row
                .text(field)
                .is_some_and(|s| *s == **value || (*ignore_case && s.eq_ignore_ascii_case(value))),
            Expr::TextIn { field, set } => row.text(field).is_some_and(|s| set.iter().any(|x| *x == *s)),
            Expr::NumCmp { field, op, value } => {
                let n = if *field == FieldKey::Pseudo(Pseudo::Level) {
                    row.level().map(|l| l.index() as f64)
                } else {
                    row.number(field)
                };
                n.is_some_and(|n| match op {
                    NumOp::Gt => n > *value,
                    NumOp::Lt => n < *value,
                })
            }
            Expr::Search(s) => row.event().is_some_and(|e| s.matches(e)),
            Expr::Levels(mask) => row.level().is_some_and(|l| mask & (1 << l.index()) != 0),
            Expr::Markers(mask) => row.event().is_some_and(|e| e.markers() & mask != 0),
            Expr::Sources(set) => row.event().is_some_and(|e| set.contains(&e.source())),
            Expr::Time { from, to } => row.event().is_some_and(|e| match e.ts() {
                Some(ts) => from.is_none_or(|f| ts >= f) && to.is_none_or(|t| ts <= t),
                None => from.is_none() && to.is_none(),
            }),
            Expr::Group { by, set, scopes } => row
                .event()
                .is_some_and(|e| set.contains(&crate::query::group_key_row_in(by, scopes, e))),
        }
    }

    /// Evaluates against an event.
    pub fn matches(&self, row: &dyn RowRef) -> bool {
        self.eval(&EventRow(row))
    }
}

/// A compiled `where` expression. An empty filter matches everything.
#[derive(Debug, Clone)]
pub struct Filter {
    pub expr: Option<Expr>,
}

impl Filter {
    pub fn matches(&self, rec: &Record, source_name: &str) -> bool {
        self.matches_event(&RecordRow { rec, source: source_name })
    }

    /// Evaluates against any event.
    pub fn matches_event(&self, row: &dyn RowRef) -> bool {
        self.matches_row(&EventRow(row))
    }

    pub fn matches_row(&self, row: &dyn Row) -> bool {
        self.expr.as_ref().is_none_or(|e| e.eval(row))
    }
}
