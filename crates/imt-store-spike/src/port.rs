//! The operations the spike measures, as a small port every candidate store implements: the
//! in-memory [`Workspace`](imt_core::Workspace) (through the real `LogStore` trait), DuckDB and
//! DataFusion + Parquet. Shapes follow the `LogStore` requests they stand for (`page`, `facet`,
//! `timeline`, `field_stats`, text search), reduced to what the benchmark compares.

use anyhow::Result;
use imt_core::model::Level;

/// Fields promoted to typed columns in the persistent stores. A closed set: a filter, facet or
/// statistic can only name a column that exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Column {
    MachineName,
    ProcessId,
    SourceContext,
    StatusCode,
    Elapsed,
}

impl Column {
    pub const ALL: [Column; 5] = [
        Column::MachineName,
        Column::ProcessId,
        Column::SourceContext,
        Column::StatusCode,
        Column::Elapsed,
    ];

    /// The IMT field name (what a profile or query says).
    pub fn field(self) -> &'static str {
        match self {
            Column::MachineName => "MachineName",
            Column::ProcessId => "ProcessId",
            Column::SourceContext => "SourceContext",
            Column::StatusCode => "StatusCode",
            Column::Elapsed => "Elapsed",
        }
    }

    /// The column name in the persistent stores.
    pub fn column(self) -> &'static str {
        match self {
            Column::MachineName => "machine_name",
            Column::ProcessId => "process_id",
            Column::SourceContext => "source_context",
            Column::StatusCode => "status_code",
            Column::Elapsed => "elapsed",
        }
    }

    pub fn kind(self) -> ColumnKind {
        match self {
            Column::MachineName | Column::SourceContext => ColumnKind::Text,
            Column::ProcessId | Column::StatusCode => ColumnKind::Integer,
            Column::Elapsed => ColumnKind::Float,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnKind {
    Text,
    Integer,
    Float,
}

/// The query criteria the benchmark uses: levels, a case-insensitive substring of the message,
/// exception or template (the grid's search box), and one `field = value`.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub levels: Vec<Level>,
    pub text: Option<String>,
    pub field: Option<(Column, String)>,
}

impl Filter {
    pub fn is_empty(&self) -> bool {
        self.levels.is_empty() && self.text.is_none() && self.field.is_none()
    }

    /// `text` as a LIKE pattern with `\` escaping the wildcards.
    pub fn like_pattern(text: &str) -> String {
        let mut p = String::with_capacity(text.len() + 2);
        p.push('%');
        for c in text.chars() {
            if matches!(c, '%' | '_' | '\\') {
                p.push('\\');
            }
            p.push(c);
        }
        p.push('%');
        p
    }
}

/// A page: the matching total and the rows at `offset` in time order.
#[derive(Debug, Clone, PartialEq)]
pub struct PageOut {
    pub total: u64,
    pub ids: Vec<u64>,
    /// Bytes of message text returned (so every store really materialises the rows).
    pub message_bytes: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FacetOut {
    pub total: u64,
    pub distinct: u64,
    pub top: Vec<(String, u64)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TimelineOut {
    pub from: i64,
    pub to: i64,
    /// Events per bucket and level (`Level::index`).
    pub histogram: Vec<[u64; 6]>,
    /// Instance lanes (distinct `MachineName`, `ProcessId`).
    pub lanes: usize,
}

impl TimelineOut {
    pub fn total(&self) -> u64 {
        self.histogram.iter().flatten().sum()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StatsOut {
    pub count: u64,
    pub missing: u64,
    pub distinct: u64,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub histogram: Vec<u64>,
}

/// A full-text search: hit count, the first hits in time order and how many rows were fetched.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchOut {
    pub hits: u64,
    pub ids: Vec<u64>,
    pub fetched: usize,
}

/// Engine settings shared by both persistent adapters, so neither gets more of the machine.
pub const THREADS: usize = 4;
pub const MEMORY_LIMIT: &str = "2GB";

/// Bins of the field-stats histogram (as `FieldStatsRequest::buckets` defaults to).
pub const HISTOGRAM_BINS: usize = 20;

/// The measured operations.
pub trait SpikeStore {
    fn page(&self, filter: &Filter, offset: u64, limit: usize) -> Result<PageOut>;
    fn facet(&self, column: Column, filter: &Filter, limit: usize) -> Result<FacetOut>;
    fn timeline(&self, filter: &Filter, buckets: usize) -> Result<TimelineOut>;
    fn field_stats(&self, column: Column, filter: &Filter) -> Result<StatsOut>;
    fn search(&self, text: &str, limit: usize) -> Result<SearchOut>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn like_patterns_escape_wildcards() {
        assert_eq!(Filter::like_pattern("a_b%c\\"), "%a\\_b\\%c\\\\%");
        assert_eq!(Filter::like_pattern("timeout"), "%timeout%");
    }

    #[test]
    fn columns_have_distinct_names() {
        let mut names: Vec<_> = Column::ALL.iter().map(|c| c.column()).collect();
        names.dedup();
        assert_eq!(names.len(), Column::ALL.len());
        assert!(Filter::default().is_empty());
    }
}
