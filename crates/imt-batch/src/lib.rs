//! IMT's event batch (ADR 0014 §3): the unit that flows from importers through the ingest pipeline into
//! the store, the full-text index, exports and (from R3) the plugin protocol.
//!
//! - [`EventId`] and [`SourceId`] are stable identities, assigned once by the store's writer and never
//!   renumbered (they are not time-order positions).
//! - [`EventSchema`] is the fixed Arrow schema of an event plus the typed columns a profile declares;
//!   other fields go to the `extra` map column.
//! - [`EventBatchBuilder`] builds a [`PendingBatch`] (no ids yet) of at most [`BATCH_ROWS`] events;
//!   the writer turns it into an [`EventBatch`] with [`PendingBatch::assign_ids`].
//! - [`EventBatch::try_new`] validates a batch from elsewhere (a query result, a plugin frame), so code
//!   reading a batch can rely on its invariants.
//!
//! This crate depends only on Arrow, not on `imt-core`, so the plugin API can reuse it.

mod batch;
mod builder;
mod error;
mod ids;
mod schema;

pub use batch::{BatchRow, EventBatch};
pub use builder::{EventBatchBuilder, NewEvent, PendingBatch};
pub use error::BatchError;
pub use ids::{EventId, EventTime, LevelCode, SourceId, TsState};
pub use schema::{col, ColumnType, DeclaredColumn, EventSchema, FieldValue, DECLARED_PREFIX};

/// Most events in one batch: the unit of the ingest pipeline, the writer and the index.
pub const BATCH_ROWS: usize = 8192;

/// Arrow, re-exported so users of the batch build and read it with the same version.
pub use arrow_array;
pub use arrow_schema;
