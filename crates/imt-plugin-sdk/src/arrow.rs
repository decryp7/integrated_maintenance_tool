//! IMT records as Arrow record batches, the data plane of `imt.plugin.v1`.
//!
//! One batch is one Arrow IPC *stream* (a schema message and one record batch). Columns, in order:
//!
//! | Column | Arrow type | Record field |
//! |---|---|---|
//! | `ts` | Int64, nullable | `ts` (ms since the Unix epoch, UTC) |
//! | `level` | UInt8 | `level` as its index: 0 Verbose, 1 Debug, 2 Information, 3 Warning, 4 Error, 5 Fatal |
//! | `message` | Utf8 | `message` |
//! | `template` | Utf8, nullable | `template` |
//! | `exception` | Utf8, nullable | `exception` |
//! | `line` | UInt32 | `line` |
//! | `id` | UInt32 | `id` (assigned by IMT; 0 from importers) |
//! | `source` | UInt32 | `source` (assigned by IMT; 0 from importers) |
//! | `markers` | UInt64 | `markers` (assigned by IMT; plugins mark through verdict replies) |
//!
//! then the record's `fields` (see below), then any extra Utf8 columns a call names (`source_name`,
//! `group`, `instance`).
//!
//! **Fields.** A record's fields are an ordered map of name to any JSON value, and names differ from file
//! to file. Each batch picks one of two layouts, which a reader tells apart by the column names:
//!
//! - *Columnar* (the usual case): one nullable column per field name used in the batch, in first-seen
//!   order. `f:<name>` columns hold values of one type: Utf8 (strings), Int64, UInt64 (integers above
//!   `i64::MAX`), Float64 or Boolean. `j:<name>` columns hold any other value (null, objects, arrays, or a
//!   mix of types) as JSON text. A null cell means the record has no such field. A record whose field
//!   order differs from the column order has `field_order` (Utf8, nullable): the column indexes in its
//!   order, comma-separated.
//! - *JSON* (when a batch uses more than 256 names, or the names are sparse): one `fields` column (Utf8,
//!   nullable) with each record's fields as a JSON object.
//!
//! Both are lossless (value types, integer vs. float, and field order are kept). The columnar layout is
//! the reason out-of-process imports stay close to in-process speed: string values (most fields) cross
//! as plain Arrow strings, with no JSON to write or parse, and a batch is about half the size of the JSON
//! layout. Rows without fields cost a null bit per column.
//!
//! Batches are written with LZ4-frame buffer compression (part of the Arrow IPC format; readers must
//! accept compressed and uncompressed batches). Decoding reads straight from the received `Bytes` (no
//! copy of uncompressed buffers) and validates everything: column types,
//! row counts, level range, field order indexes, and JSON text.

use anyhow::{anyhow, bail, Context, Result};
use arrow_array::builder::{BooleanBuilder, Float64Builder, Int64Builder, StringBuilder, UInt32Builder, UInt64Builder, UInt8Builder};
use arrow_array::{
    Array, ArrayRef, BooleanArray, Float64Array, Int64Array, RecordBatch as ArrowBatch, StringArray, UInt32Array, UInt64Array, UInt8Array,
};
use arrow_buffer::Buffer;
use arrow_ipc::reader::StreamDecoder;
use arrow_ipc::writer::StreamWriter;
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use bytes::Bytes;
use imt_core::model::{Fields, Level, Record};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// Rows per batch of imported records: large enough that per-message costs vanish, small enough to
/// pipeline encode, transfer and decode, and far below the message size limit (tuned with the transport
/// benchmark of `imt-plugin-host`).
pub const DEFAULT_BATCH_ROWS: usize = 32 * 1024;
/// Rows per batch sent to a transform: smaller, because a transform's work is cheap and both directions
/// should overlap.
pub const TRANSFORM_BATCH_ROWS: usize = 16 * 1024;
/// A batch is closed early once its text reaches this many bytes.
pub const BATCH_BYTES: usize = 24 * 1024 * 1024;
/// Most rows a received batch may hold.
pub const MAX_BATCH_ROWS: usize = 1 << 20;
/// Most field names a batch lays out as columns (more: the JSON layout).
pub const MAX_FIELD_COLUMNS: usize = 256;

/// Names of the record columns, in order (the fields and extra columns follow).
pub const RECORD_COLUMNS: [&str; 9] = ["ts", "level", "message", "template", "exception", "line", "id", "source", "markers"];

fn record_fields() -> Vec<Field> {
    vec![
        Field::new("ts", DataType::Int64, true),
        Field::new("level", DataType::UInt8, false),
        Field::new("message", DataType::Utf8, false),
        Field::new("template", DataType::Utf8, true),
        Field::new("exception", DataType::Utf8, true),
        Field::new("line", DataType::UInt32, false),
        Field::new("id", DataType::UInt32, false),
        Field::new("source", DataType::UInt32, false),
        Field::new("markers", DataType::UInt64, false),
    ]
}

/// The schema of a batch in the JSON fields layout with `extra` Utf8 columns.
pub fn schema(extra: &[&str]) -> SchemaRef {
    let mut fields = record_fields();
    fields.push(Field::new("fields", DataType::Utf8, true));
    fields.extend(extra.iter().map(|n| Field::new(*n, DataType::Utf8, false)));
    Arc::new(Schema::new(fields))
}

/// The type a field column holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Str,
    I64,
    U64,
    F64,
    Bool,
    Json,
}

fn kind_of(v: &Value) -> Kind {
    match v {
        Value::String(_) => Kind::Str,
        Value::Bool(_) => Kind::Bool,
        Value::Number(n) if n.is_i64() => Kind::I64,
        Value::Number(n) if n.is_u64() => Kind::U64,
        Value::Number(_) => Kind::F64,
        _ => Kind::Json,
    }
}

/// The field columns of a batch (columnar layout), or `None` for the JSON layout.
fn field_columns<R: Row>(rows: &[R]) -> Result<Option<Vec<(Field, ArrayRef)>>> {
    let n = rows.len();
    let mut index: HashMap<&str, usize> = HashMap::new();
    let mut keys: Vec<(&str, Kind)> = Vec::new();
    let mut present = 0usize;
    let mut reordered = false;
    for row in rows {
        let mut last: Option<usize> = None;
        for (k, v) in &row.record().fields {
            let next = index.len();
            let i = *index.entry(k.as_str()).or_insert(next);
            if i == keys.len() {
                if keys.len() == MAX_FIELD_COLUMNS {
                    return Ok(None);
                }
                keys.push((k.as_str(), kind_of(v)));
            } else if keys[i].1 != kind_of(v) {
                keys[i].1 = Kind::Json;
            }
            reordered |= last.is_some_and(|l| l >= i);
            last = Some(i);
            present += 1;
        }
    }
    if keys.is_empty() {
        return Ok(Some(Vec::new()));
    }
    // Sparse names (each record has few of many): one JSON object per record is smaller.
    if keys.len() > 16 && present * 4 < n * keys.len() {
        return Ok(None);
    }
    enum B {
        Str(StringBuilder),
        I64(Int64Builder),
        U64(UInt64Builder),
        F64(Float64Builder),
        Bool(BooleanBuilder),
        Json(StringBuilder),
    }
    let mut builders: Vec<B> = keys
        .iter()
        .map(|(_, k)| match k {
            Kind::Str => B::Str(StringBuilder::with_capacity(n, n * 8)),
            Kind::I64 => B::I64(Int64Builder::with_capacity(n)),
            Kind::U64 => B::U64(UInt64Builder::with_capacity(n)),
            Kind::F64 => B::F64(Float64Builder::with_capacity(n)),
            Kind::Bool => B::Bool(BooleanBuilder::with_capacity(n)),
            Kind::Json => B::Json(StringBuilder::with_capacity(n, n * 8)),
        })
        .collect();
    let mut order = StringBuilder::with_capacity(if reordered { n } else { 0 }, 0);
    let mut slots: Vec<Option<&Value>> = vec![None; keys.len()];
    let mut seq: Vec<usize> = Vec::new();
    let mut json = Vec::new();
    for row in rows {
        slots.iter_mut().for_each(|s| *s = None);
        seq.clear();
        for (k, v) in &row.record().fields {
            let i = index[k.as_str()];
            slots[i] = Some(v);
            seq.push(i);
        }
        for (slot, b) in slots.iter().zip(builders.iter_mut()) {
            match (b, slot) {
                (B::Str(b), Some(Value::String(s))) => b.append_value(s),
                (B::I64(b), Some(Value::Number(x))) => b.append_option(x.as_i64()),
                (B::U64(b), Some(Value::Number(x))) => b.append_option(x.as_u64()),
                (B::F64(b), Some(Value::Number(x))) => b.append_option(x.as_f64()),
                (B::Bool(b), Some(Value::Bool(x))) => b.append_value(*x),
                (B::Json(b), Some(v)) => {
                    json.clear();
                    serde_json::to_writer(&mut json, v)?;
                    b.append_value(std::str::from_utf8(&json)?);
                }
                (B::Str(b) | B::Json(b), _) => b.append_null(),
                (B::I64(b), _) => b.append_null(),
                (B::U64(b), _) => b.append_null(),
                (B::F64(b), _) => b.append_null(),
                (B::Bool(b), _) => b.append_null(),
            }
        }
        if reordered {
            if seq.windows(2).all(|w| w[0] < w[1]) {
                order.append_null();
            } else {
                order.append_value(seq.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(","));
            }
        }
    }
    let mut out: Vec<(Field, ArrayRef)> = keys
        .iter()
        .zip(builders)
        .map(|((name, _), b)| -> (Field, ArrayRef) {
            match b {
                B::Str(mut b) => (Field::new(format!("f:{name}"), DataType::Utf8, true), Arc::new(b.finish())),
                B::I64(mut b) => (Field::new(format!("f:{name}"), DataType::Int64, true), Arc::new(b.finish())),
                B::U64(mut b) => (Field::new(format!("f:{name}"), DataType::UInt64, true), Arc::new(b.finish())),
                B::F64(mut b) => (Field::new(format!("f:{name}"), DataType::Float64, true), Arc::new(b.finish())),
                B::Bool(mut b) => (Field::new(format!("f:{name}"), DataType::Boolean, true), Arc::new(b.finish())),
                B::Json(mut b) => (Field::new(format!("j:{name}"), DataType::Utf8, true), Arc::new(b.finish())),
            }
        })
        .collect();
    if reordered {
        out.push((Field::new("field_order", DataType::Utf8, true), Arc::new(order.finish())));
    }
    Ok(Some(out))
}

/// Rows to encode: a record and the values of the extra columns.
pub trait Row {
    fn record(&self) -> &Record;
    /// Value of extra column `i`.
    fn extra(&self, _i: usize) -> &str {
        ""
    }
}

impl Row for Record {
    fn record(&self) -> &Record {
        self
    }
}

impl Row for &Record {
    fn record(&self) -> &Record {
        self
    }
}

/// A record with extra column values.
pub struct WithExtra<'a, const N: usize> {
    pub record: &'a Record,
    pub extra: [&'a str; N],
}

impl<const N: usize> Row for WithExtra<'_, N> {
    fn record(&self) -> &Record {
        self.record
    }
    fn extra(&self, i: usize) -> &str {
        self.extra.get(i).copied().unwrap_or("")
    }
}

/// Encodes `rows` as one Arrow IPC stream with the record columns and the `extra` columns.
pub fn encode<R: Row>(rows: &[R], extra: &[&str]) -> Result<Bytes> {
    let n = rows.len();
    let text: usize = rows.iter().map(|r| r.record().message.len()).sum();
    let mut ts = Int64Builder::with_capacity(n);
    let mut level = UInt8Builder::with_capacity(n);
    let mut message = StringBuilder::with_capacity(n, text);
    let mut template = StringBuilder::with_capacity(n, 0);
    let mut exception = StringBuilder::with_capacity(n, 0);
    let mut line = UInt32Builder::with_capacity(n);
    let mut id = UInt32Builder::with_capacity(n);
    let mut source = UInt32Builder::with_capacity(n);
    let mut markers = UInt64Builder::with_capacity(n);
    let mut extras: Vec<StringBuilder> = extra.iter().map(|_| StringBuilder::with_capacity(n, n * 8)).collect();
    let columnar = field_columns(rows)?;
    let mut fields = StringBuilder::with_capacity(if columnar.is_some() { 0 } else { n }, 0);
    let mut json = Vec::with_capacity(256);
    for row in rows {
        let r = row.record();
        ts.append_option(r.ts);
        level.append_value(r.level.index() as u8);
        message.append_value(&r.message);
        template.append_option(r.template.as_deref());
        exception.append_option(r.exception.as_deref());
        if columnar.is_none() {
            if r.fields.is_empty() {
                fields.append_null();
            } else {
                json.clear();
                serde_json::to_writer(&mut json, &r.fields)?;
                // serde_json writes valid UTF-8.
                fields.append_value(std::str::from_utf8(&json)?);
            }
        }
        line.append_value(r.line);
        id.append_value(r.id);
        source.append_value(r.source);
        markers.append_value(r.markers);
        for (i, b) in extras.iter_mut().enumerate() {
            b.append_value(row.extra(i));
        }
    }
    let mut schema_fields = record_fields();
    let mut columns: Vec<ArrayRef> = vec![
        Arc::new(ts.finish()),
        Arc::new(level.finish()),
        Arc::new(message.finish()),
        Arc::new(template.finish()),
        Arc::new(exception.finish()),
        Arc::new(line.finish()),
        Arc::new(id.finish()),
        Arc::new(source.finish()),
        Arc::new(markers.finish()),
    ];
    match columnar {
        Some(cols) => {
            for (f, c) in cols {
                schema_fields.push(f);
                columns.push(c);
            }
        }
        None => {
            schema_fields.push(Field::new("fields", DataType::Utf8, true));
            columns.push(Arc::new(fields.finish()));
        }
    }
    for (name, b) in extra.iter().zip(extras.iter_mut()) {
        schema_fields.push(Field::new(*name, DataType::Utf8, false));
        columns.push(Arc::new(b.finish()));
    }
    let schema = Arc::new(Schema::new(schema_fields));
    let batch = ArrowBatch::try_new(schema.clone(), columns)?;
    let mut out = Vec::with_capacity(text + n * 48 + 1024);
    {
        // LZ4 frames: a batch shrinks about 4x for a few percent of encode time, which more than pays for
        // itself in copies and socket traffic (see the transport benchmark). Readers accept both.
        let opts = arrow_ipc::writer::IpcWriteOptions::default().try_with_compression(Some(arrow_ipc::CompressionType::LZ4_FRAME))?;
        let mut w = StreamWriter::try_new_with_options(&mut out, &schema, opts)?;
        w.write(&batch)?;
        w.finish()?;
    }
    Ok(Bytes::from(out))
}

/// A decoded batch: the records and the extra columns' values (one `Vec` per requested column).
#[derive(Debug, Default)]
pub struct Decoded {
    pub records: Vec<Record>,
    pub extra: Vec<Vec<String>>,
}

fn column<'a, T: 'static>(b: &'a ArrowBatch, name: &str) -> Result<&'a T> {
    b.column_by_name(name)
        .ok_or_else(|| anyhow!("the batch has no \"{name}\" column"))?
        .as_any()
        .downcast_ref::<T>()
        .ok_or_else(|| anyhow!("column \"{name}\" has the wrong type"))
}

fn opt_text(a: &StringArray, i: usize) -> Option<String> {
    (!a.is_null(i)).then(|| a.value(i).to_string())
}

/// Decodes one batch (zero-copy from `bytes`), requiring the `extra` columns.
pub fn decode(bytes: Bytes, extra: &[&str]) -> Result<Decoded> {
    let mut buffer = Buffer::from(bytes);
    let mut decoder = StreamDecoder::new();
    let mut batches = Vec::new();
    while !buffer.is_empty() {
        match decoder.decode(&mut buffer).context("the record batch is not valid Arrow IPC")? {
            Some(b) => batches.push(b),
            None => break,
        }
    }
    decoder.finish().context("the record batch is truncated")?;
    let mut out = Decoded {
        records: Vec::new(),
        extra: vec![Vec::new(); extra.len()],
    };
    for b in &batches {
        decode_batch(b, extra, &mut out)?;
    }
    Ok(out)
}

fn decode_batch(b: &ArrowBatch, extra: &[&str], out: &mut Decoded) -> Result<()> {
    let n = b.num_rows();
    if out.records.len() + n > MAX_BATCH_ROWS {
        bail!("the batch has more than {MAX_BATCH_ROWS} rows");
    }
    let ts = column::<Int64Array>(b, "ts")?;
    let level = column::<UInt8Array>(b, "level")?;
    let message = column::<StringArray>(b, "message")?;
    let template = column::<StringArray>(b, "template")?;
    let exception = column::<StringArray>(b, "exception")?;
    let fields = FieldsReader::of(b)?;
    let line = column::<UInt32Array>(b, "line")?;
    let id = column::<UInt32Array>(b, "id")?;
    let source = column::<UInt32Array>(b, "source")?;
    let markers = column::<UInt64Array>(b, "markers")?;
    let extras = extra
        .iter()
        .map(|name| column::<StringArray>(b, name))
        .collect::<Result<Vec<_>>>()?;
    out.records.reserve(n);
    for i in 0..n {
        let lv = *Level::ALL
            .get(level.value(i) as usize)
            .ok_or_else(|| anyhow!("row {i}: level {} is not 0..=5", level.value(i)))?;
        let f = fields.row(i)?;
        out.records.push(Record {
            id: id.value(i),
            ts: (!ts.is_null(i)).then(|| ts.value(i)),
            level: lv,
            message: if message.is_null(i) {
                String::new()
            } else {
                message.value(i).to_string()
            },
            template: opt_text(template, i),
            exception: opt_text(exception, i),
            fields: f,
            source: source.value(i),
            line: line.value(i),
            markers: markers.value(i),
        });
    }
    for (col, values) in extras.iter().zip(out.extra.iter_mut()) {
        values.extend((0..n).map(|i| if col.is_null(i) { String::new() } else { col.value(i).to_string() }));
    }
    Ok(())
}

/// Reads the fields of a batch in either layout.
enum FieldsReader<'a> {
    Json(&'a StringArray),
    Columns {
        /// (field name, is JSON text, column)
        columns: Vec<(&'a str, bool, &'a ArrayRef)>,
        order: Option<&'a StringArray>,
    },
}

impl<'a> FieldsReader<'a> {
    fn of(b: &'a ArrowBatch) -> Result<FieldsReader<'a>> {
        if b.schema().column_with_name("fields").is_some() {
            return Ok(FieldsReader::Json(column::<StringArray>(b, "fields")?));
        }
        let schema = b.schema_ref();
        let mut columns = Vec::new();
        for (f, c) in schema.fields().iter().zip(b.columns()) {
            let (json, name) = match (f.name().strip_prefix("f:"), f.name().strip_prefix("j:")) {
                (Some(n), _) => (false, n),
                (_, Some(n)) => (true, n),
                _ => continue,
            };
            let ok = match c.data_type() {
                DataType::Utf8 => true,
                DataType::Int64 | DataType::UInt64 | DataType::Float64 | DataType::Boolean => !json,
                _ => false,
            };
            if !ok {
                bail!("field column \"{}\" has an unsupported type {}", f.name(), c.data_type());
            }
            columns.push((name, json, c));
        }
        if columns.len() > MAX_FIELD_COLUMNS {
            bail!("the batch has more than {MAX_FIELD_COLUMNS} field columns");
        }
        let order = match b.column_by_name("field_order") {
            Some(_) => Some(column::<StringArray>(b, "field_order")?),
            None => None,
        };
        Ok(FieldsReader::Columns { columns, order })
    }

    fn value(column: &ArrayRef, json: bool, i: usize) -> Result<Option<Value>> {
        if column.is_null(i) {
            return Ok(None);
        }
        let any = column.as_any();
        let v = if let Some(a) = any.downcast_ref::<StringArray>() {
            if json {
                serde_json::from_str(a.value(i)).context("a field is not valid JSON")?
            } else {
                Value::String(a.value(i).to_string())
            }
        } else if let Some(a) = any.downcast_ref::<Int64Array>() {
            Value::from(a.value(i))
        } else if let Some(a) = any.downcast_ref::<UInt64Array>() {
            Value::from(a.value(i))
        } else if let Some(a) = any.downcast_ref::<Float64Array>() {
            Value::from(a.value(i))
        } else if let Some(a) = any.downcast_ref::<BooleanArray>() {
            Value::Bool(a.value(i))
        } else {
            bail!("unsupported field column")
        };
        Ok(Some(v))
    }

    fn row(&self, i: usize) -> Result<Fields> {
        match self {
            FieldsReader::Json(a) => {
                if a.is_null(i) {
                    Ok(Fields::new())
                } else {
                    serde_json::from_str(a.value(i)).with_context(|| format!("row {i}: \"fields\" is not a JSON object"))
                }
            }
            FieldsReader::Columns { columns, order } => {
                let mut f = Fields::new();
                let mut put = |k: usize| -> Result<()> {
                    let (name, json, c) = columns.get(k).ok_or_else(|| anyhow!("row {i}: field order names column {k}"))?;
                    if let Some(v) = Self::value(c, *json, i)? {
                        f.insert((*name).to_string(), v);
                    }
                    Ok(())
                };
                match order.filter(|o| !o.is_null(i)) {
                    Some(o) => {
                        for k in o.value(i).split(',') {
                            put(k.parse().map_err(|_| anyhow!("row {i}: field order \"{k}\" is not an index"))?)?;
                        }
                    }
                    None => {
                        for k in 0..columns.len() {
                            put(k)?;
                        }
                    }
                }
                Ok(f)
            }
        }
    }
}

/// Splits `len` rows into batch ranges of at most [`DEFAULT_BATCH_ROWS`] rows and about [`BATCH_BYTES`]
/// of text (`size(i)` estimates row `i`).
pub fn batch_ranges(len: usize, rows_per_batch: usize, size: impl Fn(usize) -> usize) -> Vec<std::ops::Range<usize>> {
    let rows_per_batch = rows_per_batch.max(1);
    let mut out = Vec::new();
    let (mut start, mut bytes) = (0, 0usize);
    for i in 0..len {
        bytes += size(i);
        if i + 1 - start >= rows_per_batch || bytes >= BATCH_BYTES {
            out.push(start..i + 1);
            start = i + 1;
            bytes = 0;
        }
    }
    if start < len {
        out.push(start..len);
    }
    out
}

/// Threads used to encode batches in parallel (at most 4: the importer and the other side need cores too).
pub fn parallelism() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, 4)
}

/// Threads used to decode batches in parallel. Decoding (allocating every string of every record) is the
/// slowest step of a transfer, so it gets up to half the cores (at most 8).
pub fn decode_parallelism() -> usize {
    (std::thread::available_parallelism().map_or(1, |n| n.get()) / 2).clamp(1, 8)
}

/// Encodes `records` in the batches `ranges`, `threads` at a time, handing each to `sink` in order (with
/// its row count). Stops when `sink` returns false.
pub fn encode_batches(
    records: &[Record],
    ranges: &[std::ops::Range<usize>],
    threads: usize,
    mut sink: impl FnMut(Result<Bytes>, u32) -> bool,
) {
    for wave in ranges.chunks(threads.max(1)) {
        let encoded: Vec<Result<Bytes>> = if wave.len() == 1 {
            vec![encode(&records[wave[0].clone()], &[])]
        } else {
            std::thread::scope(|s| {
                let handles: Vec<_> = wave.iter().map(|r| s.spawn(move || encode(&records[r.clone()], &[]))).collect();
                handles
                    .into_iter()
                    .map(|h| h.join().unwrap_or_else(|_| Err(anyhow!("encoding a batch panicked"))))
                    .collect()
            })
        };
        for (bytes, r) in encoded.into_iter().zip(wave) {
            if !sink(bytes, r.len() as u32) {
                return;
            }
        }
    }
}

/// Text size estimate of a record, for [`batch_ranges`].
pub fn record_size(r: &Record) -> usize {
    r.message.len() + r.exception.as_ref().map_or(0, String::len) + r.template.as_ref().map_or(0, String::len) + r.fields.len() * 24 + 32
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Record {
        let mut r = Record::new(7);
        r.ts = Some(-5);
        r.level = Level::Fatal;
        r.message = "héllo\nworld".into();
        r.template = Some("t {x}".into());
        r.exception = Some("boom".into());
        r.fields.insert("z".into(), json!(1));
        r.fields.insert("a".into(), json!({"n": [1, 2.5, null, true, "s"]}));
        r.id = 3;
        r.source = 4;
        r.markers = u64::MAX;
        r
    }

    #[test]
    fn records_round_trip_with_extra_columns() {
        let a = sample();
        let b = Record::new(0);
        let rows = [
            WithExtra {
                record: &a,
                extra: ["s1", "g1"],
            },
            WithExtra {
                record: &b,
                extra: ["s2", ""],
            },
        ];
        let bytes = encode(&rows, &["source_name", "group"]).unwrap();
        let d = decode(bytes, &["source_name", "group"]).unwrap();
        assert_eq!(d.records.len(), 2);
        let r = &d.records[0];
        assert_eq!(
            (r.ts, r.level, r.message.as_str(), r.line, r.id, r.source, r.markers),
            (Some(-5), Level::Fatal, "héllo\nworld", 7, 3, 4, u64::MAX)
        );
        assert_eq!(r.fields.keys().collect::<Vec<_>>(), ["z", "a"], "field order is kept");
        assert_eq!(r.fields, a.fields);
        assert!(d.records[1].fields.is_empty() && d.records[1].ts.is_none() && d.records[1].template.is_none());
        assert_eq!(d.extra, vec![vec!["s1".to_string(), "s2".into()], vec!["g1".into(), "".into()]]);
    }

    #[test]
    fn empty_batches_and_missing_columns() {
        let empty: [Record; 0] = [];
        assert!(decode(encode(&empty, &[]).unwrap(), &[]).unwrap().records.is_empty());
        let bytes = encode(&[Record::new(1)], &[]).unwrap();
        assert!(decode(bytes.clone(), &["instance"]).unwrap_err().to_string().contains("instance"));
        // Plain records encode without extra columns.
        let recs = [Record::new(1)];
        let refs: Vec<&Record> = recs.iter().collect();
        assert_eq!(decode(encode(&refs, &[]).unwrap(), &[]).unwrap().records.len(), 1);
        assert_eq!(
            WithExtra {
                record: &recs[0],
                extra: ["x"]
            }
            .extra(5),
            ""
        );
        assert_eq!(recs[0].extra(0), "");
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(decode(Bytes::from_static(b"not arrow at all"), &[]).is_err());
        let good = encode(&[sample()], &[]).unwrap();
        for cut in [1, 8, good.len() / 2, good.len() - 1] {
            assert!(decode(good.slice(..cut), &[]).is_err(), "cut at {cut}");
        }
        assert!(decode(Bytes::new(), &[]).unwrap().records.is_empty());
    }

    fn batch_of(cols: Vec<(&str, ArrayRef)>) -> Bytes {
        let fields: Vec<Field> = cols.iter().map(|(n, a)| Field::new(*n, a.data_type().clone(), true)).collect();
        let schema = Arc::new(Schema::new(fields));
        let b = ArrowBatch::try_new(schema.clone(), cols.into_iter().map(|(_, a)| a).collect()).unwrap();
        let mut out = Vec::new();
        let mut w = StreamWriter::try_new(&mut out, &schema).unwrap();
        w.write(&b).unwrap();
        w.finish().unwrap();
        drop(w);
        Bytes::from(out)
    }

    fn valid_columns(level: u8, fields: Option<&str>) -> Vec<(&'static str, ArrayRef)> {
        vec![
            ("ts", Arc::new(Int64Array::from(vec![Some(1)])) as ArrayRef),
            ("level", Arc::new(UInt8Array::from(vec![level]))),
            ("message", Arc::new(StringArray::from(vec![None::<&str>]))),
            ("template", Arc::new(StringArray::from(vec![None::<&str>]))),
            ("exception", Arc::new(StringArray::from(vec![None::<&str>]))),
            ("fields", Arc::new(StringArray::from(vec![fields]))),
            ("line", Arc::new(UInt32Array::from(vec![1]))),
            ("id", Arc::new(UInt32Array::from(vec![0]))),
            ("source", Arc::new(UInt32Array::from(vec![0]))),
            ("markers", Arc::new(UInt64Array::from(vec![0]))),
        ]
    }

    #[test]
    fn invalid_values_are_rejected() {
        assert!(decode(batch_of(valid_columns(2, None)), &[]).unwrap().records[0].message.is_empty());
        let e = decode(batch_of(valid_columns(9, None)), &[]).unwrap_err().to_string();
        assert!(e.contains("level 9"), "{e}");
        let e = decode(batch_of(valid_columns(2, Some("[1]"))), &[]).unwrap_err().to_string();
        assert!(e.contains("not a JSON object"), "{e}");
        let mut cols = valid_columns(2, None);
        cols[1] = ("level", Arc::new(UInt32Array::from(vec![2])));
        assert!(decode(batch_of(cols), &[]).unwrap_err().to_string().contains("wrong type"));
        let mut cols = valid_columns(2, None);
        cols.push(("instance", Arc::new(StringArray::from(vec![None::<&str>]))));
        assert_eq!(decode(batch_of(cols), &["instance"]).unwrap().extra, vec![vec![String::new()]]);
    }

    fn names(bytes: &Bytes) -> Vec<String> {
        let mut buffer = Buffer::from(bytes.clone());
        let b = StreamDecoder::new().decode(&mut buffer).unwrap().unwrap();
        b.schema().fields().iter().map(|f| f.name().clone()).collect()
    }

    #[test]
    fn fields_are_laid_out_as_typed_columns() {
        let mut a = Record::new(1);
        a.fields.insert("s".into(), json!("x"));
        a.fields.insert("i".into(), json!(-3));
        a.fields.insert("u".into(), json!(u64::MAX));
        a.fields.insert("f".into(), json!(2.0));
        a.fields.insert("b".into(), json!(true));
        a.fields.insert("mixed".into(), json!(1));
        let mut b = Record::new(2);
        // Another order, a missing field, a type change and a null.
        b.fields.insert("mixed".into(), json!("now text"));
        b.fields.insert("s".into(), json!("y"));
        b.fields.insert("n".into(), Value::Null);
        let c = Record::new(3);
        let bytes = encode(&[a.clone(), b.clone(), c], &[]).unwrap();
        let cols = names(&bytes);
        for name in ["f:s", "f:i", "f:u", "f:f", "f:b", "j:mixed", "j:n", "field_order"] {
            assert!(cols.contains(&name.to_string()), "{name} in {cols:?}");
        }
        assert!(!cols.contains(&"fields".to_string()));
        let d = decode(bytes, &[]).unwrap();
        assert_eq!(d.records[0].fields, a.fields);
        assert_eq!(d.records[1].fields, b.fields);
        assert_eq!(d.records[1].fields.keys().collect::<Vec<_>>(), ["mixed", "s", "n"]);
        assert!(d.records[0].fields["f"].is_f64() && d.records[0].fields["i"].is_i64());
        assert!(d.records[2].fields.is_empty());
        // Same order everywhere: no order column.
        let bytes = encode(&[a.clone(), a.clone()], &[]).unwrap();
        assert!(!names(&bytes).contains(&"field_order".to_string()));
    }

    #[test]
    fn many_or_sparse_names_use_the_json_layout() {
        let many: Vec<Record> = (0..300)
            .map(|i| {
                let mut r = Record::new(i);
                r.fields.insert(format!("k{i}"), json!(i));
                r
            })
            .collect();
        let bytes = encode(&many, &[]).unwrap();
        assert!(names(&bytes).contains(&"fields".to_string()));
        assert_eq!(decode(bytes, &[]).unwrap().records[299].fields["k299"], json!(299));
        let sparse: Vec<Record> = many[..40].to_vec();
        assert!(names(&encode(&sparse, &[]).unwrap()).contains(&"fields".to_string()));
    }

    #[test]
    fn bad_field_columns_are_rejected() {
        let mut cols = valid_columns(2, None);
        cols.remove(5);
        cols.push(("f:x", Arc::new(UInt8Array::from(vec![1u8]))));
        assert!(decode(batch_of(cols), &[]).unwrap_err().to_string().contains("unsupported type"));
        let mut cols = valid_columns(2, None);
        cols.remove(5);
        cols.push(("j:x", Arc::new(StringArray::from(vec![Some("{")]))));
        assert!(decode(batch_of(cols), &[]).is_err());
        for order in ["9", "x"] {
            let mut cols = valid_columns(2, None);
            cols.remove(5);
            cols.push(("f:x", Arc::new(StringArray::from(vec![Some("v")]))));
            cols.push(("field_order", Arc::new(StringArray::from(vec![Some(order)]))));
            assert!(decode(batch_of(cols), &[]).is_err(), "{order}");
        }
        let mut cols = valid_columns(2, None);
        cols.remove(5);
        cols.push(("f:x", Arc::new(BooleanArray::from(vec![Some(true)]))));
        cols.push(("f:y", Arc::new(Float64Array::from(vec![Some(1.5)]))));
        cols.push(("f:z", Arc::new(UInt64Array::from(vec![Some(7)]))));
        cols.push(("other", Arc::new(StringArray::from(vec![Some("ignored")]))));
        let d = decode(batch_of(cols), &[]).unwrap();
        assert_eq!(d.records[0].fields["x"], json!(true));
        assert_eq!(d.records[0].fields["y"], json!(1.5));
        assert_eq!(d.records[0].fields["z"], json!(7));
    }

    #[test]
    fn batches_encode_in_parallel_in_order() {
        let recs: Vec<Record> = (0..10).map(Record::new).collect();
        let ranges = batch_ranges(recs.len(), 3, |_| 1);
        let mut lines = Vec::new();
        encode_batches(&recs, &ranges, 2, |b, rows| {
            let d = decode(b.unwrap(), &[]).unwrap();
            assert_eq!(d.records.len(), rows as usize);
            lines.extend(d.records.iter().map(|r| r.line));
            true
        });
        assert_eq!(lines, (0..10).collect::<Vec<_>>());
        let mut n = 0;
        encode_batches(&recs, &ranges, 1, |_, _| {
            n += 1;
            false
        });
        assert_eq!(n, 1, "stops when the sink says so");
        assert!((1..=4).contains(&parallelism()));
    }

    #[test]
    fn batches_are_split_by_rows_and_bytes() {
        assert_eq!(batch_ranges(5, 2, |_| 1), vec![0..2, 2..4, 4..5]);
        assert_eq!(batch_ranges(0, 2, |_| 1), Vec::<std::ops::Range<usize>>::new());
        assert_eq!(batch_ranges(3, 100, |_| BATCH_BYTES), vec![0..1, 1..2, 2..3]);
        assert_eq!(batch_ranges(3, 0, |_| 1).len(), 3);
        assert!(record_size(&sample()) > 32);
        assert_eq!(schema(&["x"]).fields().len(), RECORD_COLUMNS.len() + 2);
    }
}
