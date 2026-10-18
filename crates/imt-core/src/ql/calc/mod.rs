//! Calculated-field expressions, as used by the pipeline's `calc` step:
//!
//! ```text
//! ElapsedMs / 1000
//! round(Bytes / 1024 / 1024, 1)
//! if(StatusCode >= 500, "server", if(StatusCode >= 400, "client", "ok"))
//! lower(MachineName) + ":" + `process id`
//! ```
//!
//! Grammar (keywords and function names are case-insensitive):
//!
//! ```text
//! expr    := or ;  or := and ('or' and)* ;  and := not ('and' not)*
//! not     := ('not' | '!') not | cmp
//! cmp     := sum [('=' | '==' | '!=' | '<' | '<=' | '>' | '>=') sum]
//! sum     := product (('+' | '-') product)*
//! product := unary (('*' | '/' | '%') unary)*
//! unary   := ('-' | '+') unary | primary
//! primary := number | "string" | 'string' | true | false | null
//!          | field | fn '(' [expr (',' expr)*] ')' | '(' expr ')'
//! field   := identifier | @pseudo | `quoted name`   (same names as the query language)
//! ```
//!
//! Evaluation never fails: a missing field, a type mismatch, a division by zero or a
//! non-finite result yields null, and a null result writes no field. Parsing happens once
//! (when the profile is compiled); errors carry the byte span they refer to.

mod eval;
mod parser;

#[cfg(test)]
mod tests;

pub use eval::Val;

use super::error::QlError;
use crate::model::Record;
use serde_json::Value;

/// Longest accepted expression, in bytes.
pub const MAX_CALC_LEN: usize = 4096;
/// Deepest accepted expression tree (operators and calls nested in each other).
pub const MAX_CALC_DEPTH: usize = super::parser::MAX_DEPTH;
/// Longest text an expression may produce; longer results become null.
pub const MAX_CALC_TEXT: usize = 1 << 20;

/// A parsed, ready-to-evaluate expression.
#[derive(Debug, Clone)]
pub struct CalcExpr {
    root: Node,
}

/// Comparison operators of calc expressions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cmp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Arith {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

/// The built-in functions; `arity` is checked when parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Func {
    Round,
    Floor,
    Ceil,
    Abs,
    Min,
    Max,
    Len,
    Lower,
    Upper,
    Coalesce,
    If,
    Substr,
    ToNumber,
}

/// Name, function, minimum and maximum number of arguments (`None` = any number).
const FUNCS: [(&str, Func, usize, Option<usize>); 13] = [
    ("round", Func::Round, 1, Some(2)),
    ("floor", Func::Floor, 1, Some(1)),
    ("ceil", Func::Ceil, 1, Some(1)),
    ("abs", Func::Abs, 1, Some(1)),
    ("min", Func::Min, 1, None),
    ("max", Func::Max, 1, None),
    ("len", Func::Len, 1, Some(1)),
    ("lower", Func::Lower, 1, Some(1)),
    ("upper", Func::Upper, 1, Some(1)),
    ("coalesce", Func::Coalesce, 1, None),
    ("if", Func::If, 3, Some(3)),
    ("substr", Func::Substr, 2, Some(3)),
    ("to_number", Func::ToNumber, 1, Some(1)),
];

#[derive(Debug, Clone, PartialEq)]
enum Lit {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
}

#[derive(Debug, Clone)]
enum Node {
    Lit(Lit),
    /// Canonical field name (`level` -> `@level`).
    Field(String),
    Neg(Box<Node>),
    Not(Box<Node>),
    And(Box<Node>, Box<Node>),
    Or(Box<Node>, Box<Node>),
    Arith(Arith, Box<Node>, Box<Node>),
    Cmp(Cmp, Box<Node>, Box<Node>),
    Call(Func, Vec<Node>),
}

impl CalcExpr {
    /// Parses `text`; the error points at the offending bytes.
    pub fn parse(text: &str) -> Result<CalcExpr, QlError> {
        parser::parse(text).map(|root| CalcExpr { root })
    }

    /// The value of the expression for one record (`source_name` answers `@source`).
    pub fn eval<'a>(&'a self, rec: &'a Record, source_name: &'a str) -> Val<'a> {
        eval::eval(&self.root, rec, source_name)
    }

    /// The value as JSON to store in a field; `None` for null (nothing to write).
    /// Whole numbers become integers so they facet and display like extracted counts.
    pub fn eval_json(&self, rec: &Record, source_name: &str) -> Option<Value> {
        self.eval(rec, source_name).into_json()
    }
}
