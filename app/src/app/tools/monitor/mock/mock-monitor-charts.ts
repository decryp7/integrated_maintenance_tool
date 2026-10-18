import { ChartData, ChartRequest, ChartSeries } from '../../../core/api.types';
import { MONITOR_METRICS } from '../monitor-charts-model';

/** How the mock behaves: `mock-monitor-charts=empty` (no samples yet) or `fail` (the first panel load fails). */
export type MockMonitorChartsMode = 'data' | 'empty' | 'fail';

export interface MockMonitorChartsOptions {
  mode: MockMonitorChartsMode;
  /** Wall clock (ms); tests pin it. */
  now: () => number;
  /** Sampling interval of the simulated monitor (ms). */
  intervalMs: number;
  /** The run asked for .NET runtime counters (the monitor config's `dotnet`). */
  dotnet: () => boolean;
}

export function mockMonitorChartsOptions(search: string): MockMonitorChartsOptions {
  const mode = new URLSearchParams(search).get('mock-monitor-charts');
  return { mode: mode === 'empty' || mode === 'fail' ? mode : 'data', now: () => Date.now(), intervalMs: 2000, dotnet: () => false };
}

/** Simulated tracked processes of the first run, as `ProcessName · ProcessId` keys. */
export const MOCK_PROCESSES = [
  { key: 'OrderService.exe · 4120', name: 'OrderService.exe', pid: 4120, scale: 1.4, phase: 0, dotnet: true },
  { key: 'Worker.exe · 5232', name: 'Worker.exe', pid: 5232, scale: 0.8, phase: 2.1, dotnet: true },
  { key: 'sqlservr.exe · 1880', name: 'sqlservr.exe', pid: 1880, scale: 2.2, phase: 4.2, dotnet: false },
] as const;

/** Process ids of a later run's output source differ by this per run, so runs are told apart in the legend. */
export const MOCK_RUN_PID_STEP = 1000;

/** A smooth, deterministic value of a sample field at `ts`, so follow-up refreshes continue the same lines. */
function sample(field: string, ts: number, scale: number, phase: number): number | null {
  const w = Math.sin(ts / 47_000 + phase);
  const slow = Math.sin(ts / 310_000 + phase);
  switch (field) {
    case 'CpuPercent':
      return Math.round((12 * scale + 9 * w + 4 * slow + 9) * 10) / 10;
    case 'WorkingSetMb':
      return Math.round((180 * scale + 25 * slow + 6 * w) * 10) / 10;
    case 'PrivateMb':
      return Math.round((150 * scale + 20 * slow + 5 * w) * 10) / 10;
    case 'Handles':
      return Math.round(320 * scale + 18 * slow);
    case 'Threads':
      return Math.round(18 * scale + 3 * w);
    case 'ReadBytesPerSec':
      return Math.max(0, Math.round(40_000 * scale * (1 + w)));
    case 'WriteBytesPerSec':
      return Math.max(0, Math.round(12_000 * scale * (1 + slow)));
    case 'DotnetHeapBytes':
      return Math.round((90 * scale + 30 * slow + 12 * w) * 1024 * 1024);
    case 'DotnetTimeInGcPercent':
      return Math.round(Math.max(0, 3 * scale + 2.5 * w) * 10) / 10;
    case 'DotnetExceptionsPerSec':
      return Math.round(Math.max(0, 0.6 * scale * (1 + w)) * 100) / 100;
    case 'DotnetContentionPerSec':
      return Math.round(Math.max(0, 4 * scale * (1 + slow)) * 10) / 10;
    default:
      return null;
  }
}

/**
 * Answers chart requests like the engine does over a monitor output: `avg(field)` per bucket, one series per process
 * (`splitBy` ProcessName, ProcessId) of the requested sources (`query.sources`, one run each), over the request's
 * live window. The `Dotnet*` fields exist only for the .NET
 * processes, and only when the run asked for runtime counters. For browser development and UI tests only.
 */
export class MockMonitorCharts {
  private failuresLeft: number;
  /** Source ids in the order first asked for: the n-th source is the n-th run (its processes get other ids). */
  private readonly runs: number[] = [];
  /** Requests received, for assertions. */
  readonly requests: ChartRequest[] = [];

  constructor(readonly options: MockMonitorChartsOptions) {
    this.failuresLeft = options.mode === 'fail' ? MONITOR_METRICS.length : 0;
  }

  async queryChart(request: ChartRequest): Promise<ChartData> {
    this.requests.push(request);
    if (this.failuresLeft > 0) {
      this.failuresLeft--;
      throw new Error('chart query failed: the monitor output is locked by another process');
    }
    const def = request.def;
    if (!def) throw new Error('chart request needs chartId or def');
    const field = /^avg\((\w+)\)$/.exec(def.y)?.[1];
    if (!field) throw new Error(`unsupported aggregate "${def.y}"`);

    const step = this.options.intervalMs;
    const to = Math.floor(this.options.now() / step) * step;
    const lastMs = request.query.window?.lastMs ?? 5 * 60_000;
    const from = to - lastMs;
    const bucketMs = Math.max(step, Math.ceil(lastMs / Math.max(10, request.buckets) / step) * step);
    const start = from - (from % bucketMs);

    const series: ChartSeries[] =
      this.options.mode === 'empty'
        ? []
        : this.runsOf(request.query.sources ?? []).flatMap((run) =>
            MOCK_PROCESSES.filter((p) => !field.startsWith('Dotnet') || (p.dotnet && this.options.dotnet())).flatMap((p) => {
              const points: [number, number | null][] = [];
              for (let t = start; t <= to; t += bucketMs) points.push([t, sample(field, t, p.scale, p.phase + run)]);
              return points.some(([, v]) => v !== null) ? [{ key: `${p.name} · ${p.pid + run * MOCK_RUN_PID_STEP}`, points }] : [];
            }),
          );
    return { def, from, to, bucketMs, series, lifecycle: [] };
  }

  /** Like the engine's `Query.sources`: only the asked runs' samples (none asked = the first run's). */
  private runsOf(sources: readonly number[]): number[] {
    if (!sources.length) return [0];
    return [...new Set(sources)].map((id) => {
      if (!this.runs.includes(id)) this.runs.push(id);
      return this.runs.indexOf(id);
    });
  }
}
