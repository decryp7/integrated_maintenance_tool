//! The Arrow schema of an event (ADR 0014 §3).
//!
//! Fixed columns, in this order: `id`, `ts`, `ts_state`, `level`, `source`, `line`, `byte_offset`,
//! `message`, `template`, `exception`, `markers`, `raw`; then one column per field the profile declares
//! with a type (named [`DECLARED_PREFIX`] + field name), then `extra`, a `MAP(VARCHAR, VARCHAR)` of every
//! other field.

use crate::error::BatchError;
use arrow_schema::{DataType, Field, Fields, Schema, SchemaRef};
use std::collections::HashMap;
use std::sync::Arc;

/// Names of the fixed columns.
pub mod col {
    pub const ID: &str = "id";
    pub const TS: &str = "ts";
    pub const TS_STATE: &str = "ts_state";
    pub const LEVEL: &str = "level";
    pub const SOURCE: &str = "source";
    pub const LINE: &str = "line";
    pub const BYTE_OFFSET: &str = "byte_offset";
    pub const MESSAGE: &str = "message";
    pub const TEMPLATE: &str = "template";
    pub const EXCEPTION: &str = "exception";
    pub const MARKERS: &str = "markers";
    pub const RAW: &str = "raw";
    pub const EXTRA: &str = "extra";
}

/// Prefix of declared field columns, so a field never collides with a fixed column.
pub const DECLARED_PREFIX: &str = "f_";

/// Most declared columns a schema has; further declared fields are stored in `extra`.
pub const MAX_DECLARED: usize = 64;

/// Longest declared field name, in bytes.
pub const MAX_FIELD_NAME: usize = 256;

/// The type of a declared field column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColumnType {
    Utf8,
    Int64,
    Float64,
    Boolean,
}

impl ColumnType {
    pub fn arrow(self) -> DataType {
        match self {
            ColumnType::Utf8 => DataType::Utf8,
            ColumnType::Int64 => DataType::Int64,
            ColumnType::Float64 => DataType::Float64,
            ColumnType::Boolean => DataType::Boolean,
        }
    }

    /// Whether `value` is stored in a column of this type (otherwise it goes to `extra` as text).
    pub fn fits(self, value: &FieldValue<'_>) -> bool {
        matches!(
            (self, value),
            (ColumnType::Utf8, FieldValue::Text(_))
                | (ColumnType::Int64, FieldValue::Int(_))
                | (ColumnType::Float64, FieldValue::Float(_))
                | (ColumnType::Boolean, FieldValue::Bool(_))
        )
    }
}

/// A field value as an event stores it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FieldValue<'a> {
    Text(&'a str),
    Int(i64),
    Float(f64),
    Bool(bool),
}

impl FieldValue<'_> {
    /// The text stored in `extra` for a value that is not text. Callers with their own canonical
    /// rendering (e.g. JSON) pass [`FieldValue::Text`] instead.
    pub fn render(&self) -> String {
        match self {
            FieldValue::Text(s) => (*s).to_string(),
            FieldValue::Int(i) => i.to_string(),
            FieldValue::Float(f) => format!("{f:?}"),
            FieldValue::Bool(b) => b.to_string(),
        }
    }
}

/// A field the profile declares with a type, stored in its own column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredColumn {
    /// The field name, as queries and rows use it.
    pub name: String,
    /// The Arrow column name ([`DECLARED_PREFIX`] + `name`).
    pub column: String,
    pub ty: ColumnType,
}

/// The schema of the events of one store generation: fixed columns, declared columns, `extra`.
#[derive(Debug, Clone)]
pub struct EventSchema {
    arrow: SchemaRef,
    pending: SchemaRef,
    declared: Vec<DeclaredColumn>,
    by_name: HashMap<String, usize>,
}

/// Number of fixed columns before the declared ones.
pub(crate) const FIXED: usize = 12;

impl EventSchema {
    /// A schema with no declared columns: every field goes to `extra`.
    pub fn plain() -> EventSchema {
        Self::new(std::iter::empty::<(String, ColumnType)>()).unwrap_or_else(|_| unreachable!("no declared fields"))
    }

    /// A schema with these declared fields. The first declaration of a name wins; a name equal to an
    /// earlier one ignoring case (SQL identifiers are case-insensitive) and declarations past
    /// [`MAX_DECLARED`] are left to `extra`. Empty, over-long or control-character names are refused.
    pub fn new<S: Into<String>>(declared: impl IntoIterator<Item = (S, ColumnType)>) -> Result<EventSchema, BatchError> {
        let mut columns: Vec<DeclaredColumn> = Vec::new();
        let mut folded: Vec<String> = Vec::new();
        for (name, ty) in declared {
            let name: String = name.into();
            if name.is_empty() || name.len() > MAX_FIELD_NAME || name.chars().any(char::is_control) {
                return Err(BatchError::Declared(
                    name.chars().take(64).collect(),
                    format!("names must be 1 to {MAX_FIELD_NAME} bytes without control characters"),
                ));
            }
            let lower = name.to_lowercase();
            if folded.contains(&lower) || columns.len() == MAX_DECLARED {
                continue;
            }
            folded.push(lower);
            columns.push(DeclaredColumn {
                column: format!("{DECLARED_PREFIX}{name}"),
                name,
                ty,
            });
        }
        let mut fields = vec![Field::new(col::ID, DataType::UInt64, false)];
        fields.extend(fixed_after_id());
        fields.extend(columns.iter().map(|c| Field::new(&c.column, c.ty.arrow(), true)));
        fields.push(extra_field());
        let arrow = Arc::new(Schema::new(fields));
        let pending = Arc::new(Schema::new(arrow.fields()[1..].to_vec()));
        let by_name = columns.iter().enumerate().map(|(i, c)| (c.name.clone(), i)).collect();
        Ok(EventSchema {
            arrow,
            pending,
            declared: columns,
            by_name,
        })
    }

    /// The event schema an Arrow schema of event batches describes (the declared columns between the fixed
    /// ones and `extra`), e.g. of batches read back from another process. A schema that is not an event schema
    /// is refused; [`crate::EventBatch::try_new`] still checks every batch against the result.
    pub fn from_arrow(schema: &Schema) -> Result<EventSchema, BatchError> {
        let fields = schema.fields();
        let declared_count = fields.len().checked_sub(FIXED + 1).ok_or_else(|| BatchError::ColumnCount {
            expected: FIXED + 1,
            found: fields.len(),
        })?;
        let mut declared = Vec::with_capacity(declared_count);
        for f in &fields[FIXED..FIXED + declared_count] {
            let name = f.name().strip_prefix(DECLARED_PREFIX).ok_or_else(|| {
                BatchError::Declared(
                    f.name().chars().take(64).collect(),
                    format!("declared columns start with {DECLARED_PREFIX}"),
                )
            })?;
            let ty = match f.data_type() {
                DataType::Utf8 => ColumnType::Utf8,
                DataType::Int64 => ColumnType::Int64,
                DataType::Float64 => ColumnType::Float64,
                DataType::Boolean => ColumnType::Boolean,
                other => {
                    return Err(BatchError::Declared(
                        name.chars().take(64).collect(),
                        format!("unsupported column type {other}"),
                    ))
                }
            };
            declared.push((name.to_string(), ty));
        }
        let built = EventSchema::new(declared)?;
        if built.declared().len() != declared_count {
            return Err(BatchError::Declared(
                "(declared columns)".into(),
                "names must be unique ignoring case and at most MAX_DECLARED".into(),
            ));
        }
        Ok(built)
    }

    /// The full Arrow schema, `id` first.
    pub fn arrow(&self) -> &SchemaRef {
        &self.arrow
    }

    /// The Arrow schema of a [`crate::PendingBatch`]: everything but `id`.
    pub fn pending_arrow(&self) -> &SchemaRef {
        &self.pending
    }

    pub fn declared(&self) -> &[DeclaredColumn] {
        &self.declared
    }

    /// Index (into [`EventSchema::declared`]) of a declared field.
    pub fn declared_index(&self, name: &str) -> Option<usize> {
        self.by_name.get(name).copied()
    }

    /// Index of the `extra` column in the full schema.
    pub fn extra_index(&self) -> usize {
        FIXED + self.declared.len()
    }

    /// Whether two schemas declare the same columns.
    pub fn same_columns(&self, other: &EventSchema) -> bool {
        self.declared == other.declared
    }
}

fn fixed_after_id() -> Vec<Field> {
    vec![
        Field::new(col::TS, DataType::Int64, true),
        Field::new(col::TS_STATE, DataType::UInt8, false),
        Field::new(col::LEVEL, DataType::UInt8, false),
        Field::new(col::SOURCE, DataType::UInt32, false),
        Field::new(col::LINE, DataType::UInt32, false),
        Field::new(col::BYTE_OFFSET, DataType::UInt64, false),
        Field::new(col::MESSAGE, DataType::Utf8, false),
        Field::new(col::TEMPLATE, DataType::Utf8, true),
        Field::new(col::EXCEPTION, DataType::Utf8, true),
        Field::new(col::MARKERS, DataType::UInt64, false),
        Field::new(col::RAW, DataType::Utf8, false),
    ]
}

/// `extra`: `MAP(VARCHAR, VARCHAR)` as Arrow's map builder lays it out.
pub(crate) fn extra_field() -> Field {
    let entries = Field::new(
        "entries",
        DataType::Struct(Fields::from(vec![
            Field::new("keys", DataType::Utf8, false),
            Field::new("values", DataType::Utf8, true),
        ])),
        false,
    );
    Field::new(col::EXTRA, DataType::Map(Arc::new(entries), false), false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_columns_then_declared_then_extra() {
        let s = EventSchema::new([("Elapsed", ColumnType::Float64), ("Status", ColumnType::Int64)]).unwrap();
        let names: Vec<&str> = s.arrow().fields().iter().map(|f| f.name().as_str()).collect();
        assert_eq!(
            names,
            [
                "id",
                "ts",
                "ts_state",
                "level",
                "source",
                "line",
                "byte_offset",
                "message",
                "template",
                "exception",
                "markers",
                "raw",
                "f_Elapsed",
                "f_Status",
                "extra"
            ]
        );
        assert_eq!(s.pending_arrow().fields().len(), names.len() - 1);
        assert_eq!(s.extra_index(), 14);
        assert_eq!(s.declared_index("Status"), Some(1));
        assert_eq!(s.declared_index("status"), None);
    }

    #[test]
    fn a_schema_is_read_back_from_its_arrow_form() {
        let s = EventSchema::new([("Elapsed", ColumnType::Float64), ("Ok", ColumnType::Boolean)]).unwrap();
        let back = EventSchema::from_arrow(s.arrow()).unwrap();
        assert!(back.same_columns(&s));
        assert_eq!(back.arrow().fields(), s.arrow().fields());
        assert!(EventSchema::from_arrow(EventSchema::plain().arrow()).unwrap().declared().is_empty());
        assert!(EventSchema::from_arrow(&Schema::new(vec![Field::new("id", DataType::UInt64, false)])).is_err());
        let mut fields: Vec<Field> = EventSchema::plain().arrow().fields().iter().map(|f| f.as_ref().clone()).collect();
        fields.insert(FIXED, Field::new("nope", DataType::Utf8, true));
        assert!(EventSchema::from_arrow(&Schema::new(fields.clone())).is_err());
        fields[FIXED] = Field::new("f_x", DataType::Date32, true);
        assert!(EventSchema::from_arrow(&Schema::new(fields.clone())).is_err());
        fields[FIXED] = Field::new("f_x", DataType::Utf8, true);
        fields.insert(FIXED, Field::new("f_X", DataType::Utf8, true));
        assert!(EventSchema::from_arrow(&Schema::new(fields)).is_err(), "case-folded duplicates");
    }

    #[test]
    fn duplicate_and_case_folded_names_go_to_extra() {
        let s = EventSchema::new([("A", ColumnType::Int64), ("A", ColumnType::Utf8), ("a", ColumnType::Utf8)]).unwrap();
        assert_eq!(s.declared().len(), 1);
        assert_eq!(s.declared()[0].ty, ColumnType::Int64);
        let many = (0..MAX_DECLARED + 5).map(|i| (format!("F{i}"), ColumnType::Utf8));
        assert_eq!(EventSchema::new(many).unwrap().declared().len(), MAX_DECLARED);
        assert!(EventSchema::new([("", ColumnType::Utf8)]).is_err());
        assert!(EventSchema::new([("a\nb", ColumnType::Utf8)]).is_err());
        assert!(EventSchema::new([("x".repeat(MAX_FIELD_NAME + 1), ColumnType::Utf8)]).is_err());
        assert!(EventSchema::plain().same_columns(&EventSchema::new(Vec::<(String, ColumnType)>::new()).unwrap()));
        assert!(!s.same_columns(&EventSchema::plain()));
    }

    #[test]
    fn values_fit_only_their_column_type() {
        assert!(ColumnType::Utf8.fits(&FieldValue::Text("x")));
        assert!(!ColumnType::Utf8.fits(&FieldValue::Int(1)));
        assert!(ColumnType::Int64.fits(&FieldValue::Int(1)));
        assert!(!ColumnType::Int64.fits(&FieldValue::Float(1.0)));
        assert!(ColumnType::Float64.fits(&FieldValue::Float(1.0)));
        assert!(!ColumnType::Float64.fits(&FieldValue::Int(1)));
        assert!(ColumnType::Boolean.fits(&FieldValue::Bool(true)));
        assert_eq!(FieldValue::Float(2.0).render(), "2.0");
        assert_eq!(FieldValue::Int(-3).render(), "-3");
        assert_eq!(FieldValue::Bool(false).render(), "false");
        assert_eq!(FieldValue::Text("t").render(), "t");
    }
}
