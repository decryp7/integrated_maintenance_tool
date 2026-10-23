//! Recursive-descent parser: tokens -> [`Program`]. Precedence: `not` > `and` (explicit or
//! implicit between adjacent terms) > `or`.

use super::error::QlError;
use super::expr::{Expr, Filter, Literal, Operand};
use super::lexer::{lex, CmpOp, Tok, Token};
use crate::model::Level;
use crate::row::{FieldKey, FieldKeys};
use crate::user_regex;

pub(super) const MAX_DEPTH: usize = 64;

/// A parsed query: a filter over events followed by pipeline stages.
#[derive(Debug, Clone)]
pub struct Program {
    pub filter: Filter,
    pub stages: Vec<Stage>,
    /// Every field the query names, in order, with the part it is in: 0 for the leading filter,
    /// `i + 1` for `stages[i]` (used to check names against the data, see [`super::unknown_field`]).
    pub field_refs: Vec<(usize, FieldRef)>,
    /// The field names the query's expressions and stages refer to, interned.
    pub keys: FieldKeys,
}

/// A field named in the query, with its source span for error reporting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldRef {
    /// Canonical name (`level` -> `@level`).
    pub name: String,
    /// `name`, interned in the program's keys.
    pub key: FieldKey,
    /// The name as written; used as the result column name.
    pub label: String,
    pub offset: usize,
    pub len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AggFn {
    Count,
    CountDistinct,
    Avg,
    Min,
    Max,
    Sum,
    /// Percentile in 0..=100.
    Percentile(f64),
}

#[derive(Debug, Clone)]
pub struct AggSpec {
    pub func: AggFn,
    pub field: Option<FieldRef>,
    /// Result column name: the alias, or e.g. `count()` / `avg(latency)`.
    pub column: String,
    pub offset: usize,
    pub len: usize,
}

#[derive(Debug, Clone)]
pub struct SortKeySpec {
    pub field: FieldRef,
    pub descending: bool,
}

#[derive(Debug, Clone)]
pub enum Stage {
    Where(Filter),
    Stats { aggs: Vec<AggSpec>, by: Vec<FieldRef> },
    Sort(Vec<SortKeySpec>),
    Head(usize),
    Fields(Vec<FieldRef>),
}

/// Maps the friendly aliases onto the pseudo-fields.
pub(super) fn canonical(word: &str) -> String {
    match word.to_ascii_lowercase().as_str() {
        "level" => "@level".into(),
        "message" => "@message".into(),
        "exception" => "@exception".into(),
        "source" => "@source".into(),
        _ => word.to_string(),
    }
}

pub fn parse(text: &str) -> Result<Program, QlError> {
    let mut p = Parser {
        toks: lex(text)?,
        pos: 0,
        text,
        depth: 0,
        part: 0,
        refs: Vec::new(),
        keys: FieldKeys::new(),
    };
    let filter = if matches!(p.peek(), None | Some(Tok::Pipe)) {
        Filter { expr: None }
    } else {
        p.filter()?
    };
    p.expect_stage_end()?;
    let mut stages = Vec::new();
    while p.peek() == Some(&Tok::Pipe) {
        p.pos += 1;
        p.part = stages.len() + 1;
        stages.push(p.stage()?);
        p.expect_stage_end()?;
    }
    Ok(Program {
        filter,
        stages,
        field_refs: p.refs,
        keys: p.keys,
    })
}

struct Parser<'a> {
    toks: Vec<Token>,
    pos: usize,
    text: &'a str,
    depth: usize,
    /// The query part being parsed (see `Program::field_refs`).
    part: usize,
    refs: Vec<(usize, FieldRef)>,
    keys: FieldKeys,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|t| &t.tok)
    }

    /// Remembers a field the query names (filters name fields without going through `field_ref`).
    fn note(&mut self, name: &str, label: &str, t: &Token) {
        let r = FieldRef {
            name: name.to_string(),
            key: self.keys.key(name),
            label: label.to_string(),
            offset: t.start,
            len: t.end - t.start,
        };
        self.refs.push((self.part, r));
    }

    fn peek_at(&self, n: usize) -> Option<&Tok> {
        self.toks.get(self.pos + n).map(|t| &t.tok)
    }

    fn next(&mut self) -> Option<Token> {
        let t = self.toks.get(self.pos).cloned();
        self.pos += t.is_some() as usize;
        t
    }

    /// Error at the next token, or at the end of the text when there is none.
    fn err_here(&self, msg: impl Into<String>) -> QlError {
        match self.toks.get(self.pos) {
            Some(t) => QlError::new(msg, t.start, t.end - t.start),
            None => QlError::new(msg, self.text.len(), 0),
        }
    }

    fn err_tok(t: &Token, msg: impl Into<String>) -> QlError {
        QlError::new(msg, t.start, t.end - t.start)
    }

    fn word_is(&self, n: usize, kw: &str) -> bool {
        matches!(self.peek_at(n), Some(Tok::Word(w)) if w.eq_ignore_ascii_case(kw))
    }

    fn expect_stage_end(&self) -> Result<(), QlError> {
        match self.peek() {
            None | Some(Tok::Pipe) => Ok(()),
            Some(Tok::RParen) => Err(self.err_here("unmatched ')'")),
            Some(_) => {
                let t = &self.toks[self.pos];
                Err(self.err_here(format!("unexpected '{}'", &self.text[t.start..t.end])))
            }
        }
    }

    fn filter(&mut self) -> Result<Filter, QlError> {
        Ok(Filter { expr: Some(self.or()?) })
    }

    fn enter(&mut self) -> Result<(), QlError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.err_here("expression is nested too deeply"));
        }
        Ok(())
    }

    fn or(&mut self) -> Result<Expr, QlError> {
        self.enter()?;
        let mut parts = vec![self.and()?];
        while self.word_is(0, "or") {
            self.pos += 1;
            parts.push(self.and()?);
        }
        self.depth -= 1;
        Ok(if parts.len() == 1 { parts.remove(0) } else { Expr::Or(parts) })
    }

    fn starts_term(&self) -> bool {
        match self.peek() {
            Some(Tok::LParen | Tok::Str(_) | Tok::Tick(_)) => true,
            Some(Tok::Word(w)) => !w.eq_ignore_ascii_case("or"),
            _ => false,
        }
    }

    fn and(&mut self) -> Result<Expr, QlError> {
        let mut parts = vec![self.not()?];
        loop {
            if self.word_is(0, "and") {
                self.pos += 1;
            } else if !self.starts_term() {
                break;
            }
            parts.push(self.not()?);
        }
        Ok(if parts.len() == 1 { parts.remove(0) } else { Expr::And(parts) })
    }

    fn not(&mut self) -> Result<Expr, QlError> {
        if self.word_is(0, "not") {
            self.pos += 1;
            self.enter()?;
            let e = self.not()?;
            self.depth -= 1;
            return Ok(Expr::Not(Box::new(e)));
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Expr, QlError> {
        let Some(t) = self.next() else {
            return Err(self.err_here("expected an expression"));
        };
        match &t.tok {
            Tok::LParen => {
                let e = self.or()?;
                if self.peek() != Some(&Tok::RParen) {
                    return Err(Self::err_tok(&t, "unclosed '(' (expected ')')"));
                }
                self.pos += 1;
                Ok(e)
            }
            Tok::Str(s) => Ok(Expr::Text(s.to_lowercase())),
            Tok::Tick(name) => {
                if self.is_comparison_next() {
                    self.note(name, name, &t);
                    self.comparison(name.clone(), &t)
                } else {
                    Err(self.err_here(format!("expected an operator after field `{name}`")))
                }
            }
            Tok::Word(w) => self.word_term(w, &t),
            _ => Err(Self::err_tok(&t, "expected an expression")),
        }
    }

    fn word_term(&mut self, w: &str, t: &Token) -> Result<Expr, QlError> {
        let lower = w.to_ascii_lowercase();
        if lower == "and" || lower == "or" {
            return Err(Self::err_tok(t, format!("expected an expression before '{lower}'")));
        }
        let adjacent_paren = self.toks.get(self.pos).is_some_and(|n| n.tok == Tok::LParen && n.start == t.end);
        if adjacent_paren {
            return self.function(&lower, t);
        }
        if self.is_comparison_next() {
            self.note(&canonical(w), w, t);
            return self.comparison(canonical(w), t);
        }
        Ok(Expr::Text(w.to_lowercase()))
    }

    fn function(&mut self, name: &str, t: &Token) -> Result<Expr, QlError> {
        if name != "exists" && name != "missing" {
            return Err(Self::err_tok(
                t,
                format!("unknown function '{name}' (use exists(field) or missing(field))"),
            ));
        }
        let open = self.next().unwrap_or_else(|| t.clone());
        let field = self.field_ref()?;
        if self.peek() != Some(&Tok::RParen) {
            return Err(Self::err_tok(&open, "unclosed '(' (expected ')')"));
        }
        self.pos += 1;
        Ok(if name == "exists" {
            Expr::Exists(field.key)
        } else {
            Expr::Missing(field.key)
        })
    }

    fn is_comparison_next(&self) -> bool {
        match self.peek() {
            Some(Tok::Op(_) | Tok::Bang) => true,
            Some(Tok::Word(w)) => w.eq_ignore_ascii_case("contains") || w.eq_ignore_ascii_case("in"),
            _ => false,
        }
    }

    fn literal(&mut self) -> Result<Literal, QlError> {
        match self.next() {
            Some(Token { tok: Tok::Word(w), .. }) => Ok(Literal {
                number: w.parse::<f64>().ok().filter(|n| n.is_finite()),
                text: w,
            }),
            Some(Token { tok: Tok::Str(s), .. }) => Ok(Literal { text: s, number: None }),
            Some(t) => Err(Self::err_tok(&t, "expected a value")),
            None => Err(self.err_here("expected a value")),
        }
    }

    /// A literal prepared for comparison with `field`; level names are resolved here.
    fn operand(&mut self, field: &str) -> Result<Operand, QlError> {
        let start = self.pos;
        let lit = self.literal()?;
        let level = if field == "@level" {
            let l = Level::parse(&lit.text).ok_or_else(|| {
                let t = &self.toks[start];
                Self::err_tok(
                    t,
                    format!(
                        "unknown level '{}' (use verbose, debug, information, warning, error, fatal)",
                        lit.text
                    ),
                )
            })?;
            Some(l)
        } else {
            None
        };
        Ok(Operand { lit, level })
    }

    fn comparison(&mut self, name: String, field_tok: &Token) -> Result<Expr, QlError> {
        // Callers checked that an operator follows; falling back to the field token keeps this total.
        let t = self.next().unwrap_or_else(|| field_tok.clone());
        let field = self.keys.key(&name);
        match t.tok {
            Tok::Op(CmpOp::Re | CmpOp::NotRe) => {
                let negate = t.tok == Tok::Op(CmpOp::NotRe);
                let start = self.pos;
                let lit = self.literal()?;
                let re =
                    user_regex::compile(&lit.text, false).map_err(|e| Self::err_tok(&self.toks[start], format!("invalid regex: {e}")))?;
                Ok(Expr::Regex { field, re, negate })
            }
            Tok::Op(op) => {
                let rhs = self.operand(&name)?;
                Ok(Expr::Cmp { field, op, rhs })
            }
            Tok::Bang => {
                if !self.word_is(0, "contains") {
                    return Err(Self::err_tok(&t, "expected 'contains' after '!'"));
                }
                self.pos += 1;
                let needle = self.literal()?.text.to_lowercase();
                Ok(Expr::Contains {
                    field,
                    needle,
                    negate: true,
                })
            }
            Tok::Word(w) if w.eq_ignore_ascii_case("contains") => {
                let needle = self.literal()?.text.to_lowercase();
                Ok(Expr::Contains {
                    field,
                    needle,
                    negate: false,
                })
            }
            // `in`: the only other word is_comparison_next() lets through.
            _ => self.in_list(field, &name, &t),
        }
    }

    fn in_list(&mut self, field: FieldKey, name: &str, in_tok: &Token) -> Result<Expr, QlError> {
        let Some(open) = self.next().filter(|t| t.tok == Tok::LParen) else {
            return Err(Self::err_tok(in_tok, "expected '(' after 'in'"));
        };
        let mut set = vec![self.operand(name)?];
        while self.peek() == Some(&Tok::Comma) {
            self.pos += 1;
            set.push(self.operand(name)?);
        }
        if self.peek() != Some(&Tok::RParen) {
            return Err(Self::err_tok(&open, "unclosed '(' (expected ')')"));
        }
        self.pos += 1;
        Ok(Expr::In { field, set })
    }

    fn field_ref(&mut self) -> Result<FieldRef, QlError> {
        let r = self.field_ref_unnoted()?;
        self.refs.push((self.part, r.clone()));
        Ok(r)
    }

    fn field_ref_unnoted(&mut self) -> Result<FieldRef, QlError> {
        match self.next() {
            Some(Token {
                tok: Tok::Word(w),
                start,
                end,
            }) => Ok(FieldRef {
                key: self.keys.key(&canonical(&w)),
                name: canonical(&w),
                label: w,
                offset: start,
                len: end - start,
            }),
            Some(Token {
                tok: Tok::Tick(w),
                start,
                end,
            }) => Ok(FieldRef {
                key: self.keys.key(&w),
                name: w.clone(),
                label: w,
                offset: start,
                len: end - start,
            }),
            Some(t) => Err(Self::err_tok(&t, "expected a field name")),
            None => Err(self.err_here("expected a field name")),
        }
    }

    fn field_list(&mut self) -> Result<Vec<FieldRef>, QlError> {
        let mut v = vec![self.field_ref()?];
        while self.peek() == Some(&Tok::Comma) {
            self.pos += 1;
            v.push(self.field_ref()?);
        }
        Ok(v)
    }

    fn stage(&mut self) -> Result<Stage, QlError> {
        let Some(Token { tok: Tok::Word(name), .. }) = self.toks.get(self.pos).cloned() else {
            return Err(self.err_here("expected a stage name (where, stats, sort, head, limit, fields)"));
        };
        let lower = name.to_ascii_lowercase();
        let first = self.pos;
        self.pos += 1;
        match lower.as_str() {
            "where" => Ok(Stage::Where(self.filter()?)),
            "stats" => self.stats(),
            "sort" => self.sort(),
            "head" | "limit" => match self.next() {
                Some(Token {
                    tok: Tok::Word(n),
                    start,
                    end,
                }) => n
                    .parse::<usize>()
                    .map(Stage::Head)
                    .map_err(|_| QlError::new(format!("'{lower}' needs a non-negative whole number"), start, end - start)),
                Some(t) => Err(Self::err_tok(&t, format!("'{lower}' needs a number"))),
                None => Err(self.err_here(format!("'{lower}' needs a number"))),
            },
            "fields" => Ok(Stage::Fields(self.field_list()?)),
            _ => Err(Self::err_tok(
                &self.toks[first],
                format!("unknown stage '{name}' (use where, stats, sort, head, limit, fields)"),
            )),
        }
    }

    fn sort(&mut self) -> Result<Stage, QlError> {
        let mut keys = Vec::new();
        loop {
            let field = self.field_ref()?;
            let descending = if self.word_is(0, "desc") {
                self.pos += 1;
                true
            } else {
                if self.word_is(0, "asc") {
                    self.pos += 1;
                }
                false
            };
            keys.push(SortKeySpec { field, descending });
            if self.peek() != Some(&Tok::Comma) {
                return Ok(Stage::Sort(keys));
            }
            self.pos += 1;
        }
    }

    fn stats(&mut self) -> Result<Stage, QlError> {
        let mut aggs = vec![self.agg()?];
        while self.peek() == Some(&Tok::Comma) {
            self.pos += 1;
            aggs.push(self.agg()?);
        }
        let mut by = Vec::new();
        if self.word_is(0, "by") {
            self.pos += 1;
            by = self.field_list()?;
        }
        Ok(Stage::Stats { aggs, by })
    }

    fn agg(&mut self) -> Result<AggSpec, QlError> {
        let Some(Token {
            tok: Tok::Word(name),
            start,
            end,
        }) = self.toks.get(self.pos).cloned()
        else {
            return Err(self.err_here("expected an aggregate such as count() or avg(field)"));
        };
        self.pos += 1;
        let lower = name.to_ascii_lowercase();
        let func = match lower.as_str() {
            "count" => AggFn::Count,
            "count_distinct" => AggFn::CountDistinct,
            "avg" => AggFn::Avg,
            "min" => AggFn::Min,
            "max" => AggFn::Max,
            "sum" => AggFn::Sum,
            "p50" => AggFn::Percentile(50.0),
            "p95" => AggFn::Percentile(95.0),
            "p99" => AggFn::Percentile(99.0),
            _ => {
                return Err(QlError::new(
                    format!("unknown function '{name}' (use count, count_distinct, avg, min, max, sum, p50, p95, p99)"),
                    start,
                    end - start,
                ))
            }
        };
        let Some(open) = self.next().filter(|t| t.tok == Tok::LParen) else {
            return Err(QlError::new(format!("expected '(' after '{lower}'"), start, end - start));
        };
        let field = if func == AggFn::Count { None } else { Some(self.field_ref()?) };
        if self.peek() != Some(&Tok::RParen) {
            return Err(Self::err_tok(&open, "unclosed '(' (expected ')'; count() takes no arguments)"));
        }
        let close = self.toks[self.pos].end;
        self.pos += 1;
        let default = match &field {
            Some(f) => format!("{lower}({})", f.label),
            None => "count()".to_string(),
        };
        let column = if self.word_is(0, "as") {
            self.pos += 1;
            match self.next() {
                Some(Token {
                    tok: Tok::Word(a) | Tok::Str(a) | Tok::Tick(a),
                    ..
                }) => a,
                Some(t) => return Err(Self::err_tok(&t, "expected an alias name after 'as'")),
                None => return Err(self.err_here("expected an alias name after 'as'")),
            }
        } else {
            default
        };
        Ok(AggSpec {
            func,
            field,
            column,
            offset: start,
            len: close - start,
        })
    }
}
