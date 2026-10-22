//! The two seams of the streaming import: importers push [`ParsedEvent`]s into an [`EventSink`]; the
//! batcher pushes [`PendingBatch`]es into a [`BatchSink`] (the store's writer).

use super::problems::{Problem, ProblemKind, Problems};
use crate::api::FileKind;
use crate::batch::{PendingBatch, SourceId};
use crate::model::Record;
use crate::pipeline::StepStats;
use anyhow::Result;

/// An event as an importer parsed it, with where it came from.
#[derive(Debug, Clone)]
pub struct ParsedEvent {
    pub record: Record,
    /// The event's original text (all its lines).
    pub raw: String,
    /// Offset of its first line in the file.
    pub byte_offset: u64,
}

/// Where an importer puts what it reads.
pub trait EventSink {
    fn event(&mut self, ev: ParsedEvent) -> Result<()>;
    /// Something on `line` could not be read as it should.
    fn problem(&mut self, kind: ProblemKind, line: u32);
    /// Problems an importer that does not stream already aggregated (`Imported::problems`). Sinks that keep
    /// problems override it; the default drops them.
    fn problems(&mut self, problems: Problems) {
        let _ = problems;
    }
}

/// A source about to be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSource {
    /// File path, or `<archive>!/<entry>` for an archive member.
    pub path: String,
    /// Display name (relative to the opened folder).
    pub name: String,
}

/// A source read completely.
#[derive(Debug, Clone)]
pub struct SourceReport {
    pub id: SourceId,
    pub path: String,
    pub name: String,
    pub importer: String,
    /// The parser that read it (ADR 0026).
    pub parser: crate::ports::ParserRef,
    pub file_kind: FileKind,
    /// Events written.
    pub events: usize,
    /// Lines the importer skipped (not events).
    pub skipped: usize,
    /// Events the pipeline removed.
    pub dropped: usize,
    pub step_stats: Vec<StepStats>,
    pub from: Option<i64>,
    pub to: Option<i64>,
    /// Size of the file in bytes (of the archive member, decompressed).
    pub bytes: u64,
    pub problems: Vec<Problem>,
}

/// Where event batches go: the store's single writer. It assigns event ids (in write order) and source
/// ids; a source is begun, written in batches, then finished, or aborted (its events removed).
pub trait BatchSink {
    fn begin_source(&mut self, source: &NewSource) -> Result<SourceId>;
    fn write(&mut self, batch: PendingBatch) -> Result<()>;
    fn finish_source(&mut self, report: &SourceReport) -> Result<()>;
    fn abort_source(&mut self, id: SourceId) -> Result<()>;
}

/// The import was cancelled; the file being read is not imported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the import was cancelled")]
pub struct ImportCancelled;
