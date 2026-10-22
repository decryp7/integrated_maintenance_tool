// Generated from the Rust wire types by `cargo test -p imt-app --lib generated_types` (ts-rs; ADR 0016).
// Do not edit: change the Rust type and run the test with IMT_WRITE_TYPES=1.

export type StoreActivity = "ready" | "importing" | "indexing";

export type LastImport = { events: number, bytes: number, elapsedMs: number, 
/**
 * Events per second over the whole import (0 when it took no measurable time).
 */
eventsPerSecond: number, 
/**
 * When it ended (Unix ms).
 */
finishedMs: number, };

export type StoreOverview = { 
/**
 * The store folder.
 */
dir: string, 
/**
 * Everything in the folder (database, segments, index, spill files).
 */
sizeBytes: number, events: number, sources: number, segments: number, 
/**
 * Bumped by every change to the events (cached analyses belong to one version).
 */
dataVersion: number, 
/**
 * The database layout version.
 */
formatVersion: number, activity: StoreActivity, lastImport: LastImport | null, 
/**
 * Segments the next compaction rewrites (rows of removed sources, or too many small segments).
 */
pendingCompaction: number, };

export type SegmentsRequest = { offset: number, 
/**
 * At most [`MAX_SEGMENT_ROWS`] (0: that many).
 */
limit: number, };

export type SegmentRow = { file: string, firstId: number, 
/**
 * One past the last id it may hold.
 */
endId: number, rows: number, 
/**
 * Rows of sources that are still in the store (the rest go at the next compaction).
 */
live: number, sizeBytes: number, 
/**
 * Earliest and latest event time (Unix ms), to the minute.
 */
from: number | null, to: number | null, 
/**
 * The Parquet compression codec (e.g. `ZSTD`).
 */
compression: string, 
/**
 * The first [`SEGMENT_SOURCES`] source ids of its rows.
 */
sources: Array<number>, sourceCount: number, 
/**
 * In the full-text index.
 */
indexed: boolean, 
/**
 * Fields stored in their own column in this segment.
 */
promoted: Array<string>, 
/**
 * The next compaction rewrites it.
 */
pendingCompaction: boolean, };

export type SegmentsPage = { total: number, offset: number, rows: Array<SegmentRow>, };

export type FieldStorage = "declared" | "promoted" | "partly" | "overflow";

export type StoreFieldType = "text" | "integer" | "number" | "boolean";

export type StoreField = { key: string, type: StoreFieldType, storage: FieldStorage, 
/**
 * Segments in which it has its own column, and segments that hold it at all.
 */
promotedSegments: number, segments: number, 
/**
 * Events that have it.
 */
events: number, 
/**
 * `events` over all events, 0..=1.
 */
fillRate: number, 
/**
 * Approximate distinct values: the largest per-segment estimate (a lower bound across segments).
 */
approxDistinct: number, };

export type StoreFields = { events: number, fields: Array<StoreField>, 
/**
 * More fields than [`MAX_FIELD_ROWS`]: the rarest were left out.
 */
truncated: boolean, };

export type StoreIndex = { 
/**
 * Documents in the index, and events in the store.
 */
docs: number, events: number, sizeBytes: number, 
/**
 * Tantivy's segments.
 */
indexSegments: number, 
/**
 * Store segments not indexed yet (search does not see their events).
 */
pendingSegments: number, catchingUp: boolean, 
/**
 * The last commit (Unix ms).
 */
lastCommitMs: number | null, 
/**
 * The index writer's heap and threads.
 */
writerHeapBytes: number, writerThreads: number, 
/**
 * Why the index is unavailable, or the last indexing failure.
 */
error: string | null, };

export type RunningJob = { kind: string, title: string, };

export type WorkerMemory = { pid: number, rssBytes: number, 
/**
 * Started in Low memory mode.
 */
lowMemory: boolean, };

export type StoreMemory = { 
/**
 * The worker process (absent when the store runs in the app's process, as in tests).
 */
worker: WorkerMemory | null, importBudgetBytes: number, queryBudgetBytes: number, 
/**
 * DuckDB's memory limit (as configured, e.g. `192MB`) and in bytes.
 */
duckLimit: string, duckLimitBytes: number, 
/**
 * What DuckDB's buffer manager holds now, and what it spilled to the store's `tmp` folder.
 */
duckUsedBytes: number, duckSpilledBytes: number, duckThreads: number, 
/**
 * The SQL console's own memory limit (inside the query budget).
 */
consoleLimitBytes: number, importing: boolean, 
/**
 * Jobs of the window now (filled by the app).
 */
runningJobs: Array<RunningJob>, 
/**
 * Analyses kept for the current data version, and their size.
 */
cachedAnalyses: number, cachedBytes: number, };

export type MaintenanceAction = "compact" | "rebuildIndex" | "clearCachedAnalyses";

export type MaintenanceReport = { action: MaintenanceAction, message: string, };

export type SqlColumn = { name: string, type: string, };

export type SqlResult = { 
/**
 * The result's handle for [`SqlPageRequest`] (the store keeps the last few).
 */
result: number, columns: Array<SqlColumn>, 
/**
 * The first [`SQL_PAGE_ROWS`] rows; cells are JSON scalars (null, boolean, number, string).
 */
rows: Array<Array<string | number | boolean | null>>, 
/**
 * Rows kept (at most [`SQL_MAX_ROWS`]).
 */
total: number, 
/**
 * The query returned more rows than were kept.
 */
truncated: boolean, 
/**
 * The rows carry event ids: a column `id` of an integer type (from `imt.events`).
 */
eventIds: boolean, 
/**
 * The query plan, when [`SqlRequest::explain`] asked for it (then there are no rows).
 */
plan: string | null, elapsedMs: number, };

export type SqlPageRequest = { result: number, offset: number, };

export type SqlPage = { offset: number, rows: Array<Array<string | number | boolean | null>>, };

export type SqlIdsRequest = { result: number, };

export type SqlIds = { ids: number[], 
/**
 * The query returned more rows than the result kept: these are the ids of the kept rows only.
 */
truncated: boolean, };

export type SqlViewColumn = { name: string, type: string, description: string, };

export type SqlView = { 
/**
 * Its name in the `imt` schema (also reachable unqualified).
 */
name: string, description: string, columns: Array<SqlViewColumn>, };

export type SqlSchema = { version: number, views: Array<SqlView>, 
/**
 * Scalar and aggregate functions (names), and the table functions the console allows.
 */
functions: Array<string>, tableFunctions: Array<string>, 
/**
 * The limits, for the editor's hints.
 */
maxRows: number, pageRows: number, timeLimitMs: number, };
