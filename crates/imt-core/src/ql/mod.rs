//! A small query language for the log grid, in the spirit of KQL and LogQL.
//!
//! ```text
//! level >= warning and message contains "timeout" | stats count(), p95(duration) as p95 by source | sort p95 desc | head 10
//! ```
//!
//! Grammar (keywords are case-insensitive):
//!
//! ```text
//! query   := [expr] ( '|' stage )*
//! expr    := or ;  or := and ('or' and)* ;  and := not (['and'] not)* ;  not := 'not' not | term
//! term    := '(' expr ')' | text | field op value | field ('contains'|'!contains') value
//!          | field '~' regex | field '!~' regex | field 'in' '(' value (',' value)* ')'
//!          | 'exists' '(' field ')' | 'missing' '(' field ')'
//! op      := '=' | '!=' | '>' | '>=' | '<' | '<='
//! text    := bare word | "quoted" | 'quoted'      (full-text over message, exception and template)
//! field   := identifier | @pseudo | `quoted name`   (level, message, exception, source alias @level ...)
//! stage   := 'where' expr | 'stats' agg (',' agg)* ['by' field (',' field)*]
//!          | 'sort' field ['asc'|'desc'] (',' ...)* | ('head'|'limit') N | 'fields' field (',' field)*
//! agg     := ('count' '(' ')' | fn '(' field ')') ['as' name]
//! fn      := count_distinct | avg | min | max | sum | p50 | p95 | p99
//! ```
//!
//! Everything here is pure: [`parse`] turns text into a typed [`Program`], [`run`] evaluates it over
//! an iterator of records, and [`completions`] drives editor suggestions.

pub mod calc;
mod complete;
mod control;
mod error;
mod exec;
mod expr;
mod lexer;
mod parser;
mod stats;
mod suggest;

pub use calc::CalcExpr;
pub use complete::{completions, Completion, CompletionKind};
pub use control::{CancelHook, Cancelled, HookGuard, QueryControl, QueryProgress, QueryStage, RunError, Watched, BATCH};
pub use error::QlError;
pub use exec::{run, run_rows, run_with};
pub use expr::{full_text_match, EventRow, Expr, Filter, Literal, NumOp, Operand, Row, TableRow, TextSearch};
pub use lexer::CmpOp;
pub use parser::{parse, AggFn, AggSpec, FieldRef, Program, SortKeySpec, Stage};
pub use stats::MAX_PERCENTILE_VALUES;
pub use suggest::{closest, distance, unknown_field, PSEUDO_FIELDS};

use serde::Serialize;
use serde_json::Value;

/// What a query produces: matching event ids (in input order unless sorted), or a table after `stats`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum QlResult {
    Events {
        ids: Vec<u32>,
        /// Set by a `fields` stage: the columns the UI should show.
        fields: Option<Vec<String>>,
    },
    Table {
        columns: Vec<String>,
        rows: Vec<Vec<Value>>,
    },
}
