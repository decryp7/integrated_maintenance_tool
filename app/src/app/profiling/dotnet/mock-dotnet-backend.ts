import { DotnetCaptureRequest, DotnetCaptureStatus, DotnetProcess, EtlConverted, EtlProcess } from '../../core/api.types';
import { DotnetBackend } from './dotnet-backend';

/** Demo captures open these (the profiling mock knows them). */
export const MOCK_PROFILE_PATH = '/demo/profiles/orders.speedscope.json';
export const MOCK_HEAP_PATH = '/demo/profiles/heap-3.heapsnapshot';

/**
 * Plausible stand-in for the `dotnet-profiler` tool (browser demo and UI tests only): a .NET 10 service, a
 * 32-bit .NET Framework service that needs elevation, and captures whose size grows with time. Time is injectable.
 */
export class MockDotnetBackend extends DotnetBackend {
  private readonly list: DotnetCaptureStatus[] = [];
  private readonly started = new Map<string, { at: number; durationMs: number | null }>();
  private next = 1;
  /** Operations that fail (tests): name → message. */
  readonly failures = new Map<string, string>();

  constructor(private readonly now: () => number = () => Date.now()) {
    super();
  }

  private fail(op: string): void {
    const m = this.failures.get(op);
    if (m) throw new Error(m);
  }

  async processes(): Promise<DotnetProcess[]> {
    this.fail('processes');
    return [
      { pid: 4242, name: 'Shop.Web.exe', commandLine: 'Shop.Web.exe --urls http://*:5000', runtimeVersion: '.NET 10.0.1', arch: 'x64', runtime: 'core', needsElevation: false },
      { pid: 5150, name: 'Legacy.Billing.exe', commandLine: null, runtimeVersion: '.NET Framework 4 (32-bit)', arch: 'x86', runtime: 'framework', needsElevation: true },
      { pid: 6001, name: 'Inventory.Worker.exe', commandLine: null, runtimeVersion: '.NET Framework 4 (64-bit)', arch: 'x64', runtime: 'framework', needsElevation: false },
    ];
  }

  async start(r: DotnetCaptureRequest): Promise<DotnetCaptureStatus> {
    this.fail('start');
    const procs = await this.processes();
    const p = procs.find((x) => x.pid === r.pid);
    if (!p) throw new Error(`process ${r.pid} is not a running .NET process (refresh the list)`);
    if (this.list.some((c) => c.pid === r.pid && (c.state === 'recording' || c.state === 'stopping'))) {
      throw new Error(`process ${r.pid} is already being recorded; stop that capture first`);
    }
    const id = `dotnet-${this.next++}`;
    const status: DotnetCaptureStatus = { id, pid: p.pid, processName: p.name, kind: r.kind, state: 'recording', elapsedMs: 0, bytes: 0, path: null, error: null, needsElevation: false, truncated: false, notes: [] };
    if (p.needsElevation) {
      status.state = 'failed';
      status.needsElevation = true;
      status.error = 'recording this process needs administrator rights (ETW); restart IMT as administrator';
    } else {
      this.started.set(id, { at: this.now(), durationMs: r.kind === 'heap' ? 1500 : r.durationSec ? r.durationSec * 1000 : null });
    }
    this.list.unshift(status);
    return { ...status };
  }

  private refresh(c: DotnetCaptureStatus): DotnetCaptureStatus {
    const s = this.started.get(c.id);
    if (s && c.state === 'recording') {
      const elapsed = this.now() - s.at;
      c.elapsedMs = elapsed;
      c.bytes = Math.round(elapsed * 350);
      if (s.durationMs !== null && elapsed >= s.durationMs) this.finish(c);
    }
    return { ...c };
  }

  private finish(c: DotnetCaptureStatus): void {
    c.state = 'done';
    c.path = c.kind === 'heap' ? MOCK_HEAP_PATH : MOCK_PROFILE_PATH;
    this.started.delete(c.id);
  }

  async captures(): Promise<DotnetCaptureStatus[]> {
    this.fail('captures');
    return this.list.map((c) => this.refresh(c));
  }

  private find(id: string): DotnetCaptureStatus {
    const c = this.list.find((x) => x.id === id);
    if (!c) throw new Error(`there is no capture "${id}"`);
    return c;
  }

  async stop(id: string): Promise<DotnetCaptureStatus> {
    this.fail('stop');
    const c = this.find(id);
    this.refresh(c);
    if (c.state === 'recording') this.finish(c);
    return { ...c };
  }

  async cancel(id: string): Promise<DotnetCaptureStatus> {
    this.fail('cancel');
    const c = this.find(id);
    if (c.state === 'recording') {
      c.state = 'cancelled';
      this.started.delete(id);
    }
    return { ...c };
  }

  async etlProcesses(path: string): Promise<EtlProcess[]> {
    this.fail('etlProcesses');
    if (!path.toLowerCase().endsWith('.etl')) throw new Error(`${path} is not an .etl file`);
    return [
      { pid: 4242, name: 'Shop.Web.exe', commandLine: null, samples: 12500, clrEvents: 3400, pointerSize: 8 },
      { pid: 880, name: 'sqlservr.exe', commandLine: null, samples: 8200, clrEvents: 0, pointerSize: 8 },
      { pid: 5150, name: 'Legacy.Billing.exe', commandLine: null, samples: 900, clrEvents: 120, pointerSize: 4 },
    ];
  }

  async etlConvert(path: string, pid: number): Promise<EtlConverted> {
    this.fail('etlConvert');
    return { path: MOCK_PROFILE_PATH, notes: [`Converted process ${pid} of ${path}`] };
  }
}
