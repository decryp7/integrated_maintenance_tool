import { TestBed } from '@angular/core/testing';
import { provideTinyBackend } from '../../testing/helpers';
import { FailureReporter } from '../core/failure-reporter';
import { MockWindowingBackend } from './mock-windowing-backend';
import { provideWindowingBackend, TauriWindowingBackend, WindowingBackend } from './windowing-backend';
import { parseOpenAsk, parseTasks, parseWindows } from './windowing-payloads';
import { BackgroundTask } from './windowing.types';
import { at } from '../../testing/must';

const task: BackgroundTask = { id: 't', label: 'Live tail', kind: 'liveTail', window: 'main', progress: 0.5, stoppable: true, startedAt: 1 };

function fakes(result: unknown = undefined) {
  const invoke = vi.fn(async (..._args: unknown[]) => result);
  const handlers = new Map<string, (payload: unknown) => void>();
  const listen = vi.fn(async (event: string, handler: (payload: unknown) => void) => {
    handlers.set(event, handler);
    return () => undefined;
  });
  return { invoke, listen, handlers, backend: new TauriWindowingBackend(invoke as never, listen, async () => 'w-2') };
}

describe('TauriWindowingBackend', () => {
  it('maps every command to its Tauri name and camelCase arguments', async () => {
    const { invoke, backend: b } = fakes();
    const calls: [() => Promise<unknown>, string, unknown?][] = [
      [() => b.newWindow(), 'new_window'],
      [() => b.closeWindow(), 'close_window'],
      [() => b.listWindows(), 'list_windows'],
      [() => b.focusWindow('main'), 'focus_window', { label: 'main' }],
      [() => b.setWindowTitle('P — IMT'), 'set_window_title', { title: 'P — IMT' }],
      [() => b.restoreWindows(), 'restore_windows'],
      [() => b.resolveClose('tray', true), 'resolve_close', { choice: 'tray', remember: true }],
      [() => b.resolveOpen('newWindow'), 'resolve_open', { choice: 'newWindow' }],
      [() => b.resolveOpen('cancel'), 'resolve_open', { choice: 'cancel' }],
      [() => b.backgroundTasks(), 'background_tasks'],
      [() => b.stopBackgroundTask('t'), 'stop_background_task', { id: 't' }],
      [() => b.setRecentProjects([{ path: '/p', name: 'P' }]), 'set_recent_projects', { projects: [{ path: '/p', name: 'P' }] }],
    ];
    for (const [call, name, args] of calls) {
      invoke.mockClear();
      await call();
      expect(invoke).toHaveBeenCalledTimes(1);
      expect(invoke.mock.calls[0]?.[0]).toBe(name);
      if (args) expect(at(invoke.mock.calls, 0)[1]).toEqual(args);
    }
    expect(await b.label()).toBe('w-2');
  });

  it('parses list results', async () => {
    expect(await fakes([{ label: 'main', title: 'IMT', focused: true, visible: true }, 7]).backend.listWindows()).toEqual([{ label: 'main', title: 'IMT', focused: true, visible: true }]);
    expect(await fakes([task]).backend.backgroundTasks()).toEqual([task]);
    expect(await fakes(null).backend.backgroundTasks()).toEqual([]);
  });

  it('subscribes to each event and passes typed payloads', async () => {
    const { listen, handlers, backend } = fakes();
    const got: unknown[] = [];
    await backend.onCloseRequested(() => got.push('close'));
    await backend.onOpenAsk((a) => got.push(a));
    await backend.onCapture(() => got.push('capture'));
    await backend.onLiveStopped(() => got.push('live'));
    await backend.onBackgroundTasks((t) => got.push(t));
    await backend.onWindowsChanged((w) => got.push(w));
    expect(listen.mock.calls.map((c) => c[0])).toEqual([
      'imt://close-requested',
      'imt://open-ask',
      'imt://capture',
      'imt://live-stopped',
      'imt://background-tasks',
      'imt://windows-changed',
    ]);
    handlers.get('imt://close-requested')!(null);
    handlers.get('imt://open-ask')!({ count: 2, names: ['a.log', 'b.log'] });
    handlers.get('imt://open-ask')!('garbage');
    handlers.get('imt://capture')!(null);
    handlers.get('imt://live-stopped')!(null);
    handlers.get('imt://background-tasks')!([task]);
    handlers.get('imt://windows-changed')!([{ label: 'main' }]);
    expect(got).toEqual(['close', { count: 2, names: ['a.log', 'b.log'] }, 'capture', 'live', [task], [{ label: 'main', title: '', focused: false, visible: true }]]);
  });
});

describe('windowing payloads', () => {
  it('keeps well-formed windows and defaults missing fields', () => {
    expect(parseWindows('x')).toEqual([]);
    expect(parseWindows([null, { title: 'no label' }, { label: 'a', title: 3, focused: 'yes', visible: false }])).toEqual([{ label: 'a', title: '', focused: false, visible: false }]);
  });

  it('keeps well-formed tasks, clamps progress and defaults unknown kinds', () => {
    expect(parseTasks({})).toEqual([]);
    expect(parseTasks([{ id: 'x' }, { id: 'a', label: 'A', kind: 'weird', progress: 7, stoppable: 'no' }, { id: 'b', label: 'B', kind: 'download', progress: -1, window: 'w', startedAt: 5, stoppable: true }])).toEqual([
      { id: 'a', label: 'A', kind: 'other', window: null, progress: 1, stoppable: false, startedAt: 0 },
      { id: 'b', label: 'B', kind: 'download', window: 'w', progress: 0, stoppable: true, startedAt: 5 },
    ]);
    expect(parseTasks([{ id: 'n', label: 'N', progress: Number.NaN }])[0]?.progress).toBeNull();
  });

  it('reads open requests, counting names when the count is missing', () => {
    expect(parseOpenAsk(null)).toBeNull();
    expect(parseOpenAsk({ count: 0, names: [] })).toBeNull();
    expect(parseOpenAsk({ names: ['a', 1, 'b'] })).toEqual({ count: 2, names: ['a', 'b'] });
    expect(parseOpenAsk({ count: 3.7, names: 'x' })).toEqual({ count: 3, names: [] });
  });
});

describe('provideWindowingBackend', () => {
  afterEach(() => {
    delete (window as unknown as Record<string, unknown>)['__TAURI_INTERNALS__'];
    TestBed.resetTestingModule();
  });

  it('uses the mock in a plain browser, also without the provider (root default)', async () => {
    TestBed.configureTestingModule({ providers: provideTinyBackend() });
    const root = TestBed.inject(WindowingBackend);
    expect(await root.label()).toBe('main');
    TestBed.resetTestingModule();
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), provideWindowingBackend()] });
    expect(await TestBed.inject(WindowingBackend).restoreWindows()).toBe(0);
    expect(new MockWindowingBackend()).toBeInstanceOf(MockWindowingBackend);
  });

  it('reports failed commands to the FailureReporter', async () => {
    const backendFailed = vi.fn();
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), provideWindowingBackend(), { provide: FailureReporter, useValue: { backendFailed, panelFailed: vi.fn() } }] });
    await expect(TestBed.inject(WindowingBackend).stopBackgroundTask('nope')).rejects.toBeDefined();
    expect(backendFailed).toHaveBeenCalledWith('stopBackgroundTask', expect.anything());
  });

  it('uses Tauri inside the desktop shell', () => {
    (window as unknown as Record<string, unknown>)['__TAURI_INTERNALS__'] = {};
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), provideWindowingBackend()] });
    expect(TestBed.inject(WindowingBackend)).not.toBeInstanceOf(MockWindowingBackend);
    expect(typeof TestBed.inject(WindowingBackend).newWindow).toBe('function');
  });
});
