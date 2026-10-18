//! Evaluates a parsed calc expression against one record. Total: every input yields a value,
//! and anything that cannot be computed is [`Val::Null`].

use super::{Arith, Cmp, Func, Lit, Node, MAX_CALC_TEXT};
use crate::model::Record;
use serde_json::Value;
use std::borrow::Cow;
use std::cmp::Ordering;

/// A calc value. Text borrows from the record or the expression where it can.
#[derive(Debug, Clone, PartialEq)]
pub enum Val<'a> {
    Null,
    Bool(bool),
    /// Always finite.
    Num(f64),
    Str(Cow<'a, str>),
}

/// Largest integer an f64 holds exactly (2^53).
const MAX_EXACT_INT: f64 = 9_007_199_254_740_992.0;

impl<'a> Val<'a> {
    /// The value as a number: numbers, and text that reads as a finite number.
    fn num(&self) -> Option<f64> {
        match self {
            Val::Num(n) => Some(*n),
            Val::Str(s) => s.trim().parse::<f64>().ok().filter(|n| n.is_finite()),
            Val::Null | Val::Bool(_) => None,
        }
    }

    /// The value as text (null is empty; callers handle null first).
    pub(super) fn text(&self) -> Cow<'_, str> {
        match self {
            Val::Null => Cow::Borrowed(""),
            Val::Bool(b) => Cow::Borrowed(if *b { "true" } else { "false" }),
            Val::Num(n) => Cow::Owned(format_num(*n)),
            Val::Str(s) => Cow::Borrowed(s),
        }
    }

    fn is_null(&self) -> bool {
        matches!(self, Val::Null)
    }

    /// The value as JSON to store in a field; `None` for null. Whole numbers become integers.
    pub fn into_json(self) -> Option<Value> {
        match self {
            Val::Null => None,
            Val::Bool(b) => Some(Value::Bool(b)),
            Val::Num(n) if n.fract() == 0.0 && n.abs() < MAX_EXACT_INT => Some(Value::from(n as i64)),
            Val::Num(n) => serde_json::Number::from_f64(n).map(Value::Number),
            Val::Str(s) => Some(Value::String(s.into_owned())),
        }
    }
}

fn num(n: f64) -> Val<'static> {
    if n.is_finite() {
        Val::Num(n)
    } else {
        Val::Null
    }
}

fn format_num(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < MAX_EXACT_INT {
        (n as i64).to_string()
    } else {
        n.to_string()
    }
}

fn borrowed(s: Option<&str>) -> Val<'_> {
    s.map_or(Val::Null, |s| Val::Str(Cow::Borrowed(s)))
}

fn field<'a>(name: &str, rec: &'a Record, source: &'a str) -> Val<'a> {
    match name {
        "@level" => Val::Str(Cow::Borrowed(rec.level.as_str())),
        "@message" => borrowed(Some(&rec.message)),
        "@exception" => borrowed(rec.exception.as_deref()),
        "@template" => borrowed(rec.template.as_deref()),
        "@source" => borrowed(Some(source)),
        "@line" => Val::Num(f64::from(rec.line)),
        _ => match rec.fields.get(name) {
            None | Some(Value::Null) => Val::Null,
            Some(Value::Bool(b)) => Val::Bool(*b),
            Some(Value::Number(n)) => n.as_f64().map_or(Val::Null, num),
            Some(Value::String(s)) => borrowed(Some(s)),
            Some(other) => Val::Str(Cow::Owned(other.to_string())),
        },
    }
}

pub(super) fn eval<'a>(node: &'a Node, rec: &'a Record, src: &'a str) -> Val<'a> {
    let ev = |n: &'a Node| eval(n, rec, src);
    match node {
        Node::Lit(Lit::Null) => Val::Null,
        Node::Lit(Lit::Bool(b)) => Val::Bool(*b),
        Node::Lit(Lit::Num(n)) => Val::Num(*n),
        Node::Lit(Lit::Str(s)) => Val::Str(Cow::Borrowed(s)),
        Node::Field(f) => field(f, rec, src),
        Node::Neg(e) => ev(e).num().map_or(Val::Null, |n| num(-n)),
        Node::Not(e) => match ev(e) {
            Val::Bool(b) => Val::Bool(!b),
            _ => Val::Null,
        },
        Node::And(a, b) => logic(ev(a), || ev(b), false),
        Node::Or(a, b) => logic(ev(a), || ev(b), true),
        Node::Arith(op, a, b) => arith(*op, ev(a), ev(b)),
        Node::Cmp(op, a, b) => compare(*op, &ev(a), &ev(b)),
        Node::Call(f, args) => call(*f, args, rec, src),
    }
}

/// `Some(Some(b))` for a boolean, `Some(None)` for null (unknown), `None` for other types.
fn truth(v: &Val) -> Option<Option<bool>> {
    match v {
        Val::Bool(b) => Some(Some(*b)),
        Val::Null => Some(None),
        _ => None,
    }
}

/// Three-valued `and` (`decisive` = false) / `or` (`decisive` = true), short-circuiting on the
/// value that decides the result; non-boolean operands make it null.
fn logic<'a>(a: Val<'a>, b: impl FnOnce() -> Val<'a>, decisive: bool) -> Val<'a> {
    let l = match truth(&a) {
        None => return Val::Null,
        Some(Some(x)) if x == decisive => return Val::Bool(decisive),
        Some(l) => l,
    };
    match (l, truth(&b())) {
        (_, Some(Some(x))) if x == decisive => Val::Bool(decisive),
        (Some(_), Some(Some(_))) => Val::Bool(!decisive),
        _ => Val::Null,
    }
}

fn arith<'a>(op: Arith, a: Val<'a>, b: Val<'a>) -> Val<'a> {
    if a.is_null() || b.is_null() {
        return Val::Null;
    }
    if op == Arith::Add && (matches!(a, Val::Str(_)) || matches!(b, Val::Str(_))) {
        let (x, y) = (a.text(), b.text());
        if x.len() + y.len() > MAX_CALC_TEXT {
            return Val::Null;
        }
        return Val::Str(Cow::Owned(format!("{x}{y}")));
    }
    let (Some(x), Some(y)) = (a.num(), b.num()) else {
        return Val::Null;
    };
    match op {
        Arith::Add => num(x + y),
        Arith::Sub => num(x - y),
        Arith::Mul => num(x * y),
        Arith::Div | Arith::Rem if y == 0.0 => Val::Null,
        Arith::Div => num(x / y),
        Arith::Rem => num(x % y),
    }
}

fn compare<'a>(op: Cmp, a: &Val, b: &Val) -> Val<'a> {
    let ord = match (a, b) {
        (Val::Null, _) | (_, Val::Null) => return Val::Null,
        (Val::Bool(x), Val::Bool(y)) => x.cmp(y),
        _ => match (a.num(), b.num()) {
            // Both finite, so the comparison is total.
            (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
            _ => a.text().cmp(&b.text()),
        },
    };
    Val::Bool(match op {
        Cmp::Eq => ord == Ordering::Equal,
        Cmp::Ne => ord != Ordering::Equal,
        Cmp::Lt => ord == Ordering::Less,
        Cmp::Le => ord != Ordering::Greater,
        Cmp::Gt => ord == Ordering::Greater,
        Cmp::Ge => ord != Ordering::Less,
    })
}

fn call<'a>(f: Func, args: &'a [Node], rec: &'a Record, src: &'a str) -> Val<'a> {
    let arg = |i: usize| args.get(i).map_or(Val::Null, |a| eval(a, rec, src));
    let unary = |op: fn(f64) -> f64| arg(0).num().map_or(Val::Null, |x| num(op(x)));
    let text = |op: fn(&str) -> String| match arg(0) {
        Val::Null => Val::Null,
        v => Val::Str(Cow::Owned(op(&v.text()))),
    };
    match f {
        Func::Round => round(arg(0), (args.len() > 1).then(|| arg(1))),
        Func::Floor => unary(f64::floor),
        Func::Ceil => unary(f64::ceil),
        Func::Abs => unary(f64::abs),
        Func::Min | Func::Max => {
            let mut best: Option<f64> = None;
            for a in args {
                let Some(n) = eval(a, rec, src).num() else {
                    return Val::Null;
                };
                best = Some(match best {
                    None => n,
                    Some(b) if f == Func::Min => b.min(n),
                    Some(b) => b.max(n),
                });
            }
            best.map_or(Val::Null, Val::Num)
        }
        Func::Len => match arg(0) {
            Val::Null => Val::Null,
            v => Val::Num(v.text().chars().count() as f64),
        },
        Func::Lower => text(str::to_lowercase),
        Func::Upper => text(str::to_uppercase),
        Func::Coalesce => args.iter().map(|a| eval(a, rec, src)).find(|v| !v.is_null()).unwrap_or(Val::Null),
        Func::If => match arg(0) {
            Val::Bool(true) => arg(1),
            Val::Bool(false) | Val::Null => arg(2),
            _ => Val::Null,
        },
        Func::Substr => substr(arg(0), arg(1), (args.len() > 2).then(|| arg(2))),
        Func::ToNumber => match arg(0) {
            Val::Bool(b) => Val::Num(if b { 1.0 } else { 0.0 }),
            v => v.num().map_or(Val::Null, Val::Num),
        },
    }
}

/// `round(x[, digits])`: half away from zero; `digits` is truncated and clamped to -15..=15.
fn round<'a>(x: Val, digits: Option<Val>) -> Val<'a> {
    let Some(x) = x.num() else {
        return Val::Null;
    };
    let digits = match digits.map(|d| d.num()) {
        None => 0.0,
        Some(Some(d)) => d.trunc().clamp(-15.0, 15.0),
        Some(None) => return Val::Null,
    };
    let p = 10f64.powi(digits as i32);
    let r = (x * p).round() / p;
    // Huge values are already whole at this precision (x * p overflowed).
    num(if r.is_finite() { r } else { x })
}

/// `substr(s, start[, len])` in characters; a negative `start` counts from the end.
fn substr<'a>(s: Val<'a>, start: Val, len: Option<Val>) -> Val<'a> {
    let Some(start) = start.num().map(f64::trunc).filter(|_| !s.is_null()) else {
        return Val::Null;
    };
    let len = match len.map(|l| l.num()) {
        None => usize::MAX,
        Some(Some(l)) if l >= 0.0 => l as usize,
        Some(_) => return Val::Null,
    };
    let text = s.text();
    let start = if start < 0.0 {
        text.chars().count().saturating_sub(-start as usize)
    } else {
        start as usize
    };
    Val::Str(Cow::Owned(text.chars().skip(start).take(len).collect()))
}
