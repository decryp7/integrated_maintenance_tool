//! A validated batch of events and row access to it.

use crate::error::BatchError;
use crate::ids::{EventId, LevelCode, SourceId, TsState};
use crate::schema::{col, ColumnType, EventSchema, FieldValue, FIXED};
use crate::BATCH_ROWS;
use arrow_array::cast::AsArray;
use arrow_array::types::{Float64Type, Int64Type, UInt32Type, UInt64Type, UInt8Type};
use arrow_array::{Array, BooleanArray, Float64Array, Int64Array, MapArray, PrimitiveArray, RecordBatch, StringArray};
use std::sync::Arc;

#[derive(Debug, Clone)]
enum DeclaredArray {
    Utf8(StringArray),
    Int64(Int64Array),
    Float64(Float64Array),
    Boolean(BooleanArray),
}

impl DeclaredArray {
    fn value(&self, row: usize) -> Option<FieldValue<'_>> {
        let valid = |a: &dyn Array| a.is_valid(row);
        match self {
            DeclaredArray::Utf8(a) => valid(a).then(|| FieldValue::Text(a.value(row))),
            DeclaredArray::Int64(a) => valid(a).then(|| FieldValue::Int(a.value(row))),
            DeclaredArray::Float64(a) => valid(a).then(|| FieldValue::Float(a.value(row))),
            DeclaredArray::Boolean(a) => valid(a).then(|| FieldValue::Bool(a.value(row))),
        }
    }
}

/// The columns, downcast once.
#[derive(Debug, Clone)]
struct Columns {
    id: PrimitiveArray<UInt64Type>,
    ts: PrimitiveArray<Int64Type>,
    ts_state: PrimitiveArray<UInt8Type>,
    level: PrimitiveArray<UInt8Type>,
    source: PrimitiveArray<UInt32Type>,
    line: PrimitiveArray<UInt32Type>,
    byte_offset: PrimitiveArray<UInt64Type>,
    message: StringArray,
    template: StringArray,
    exception: StringArray,
    markers: PrimitiveArray<UInt64Type>,
    raw: StringArray,
    declared: Vec<DeclaredArray>,
    extra: MapArray,
    extra_keys: StringArray,
    extra_values: StringArray,
}

/// Events with ids, in the [`EventSchema`] layout, whose invariants were checked:
/// at most [`BATCH_ROWS`] rows; no nulls in required columns; valid level and `ts_state` codes; `ts` null
/// exactly when the event is untimed.
#[derive(Debug, Clone)]
pub struct EventBatch {
    schema: Arc<EventSchema>,
    batch: RecordBatch,
    cols: Columns,
}

fn check_name(rb: &RecordBatch, index: usize, expected: &str) -> Result<(), BatchError> {
    let found = rb.schema().field(index).name().clone();
    if found != expected {
        return Err(BatchError::ColumnName {
            index,
            expected: expected.into(),
            found,
        });
    }
    Ok(())
}

fn type_error(rb: &RecordBatch, index: usize, expected: &str) -> BatchError {
    BatchError::ColumnType {
        column: rb.schema().field(index).name().clone(),
        expected: expected.into(),
        found: format!("{}", rb.column(index).data_type()),
    }
}

fn primitive<T: arrow_array::ArrowPrimitiveType>(
    rb: &RecordBatch,
    index: usize,
    name: &str,
    nullable: bool,
) -> Result<PrimitiveArray<T>, BatchError> {
    check_name(rb, index, name)?;
    let a = rb
        .column(index)
        .as_primitive_opt::<T>()
        .ok_or_else(|| type_error(rb, index, &format!("{}", T::DATA_TYPE)))?
        .clone();
    if !nullable && a.null_count() > 0 {
        return Err(BatchError::Null { column: name.into() });
    }
    Ok(a)
}

fn utf8(rb: &RecordBatch, index: usize, name: &str, nullable: bool) -> Result<StringArray, BatchError> {
    check_name(rb, index, name)?;
    let a = rb
        .column(index)
        .as_string_opt::<i32>()
        .ok_or_else(|| type_error(rb, index, "Utf8"))?
        .clone();
    if !nullable && a.null_count() > 0 {
        return Err(BatchError::Null { column: name.into() });
    }
    Ok(a)
}

impl EventBatch {
    /// Checks that `batch` has `schema`'s columns (names and types; the map column's inner field names
    /// may differ) and the batch invariants.
    pub fn try_new(schema: Arc<EventSchema>, batch: RecordBatch) -> Result<EventBatch, BatchError> {
        let rows = batch.num_rows();
        if rows > BATCH_ROWS {
            return Err(BatchError::TooManyRows { rows, max: BATCH_ROWS });
        }
        let expected = schema.arrow().fields().len();
        if batch.num_columns() != expected {
            return Err(BatchError::ColumnCount {
                expected,
                found: batch.num_columns(),
            });
        }
        let rb = &batch;
        let mut declared = Vec::with_capacity(schema.declared().len());
        for (i, c) in schema.declared().iter().enumerate() {
            let index = FIXED + i;
            check_name(rb, index, &c.column)?;
            let a = rb.column(index);
            let wrong = || type_error(rb, index, &format!("{}", c.ty.arrow()));
            declared.push(match c.ty {
                ColumnType::Utf8 => DeclaredArray::Utf8(a.as_string_opt::<i32>().ok_or_else(wrong)?.clone()),
                ColumnType::Int64 => DeclaredArray::Int64(a.as_primitive_opt::<Int64Type>().ok_or_else(wrong)?.clone()),
                ColumnType::Float64 => DeclaredArray::Float64(a.as_primitive_opt::<Float64Type>().ok_or_else(wrong)?.clone()),
                ColumnType::Boolean => DeclaredArray::Boolean(a.as_boolean_opt().ok_or_else(wrong)?.clone()),
            });
        }
        let extra_index = schema.extra_index();
        check_name(rb, extra_index, col::EXTRA)?;
        let extra = rb
            .column(extra_index)
            .as_map_opt()
            .ok_or_else(|| type_error(rb, extra_index, "Map(Utf8, Utf8)"))?
            .clone();
        let (Some(extra_keys), Some(extra_values)) = (extra.keys().as_string_opt::<i32>(), extra.values().as_string_opt::<i32>()) else {
            return Err(type_error(rb, extra_index, "Map(Utf8, Utf8)"));
        };
        if extra_keys.null_count() > 0 {
            return Err(BatchError::Null {
                column: "extra keys".into(),
            });
        }
        let (extra_keys, extra_values) = (extra_keys.clone(), extra_values.clone());
        let cols = Columns {
            id: primitive::<UInt64Type>(rb, 0, col::ID, false)?,
            ts: primitive::<Int64Type>(rb, 1, col::TS, true)?,
            ts_state: primitive::<UInt8Type>(rb, 2, col::TS_STATE, false)?,
            level: primitive::<UInt8Type>(rb, 3, col::LEVEL, false)?,
            source: primitive::<UInt32Type>(rb, 4, col::SOURCE, false)?,
            line: primitive::<UInt32Type>(rb, 5, col::LINE, false)?,
            byte_offset: primitive::<UInt64Type>(rb, 6, col::BYTE_OFFSET, false)?,
            message: utf8(rb, 7, col::MESSAGE, false)?,
            template: utf8(rb, 8, col::TEMPLATE, true)?,
            exception: utf8(rb, 9, col::EXCEPTION, true)?,
            markers: primitive::<UInt64Type>(rb, 10, col::MARKERS, false)?,
            raw: utf8(rb, 11, col::RAW, false)?,
            declared,
            extra,
            extra_keys,
            extra_values,
        };
        for row in 0..rows {
            if LevelCode::new(cols.level.value(row)).is_none() {
                return Err(BatchError::Row {
                    row,
                    problem: format!("level code {} is not 0..=5", cols.level.value(row)),
                });
            }
            let state = TsState::from_code(cols.ts_state.value(row)).ok_or_else(|| BatchError::Row {
                row,
                problem: format!("ts_state {} is not 0..=2", cols.ts_state.value(row)),
            })?;
            if (state == TsState::Untimed) != cols.ts.is_null(row) {
                return Err(BatchError::Row {
                    row,
                    problem: "ts must be null exactly when the event is untimed".into(),
                });
            }
        }
        Ok(EventBatch { schema, batch, cols })
    }

    pub fn schema(&self) -> &Arc<EventSchema> {
        &self.schema
    }

    pub fn record_batch(&self) -> &RecordBatch {
        &self.batch
    }

    pub fn into_record_batch(self) -> RecordBatch {
        self.batch
    }

    /// The batch with every timestamp moved by `offset` milliseconds (saturating); untimed events stay untimed.
    /// Clock-skew correction applies a source's offset to the events as they are written.
    pub fn shifted(&self, offset: i64) -> Result<EventBatch, BatchError> {
        if offset == 0 {
            return Ok(self.clone());
        }
        let ts: Int64Array = self.cols.ts.iter().map(|t| t.map(|v| v.saturating_add(offset))).collect();
        let mut columns = self.batch.columns().to_vec();
        columns[1] = Arc::new(ts);
        let batch = RecordBatch::try_new(self.batch.schema(), columns)?;
        EventBatch::try_new(self.schema.clone(), batch)
    }

    pub fn num_rows(&self) -> usize {
        self.batch.num_rows()
    }

    pub fn is_empty(&self) -> bool {
        self.num_rows() == 0
    }

    /// Row `index` (< [`EventBatch::num_rows`]).
    pub fn row(&self, index: usize) -> BatchRow<'_> {
        assert!(index < self.num_rows(), "row {index} of a batch of {}", self.num_rows());
        BatchRow { batch: self, index }
    }

    pub fn rows(&self) -> impl ExactSizeIterator<Item = BatchRow<'_>> {
        (0..self.num_rows()).map(|index| BatchRow { batch: self, index })
    }
}

/// One event of an [`EventBatch`].
#[derive(Debug, Clone, Copy)]
pub struct BatchRow<'a> {
    batch: &'a EventBatch,
    index: usize,
}

impl<'a> BatchRow<'a> {
    fn cols(&self) -> &'a Columns {
        &self.batch.cols
    }

    pub fn index(&self) -> usize {
        self.index
    }

    pub fn batch(&self) -> &'a EventBatch {
        self.batch
    }

    pub fn id(&self) -> EventId {
        EventId(self.cols().id.value(self.index))
    }

    pub fn ts(&self) -> Option<i64> {
        let ts = &self.cols().ts;
        ts.is_valid(self.index).then(|| ts.value(self.index))
    }

    pub fn ts_state(&self) -> TsState {
        TsState::from_code(self.cols().ts_state.value(self.index)).unwrap_or(TsState::Untimed)
    }

    pub fn level(&self) -> LevelCode {
        LevelCode::new(self.cols().level.value(self.index)).unwrap_or(LevelCode::FATAL)
    }

    pub fn source(&self) -> SourceId {
        SourceId(self.cols().source.value(self.index))
    }

    pub fn line(&self) -> u32 {
        self.cols().line.value(self.index)
    }

    pub fn byte_offset(&self) -> u64 {
        self.cols().byte_offset.value(self.index)
    }

    pub fn message(&self) -> &'a str {
        self.cols().message.value(self.index)
    }

    pub fn template(&self) -> Option<&'a str> {
        let a = &self.cols().template;
        a.is_valid(self.index).then(|| a.value(self.index))
    }

    pub fn exception(&self) -> Option<&'a str> {
        let a = &self.cols().exception;
        a.is_valid(self.index).then(|| a.value(self.index))
    }

    pub fn markers(&self) -> u64 {
        self.cols().markers.value(self.index)
    }

    pub fn raw(&self) -> &'a str {
        self.cols().raw.value(self.index)
    }

    /// Value of a declared column, by its index in [`EventSchema::declared`].
    pub fn declared(&self, column: usize) -> Option<FieldValue<'a>> {
        self.cols().declared.get(column).and_then(|a| a.value(self.index))
    }

    /// Value of `extra[name]`.
    pub fn extra(&self, name: &str) -> Option<&'a str> {
        let c = self.cols();
        let offsets = c.extra.value_offsets();
        let (start, end) = (offsets[self.index] as usize, offsets[self.index + 1] as usize);
        (start..end)
            .find(|&i| c.extra_keys.value(i) == name)
            .and_then(|i| c.extra_values.is_valid(i).then(|| c.extra_values.value(i)))
    }

    /// A field by name: its declared column, else `extra`.
    pub fn field(&self, name: &str) -> Option<FieldValue<'a>> {
        self.batch
            .schema
            .declared_index(name)
            .and_then(|i| self.declared(i))
            .or_else(|| self.extra(name).map(FieldValue::Text))
    }

    /// Every field of the event: declared columns with a value, then `extra` entries.
    pub fn fields(&self) -> impl Iterator<Item = (&'a str, FieldValue<'a>)> + 'a {
        let row = *self;
        let c = self.cols();
        let declared = self.batch.schema.declared();
        let offsets = c.extra.value_offsets();
        let (start, end) = (offsets[self.index] as usize, offsets[self.index + 1] as usize);
        let typed = declared
            .iter()
            .enumerate()
            .filter_map(move |(i, d)| row.declared(i).map(|v| (d.name.as_str(), v)));
        let extra = (start..end)
            .filter(move |&i| c.extra_values.is_valid(i))
            .map(move |i| (c.extra_keys.value(i), FieldValue::Text(c.extra_values.value(i))));
        typed.chain(extra)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::{EventBatchBuilder, NewEvent};
    use crate::ids::EventTime;
    use arrow_array::{ArrayRef, UInt8Array};
    use arrow_schema::{DataType, Field, Schema};

    fn sample() -> EventBatch {
        let schema = Arc::new(EventSchema::new([("N", ColumnType::Int64)]).unwrap());
        let mut b = EventBatchBuilder::new(schema);
        for (i, time) in [EventTime::Parsed(1), EventTime::Untimed].into_iter().enumerate() {
            let ev = NewEvent {
                time,
                level: LevelCode::new(i as u8).unwrap(),
                source: SourceId(0),
                line: i as u32 + 1,
                byte_offset: 0,
                message: "m",
                template: Some("t"),
                exception: None,
                markers: 0,
                raw: "m",
            };
            b.push(&ev, [("N", FieldValue::Int(i as i64)), ("X", FieldValue::Text("x"))])
                .unwrap();
        }
        b.finish().assign_ids(EventId(0)).unwrap()
    }

    /// `batch` with column `index` replaced.
    fn with_column(batch: &EventBatch, index: usize, column: ArrayRef, field: Field) -> RecordBatch {
        let mut columns = batch.record_batch().columns().to_vec();
        columns[index] = column;
        let mut fields: Vec<Field> = batch.schema().arrow().fields().iter().map(|f| f.as_ref().clone()).collect();
        fields[index] = field;
        RecordBatch::try_new(Arc::new(Schema::new(fields)), columns).unwrap()
    }

    #[test]
    fn accepts_its_own_batches() {
        let b = sample();
        let again = EventBatch::try_new(b.schema().clone(), b.record_batch().clone()).unwrap();
        assert_eq!(again.num_rows(), 2);
        assert!(!again.is_empty());
        assert_eq!(again.rows().map(|r| r.line()).collect::<Vec<_>>(), [1, 2]);
        let r = again.row(1);
        assert_eq!((r.index(), r.template(), r.exception()), (1, Some("t"), None));
        assert_eq!(r.batch().num_rows(), 2);
        assert_eq!(r.extra("X"), Some("x"));
        assert_eq!(r.extra("N"), None, "declared, not extra");
        assert_eq!(again.into_record_batch().num_columns(), 14);
    }

    #[test]
    fn shifting_moves_timed_events_only() {
        let b = sample();
        assert_eq!(b.shifted(0).unwrap().row(0).ts(), Some(1));
        let s = b.shifted(-5).unwrap();
        assert_eq!((s.row(0).ts(), s.row(1).ts()), (Some(-4), None));
        assert_eq!(s.row(0).message(), "m");
        assert_eq!(b.row(0).ts(), Some(1), "the original is unchanged");
        let big = b.shifted(i64::MAX).unwrap().shifted(i64::MAX).unwrap();
        assert_eq!(big.row(0).ts(), Some(i64::MAX), "saturates");
    }

    #[test]
    fn refuses_broken_batches() {
        let b = sample();
        let schema = b.schema().clone();
        // Wrong level code.
        let bad_level = with_column(
            &b,
            3,
            Arc::new(UInt8Array::from(vec![0, 9])),
            Field::new("level", DataType::UInt8, false),
        );
        assert!(matches!(
            EventBatch::try_new(schema.clone(), bad_level),
            Err(BatchError::Row { row: 1, .. })
        ));
        // Wrong ts_state code.
        let bad_state = with_column(
            &b,
            2,
            Arc::new(UInt8Array::from(vec![0, 7])),
            Field::new("ts_state", DataType::UInt8, false),
        );
        assert!(EventBatch::try_new(schema.clone(), bad_state).is_err());
        // Untimed with a ts.
        let both = with_column(
            &b,
            1,
            Arc::new(Int64Array::from(vec![1, 2])),
            Field::new("ts", DataType::Int64, true),
        );
        assert!(matches!(EventBatch::try_new(schema.clone(), both), Err(BatchError::Row { .. })));
        // Null in a required column.
        let null_line = with_column(
            &b,
            5,
            Arc::new(arrow_array::UInt32Array::from(vec![Some(1), None])),
            Field::new("line", DataType::UInt32, true),
        );
        assert!(matches!(
            EventBatch::try_new(schema.clone(), null_line),
            Err(BatchError::Null { .. })
        ));
        // Wrong type and wrong name.
        let wrong_type = with_column(
            &b,
            5,
            Arc::new(Int64Array::from(vec![1, 2])),
            Field::new("line", DataType::Int64, false),
        );
        assert!(matches!(
            EventBatch::try_new(schema.clone(), wrong_type),
            Err(BatchError::ColumnType { .. })
        ));
        let wrong_name = with_column(
            &b,
            5,
            Arc::new(arrow_array::UInt32Array::from(vec![1, 2])),
            Field::new("lines", DataType::UInt32, false),
        );
        assert!(matches!(
            EventBatch::try_new(schema.clone(), wrong_name),
            Err(BatchError::ColumnName { .. })
        ));
        let wrong_declared = with_column(
            &b,
            12,
            Arc::new(StringArray::from(vec!["a", "b"])),
            Field::new("f_N", DataType::Utf8, true),
        );
        assert!(matches!(
            EventBatch::try_new(schema.clone(), wrong_declared),
            Err(BatchError::ColumnType { .. })
        ));
        let not_map = with_column(
            &b,
            13,
            Arc::new(StringArray::from(vec!["a", "b"])),
            Field::new("extra", DataType::Utf8, false),
        );
        assert!(EventBatch::try_new(schema.clone(), not_map).is_err());
        // Wrong column count.
        assert!(matches!(
            EventBatch::try_new(Arc::new(EventSchema::plain()), b.record_batch().clone()),
            Err(BatchError::ColumnCount { .. })
        ));
    }
}
