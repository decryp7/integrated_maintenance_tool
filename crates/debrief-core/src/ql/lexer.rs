//! Splits query text into tokens with byte spans. Never fails except on unterminated quotes.
//!
//! [`lex`] is the query-language mode; [`lex_calc`] additionally splits on the arithmetic
//! operators `+ - * / %` for calculated-field expressions (see [`super::calc`]).

use super::error::QlError;

/// Arithmetic operators; only produced by [`lex_calc`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArithOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
    Re,
    NotRe,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    /// Bare word (also numbers, keywords and `@pseudo` fields).
    Word(String),
    /// "..." or '...'.
    Str(String),
    /// `...` (a field name, taken literally).
    Tick(String),
    Op(CmpOp),
    Bang,
    LParen,
    RParen,
    Pipe,
    Comma,
    /// `+ - * / %` (calc mode only).
    Arith(ArithOp),
}

#[derive(Debug, Clone)]
pub struct Token {
    pub tok: Tok,
    pub start: usize,
    pub end: usize,
}

fn is_delim(b: u8, arith: bool) -> bool {
    b.is_ascii_whitespace()
        || matches!(
            b,
            b'(' | b')' | b'|' | b',' | b'=' | b'!' | b'<' | b'>' | b'~' | b'"' | b'\'' | b'`'
        )
        || (arith && matches!(b, b'+' | b'-' | b'*' | b'/' | b'%'))
}

/// In calc mode the sign of an exponent (`1e-3`, `2.5E+4`) belongs to the number.
fn exponent_sign(b: &[u8], word_start: usize, j: usize) -> bool {
    matches!(b[j], b'+' | b'-')
        && b[word_start].is_ascii_digit()
        && matches!(b[j - 1], b'e' | b'E')
        && b.get(j + 1).is_some_and(u8::is_ascii_digit)
}

/// Query-language tokens: `+ - * / %` are part of words (`-150ms`, `/api/x`).
pub fn lex(text: &str) -> Result<Vec<Token>, QlError> {
    lex_with(text, false)
}

/// Calculated-field tokens: like [`lex`], but `+ - * / %` are [`Tok::Arith`] operators.
pub fn lex_calc(text: &str) -> Result<Vec<Token>, QlError> {
    lex_with(text, true)
}

fn lex_with(text: &str, arith: bool) -> Result<Vec<Token>, QlError> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let next = b.get(i + 1).copied();
        let (tok, len) = match c {
            b'+' if arith => (Tok::Arith(ArithOp::Add), 1),
            b'-' if arith => (Tok::Arith(ArithOp::Sub), 1),
            b'*' if arith => (Tok::Arith(ArithOp::Mul), 1),
            b'/' if arith => (Tok::Arith(ArithOp::Div), 1),
            b'%' if arith => (Tok::Arith(ArithOp::Rem), 1),
            b'(' => (Tok::LParen, 1),
            b')' => (Tok::RParen, 1),
            b'|' => (Tok::Pipe, 1),
            b',' => (Tok::Comma, 1),
            b'=' => (Tok::Op(CmpOp::Eq), 1),
            b'~' => (Tok::Op(CmpOp::Re), 1),
            b'!' => match next {
                Some(b'=') => (Tok::Op(CmpOp::Ne), 2),
                Some(b'~') => (Tok::Op(CmpOp::NotRe), 2),
                _ => (Tok::Bang, 1),
            },
            b'>' if next == Some(b'=') => (Tok::Op(CmpOp::Ge), 2),
            b'>' => (Tok::Op(CmpOp::Gt), 1),
            b'<' if next == Some(b'=') => (Tok::Op(CmpOp::Le), 2),
            b'<' => (Tok::Op(CmpOp::Lt), 1),
            b'"' | b'\'' => lex_string(text, i)?,
            b'`' => match text[i + 1..].find('`') {
                None => return Err(QlError::new("unterminated field name (missing closing `)", i, text.len() - i)),
                Some(0) => return Err(QlError::new("empty field name", i, 2)),
                Some(n) => (Tok::Tick(text[i + 1..i + 1 + n].to_string()), n + 2),
            },
            _ => {
                let mut j = i;
                while j < b.len() && (!is_delim(b[j], arith) || (arith && exponent_sign(b, i, j))) {
                    j += 1;
                }
                (Tok::Word(text[i..j].to_string()), j - i)
            }
        };
        out.push(Token {
            tok,
            start: i,
            end: i + len,
        });
        i += len;
    }
    Ok(out)
}

/// Quoted string starting at `start`. Only `\"`, `\'` and `\\` are escapes; any other
/// backslash is kept so regexes like "\d+" work unchanged.
fn lex_string(text: &str, start: usize) -> Result<(Tok, usize), QlError> {
    let quote = text.as_bytes()[start];
    let mut s = String::new();
    let mut chars = text[start + 1..].char_indices();
    while let Some((k, c)) = chars.next() {
        if c == '\\' {
            match text[start + 1 + k + 1..].chars().next() {
                Some(n) if n == '"' || n == '\'' || n == '\\' => {
                    s.push(n);
                    chars.next();
                }
                _ => s.push('\\'),
            }
        } else if c as u32 == quote as u32 {
            return Ok((Tok::Str(s), k + 2));
        } else {
            s.push(c);
        }
    }
    Err(QlError::new(
        "unterminated string (missing closing quote)",
        start,
        text.len() - start,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(s: &str) -> Vec<Tok> {
        lex(s).unwrap().into_iter().map(|t| t.tok).collect()
    }

    #[test]
    fn lexes_everything() {
        assert_eq!(
            toks("a>=1 b<=2 c>3 d<4 e!=5 f!~g h~i j=k !x"),
            vec![
                Tok::Word("a".into()),
                Tok::Op(CmpOp::Ge),
                Tok::Word("1".into()),
                Tok::Word("b".into()),
                Tok::Op(CmpOp::Le),
                Tok::Word("2".into()),
                Tok::Word("c".into()),
                Tok::Op(CmpOp::Gt),
                Tok::Word("3".into()),
                Tok::Word("d".into()),
                Tok::Op(CmpOp::Lt),
                Tok::Word("4".into()),
                Tok::Word("e".into()),
                Tok::Op(CmpOp::Ne),
                Tok::Word("5".into()),
                Tok::Word("f".into()),
                Tok::Op(CmpOp::NotRe),
                Tok::Word("g".into()),
                Tok::Word("h".into()),
                Tok::Op(CmpOp::Re),
                Tok::Word("i".into()),
                Tok::Word("j".into()),
                Tok::Op(CmpOp::Eq),
                Tok::Word("k".into()),
                Tok::Bang,
                Tok::Word("x".into()),
            ]
        );
        assert_eq!(toks("(|,)"), vec![Tok::LParen, Tok::Pipe, Tok::Comma, Tok::RParen]);
    }

    #[test]
    fn strings_and_ticks() {
        assert_eq!(
            toks(r#""a \" b" 'c\'d' "\d+" "x\\""#),
            vec![
                Tok::Str("a \" b".into()),
                Tok::Str("c'd".into()),
                Tok::Str("\\d+".into()),
                Tok::Str("x\\".into()),
            ]
        );
        assert_eq!(toks("`odd name` é"), vec![Tok::Tick("odd name".into()), Tok::Word("é".into())]);
    }

    #[test]
    fn calc_mode_splits_arithmetic() {
        let calc = |s: &str| lex_calc(s).unwrap().into_iter().map(|t| t.tok).collect::<Vec<_>>();
        let w = |s: &str| Tok::Word(s.into());
        assert_eq!(
            calc("a+b-c*d/e%f"),
            vec![
                w("a"),
                Tok::Arith(ArithOp::Add),
                w("b"),
                Tok::Arith(ArithOp::Sub),
                w("c"),
                Tok::Arith(ArithOp::Mul),
                w("d"),
                Tok::Arith(ArithOp::Div),
                w("e"),
                Tok::Arith(ArithOp::Rem),
                w("f"),
            ]
        );
        // Exponent signs stay in numbers; `e-` after a name or without digits does not.
        assert_eq!(calc("1e-3 2E+4"), vec![w("1e-3"), w("2E+4")]);
        assert_eq!(calc("be-3"), vec![w("be"), Tok::Arith(ArithOp::Sub), w("3")]);
        assert_eq!(calc("1e-x"), vec![w("1e"), Tok::Arith(ArithOp::Sub), w("x")]);
        assert_eq!(calc("1e-"), vec![w("1e"), Tok::Arith(ArithOp::Sub)]);
        // The query language keeps them in words.
        assert_eq!(toks("-150ms /api/x"), vec![w("-150ms"), w("/api/x")]);
    }

    #[test]
    fn lex_errors() {
        let e = lex("a \"open").unwrap_err();
        assert_eq!((e.offset, e.len), (2, 5));
        let e = lex("a `open").unwrap_err();
        assert_eq!((e.offset, e.len), (2, 5));
        let e = lex("``").unwrap_err();
        assert_eq!((e.offset, e.len), (0, 2));
        assert!(lex("\"a\\").is_err());
    }
}
