import type { ProfilingBackend } from '../../profiling/profiling-backend';
import { ProfilingFailure } from '../../profiling/profiling-errors';
import {
  Butterfly,
  ButterflyRequest,
  CaptureSummary,
  DiffFlameRequest,
  FindingsRequest,
  FlameGraph,
  FlameRequest,
  FramePaths,
  FramePathsRequest,
  FunctionDiffPage,
  FunctionDiffRequest,
  HeapDiffPage,
  HeapDiffRequest,
  HeapDominatorsRequest,
  HeapGrowthPage,
  HeapGrowthRequest,
  HeapInstancesRequest,
  HeapObjectsPage,
  HeapPaths,
  HeapPathsRequest,
  HeapSummary,
  HeapTypesPage,
  HeapTypesRequest,
  HotPath,
  HotPathRequest,
  HotspotsPage,
  HotspotsRequest,
  MetricId,
  ProfileFinding,
  ProfileMoment,
  ProfileMomentRequest,
  ProfileSpan,
  ProfileSummary,
  SniffResult,
  ThreadTimeline,
  ThreadTimelineRequest,
  TreePage,
  TreeRequest,
} from '../api.types';
import { heapSnapshot, MockHeapData, MockProfileData, ordersProfile, regressionProfile } from './mock-profiling-data';
import { mockButterfly, mockFindings, mockFlame, mockFramePaths, mockFunctionDiff, mockHotPath, mockHotspots, mockThreads, mockTree, FlameParams, viewSamples, grandTotal } from './mock-profiling-engine';
import { HeapIndex, indexHeap, mockHeapDiff, mockHeapDominators, mockHeapGrowth, mockHeapInstances, mockHeapPaths, mockHeapTypes } from './mock-profiling-heap';

/** Operations `mockProfilingFail` may name. */
export const PROFILING_FAILABLE = ['open', 'sniff', 'hotspots', 'tree', 'hotPath', 'framePaths', 'butterfly', 'flame', 'diffFlame', 'functionDiff', 'threads', 'findings', 'moment', 'heapTypes', 'heapDiff', 'heapGrowth', 'heapDominators', 'heapInstances', 'heapPaths', 'spans'] as const;
export type ProfilingFailable = (typeof PROFILING_FAILABLE)[number];

export interface MockProfilingOptions {
  /** Delay of every answer in ms (default 0). */
  latencyMs?: number;
  /** The first N calls of an operation reject. */
  fail?: Partial<Record<ProfilingFailable, number>>;
  /** Paths opened before the first call (e.g. for component tests). */
  preopen?: string[];
}

type Capture = { kind: 'profile'; id: number; path: string; data: MockProfileData } | { kind: 'heap'; id: number; path: string; data: MockHeapData; index: HeapIndex };

const MAX_CAPTURES = 32;
const MAX_LIMIT = 10_000;
const PROFILE_EXT = /\.(speedscope\.json|cpuprofile|nettrace|folded|pb\.gz|trace\.json|perf\.txt)$/i;
const HEAP_NAME = /heap-?(\d)?.*\.heapsnapshot$/i;
/** Files named `missing…` do not exist (restoring a project whose captures were deleted). */
const MISSING = /^missing/i;

const fileName = (path: string) => path.split(/[\\/]/).pop() ?? path;

/**
 * In-memory profiling backend for the browser demo and UI tests: recognises captures by name
 * (the engine sniffs content), and answers every query from small synthetic profiles. Rejections
 * are `ProfilingFailure`s with the engine's codes; a file named `missing…` does not exist.
 */
export class MockProfilingBackend implements ProfilingBackend {
  private readonly captures = new Map<number, Capture>();
  private nextId = 1;
  private readonly failures: Partial<Record<ProfilingFailable, number>>;
  private readonly preopen: string[];

  constructor(private readonly options: MockProfilingOptions = {}) {
    this.failures = { ...options.fail };
    this.preopen = [...(options.preopen ?? [])];
  }

  /**
   * Browser-demo switches: `?mockProfilingFail=hotspots:1,flame:2` (the first N calls reject) and
   * `?mockProfilingLatency=300` (ms per answer).
   */
  static optionsFromSearch(search: string): MockProfilingOptions {
    const p = new URLSearchParams(search);
    const options: MockProfilingOptions = {};
    const latency = Number(p.get('mockProfilingLatency'));
    if (Number.isInteger(latency) && latency > 0 && latency < 60_000) options.latencyMs = latency;
    const fail: Partial<Record<ProfilingFailable, number>> = {};
    for (const part of (p.get('mockProfilingFail') ?? '').split(',')) {
      const [name, count] = part.split(':');
      if (name !== undefined && (PROFILING_FAILABLE as readonly string[]).includes(name)) fail[name as ProfilingFailable] = Math.max(1, Number(count) || 1);
    }
    if (Object.keys(fail).length) options.fail = fail;
    return options;
  }

  private async answer<T>(op: ProfilingFailable | null, fn: () => T): Promise<T> {
    if (this.preopen.length) for (const path of this.preopen.splice(0)) this.openNow(path);
    if (this.options.latencyMs) await new Promise((r) => setTimeout(r, this.options.latencyMs));
    if (op && (this.failures[op] ?? 0) > 0) {
      this.failures[op]!--;
      throw new ProfilingFailure('io', `mock: ${op} failed (simulated)`);
    }
    try {
      return fn();
    } catch (e) {
      // The engine stand-ins throw plain errors for invalid requests.
      throw e instanceof ProfilingFailure ? e : new ProfilingFailure('invalid', e instanceof Error ? e.message : String(e));
    }
  }

  private openNow(path: string): CaptureSummary {
    const name = fileName(path);
    if (MISSING.test(name)) throw new ProfilingFailure('notFound', `cannot open ${path}: No such file or directory (os error 2)`);
    if (this.captures.size >= MAX_CAPTURES) throw new ProfilingFailure('limit', `open captures limit exceeded (${MAX_CAPTURES}); close a capture first`);
    const id = this.nextId++;
    const heap = HEAP_NAME.exec(name);
    if (heap) {
      const data = heapSnapshot(Math.min(3, Math.max(1, Number(heap[1] ?? 1))));
      this.captures.set(id, { kind: 'heap', id, path, data: { ...data, name }, index: indexHeap(data) });
    } else if (PROFILE_EXT.test(name)) {
      const data = /regression/i.test(name) ? regressionProfile() : ordersProfile();
      this.captures.set(id, { kind: 'profile', id, path, data: { ...data, name } });
    } else {
      this.nextId--;
      throw new ProfilingFailure('notACapture', `${name} is not a profile or heap snapshot that IMT recognises (speedscope, folded stacks, .cpuprofile, Chrome trace, pprof, perf script, .nettrace, .heapsnapshot)`);
    }
    return this.summary(this.captures.get(id)!);
  }

  private summary(c: Capture): CaptureSummary {
    if (c.kind === 'heap') {
      const objects = c.data.objects.length - 1;
      const heap: HeapSummary = { capture: 'heap', id: c.id, name: c.data.name, path: c.path, format: 'heapsnapshot', takenMs: c.data.takenMs, typeCount: new Set(c.data.objects.slice(1).map((o) => o.type)).size, objectCount: objects, totalBytes: c.data.objects.reduce((a, o) => a + o.shallow, 0), hasGraph: true, notes: c.data.notes };
      return heap;
    }
    const d = c.data;
    const profile: ProfileSummary = {
      capture: 'profile',
      id: c.id,
      name: d.name,
      path: c.path,
      format: d.format,
      kind: d.kind,
      startedMs: d.startedMs,
      durationNs: d.durationNs,
      hasTime: true,
      sampleCount: d.samples.length,
      frameCount: d.frames.length,
      eventCount: d.events.length,
      metrics: d.metrics,
      defaultMetric: 'timeNs',
      threads: d.threads,
      eventKinds: [...new Set(d.events.map((e) => e.kind))],
      notes: d.notes,
    };
    return profile;
  }

  private profile(id: number): MockProfileData {
    const c = this.captures.get(id);
    if (!c) throw new ProfilingFailure('notFound', `capture ${id} is not open`);
    if (c.kind !== 'profile') throw new ProfilingFailure('invalid', `capture ${id} is a heap snapshot, not a profile`);
    return c.data;
  }

  private heap(id: number): HeapIndex {
    const c = this.captures.get(id);
    if (!c) throw new ProfilingFailure('notFound', `capture ${id} is not open`);
    if (c.kind !== 'heap') throw new ProfilingFailure('invalid', `capture ${id} is a profile, not a heap snapshot`);
    return c.index;
  }

  private metric(p: MockProfileData, m: MetricId | null | undefined): MetricId {
    if (m && !p.metrics.some((x) => x.id === m)) throw new ProfilingFailure('unsupported', `the profile has no ${m} metric`);
    return m ?? 'timeNs';
  }

  private static limit(n: number | undefined, fallback = 200): number {
    if (n !== undefined && (n < 1 || n > MAX_LIMIT)) throw new ProfilingFailure('invalid', `limit must be 1..${MAX_LIMIT}`);
    return n ?? fallback;
  }

  open(path: string) {
    return this.answer('open', () => this.openNow(path));
  }
  sniff(paths: string[]) {
    return this.answer('sniff', () =>
      paths.map((path): SniffResult => {
        const name = fileName(path);
        const kind = MISSING.test(name) ? 'none' : HEAP_NAME.test(name) ? 'heap' : PROFILE_EXT.test(name) ? 'capture' : 'none';
        return { path, kind };
      }),
    );
  }
  list() {
    return this.answer(null, () => [...this.captures.values()].map((c) => this.summary(c)));
  }
  close(id: number) {
    return this.answer(null, () => this.captures.delete(id));
  }
  spans() {
    return this.answer('spans', () =>
      [...this.captures.values()].map((c): ProfileSpan =>
        c.kind === 'heap'
          ? { id: c.id, name: c.data.name, capture: 'heap', kind: null, fromMs: c.data.takenMs, toMs: c.data.takenMs }
          : { id: c.id, name: c.data.name, capture: 'profile', kind: c.data.kind, fromMs: c.data.startedMs!, toMs: c.data.startedMs! + c.data.durationNs / 1e6 },
      ),
    );
  }
  hotspots(r: HotspotsRequest): Promise<HotspotsPage> {
    return this.answer('hotspots', () => {
      const p = this.profile(r.profileId);
      return mockHotspots(p, r.view, r.groupBy ?? 'function', this.metric(p, r.metric), r.sort, r.search, r.offset ?? 0, MockProfilingBackend.limit(r.limit));
    });
  }
  tree(r: TreeRequest): Promise<TreePage> {
    return this.answer('tree', () => {
      const p = this.profile(r.profileId);
      return mockTree(p, r.view, r.direction ?? 'topDown', r.path ?? [], this.metric(p, r.metric), r.offset ?? 0, MockProfilingBackend.limit(r.limit));
    });
  }
  hotPath(r: HotPathRequest): Promise<HotPath> {
    return this.answer('hotPath', () => {
      const p = this.profile(r.profileId);
      return mockHotPath(p, r.view, r.direction ?? 'topDown', r.path ?? [], this.metric(p, r.metric), r.minFraction ?? 0.01);
    });
  }
  framePaths(r: FramePathsRequest): Promise<FramePaths> {
    return this.answer('framePaths', () => {
      const p = this.profile(r.profileId);
      const limit = r.limit ?? 5;
      if (limit < 1 || limit > 50) throw new ProfilingFailure('invalid', 'limit must be between 1 and 50');
      return mockFramePaths(p, r.view, r.frame, this.metric(p, r.metric), limit);
    });
  }
  butterfly(r: ButterflyRequest): Promise<Butterfly> {
    return this.answer('butterfly', () => {
      const p = this.profile(r.profileId);
      return mockButterfly(p, r.view, r.frame, this.metric(p, r.metric), MockProfilingBackend.limit(r.limit, 50));
    });
  }
  private flameParams(p: MockProfileData, r: FlameRequest | DiffFlameRequest, direction: FlameParams['direction']): FlameParams {
    if (!(r.widthPx >= 1 && r.widthPx <= 100_000)) throw new ProfilingFailure('invalid', 'widthPx must be 1..100000');
    const viewport = r.viewport ?? { start: 0, end: 1 };
    if (!(viewport.start >= 0 && viewport.start < viewport.end && viewport.end <= 1)) throw new ProfilingFailure('invalid', 'viewport must satisfy 0 ≤ start < end ≤ 1');
    return {
      direction,
      focus: r.focus ?? [],
      viewport,
      widthPx: r.widthPx,
      minWidthPx: r.minWidthPx ?? 0.5,
      maxDepth: r.maxDepth ?? Number.MAX_SAFE_INTEGER,
      metric: this.metric(p, r.metric),
      order: r.order ?? 'name',
      search: r.search ?? null,
      searchRegex: r.searchRegex ?? false,
      maxRects: Math.min(200_000, r.maxRects ?? 50_000),
    };
  }
  flame(r: FlameRequest): Promise<FlameGraph> {
    return this.answer('flame', () => {
      const p = this.profile(r.profileId);
      return mockFlame(p, r.view, this.flameParams(p, r, r.direction ?? 'topDown'));
    });
  }
  diffFlame(r: DiffFlameRequest): Promise<FlameGraph> {
    return this.answer('diffFlame', () => {
      const target = this.profile(r.targetId);
      const base = this.profile(r.baselineId);
      const params = this.flameParams(target, r, 'topDown');
      if (!base.metrics.some((m) => m.id === params.metric)) throw new ProfilingFailure('unsupported', `the baseline has no ${params.metric} metric`);
      return mockFlame(target, r.view, params, base);
    });
  }
  functionDiff(r: FunctionDiffRequest): Promise<FunctionDiffPage> {
    return this.answer('functionDiff', () => {
      const target = this.profile(r.targetId);
      const base = this.profile(r.baselineId);
      return mockFunctionDiff(base, target, r.view, r.groupBy ?? 'function', this.metric(target, r.metric), r.sort ?? 'absDeltaSelf', r.search, r.offset ?? 0, MockProfilingBackend.limit(r.limit));
    });
  }
  threads(r: ThreadTimelineRequest): Promise<ThreadTimeline> {
    return this.answer('threads', () => {
      const p = this.profile(r.profileId);
      const buckets = r.buckets ?? 400;
      if (buckets < 1 || buckets > 4000) throw new ProfilingFailure('invalid', 'buckets must be 1..4000');
      return mockThreads(p, r.view, buckets, this.metric(p, r.metric ?? 'samples'), r.eventKinds ?? null, r.maxEvents ?? 2000, p.startedMs);
    });
  }
  findings(r: FindingsRequest): Promise<ProfileFinding[]> {
    return this.answer('findings', () => {
      const p = this.profile(r.profileId);
      const baseline = r.baselineId ? { id: r.baselineId, data: this.profile(r.baselineId) } : null;
      return mockFindings(r.profileId, p, r.view, baseline);
    });
  }
  moment(r: ProfileMomentRequest): Promise<ProfileMoment[]> {
    return this.answer('moment', () => {
      const out: ProfileMoment[] = [];
      for (const c of this.captures.values()) {
        if (c.kind !== 'profile' || c.data.startedMs === null) continue;
        const p = c.data;
        const from = Math.max(0, (r.fromMs - p.startedMs!) * 1e6);
        const to = Math.min(p.durationNs, (r.toMs - p.startedMs!) * 1e6);
        if (to < from) continue;
        const view = { range: { fromNs: Math.floor(from), toNs: Math.ceil(to) } };
        const samples = viewSamples(p, view);
        if (!samples.length) continue;
        const page = mockHotspots(p, view, 'function', 'timeNs', null, null, 0, Math.min(50, r.top ?? 5));
        out.push({ profileId: c.id, name: p.name, range: view.range, samples: samples.length, metric: 'timeNs', measurement: page.measurement, value: grandTotal(p, samples).timeNs ?? 0, top: page.rows });
      }
      return out;
    });
  }
  heapTypes(r: HeapTypesRequest): Promise<HeapTypesPage> {
    return this.answer('heapTypes', () => mockHeapTypes(this.heap(r.snapshotId), r.sort ?? 'bytes', r.search, r.offset ?? 0, MockProfilingBackend.limit(r.limit)));
  }
  heapDiff(r: HeapDiffRequest): Promise<HeapDiffPage> {
    return this.answer('heapDiff', () => mockHeapDiff(this.heap(r.baselineId), this.heap(r.targetId), r.sort ?? 'bytesDelta', r.search, r.offset ?? 0, MockProfilingBackend.limit(r.limit)));
  }
  heapGrowth(r: HeapGrowthRequest): Promise<HeapGrowthPage> {
    return this.answer('heapGrowth', () => mockHeapGrowth(r.snapshotIds.map((id) => this.heap(id)), r.search, r.steadyOnly ?? false, r.offset ?? 0, MockProfilingBackend.limit(r.limit)));
  }
  heapDominators(r: HeapDominatorsRequest): Promise<HeapObjectsPage> {
    return this.answer('heapDominators', () => mockHeapDominators(this.heap(r.snapshotId), r.parent ?? null, r.offset ?? 0, MockProfilingBackend.limit(r.limit)));
  }
  heapInstances(r: HeapInstancesRequest): Promise<HeapObjectsPage> {
    return this.answer('heapInstances', () => mockHeapInstances(this.heap(r.snapshotId), r.typeName, r.offset ?? 0, MockProfilingBackend.limit(r.limit)));
  }
  heapPaths(r: HeapPathsRequest): Promise<HeapPaths> {
    return this.answer('heapPaths', () => mockHeapPaths(this.heap(r.snapshotId), r.object, Math.min(20, r.maxPaths ?? 5)));
  }
}
