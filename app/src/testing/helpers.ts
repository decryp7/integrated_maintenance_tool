import { Component, EnvironmentProviders, Provider, signal } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { MarkerKind, PluginInfo, Query } from '../app/core/api.types';
import { DebriefBackend } from '../app/core/backend';
import { Logger, LogProps } from '../app/core/logger';
import { DEMO_START, MockDataset, MockRecord } from '../app/core/mock/mock-data';
import { MockBackend, MockBackendOptions } from '../app/core/mock/mock-backend';
import { EMPTY_QUERY } from '../app/core/workspace.store';
import { provideDebriefBuiltins } from '../app/builtin/provide-builtins';
import { BUILTIN_VIEWS } from '../app/builtin/builtin-views';
import { DockService } from '../app/dock/dock.service';
import { provideWorkspace } from '../app/core/workspace/provide-workspace';
import { ConsumerGate } from '../app/core/panel-visibility';

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

export function tinyBackend(options: MockBackendOptions = {}): MockBackend {
  return new MockBackend({ latency: false, dataset: tinyDataset, pluginMarkerKinds: [], ...options });
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

/** Stand-in for DockService in component tests: records calls, no dockview. */
export class FakeDock {
  readonly openIds = signal<ReadonlySet<string>>(new Set(['timeline']));
  readonly calls: string[] = [];
  isOpen = (id: string) => this.openIds().has(id);
  /** Every open view counts as showing, unless a test sets it apart. */
  readonly hidden = new Set<string>();
  isVisible = (id: string) => this.openIds().has(id) && !this.hidden.has(id);
  /** Records `show:<id>`, or `show:<id>:inactive` for a background open. */
  show = (id: string, options: { inactive?: boolean } = {}) => void this.calls.push(options.inactive ? `show:${id}:inactive` : `show:${id}`);
  toggle = (id: string) => void this.calls.push(`toggle:${id}`);
  close = (id: string) => void this.calls.push(`close:${id}`);
  reset = () => void this.calls.push('reset');
  slideIn = (id: string) => void this.calls.push(`slideIn:${id}`);
  focusFront = () => void this.calls.push('focusFront');
  // A method (not a field): some specs replace it on the prototype.
  setTitle(id: string, title: string): void {
    this.calls.push(`title:${id}:${title}`);
  }
  /** Ticks like the dock's layout changes (`DockService.layoutChanged`). */
  readonly layoutChanged = signal(0);
  readonly activeViewId = signal<string | null>(null);
  /** Like `DockService.ready`: the dock has its layout. */
  readonly ready = signal(true);
}

export function provideFakeDock(dock = new FakeDock()): Provider[] {
  return [{ provide: DockService, useValue: dock }];
}

/** Backend and logger only (no built-in views/commands), for testing registries in isolation. */
export function provideTinyBackendBare(backend: DebriefBackend = tinyBackend()): Provider[] {
  return [
    { provide: DebriefBackend, useValue: backend },
    { provide: Logger, useFactory: () => new MemoryLogger() },
  ];
}

@Component({ selector: 'debrief-stub-panel', template: 'stub panel' })
class StubPanel {}

/** The built-in views with the heavy panels (charts, profile editor) replaced by a stub, for dock tests in jsdom. */
export function provideDockTesting(backend: DebriefBackend = tinyBackend()): (Provider | EnvironmentProviders)[] {
  const views = BUILTIN_VIEWS.map((v) => (v.id === 'metrics' || v.id === 'profile' ? { ...v, component: async () => StubPanel } : v));
  return [provideDebriefBuiltins(views), provideWorkspace(), ...provideTinyBackendBare(backend)];
}

export function provideTinyBackend(backend: DebriefBackend = tinyBackend()): (Provider | EnvironmentProviders)[] {
  return [provideDebriefBuiltins(), provideWorkspace(), ...provideTinyBackendBare(backend)];
}

/** The MemoryLogger provided by provideTinyBackend. */
export function testLogger(): MemoryLogger {
  return TestBed.inject(Logger) as MemoryLogger;
}

export function query(patch: Partial<Query> = {}): Query {
  return { ...EMPTY_QUERY, ...patch };
}

/** A plugin as the engine reports it: active, healthy, no dependencies, plus `patch`. */
export function pluginInfo(patch: Partial<PluginInfo> & Pick<PluginInfo, 'id'>): PluginInfo {
  return {
    name: patch.id,
    version: '1.0.0',
    contributes: [],
    status: 'active',
    failures: 0,
    lastError: null,
    requires: [],
    provides: [],
    inactiveReason: null,
    dependents: [],
    warnings: [],
    hung: false,
    ...patch,
  };
}

/** A marker kind as the engine reports it: a profile kind with no style, plus `patch`. */
export function markerKind(patch: Partial<MarkerKind> & Pick<MarkerKind, 'kind'>): MarkerKind {
  return {
    owner: 'profile',
    role: null,
    label: patch.kind,
    lifecycle: false,
    color: null,
    icon: null,
    badge: null,
    shape: null,
    description: null,
    when: [],
    ...patch,
  };
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

/** Registers a visible consumer of a gated root store (as an open panel would), so it queries the engine in a test. */
export function useGate(gate: ConsumerGate, visible = signal(true)): { visible: typeof visible; release(): void } {
  let release = () => undefined as void;
  gate.add(visible, { onDestroy: (fn: () => void) => ((release = fn), () => undefined) });
  return { visible, release: () => release() };
}

/**
 * Animation frames the test runs by hand (`flush()` runs every frame requested so far). Pointer moves are published
 * once per frame, so a test dispatching moves flushes before it reads the result. `restore()` puts the real ones back.
 */
export function manualFrames(): { flush(): void; readonly pending: number; restore(): void } {
  const queue = new Map<number, FrameRequestCallback>();
  let next = 0;
  const real = { request: globalThis.requestAnimationFrame, cancel: globalThis.cancelAnimationFrame };
  globalThis.requestAnimationFrame = (cb: FrameRequestCallback) => (queue.set(++next, cb), next);
  globalThis.cancelAnimationFrame = (h: number) => void queue.delete(h);
  return {
    flush() {
      const due = [...queue.values()];
      queue.clear();
      for (const cb of due) cb(performance.now());
    },
    get pending() {
      return queue.size;
    },
    restore() {
      globalThis.requestAnimationFrame = real.request;
      globalThis.cancelAnimationFrame = real.cancel;
    },
  };
}
