//! Context-aware editor suggestions. Works on the tokens before the cursor, so it is tolerant of
//! half-written queries.

use super::lexer::{lex, Tok, Token};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CompletionKind {
    Field,
    Keyword,
    Operator,
    Function,
    Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Completion {
    pub label: String,
    pub kind: CompletionKind,
    /// Text that replaces the word being typed.
    pub insert: String,
}

const STAGES: [&str; 6] = ["where", "stats", "sort", "head", "limit", "fields"];
const AGGS: [&str; 9] = ["count", "count_distinct", "avg", "min", "max", "sum", "p50", "p95", "p99"];
const OPERATORS: [&str; 10] = ["=", "!=", ">", ">=", "<", "<=", "~", "!~", "contains", "in"];
const PSEUDO: [&str; 10] = [
    "@level",
    "@message",
    "@exception",
    "@template",
    "@source",
    "@line",
    "level",
    "message",
    "exception",
    "source",
];
const LEVELS: [&str; 6] = ["verbose", "debug", "information", "warning", "error", "fatal"];

fn item(label: &str, kind: CompletionKind) -> Completion {
    let insert = match kind {
        CompletionKind::Function if label == "count" => "count()".to_string(),
        CompletionKind::Function => format!("{label}("),
        _ => label.to_string(),
    };
    Completion {
        label: label.to_string(),
        kind,
        insert,
    }
}

fn items(labels: &[&str], kind: CompletionKind) -> Vec<Completion> {
    labels.iter().map(|l| item(l, kind)).collect()
}

fn fields(names: &[String]) -> Vec<Completion> {
    let mut v = items(&PSEUDO, CompletionKind::Field);
    v.extend(names.iter().map(|n| {
        let label = if n.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-')) {
            n.clone()
        } else {
            format!("`{n}`")
        };
        Completion {
            insert: label.clone(),
            label,
            kind: CompletionKind::Field,
        }
    }));
    v
}

fn filter_start(names: &[String]) -> Vec<Completion> {
    let mut v = fields(names);
    v.push(item("not", CompletionKind::Keyword));
    v.extend(items(&["exists", "missing"], CompletionKind::Function));
    v
}

fn is_word(t: &Token, kw: &str) -> bool {
    matches!(&t.tok, Tok::Word(w) if w.eq_ignore_ascii_case(kw))
}

fn is_field_token(t: &Token) -> bool {
    matches!(&t.tok, Tok::Tick(_))
        || matches!(&t.tok, Tok::Word(w) if !["and", "or", "not", "contains", "in", "by", "as", "asc", "desc"].iter().any(|k| w.eq_ignore_ascii_case(k)))
}

fn level_values(field: Option<&Token>) -> Vec<Completion> {
    let is_level = field.is_some_and(|t| matches!(&t.tok, Tok::Word(w) if matches!(w.to_ascii_lowercase().as_str(), "level" | "@level")));
    if is_level {
        items(&LEVELS, CompletionKind::Value)
    } else {
        Vec::new()
    }
}

fn after_term() -> Vec<Completion> {
    let mut v = items(&["and", "or"], CompletionKind::Keyword);
    v.push(item("|", CompletionKind::Operator));
    v
}

/// Suggestions for a filter expression given the tokens of the expression so far.
fn filter_context(toks: &[Token], names: &[String]) -> Vec<Completion> {
    let Some(last) = toks.last() else {
        return filter_start(names);
    };
    let prev = toks.len().checked_sub(2).map(|i| &toks[i]);
    let prev2 = toks.len().checked_sub(3).map(|i| &toks[i]);
    match &last.tok {
        Tok::Op(_) => level_values(prev),
        Tok::Word(w) if w.eq_ignore_ascii_case("contains") && prev.is_some_and(is_field_token) => level_values(prev),
        Tok::Word(w) if w.eq_ignore_ascii_case("in") && prev.is_some_and(is_field_token) => vec![item("(", CompletionKind::Operator)],
        Tok::Bang => vec![item("contains", CompletionKind::Operator)],
        Tok::LParen if prev.is_some_and(|p| is_word(p, "in")) => level_values(prev2),
        Tok::LParen if prev.is_some_and(|p| is_word(p, "exists") || is_word(p, "missing")) => fields(names),
        Tok::Comma => {
            // Inside `in (...)`: the field is the token before the nearest `in`.
            let field = toks
                .iter()
                .rposition(|t| is_word(t, "in"))
                .and_then(|i| i.checked_sub(1))
                .map(|i| &toks[i]);
            level_values(field)
        }
        Tok::RParen => after_term(),
        Tok::Word(w) if ["and", "or", "not"].iter().any(|k| w.eq_ignore_ascii_case(k)) => filter_start(names),
        Tok::Str(_) => after_term(),
        // A word or `field` that follows an operator is a finished value.
        _ if prev.is_some_and(|p| matches!(p.tok, Tok::Op(_)) || is_word(p, "contains")) => after_term(),
        _ if is_field_token(last) => {
            let mut v = items(&OPERATORS, CompletionKind::Operator);
            v.push(item("!contains", CompletionKind::Operator));
            v.extend(items(&["and", "or"], CompletionKind::Keyword));
            v
        }
        _ => filter_start(names),
    }
}

fn stats_context(toks: &[Token], names: &[String]) -> Vec<Completion> {
    let in_by = toks.iter().any(|t| is_word(t, "by"));
    let open = toks.iter().filter(|t| t.tok == Tok::LParen).count() > toks.iter().filter(|t| t.tok == Tok::RParen).count();
    let Some(last) = toks.last() else {
        return items(&AGGS, CompletionKind::Function);
    };
    if open {
        return fields(names);
    }
    match &last.tok {
        Tok::Comma if in_by => fields(names),
        Tok::Comma => items(&AGGS, CompletionKind::Function),
        Tok::RParen => items(&["as", "by"], CompletionKind::Keyword),
        _ if is_word(last, "by") => fields(names),
        _ if is_word(last, "as") => Vec::new(),
        _ if in_by => Vec::new(),
        _ => items(&["by"], CompletionKind::Keyword),
    }
}

fn field_list_context(toks: &[Token], names: &[String], sorting: bool) -> Vec<Completion> {
    match toks.last().map(|t| &t.tok) {
        None | Some(Tok::Comma) => fields(names),
        Some(_) if sorting && toks.last().is_some_and(is_field_token) => items(&["asc", "desc"], CompletionKind::Keyword),
        _ => Vec::new(),
    }
}

/// Suggestions at byte `cursor` of `text`. `field_names` are the custom fields known to the workspace.
pub fn completions(text: &str, cursor: usize, field_names: &[String]) -> Vec<Completion> {
    let mut cursor = cursor.min(text.len());
    while !text.is_char_boundary(cursor) {
        cursor -= 1;
    }
    let Ok(mut toks) = lex(&text[..cursor]) else {
        return Vec::new();
    };
    // A word touching the cursor is being typed: use it as a prefix filter, not as context.
    let partial = match toks.last() {
        Some(Token {
            tok: Tok::Word(w), end, ..
        }) if *end == cursor => {
            let w = w.clone();
            toks.pop();
            w
        }
        _ => String::new(),
    };
    let lower = partial.to_lowercase();
    let stage_start = toks.iter().rposition(|t| t.tok == Tok::Pipe);
    let candidates = match stage_start {
        Some(i) if i + 1 == toks.len() => items(&STAGES, CompletionKind::Keyword),
        _ => {
            let (stage, rest) = match stage_start {
                Some(i) => match &toks[i + 1].tok {
                    Tok::Word(w) => (w.to_ascii_lowercase(), &toks[i + 2..]),
                    _ => (String::new(), &toks[i + 1..]),
                },
                None => (String::new(), &toks[..]),
            };
            match stage.as_str() {
                "stats" => stats_context(rest, field_names),
                "sort" => field_list_context(rest, field_names, true),
                "fields" => field_list_context(rest, field_names, false),
                "head" | "limit" => Vec::new(),
                _ => filter_context(rest, field_names),
            }
        }
    };
    let mut out: Vec<Completion> = Vec::new();
    for c in candidates {
        if c.label.to_lowercase().starts_with(&lower) && !out.iter().any(|o| o.label == c.label) {
            out.push(c);
        }
    }
    out
}
