//! The query language on the store: the filter part (the program's leading filter and `where` stages,
//! with the rest of the query) compiles to SQL where it can; the program then runs over the matching
//! events streamed in time order (`stats` streams, nothing is kept per event). A cancel through the
//! [`QueryControl`] interrupts the running DuckDB statement.

use crate::analysis::{EventStream, MAX_ANALYSED};
use crate::sql::{order_by, Columns, FROM};
use crate::Inner;
use imt_core::api::{CompletionRequest, QueryRequest};
use imt_core::ports::{StoreError, StoreResult};
use imt_core::ql::{self, Completion, QlError, QlResult, QueryControl, RunError};

impl Inner {
    fn field_names(&self) -> StoreResult<Vec<String>> {
        Ok(self.summary()?.fields.into_iter().map(|f| f.name).collect())
    }

    pub(crate) fn run_query_with(&self, req: &QueryRequest, control: &QueryControl) -> StoreResult<QlResult> {
        // An interrupted statement fails with an engine error: that is the cancel.
        self.run_program(req, control).map_err(|e| match e {
            StoreError::Engine(_) if control.is_cancelled() => StoreError::Cancelled,
            e => e,
        })
    }

    fn run_program(&self, req: &QueryRequest, control: &QueryControl) -> StoreResult<QlResult> {
        let program = ql::parse(&req.query.ql).map_err(StoreError::Query)?;
        control.check().map_err(|_| StoreError::Cancelled)?;
        let conn = self.conn()?;
        let ctx = self.ctx(&conn)?;
        // The query with its `ql` compiles the program's filter part too (CompiledQuery lowers it).
        let (sf, cq) = self.compile(&ctx, &req.query)?;
        let handle = conn.interrupt_handle();
        let _hook = control.on_cancel(Box::new(move || handle.interrupt()));
        // Only the fields the program names, and the message (full text); in time order, which also
        // gives `stats` its first-seen group order.
        let names: Vec<String> = program.keys.names().iter().map(|n| n.to_string()).collect();
        let filters = std::iter::once(&program.filter).chain(program.stages.iter().filter_map(|s| match s {
            ql::Stage::Where(f) => Some(f),
            _ => None,
        }));
        let message =
            program.field_refs.iter().any(|(_, f)| f.name == "@message") || filters.filter_map(|f| f.expr.as_ref()).any(full_text);
        let select = self.select_list(&sf, &cq, &Columns::only(message, names));
        let order = order_by(false);
        let sql = format!("SELECT {select} FROM {FROM} WHERE {} {order}", sf.all_sql());
        let mut stream = EventStream::new(&conn, &sql, self.schema.clone(), &ctx, &cq, sf.exact, MAX_ANALYSED)?.controlled(control);
        let result = ql::run_rows(&program, &mut stream, control);
        let coverage = stream.finish()?;
        let result = result.map_err(|e| match e {
            RunError::Cancelled => StoreError::Cancelled,
            RunError::Invalid(q) => StoreError::Query(q),
        })?;
        if !coverage.complete() {
            return Err(StoreError::Invalid(format!(
                "the query matches {} events, more than the {MAX_ANALYSED} the query language evaluates; narrow it (time range, levels, a filter) first",
                coverage.matched
            )));
        }
        Ok(result)
    }

    pub(crate) fn check_query(&self, text: &str) -> StoreResult<Option<QlError>> {
        Ok(match ql::parse(text) {
            Err(e) => Some(e),
            Ok(program) => ql::unknown_field(&program, &self.field_names()?),
        })
    }

    pub(crate) fn complete_query(&self, req: &CompletionRequest) -> StoreResult<Vec<Completion>> {
        Ok(ql::completions(&req.text, req.cursor, &self.field_names()?))
    }
}

/// Whether `e` reads the message (full text, or a comparison on `@message`).
fn full_text(e: &ql::Expr) -> bool {
    use imt_core::row::{FieldKey, Pseudo};
    let message = |k: &FieldKey| *k == FieldKey::Pseudo(Pseudo::Message);
    match e {
        ql::Expr::And(v) | ql::Expr::Or(v) => v.iter().any(full_text),
        ql::Expr::Not(x) => full_text(x),
        ql::Expr::Text(_) | ql::Expr::Search(_) => true,
        ql::Expr::Cmp { field, .. }
        | ql::Expr::Contains { field, .. }
        | ql::Expr::Regex { field, .. }
        | ql::Expr::In { field, .. }
        | ql::Expr::TextEq { field, .. }
        | ql::Expr::TextIn { field, .. }
        | ql::Expr::NumCmp { field, .. } => message(field),
        ql::Expr::Exists(field) | ql::Expr::Missing(field) => message(field),
        ql::Expr::Group { by, .. } => by.iter().any(message),
        _ => false,
    }
}
