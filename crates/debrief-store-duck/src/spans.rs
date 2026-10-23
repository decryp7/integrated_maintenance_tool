//! Span rule inference on the store: the example events are read by id, the engine proposes rules
//! (`debrief_core::spans::infer`) and each is evaluated over the stored events. Only events that can be a
//! start or stop are read (the rule's marker conditions compiled to SQL), in time order.

use crate::analysis::{EventStream, MAX_ANALYSED};
use crate::sql::{expr_sql, order_by, row_columns, Columns, FROM};
use crate::Inner;
use debrief_core::ports::{Analyzed, Filter, StoreError, StoreResult};
use debrief_core::query::{group_key_in, CompiledQuery};
use debrief_core::row::{OwnedRow, RowRef};
use debrief_core::spans::infer::{
    propose, EvaluatedCandidate, SpanEvaluation, SpanEvaluator, SpanProposal, SpanRuleCheck, SpanRuleRequest,
};
use std::collections::HashMap;

fn invalid(e: anyhow::Error) -> StoreError {
    StoreError::Invalid(format!("{e:#}"))
}

impl Inner {
    /// The events `ids` (all columns but `raw`).
    fn events_by_id(&self, ids: &[u64]) -> StoreResult<HashMap<u64, OwnedRow>> {
        let conn = self.conn()?;
        let ctx = self.ctx(&conn)?;
        let cq = CompiledQuery::new(&Filter::default(), &ctx.default_group_by, &ctx.marker_kinds).map_err(invalid)?;
        let list: Vec<String> = ids.iter().map(u64::to_string).collect();
        let sql = format!(
            "SELECT {} FROM {FROM} WHERE e.id IN ({})",
            row_columns(&self.schema, &Columns::all()),
            list.join(", ")
        );
        let mut stream = EventStream::new(&conn, &sql, self.schema.clone(), &ctx, &cq, true, usize::MAX)?;
        let rows: HashMap<u64, OwnedRow> = stream.by_ref().map(|r| (r.id, r)).collect();
        stream.finish()?;
        Ok(rows)
    }

    pub(crate) fn propose_span_rule(&self, req: &SpanRuleRequest) -> StoreResult<SpanProposal> {
        let ids: Vec<u64> = req.examples.iter().flat_map(|e| [e.start.0, e.end.0]).collect();
        if ids.is_empty() {
            return Err(StoreError::Invalid(
                "pick at least one start event and the end event that belongs to it".into(),
            ));
        }
        let rows = self.events_by_id(&ids)?;
        let mut pairs: Vec<(&dyn RowRef, &dyn RowRef)> = Vec::new();
        for e in &req.examples {
            match (rows.get(&e.start.0), rows.get(&e.end.0)) {
                (Some(a), Some(b)) => pairs.push((a, b)),
                _ => return Err(StoreError::NotFound(format!("the example events {} and {}", e.start.0, e.end.0))),
            }
        }
        let profile = self.profile();
        let candidates = propose(&pairs, &req.hints, &profile.profile).map_err(invalid)?;
        let mut out = SpanProposal::default();
        for candidate in candidates {
            let check = SpanRuleCheck {
                profile_override: candidate.profile_override.clone(),
                kind: candidate.kind.clone(),
            };
            let evaluated = self.evaluate_span_rule(&check)?;
            if !evaluated.coverage.complete() {
                out.notes.extend(evaluated.coverage.limits.iter().cloned());
            }
            out.candidates.push(EvaluatedCandidate {
                candidate,
                evaluation: evaluated.result,
            });
        }
        // The rule that pairs the most events cleanly first.
        out.candidates.sort_by_key(|c| {
            let e = &c.evaluation;
            std::cmp::Reverse(e.spans as i64 - (e.unmatched_starts + e.unmatched_ends) as i64)
        });
        Ok(out)
    }

    /// The Runs editor's preview: the draft paired over the timed events in time order (at most
    /// [`MAX_ANALYSED`]).
    pub(crate) fn preview_spans(&self, req: &debrief_core::spans::SpanPreviewRequest) -> StoreResult<debrief_core::spans::SpanPreview> {
        let profile = self.profile();
        let conn = self.conn()?;
        let ctx = self.ctx(&conn)?;
        let mut p = debrief_core::spans::preview::SpanPreviewer::new(req, &profile, ctx.default_group_by.clone());
        drop(conn);
        if p.wants_events() {
            let (group_by, scopes) = (ctx.default_group_by.clone(), ctx.key_scopes.clone());
            self.stream_rows(&Filter::default(), MAX_ANALYSED, &mut |r| {
                if r.rec.ts.is_some() {
                    let lane = group_key_in(&group_by, &scopes, &r.rec, &r.source);
                    p.add(&r.rec, &r.source, &lane);
                }
            })?;
        }
        Ok(p.finish())
    }

    pub(crate) fn evaluate_span_rule(&self, req: &SpanRuleCheck) -> StoreResult<Analyzed<SpanEvaluation>> {
        let active = self.profile();
        let candidate = active
            .profile
            .with_override(&req.profile_override)
            .map_err(|e| StoreError::Invalid(format!("the rule is not a valid profile override: {e}")))?;
        let compiled = self.registry().compile_profile(candidate).map_err(invalid)?;
        let mut evaluator = SpanEvaluator::new(&compiled, &req.kind).map_err(invalid)?;
        let conn = self.conn()?;
        let ctx = self.ctx(&conn)?;
        // Only events that can start or stop a run: per start/stop marker, its conditions SQL can express.
        let gates: Vec<String> = evaluator
            .gates()
            .iter()
            .map(|g| {
                let parts: Vec<String> = g.iter().filter_map(|c| expr_sql(c.expr(), &self.schema, &[])).collect();
                if parts.is_empty() {
                    "TRUE".to_string()
                } else {
                    format!("({})", parts.join(" AND "))
                }
            })
            .collect();
        let filter = if gates.is_empty() {
            "FALSE".to_string()
        } else {
            gates.join(" OR ")
        };
        let cq = CompiledQuery::new(&Filter::default(), &ctx.default_group_by, &ctx.marker_kinds).map_err(invalid)?;
        let sql = format!(
            "SELECT {} FROM {FROM} WHERE e.ts IS NOT NULL AND ({filter}) {}",
            row_columns(&self.schema, &Columns::all()),
            order_by(false)
        );
        let mut stream = EventStream::new(&conn, &sql, self.schema.clone(), &ctx, &cq, true, MAX_ANALYSED)?;
        for r in stream.by_ref() {
            let lane = group_key_in(&ctx.default_group_by, &ctx.key_scopes, &r.rec, &r.source);
            evaluator.add(&r.rec, &r.source, &lane);
        }
        let coverage = stream.finish()?;
        Ok(Analyzed {
            result: evaluator.finish(),
            coverage,
        })
    }
}
