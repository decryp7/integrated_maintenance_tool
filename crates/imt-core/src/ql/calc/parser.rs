//! Recursive-descent parser for calc expressions: tokens (from [`lex_calc`]) -> [`Node`].
//! Precedence, loosest first: `or`, `and`, `not`, comparisons, `+ -`, `* / %`, unary `-`.

use super::{Arith, Cmp, Lit, Node, FUNCS, MAX_CALC_DEPTH, MAX_CALC_LEN};
use crate::ql::error::QlError;
use crate::ql::lexer::{lex_calc, ArithOp, CmpOp, Tok, Token};
use crate::ql::parser::canonical;
use crate::ql::suggest::closest;

/// A node and the depth of its tree.
type Parsed = (Node, usize);

/// Byte offset and length of a token.
type Span = (usize, usize);

pub(super) fn parse(text: &str) -> Result<Node, QlError> {
    if text.len() > MAX_CALC_LEN {
        return Err(QlError::new(
            format!("expression is too long ({} bytes, at most {MAX_CALC_LEN})", text.len()),
            MAX_CALC_LEN,
            text.len() - MAX_CALC_LEN,
        ));
    }
    let mut p = Parser {
        toks: lex_calc(text)?,
        pos: 0,
        text,
        nest: 0,
    };
    if p.toks.is_empty() {
        return Err(QlError::new("expected an expression", 0, text.len()));
    }
    let (node, _) = p.or()?;
    if let Some(t) = p.toks.get(p.pos) {
        let msg = match t.tok {
            Tok::RParen => "unmatched ')'".to_string(),
            _ => format!("unexpected '{}' (expected an operator)", &text[t.start..t.end]),
        };
        return Err(QlError::new(msg, t.start, t.end - t.start));
    }
    Ok(node)
}

struct Parser<'a> {
    toks: Vec<Token>,
    pos: usize,
    text: &'a str,
    /// Recursion depth of the parser itself (parentheses, unary operators, calls).
    nest: usize,
}

const TOO_DEEP: &str = "expression is nested too deeply";

impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|t| &t.tok)
    }

    /// Consumes the next token and returns its span (the end of the text when there is none).
    fn take(&mut self) -> Span {
        let span = self.toks.get(self.pos).map_or((self.text.len(), 0), |t| (t.start, t.end - t.start));
        self.pos += 1;
        span
    }

    fn err_here(&self, msg: impl Into<String>) -> QlError {
        match self.toks.get(self.pos) {
            Some(t) => QlError::new(msg, t.start, t.end - t.start),
            None => QlError::new(msg, self.text.len(), 0),
        }
    }

    fn keyword(&self, kw: &str) -> bool {
        matches!(self.peek(), Some(Tok::Word(w)) if w.eq_ignore_ascii_case(kw))
    }

    fn enter(&mut self) -> Result<(), QlError> {
        self.nest += 1;
        if self.nest > MAX_CALC_DEPTH {
            return Err(self.err_here(TOO_DEEP));
        }
        Ok(())
    }

    /// Builds a node of tree depth `depth`, refusing trees deeper than the limit.
    fn node(node: Node, depth: usize, at: Span) -> Result<Parsed, QlError> {
        if depth > MAX_CALC_DEPTH {
            return Err(QlError::new(format!("{TOO_DEEP} (at most {MAX_CALC_DEPTH} levels)"), at.0, at.1));
        }
        Ok((node, depth))
    }

    fn or(&mut self) -> Result<Parsed, QlError> {
        self.enter()?;
        let mut l = self.and()?;
        while self.keyword("or") {
            let at = self.take();
            let r = self.and()?;
            l = Self::node(Node::Or(Box::new(l.0), Box::new(r.0)), 1 + l.1.max(r.1), at)?;
        }
        self.nest -= 1;
        Ok(l)
    }

    fn and(&mut self) -> Result<Parsed, QlError> {
        let mut l = self.not()?;
        while self.keyword("and") {
            let at = self.take();
            let r = self.not()?;
            l = Self::node(Node::And(Box::new(l.0), Box::new(r.0)), 1 + l.1.max(r.1), at)?;
        }
        Ok(l)
    }

    fn not(&mut self) -> Result<Parsed, QlError> {
        if !(self.keyword("not") || self.peek() == Some(&Tok::Bang)) {
            return self.cmp();
        }
        let at = self.take();
        self.enter()?;
        let e = self.not()?;
        self.nest -= 1;
        Self::node(Node::Not(Box::new(e.0)), e.1 + 1, at)
    }

    fn cmp(&mut self) -> Result<Parsed, QlError> {
        let l = self.sum()?;
        let Some(Tok::Op(op)) = self.peek().cloned() else {
            return Ok(l);
        };
        let at = self.take();
        let cmp = match op {
            CmpOp::Eq => {
                // Accept `==` as well as `=`.
                if matches!(self.toks.get(self.pos), Some(t) if t.tok == Tok::Op(CmpOp::Eq) && t.start == at.0 + 1) {
                    self.pos += 1;
                }
                Cmp::Eq
            }
            CmpOp::Ne => Cmp::Ne,
            CmpOp::Lt => Cmp::Lt,
            CmpOp::Le => Cmp::Le,
            CmpOp::Gt => Cmp::Gt,
            CmpOp::Ge => Cmp::Ge,
            CmpOp::Re | CmpOp::NotRe => {
                return Err(QlError::new(
                    "regex matching ('~') is not supported in calculated fields",
                    at.0,
                    at.1,
                ));
            }
        };
        let r = self.sum()?;
        if matches!(self.peek(), Some(Tok::Op(_))) {
            return Err(self.err_here("comparisons cannot be chained (combine them with 'and')"));
        }
        Self::node(Node::Cmp(cmp, Box::new(l.0), Box::new(r.0)), 1 + l.1.max(r.1), at)
    }

    fn sum(&mut self) -> Result<Parsed, QlError> {
        let mut l = self.product()?;
        while let Some(Tok::Arith(op @ (ArithOp::Add | ArithOp::Sub))) = self.peek().cloned() {
            let at = self.take();
            let r = self.product()?;
            let op = if op == ArithOp::Add { Arith::Add } else { Arith::Sub };
            l = Self::node(Node::Arith(op, Box::new(l.0), Box::new(r.0)), 1 + l.1.max(r.1), at)?;
        }
        Ok(l)
    }

    fn product(&mut self) -> Result<Parsed, QlError> {
        let mut l = self.unary()?;
        while let Some(Tok::Arith(op @ (ArithOp::Mul | ArithOp::Div | ArithOp::Rem))) = self.peek().cloned() {
            let at = self.take();
            let r = self.unary()?;
            let op = match op {
                ArithOp::Mul => Arith::Mul,
                ArithOp::Div => Arith::Div,
                _ => Arith::Rem,
            };
            l = Self::node(Node::Arith(op, Box::new(l.0), Box::new(r.0)), 1 + l.1.max(r.1), at)?;
        }
        Ok(l)
    }

    fn unary(&mut self) -> Result<Parsed, QlError> {
        let negate = match self.peek() {
            Some(Tok::Arith(ArithOp::Sub)) => true,
            Some(Tok::Arith(ArithOp::Add)) => false,
            _ => return self.primary(),
        };
        let at = self.take();
        self.enter()?;
        let e = self.unary()?;
        self.nest -= 1;
        match e {
            _ if !negate => Ok(e),
            (Node::Lit(Lit::Num(n)), d) => Ok((Node::Lit(Lit::Num(-n)), d)),
            (e, d) => Self::node(Node::Neg(Box::new(e)), d + 1, at),
        }
    }

    fn primary(&mut self) -> Result<Parsed, QlError> {
        let Some(t) = self.toks.get(self.pos).cloned() else {
            return Err(self.err_here("expected a value"));
        };
        self.pos += 1;
        let span = (t.start, t.end - t.start);
        match t.tok {
            Tok::LParen => {
                let e = self.or()?;
                if self.peek() != Some(&Tok::RParen) {
                    return Err(QlError::new("unclosed '(' (expected ')')", t.start, 1));
                }
                self.pos += 1;
                Ok(e)
            }
            Tok::Str(s) => Ok((Node::Lit(Lit::Str(s)), 1)),
            Tok::Tick(name) => Ok((Node::Field(name), 1)),
            Tok::Word(w) => self.word(w, span),
            _ => Err(QlError::new(
                format!("expected a value, found '{}'", &self.text[t.start..t.end]),
                span.0,
                span.1,
            )),
        }
    }

    fn word(&mut self, w: String, span: Span) -> Result<Parsed, QlError> {
        if self.peek() == Some(&Tok::LParen) {
            return self.call(&w, span);
        }
        let lit = match w.to_ascii_lowercase().as_str() {
            "true" => Lit::Bool(true),
            "false" => Lit::Bool(false),
            "null" => Lit::Null,
            kw @ ("and" | "or" | "not") => return Err(QlError::new(format!("expected a value, found '{kw}'"), span.0, span.1)),
            _ if w.starts_with(|c: char| c.is_ascii_digit() || c == '.') => {
                let n = w.parse::<f64>().ok().filter(|n| n.is_finite());
                Lit::Num(n.ok_or_else(|| QlError::new(format!("invalid number '{w}'"), span.0, span.1))?)
            }
            _ => return Ok((Node::Field(canonical(&w)), 1)),
        };
        Ok((Node::Lit(lit), 1))
    }

    fn call(&mut self, name: &str, span: Span) -> Result<Parsed, QlError> {
        let lower = name.to_ascii_lowercase();
        let Some(&(_, func, min, max)) = FUNCS.iter().find(|f| f.0 == lower) else {
            let names: Vec<String> = FUNCS.iter().map(|f| f.0.to_string()).collect();
            let mut e = QlError::new(format!("unknown function '{name}' (use {})", names.join(", ")), span.0, span.1);
            e.suggestions = closest(name, &names);
            return Err(e);
        };
        let open = self.take();
        let mut args = Vec::new();
        let mut depth = 0;
        if self.peek() != Some(&Tok::RParen) {
            loop {
                let (a, d) = self.or()?;
                args.push(a);
                depth = depth.max(d);
                if self.peek() != Some(&Tok::Comma) {
                    break;
                }
                self.pos += 1;
            }
        }
        if self.peek() != Some(&Tok::RParen) {
            return Err(QlError::new("unclosed '(' (expected ')' or ',')", open.0, open.1));
        }
        self.pos += 1;
        if args.len() < min || max.is_some_and(|m| args.len() > m) {
            let want = match max {
                Some(m) if m == min => min.to_string(),
                Some(m) => format!("{min} to {m}"),
                None => format!("at least {min}"),
            };
            let plural = if want.ends_with(" 1") || want == "1" { "" } else { "s" };
            return Err(QlError::new(
                format!("{lower}() takes {want} argument{plural}, got {}", args.len()),
                span.0,
                span.1,
            ));
        }
        Self::node(Node::Call(func, args), depth + 1, span)
    }
}
