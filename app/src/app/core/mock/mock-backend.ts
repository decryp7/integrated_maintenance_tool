import { parse, TomlError } from 'smol-toml';
import {
  AlertTarget,
  AnnotationDto,
  AnnotationsRequest,
  ChartData,
  ChartRequest,
  ContextRequest,
  ExportRequest,
  ExplainNode,
  ExtractPreview,
  ExtractPreviewRequest,
  Facet,
  FacetRequest,
  InferredRegex,
  LEVELS,
  Level,
  LiveAlert,
  LiveOptions,
  LiveState,
  Page,
  PageRequest,
  PartialMatches,
  ParsePreview,
  ParsePreviewRequest,
  PluginInfo,
  ProfileDoc,
  ProfileValidation,
  Query,
  SourceContext,
  SubscriberMetrics,
  TailReport,
  TailStatus,
  Timeline,
  TimelineRequest,
  WorkspaceSummary,
} from '../api.types';
import type { ImtBackend } from '../backend';
import { ProfileModel } from '../profile.model';
import { generateDemo, generateExtraSource, MockDataset, MockRecord } from './mock-data';
import { mockAnnotations } from './mock-annotations';
import { mockPreviewSpans, mockQueryRuns, mockRunAnnotations, RunContext, spanRules } from './mock-runs';
import { RunsReply, RunsRequest, SpanPreview, SpanPreviewRequest } from '../api.types';
import { MockEngine, safeRegex } from './mock-engine';
import { MOCK_PLUGIN_KINDS, MOCK_WINDOWS_PLUGIN, MockPluginKind } from './mock-plugin-markers';
import type { Failable } from './mock-options';
import { mockPreviewParse } from './mock-parse';
import { mockExplainRegex, mockInferRegex } from './mock-regex';
import { syntheticFacet, syntheticPage } from './mock-synthetic';
import { MOCK_PROFILE_TOML } from './mock-profile';
import { mockJobs, mockSteps } from '../../jobs/mock-jobs';
import { emptyReport, LEGACY_LIVE, MockAlertMutes, MockLiveConfig, MockLiveSim, mockGroupOf, mockResolveLiveOptions, mockResolveWindow, mockWatchMode } from './mock-live';
import { validateLiveSection } from './mock-live-profile';
import { BUILTIN_ICONS } from '../../markers/builtin-icons';
import { mockDashboards, mockDashboardsError } from '../../share/mock/mock-dashboards';
import { mockRedactCfg, MockRedactor } from '../../share/mock/mock-redact';
import { OffsetChange, SetOffsetRequest, SuggestRequest, SuggestResult } from '../api.types';
import { CmdError } from '../cmd-error';
import { mockSuggest, recordKey, resolveOffset, shiftSource } from './mock-skew';
import { FileOpened, ImportProgress, OpenFileRequest, ProfileVersionInfo } from '../api.types';
import { mockFileKind, mockOpenSourceFile } from './mock-files';
import { mockNotifications } from '../../notifications/mock-notifications';
import { timestampNotice, withTimestampIssue } from './mock-timestamps';
import { MockParserLibrary, MockParserSource } from './mock-parsers';
import { isImagePath } from '../../ai/vision/image-files';
import { IDLE_PROGRESS, isMockDemoPath, MockFiles, MockImportSim, MockProfileHistory, NOT_IMPORTED_REASON, slowImportFiles } from './mock-import';
import { MockClock, MockTimer, systemMockClock } from './mock-clock';

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
  /** Interval between live tail events (default 1500 ms; 250 ms with `liveRate`). */
  liveMs?: number;
  /** Realistic live simulation: this many records per second across the tailed files (default: one per tick). */
  liveRate?: number;
  /** With `liveRate`: rotate the tailed file every this many ms (default 30000; 0 = never). */
  liveRotateMs?: number;
  /** With `liveRate`: raise a live alert every this many ms (default 10000; 0 = never). */
  liveAlertMs?: number;
  /** Publish the `setLive` calls as `globalThis.__imtMockLiveCalls` (end-to-end tests spy on them). */
  exposeLiveCalls?: boolean;
  /** Marker kinds contributed by (mock) plugins (default: the Windows plugin's; `[]` = none, like safe mode). */
  pluginMarkerKinds?: readonly MockPluginKind[];
  /** Opening paths is slow and cancellable: each path stands for a folder of files read this many ms apart. */
  slowImportMs?: number;
  /** The clock of every simulated delay, live tick and import step (default: the page's timers). */
  clock?: MockClock;
}

/** One recorded `setLive` call. */
export interface MockLiveCall {
  enabled: boolean;
  options: LiveOptions | null;
}

/** In-browser stand-in for the Rust backend (used by `ng serve`). */
export class MockBackend implements ImtBackend {
  private dataset: MockDataset;
  private profileToml = MOCK_PROFILE_TOML;
  private profile: ProfileModel;
  private engine: MockEngine;
  private loadMs = 0;
  private nextSourceId = 100;

  private readonly latency: boolean;
  private readonly makeDataset: () => MockDataset;
  private readonly options: MockBackendOptions;
  private readonly failuresLeft: Partial<Record<Failable, number>>;
  private readonly changeHandlers = new Set<(report: TailReport | null) => void>();
  private readonly alertHandlers = new Set<(alert: LiveAlert) => void>();
  private readonly clock: MockClock;
  private liveTimer: MockTimer | undefined;
  /** Events appended by live tail since the start. */
  private liveAdded = 0;
  private liveOn = false;
  private liveOptions: LiveOptions | undefined;
  private liveSim: MockLiveSim;
  private readonly mutes = new MockAlertMutes(() => this.clock.now());
  /** Records evicted by the simulated retention limit. */
  private evicted = 0;
  /** Every `setLive` call, in order (tests spy on it). */
  readonly liveCalls: MockLiveCall[] = [];
  /** The redacted view (see `setMockRedaction`); null while off. */
  private redactor: MockRedactor | null = null;
  /** The slow import of `slowImportMs` (null: opening is quick and reports no progress). */
  private readonly importSim: MockImportSim | null;
  /** Progress of the last quick open (a slow one reports its own). */
  private quickProgress: ImportProgress = IDLE_PROGRESS;
  /** Profiles replaced by `saveProfile`, like the shell's profile history. */
  private readonly history = new MockProfileHistory();
  /** Files "written" by exports (a second export to a path needs `overwrite`). */
  readonly files = new MockFiles();
  /** The parser library (ADR 0026): which parser read each source; `MockParsersBackend` manages it. */
  readonly parsers = new MockParserLibrary();

  constructor(options: MockBackendOptions = {}) {
    this.options = options;
    this.clock = options.clock ?? systemMockClock;
    this.failuresLeft = { ...options.fail };
    this.latency = options.latency ?? true;
    this.makeDataset = options.dataset ?? (() => generateDemo());
    this.engine = new MockEngine(options.pluginMarkerKinds ?? MOCK_PLUGIN_KINDS);
    this.dataset = this.makeDataset();
    this.profile = parse(this.profileToml) as ProfileModel;
    this.liveSim = new MockLiveSim(liveConfig(options), () => this.nextSourceId++);
    this.importSim = options.slowImportMs ? new MockImportSim(options.slowImportMs, this.clock) : null;
    if (options.exposeLiveCalls) (globalThis as Record<string, unknown>)['__imtMockLiveCalls'] = this.liveCalls;
    this.apply();
  }

  /** Rejections carry a plain message string, like Tauri command errors, or a `CmdError` where the engine sends a coded one. */
  private delay<T>(value: () => T, ms = 60): Promise<T> {
    const run = (resolve: (v: T) => void, reject: (e: string | CmdError) => void) => {
      try {
        resolve(value());
      } catch (e) {
        reject(e instanceof CmdError ? e : e instanceof Error ? e.message : String(e));
      }
    };
    return new Promise((resolve, reject) => (this.latency ? this.clock.setTimeout(() => run(resolve, reject), ms) : run(resolve, reject)));
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
    const t0 = this.clock.elapsed();
    this.engine.rebuild(this.dataset.records, this.dataset.sources, this.profile);
    this.loadMs = Math.round(this.clock.elapsed() - t0);
  }

  /** The open sources as the parser library sees them. */
  parserSources(): MockParserSource[] {
    return this.engine.sources.map((s) => ({ id: s.id, path: s.path, name: s.name, importer: s.importer }));
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
        group: mockGroupOf(s.path),
        importer: s.importer,
        records: synthetic !== null && i === 0 ? synthetic : (perSource.get(s.id)?.n ?? 0),
        skipped: s.skipped,
        dropped: e.droppedBySource.get(s.id) ?? 0,
        error: null,
        from: perSource.get(s.id)?.from ?? null,
        to: perSource.get(s.id)?.to ?? null,
        offsetMs: s.offsetMs ?? 0,
        fileKind: mockFileKind(s.path),
        timestampIssue: s.timestampIssue ?? null,
        routedFrom: s.routedFrom ?? null,
        problems: s.problems ?? [],
        parser: this.parsers.recordedId(s),
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
      evicted: this.evicted,
      plugins: MOCK_PLUGINS,
      exporters: MOCK_EXPORTERS,
      pipeline: e.pipelineStats,
      dashboards: mockDashboards(this.profile),
      redactedView: this.redactor !== null,
      untimed: synthetic !== null ? 0 : e.records.filter((r) => r.ts === null).length,
    };
  }

  /**
   * Switches the redacted view (used by the mock share backend): pages, facets and source context are
   * redacted with the profile's `[redact]` rules until switched off. Throws when there are no rules.
   */
  setMockRedaction(on: boolean): void {
    if (!on) {
      this.redactor = null;
      return;
    }
    const redactor = new MockRedactor(mockRedactCfg(this.profile), String(Math.random()));
    if (redactor.empty) throw new Error('the profile has no redaction rules: add detectors or [[redact.rule]] to its [redact] section');
    this.redactor = redactor;
  }

  /** The profile's `[redact]` rules with a fresh salt (exports). */
  mockExportRedactor(): MockRedactor {
    return new MockRedactor(mockRedactCfg(this.profile), String(Math.random()));
  }

  private redactPage(page: Page): Page {
    const r = this.redactor;
    if (!r) return page;
    const fields = (f: Record<string, unknown>) =>
      Object.fromEntries(Object.entries(f).map(([k, v]) => [k, typeof v === 'string' ? r.field(k, v) : v]));
    return {
      ...page,
      rows: page.rows.map((row) => ({
        ...row,
        message: r.field('@message', row.message),
        template: row.template === null ? null : r.field('@template', row.template),
        exception: row.exception === null ? null : r.field('@exception', row.exception),
        fields: fields(row.fields),
      })),
    };
  }

  private redactFacet(facet: Facet): Facet {
    const r = this.redactor;
    if (!r) return facet;
    const merged = new Map<string, number>();
    for (const v of facet.values) {
      const key = r.field(facet.field, v.value);
      merged.set(key, (merged.get(key) ?? 0) + v.count);
    }
    const values = [...merged].map(([value, count]) => ({ value, count })).sort((a, b) => b.count - a.count || a.value.localeCompare(b.value));
    return { ...facet, values, distinct: values.length };
  }

  workspaceSummary() {
    return this.failing('workspaceSummary') ?? this.delay(() => this.summary());
  }

  openPaths(paths: string[]): Promise<WorkspaceSummary> {
    const demo = paths.filter(isMockDemoPath);
    const others = paths.filter((p) => !isMockDemoPath(p));
    if (demo.length) this.addDemo();
    if (this.importSim) return this.openSlowly(others);
    return this.delay(() => {
      for (const path of others) this.addExtra(path);
      this.apply();
      this.quickProgress = { ...IDLE_PROGRESS, done: paths.length, total: paths.length };
      this.reportTimestampIssues();
      return this.summary();
    }, 300);
  }

  /** Adds one generated source for `path` (once). */
  private addExtra(path: string): void {
    if (this.dataset.sources.some((s) => s.path === path)) return;
    // No importer reads images: they are listed as unrecognised (the Explorer offers them to the AI, ADR 0025).
    if (isImagePath(path)) {
      if (!this.dataset.unrecognized.some((u) => u.path === path)) this.dataset.unrecognized.push({ path, name: path.split(/[\\/]/).pop() ?? path, reason: 'An image, not a log file' });
      return;
    }
    const id = this.nextSourceId++;
    const generated = generateExtraSource(id, path, id);
    const extra = withTimestampIssue(path, generated.source, generated.records);
    this.dataset.sources.push(extra.source);
    this.dataset.records.push(...extra.records);
  }

  /** The demo incident (the bundled samples' stand-in), merged into the workspace unless it is already there. */
  private addDemo(): void {
    const demo = generateDemo();
    if (!this.dataset.sources.length) {
      this.dataset = { ...demo, unrecognized: [...demo.unrecognized] };
      return;
    }
    const ids = new Map<number, number>();
    for (const s of demo.sources) {
      if (this.dataset.sources.some((x) => x.path === s.path)) continue;
      const id = this.nextSourceId++;
      ids.set(s.id, id);
      this.dataset.sources.push({ ...s, id });
    }
    this.dataset.records.push(...demo.records.filter((r) => ids.has(r.source)).map((r) => ({ ...r, source: ids.get(r.source)! })));
  }

  /**
   * A slow, cancellable import, run as an import job whose progress carries the files, bytes and current file (like
   * the engine's): the files not read before a cancel are listed as not imported.
   */
  private async openSlowly(paths: string[]): Promise<WorkspaceSummary> {
    const sim = this.importSim!;
    const files = slowImportFiles(paths);
    const job = mockJobs().start({ kind: 'import', title: `Open ${paths.length === 1 ? (paths[0] ?? '') : `${paths.length} items`}`, cancellable: false }, async (ctx) => {
      const value = await sim.run(
        files,
        (file) => this.addExtra(file),
        (p) =>
          ctx.progress({
            fraction: p.bytesTotal ? p.bytesDone / p.bytesTotal : null,
            done: p.done,
            total: p.total,
            etaMs: p.etaMs,
            bytesDone: p.bytesDone,
            bytesTotal: p.bytesTotal,
            ...(p.current ? { current: p.current } : {}),
          }),
      );
      return { value, result: { type: 'workspace' } };
    });
    const { skipped } = await job.done;
    for (const file of skipped) {
      if (!this.dataset.unrecognized.some((u) => u.path === file)) this.dataset.unrecognized.push({ path: file, name: file.split('/').pop() ?? file, reason: NOT_IMPORTED_REASON });
    }
    this.apply();
    return this.summary();
  }

  /** `import_progress`: the slow import's progress, or that of the last quick open. */
  importProgress(): Promise<ImportProgress> {
    return Promise.resolve(this.importSim ? this.importSim.progress() : { ...this.quickProgress });
  }

  /** Cancel of the import (the Tauri backend cancels its job). */
  cancelImport(): Promise<void> {
    this.importSim?.cancel();
    return Promise.resolve();
  }

  /** `profile_history`. */
  profileHistory(): Promise<ProfileVersionInfo[]> {
    return this.delay(() => this.history.list(), 20);
  }

  /** `profile_version`. */
  profileVersion(file: string): Promise<ProfileDoc> {
    return this.delay(() => ({ ...this.history.get(file), path: null }), 20);
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

  /** As the engine does after an import: one message per file whose timestamps were not read (once per file). */
  private reportTimestampIssues(): void {
    for (const s of this.dataset.sources) {
      const notice = timestampNotice(s);
      if (notice) mockNotifications().post(notice);
    }
  }

  reloadWorkspace() {
    return this.delay(() => {
      this.dataset = this.makeDataset();
      this.apply();
      this.reportTimestampIssues();
      return this.summary();
    }, 400);
  }

  queryPage(request: PageRequest): Promise<Page> {
    return (
      this.failing('queryPage') ??
      this.delay(() => {
        const page = this.redactPage(this.options.syntheticTotal ? syntheticPage(request, this.syntheticCount()) : this.engine.page(this.windowed(request)));
        return this.options.corruptMarkers ? { ...page, rows: page.rows.map((r) => ({ ...r, markers: undefined as never })) } : page;
      }, 20)
    );
  }

  searchPartial(query: Query): Promise<PartialMatches | null> {
    return this.failing('searchPartial') ?? this.delay(() => this.engine.partialMatches(query), 60);
  }

  queryTimeline(request: TimelineRequest): Promise<Timeline> {
    return this.failing('queryTimeline') ?? this.delay(() => this.engine.timeline(this.windowed(request)), 30);
  }

  queryFacet(request: FacetRequest): Promise<Facet> {
    return (
      this.failing('queryFacet') ??
      this.delay(() => this.redactFacet(this.options.syntheticFacetValues ? syntheticFacet(request, this.options.syntheticFacetValues) : this.engine.facet(this.windowed(request))), 20)
    );
  }

  queryChart(request: ChartRequest): Promise<ChartData> {
    return this.delay(() => this.engine.chart(this.windowed(request)), 30);
  }

  getProfile(): Promise<ProfileDoc> {
    return this.delay(() => ({ name: this.profile.name ?? 'Unnamed profile', path: null, toml: this.profileToml }));
  }

  validateProfile(toml: string): Promise<ProfileValidation> {
    return this.delay(() => validate(toml).validation, 10);
  }

  /** Like the engine: checked and saved now, re-imported as an import job (resolves with it at once). */
  saveProfile(toml: string): Promise<number> {
    return this.delay(() => this.startSaveProfile(toml), 0);
  }

  private startSaveProfile(toml: string): number {
    const { validation, model } = validate(toml);
    if (!validation.ok || !model) throw new Error(validation.error ?? 'Invalid profile');
    this.history.archive(this.profileToml, this.profile.name ?? 'Unnamed profile', this.clock.now());
    this.profileToml = toml;
    this.profile = model;
    const job = mockJobs().start({ kind: 'import', title: `Apply the profile ${model.name ?? 'Unnamed profile'}` }, async (ctx) => {
      await mockSteps(ctx, 250, 5);
      if (this.redactor) {
        const next = new MockRedactor(mockRedactCfg(model), String(Math.random()));
        this.redactor = next.empty ? null : next;
      }
      this.apply();
      return { value: this.summary(), result: { type: 'workspace' } };
    });
    job.done.catch(() => undefined);
    return job.id;
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

  /** Like the engine: the file is checked now, the export is a job (resolves with it at once). */
  exportEvents(request: ExportRequest): Promise<number> {
    return this.delay(() => this.startExport(request), 0);
  }

  private startExport(request: ExportRequest): number {
    if (!MOCK_EXPORTERS.some((x) => x.id === request.format)) throw new Error(`Unknown export format '${request.format}'`);
    this.files.write(request.path, request.overwrite);
    const file = request.path.split(/[\\/]/).pop() ?? request.path;
    const job = mockJobs().start({ kind: 'export', title: `Export events to ${file}`, link: { type: 'path', path: request.path } }, async (ctx) => {
      await mockSteps(ctx, 100, 5);
      // The browser demo writes nothing: it only reports what the desktop app would export.
      const matched = this.engine.matching(this.windowed(request).query);
      const redactions: Record<string, number> = {};
      if (request.redact) {
        const r = this.mockExportRedactor();
        for (const id of matched) {
          const rec = this.engine.records[id];
          if (rec) r.apply(`${rec.message}\n${rec.exception ?? ''}`, redactions);
        }
      }
      const masked = Object.values(redactions).reduce((x, y) => x + y, 0);
      ctx.message(request.redact ? `Exported ${matched.length} events (redacted, ${masked} values masked).` : `Exported ${matched.length} events.`);
      return { value: matched.length, result: { type: 'file', path: request.path } };
    });
    job.done.catch(() => undefined);
    return job.id;
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
        const r = this.redactor;
        return { sourceName: this.engine.sources.find((s) => s.id === rec.source)?.name ?? '', line: rec.line, lines: r ? lines.map((l) => ({ ...l, text: r.apply(l.text) })) : lines };
      }, 20)
    );
  }

  /** `FilesBackend.openSourceFile`: the engine's checks; nothing is opened in a browser. */
  openSourceFile(request: OpenFileRequest): Promise<FileOpened> {
    return this.delay(() => mockOpenSourceFile(this.engine.sources, request), 20);
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
    return (
      this.failing('previewParse') ??
      this.delay(() => {
        if (request.toml.trim()) return { ...mockPreviewParse(request), parser: null };
        // No candidate parser: the one the library would choose for the sample.
        const parser = this.parsers.chooseFor(request.fileName, request.sample);
        return { ...mockPreviewParse({ ...request, toml: parser ? this.parsers.get(parser).toml : '' }), parser };
      }, 20)
    );
  }

  /** The request with its sliding window resolved against the newest record (mock approximation of the engine). */
  private windowed<R extends { query: Query }>(request: R): R {
    if (!request.query.window) return request;
    const wall = String((this.profile['live'] as { clock?: unknown } | undefined)?.clock ?? '').toLowerCase() === 'wall';
    return { ...request, query: mockResolveWindow(request.query, wall ? this.clock.now() : this.engine.newestTs()) };
  }

  setLive(enabled: boolean, options?: LiveOptions): Promise<number> {
    this.liveCalls.push({ enabled, options: options ?? null });
    return this.delay(() => {
      this.clock.clearInterval(this.liveTimer);
      this.liveTimer = undefined;
      this.liveOn = enabled;
      if (enabled) {
        this.liveOptions = options;
        this.liveTimer = this.clock.setInterval(() => this.liveTick(), this.liveSim.config.tickMs);
      } else {
        this.liveSim.idle();
      }
      return enabled ? this.dataset.sources.length : 0;
    }, 10);
  }

  onWorkspaceChanged(handler: (report: TailReport | null) => void): Promise<() => void> {
    this.changeHandlers.add(handler);
    return Promise.resolve(() => void this.changeHandlers.delete(handler));
  }

  onLiveAlert(handler: (alert: LiveAlert) => void): Promise<() => void> {
    this.alertHandlers.add(handler);
    return Promise.resolve(() => void this.alertHandlers.delete(handler));
  }

  tailStatus(): Promise<TailStatus> {
    return this.delay(() => {
      const counts = new Map<number, number>();
      for (const r of this.engine.records) counts.set(r.source, (counts.get(r.source) ?? 0) + 1);
      return this.liveSim.status(this.dataset.sources, (id) => counts.get(id) ?? 0, mockWatchMode(this.liveOn, this.liveOptions), this.evicted, this.engine.records.length);
    }, 15);
  }

  liveState(): Promise<LiveState> {
    return this.delay(() => ({ enabled: this.liveOn, options: mockResolveLiveOptions(this.liveOptions) }), 5);
  }

  muteAlerts(target: AlertTarget, untilMs: number): Promise<void> {
    return this.delay(() => this.mutes.mute(target, untilMs), 5);
  }

  unmuteAlerts(target: AlertTarget): Promise<void> {
    return this.delay(() => this.mutes.unmute(target), 5);
  }

  openPathsLive(paths: string[], startAtEnd: boolean): Promise<WorkspaceSummary> {
    if (!startAtEnd) return this.openPaths(paths);
    return this.delay(() => {
      for (const path of paths) {
        if (this.dataset.sources.some((s) => s.path === path)) continue;
        // Only lines written from now on: the file's history is skipped, so the source starts empty.
        const id = this.nextSourceId++;
        this.dataset.sources.push({ id, path, name: path.split(/[\\/]/).pop() || path, importer: 'text', skipped: 0 });
        this.liveSim.follow(id);
      }
      this.apply();
      return this.summary();
    }, 300);
  }

  /** One simulated watcher batch: new records (and maybe a rotation and an alert), then listeners are told, like the engine's file watcher does. */
  private liveTick(): void {
    const t0 = this.clock.elapsed();
    const report = emptyReport();
    let alerts: LiveAlert[] = [];
    if (this.options.syntheticTotal) {
      this.liveAdded++;
      report.appended = 1;
    } else {
      const last = this.lastBySource();
      const batch = this.liveSim.tick(this.dataset.sources, this.engine.newestTs(), (id) => last.get(id), this.clock.now());
      for (const r of batch.rotations) {
        const rotated = this.dataset.sources.find((s) => s.id === r.sourceId);
        if (rotated) {
          rotated.path = r.to;
          rotated.name = r.to.split(/[\\/]/).pop() || r.to;
        }
        this.dataset.sources.push(r.replacement);
        report.rotated.push({ sourceId: r.sourceId, from: r.from, to: r.to });
        report.added.push(r.replacement.id);
        report.fileStates.push({ sourceId: r.sourceId, state: { kind: 'rotated' } }, { sourceId: r.replacement.id, state: { kind: 'tailing' } });
      }
      if (batch.rotations.length) this.engine.setSources(this.dataset.sources);
      this.dataset.records.push(...batch.records);
      const appended = this.engine.append(batch.records, this.profile);
      this.liveAdded += batch.records.length;
      report.appended = appended.length;
      report.sourcesChanged = [...new Set([...batch.records.map((r) => r.source), ...report.added])];
      this.applyRetention(report);
      alerts = batch.alerts.filter((a) => !this.mutes.covers(a));
    }
    report.newestTs = this.engine.newestTs();
    report.alertsRaised = alerts.length;
    report.elapsedMs = Math.round((this.clock.elapsed() - t0) * 10) / 10;
    for (const handler of [...this.changeHandlers]) handler(report);
    for (const alert of alerts) {
      for (const handler of [...this.alertHandlers]) handler(alert);
      mockNotifications().post(alertNotice(alert));
    }
  }

  /** The newest raw record of each source (for the instance fields and line numbers of new lines). */
  private lastBySource(): Map<number, MockRecord> {
    const last = new Map<number, MockRecord>();
    const wanted = this.dataset.sources.length;
    for (let i = this.dataset.records.length - 1; i >= 0 && last.size < wanted; i--) {
      const r = this.dataset.records[i];
      if (r && !last.has(r.source)) last.set(r.source, r);
    }
    return last;
  }

  /** Simulated retention: the oldest records beyond `maxRecords` or older than `maxAgeMs` are evicted. */
  private applyRetention(report: TailReport): void {
    const retention = this.liveOptions?.retention;
    const newest = this.engine.newestTs();
    const maxAge = retention?.maxAgeMs ?? null;
    const maxRecords = retention?.maxRecords ?? null;
    if (maxAge === null && maxRecords === null) return;
    const before = this.dataset.records.length;
    let kept = maxAge !== null && newest !== null ? this.dataset.records.filter((r) => r.ts === null || r.ts >= newest - maxAge) : this.dataset.records;
    if (maxRecords !== null && kept.length > maxRecords) {
      kept = [...kept].sort((a, b) => (a.ts ?? 0) - (b.ts ?? 0)).slice(kept.length - maxRecords);
    }
    const evicted = before - kept.length;
    if (evicted <= 0) return;
    this.dataset.records = kept;
    this.apply();
    this.evicted += evicted;
    report.evicted = evicted;
    report.idsChangedFrom = 0;
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

  annotations(request: AnnotationsRequest): Promise<AnnotationDto[]> {
    return (
      this.failing('annotations') ??
      this.delay(() => {
        const groupBy = this.engine.defaultGroupBy;
        const runs = mockRunAnnotations(spanRules(this.profile), this.runContext())
          .flat()
          .filter((a) => (request.from == null || a.to >= request.from) && (request.to == null || a.from <= request.to) && (request.instance == null || a.instance === request.instance));
        return [...mockAnnotations(this.engine.records, (r) => this.engine.groupKey(r, groupBy), request), ...runs].sort((a, b) => a.from - b.from || a.kind.localeCompare(b.kind));
      }, 25)
    );
  }

  /** The run context over the current records (lanes by the default group-by). */
  private runContext(): RunContext {
    const groupBy = this.engine.defaultGroupBy;
    return { records: this.engine.records, laneOf: (r) => this.engine.groupKey(r, groupBy), groupBy };
  }

  /** `query_runs`; see mock-runs.ts. */
  queryRuns(request: RunsRequest): Promise<RunsReply> {
    return (
      this.failing('queryRuns') ??
      this.delay(() => {
        const rules = spanRules(this.profile);
        return mockQueryRuns(rules, mockRunAnnotations(rules, this.runContext()), request, this.engine.markerKinds);
      }, 25)
    );
  }

  /** `preview_spans`; see mock-runs.ts. */
  previewSpans(request: SpanPreviewRequest): Promise<SpanPreview> {
    return this.failing('previewSpans') ?? this.delay(() => mockPreviewSpans(request.draft, this.profile, this.runContext(), this.engine.markerKinds), 30);
  }

  /** Clock-skew correction (`set_source_offset`); see mock-skew.ts. */
  setSourceOffset(request: SetOffsetRequest): Promise<OffsetChange> {
    return this.delay(() => {
      const before = this.engine.records;
      const { source, offsetMs } = resolveOffset(request, before, this.dataset.sources);
      const keys = (request.track ?? []).map((id) => (before[id] ? recordKey(before[id]) : null));
      const previousMs = shiftSource(this.dataset.records, this.dataset.sources, source, offsetMs);
      this.apply();
      const after = new Map(this.engine.records.map((r) => [recordKey(r), r.id]));
      return {
        source,
        offsetMs,
        previousMs,
        // The log store keeps event ids stable when events move.
        idsChangedFrom: null,
        tracked: keys.map((k) => (k === null ? null : (after.get(k) ?? null))),
      };
    });
  }

  /** Offset suggestions from shared correlation ids (`suggest_offsets`). */
  suggestOffsets(request: SuggestRequest = {}): Promise<SuggestResult> {
    return this.delay(() => mockSuggest(this.engine.records, this.dataset.sources, request));
  }

  pluginEvents(): Promise<SubscriberMetrics[]> {
    return this.delay(() => MOCK_EVENT_METRICS.map((m) => ({ ...m, delivered: m.delivered + this.liveAdded })));
  }

  exportProfile(path: string, toml: string, overwrite = false): Promise<void> {
    return this.delay(() => {
      this.files.write(path, overwrite);
      const url = URL.createObjectURL(new Blob([toml], { type: 'application/toml' }));
      const a = document.createElement('a');
      a.href = url;
      a.download = path.split(/[\\/]/).pop() || 'profile.toml';
      a.click();
      URL.revokeObjectURL(url);
    });
  }
}

/** The simulation settings for the backend options (legacy: one event per tick). */
function liveConfig(options: MockBackendOptions): MockLiveConfig {
  if (!options.liveRate) return { ...LEGACY_LIVE, tickMs: options.liveMs ?? LEGACY_LIVE.tickMs };
  return {
    tickMs: options.liveMs ?? 250,
    ratePerSec: options.liveRate,
    rotateEveryMs: options.liveRotateMs ?? 30_000,
    alertEveryMs: options.liveAlertMs ?? 10_000,
  };
}

const PLUGIN_DEFAULTS = { failures: 0, lastError: null, requires: [], provides: [], inactiveReason: null, dependents: [], warnings: [], hung: false };
const MOCK_PLUGINS: PluginInfo[] = [
  { ...PLUGIN_DEFAULTS, id: 'net.decryptology.registry', name: 'Windows registry importer', version: '0.1.0', contributes: ['importer:registry'], status: 'active' },
  {
    ...PLUGIN_DEFAULTS,
    id: MOCK_WINDOWS_PLUGIN,
    name: 'Windows event logs',
    version: '0.2.0',
    contributes: ['importer:evtx', 'marker:app-crash', 'marker:service-stop', 'annotator:windows.reboots', 'annotator:windows.services', 'service:net.decryptology.windows.events'],
    status: 'active',
    provides: [{ id: 'net.decryptology.windows.events', version: '1.0.0' }],
    dependents: ['net.decryptology.web'],
  },
  {
    ...PLUGIN_DEFAULTS,
    id: 'net.decryptology.web',
    name: 'Web server logs',
    version: '0.1.0',
    contributes: ['importer:w3c'],
    status: 'active',
    requires: [
      { id: MOCK_WINDOWS_PLUGIN, req: '^0.2', optional: false },
      { id: 'net.decryptology.perfmon', req: '^1.0', optional: true },
    ],
    warnings: ['optional dependency net.decryptology.perfmon is not installed'],
  },
  {
    ...PLUGIN_DEFAULTS,
    id: 'net.decryptology.otel',
    name: 'OpenTelemetry',
    version: '0.1.0',
    contributes: [],
    status: 'inactive',
    requires: [{ id: 'net.decryptology.collector', req: '^1.0', optional: false }],
    inactiveReason: 'needs net.decryptology.collector ^1.0 (not installed)',
  },
];
/** Event-bus counters the demo reports (delivered grows with live tail). */
const MOCK_EVENT_METRICS: SubscriberMetrics[] = [
  { id: 1, plugin: MOCK_WINDOWS_PLUGIN, event: 'imt_core::events::RecordsAppended', delivered: 3, dropped: 0, failures: 0, skipped: 0, queued: 0 },
  { id: 2, plugin: 'net.decryptology.web', event: 'imt_core::events::ProfileChanged', delivered: 1, dropped: 0, failures: 1, skipped: 0, queued: 0 },
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
    return { validation: { ok: false, error: err.message.split('\n')[0] ?? '', line: err.line ?? null }, model: null };
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
  const live = validateLiveSection(model, lineOf);
  if (live) return { validation: live, model };
  try {
    mockRedactCfg(model);
  } catch (e) {
    return { validation: { ok: false, error: e instanceof Error ? e.message : String(e), line: lineOf('[redact]') }, model };
  }
  const dashboards = mockDashboardsError(mockDashboards(model));
  if (dashboards) return { validation: { ok: false, error: `[[dashboard]]: ${dashboards}`, line: lineOf('[[dashboard]]') }, model };
  for (const c of model.chart ?? []) {
    if (!c.id || !c.y) return { validation: { ok: false, error: '[[chart]] needs id and y', line: lineOf('[[chart]]') }, model };
    if (!/^(count|avg|min|max|sum|last)\(\s*[^)]*\)$/.test(c.y)) return { validation: { ok: false, error: `Chart '${c.id}': y must look like avg(field)`, line: lineOf(c.y) }, model };
  }
  return { validation: { ok: true, error: null, line: null }, model };
}

/** A live alert as the engine posts it to the notification centre: repeats merge per kind and instance (or rule). */
function alertNotice(alert: LiveAlert) {
  const names: Record<string, string> = { errorSpike: 'Error spike', silence: 'Silence', crash: 'Crash', slowOutlier: 'Slow outlier', rateChange: 'Rate change', memoryGrowth: 'Memory growth' };
  const what = alert.kind === 'rule' ? (alert.rule ?? 'Alert rule') : (names[alert.kind] ?? alert.kind);
  const who = alert.rule ?? alert.instance ?? '';
  const severity = alert.severity === 'critical' ? ('error' as const) : alert.severity === 'warning' ? ('warning' as const) : ('info' as const);
  return {
    source: 'liveAlert' as const,
    severity,
    title: alert.instance ? `${what} on ${alert.instance}` : what,
    body: alert.message,
    primary: { type: 'showAlert' as const, alertId: alert.id },
    dedupeKey: `live:${alert.kind}:${who}`,
  };
}
