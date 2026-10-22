import {
  MaintenanceAction,
  SegmentRow,
  SegmentsPage,
  SegmentsRequest,
  SqlOutcome,
  SqlPage,
  SqlPageRequest,
  SqlResult,
  SqlSchema,
  StoreFields,
  StoreIndex,
  StoreMemory,
  StoreOverview,
  WorkspaceSummary,
} from '../../core/api.types';
import type { ImtBackend } from '../../core/backend';
import { CmdError } from '../../core/cmd-error';
import { EMPTY_QUERY } from '../../core/workspace/query.store';
import type { StorePaneBackend } from '../store-pane-backend';

const MB = 1024 * 1024;
/** Rows per mock segment (the engine's are 1M). */
const SEGMENT_ROWS = 2_000;
const PAGE = 500;
const MAX_ROWS = 10_000;
/** Words a console query may start with (the engine checks this, and much more, in Rust). */
const QUERY_WORDS = ['select', 'with', 'from', 'values', 'table', 'describe', 'summarize', 'pivot', 'unpivot', 'show', 'explain', '('];

const SCHEMA: SqlSchema = {
  version: 1,
  views: [
    {
      name: 'events',
      description: 'Every event of the store.',
      columns: [
        { name: 'id', type: 'UBIGINT', description: 'The event id.' },
        { name: 'ts', type: 'TIMESTAMP', description: 'Event time (UTC).' },
        { name: 'level', type: 'VARCHAR', description: 'The level.' },
        { name: 'source', type: 'UINTEGER', description: 'The source id.' },
        { name: 'line', type: 'UINTEGER', description: 'Line number.' },
        { name: 'byte_offset', type: 'UBIGINT', description: 'Byte offset.' },
        { name: 'message', type: 'VARCHAR', description: 'The message.' },
        { name: 'template', type: 'VARCHAR', description: 'The template.' },
        { name: 'exception', type: 'VARCHAR', description: 'The exception.' },
        { name: 'fields', type: 'MAP(VARCHAR, VARCHAR)', description: 'Every other field.' },
      ],
    },
    {
      name: 'sources',
      description: 'The sources.',
      columns: [
        { name: 'id', type: 'UINTEGER', description: 'The source id.' },
        { name: 'name', type: 'VARCHAR', description: 'File name.' },
        { name: 'path', type: 'VARCHAR', description: 'Full path.' },
        { name: 'events', type: 'UBIGINT', description: 'Events read.' },
      ],
    },
    {
      name: 'fields',
      description: 'The fields.',
      columns: [
        { name: 'key', type: 'VARCHAR', description: 'Field name.' },
        { name: 'type', type: 'VARCHAR', description: 'Its type.' },
        { name: 'storage', type: 'VARCHAR', description: 'How it is stored.' },
        { name: 'events', type: 'UBIGINT', description: 'Events with it.' },
      ],
    },
    {
      name: 'notes',
      description: 'The notes.',
      columns: [
        { name: 'id', type: 'VARCHAR', description: 'The note id.' },
        { name: 'text', type: 'VARCHAR', description: 'Its text.' },
      ],
    },
  ],
  functions: ['avg', 'count', 'date_trunc', 'lower', 'max', 'min', 'regexp_matches', 'strftime', 'sum', 'upper'],
  tableFunctions: ['range', 'generate_series', 'unnest', 'generate_subscripts'],
  maxRows: MAX_ROWS,
  pageRows: PAGE,
  timeLimitMs: 30_000,
};

const refusal = (message: string, offset = 0, len = 0): SqlOutcome => ({ error: { message, offset, len } });

/**
 * Browser stand-in for the Store pane, derived from the mock workspace: segments of 2,000 events, the summary's fields,
 * an index that is always caught up, and a console that answers `count(*)`, `SELECT ... FROM events/sources/fields`
 * (with `LIMIT`) and refuses what does not start like a query. Plausible, never the reference: the engine is.
 * `?mockStoreFail=1` makes every report fail; `?mockSqlSlow=1` makes runs wait until stopped.
 */
export class MockStorePaneBackend implements StorePaneBackend {
  private results = new Map<number, unknown[][]>();
  private nextResult = 1;
  private nextJob = 100;
  private readonly running = new Map<string, () => void>();
  private cachedAnalyses = 3;

  constructor(private readonly imt: ImtBackend) {}

  private flag(name: string): boolean {
    return typeof location !== 'undefined' && new URLSearchParams(location.search).get(name) === '1';
  }

  private async summary(): Promise<WorkspaceSummary> {
    if (this.flag('mockStoreFail')) throw new CmdError({ code: 'failed', message: 'the store worker stopped (mock)', retryable: true });
    return this.imt.workspaceSummary();
  }

  async overview(): Promise<StoreOverview> {
    const s = await this.summary();
    const segments = Math.ceil(s.total / SEGMENT_ROWS);
    return {
      dir: '/home/me/.local/share/imt/scratch/main',
      sizeBytes: s.total * 180 + 4 * MB,
      events: s.total,
      sources: s.sources.length,
      segments,
      dataVersion: 4,
      formatVersion: 3,
      activity: 'ready',
      lastImport: s.total ? { events: s.total, bytes: s.total * 220, elapsedMs: 1_250, eventsPerSecond: Math.round(s.total / 1.25), finishedMs: Date.now() - 60_000 } : null,
      pendingCompaction: segments > 3 ? 1 : 0,
    };
  }

  async listSegments(request: SegmentsRequest): Promise<SegmentsPage> {
    const s = await this.summary();
    const total = Math.ceil(s.total / SEGMENT_ROWS);
    const limit = Math.min(request.limit || PAGE, PAGE);
    const rows: SegmentRow[] = [];
    for (let i = request.offset; i < Math.min(total, request.offset + limit); i++) {
      const n = Math.min(SEGMENT_ROWS, s.total - i * SEGMENT_ROWS);
      rows.push({
        file: `seg-${String(i + 1).padStart(6, '0')}.parquet`,
        firstId: i * SEGMENT_ROWS,
        endId: (i + 1) * SEGMENT_ROWS,
        rows: n,
        live: n,
        sizeBytes: n * 150,
        from: s.from,
        to: s.to,
        compression: 'ZSTD',
        sources: s.sources.slice(0, 8).map((x) => x.id),
        sourceCount: s.sources.length,
        indexed: true,
        promoted: s.fields.slice(0, 3).map((f) => f.name),
        pendingCompaction: n < SEGMENT_ROWS / 2 && total > 3,
      });
    }
    return { total, offset: request.offset, rows };
  }

  async listFields(): Promise<StoreFields> {
    const s = await this.summary();
    return {
      events: s.total,
      truncated: false,
      fields: [...s.fields]
        .sort((a, b) => b.count - a.count)
        .map((f, i) => ({
          key: f.name,
          type: f.numeric ? 'number' : 'text',
          storage: i < 3 ? 'promoted' : 'overflow',
          promotedSegments: i < 3 ? 1 : 0,
          segments: 1,
          events: f.count,
          fillRate: s.total ? f.count / s.total : 0,
          approxDistinct: f.distinct,
        })),
    };
  }

  async describeIndex(): Promise<StoreIndex> {
    const s = await this.summary();
    return {
      docs: s.total,
      events: s.total,
      sizeBytes: s.total * 40,
      indexSegments: 2,
      pendingSegments: 0,
      catchingUp: false,
      lastCommitMs: Date.now() - 50_000,
      writerHeapBytes: 64 * MB,
      writerThreads: 2,
      error: null,
    };
  }

  async describeMemory(): Promise<StoreMemory> {
    await this.summary();
    return {
      worker: { pid: 4242, rssBytes: 142 * MB, lowMemory: false },
      importBudgetBytes: 768 * MB,
      queryBudgetBytes: 300 * MB,
      duckLimit: '192MB',
      duckLimitBytes: 192_000_000,
      duckUsedBytes: 38 * MB,
      duckSpilledBytes: 0,
      duckThreads: 4,
      consoleLimitBytes: 128_000_000,
      importing: false,
      runningJobs: [],
      cachedAnalyses: this.cachedAnalyses,
      cachedBytes: this.cachedAnalyses * 24_000,
    };
  }

  async maintain(action: MaintenanceAction): Promise<number> {
    if (action === 'clearCachedAnalyses') this.cachedAnalyses = 0;
    return this.nextJob++;
  }

  async describeSqlSchema(): Promise<SqlSchema> {
    return SCHEMA;
  }

  async querySql(sql: string, explain: boolean, runId: string): Promise<SqlOutcome> {
    const text = sql.trim().replace(/;\s*$/, '');
    const first = /^\s*(\(|[A-Za-z_]+)/.exec(text);
    if (!first) return refusal('Write a SELECT query.');
    const keyword = first[1] ?? '';
    const word = keyword.toLowerCase();
    if (!QUERY_WORDS.includes(word)) {
      return refusal(`The SQL console is read-only: ${keyword.toUpperCase()} is not allowed. Write a SELECT query.`, text.indexOf(keyword), keyword.length);
    }
    if (/;\s*\S/.test(text)) return refusal('Only one statement runs at a time: remove the second one.', text.indexOf(';') + 1, 0);
    if (/\bread_\w+\s*\(|\bglob\s*\(/i.test(text)) return refusal('The table function is not allowed in the SQL console.', 0, 0);
    if (this.flag('mockSqlSlow')) {
      await new Promise<void>((resolve) => this.running.set(runId, resolve));
      this.running.delete(runId);
      return { cancelled: true };
    }
    const started = Date.now();
    if (explain || word === 'explain') return { result: this.result([], [], false, '┌─────────────┐\n│ READ_PARQUET │\n└─────────────┘', started) };
    const s = await this.summary();
    if (/count\s*\(\s*\*\s*\)/i.test(text)) return { result: this.result([{ name: 'count_star()', type: 'BIGINT' }], [[s.total]], false, null, started) };
    if (/\bfrom\s+(imt\.)?sources\b/i.test(text)) {
      return { result: this.result([{ name: 'id', type: 'UINTEGER' }, { name: 'name', type: 'VARCHAR' }, { name: 'events', type: 'UBIGINT' }], s.sources.map((x) => [x.id, x.name, x.records]), false, null, started) };
    }
    if (/\bfrom\s+(imt\.)?fields\b/i.test(text)) {
      return { result: this.result([{ name: 'key', type: 'VARCHAR' }, { name: 'events', type: 'UBIGINT' }], s.fields.map((f) => [f.name, f.count]), false, null, started) };
    }
    if (/\bfrom\s+(imt\.)?events\b/i.test(text)) {
      const limit = Number(/\blimit\s+(\d+)/i.exec(text)?.[1] ?? MAX_ROWS + 1);
      const page = await this.imt.queryPage({ query: EMPTY_QUERY, offset: 0, limit: Math.min(limit, MAX_ROWS + 1, 5_000), descending: false });
      const rows = page.rows.map((r) => [r.id, r.ts === null ? null : new Date(r.ts).toISOString().replace('T', ' ').replace('Z', ''), r.level, r.message]);
      const truncated = rows.length > MAX_ROWS || (limit > MAX_ROWS && s.total > MAX_ROWS);
      return {
        result: this.result(
          [
            { name: 'id', type: 'UBIGINT' },
            { name: 'ts', type: 'TIMESTAMP' },
            { name: 'level', type: 'VARCHAR' },
            { name: 'message', type: 'VARCHAR' },
          ],
          rows.slice(0, MAX_ROWS),
          truncated,
          null,
          started,
        ),
      };
    }
    if (/\bfrom\s+(imt\.)?\w+/i.test(text) && !/\bfrom\s+(imt\.)?(notes|range|unnest|generate_series)\b/i.test(text)) {
      const name = /\bfrom\s+(?:imt\.)?(\w+)/i.exec(text)?.[1] ?? '';
      return refusal(`Catalog Error: Table with name ${name} does not exist!`, 0, 0);
    }
    return { result: this.result([{ name: '1', type: 'INTEGER' }], [[1]], false, null, started) };
  }

  private result(columns: SqlResult['columns'], rows: unknown[][], truncated: boolean, plan: string | null, started: number): SqlResult {
    const result = plan === null ? this.nextResult++ : 0;
    if (plan === null) this.results.set(result, rows);
    const eventIds = columns.some((c) => c.name.toLowerCase() === 'id' && /INT/.test(c.type));
    return { result, columns, rows: rows.slice(0, PAGE) as SqlResult['rows'], total: rows.length, truncated, eventIds, plan, elapsedMs: Date.now() - started };
  }

  async cancelSql(runId: string): Promise<boolean> {
    const stop = this.running.get(runId);
    stop?.();
    return !!stop;
  }

  async fetchSqlPage(request: SqlPageRequest): Promise<SqlPage> {
    const rows = this.results.get(request.result);
    if (!rows) throw new CmdError({ code: 'notFound', message: 'this result is gone (newer queries replaced it); run the query again', retryable: false });
    return { offset: request.offset, rows: rows.slice(request.offset, request.offset + PAGE) as SqlPage['rows'] };
  }
}
