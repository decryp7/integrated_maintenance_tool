//! nl_to_query: a question in plain language becomes a query-language filter. The engine checks the
//! syntax; the first invalid answer gets one repair round with the engine's error message.

use super::proposals::QueryProposal;
use super::tools::truncate_chars;
use super::Assistant;
use crate::access::StoreRef;
use crate::error::{AiError, Result};
use crate::llm::LlmProvider;
use imt_core::api::QueryRequest;
use imt_core::ql::QlResult;
use imt_core::query::Query;
use serde::Deserialize;
use serde_json::json;

const SYSTEM: &str = "You translate questions about logs into the IMT query language. Answer with JSON only.";

const SYNTAX: &str = "Write ONE query in the IMT log query language. Copy the style of these examples exactly:\n\
- level >= error\n\
- level = warning\n\
- level >= error and message contains \"timeout\"\n\
- MachineName = WEB02 and level = warning\n\
- message ~ \"capture.*failed\"\n\
- @source contains \"billing\"\n\
- exists(@exception)\n\
- | stats count() as n by MachineName | sort n desc\n\
- level >= error | stats count() as n by MachineName | sort n desc | head 5\n\
- exists(Elapsed) | sort Elapsed desc | head 20\n\
Rules: the part before the first | is a filter (comparisons joined with and, or, not) and may be empty. ALL conditions go in that first part, joined with `and`; never write a condition after a | (wrong: `level >= error | MachineName = WEB02`, right: `level >= error and MachineName = WEB02`). `level >= warning` means warning OR WORSE; for exactly one level use `level = warning`. \
Add stages after | ONLY when the question asks for a count, a ranking, a top N or a grouping: stats (count(), avg(f), max(f), p95(f); always name the result with `as` and sort by that name), sort, head, fields. \
Use the exact field names listed. Special fields: @level, @message, @exception, @source (the file the event came from). \
Levels: verbose, debug, information, warning, error, fatal. Do not write SQL. Do not write the word filter.";

#[derive(Deserialize)]
struct Answer {
    ql: String,
    #[serde(default)]
    explanation: String,
}

impl<P: LlmProvider> Assistant<P> {
    pub fn nl_to_query<'s>(&self, store: impl Into<StoreRef<'s>>, question: &str) -> Result<QueryProposal> {
        let store: StoreRef<'_> = store.into();
        if question.trim().is_empty() {
            return Err(AiError::Invalid("ask a question".into()));
        }
        let fields: Vec<String> = store.with(|s| {
            s.summary()
                .map(|s| s.fields)
                .unwrap_or_default()
                .iter()
                .take(40)
                .map(|f| format!("{}{}", f.name, if f.numeric { " (number)" } else { "" }))
                .collect()
        });
        let user = format!(
            "{SYNTAX}\n\nUse only these fields: {}\n\nQuestion: {}\nReturn JSON {{\"ql\": \"the query\", \"explanation\": \"one sentence\"}}.",
            truncate_chars(&fields.join(", "), self.prompt_chars() / 4),
            truncate_chars(question.trim(), 500)
        );
        let schema = json!({"type": "object", "properties": {"ql": {"type": "string"}, "explanation": {"type": "string"}}, "required": ["ql", "explanation"]});
        let (q, repaired) = self.generate_validated::<Answer, (String, String)>(SYSTEM, &user, schema, |a| {
            let ql = a.ql.trim().to_string();
            if ql.is_empty() {
                return Err("the query is empty".into());
            }
            match store.with(|s| s.check_query(&ql)) {
                Ok(Some(e)) => Err(format!(
                    "`{ql}` is not valid: {} (at byte {}). Fix the syntax; use only the available fields.",
                    e.message, e.offset
                )),
                Ok(None) => Ok((ql, a.explanation.trim().to_string())),
                Err(e) => Err(format!("`{ql}` cannot be checked: {e}")),
            }
        })?;
        // Read-only: running a query never changes anything.
        let req = QueryRequest {
            query: Query {
                ql: q.0.clone(),
                ..Query::default()
            },
        };
        let estimated_matches = match store.with(|s| s.run_query(&req)) {
            Ok(QlResult::Events { ids, .. }) => Some(ids.len()),
            Ok(QlResult::Table { rows, .. }) => Some(rows.len()),
            Err(_) => None,
        };
        Ok(QueryProposal {
            ql: q.0,
            explanation: q.1,
            estimated_matches,
            repaired,
        })
    }
}
