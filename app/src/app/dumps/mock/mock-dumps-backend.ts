import {
  DocKind,
  DumpDocSummary,
  DumpFinding,
  DumpFindingsRequest,
  DumpMarker,
  DumpOverview,
  DumpSniffResult,
  FrameRow,
  ModuleRow,
  ModulesPage,
  ModulesRequest,
  StackDto,
  StackRequest,
  ThreadRow,
  ThreadsPage,
  ThreadsRequest,
  UmdhDiffPage,
  UmdhDiffRequest,
  UmdhRow,
  UmdhStackDto,
  UmdhStackRequest,
  UmdhTreeNode,
  UmdhTreeRequest,
} from '../../core/api.types';
import { CmdError } from '../../core/cmd-error';
import { DumpsBackend } from '../dumps-backend';

/** The demo files the browser build opens (File ▸ Open dump… and Open UMDH snapshots…). */
export const MOCK_DUMP_PATHS = ['/samples/windows/WEB02/OrderService.exe.4120.dmp'];
export const MOCK_UMDH_PATHS = [
  '/samples/dumps/umdh/OrderService-4120-snap-1030.txt',
  '/samples/dumps/umdh/OrderService-4120-snap-1035.txt',
  '/samples/dumps/umdh/OrderService-4120-snap-1040.txt',
];

const CRASH_MS = Date.UTC(2026, 9, 14, 10, 41, 24);
const fail = (code: 'notFound' | 'unsupported' | 'invalid', message: string) => new CmdError({ code, message, retryable: false });
const name = (path: string) => path.split(/[\\/]/).pop() ?? path;

/** A frame row of the mock dump. */
function frame(index: number, module: string, fn: string | null, offset: number, framework: boolean, culprit = false): FrameRow {
  const label = fn ? `${module}!${fn}+0x${offset.toString(16)}` : `${module}+0x${offset.toString(16)}`;
  return { index, address: `0x7ffa${(0x1000 + offset).toString(16)}`, module, function: fn, label, source: culprit ? 'ordercache.cpp:88' : null, trust: index === 0 ? 'context' : 'cfi', framework, culprit };
}

const STACKS: Record<number, FrameRow[]> = {
  4121: [
    frame(0, 'KERNELBASE.dll', 'RaiseException', 0x6c, true),
    frame(1, 'coreclr.dll', 'IL_Throw', 0x1b4, true),
    frame(2, 'Shop.Orders.dll', 'OrderCache::Add', 0x4a, false, true),
    frame(3, 'Shop.Orders.dll', 'OrderService::Handle', 0x112, false),
    frame(4, 'OrderService.exe', 'wmain', 0x2b, false),
    frame(5, 'kernel32.dll', 'BaseThreadInitThunk', 0x14, true),
  ],
  4122: [
    frame(0, 'ntdll.dll', 'NtWaitForAlertByThreadId', 0x14, true),
    frame(1, 'ntdll.dll', 'RtlpWaitOnCriticalSection', 0x80, true),
    frame(2, 'Shop.Payments.dll', 'Ledger::Post', 0x22, false),
  ],
  4123: [
    frame(0, 'ntdll.dll', 'NtWaitForAlertByThreadId', 0x14, true),
    frame(1, 'ntdll.dll', 'RtlpWaitOnCriticalSection', 0x80, true),
    frame(2, 'Shop.Payments.dll', 'Ledger::Post', 0x22, false),
  ],
  4124: [frame(0, 'ntdll.dll', 'NtWaitForSingleObject', 0x14, true), frame(1, 'OrderService.exe', 'Timer::Run', 0x10, false)],
};

const THREADS: ThreadRow[] = [
  { id: 4121, name: 'main', state: 'faulted', frames: 6, top: STACKS[4121]![0]!.label, userFrame: STACKS[4121]![2]!.label },
  { id: 4122, name: 'worker-1', state: 'lockWait', frames: 3, top: STACKS[4122]![0]!.label, userFrame: STACKS[4122]![2]!.label },
  { id: 4123, name: 'worker-2', state: 'lockWait', frames: 3, top: STACKS[4123]![0]!.label, userFrame: STACKS[4123]![2]!.label },
  { id: 4124, name: 'timer', state: 'waiting', frames: 2, top: STACKS[4124]![0]!.label, userFrame: STACKS[4124]![1]!.label },
];

const MODULES: ModuleRow[] = [
  ['OrderService.exe', 'C:\\Program Files\\Shop\\OrderService.exe', '0x7ff6a0000000', 0x40000, '1.4.0.0', 'loaded', false],
  ['Shop.Orders.dll', 'C:\\Program Files\\Shop\\Shop.Orders.dll', '0x7ff6b0000000', 0x120000, '1.4.0.0', 'loaded', false],
  ['Shop.Payments.dll', 'C:\\Program Files\\Shop\\Shop.Payments.dll', '0x7ff6b2000000', 0x80000, '1.4.0.0', 'serversDisabled', false],
  ['coreclr.dll', 'C:\\Program Files\\dotnet\\shared\\Microsoft.NETCore.App\\8.0.1\\coreclr.dll', '0x7ffb10000000', 0x4a0000, '8.0.1.0', 'notOnServer', true],
  ['KERNELBASE.dll', 'C:\\Windows\\System32\\KERNELBASE.dll', '0x7ffb1a000000', 0x2c0000, '10.0.19041.3636', 'loaded', true],
  ['ntdll.dll', 'C:\\Windows\\System32\\ntdll.dll', '0x7ffb1c000000', 0x1f8000, '10.0.19041.3636', 'loaded', true],
].map(([n, path, base, size, version, symbols, framework]) => ({
  name: n as string,
  path: path as string,
  base: base as string,
  size: size as number,
  version: version as string,
  debugFile: `${(n as string).replace(/\.(dll|exe)$/, '')}.pdb`,
  debugId: '9C8A0B5D4E2F4A1B8C3D2E1F0A9B8C7D1',
  symbols: { kind: symbols as 'loaded' | 'serversDisabled' | 'notOnServer' },
  framework: framework as boolean,
}));

const DUMP_FINDINGS = (id: number): DumpFinding[] => [
  {
    kind: 'crashCause',
    severity: 'critical',
    title: 'CLR_EXCEPTION in KERNELBASE.dll!RaiseException+0x6c',
    detail: 'A .NET exception was thrown and not handled (the managed exception object has the details). Thread 4121 raised it.',
    evidence: { docId: id, threadId: 4121, frame: 0, module: null, stack: null, threads: [] },
  },
  {
    kind: 'culprit',
    severity: 'critical',
    title: 'Likely culprit: Shop.Orders.dll!OrderCache::Add+0x4a',
    detail: 'The first frame in your code on the faulting thread; the 2 frame(s) above it are in KERNELBASE.dll, coreclr.dll, which crashed on what this code passed in.',
    evidence: { docId: id, threadId: 4121, frame: 2, module: 'Shop.Orders.dll', stack: null, threads: [] },
  },
  {
    kind: 'lockWait',
    severity: 'warning',
    title: '2 thread(s) wait on a lock in Shop.Payments.dll!Ledger::Post',
    detail: 'Threads 4122, 4123 are blocked acquiring the same kind of lock here: contention, or a deadlock with a thread that holds it while waiting on something else.',
    evidence: { docId: id, threadId: 4122, frame: null, module: null, stack: null, threads: [4122, 4123] },
  },
  {
    kind: 'dotnet',
    severity: 'warning',
    title: 'The process runs .NET 8.0.1.0 (coreclr.dll)',
    detail: 'Managed frames show as JIT-compiled code without names, and managed exception objects are not read yet. Open the dump in Visual Studio or WinDbg with SOS (!clrstack, !pe) for the managed side.',
    evidence: { docId: id, threadId: null, frame: null, module: null, stack: null, threads: [] },
  },
];

interface UmdhStackMock {
  key: string;
  label: string;
  module: string;
  frames: string[];
  bytes: number[];
}

const UMDH: UmdhStackMock[] = [
  { key: 'cache', label: 'Shop.Orders!OrderCache::Add+0x4a', module: 'Shop.Orders', frames: ['ntdll!RtlAllocateHeap+0x21', 'ucrtbase!_malloc_base+0x36', 'Shop.Orders!OrderCache::Add+0x4a', 'Shop.Orders!OrderService::Handle+0x112', 'OrderService!wmain+0x2b'], bytes: [10 << 20, 20 << 20, 30 << 20] },
  { key: 'retry', label: 'Shop.Payments!Capture::Retry+0x3c', module: 'Shop.Payments', frames: ['ntdll!RtlAllocateHeap+0x21', 'ucrtbase!_malloc_base+0x36', 'Shop.Payments!Capture::Retry+0x3c', 'Shop.Payments!Capture::Run+0xa0', 'OrderService!wmain+0x2b'], bytes: [102_400, 167_936, 247_808] },
  { key: 'session', label: 'Shop.Orders!Session::Create+0x19', module: 'Shop.Orders', frames: ['ntdll!RtlAllocateHeap+0x21', 'ucrtbase!_malloc_base+0x36', 'Shop.Orders!Session::Create+0x19', 'OrderService!wmain+0x2b'], bytes: [204_800, 210_944, 202_752] },
  { key: 'report', label: 'Shop.Reports!ReportBuffer::Allocate+0x11', module: 'Shop.Reports', frames: ['ntdll!RtlAllocateHeap+0x21', 'Shop.Reports!ReportBuffer::Allocate+0x11', 'OrderService!wmain+0x2b'], bytes: [32 << 20, 32 << 20, 32 << 20] },
];

interface MockDoc {
  summary: DumpDocSummary;
  /** Index into the UMDH series (snapshots only). */
  snapshot: number | null;
}

/**
 * The dump engine of the browser build: a crash dump of OrderService (CLR exception, a culprit frame, two threads
 * waiting on a lock) and three UMDH snapshots of the same process in which the order cache leaks. Plausible
 * answers for the contract; the engine (`imt_dump`) is the reference for behaviour.
 */
export class MockDumpsBackend extends DumpsBackend {
  private docs: MockDoc[] = [];
  private nextId = 1;

  private kindOf(path: string): 'dump' | 'umdh' | 'none' {
    if (/\.(dmp|mdmp)$/i.test(path)) return 'dump';
    if (/umdh|snap|heap/i.test(path) && /\.(txt|log)$/i.test(path)) return 'umdh';
    return 'none';
  }

  async sniff(paths: string[]): Promise<DumpSniffResult[]> {
    return paths.map((path) => ({ path, kind: this.kindOf(path) }));
  }

  async open(path: string): Promise<DumpDocSummary> {
    const kind = this.kindOf(path);
    if (kind === 'none') throw fail('unsupported', `${path} is neither a dump nor a UMDH log IMT can read`);
    if (path.includes('missing')) throw fail('notFound', `cannot open ${path}: The system cannot find the file specified.`);
    this.docs = this.docs.filter((d) => d.summary.path !== path);
    const id = this.nextId++;
    const snapshot = kind === 'umdh' ? Math.max(0, MOCK_UMDH_PATHS.indexOf(path)) : null;
    const summary: DumpDocSummary =
      kind === 'dump'
        ? { id, name: name(path), path, kind: 'dump', timeMs: CRASH_MS, headline: { type: 'exception', name: 'CLR_EXCEPTION', location: 'KERNELBASE.dll!RaiseException+0x6c' }, process: 'OrderService.exe', severity: 'critical' }
        : {
            id,
            name: name(path),
            path,
            kind: 'umdhSnapshot' as DocKind,
            timeMs: CRASH_MS - (11 - 5 * (snapshot ?? 0)) * 60_000,
            headline: { type: 'umdhSnapshot', stacks: UMDH.length, bytes: UMDH.reduce((n, s) => n + s.bytes[snapshot ?? 0]!, 0) },
            process: null,
            severity: 'info',
          };
    this.docs.push({ summary, snapshot });
    return summary;
  }

  async list(): Promise<DumpDocSummary[]> {
    return this.docs.map((d) => d.summary);
  }

  async close(id: number): Promise<boolean> {
    const before = this.docs.length;
    this.docs = this.docs.filter((d) => d.summary.id !== id);
    return before !== this.docs.length;
  }

  private doc(id: number): MockDoc {
    const d = this.docs.find((x) => x.summary.id === id);
    if (!d) throw fail('notFound', `dump document ${id} is not open (any more)`);
    return d;
  }

  private dump(id: number): MockDoc {
    const d = this.doc(id);
    if (d.summary.kind !== 'dump') throw fail('unsupported', 'this is a UMDH log, not a dump');
    return d;
  }

  async overview(id: number): Promise<DumpOverview> {
    const d = this.dump(id);
    return {
      summary: d.summary,
      dumpKind: 'mini',
      os: 'Windows NT 10.0.19045',
      arch: 'x64',
      processors: 8,
      cpu: 'GenuineIntel family 6',
      processName: 'OrderService.exe',
      processPath: 'C:\\Program Files\\Shop\\OrderService.exe',
      pid: 4120,
      startedMs: CRASH_MS - 4 * 3_600_000,
      uptimeS: 4 * 3600,
      exception: {
        code: '0xE0434352',
        name: 'CLR_EXCEPTION',
        meaning: 'A .NET exception was thrown and not handled (the managed exception object has the details).',
        access: null,
        address: '0x7ffb1a00106c',
        location: 'KERNELBASE.dll!RaiseException+0x6c',
        threadId: 4121,
        fastFail: null,
      },
      threads: THREADS.length,
      modules: MODULES.length,
      modulesWithSymbols: MODULES.filter((m) => m.symbols.kind === 'loaded').length,
      symbolProblems: { missing: 0, serversDisabled: 1, notOnServer: 1, mismatch: 0, unreadable: 0, downloadFailed: 0, noIdentity: 0 },
      framesSymbolised: 14,
      frames: 14,
      handles: { total: 412, byType: [{ name: 'Event', count: 200 }, { name: 'File', count: 120 }, { name: 'Key', count: 92 }] },
      memory: { regions: 900, committed: 300 << 20, reserved: 600 << 20, free: 2 ** 46, image: 80 << 20, mapped: 20 << 20, private: 200 << 20, largestFree: 2 ** 45 },
      dotnet: '.NET 8.0.1.0 (coreclr.dll)',
      unloadedModules: ['Legacy.dll'],
      notes: [],
    };
  }

  async threads(r: ThreadsRequest): Promise<ThreadsPage> {
    this.dump(r.id);
    const f = r.filter?.trim().toLowerCase();
    const rows = THREADS.filter((t) => !f || `${t.id} ${t.name ?? ''} ${(STACKS[t.id] ?? []).map((x) => x.label).join(' ')}`.toLowerCase().includes(f));
    return { rows: rows.slice(r.offset, r.offset + r.limit), total: rows.length };
  }

  async stack(r: StackRequest): Promise<StackDto> {
    this.dump(r.id);
    const frames = STACKS[r.threadId];
    if (!frames) throw fail('notFound', `the dump has no thread ${r.threadId}`);
    return { threadId: r.threadId, frames, truncated: false };
  }

  async modules(r: ModulesRequest): Promise<ModulesPage> {
    this.dump(r.id);
    const f = r.filter?.trim().toLowerCase();
    const rows = MODULES.filter((m) => !f || `${m.path} ${m.version ?? ''}`.toLowerCase().includes(f));
    return { rows: rows.slice(r.offset, r.offset + r.limit), total: rows.length };
  }

  /** The series positions of `ids` (UMDH snapshots), oldest first. */
  private series(ids: number[]): number[] {
    return ids.map((id) => this.doc(id).snapshot ?? 0).sort((a, b) => a - b);
  }

  async findings(r: DumpFindingsRequest): Promise<DumpFinding[]> {
    if (!r.ids.length) return [];
    const first = this.doc(r.ids[0]!);
    if (first.summary.kind === 'dump') return DUMP_FINDINGS(first.summary.id);
    const s = this.series(r.ids);
    const target = r.ids.find((id) => this.doc(id).snapshot === s[s.length - 1]) ?? r.ids[0]!;
    if (s.length < 2) {
      return [{ kind: 'largeAllocation', severity: 'warning', title: 'A single allocation of 32.0 MB from Shop.Reports!ReportBuffer::Allocate+0x11', detail: 'Very large single allocations fragment the heap and fail first when address space runs low.', evidence: { docId: target, threadId: null, frame: null, module: 'Shop.Reports', stack: 'report', threads: [] } }];
    }
    const out: DumpFinding[] = [
      { kind: 'leakGrowth', severity: 'critical', title: 'Shop.Orders!OrderCache::Add+0x4a grew by 20.0 MB (+320 allocations)', detail: '99% of the heap\'s growth comes from this allocation stack (10.0 MB → 30.0 MB). Memory allocated here is not freed; look at who keeps it.', evidence: { docId: target, threadId: null, frame: null, module: 'Shop.Orders', stack: 'cache', threads: [] } },
    ];
    if (s.length >= 3) {
      out.push({ kind: 'steadyGrowth', severity: 'critical', title: `Steady growth in Shop.Orders!OrderCache::Add+0x4a across ${s.length} snapshots`, detail: 'It grew in every snapshot (10.0 MB → 20.0 MB → 30.0 MB): a leak rather than a cache warming up.', evidence: { docId: target, threadId: null, frame: null, module: 'Shop.Orders', stack: 'cache', threads: [] } });
    }
    out.push({ kind: 'heapTotal', severity: 'info', title: 'The heap changed by 20.1 MB (42.3 MB → 62.4 MB)', detail: `Compared the first and the last of ${s.length} snapshots.`, evidence: { docId: target, threadId: null, frame: null, module: null, stack: null, threads: [] } });
    return out;
  }

  private growth(base: number | null, target: number): { key: string; m: UmdhStackMock; delta: number; bytes: number }[] {
    const t = this.doc(target);
    if (t.summary.kind === 'dump') throw fail('unsupported', 'this is a dump, not a UMDH log');
    const ti = t.snapshot ?? 0;
    const bi = base === null ? null : (this.doc(base).snapshot ?? 0);
    return UMDH.map((m) => ({ key: m.key, m, bytes: m.bytes[ti]!, delta: m.bytes[ti]! - (bi === null ? 0 : m.bytes[bi]!) })).sort((a, b) => b.delta - a.delta);
  }

  async umdhDiff(r: UmdhDiffRequest): Promise<UmdhDiffPage> {
    const g = this.growth(r.base, r.target);
    const by = new Map<string, UmdhRow>();
    for (const x of g) {
      const key = r.group === 'stack' ? x.key : r.group === 'module' ? x.m.module : x.m.label.replace(/\+0x[0-9a-f]+$/, '');
      const row = by.get(key) ?? { key, label: r.group === 'stack' ? x.m.label : key, module: x.m.module, deltaBytes: 0, deltaCount: 0, bytes: 0, count: 0, stacks: 0 };
      row.deltaBytes += x.delta;
      row.deltaCount += Math.round(x.delta / 65_536);
      row.bytes += x.bytes;
      row.count += Math.round(x.bytes / 65_536);
      row.stacks += 1;
      by.set(key, row);
    }
    const f = r.filter?.trim().toLowerCase();
    const rows = [...by.values()].filter((x) => !f || x.label.toLowerCase().includes(f)).sort((a, b) => b.deltaBytes - a.deltaBytes);
    return { rows: rows.slice(r.offset, r.offset + r.limit), total: rows.length, deltaBytes: g.reduce((n, x) => n + x.delta, 0), deltaCount: 0, bytes: g.reduce((n, x) => n + x.bytes, 0) };
  }

  async umdhStack(r: UmdhStackRequest): Promise<UmdhStackDto> {
    const x = this.growth(r.base, r.target).find((g) => g.key === r.key);
    if (!x) throw fail('notFound', 'no such allocation stack');
    return {
      key: x.key,
      deltaBytes: x.delta,
      deltaCount: Math.round(x.delta / 65_536),
      ids: [0xa2c3],
      frames: x.m.frames.map((label, index) => {
        const [module, fn] = label.split('!');
        const framework = /^(ntdll|ucrtbase)/.test(label);
        return { index, address: '', module: module ?? null, function: fn?.replace(/\+0x[0-9a-f]+$/, '') ?? null, label, source: null, trust: 'recorded', framework, culprit: label === x.m.label };
      }),
    };
  }

  async umdhTree(r: UmdhTreeRequest): Promise<UmdhTreeNode[]> {
    const nodes = new Map<string, UmdhTreeNode>();
    for (const x of this.growth(r.base, r.target)) {
      const rev = [...x.m.frames].reverse().map((l) => l.replace(/\+0x[0-9a-f]+$/, ''));
      if (rev.length <= r.path.length || !r.path.every((p, i) => rev[i] === p)) continue;
      const key = rev[r.path.length]!;
      const n = nodes.get(key) ?? { key, label: key, deltaBytes: 0, deltaCount: 0, hasChildren: false, framework: /^(ntdll|ucrtbase)/.test(key) };
      n.deltaBytes += x.delta;
      n.hasChildren ||= rev.length > r.path.length + 1;
      nodes.set(key, n);
    }
    return [...nodes.values()].sort((a, b) => b.deltaBytes - a.deltaBytes).slice(0, r.limit);
  }

  async markers(): Promise<DumpMarker[]> {
    return this.docs
      .filter((d) => d.summary.timeMs !== null)
      .map((d) => ({ id: d.summary.id, name: d.summary.name, kind: d.summary.kind, timeMs: d.summary.timeMs!, headline: d.summary.headline, severity: d.summary.severity }));
  }

  async explain(r: DumpFindingsRequest): Promise<string> {
    if (!r.ids.length) throw fail('invalid', 'nothing to explain');
    const f = await this.findings(r);
    return [`Explain these documents in plain words: ${r.ids.map((id) => this.doc(id).summary.name).join(', ')}.`, 'Findings computed by IMT\'s dump engine:', ...f.map((x) => `- [${x.severity}] ${x.title}. ${x.detail}`)].join('\n');
  }
}
