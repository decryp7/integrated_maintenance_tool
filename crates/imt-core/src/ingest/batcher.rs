//! From parsed events to event batches: the profile's ingest pipeline (extractors, `[[pipeline]]`
//! steps, marker rules) runs per batch of at most [`BATCH_ROWS`] events, then the events are built into
//! a [`PendingBatch`] and handed to the [`BatchSink`].

use super::problems::{ProblemKind, Problems};
use super::sink::{BatchSink, EventSink, ImportCancelled, ParsedEvent};
use crate::batch::{ColumnType, EventBatchBuilder, EventSchema, EventTime, FieldValue, LevelCode, NewEvent, SourceId, BATCH_ROWS};
use crate::model::{value_text, Record};
use crate::pipeline::StepStats;
use crate::profile::CompiledProfile;
use crate::profile::FieldType;
use anyhow::Result;
use serde_json::Value;
use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// The column type of an extractor field type (`[extract.fields]`); `auto` and unknown types are
/// stored in `extra`.
pub fn column_type(kind: FieldType) -> Option<ColumnType> {
    match kind {
        FieldType::String => Some(ColumnType::Utf8),
        FieldType::Integer => Some(ColumnType::Int64),
        FieldType::Number | FieldType::Bytes | FieldType::Duration => Some(ColumnType::Float64),
        FieldType::Bool => Some(ColumnType::Boolean),
        FieldType::Auto => None,
    }
}

/// The event schema of a profile: a typed column for every field its extractors declare with a type.
pub fn schema_for(profile: &CompiledProfile) -> EventSchema {
    let declared = profile
        .extractors
        .iter()
        .flat_map(|e| e.fields.iter())
        .filter_map(|(name, spec)| column_type(spec.kind).map(|t| (name.clone(), t)));
    // Field names come from regex capture groups, which are never empty or contain control characters.
    EventSchema::new(declared).unwrap_or_else(|_| EventSchema::plain())
}

/// The value stored in a declared column for a JSON value, if it fits the column.
fn typed<'v>(schema: &EventSchema, name: &str, v: &'v Value) -> Option<FieldValue<'v>> {
    let ty = schema.declared()[schema.declared_index(name)?].ty;
    match (ty, v) {
        (ColumnType::Utf8, Value::String(s)) => Some(FieldValue::Text(s)),
        (ColumnType::Int64, Value::Number(n)) if n.is_i64() => n.as_i64().map(FieldValue::Int),
        (ColumnType::Float64, Value::Number(n)) if n.is_f64() => n.as_f64().map(FieldValue::Float),
        (ColumnType::Boolean, Value::Bool(b)) => Some(FieldValue::Bool(*b)),
        _ => None,
    }
}

/// Where an event came from, kept beside its record while the pipeline runs.
struct Meta {
    raw: String,
    byte_offset: u64,
}

/// What a source's events came to.
#[derive(Debug, Default, Clone)]
pub struct BatchedSource {
    /// Events the importer produced.
    pub parsed: usize,
    /// Events written (after the pipeline).
    pub events: usize,
    pub from: Option<i64>,
    pub to: Option<i64>,
    /// Events in/out of each pipeline step, summed over the batches.
    pub step_stats: Vec<StepStats>,
    pub problems: Problems,
}

impl BatchedSource {
    /// Adds the totals of another part of the same source (a chunk read in parallel).
    pub fn merge(&mut self, other: BatchedSource) {
        self.parsed += other.parsed;
        self.events += other.events;
        self.from = match (self.from, other.from) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        self.to = match (self.to, other.to) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
        merge_stats(&mut self.step_stats, &other.step_stats);
        self.problems.merge(other.problems);
    }
}

/// Collects one source's parsed events, runs the ingest pipeline per batch and writes the batches.
pub struct RecordBatcher<'a> {
    profile: &'a CompiledProfile,
    source: SourceId,
    source_name: &'a str,
    sink: &'a mut dyn BatchSink,
    cancel: &'a AtomicBool,
    records: Vec<Record>,
    metas: Vec<Meta>,
    builder: EventBatchBuilder,
    out: BatchedSource,
}

impl<'a> RecordBatcher<'a> {
    pub fn new(
        profile: &'a CompiledProfile,
        schema: Arc<EventSchema>,
        source: SourceId,
        source_name: &'a str,
        sink: &'a mut dyn BatchSink,
        cancel: &'a AtomicBool,
    ) -> RecordBatcher<'a> {
        RecordBatcher {
            profile,
            source,
            source_name,
            sink,
            cancel,
            records: Vec::with_capacity(BATCH_ROWS),
            metas: Vec::with_capacity(BATCH_ROWS),
            builder: EventBatchBuilder::new(schema),
            out: BatchedSource::default(),
        }
    }

    /// Runs the pipeline on the collected events and writes them.
    fn flush(&mut self) -> Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(ImportCancelled.into());
        }
        if self.records.is_empty() {
            return Ok(());
        }
        let mut records = std::mem::take(&mut self.records);
        for (i, r) in records.iter_mut().enumerate() {
            // The pipeline keeps ids; they find each surviving record's raw text again (0 = unknown).
            r.id = i as u32 + 1;
            r.source = self.source.0;
        }
        let (records, stats) = self.profile.ingest(records, self.source_name);
        merge_stats(&mut self.out.step_stats, &stats);
        let schema = self.builder.schema().clone();
        for r in &records {
            let meta = (r.id as usize).checked_sub(1).and_then(|i| self.metas.get(i));
            let raw = meta.map_or(r.message.as_str(), |m| m.raw.as_str());
            if let Some(ts) = r.ts {
                self.out.from = Some(self.out.from.map_or(ts, |f| f.min(ts)));
                self.out.to = Some(self.out.to.map_or(ts, |t| t.max(ts)));
            } else {
                self.out.problems.add(ProblemKind::TimestampUnparsed, r.line);
            }
            let ev = NewEvent {
                time: EventTime::parsed(r.ts),
                level: LevelCode::new(r.level.index() as u8).unwrap_or(LevelCode::FATAL),
                source: self.source,
                line: r.line,
                byte_offset: meta.map_or(0, |m| m.byte_offset),
                message: &r.message,
                template: r.template.as_deref(),
                exception: r.exception.as_deref(),
                markers: r.markers,
                raw,
            };
            let texts: Vec<Option<Cow<str>>> = r
                .fields
                .iter()
                .map(|(k, v)| typed(&schema, k, v).is_none().then(|| value_text(v)))
                .collect();
            let fields = r.fields.iter().zip(&texts).filter_map(|((k, v), t)| match t {
                Some(t) => Some((k.as_str(), FieldValue::Text(t))),
                None => typed(&schema, k, v).map(|v| (k.as_str(), v)),
            });
            self.builder.push(&ev, fields)?;
            self.out.events += 1;
            if self.builder.is_full() {
                self.sink.write(self.builder.finish())?;
            }
        }
        if !self.builder.is_empty() {
            self.sink.write(self.builder.finish())?;
        }
        self.metas.clear();
        Ok(())
    }

    /// Writes what is left and returns the source's totals.
    pub fn finish(mut self) -> Result<BatchedSource> {
        self.flush()?;
        Ok(self.out)
    }
}

fn merge_stats(into: &mut Vec<StepStats>, add: &[StepStats]) {
    if into.len() < add.len() {
        into.resize(add.len(), StepStats::default());
    }
    for (a, b) in into.iter_mut().zip(add) {
        a.input += b.input;
        a.output += b.output;
        a.errors += b.errors;
    }
}

impl EventSink for RecordBatcher<'_> {
    fn event(&mut self, ev: ParsedEvent) -> Result<()> {
        self.out.parsed += 1;
        self.records.push(ev.record);
        self.metas.push(Meta {
            raw: ev.raw,
            byte_offset: ev.byte_offset,
        });
        if self.records.len() >= BATCH_ROWS {
            self.flush()?;
        }
        Ok(())
    }

    fn problem(&mut self, kind: ProblemKind, line: u32) {
        self.out.problems.add(kind, line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch::{EventBatch, EventId, PendingBatch};
    use crate::ingest::sink::{NewSource, SourceReport};
    use crate::model::Level;
    use crate::profile::Profile;

    #[derive(Default)]
    struct Collect {
        batches: Vec<EventBatch>,
        next: u64,
    }

    impl BatchSink for Collect {
        fn begin_source(&mut self, _source: &NewSource) -> Result<SourceId> {
            Ok(SourceId(4))
        }
        fn write(&mut self, batch: PendingBatch) -> Result<()> {
            let n = batch.len() as u64;
            self.batches.push(batch.assign_ids(EventId(self.next))?);
            self.next += n;
            Ok(())
        }
        fn finish_source(&mut self, _report: &SourceReport) -> Result<()> {
            Ok(())
        }
        fn abort_source(&mut self, _id: SourceId) -> Result<()> {
            Ok(())
        }
    }

    fn profile(toml: &str) -> CompiledProfile {
        CompiledProfile::compile(Profile::from_toml(toml).unwrap()).unwrap()
    }

    fn event(line: u32, message: &str, ts: Option<i64>) -> ParsedEvent {
        let mut record = Record::new(line);
        record.ts = ts;
        record.message = message.into();
        record.level = Level::Error;
        ParsedEvent {
            record,
            raw: format!("raw {line}"),
            byte_offset: line as u64 * 10,
        }
    }

    const PROFILE: &str = r#"
[[extract]]
name = "ms"
regex = 'took (?P<Ms>\d+) ms'
[extract.fields]
Ms = { type = "integer" }

[[pipeline]]
step = "drop"
when = [{ field = "@message", op = "contains", value = "noise" }]

[[marker]]
kind = "slow"
when = [{ field = "Ms", op = "gt", value = "100" }]
"#;

    #[test]
    fn runs_the_pipeline_per_batch_and_keeps_raw_text() {
        let p = profile(PROFILE);
        let schema = Arc::new(schema_for(&p));
        assert_eq!(schema.declared()[0].ty, ColumnType::Int64);
        let mut sink = Collect::default();
        let cancel = AtomicBool::new(false);
        let mut b = RecordBatcher::new(&p, schema, SourceId(4), "app.log", &mut sink, &cancel);
        let total = BATCH_ROWS + 10;
        for i in 1..=total as u32 {
            let msg = match i % 3 {
                0 => "noise".to_string(),
                1 => format!("took {} ms", i % 200),
                _ => "plain".to_string(),
            };
            b.event(event(i, &msg, (i != 5).then_some(i as i64))).unwrap();
        }
        b.problem(ProblemKind::LineNotParsed, 3);
        let out = b.finish().unwrap();
        assert_eq!(out.parsed, total);
        let dropped = total / 3;
        assert_eq!(out.events, total - dropped);
        assert_eq!(out.step_stats.len(), 1);
        assert_eq!((out.step_stats[0].input, out.step_stats[0].output), (total, total - dropped));
        assert_eq!((out.from, out.to), (Some(1), Some(total as i64 - 1)));
        assert_eq!(out.problems.count(ProblemKind::TimestampUnparsed), 1);
        assert_eq!(out.problems.count(ProblemKind::LineNotParsed), 1);
        let rows: Vec<_> = sink.batches.iter().flat_map(|b| b.rows()).collect();
        assert_eq!(rows.len(), out.events);
        assert!(sink.batches.iter().all(|b| b.num_rows() <= BATCH_ROWS));
        let r = rows.iter().find(|r| r.line() == 199).unwrap();
        assert_eq!((r.raw(), r.byte_offset(), r.source()), ("raw 199", 1990, SourceId(4)));
        assert_eq!(r.field("Ms"), Some(FieldValue::Int(199)));
        assert_eq!(r.markers(), 1, "slow");
        assert_eq!(r.level().get(), Level::Error.index() as u8);
        let untimed = rows.iter().find(|r| r.line() == 5).unwrap();
        assert_eq!(untimed.ts(), None);
    }

    #[test]
    fn cancelling_stops_at_the_next_batch() {
        let p = profile("");
        let mut sink = Collect::default();
        let cancel = AtomicBool::new(true);
        let mut b = RecordBatcher::new(&p, Arc::new(EventSchema::plain()), SourceId(0), "s", &mut sink, &cancel);
        b.event(event(1, "x", Some(1))).unwrap();
        let err = b.finish().unwrap_err();
        assert!(err.is::<ImportCancelled>());
    }

    #[test]
    fn field_types_map_to_columns() {
        assert_eq!(column_type(FieldType::Integer), Some(ColumnType::Int64));
        assert_eq!(column_type(FieldType::Duration), Some(ColumnType::Float64));
        assert_eq!(column_type(FieldType::Bool), Some(ColumnType::Boolean));
        assert_eq!(column_type(FieldType::String), Some(ColumnType::Utf8));
        assert_eq!(column_type(FieldType::Auto), None);
        let schema = EventSchema::new([("S", ColumnType::Utf8), ("B", ColumnType::Boolean), ("F", ColumnType::Float64)]).unwrap();
        assert_eq!(typed(&schema, "S", &serde_json::json!("x")), Some(FieldValue::Text("x")));
        assert_eq!(typed(&schema, "B", &serde_json::json!(true)), Some(FieldValue::Bool(true)));
        assert_eq!(typed(&schema, "F", &serde_json::json!(2.0)), Some(FieldValue::Float(2.0)));
        assert_eq!(typed(&schema, "F", &serde_json::json!(2)), None, "an integer stays text in extra");
        assert_eq!(typed(&schema, "X", &serde_json::json!(2)), None);
        let mut stats = Vec::new();
        merge_stats(
            &mut stats,
            &[StepStats {
                input: 2,
                output: 1,
                errors: 0,
            }],
        );
        merge_stats(
            &mut stats,
            &[StepStats {
                input: 3,
                output: 3,
                errors: 1,
            }],
        );
        assert_eq!((stats[0].input, stats[0].output, stats[0].errors), (5, 4, 1));
    }
}
