import { Provider } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { Query } from '../app/core/api.types';
import { ImtBackend } from '../app/core/backend';
import { Logger, LogProps } from '../app/core/logger';
import { DEMO_START, MockDataset, MockRecord } from '../app/core/mock/mock-data';
import { MockBackend } from '../app/core/mock/mock-backend';
import { EMPTY_QUERY } from '../app/core/workspace.store';

export const T0 = DEMO_START;
export const SEC = 1000;

type Partial0 = Omit<MockRecord, 'id' | 'markers' | 'line' | 'template'> & { template?: string | null; line?: number };

function rec(r: Partial0, line: number): MockRecord {
  return { id: 0, markers: [], template: null, line, ...r };
}

/**
 * Small hand-made incident: WEB01/100 crashes (fatal), restarts as WEB01/200;
 * WEB02/300 runs and shuts down cleanly. One record has no timestamp.
 */
export function tinyDataset(): MockDataset {
  const svc = (machine: string, pid: number, extra: Record<string, unknown> = {}) => ({ Application: 'Svc', MachineName: machine, ProcessId: pid, ...extra });
  const records: MockRecord[] = [
    rec({ ts: T0, level: 'Information', message: 'Application starting — Svc 1.0 (net9.0)', template: 'Application starting — {Application} {Version} ({Framework})', exception: null, fields: svc('WEB01', 100, { Version: '1.0' }), source: 0 }, 1),
    rec({ ts: T0 + 60 * SEC, level: 'Information', message: 'Process stats CPU 10.5% · WorkingSet 100 MB · Threads 20', exception: null, fields: svc('WEB01', 100, { SourceContext: 'Diagnostics.ProcessMonitor' }), source: 0 }, 2),
    rec({ ts: T0 + 120 * SEC, level: 'Warning', message: 'Slow request /api took 1500 ms', exception: null, fields: svc('WEB01', 100, { CorrelationId: 'c1', ElapsedMs: 1500 }), source: 0 }, 3),
    rec({ ts: T0 + 180 * SEC, level: 'Error', message: 'Failed to capture payment', exception: 'System.TimeoutException: timed out\n   at X.Y()', fields: svc('WEB01', 100, { CorrelationId: 'c1', ElapsedMs: 5000 }), source: 0 }, 4),
    rec({ ts: T0 + 240 * SEC, level: 'Fatal', message: 'Host terminated unexpectedly', exception: 'System.TimeoutException: boom', fields: svc('WEB01', 100), source: 0 }, 6),
    rec({ ts: T0 + 300 * SEC, level: 'Information', message: 'Application starting — Svc 1.1 (net9.0)', exception: null, fields: svc('WEB01', 200), source: 0 }, 8),
    rec({ ts: T0 + 360 * SEC, level: 'Information', message: 'Process stats CPU 20.0% · WorkingSet 1.5 GB · Threads 30', exception: null, fields: svc('WEB01', 200, { SourceContext: 'Diagnostics.ProcessMonitor' }), source: 0 }, 9),
    rec({ ts: T0 + 30 * SEC, level: 'Information', message: 'Application starting — Worker', exception: null, fields: svc('WEB02', 300), source: 1 }, 1),
    rec({ ts: T0 + 400 * SEC, level: 'Debug', message: 'Cache hit for order:1', exception: null, fields: svc('WEB02', 300, { ElapsedMs: 2 }), source: 1 }, 2),
    rec({ ts: null, level: 'Information', message: 'line without a timestamp', exception: null, fields: svc('WEB02', 300), source: 1 }, 3),
    rec({ ts: T0 + 500 * SEC, level: 'Information', message: 'Application is shutting down...', exception: null, fields: svc('WEB02', 300), source: 1 }, 4),
  ];
  return {
    sources: [
      { id: 0, path: '/logs/web01/app.clef', name: 'app.clef', importer: 'json', skipped: 0 },
      { id: 1, path: '/logs/web02/app.log', name: 'app.log', importer: 'text', skipped: 1 },
    ],
    unrecognized: [{ path: '/logs/system/export.reg', name: 'export.reg', reason: 'No importer' }],
    records,
  };
}

export function tinyBackend(): MockBackend {
  return new MockBackend({ latency: false, dataset: tinyDataset });
}

export interface LogEntry {
  level: 'debug' | 'info' | 'warn' | 'error';
  category: string;
  message: string;
  props?: LogProps;
}

/** Captures log events for assertions and keeps test output quiet. */
export class MemoryLogger extends Logger {
  readonly entries: LogEntry[] = [];
  debug(category: string, message: string, props?: LogProps): void {
    this.entries.push({ level: 'debug', category, message, props });
  }
  info(category: string, message: string, props?: LogProps): void {
    this.entries.push({ level: 'info', category, message, props });
  }
  warn(category: string, message: string, props?: LogProps): void {
    this.entries.push({ level: 'warn', category, message, props });
  }
  error(category: string, message: string, props?: LogProps): void {
    this.entries.push({ level: 'error', category, message, props });
  }
}

export function provideTinyBackend(backend: ImtBackend = tinyBackend()): Provider[] {
  return [
    { provide: ImtBackend, useValue: backend },
    { provide: Logger, useFactory: () => new MemoryLogger() },
  ];
}

/** The MemoryLogger provided by provideTinyBackend. */
export function testLogger(): MemoryLogger {
  return TestBed.inject(Logger) as MemoryLogger;
}

export function query(patch: Partial<Query> = {}): Query {
  return { ...EMPTY_QUERY, ...patch };
}

/** Lets promises, effects and change detection settle. */
export async function settle<T>(fixture?: ComponentFixture<T>, rounds = 4): Promise<void> {
  for (let i = 0; i < rounds; i++) {
    await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
    TestBed.tick();
    if (fixture) await fixture.whenStable();
  }
}

export function el<T extends Element = HTMLElement>(fixture: ComponentFixture<unknown>, selector: string): T {
  const found = (fixture.nativeElement as HTMLElement).querySelector<T>(selector);
  if (!found) throw new Error(`No element for ${selector}`);
  return found;
}

export function all<T extends Element = HTMLElement>(fixture: ComponentFixture<unknown>, selector: string): T[] {
  return [...(fixture.nativeElement as HTMLElement).querySelectorAll<T>(selector)];
}

export function text(fixture: ComponentFixture<unknown>): string {
  return (fixture.nativeElement as HTMLElement).textContent?.replace(/\s+/g, ' ') ?? '';
}

export function buttonByText(fixture: ComponentFixture<unknown>, label: string | RegExp): HTMLButtonElement {
  const b = all<HTMLButtonElement>(fixture, 'button').find((x) => (typeof label === 'string' ? x.textContent?.trim() === label : label.test(x.textContent ?? '')));
  if (!b) throw new Error(`No button "${label}"`);
  return b;
}

export function typeInto(input: HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement, value: string, event = 'input'): void {
  input.value = value;
  input.dispatchEvent(new Event(event, { bubbles: true }));
}
