import { invoke as tauriInvoke } from '../core/activity/ipc';
import { inject, Provider } from '@angular/core';
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
  HeapTypesPage,
  HeapTypesRequest,
  HotPath,
  HotPathRequest,
  HotspotsPage,
  HotspotsRequest,
  ProfileFinding,
  ProfileMoment,
  ProfileMomentRequest,
  ProfileSpan,
  SniffResult,
  ThreadTimeline,
  ThreadTimelineRequest,
  TreePage,
  TreeRequest,
} from '../core/api.types';
import { reportBackendFailures } from '../core/failure-reporter';
import { Logger, withFailureLogging } from '../core/logger';
import { isTauri } from '../core/platform';
import { MockProfilingBackend } from '../core/mock/mock-profiling';
import { toProfilingFailure } from './profiling-errors';

/**
 * The profiling engine as the UI sees it (commands in `app/src-tauri/src/profiling.rs`). Every
 * aggregation happens in the engine; the views only present what these return. Queries have no
 * side effects; only `open` and `close` change state. Every method rejects with a
 * `ProfilingFailure` (an `Error` carrying the engine's error code).
 */
export abstract class ProfilingBackend {
  /** Opens a profile or heap snapshot; the engine recognises the format by content. */
  abstract open(path: string): Promise<CaptureSummary>;
  /** What each path is (profile, heap snapshot or neither), without opening it: dropped files are routed with it. */
  abstract sniff(paths: string[]): Promise<SniffResult[]>;
  abstract list(): Promise<CaptureSummary[]>;
  abstract close(id: number): Promise<boolean>;
  abstract spans(): Promise<ProfileSpan[]>;
  abstract hotspots(request: HotspotsRequest): Promise<HotspotsPage>;
  abstract tree(request: TreeRequest): Promise<TreePage>;
  abstract hotPath(request: HotPathRequest): Promise<HotPath>;
  /** The hottest top-down paths to a function (reveal it in the call tree, zoom the flame graph to it). */
  abstract framePaths(request: FramePathsRequest): Promise<FramePaths>;
  abstract butterfly(request: ButterflyRequest): Promise<Butterfly>;
  abstract flame(request: FlameRequest): Promise<FlameGraph>;
  abstract diffFlame(request: DiffFlameRequest): Promise<FlameGraph>;
  abstract functionDiff(request: FunctionDiffRequest): Promise<FunctionDiffPage>;
  abstract threads(request: ThreadTimelineRequest): Promise<ThreadTimeline>;
  abstract findings(request: FindingsRequest): Promise<ProfileFinding[]>;
  abstract moment(request: ProfileMomentRequest): Promise<ProfileMoment[]>;
  abstract heapTypes(request: HeapTypesRequest): Promise<HeapTypesPage>;
  abstract heapDiff(request: HeapDiffRequest): Promise<HeapDiffPage>;
  abstract heapGrowth(request: HeapGrowthRequest): Promise<HeapGrowthPage>;
  abstract heapDominators(request: HeapDominatorsRequest): Promise<HeapObjectsPage>;
  abstract heapInstances(request: HeapInstancesRequest): Promise<HeapObjectsPage>;
  abstract heapPaths(request: HeapPathsRequest): Promise<HeapPaths>;
}

/** Talks to the Rust shell; `invoke` is Tauri's (tests pass a fake). Rejections become `ProfilingFailure`s. */
export class TauriProfilingBackend extends ProfilingBackend {
  constructor(private readonly invoke: typeof tauriInvoke = tauriInvoke) {
    super();
  }

  private async call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
    try {
      return await (args === undefined ? this.invoke<T>(cmd) : this.invoke<T>(cmd, args));
    } catch (e) {
      throw toProfilingFailure(e);
    }
  }

  open(path: string) {
    return this.call<CaptureSummary>('profiling_open', { path });
  }
  sniff(paths: string[]) {
    return this.call<SniffResult[]>('profiling_sniff', { paths });
  }
  list() {
    return this.call<CaptureSummary[]>('profiling_list');
  }
  close(id: number) {
    return this.call<boolean>('profiling_close', { id });
  }
  spans() {
    return this.call<ProfileSpan[]>('profiling_spans');
  }
  hotspots(request: HotspotsRequest) {
    return this.call<HotspotsPage>('profiling_hotspots', { request });
  }
  tree(request: TreeRequest) {
    return this.call<TreePage>('profiling_tree', { request });
  }
  hotPath(request: HotPathRequest) {
    return this.call<HotPath>('profiling_hot_path', { request });
  }
  framePaths(request: FramePathsRequest) {
    return this.call<FramePaths>('profiling_frame_paths', { request });
  }
  butterfly(request: ButterflyRequest) {
    return this.call<Butterfly>('profiling_butterfly', { request });
  }
  flame(request: FlameRequest) {
    return this.call<FlameGraph>('profiling_flame', { request });
  }
  diffFlame(request: DiffFlameRequest) {
    return this.call<FlameGraph>('profiling_diff_flame', { request });
  }
  functionDiff(request: FunctionDiffRequest) {
    return this.call<FunctionDiffPage>('profiling_function_diff', { request });
  }
  threads(request: ThreadTimelineRequest) {
    return this.call<ThreadTimeline>('profiling_threads', { request });
  }
  findings(request: FindingsRequest) {
    return this.call<ProfileFinding[]>('profiling_findings', { request });
  }
  moment(request: ProfileMomentRequest) {
    return this.call<ProfileMoment[]>('profiling_moment', { request });
  }
  heapTypes(request: HeapTypesRequest) {
    return this.call<HeapTypesPage>('profiling_heap_types', { request });
  }
  heapDiff(request: HeapDiffRequest) {
    return this.call<HeapDiffPage>('profiling_heap_diff', { request });
  }
  heapGrowth(request: HeapGrowthRequest) {
    return this.call<HeapGrowthPage>('profiling_heap_growth', { request });
  }
  heapDominators(request: HeapDominatorsRequest) {
    return this.call<HeapObjectsPage>('profiling_heap_dominators', { request });
  }
  heapInstances(request: HeapInstancesRequest) {
    return this.call<HeapObjectsPage>('profiling_heap_instances', { request });
  }
  heapPaths(request: HeapPathsRequest) {
    return this.call<HeapPaths>('profiling_heap_paths', { request });
  }
}

/** The real backend inside Tauri, the in-memory mock (URL switches as in `mock-profiling.ts`) in a browser. */
export function createProfilingBackend(): ProfilingBackend {
  const real = isTauri() ? new TauriProfilingBackend() : new MockProfilingBackend(MockProfilingBackend.optionsFromSearch(location.search));
  return withFailureLogging<ProfilingBackend>(real, inject(Logger), { onFailure: reportBackendFailures() });
}

export function provideProfilingBackend(): Provider[] {
  return [{ provide: ProfilingBackend, useFactory: createProfilingBackend }];
}
