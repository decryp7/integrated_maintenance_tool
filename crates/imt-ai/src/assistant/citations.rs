//! Pulling evidence references out of an answer: record ids and queries. The chat shows them as links
//! to the grid and timeline; ids the model never saw in a tool result are flagged as unverified.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Citation {
    Record { id: u32, verified: bool },
    Query { ql: String },
    Web { title: String, url: String },
}

fn id_lists() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"(?i)\b(?:events?|records?|ids?)\b\s*#?\d+(?:\s*(?:,|and|&)?\s*#?\d+)*|#\d+")
            .unwrap_or_else(|_| unreachable!("static pattern"))
    })
}

fn numbers() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\d+").unwrap_or_else(|_| unreachable!("static pattern")))
}

fn backticks() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"`([^`\n]{3,300})`").unwrap_or_else(|_| unreachable!("static pattern")))
}

/// `known_ids`: ids that appeared in tool results this turn. `is_query`: true when the engine accepts the text
/// as a query (so inline code that is not a query is not cited).
pub fn parse_citations(text: &str, known_ids: &[u32], is_query: &dyn Fn(&str) -> bool) -> Vec<Citation> {
    let mut out: Vec<Citation> = Vec::new();
    for m in id_lists().find_iter(text) {
        for n in numbers().find_iter(m.as_str()) {
            if let Ok(id) = n.as_str().parse::<u32>() {
                let c = Citation::Record {
                    id,
                    verified: known_ids.contains(&id),
                };
                if !out.contains(&c) {
                    out.push(c);
                }
            }
        }
    }
    for cap in backticks().captures_iter(text) {
        let ql = cap[1].trim().to_string();
        let looks_like_query = ql.contains(['=', '>', '<', '~', '|']) || ql.contains(" contains ") || ql.contains("stats ");
        let c = Citation::Query { ql: ql.clone() };
        if looks_like_query && !out.contains(&c) && is_query(&ql) {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn always(_: &str) -> bool {
        true
    }

    #[test]
    fn finds_ids_in_common_phrasings() {
        let t = "See event 12, 15 and 18; also record #7, (#99) and [id 3]. The year 2026 is not an id.";
        let c = parse_citations(t, &[12, 7], &always);
        let ids: Vec<(u32, bool)> = c
            .iter()
            .filter_map(|c| {
                if let Citation::Record { id, verified } = c {
                    Some((*id, *verified))
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(ids, vec![(12, true), (15, false), (18, false), (7, true), (99, false), (3, false)]);
    }

    #[test]
    fn finds_queries_and_ignores_other_code() {
        let t = "Run `level >= error and message contains \"timeout\"` or look at `foo` and `x = 1`.";
        let c = parse_citations(t, &[], &|q| q.starts_with("level"));
        assert_eq!(
            c,
            vec![Citation::Query {
                ql: "level >= error and message contains \"timeout\"".into()
            }]
        );
        let none = parse_citations("`a = b` `a = b`", &[], &|_| false);
        assert!(none.is_empty());
        let dedup = parse_citations("`a = b` and again `a = b`", &[], &always);
        assert_eq!(dedup.len(), 1);
    }

    #[test]
    fn plain_text_has_no_citations() {
        assert!(parse_citations("Nothing to cite here, 2026-10-14.", &[], &always).is_empty());
    }
}
