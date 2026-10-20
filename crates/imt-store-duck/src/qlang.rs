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
        // A program whose first stage after its filter is `stats` aggregates in SQL when it can.
        if sf.exact {
            let lead = program.stages.iter().take_while(|s| matches!(s, ql::Stage::Where(_))).count();
            if let Some(ql::Stage::Stats { aggs, by }) = program.stages.get(lead) {
                if let Some(sql) = stats_sql(aggs, by, &ctx.fields, &sf.all_sql()) {
                    let (columns, rows) = self.stats_rows(&conn, &sql, aggs, by)?;
                    return ql::run_table(columns, rows, &program.stages[lead + 1..], control).map_err(|e| match e {
                        RunError::Cancelled => StoreError::Cancelled,
                        RunError::Invalid(q) => StoreError::Query(q),
                    });
                }
            }
        }
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
        let sql = format!("SELECT {select} FROM {FROM} WHERE {} {}", sf.all_sql(), order_by(false));
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

    /// Runs the SQL of a `stats` stage: the group keys, then per aggregate its value (and, for a
    /// percentile, the count of values, checked against the engine's limit).
    fn stats_rows(
        &self,
        conn: &duckdb::Connection,
        sql: &str,
        aggs: &[ql::AggSpec],
        by: &[ql::FieldRef],
    ) -> StoreResult<(Vec<String>, Vec<Vec<serde_json::Value>>)> {
        use serde_json::Value;
        let mut stmt = conn.prepare(sql).map_err(crate::engine)?;
        let mut rows = stmt.query([]).map_err(crate::engine)?;
        let mut out = Vec::new();
        while let Some(r) = rows.next().map_err(crate::engine)? {
            let mut cells = Vec::with_capacity(by.len() + aggs.len());
            let mut i = 0;
            for f in by {
                cells.push(if f.name == "@line" {
                    r.get::<_, Option<i64>>(i).map_err(crate::engine)?.map_or(Value::Null, Value::from)
                } else {
                    r.get::<_, Option<String>>(i)
                        .map_err(crate::engine)?
                        .map_or(Value::Null, Value::String)
                });
                i += 1;
            }
            for a in aggs {
                cells.push(match a.func {
                    ql::AggFn::Count | ql::AggFn::CountDistinct => Value::from(r.get::<_, i64>(i).map_err(crate::engine)?),
                    ql::AggFn::Percentile(_) => {
                        let n: i64 = r.get(i + 1).map_err(crate::engine)?;
                        if n as usize > ql::MAX_PERCENTILE_VALUES {
                            return Err(StoreError::Query(QlError::new(
                                format!(
                                    "more than {} values for an exact percentile; narrow the filter first",
                                    ql::MAX_PERCENTILE_VALUES
                                ),
                                a.offset,
                                a.len,
                            )));
                        }
                        let v = r
                            .get::<_, Option<f64>>(i)
                            .map_err(crate::engine)?
                            .map_or(Value::Null, ql::json_number);
                        i += 1;
                        v
                    }
                    _ => r
                        .get::<_, Option<f64>>(i)
                        .map_err(crate::engine)?
                        .map_or(Value::Null, ql::json_number),
                });
                i += 1;
            }
            out.push(cells);
        }
        let columns = by
            .iter()
            .map(|f| f.label.clone())
            .chain(aggs.iter().map(|a| a.column.clone()))
            .collect();
        Ok((columns, out))
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

/// The SQL of a program's first `stats` over the events matching `filter`, groups in the order their first
/// events appear (as the engine lists them); `None` when a key or value cannot be expressed exactly (the
/// program then streams the events).
fn stats_sql(aggs: &[ql::AggSpec], by: &[ql::FieldRef], fields: &crate::sql::Fields, filter: &str) -> Option<String> {
    use crate::sql::{number_expr, text_expr, FieldSource};
    let mut keys = Vec::new();
    for f in by {
        if f.name == "@line" {
            keys.push("CAST(e.line AS BIGINT)".to_string());
            continue;
        }
        // Declared numeric or boolean columns keep their JSON type in the engine: not expressible as text.
        if fields
            .events()
            .declared_index(&f.name)
            .is_some_and(|i| fields.events().declared()[i].ty != imt_core::batch::ColumnType::Utf8)
        {
            return None;
        }
        keys.push(text_expr(&f.name, fields)?);
    }
    let mut values = Vec::new();
    for a in aggs {
        let field = a.field.as_ref().map(|f| f.name.as_str());
        let num = || field.and_then(|f| number_expr(f, fields));
        values.push(match a.func {
            ql::AggFn::Count => "count(*)".to_string(),
            ql::AggFn::CountDistinct => format!("count(DISTINCT {})", text_expr(field?, fields)?),
            ql::AggFn::Sum => format!("coalesce(sum({}), 0)", num()?),
            ql::AggFn::Avg => format!("avg({})", num()?),
            ql::AggFn::Min => format!("min({})", num()?),
            ql::AggFn::Max => format!("max({})", num()?),
            ql::AggFn::Percentile(p) => {
                let n = num()?;
                let p = crate::sql::double(p / 100.0)?;
                // Nearest rank, as the engine computes it: the ceil(p * count)-th smallest value.
                format!(
                    "list_sort(list({n}) FILTER (WHERE {n} IS NOT NULL))[greatest(1, least(count({n}), CAST(ceil({p} * count({n})) AS BIGINT)))], count({n})"
                )
            }
        });
    }
    let order = "min(struct_pack(k := coalesce(e.ts, 9223372036854775807), s := e.source, l := e.line, i := e.id))";
    let select = keys.iter().chain(values.iter()).cloned().collect::<Vec<_>>().join(", ");
    Some(if keys.is_empty() {
        format!("SELECT {select} FROM {FROM} WHERE {filter}")
    } else {
        let group = (1..=keys.len()).map(|i| i.to_string()).collect::<Vec<_>>().join(", ");
        format!("SELECT {select} FROM {FROM} WHERE {filter} GROUP BY {group} ORDER BY {order}")
    })
}
