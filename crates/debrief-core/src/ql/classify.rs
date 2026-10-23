//! Is the text in the search field a plain message search or a query-language query?
//!
//! The filter bar has one field for both. Plain words ("connection refused", "timeout 504", "error (retry)")
//! search the messages as they always did; text with query structure (`level >= warning`, `| stats count()`,
//! `exists(TraceId)`, `UserId = 42`) is a query. A comparison only counts when its left side names a field the data
//! has (or a pseudo-field), so log text such as `status=500` stays a message search when there is no `status`
//! field. Half-written queries (`level >=`) are queries, so the editor can say what is missing.

use super::lexer::{lex, Tok, Token};
use super::suggest::PSEUDO_FIELDS;
use serde::{Deserialize, Serialize};

/// The longest text classified; longer text is a message search (the search box has its own limits).
pub const MAX_CLASSIFY_LEN: usize = 8 * 1024;
/// Field names taken into account at most (the rest are ignored).
pub const MAX_CLASSIFY_FIELDS: usize = 5_000;

/// What the search field holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InputKind {
    /// Words to find in messages, exceptions and templates.
    Text,
    /// A query-language query.
    Query,
}

/// Field names the query language knows without the data: the pseudo-fields and their plain aliases.
const ALIASES: [&str; 5] = ["level", "message", "exception", "template", "source"];

fn known(name: &str, fields: &[String]) -> bool {
    let lower = name.to_ascii_lowercase();
    PSEUDO_FIELDS.iter().any(|p| p.eq_ignore_ascii_case(name))
        || ALIASES.contains(&lower.as_str())
        || fields
            .iter()
            .take(MAX_CLASSIFY_FIELDS)
            .any(|f| f == name || f.eq_ignore_ascii_case(name))
}

fn word(t: Option<&Token>) -> Option<&str> {
    match t.map(|t| &t.tok) {
        Some(Tok::Word(w)) => Some(w.as_str()),
        _ => None,
    }
}

/// Does the token before `i` name a field (a back-quoted name always does)?
fn field_before(tokens: &[Token], i: usize, fields: &[String]) -> bool {
    match i.checked_sub(1).and_then(|j| tokens.get(j)).map(|t| &t.tok) {
        Some(Tok::Tick(_)) => true,
        Some(Tok::Word(w)) => known(w, fields),
        _ => false,
    }
}

/// Classifies the search field's text; `fields` are the names the loaded data has. Never fails: anything that does
/// not look like a query (including text the query lexer rejects, such as an apostrophe) is a message search.
pub fn classify_input(text: &str, fields: &[String]) -> InputKind {
    if text.len() > MAX_CLASSIFY_LEN || text.trim().is_empty() {
        return InputKind::Text;
    }
    let Ok(tokens) = lex(text) else {
        return InputKind::Text;
    };
    for (i, t) in tokens.iter().enumerate() {
        let query = match &t.tok {
            // `| stats ...`, `| where ...`: a pipeline is always a query.
            Tok::Pipe | Tok::Tick(_) => true,
            Tok::Op(_) => field_before(&tokens, i, fields),
            // `!contains`
            Tok::Bang => word(tokens.get(i + 1)).is_some_and(|w| w.eq_ignore_ascii_case("contains")) && field_before(&tokens, i, fields),
            Tok::Word(w) if w.eq_ignore_ascii_case("contains") || w.eq_ignore_ascii_case("in") => {
                field_before(&tokens, i, fields) && tokens.get(i + 1).is_some_and(|n| !matches!(n.tok, Tok::Pipe))
            }
            // `exists(Field)`, `missing(Field)`
            Tok::Word(w) if w.eq_ignore_ascii_case("exists") || w.eq_ignore_ascii_case("missing") => {
                matches!(tokens.get(i + 1).map(|n| &n.tok), Some(Tok::LParen))
            }
            _ => false,
        };
        if query {
            return InputKind::Query;
        }
    }
    InputKind::Text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields() -> Vec<String> {
        vec!["UserId".into(), "SourceContext".into(), "status_code".into()]
    }

    fn kind(text: &str) -> InputKind {
        classify_input(text, &fields())
    }

    #[test]
    fn plain_words_are_a_message_search() {
        for text in [
            "connection refused",
            "timeout 504",
            "error (retry)",
            "\"exact phrase\"",
            "not found",
            "a, b, c",
            "the file contains errors",
            "Hello!",
            "can't connect",
            "in progress",
            "exists in cache",
        ] {
            assert_eq!(kind(text), InputKind::Text, "{text}");
        }
    }

    #[test]
    fn query_structure_is_a_query() {
        for text in [
            "level >= warning",
            "level >=",
            "@level = error",
            "UserId = 42",
            "userid != 7",
            "message contains \"timeout\"",
            "SourceContext !contains Api",
            "source in (a, b)",
            "exists(TraceId)",
            "missing (TraceId)",
            "timeout | stats count() by source",
            "| head 10",
            "`Some field` = 1",
            "status_code > 499",
            "message ~ \"^GET\"",
        ] {
            assert_eq!(kind(text), InputKind::Query, "{text}");
        }
    }

    #[test]
    fn a_comparison_on_an_unknown_name_stays_text() {
        assert_eq!(kind("status=500"), InputKind::Text);
        assert_eq!(kind("x > 1"), InputKind::Text);
        assert_eq!(kind("= 5"), InputKind::Text);
        assert_eq!(kind("contains x"), InputKind::Text);
        assert_eq!(kind("UserId contains"), InputKind::Text);
    }

    #[test]
    fn empty_too_long_and_unlexable_text_is_text() {
        assert_eq!(kind(""), InputKind::Text);
        assert_eq!(kind("   "), InputKind::Text);
        assert_eq!(kind("level = \"unterminated"), InputKind::Text);
        let long = format!("level = {}", "x".repeat(MAX_CLASSIFY_LEN));
        assert_eq!(kind(&long), InputKind::Text);
    }

    #[test]
    fn without_data_only_pseudo_fields_and_aliases_count() {
        assert_eq!(classify_input("level = error", &[]), InputKind::Query);
        assert_eq!(classify_input("UserId = 42", &[]), InputKind::Text);
    }

    #[test]
    fn field_names_match_case_insensitively_and_only_a_bounded_list_is_looked_at() {
        assert_eq!(kind("USERID = 1"), InputKind::Query);
        assert_eq!(kind("LEVEL >= error"), InputKind::Query);
        assert_eq!(kind("@Message ~ \"x\""), InputKind::Query);
        let mut many: Vec<String> = (0..MAX_CLASSIFY_FIELDS).map(|i| format!("f{i}")).collect();
        many.push("Beyond".into());
        assert_eq!(classify_input("f4999 = 1", &many), InputKind::Query);
        assert_eq!(classify_input("Beyond = 1", &many), InputKind::Text);
    }

    #[test]
    fn bang_only_counts_as_not_contains_after_a_field() {
        assert_eq!(kind("Hello !"), InputKind::Text);
        assert_eq!(kind("level ! error"), InputKind::Text);
        assert_eq!(kind("x !contains y"), InputKind::Text);
        assert_eq!(kind("!contains level"), InputKind::Text);
        assert_eq!(kind("level !contains x"), InputKind::Query);
    }

    #[test]
    fn contains_and_in_need_something_after_them_that_is_not_a_pipe() {
        assert_eq!(kind("level in"), InputKind::Text);
        assert_eq!(kind("level contains"), InputKind::Text);
        assert_eq!(kind("level in | head 1"), InputKind::Query);
        assert_eq!(kind("message CONTAINS \"x\""), InputKind::Query);
    }

    #[test]
    fn exists_and_missing_need_a_parenthesis() {
        assert_eq!(kind("EXISTS(TraceId)"), InputKind::Query);
        assert_eq!(kind("missing"), InputKind::Text);
        assert_eq!(kind("exists TraceId"), InputKind::Text);
        assert_eq!(kind("a `b` c"), InputKind::Query);
    }

    #[test]
    fn serializes_camel_case() {
        assert_eq!(serde_json::to_string(&InputKind::Query).unwrap(), "\"query\"");
        assert_eq!(serde_json::from_str::<InputKind>("\"text\"").unwrap(), InputKind::Text);
    }
}
