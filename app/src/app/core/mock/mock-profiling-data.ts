import { FrameDto, FrameKind, MetricDto, MetricValues, ProfileEventDto, ProfileThread } from '../api.types';
import { DEMO_START } from './mock-data';

/**
 * A small synthetic "OrderService" profile set for the browser demo and UI tests, modelled on the
 * engine's own samples (`samples/profiling`): a baseline CPU profile, a regression in which
 * serialisation grows and `AuditLog.Write` appears, and three heap snapshots in which `Order`
 * grows steadily and `AuditLog` first appears in the second one. Deterministic.
 */

const MIN = 60_000;
const MS_NS = 1_000_000;

/** One weighted stack observation (stack is root first). */
export interface MockSample {
  stack: number[];
  thread: number;
  t: number;
  v: MetricValues;
}

export interface MockProfileData {
  name: string;
  format: string;
  kind: 'cpu' | 'mixed';
  startedMs: number | null;
  durationNs: number;
  frames: FrameDto[];
  threads: ProfileThread[];
  metrics: MetricDto[];
  samples: MockSample[];
  events: ProfileEventDto[];
  notes: string[];
}

function frame(id: number, name: string, module: string | null, kind: FrameKind = 'managed', isFramework = false): FrameDto {
  return { id, name, module, file: kind === 'managed' && !isFramework ? `src/${name.split('.').slice(-2, -1)[0] ?? 'Program'}.cs` : null, line: kind === 'managed' && !isFramework ? 10 + id * 7 : null, kind, isFramework };
}

/** Frames shared by both profiles (ids are the profile's frame ids). */
export const MOCK_FRAMES: FrameDto[] = [
  frame(0, 'Shop.Orders.Program.Main', 'Shop.Orders'),
  frame(1, 'System.Threading.ThreadPoolWorkQueue.Dispatch', 'System.Private.CoreLib', 'managed', true),
  frame(2, 'Shop.Orders.Api.OrderController.Post', 'Shop.Orders'),
  frame(3, 'Shop.Orders.OrderService.Place', 'Shop.Orders'),
  frame(4, 'Shop.Orders.Pricing.PriceCalculator.Calculate', 'Shop.Orders'),
  frame(5, 'Shop.Orders.Serialization.JsonWriter.Write', 'Shop.Orders'),
  frame(6, 'System.Text.Json.JsonSerializer.Serialize', 'System.Text.Json', 'managed', true),
  frame(7, 'System.Text.Json.Utf8JsonWriter.WriteString', 'System.Text.Json', 'managed', true),
  frame(8, 'Shop.Orders.Data.OrderRepository.Save', 'Shop.Orders'),
  frame(9, 'Microsoft.Data.SqlClient.SqlCommand.ExecuteReader', 'Microsoft.Data.SqlClient', 'managed', true),
  frame(10, 'System.Threading.Monitor.Enter', 'System.Private.CoreLib', 'managed', true),
  frame(11, 'ntdll!NtWaitForSingleObject', 'ntdll.dll', 'native', true),
  frame(12, 'ntoskrnl!KiSwapContext', 'ntoskrnl.exe', 'kernel', true),
  frame(13, 'coreclr!gc_heap::garbage_collect', 'coreclr.dll', 'native', true),
  frame(14, 'Shop.Orders.Validation.OrderValidator.Validate', 'Shop.Orders'),
  frame(15, 'Shop.Orders.Validation.OrderValidator.ThrowInvalid', 'Shop.Orders'),
  frame(16, '[unknown]', null, 'unknown'),
  frame(17, 'Shop.Orders.Program.Startup', 'Shop.Orders'),
  frame(18, 'Microsoft.Extensions.Configuration.ConfigurationBuilder.Build', 'Microsoft.Extensions.Configuration', 'managed', true),
  frame(19, 'Shop.Orders.AuditLog.Write', 'Shop.Orders'),
];

const REQ = [0, 1, 2, 3];

/** Stack templates (root first), weight per 100 samples, thread pool, and per-sample extras. */
interface Template {
  stack: number[];
  weight: number;
  regressionWeight: number;
  threads: number[];
  extra?: MetricValues;
}

const TEMPLATES: Template[] = [
  { stack: [...REQ, 4, 4, 4], weight: 22, regressionWeight: 14, threads: [1, 2], extra: { allocCount: 2, allocBytes: 256 } },
  { stack: [...REQ, 5, 6, 7], weight: 12, regressionWeight: 32, threads: [1, 2], extra: { allocCount: 6, allocBytes: 4096 } },
  { stack: [...REQ, 8, 9, 10, 11], weight: 14, regressionWeight: 10, threads: [1, 2], extra: { waitNs: 2 * MS_NS } },
  { stack: [...REQ, 8, 9, 12], weight: 7, regressionWeight: 6, threads: [2] },
  { stack: [0, 13], weight: 10, regressionWeight: 8, threads: [3] },
  { stack: [...REQ, 14, 15], weight: 5, regressionWeight: 4, threads: [1], extra: { exceptions: 1, allocCount: 1, allocBytes: 512 } },
  { stack: [0, 16], weight: 3, regressionWeight: 2, threads: [0] },
  { stack: [0, 17, 18], weight: 4, regressionWeight: 3, threads: [0] },
  { stack: [...REQ, 3], weight: 23, regressionWeight: 13, threads: [1, 2] },
  { stack: [...REQ, 19], weight: 0, regressionWeight: 8, threads: [1, 2], extra: { allocCount: 3, allocBytes: 1024 } },
];

const THREADS: ProfileThread[] = [
  { id: 0, osId: 4012, name: 'Main thread' },
  { id: 1, osId: 4120, name: '.NET ThreadPool Worker #1' },
  { id: 2, osId: 4121, name: '.NET ThreadPool Worker #2' },
  { id: 3, osId: 4130, name: '.NET BGC' },
];

const metric = (id: MetricDto['id'], measurement: MetricDto['measurement'], unit: MetricDto['unit'], label: string): MetricDto => ({ id, measurement, unit, label });

const FULL_METRICS: MetricDto[] = [
  metric('samples', 'sampled', 'count', 'Samples'),
  metric('timeNs', 'sampled', 'nanoseconds', 'CPU time'),
  metric('calls', 'exact', 'count', 'Calls'),
  metric('maxCallNs', 'exact', 'nanoseconds', 'Max per call'),
  metric('allocCount', 'sampled', 'count', 'Allocations'),
  metric('allocBytes', 'sampled', 'bytes', 'Allocated bytes'),
  metric('exceptions', 'exact', 'count', 'Exceptions'),
  metric('waitNs', 'exact', 'nanoseconds', 'Wait time'),
];

const CPU_METRICS: MetricDto[] = FULL_METRICS.slice(0, 2);

/** Spreads `count` samples over the templates by weight, deterministically, across `durationNs`. */
function samplesFor(count: number, durationNs: number, regression: boolean, full: boolean): MockSample[] {
  const weighted = TEMPLATES.flatMap((t) => Array<Template>(regression ? t.regressionWeight : t.weight).fill(t));
  const out: MockSample[] = [];
  for (let i = 0; i < count; i++) {
    // Stride 37 interleaves the templates over time instead of running them in blocks.
    const t = weighted[(i * 37) % weighted.length];
    if (!t) break;
    const thread = t.threads[i % t.threads.length] ?? 0;
    const time = Math.floor(((i + 0.5) * durationNs) / count);
    // Request stacks slow down in the middle second (the "incident"), which the timeline shows.
    const ns = (thread === 1 || thread === 2) && time > durationNs * 0.45 && time < durationNs * 0.6 ? 2 * MS_NS : MS_NS;
    const v: MetricValues = full ? { samples: 1, timeNs: ns, calls: 1, maxCallNs: ns, ...t.extra } : { samples: 1, timeNs: ns };
    out.push({ stack: t.stack, thread, t: time, v });
  }
  return out;
}

function eventsFor(samples: MockSample[], durationNs: number): ProfileEventDto[] {
  const events: ProfileEventDto[] = [];
  let gc = 0;
  for (const s of samples) {
    const leaf = s.stack[s.stack.length - 1];
    if (leaf === 13 && gc++ % 4 === 0) events.push({ kind: 'gc', thread: 3, startNs: s.t, durNs: 3 * MS_NS, label: `GC #${gc} (gen ${gc % 3})` });
    else if (leaf === 15) events.push({ kind: 'exception', thread: s.thread, startNs: s.t, durNs: 0, label: 'System.ArgumentException: Quantity must be positive' });
    else if (leaf === 11 && s.t > durationNs * 0.45 && s.t < durationNs * 0.6) events.push({ kind: 'lockWait', thread: s.thread, startNs: s.t, durNs: 4 * MS_NS, label: 'Monitor contention on OrderRepository._sync' });
  }
  events.push({ kind: 'jit', thread: 0, startNs: 2 * MS_NS, durNs: 5 * MS_NS, label: 'JIT Shop.Orders.OrderService.Place' });
  events.push({ kind: 'jit', thread: 1, startNs: 9 * MS_NS, durNs: 2 * MS_NS, label: 'JIT Shop.Orders.Pricing.PriceCalculator.Calculate' });
  events.push({ kind: 'marker', thread: null, startNs: Math.floor(durationNs / 2), durNs: 0, label: 'Load test step 2' });
  return events.sort((a, b) => a.startNs - b.startNs);
}

/** The demo log incident is at 10:41 (DEMO_START + 101 min); the baseline capture covers it. */
export const ORDERS_STARTED_MS = DEMO_START + 100 * MIN + 50_000;
export const REGRESSION_STARTED_MS = DEMO_START + 150 * MIN;

export function ordersProfile(): MockProfileData {
  const durationNs = 20_000 * MS_NS;
  const samples = samplesFor(1200, durationNs, false, true);
  return {
    name: 'orders.speedscope.json',
    format: 'speedscope',
    kind: 'mixed',
    startedMs: ORDERS_STARTED_MS,
    durationNs,
    frames: MOCK_FRAMES.filter((f) => f.id !== 19),
    threads: THREADS,
    metrics: FULL_METRICS,
    samples,
    events: eventsFor(samples, durationNs),
    notes: ['12 addresses could not be resolved and are shown as [unknown].'],
  };
}

export function regressionProfile(): MockProfileData {
  const durationNs = 15_000 * MS_NS;
  return {
    name: 'orders-regression.speedscope.json',
    format: 'speedscope',
    kind: 'cpu',
    startedMs: REGRESSION_STARTED_MS,
    durationNs,
    frames: MOCK_FRAMES,
    threads: THREADS,
    metrics: CPU_METRICS,
    samples: samplesFor(900, durationNs, true, false),
    events: [],
    notes: ['Stack depth limit: 3 stacks deeper than 4096 frames keep their outer frames.', 'Events limit reached: 120 events beyond 2,000,000 were dropped.'],
  };
}

// ---------------------------------------------------------------------------------------------
// Heap snapshots: a tree-shaped object graph (one referrer per object) plus shared customers.
// ---------------------------------------------------------------------------------------------

export interface MockObject {
  id: number;
  type: string;
  shallow: number;
  /** Referrers with the reference label (first = dominator in this simple graph). */
  from: { id: number; edge: string }[];
}

export interface MockHeapData {
  name: string;
  takenMs: number;
  objects: MockObject[];
  notes: string[];
}

export const HEAP_TAKEN_MS = [DEMO_START + 60 * MIN, DEMO_START + 90 * MIN, DEMO_START + 120 * MIN];

/** Snapshot `n` (1..3). */
export function heapSnapshot(n: number): MockHeapData {
  const objects: MockObject[] = [];
  const add = (type: string, shallow: number, from: { id: number; edge: string }[]): number => {
    const id = objects.length;
    objects.push({ id, type, shallow, from });
    return id;
  };
  const root = add('(GC roots)', 0, []);
  const cache = add('Shop.Orders.OrderCache', 48, [{ id: root, edge: 'static OrderCache.Instance' }]);
  const dict = add('System.Collections.Generic.Dictionary<Int32, Order>', 96, [{ id: cache, edge: '_orders' }]);
  const sessions = add('Shop.Orders.SessionStore', 40, [{ id: root, edge: 'static SessionStore.Current' }]);
  const customers: number[] = [];
  for (let i = 0; i < 30; i++) customers.push(add('Shop.Orders.Customer', 72, [{ id: sessions, edge: `[${i}]` }]));
  for (let i = 0; i < 50 * n; i++) {
    const order = add('Shop.Orders.Order', 120, [{ id: dict, edge: `[${i}]` }, { id: customers[i % customers.length] ?? root, edge: 'LastOrder' }]);
    for (let l = 0; l < 2; l++) add('Shop.Orders.OrderLine', 64, [{ id: order, edge: `Lines[${l}]` }]);
    add('System.String', 40 + (i % 5) * 8, [{ id: order, edge: 'Reference' }]);
  }
  const strings = n === 2 ? 900 : n === 3 ? 1020 : 1000;
  const pool = add('System.String[]', 8 * strings, [{ id: root, edge: 'static StringPool.Items' }]);
  for (let i = 0; i < strings - 50 * n; i++) add('System.String', 32, [{ id: pool, edge: `[${i}]` }]);
  if (n >= 2) {
    const audit = add('Shop.Orders.AuditLog', 56, [{ id: root, edge: 'static AuditLog.Default' }]);
    for (let i = 0; i < 10 * (n - 1); i++) add('Shop.Orders.AuditEntry', 88, [{ id: audit, edge: `_entries[${i}]` }]);
  }
  for (let i = 0; i < (n === 2 ? 3 : 5); i++) add('System.Byte[]', 4096, [{ id: root, edge: `handle #${i}` }]);
  return { name: `heap-${n}.heapsnapshot`, takenMs: HEAP_TAKEN_MS[n - 1] ?? 0, objects, notes: n === 3 ? ['Weak and shortcut edges were skipped.'] : [] };
}

/** Paths the browser demo's "Open performance capture…" dialog returns. */
export const MOCK_CAPTURE_PATHS = [
  '/demo/profiles/orders.speedscope.json',
  '/demo/profiles/orders-regression.speedscope.json',
  '/demo/profiles/heap-1.heapsnapshot',
  '/demo/profiles/heap-2.heapsnapshot',
  '/demo/profiles/heap-3.heapsnapshot',
];
