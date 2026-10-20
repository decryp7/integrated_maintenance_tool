//! Why a batch could not be built or accepted.

/// A batch or schema that breaks one of the documented invariants.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BatchError {
    #[error("a batch holds at most {max} events; this one has {rows}")]
    TooManyRows { rows: usize, max: usize },
    #[error("column {index} should be \"{expected}\", found \"{found}\"")]
    ColumnName { index: usize, expected: String, found: String },
    #[error("column \"{column}\" has the wrong type: expected {expected}, found {found}")]
    ColumnType { column: String, expected: String, found: String },
    #[error("expected {expected} columns, found {found}")]
    ColumnCount { expected: usize, found: usize },
    #[error("column \"{column}\" must not contain nulls")]
    Null { column: String },
    #[error("row {row}: {problem}")]
    Row { row: usize, problem: String },
    #[error("declared field \"{0}\" is invalid: {1}")]
    Declared(String, String),
    #[error("arrow: {0}")]
    Arrow(String),
}

impl From<arrow_schema::ArrowError> for BatchError {
    fn from(e: arrow_schema::ArrowError) -> Self {
        BatchError::Arrow(e.to_string())
    }
}
