import { parse, TomlError } from 'smol-toml';
import {
  ChartData,
  ChartRequest,
  ContextRequest,
  ExportRequest,
  ExportResult,
  ExplainNode,
  ExtractPreview,
  ExtractPreviewRequest,
  Facet,
  FacetRequest,
  InferredRegex,
  LEVELS,
  Level,
  Page,
  PageRequest,
  ParsePreview,
  ParsePreviewRequest,
  PluginInfo,
  ProfileDoc,
  ProfileValidation,
  SourceContext,
  Timeline,
  TimelineRequest,
  WorkspaceSummary,
} from '../api.types';
import type { ImtBackend } from '../backend';
import { ProfileModel } from '../profile.model';
import { generateDemo, generateExtraSource, MockDataset } from './mock-data';
import { MockEngine, safeRegex } from './mock-engine';
import type { Failable } from './mock-options';
import { mockPreviewParse } from './mock-parse';
import { mockExplainRegex, mockInferRegex } from './mock-regex';
import { syntheticFacet, syntheticPage } from './mock-synthetic';
import { MOCK_PROFILE_TOML } from './mock-profile';
import { BUILTIN_ICONS } from '../../markers/builtin-icons';

/** Built-in marker icon names the engine accepts (mirrors MARKER_ICONS). */
const MOCK_ICONS: ReadonlySet<string> = new Set(BUILTIN_ICONS.map((i) => i.name));

export interface MockBackendOptions {
  /** Simulate IPC latency so loading states are visible (default true). Tests turn it off. */
  latency?: boolean;
  /** Dataset factory (default: the generated demo incident). */
  dataset?: () => MockDataset;
  /** Pretend the result has this many events, generated lazily by index (see mock-synthetic.ts). Queries are ignored. */
  syntheticTotal?: number;
  /** Every field reports this many distinct values; facets return at most 1000 of them, like the engine. */
  syntheticFacetValues?: number;
  /** The first N calls of each named command reject with a simulated error. */
  fail?: Partial<Record<Failable, number>>;
  /** Page rows lose their `markers`, as a buggy backend would send them (panel error boundary tests). */
  corruptMarkers?: boolean;
  /** Interval between live tail events (default 1500 ms). */
  liveMs?: number;
}

/** In-browser stand-in for the Rust backend (used by `ng serve`). */
export class MockBackend implements ImtBackend {
  private dataset: MockDataset;
  private profileToml = MOCK_PROFILE_TOML;
  private profile: ProfileModel;
  private engine = new MockEngine();
  private loadMs = 0;
  private nextSourceId = 100;

  private readonly latency: boolean;
  private readonly makeDataset: () => MockDataset;
  private readonly options: MockBackendOptions;
  private readonly failuresLeft: Partial<Record<Failable, number>>;
  private readonly changeHandlers = new Set<() => void>();
  private liveTimer: ReturnType<typeof setInterval> | undefined;
  /** Events appended by live tail since the start. */
  private liveAdded = 0;

  constructor(options: MockBackendOptions = {}) {
    this.options = options;
    this.failuresLeft = { ...options.fail };
    this.latency = options.latency ?? true;
    this.makeDataset = options.dataset ?? (() => generateDemo());
    this.dataset = this.makeDataset();
    this.profile = parse(this.profileToml) as ProfileModel;
    this.apply();
  }

  /** Rejections carry a plain message string, like Tauri command errors. */
  private delay<T>(value: () => T, ms = 60): Promise<T> {
    const run = (resolve: (v: T) => void, reject: (e: string) => void) => {
      try {
        resolve(value());
      } catch (e) {
        reject(e instanceof Error ? e.message : String(e));
      }
    };
    return new Promise((resolve, reject) => (this.latency ? setTimeout(() => run(resolve, reject), ms) : run(resolve, reject)));
  }

  /** Rejects (like a failing Tauri command) while the command still has simulated failures left. */
  private failing(command: Failable): Promise<never> | null {
    const left = this.failuresLeft[command] ?? 0;
    if (left <= 0) return null;
    this.failuresLeft[command] = left - 1;
    return this.delay<never>(() => {
      throw new Error(`Simulated ${command} failure`);
    });
  }

  private syntheticCount(): number {
    return (this.options.syntheticTotal ?? 0) + this.liveAdded;
  }

  private apply(): void {
    const t0 = performance.now();
    this.engine.rebuild(this.dataset.records, this.dataset.sources, this.profile);
    this.loadMs = Math.round(performance.now() - t0);
  }

  private summary(): WorkspaceSummary {
    const e = this.engine;
    const levels = Object.fromEntries(LEVELS.map((l) => [l, 0])) as Record<Level, number>;
    const perSource = new Map<number, { n: number; from: number | null; to: number | null }>();
    let from: number | null = null;
    let to: number | null = null;
    for (const r of e.records) {
      levels[r.level]++;
      const s = perSource.get(r.source) ?? { n: 0, from: null, to: null };
      s.n++;
      if (r.ts !== null) {
        s.from = s.from === null ? r.ts : Math.min(s.from, r.ts);
        s.to = s.to === null ? r.ts : Math.max(s.to, r.ts);
        from = from === null ? r.ts : Math.min(from, r.ts);
        to = to === null ? r.ts : Math.max(to, r.ts);
      }
      perSource.set(r.source, s);
    }
    const synthetic = this.options.syntheticTotal ? this.syntheticCount() : null;
    return {
      sources: e.sources.map((s, i) => ({
        id: s.id,
        path: s.path,
        name: s.name,
        importer: s.importer,
        records: synthetic !== null && i === 0 ? synthetic : (perSource.get(s.id)?.n ?? 0),
        skipped: s.skipped,
        dropped: e.droppedBySource.get(s.id) ?? 0,
        error: null,
        from: perSource.get(s.id)?.from ?? null,
        to: perSource.get(s.id)?.to ?? null,
      })),
      unrecognized: this.dataset.unrecognized,
      total: synthetic ?? e.records.length,
      from,
      to,
      levels,
      fields: e.fields(),
      defaultGroupBy: e.defaultGroupBy,
      markerKinds: e.markerKinds,
      icons: e.icons,
      charts: e.charts,
      profileName: this.profile.name ?? 'Unnamed profile',
      loadMs: this.loadMs,
      dropped: [...e.droppedBySource.values()].reduce((a, b) => a + b, 0),
      plugins: MOCK_PLUGINS,
      exporters: MOCK_EXPORTERS,
      pipeline: e.pipelineStats,
    };
  }

  workspaceSummary() {
    return this.failing('workspaceSummary') ?? this.delay(() => this.summary());
  }

  openPaths(paths: string[]) {
    return this.delay(() => {
      for (const path of paths) {
        if (this.dataset.sources.some((s) => s.path === path)) continue;
        const id = this.nextSourceId++;
        const extra = generateExtraSource(id, path, id);
        this.dataset.sources.push(extra.source);
        this.dataset.records.push(...extra.records);
      }
      this.apply();
      return this.summary();
    }, 300);
  }

  removeSource(sourceId: number) {
    return this.delay(() => {
      this.dataset.sources = this.dataset.sources.filter((s) => s.id !== sourceId);
      this.dataset.records = this.dataset.records.filter((r) => r.source !== sourceId);
      this.apply();
      return this.summary();
    });
  }

  clearWorkspace() {
    return this.delay(() => {
      this.dataset = { sources: [], unrecognized: [], records: [] };
      this.apply();
      return this.summary();
    });
  }

  reloadWorkspace() {
    return this.delay(() => {
      this.dataset = this.makeDataset();
      this.apply();
      return this.summary();
    }, 400);
  }

  queryPage(request: PageRequest): Promise<Page> {
    return (
      this.failing('queryPage') ??
      this.delay(() => {
        const page = this.options.syntheticTotal ? syntheticPage(request, this.syntheticCount()) : this.engine.page(request);
        return this.options.corruptMarkers ? { ...page, rows: page.rows.map((r) => ({ ...r, markers: undefined as never })) } : page;
      }, 20)
    );
  }

  queryTimeline(request: TimelineRequest): Promise<Timeline> {
    return this.failing('queryTimeline') ?? this.delay(() => this.engine.timeline(request), 30);
  }

  queryFacet(request: FacetRequest): Promise<Facet> {
    return (
      this.failing('queryFacet') ??
      this.delay(() => (this.options.syntheticFacetValues ? syntheticFacet(request, this.options.syntheticFacetValues) : this.engine.facet(request)), 20)
    );
  }

  queryChart(request: ChartRequest): Promise<ChartData> {
    return this.delay(() => this.engine.chart(request), 30);
  }

  getProfile(): Promise<ProfileDoc> {
    return this.delay(() => ({ name: this.profile.name ?? 'Unnamed profile', path: null, toml: this.profileToml }));
  }

  validateProfile(toml: string): Promise<ProfileValidation> {
    return this.delay(() => validate(toml).validation, 10);
  }

  saveProfile(toml: string): Promise<WorkspaceSummary> {
    return this.delay(() => {
      const { validation, model } = validate(toml);
      if (!validation.ok || !model) throw new Error(validation.error ?? 'Invalid profile');
      this.profileToml = toml;
      this.profile = model;
      this.apply();
      return this.summary();
    }, 250);
  }

  previewExtractor(request: ExtractPreviewRequest): Promise<ExtractPreview> {
    return this.delay(() => {
      const { validation, model } = validate(request.toml);
      if (!model) return { candidates: 0, matches: 0, failed: 0, samples: [], error: validation.error };
      const rule = model.extract?.find((x) => x.name === request.extractor);
      if (!rule) return { candidates: 0, matches: 0, failed: 0, samples: [], error: `No extractor named '${request.extractor}'` };
      return this.engine.previewExtractor(rule, request.query, request.limit);
    }, 40);
  }

  importProfile(path: string): Promise<ProfileDoc> {
    // Browser demo: there is no file system, so "importing" returns the demo profile under the chosen name.
    return this.delay(() => ({ name: this.profile.name ?? 'Imported profile', path, toml: this.profileToml }));
  }

  exportEvents(request: ExportRequest): Promise<ExportResult> {
    return this.delay(() => {
      if (!MOCK_EXPORTERS.some((x) => x.id === request.format)) throw new Error(`Unknown export format '${request.format}'`);
      // The browser demo writes nothing: it only reports what the desktop app would export.
      return { format: request.format, path: request.path, events: this.engine.matching(request.query).length };
    }, 100);
  }

  sourceContext(request: ContextRequest): Promise<SourceContext> {
    return (
      this.failing('sourceContext') ??
      this.delay(() => {
        const rec = this.engine.records[request.recordId];
        if (!rec) throw new Error('That event is no longer in the workspace.');
        const before = Math.max(0, Math.min(500, Math.floor(request.before)));
        const after = Math.max(0, Math.min(500, Math.floor(request.after)));
        const first = Math.max(1, rec.line - before);
        const lines = [];
        for (let n = first; n <= rec.line + after; n++) {
          lines.push({ line: n, text: n === rec.line ? `${rec.ts === null ? '' : new Date(rec.ts).toISOString()} [${rec.level}] ${rec.message}` : `context line ${n} of ${this.engine.sources.find((s) => s.id === rec.source)?.name ?? ''}` });
        }
        return { sourceName: this.engine.sources.find((s) => s.id === rec.source)?.name ?? '', line: rec.line, lines };
      }, 20)
    );
  }

  readFileHead(path: string, maxBytes = 65536): Promise<string> {
    return (
      this.failing('readFileHead') ??
      this.delay(() => {
        const known = this.dataset.sources.some((s) => s.path === path) || this.dataset.unrecognized.some((u) => u.path === path);
        if (!known) throw new Error('only files in the workspace can be previewed');
        return SAMPLE_HEAD.slice(0, Math.max(1, maxBytes));
      }, 20)
    );
  }

  previewParse(request: ParsePreviewRequest): Promise<ParsePreview> {
    return this.failing('previewParse') ?? this.delay(() => mockPreviewParse(request), 20);
  }

  setLive(enabled: boolean): Promise<number> {
    return this.delay(() => {
      clearInterval(this.liveTimer);
      this.liveTimer = undefined;
      if (enabled) this.liveTimer = setInterval(() => this.liveTick(), this.options.liveMs ?? 1500);
      return enabled ? this.dataset.sources.length : 0;
    }, 10);
  }

  onWorkspaceChanged(handler: () => void): Promise<() => void> {
    this.changeHandlers.add(handler);
    return Promise.resolve(() => void this.changeHandlers.delete(handler));
  }

  /** One new event arrives in the newest source, then listeners are told, like the engine's file watcher does. */
  private liveTick(): void {
    this.liveAdded++;
    if (!this.options.syntheticTotal) {
      const last = this.dataset.records.reduce((m, r) => Math.max(m, r.ts ?? 0), 0);
      const source = this.dataset.sources[0];
      if (source) {
        this.dataset.records.push({ id: 0, ts: last + 1000, level: 'Information', message: `Live event ${this.liveAdded}`, template: null, exception: null, fields: { Live: true }, source: source.id, line: 100_000 + this.liveAdded, markers: [] });
        this.apply();
      }
    }
    for (const handler of [...this.changeHandlers]) handler();
  }

  inferRegex(lines: string[]): Promise<InferredRegex> {
    return this.delay(() => mockInferRegex(lines), 40);
  }

  explainRegex(pattern: string): Promise<ExplainNode[]> {
    return this.delay(() => {
      if (!safeRegex(pattern)) throw new Error('regex parse error: the pattern is not a valid regex');
      return mockExplainRegex(pattern);
    }, 20);
  }

  exportProfile(path: string, toml: string): Promise<void> {
    return this.delay(() => {
      const url = URL.createObjectURL(new Blob([toml], { type: 'application/toml' }));
      const a = document.createElement('a');
      a.href = url;
      a.download = path.split(/[\\/]/).pop() || 'profile.toml';
      a.click();
      URL.revokeObjectURL(url);
    });
  }
}

const MOCK_PLUGINS: PluginInfo[] = [
  { id: 'registry', name: 'Windows registry importer', version: '0.1.0', contributes: ['importer'], status: 'active', failures: 0, lastError: null },
  { id: 'export-basic', name: 'CSV and CLEF exporters', version: '0.1.0', contributes: ['exporter'], status: 'active', failures: 0, lastError: null },
];
/** What `readFileHead` returns for any file in the browser demo. */
const SAMPLE_HEAD = [
  '2026-10-14 10:41:02.114 +00:00 [INF] Order 1042 accepted for customer 77',
  '2026-10-14 10:41:02.980 +00:00 [WRN] Payment gateway slow: 1500 ms',
  '2026-10-14 10:41:03.201 +00:00 [ERR] Payment capture failed for order 1042',
  '   at Payments.Capture() in Capture.cs:line 88',
  '2026-10-14 10:41:04.007 +00:00 [INF] Order 1043 accepted for customer 12',
  '',
].join('\n');
const MOCK_EXPORTERS = [
  { id: 'csv', name: 'CSV', extension: 'csv' },
  { id: 'clef', name: 'Compact Log Event Format', extension: 'clef' },
];

function validate(toml: string): { validation: ProfileValidation; model: ProfileModel | null } {
  let model: ProfileModel;
  try {
    model = parse(toml) as ProfileModel;
  } catch (e) {
    const err = e as TomlError;
    return { validation: { ok: false, error: err.message.split('\n')[0], line: err.line ?? null }, model: null };
  }
  const lines = toml.split('\n');
  const lineOf = (needle: string) => {
    const i = lines.findIndex((l) => l.includes(needle));
    return i >= 0 ? i + 1 : null;
  };
  for (const x of model.extract ?? []) {
    if (!x.name) return { validation: { ok: false, error: '[[extract]] is missing a name', line: lineOf('[[extract]]') }, model };
    if (!safeRegex(x.regex ?? '')) return { validation: { ok: false, error: `Extractor '${x.name}': invalid regex`, line: lineOf(x.regex) }, model };
  }
  for (const m of model.marker ?? []) {
    if (!m.kind) return { validation: { ok: false, error: '[[marker]] is missing a kind', line: lineOf('[[marker]]') }, model };
    if (m['badge'] !== undefined && [...String(m['badge'])].length > 3) return { validation: { ok: false, error: `marker "${m.kind}": badge must be 1 to 3 characters`, line: lineOf('badge') }, model };
    if (m['icon'] && !(model['icon'] as { name: string }[] | undefined)?.some((i) => i.name === m['icon']) && !MOCK_ICONS.has(String(m['icon']))) {
      return { validation: { ok: false, error: `marker "${m.kind}": unknown icon "${m['icon']}"`, line: lineOf('icon') }, model };
    }
  }
  for (const c of model.chart ?? []) {
    if (!c.id || !c.y) return { validation: { ok: false, error: '[[chart]] needs id and y', line: lineOf('[[chart]]') }, model };
    if (!/^(count|avg|min|max|sum|last)\(\s*[^)]*\)$/.test(c.y)) return { validation: { ok: false, error: `Chart '${c.id}': y must look like avg(field)`, line: lineOf(c.y) }, model };
  }
  return { validation: { ok: true, error: null, line: null }, model };
}
