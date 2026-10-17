import { parse, TomlError } from 'smol-toml';
import {
  ChartData,
  ChartRequest,
  ExtractPreview,
  ExtractPreviewRequest,
  Facet,
  FacetRequest,
  LEVELS,
  Level,
  Page,
  PageRequest,
  ProfileDoc,
  ProfileValidation,
  Timeline,
  TimelineRequest,
  WorkspaceSummary,
} from '../api.types';
import type { ImtBackend } from '../backend';
import { ProfileModel } from '../profile.model';
import { generateDemo, generateExtraSource, MockDataset } from './mock-data';
import { MockEngine, safeRegex } from './mock-engine';
import { MOCK_PROFILE_TOML } from './mock-profile';

export interface MockBackendOptions {
  /** Simulate IPC latency so loading states are visible (default true). Tests turn it off. */
  latency?: boolean;
  /** Dataset factory (default: the generated demo incident). */
  dataset?: () => MockDataset;
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

  constructor(options: MockBackendOptions = {}) {
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
    return {
      sources: e.sources.map((s) => ({
        id: s.id,
        path: s.path,
        name: s.name,
        importer: s.importer,
        records: perSource.get(s.id)?.n ?? 0,
        skipped: s.skipped,
        error: null,
        from: perSource.get(s.id)?.from ?? null,
        to: perSource.get(s.id)?.to ?? null,
      })),
      unrecognized: this.dataset.unrecognized,
      total: e.records.length,
      from,
      to,
      levels,
      fields: e.fields(),
      defaultGroupBy: e.defaultGroupBy,
      markerKinds: e.markerKinds,
      charts: e.charts,
      profileName: this.profile.name ?? 'Unnamed profile',
      loadMs: this.loadMs,
    };
  }

  workspaceSummary() {
    return this.delay(() => this.summary());
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
    return this.delay(() => this.engine.page(request), 20);
  }

  queryTimeline(request: TimelineRequest): Promise<Timeline> {
    return this.delay(() => this.engine.timeline(request), 30);
  }

  queryFacet(request: FacetRequest): Promise<Facet> {
    return this.delay(() => this.engine.facet(request), 20);
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
  }
  for (const c of model.chart ?? []) {
    if (!c.id || !c.y) return { validation: { ok: false, error: '[[chart]] needs id and y', line: lineOf('[[chart]]') }, model };
    if (!/^(count|avg|min|max|sum|last)\(\s*[^)]*\)$/.test(c.y)) return { validation: { ok: false, error: `Chart '${c.id}': y must look like avg(field)`, line: lineOf(c.y) }, model };
  }
  return { validation: { ok: true, error: null, line: null }, model };
}
