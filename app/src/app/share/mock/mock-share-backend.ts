import { parse } from 'smol-toml';
import {
  Analyzed,
  BundleManifest,
  BundleOpened,
  Finding,
  Query,
  RedactionPreview,
  RedactionPreviewRequest,
  RedactionSummary,
  RedactionSummaryRequest,
  Report,
  ReportExportRequest,
  TileData,
  TileRequest,
  VisualRequest,
  VisualSaved,
  WorkspaceSummary,
} from '../../core/api.types';
import { AnalysisBackend } from '../../core/analysis-backend';
import { ImtBackend } from '../../core/backend';
import { MockBackend } from '../../core/mock/mock-backend';
import { EMPTY_QUERY } from '../../core/workspace.store';
import { ShareBackend } from '../share-backend';
import { mockTileError } from './mock-dashboards';
import { MockAnalysisBackend, recordsFrom } from '../../analysis/mock/mock-analysis-backend';
import { MockFiles } from '../../core/mock/mock-import';
import { mockRedactCfg, mockRedactionPreview, MockRedactor } from './mock-redact';
import { mockJobs, mockSteps } from '../../jobs/mock-jobs';
import { CmdError } from '../../core/cmd-error';

/** How long a mock export takes (`?mockExportMs=`, default instant), to see it as a job in the Jobs pane. */
export function mockExportMs(search = typeof window === 'undefined' ? '' : window.location.search): number {
  const v = new URLSearchParams(search).get('mockExportMs');
  return v !== null && /^\d{1,6}$/.test(v) ? Number(v) : 0;
}

const fileName = (path: string) => path.split(/[\\/]/).pop() ?? path;

/** An old share bundle the mock knows (tests add them): what opening it gives back. */
export interface MockBundle {
  name: string;
  ui: unknown;
  reports: Report[];
  manifest: BundleManifest;
}

const SPARK = 24;

/** `base` with the tile's query-language text appended as the engine does. */
export function mockTileQuery(base: Query, tileQl: string | undefined): Query {
  const tile = (tileQl ?? '').trim();
  const own = (base.ql ?? '').trim();
  if (!own || !tile) return { ...base, ql: own || tile };
  const bar = tile.indexOf('|');
  const filter = (bar < 0 ? tile : tile.slice(0, bar)).trim();
  const stages = bar < 0 ? '' : tile.slice(bar + 1).trim();
  return { ...base, ql: `${own}${filter ? ` | where ${filter}` : ''}${stages ? ` | ${stages}` : ''}` };
}

function durationMs(text: string): number {
  const m = /^(\d+(?:\.\d+)?)\s*(ms|s|m|h|d)?$/.exec(text.trim());
  if (!m) throw new Error(`"${text}" is not a duration`);
  const factor = { ms: 1, s: 1000, m: 60_000, h: 3_600_000, d: 86_400_000 }[(m[2] ?? 'ms') as 'ms'];
  return Number(m[1]) * factor;
}

/**
 * Browser stand-in for the share commands, built on the mock workspace and mock analysis backends.
 * Plausible, never the reference: the engine is.
 */
export class MockShareBackend extends ShareBackend {
  private readonly bundles = new Map<string, MockBundle>();
  /** Every saved image and report (tests and e2e read them). */
  readonly written: { path: string; kind: string; bytes: number }[] = [];
  /** Paths written so far: writing one again needs `overwrite`, like the engine. */
  private readonly files = new MockFiles();

  constructor(
    private readonly imt: ImtBackend,
    private readonly analysis: AnalysisBackend,
  ) {
    super();
    (globalThis as Record<string, unknown>)['__imtMockShareWritten'] = this.written;
  }

  private mock(): MockBackend | null {
    return this.imt instanceof MockBackend ? this.imt : null;
  }

  private async profileModel(): Promise<Record<string, unknown>> {
    return parse((await this.imt.getProfile()).toml) as Record<string, unknown>;
  }

  /** Like the engine: a profile without `[redact]` rules refuses the view with what to add (`invalid`). */
  async setRedactedView(enabled: boolean): Promise<WorkspaceSummary> {
    try {
      this.mock()?.setMockRedaction(enabled);
    } catch (e) {
      throw new CmdError({ code: 'invalid', message: e instanceof Error ? e.message : String(e), retryable: false, hint: 'Edit the profile: add detectors or [[redact.rule]] entries to its [redact] section.' });
    }
    return this.imt.workspaceSummary();
  }

  async previewRedaction(request: RedactionPreviewRequest): Promise<RedactionPreview> {
    return mockRedactionPreview(request.toml ?? (await this.profileModel()), request.sample);
  }

  /** What the profile's rules would change in the first page of matching events (the engine scans up to 200,000). */
  async redactionSummary(request: RedactionSummaryRequest): Promise<RedactionSummary> {
    const r = new MockRedactor(mockRedactCfg(await this.profileModel()), 'summary');
    const page = await this.imt.queryPage({ query: request.query, offset: 0, limit: 5000, descending: false, anchor: null });
    const counts: Record<string, number> = {};
    const samples = [];
    let changed = 0;
    for (const row of page.rows) {
      const before = row.message;
      const after = r.apply(before, counts);
      if (after !== before) {
        changed++;
        if (samples.length < (request.samples ?? 5)) samples.push({ recordId: row.id, before, after });
      }
    }
    return { scanned: page.rows.length, changed, truncated: page.total > page.rows.length, hits: r.hits(counts), samples };
  }

  async dashboardTile(request: TileRequest): Promise<TileData> {
    const tile = request.tile;
    const error = mockTileError(tile);
    if (error) throw new Error(error);
    let base: Query = { ...request.query };
    if (tile.window) base = { ...base, from: null, to: null, window: { kind: 'live', lastMs: durationMs(tile.window) } };
    switch (tile.type) {
      case 'chart': {
        const def = tile.chart
          ? null
          : { id: tile.id, title: tile.title || tile.id, y: tile.value ?? 'count()', bucket: 'auto', splitBy: tile.by ?? [], type: tile.style ?? 'line', unit: tile.unit ?? '' };
        const data = await this.imt.queryChart({ query: mockTileQuery(base, tile.query), chartId: tile.chart ?? null, def, buckets: 120 });
        return { kind: 'chart', data };
      }
      case 'stat':
        return this.stat(tile.query ?? '', base, tile.unit ?? null);
      case 'table': {
        const out = await this.analysis.runQuery(mockTileQuery(base, tile.query));
        if ('error' in out) throw new Error(out.error.message);
        const limit = Math.min(100, Math.max(1, tile.limit ?? 100));
        if (out.result.kind === 'table') return { kind: 'table', columns: out.result.columns, rows: out.result.rows.slice(0, limit), truncated: out.result.rows.length > limit };
        const page = await this.imt.queryPage({ query: { ...EMPTY_QUERY }, offset: 0, limit: 1_000_000, descending: false, anchor: null });
        const byId = new Map(page.rows.map((r) => [r.id, r]));
        const rows = out.result.ids.slice(0, limit).map((id) => {
          const r = byId.get(id);
          return [r?.ts ?? null, r?.level ?? '', r?.message ?? ''];
        });
        return { kind: 'table', columns: ['@t', '@level', '@message'], rows, truncated: out.result.ids.length > limit };
      }
      case 'top':
        return { kind: 'top', facet: await this.imt.queryFacet({ query: mockTileQuery(base, tile.query), field: tile.field ?? '', limit: tile.limit ?? 10 }) };
      case 'alerts':
        return { kind: 'alerts', items: (await this.analysis.queryAnomalies(mockTileQuery(base, tile.query))).result.slice(0, tile.limit ?? 20) };
      case 'annotations':
        return { kind: 'annotations', items: (await this.imt.annotations({ from: base.from ?? null, to: base.to ?? null })).slice(0, tile.limit ?? 20) };
    }
  }

  private async stat(ql: string, base: Query, unit: string | null): Promise<TileData> {
    const value = async (analysis: AnalysisBackend, q: Query): Promise<{ label: string; value: unknown }> => {
      const out = await analysis.runQuery(mockTileQuery(q, ql));
      if ('error' in out) throw new Error(out.error.message);
      if (out.result.kind === 'events') return { label: 'events', value: out.result.ids.length };
      return { label: out.result.columns[out.result.columns.length - 1] ?? '', value: out.result.rows[0]?.[out.result.rows[0].length - 1] ?? null };
    };
    const total = await value(this.analysis, base);
    const summary = await this.imt.workspaceSummary();
    const from = base.from ?? summary.from ?? 0;
    const to = base.to ?? summary.to ?? from;
    const step = Math.max(1, Math.floor((to - from) / SPARK));
    // One snapshot of the workspace for all the buckets, each bucket evaluated over its own slice only: reloading
    // and filtering every record per bucket made a stat tile cost SPARK full scans (seconds on the demo data).
    const records = await recordsFrom(this.imt)();
    const sparkline: [number, number | null][] = [];
    for (let i = 0; i < SPARK; i++) {
      const start = from + i * step;
      const end = i === SPARK - 1 ? to : start + step - 1;
      const slice = records.filter((r) => r.ts !== null && r.ts >= start && r.ts <= end);
      const v = await value(new MockAnalysisBackend(async () => slice), { ...base, window: null, from: start, to: end });
      sparkline.push([start, typeof v.value === 'number' ? v.value : null]);
    }
    const n = typeof total.value === 'number' ? total.value : total.value === null ? null : Number(total.value);
    return { kind: 'stat', value: n !== null && Number.isFinite(n) ? n : null, text: total.value === null ? '' : String(total.value), label: total.label, unit, sparkline };
  }

  async exportReport(request: ReportExportRequest): Promise<number> {
    if (!request.report.id) throw new Error('not a valid report: a report needs an id');
    if (!request.path.toLowerCase().endsWith(`.${request.format}`)) throw new Error(`the file must end in .${request.format}`);
    const counts: Record<string, number> = {};
    if (request.redact) {
      const r = new MockRedactor(mockRedactCfg(await this.profileModel()), 'report');
      r.apply(JSON.stringify(request.report), counts);
    }
    const masked = Object.values(counts).reduce((a, b) => a + b, 0);
    this.files.write(request.path, request.overwrite);
    // The file is checked first (like the engine); writing it is an export job, returned at once.
    const job = mockJobs().start({ kind: 'export', title: `Export report ${fileName(request.path)}`, link: { type: 'path', path: request.path } }, async (ctx) => {
      await mockSteps(ctx, mockExportMs());
      const bytes = JSON.stringify(request.report).length;
      this.written.push({ path: request.path, kind: request.format, bytes });
      ctx.message(request.redact ? `Report written (${masked} values redacted).` : 'Report written.');
      return { value: bytes, result: { type: 'file', path: request.path } };
    });
    job.done.catch(() => undefined);
    return job.id;
  }

  async saveVisual(request: VisualRequest): Promise<VisualSaved> {
    if (!request.path.toLowerCase().endsWith(`.${request.kind}`)) throw new Error(`the file must end in .${request.kind}`);
    if (request.kind === 'png' && !request.data.startsWith('data:image/png;base64,')) throw new Error('not a PNG image');
    if (request.kind === 'svg' && !request.data.trimStart().startsWith('<svg')) throw new Error('an SVG image must have an <svg> root element');
    this.files.write(request.path, request.overwrite);
    const bytes = request.data.length;
    this.written.push({ path: request.path, kind: request.kind, bytes });
    return { path: request.path, bytes };
  }

  /** Findings derived from the mock anomalies, unrecognised files and plugin problems (the engine's health check covers much more). */
  async overview(query: Query): Promise<Analyzed<Finding[]>> {
    const { result: anomalies, coverage } = await this.analysis.queryAnomalies(query);
    const summary = await this.imt.workspaceSummary();
    const severity = (kind: string): Finding['severity'] => (kind === 'crash' ? 'critical' : kind === 'errorSpike' || kind === 'memoryGrowth' || kind === 'silence' ? 'warning' : 'info');
    const rank = { critical: 0, warning: 1, info: 2 };
    const found: Finding[] = anomalies.map((a): Finding => ({
      kind: a.kind,
      severity: severity(a.kind),
      title: a.message,
      detail: `${a.instance ?? 'All instances'} · score ${a.score.toFixed(1)}`,
      action: { kind: 'zoom', from: a.from, to: a.to, recordId: a.sampleIds[0] ?? null },
    }));
    for (const u of summary.unrecognized) {
      found.push({ kind: 'fileNotOpened', severity: 'info', title: 'File not opened', detail: `${u.name}: ${u.reason}`, action: { kind: 'setupFormat', path: u.path } });
    }
    for (const p of summary.plugins.filter((x) => x.status === 'quarantined')) {
      found.push({ kind: 'pluginQuarantined', severity: 'warning', title: 'Plugin disabled after crashes', detail: `${p.name}: ${p.lastError ?? ''}`, action: { kind: 'openPanel', panel: 'plugins' } });
    }
    return { result: found.sort((x, y) => rank[x.severity] - rank[y.severity]), coverage };
  }

  /** Makes `path` an old share bundle that `openBundle` knows (tests). */
  addBundle(path: string, bundle: MockBundle): void {
    this.bundles.set(path, structuredClone(bundle));
  }

  async openBundle(path: string): Promise<BundleOpened> {
    const known = this.bundles.get(path);
    if (!known && !path.toLowerCase().endsWith('.imtz')) throw new Error(`${path} is not an IMT bundle (not a zip file)`);
    const summary = await this.imt.workspaceSummary();
    return {
      path,
      name: known?.name ?? path.split(/[\\/]/).pop()!.replace(/\.imtz$/i, ''),
      summary,
      ui: known ? structuredClone(known.ui) : null,
      reports: known ? structuredClone(known.reports) : [],
      missing: [],
      readOnly: true,
      bundle: known?.manifest ?? null,
      folder: `/tmp/imt-bundles/${encodeURIComponent(path)}`,
    };
  }
}
