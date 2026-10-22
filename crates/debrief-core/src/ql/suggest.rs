//! "Did you mean": fields a query names that the data does not have, with the
//! closest names it does have.

use super::error::QlError;
use super::expr::TableRow;
use super::parser::{Program, Stage};

/// Pseudo-fields every event has.
pub const PSEUDO_FIELDS: [&str; 7] = ["@level", "@message", "@exception", "@template", "@source", "@line", "@id"];
/// Suggestions given at most.
const MAX_SUGGESTIONS: usize = 3;

/// The first field `program` names that is not in `fields` (nor a pseudo-field), as an error
/// pointing at the name with up to three close known names. After a `stats` stage the names
/// are its columns. `None` when every name is known, and always when `fields` is empty (nothing
/// loaded yet: any name may appear).
pub fn unknown_field(program: &Program, fields: &[String]) -> Option<QlError> {
    if fields.is_empty() {
        return None;
    }
    let event_names: Vec<String> = PSEUDO_FIELDS.iter().map(|s| s.to_string()).chain(fields.iter().cloned()).collect();
    // `None` while the query works on events; the table's columns after a `stats`.
    let mut columns: Option<Vec<String>> = None;
    let mut refs = program.field_refs.iter().peekable();
    for part in 0..=program.stages.len() {
        while let Some((_, r)) = refs.next_if(|(p, _)| *p == part) {
            let known = match &columns {
                None => event_names.contains(&r.name),
                Some(cols) => TableRow::column(cols, &r.name).is_some(),
            };
            if !known {
                let candidates = columns.as_deref().unwrap_or(&event_names);
                return Some(QlError {
                    message: format!("unknown field \"{}\"", r.label),
                    offset: r.offset,
                    len: r.len,
                    suggestions: closest(&r.label, candidates),
                });
            }
        }
        if let Some(Stage::Stats { aggs, by }) = part.checked_sub(1).and_then(|i| program.stages.get(i)) {
            columns = Some(
                by.iter()
                    .map(|f| f.label.clone())
                    .chain(aggs.iter().map(|a| a.column.clone()))
                    .collect(),
            );
        }
    }
    None
}

/// Up to three of `candidates` closest to `name` (case-insensitive edit distance, `@` ignored),
/// best first; only names within about a third of the length.
pub fn closest(name: &str, candidates: &[String]) -> Vec<String> {
    let norm = |s: &str| s.trim_start_matches('@').to_lowercase();
    let want = norm(name);
    let limit = (want.chars().count() / 3).max(2);
    let mut scored: Vec<(usize, &String)> = candidates
        .iter()
        .map(|c| (distance(&want, &norm(c)), c))
        .filter(|(d, _)| *d <= limit)
        .collect();
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
    let mut out: Vec<String> = Vec::new();
    for (_, c) in scored {
        if !out.contains(c) {
            out.push(c.clone());
        }
        if out.len() == MAX_SUGGESTIONS {
            break;
        }
    }
    out
}

/// Optimal string alignment distance (Damerau–Levenshtein with adjacent transpositions).
pub fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut v = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                v = v.min(d[i - 2][j - 2] + 1);
            }
            d[i][j] = v;
        }
    }
    d[a.len()][b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ql::parse;

    fn fields() -> Vec<String> {
        ["MachineName", "ProcessId", "ElapsedMs", "RequestPath"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    fn check(q: &str) -> Option<QlError> {
        unknown_field(&parse(q).unwrap(), &fields())
    }

    #[test]
    fn edit_distance_counts_transpositions_once() {
        assert_eq!(distance("", "abc"), 3);
        assert_eq!(distance("level", "levle"), 1);
        assert_eq!(distance("kitten", "sitting"), 3);
        assert_eq!(distance("same", "same"), 0);
    }

    #[test]
    fn suggestions_are_close_names_best_first() {
        let f = fields();
        assert_eq!(closest("machinename", &f), ["MachineName"]);
        assert_eq!(closest("ElapsedMS", &f), ["ElapsedMs"]);
        assert!(closest("zzzzzz", &f).is_empty());
        let many: Vec<String> = ["ab", "ac", "ad", "ae", "ab"].iter().map(|s| s.to_string()).collect();
        assert_eq!(closest("aa", &many), ["ab", "ac", "ad"], "at most three, no duplicates");
        assert_eq!(closest("levle", &PSEUDO_FIELDS.map(String::from)), ["@level"]);
    }

    #[test]
    fn unknown_fields_point_at_the_name() {
        let e = check("level >= error and MachineNme = WEB01").unwrap();
        assert_eq!(e.message, "unknown field \"MachineNme\"");
        assert_eq!((e.offset, e.len), (19, 10));
        assert_eq!(e.suggestions, ["MachineName"]);
        let tick = check("`Elapsed Ms` > 5").unwrap();
        assert_eq!((tick.offset, tick.len), (0, 12));
        assert!(check("exists(Procesid)").unwrap().suggestions.contains(&"ProcessId".to_string()));
        assert!(check("| sort ElapsedMz desc").is_some());
        assert!(check("| fields MachineName, nope").is_some());
    }

    #[test]
    fn known_names_pseudo_fields_and_plain_text_pass() {
        for q in [
            "",
            "timeout",
            "\"connection refused\"",
            "level >= warning and message contains x and source = a and @line > 3 and @template contains y",
            "exception contains Timeout or MachineName in (WEB01, WEB02)",
            "| stats count() as n, avg(ElapsedMs) by MachineName | where n > 3 | sort n desc | fields MachineName, n",
            "| stats count() by level | sort level",
            "| stats p95(ElapsedMs) as p | sort p",
        ] {
            assert_eq!(check(q), None, "{q}");
        }
    }

    #[test]
    fn after_stats_the_columns_are_the_names() {
        let e = check("| stats count() as hits by MachineName | sort hitz desc").unwrap();
        assert_eq!(e.suggestions, ["hits"]);
        assert!(check("| stats count() by MachineName | where ProcessId > 1").is_some());
        assert!(
            check("| stats avg(Elapsed) by MachineName").is_some(),
            "the stage's own input is events"
        );
    }

    #[test]
    fn nothing_loaded_means_nothing_is_unknown() {
        assert_eq!(unknown_field(&parse("Anything = 1").unwrap(), &[]), None);
    }
}
