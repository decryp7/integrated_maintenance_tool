import { EtlConverted, EtlProcess, NativeCaptureMode, NativeCaptureRequest, NativeCaptureStatus, NativeDefaults, NativeProcess } from '../../core/api.types';
import { CmdError } from '../../core/cmd-error';
import { NativeBackend } from './native-backend';

/** Demo captures open this (the profiling mock knows it). */
export const MOCK_NATIVE_PROFILE_PATH = '/demo/profiles/orders.speedscope.json';

/**
 * Plausible stand-in for the native profiling tools (browser demo and UI tests only): a 64-bit service, a 32-bit
 * tool and an elevated service the sampler cannot open, captures whose size grows with time, and an ETW mode that
 * answers like the elevated process. Time is injectable.
 */
export class MockNativeBackend extends NativeBackend {
  private readonly list: NativeCaptureStatus[] = [];
  private readonly started = new Map<string, { at: number; durationMs: number | null }>();
  private next = 1;
  /** Operations that fail (tests): name → message. */
  readonly failures = new Map<string, string>();
  /** The elevated process is refused (the user declined the administrator prompt). */
  declineElevation = false;
  /** Modes asked for by `captures` (tests: ETW is not polled before it was used). */
  readonly polled: NativeCaptureMode[][] = [];

  constructor(private readonly now: () => number = () => Date.now()) {
    super();
  }

  private fail(op: string): void {
    const m = this.failures.get(op);
    if (m) throw new Error(m);
  }

  async defaults(): Promise<NativeDefaults> {
    this.fail('defaults');
    return { symbolPath: 'C:\\symbols', symbolServer: false, rateHz: 100, maxRateHz: 1000, maxDurationSec: 3600, supported: true };
  }

  async processes(): Promise<NativeProcess[]> {
    this.fail('processes');
    return [
      { pid: 3100, name: 'OrderService.exe', path: 'C:\\Program Files\\Shop\\OrderService.exe', arch: 'x64', accessible: true },
      { pid: 3200, name: 'LegacyReport.exe', path: 'C:\\Tools\\LegacyReport.exe', arch: 'x86', accessible: true },
      { pid: 640, name: 'svchost.exe', path: null, arch: 'x64', accessible: false },
    ];
  }

  async start(mode: NativeCaptureMode, r: NativeCaptureRequest): Promise<NativeCaptureStatus> {
    this.fail('start');
    const p = (await this.processes()).find((x) => x.pid === r.pid);
    if (!p) throw new Error(`process ${r.pid} is not running`);
    const denied = (message: string) => new CmdError({ code: 'denied', message, retryable: false, hint: 'Restart IMT as administrator, then try again.' });
    if (mode === 'sampling' && !p.accessible) {
      throw denied(`IMT cannot read ${p.name} (${p.pid}): it runs elevated or as another user. Record it with ETW (administrator)`);
    }
    if (mode === 'etw' && this.declineElevation) {
      throw denied('Native profiling needs administrator rights for this; the administrator prompt was declined');
    }
    const id = `${mode}-${this.next++}`;
    const status: NativeCaptureStatus = { id, pid: p.pid, processName: p.name, mode, state: 'recording', elapsedMs: 0, bytes: 0, samples: 0, path: null, error: null, needsElevation: false, truncated: false, notes: [] };
    this.started.set(id, { at: this.now(), durationMs: r.durationSec ? r.durationSec * 1000 : null });
    this.list.unshift(status);
    return { ...status };
  }

  private refresh(c: NativeCaptureStatus): NativeCaptureStatus {
    const s = this.started.get(c.id);
    if (s && c.state === 'recording') {
      const elapsed = this.now() - s.at;
      c.elapsedMs = elapsed;
      c.bytes = Math.round(elapsed * 220);
      c.samples = Math.round(elapsed / 10);
      if (s.durationMs !== null && elapsed >= s.durationMs) this.finish(c);
    }
    return { ...c };
  }

  private finish(c: NativeCaptureStatus): void {
    c.state = 'done';
    c.path = MOCK_NATIVE_PROFILE_PATH;
    c.notes = ['Symbols: 3 of 5 modules had a PDB; without one (exports or addresses only): ntdll.dll, kernel32.dll'];
    this.started.delete(c.id);
  }

  async captures(modes: readonly NativeCaptureMode[]): Promise<NativeCaptureStatus[]> {
    this.fail('captures');
    this.polled.push([...modes]);
    return this.list.filter((c) => modes.includes(c.mode)).map((c) => this.refresh(c));
  }

  private find(id: string): NativeCaptureStatus {
    const c = this.list.find((x) => x.id === id);
    if (!c) throw new Error(`no capture "${id}"`);
    return c;
  }

  async stop(_mode: NativeCaptureMode, id: string): Promise<NativeCaptureStatus> {
    this.fail('stop');
    const c = this.find(id);
    this.refresh(c);
    if (c.state === 'recording') this.finish(c);
    return { ...c };
  }

  async cancel(_mode: NativeCaptureMode, id: string): Promise<NativeCaptureStatus> {
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
      { pid: 3100, name: 'OrderService.exe', commandLine: null, samples: 15800, clrEvents: 0, pointerSize: 8 },
      { pid: 880, name: 'sqlservr.exe', commandLine: null, samples: 8200, clrEvents: 0, pointerSize: 8 },
      { pid: 3200, name: 'LegacyReport.exe', commandLine: null, samples: 900, clrEvents: 0, pointerSize: 4 },
    ];
  }

  async etlConvert(path: string, pid: number): Promise<EtlConverted> {
    this.fail('etlConvert');
    return { path: MOCK_NATIVE_PROFILE_PATH, notes: [`Converted process ${pid} of ${path}`] };
  }
}
