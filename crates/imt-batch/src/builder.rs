//! Building batches row by row.

use crate::batch::EventBatch;
use crate::error::BatchError;
use crate::ids::{EventId, EventTime, LevelCode, SourceId};
use crate::schema::{ColumnType, EventSchema, FieldValue};
use crate::BATCH_ROWS;
use arrow_array::builder::{
    BooleanBuilder, Float64Builder, Int64Builder, MapBuilder, StringBuilder, UInt32Builder, UInt64Builder, UInt8Builder,
};
use arrow_array::{Array, ArrayRef, RecordBatch, UInt64Array};
use std::sync::Arc;

/// One event to append, without its id (the store's writer assigns ids).
#[derive(Debug, Clone, Copy)]
pub struct NewEvent<'a> {
    pub time: EventTime,
    pub level: LevelCode,
    pub source: SourceId,
    /// 1-based line (or record number) of the event's first line in its source.
    pub line: u32,
    /// Byte offset of the event's first line in its source file (decompressed for compressed files).
    pub byte_offset: u64,
    pub message: &'a str,
    pub template: Option<&'a str>,
    pub exception: Option<&'a str>,
    /// Bit i set = marker kind i of the generation's `marker_kinds` matched.
    pub markers: u64,
    /// The original event text (all its lines), from which fields can be re-derived.
    pub raw: &'a str,
}

enum DeclaredBuilder {
    Utf8(StringBuilder),
    Int64(Int64Builder),
    Float64(Float64Builder),
    Boolean(BooleanBuilder),
}

impl DeclaredBuilder {
    fn new(ty: ColumnType, capacity: usize) -> DeclaredBuilder {
        match ty {
            ColumnType::Utf8 => DeclaredBuilder::Utf8(StringBuilder::with_capacity(capacity, capacity * 8)),
            ColumnType::Int64 => DeclaredBuilder::Int64(Int64Builder::with_capacity(capacity)),
            ColumnType::Float64 => DeclaredBuilder::Float64(Float64Builder::with_capacity(capacity)),
            ColumnType::Boolean => DeclaredBuilder::Boolean(BooleanBuilder::with_capacity(capacity)),
        }
    }

    /// Appends `value` (which fits the column) or a null.
    fn append(&mut self, value: Option<&FieldValue<'_>>) {
        match (self, value) {
            (DeclaredBuilder::Utf8(b), Some(FieldValue::Text(s))) => b.append_value(s),
            (DeclaredBuilder::Int64(b), Some(FieldValue::Int(i))) => b.append_value(*i),
            (DeclaredBuilder::Float64(b), Some(FieldValue::Float(f))) => b.append_value(*f),
            (DeclaredBuilder::Boolean(b), Some(FieldValue::Bool(v))) => b.append_value(*v),
            (DeclaredBuilder::Utf8(b), _) => b.append_null(),
            (DeclaredBuilder::Int64(b), _) => b.append_null(),
            (DeclaredBuilder::Float64(b), _) => b.append_null(),
            (DeclaredBuilder::Boolean(b), _) => b.append_null(),
        }
    }

    fn finish(&mut self) -> ArrayRef {
        match self {
            DeclaredBuilder::Utf8(b) => Arc::new(b.finish()),
            DeclaredBuilder::Int64(b) => Arc::new(b.finish()),
            DeclaredBuilder::Float64(b) => Arc::new(b.finish()),
            DeclaredBuilder::Boolean(b) => Arc::new(b.finish()),
        }
    }
}

/// Builds a [`PendingBatch`] of at most [`BATCH_ROWS`] events.
pub struct EventBatchBuilder {
    schema: Arc<EventSchema>,
    ts: Int64Builder,
    ts_state: UInt8Builder,
    level: UInt8Builder,
    source: UInt32Builder,
    line: UInt32Builder,
    byte_offset: UInt64Builder,
    message: StringBuilder,
    template: StringBuilder,
    exception: StringBuilder,
    markers: UInt64Builder,
    raw: StringBuilder,
    declared: Vec<DeclaredBuilder>,
    extra: MapBuilder<StringBuilder, StringBuilder>,
    rows: usize,
    /// Per declared column: the value of the row being appended.
    row_values: Vec<Option<FieldValue<'static>>>,
}

impl EventBatchBuilder {
    pub fn new(schema: Arc<EventSchema>) -> EventBatchBuilder {
        let n = 1024;
        let declared = schema.declared().iter().map(|c| DeclaredBuilder::new(c.ty, n)).collect();
        let row_values = vec![None; schema.declared().len()];
        EventBatchBuilder {
            ts: Int64Builder::with_capacity(n),
            ts_state: UInt8Builder::with_capacity(n),
            level: UInt8Builder::with_capacity(n),
            source: UInt32Builder::with_capacity(n),
            line: UInt32Builder::with_capacity(n),
            byte_offset: UInt64Builder::with_capacity(n),
            message: StringBuilder::with_capacity(n, n * 64),
            template: StringBuilder::new(),
            exception: StringBuilder::new(),
            markers: UInt64Builder::with_capacity(n),
            raw: StringBuilder::with_capacity(n, n * 96),
            extra: MapBuilder::new(None, StringBuilder::new(), StringBuilder::new()),
            declared,
            rows: 0,
            row_values,
            schema,
        }
    }

    pub fn schema(&self) -> &Arc<EventSchema> {
        &self.schema
    }

    pub fn len(&self) -> usize {
        self.rows
    }

    pub fn is_empty(&self) -> bool {
        self.rows == 0
    }

    pub fn is_full(&self) -> bool {
        self.rows >= BATCH_ROWS
    }

    /// Appends one event with its fields. A field whose name is declared and whose value fits the
    /// column's type goes to that column (the first such value per row); every other field goes to
    /// `extra` as text.
    pub fn push<'f>(&mut self, ev: &NewEvent<'_>, fields: impl IntoIterator<Item = (&'f str, FieldValue<'f>)>) -> Result<(), BatchError> {
        if self.is_full() {
            return Err(BatchError::TooManyRows {
                rows: self.rows + 1,
                max: BATCH_ROWS,
            });
        }
        self.ts.append_option(ev.time.ts());
        self.ts_state.append_value(ev.time.state().code());
        self.level.append_value(ev.level.get());
        self.source.append_value(ev.source.0);
        self.line.append_value(ev.line);
        self.byte_offset.append_value(ev.byte_offset);
        self.message.append_value(ev.message);
        self.template.append_option(ev.template);
        self.exception.append_option(ev.exception);
        self.markers.append_value(ev.markers);
        self.raw.append_value(ev.raw);

        // Declared values are copied out of `fields` before they are appended, so `row_values` holds
        // no borrow past this call; text values are appended right away instead.
        let mut text_columns: Vec<(usize, &str)> = Vec::new();
        for (name, value) in fields {
            if let Some(i) = self.schema.declared_index(name) {
                let fits = self.schema.declared()[i].ty.fits(&value);
                let free = self.row_values[i].is_none() && !text_columns.iter().any(|(j, _)| *j == i);
                if fits && free {
                    match value {
                        FieldValue::Text(s) => text_columns.push((i, s)),
                        FieldValue::Int(v) => self.row_values[i] = Some(FieldValue::Int(v)),
                        FieldValue::Float(v) => self.row_values[i] = Some(FieldValue::Float(v)),
                        FieldValue::Bool(v) => self.row_values[i] = Some(FieldValue::Bool(v)),
                    }
                    continue;
                }
            }
            self.extra.keys().append_value(name);
            match value {
                FieldValue::Text(s) => self.extra.values().append_value(s),
                other => self.extra.values().append_value(other.render()),
            }
        }
        self.extra.append(true)?;
        for (i, b) in self.declared.iter_mut().enumerate() {
            match text_columns.iter().find(|(j, _)| *j == i) {
                Some((_, s)) => b.append(Some(&FieldValue::Text(s))),
                None => b.append(self.row_values[i].take().as_ref()),
            }
        }
        self.rows += 1;
        Ok(())
    }

    /// The rows appended so far, as a batch without ids; the builder starts over empty.
    pub fn finish(&mut self) -> PendingBatch {
        let mut columns: Vec<ArrayRef> = vec![
            Arc::new(self.ts.finish()),
            Arc::new(self.ts_state.finish()),
            Arc::new(self.level.finish()),
            Arc::new(self.source.finish()),
            Arc::new(self.line.finish()),
            Arc::new(self.byte_offset.finish()),
            Arc::new(self.message.finish()),
            Arc::new(self.template.finish()),
            Arc::new(self.exception.finish()),
            Arc::new(self.markers.finish()),
            Arc::new(self.raw.finish()),
        ];
        columns.extend(self.declared.iter_mut().map(DeclaredBuilder::finish));
        columns.push(Arc::new(self.extra.finish()));
        let rows = std::mem::take(&mut self.rows);
        PendingBatch {
            schema: self.schema.clone(),
            columns,
            rows,
        }
    }
}

/// Events built and not yet written: everything but their ids.
#[derive(Debug, Clone)]
pub struct PendingBatch {
    schema: Arc<EventSchema>,
    columns: Vec<ArrayRef>,
    rows: usize,
}

impl PendingBatch {
    pub fn len(&self) -> usize {
        self.rows
    }

    pub fn is_empty(&self) -> bool {
        self.rows == 0
    }

    pub fn schema(&self) -> &Arc<EventSchema> {
        &self.schema
    }

    /// The batch with ids `first`, `first + 1`, ... in row order.
    pub fn assign_ids(self, first: EventId) -> Result<EventBatch, BatchError> {
        let end = first.0.checked_add(self.rows as u64).ok_or_else(|| BatchError::Row {
            row: 0,
            problem: "event ids are exhausted".into(),
        })?;
        let ids: ArrayRef = Arc::new(UInt64Array::from_iter_values(first.0..end));
        let mut columns = Vec::with_capacity(self.columns.len() + 1);
        columns.push(ids);
        columns.extend(self.columns);
        // The map builder names its parts like the schema does; normalise anything else to the schema's type.
        let extra = self.schema.extra_index();
        if columns[extra].data_type() != self.schema.arrow().field(extra).data_type() {
            return Err(BatchError::ColumnType {
                column: crate::col::EXTRA.into(),
                expected: format!("{}", self.schema.arrow().field(extra).data_type()),
                found: format!("{}", columns[extra].data_type()),
            });
        }
        let rb = RecordBatch::try_new(self.schema.arrow().clone(), columns)?;
        EventBatch::try_new(self.schema, rb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TsState;

    fn event<'a>(message: &'a str, time: EventTime) -> NewEvent<'a> {
        NewEvent {
            time,
            level: LevelCode::new(2).unwrap(),
            source: SourceId(1),
            line: 7,
            byte_offset: 120,
            message,
            template: None,
            exception: Some("boom"),
            markers: 0b101,
            raw: "raw line",
        }
    }

    #[test]
    fn builds_rows_with_declared_and_extra_fields() {
        let schema = Arc::new(EventSchema::new([("Elapsed", ColumnType::Float64), ("Host", ColumnType::Utf8)]).unwrap());
        let mut b = EventBatchBuilder::new(schema.clone());
        assert!(b.is_empty());
        b.push(
            &event("a", EventTime::Parsed(10)),
            [
                ("Elapsed", FieldValue::Float(1.5)),
                ("Host", FieldValue::Text("WEB01")),
                ("Other", FieldValue::Int(3)),
                ("Host", FieldValue::Text("second")),
            ],
        )
        .unwrap();
        // A value of the wrong type for a declared column is kept in `extra`.
        b.push(
            &event("b", EventTime::Untimed),
            [("Elapsed", FieldValue::Text("n/a")), ("Flag", FieldValue::Bool(true))],
        )
        .unwrap();
        assert_eq!(b.len(), 2);
        let batch = b.finish().assign_ids(EventId(100)).unwrap();
        assert!(b.is_empty());
        assert_eq!(batch.num_rows(), 2);
        let r0 = batch.row(0);
        assert_eq!(r0.id(), EventId(100));
        assert_eq!(r0.ts(), Some(10));
        assert_eq!(r0.ts_state(), TsState::Parsed);
        assert_eq!(r0.level().get(), 2);
        assert_eq!(r0.source(), SourceId(1));
        assert_eq!((r0.line(), r0.byte_offset()), (7, 120));
        assert_eq!((r0.message(), r0.template(), r0.exception()), ("a", None, Some("boom")));
        assert_eq!((r0.markers(), r0.raw()), (0b101, "raw line"));
        assert_eq!(r0.field("Elapsed"), Some(FieldValue::Float(1.5)));
        assert_eq!(r0.field("Host"), Some(FieldValue::Text("WEB01")));
        assert_eq!(r0.field("Other"), Some(FieldValue::Text("3")));
        let fields: Vec<_> = r0.fields().collect();
        assert_eq!(fields.len(), 4, "{fields:?}");
        let r1 = batch.row(1);
        assert_eq!(r1.id(), EventId(101));
        assert_eq!((r1.ts(), r1.ts_state()), (None, TsState::Untimed));
        assert_eq!(r1.field("Elapsed"), Some(FieldValue::Text("n/a")));
        assert_eq!(r1.field("Flag"), Some(FieldValue::Text("true")));
        assert_eq!(r1.field("Host"), None);
        assert_eq!(r1.field("missing"), None);
    }

    #[test]
    fn a_full_builder_refuses_more_rows() {
        let mut b = EventBatchBuilder::new(Arc::new(EventSchema::plain()));
        for _ in 0..BATCH_ROWS {
            b.push(&event("x", EventTime::Parsed(1)), []).unwrap();
        }
        assert!(b.is_full());
        assert!(matches!(
            b.push(&event("x", EventTime::Parsed(1)), []),
            Err(BatchError::TooManyRows { .. })
        ));
        let pending = b.finish();
        assert_eq!(pending.len(), BATCH_ROWS);
        assert!(!pending.is_empty());
        assert!(pending.schema().declared().is_empty());
        assert!(pending.clone().assign_ids(EventId(u64::MAX - 3)).is_err(), "ids would overflow");
        let batch = pending.assign_ids(EventId(0)).unwrap();
        assert_eq!(batch.row(BATCH_ROWS - 1).id(), EventId(BATCH_ROWS as u64 - 1));
    }

    #[test]
    fn all_declared_types_round_trip() {
        let schema = Arc::new(
            EventSchema::new([
                ("I", ColumnType::Int64),
                ("F", ColumnType::Float64),
                ("B", ColumnType::Boolean),
                ("S", ColumnType::Utf8),
            ])
            .unwrap(),
        );
        let mut b = EventBatchBuilder::new(schema);
        let fields = [
            ("I", FieldValue::Int(-4)),
            ("F", FieldValue::Float(0.25)),
            ("B", FieldValue::Bool(false)),
            ("S", FieldValue::Text("s")),
        ];
        b.push(&event("m", EventTime::Inherited(5)), fields).unwrap();
        b.push(&event("n", EventTime::Parsed(6)), []).unwrap();
        let batch = b.finish().assign_ids(EventId(1)).unwrap();
        let r = batch.row(0);
        for (name, value) in fields {
            assert_eq!(r.field(name), Some(value), "{name}");
        }
        assert_eq!(r.ts_state(), TsState::Inherited);
        assert_eq!(batch.row(1).fields().count(), 0);
    }
}
