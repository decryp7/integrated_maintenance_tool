import { TestBed } from '@angular/core/testing';
import { provideTinyBackend } from '../../testing/helpers';
import { MockToolsBackend } from '../tools/mock/mock-tools-backend';
import { FailureReporter } from './failure-reporter';
import { CollectBackend, LifecycleBackend, PerfmonBackend, ProjectBackend, provideToolsBackend, TauriToolsBackend, ToolsBackend } from './tools-backend';
import { CollectorTemplate } from './tools.types';

const invoke = vi.hoisted(() => vi.fn(async (..._args: unknown[]) => 'ok'));
const template: CollectorTemplate = { name: 'T', processes: [], presets: [], extraCounters: [], sampleIntervalSecs: 5, format: 'csv', outputDir: 'x', maxSizeMb: null, durationSecs: null };

describe('TauriToolsBackend', () => {
  it('maps every method to its Tauri command and arguments', async () => {
    const listen = vi.fn(async (_e: string, _h: () => void) => () => undefined);
    const b = new TauriToolsBackend(invoke as never, listen);
    const config = { targets: [{ name: 'a' }], intervalMs: 1000, cpu: 'machine' as const, counters: [], includeChildren: false };
    const calls: [() => Promise<unknown>, string, unknown?][] = [
      [() => b.perfmonPresets(), 'perfmon_presets'],
      [() => b.perfmonGenerate(template, '/x.xml'), 'perfmon_generate', { template, xmlPath: '/x.xml' }],
      [() => b.perfmonInstall(template), 'perfmon_install', { template }],
      [() => b.perfmonControl('T', 'start'), 'perfmon_control', { name: 'T', action: 'start' }],
      [() => b.perfmonOutputs('T'), 'perfmon_outputs', { name: 'T' }],
      [() => b.monitorStart(config, '/d'), 'monitor_start', { config, dir: '/d' }],
      [() => b.monitorStop(), 'monitor_stop'],
      [() => b.monitorStatus(), 'monitor_status'],
      [() => b.collectDefaultPlan(), 'collect_default_plan'],
      [() => b.collectDiagnostics('{}', '/o'), 'collect_diagnostics', { plan: '{}', outDir: '/o' }],
      [() => b.elevation(), 'elevation'],
      [() => b.restartElevated(['/s']), 'restart_elevated', { reopen: ['/s'] }],
      [() => b.appStatus(), 'app_status'],
      [() => b.restartApp(true, ['/s']), 'restart_app', { safeMode: true, reopen: ['/s'] }],
      [() => b.pluginReset('p'), 'plugin_reset', { id: 'p' }],
      [() => b.pluginSetEnabled('p', false), 'plugin_set_enabled', { id: 'p', enabled: false }],
      [() => b.saveSession({ a: 1 }), 'save_session', { ui: { a: 1 } }],
      [() => b.lastSession(), 'last_session'],
      [() => b.saveProject('/p', 'n', { a: 1 }), 'save_project', { path: '/p', name: 'n', ui: { a: 1 } }],
      [() => b.openProject('/p'), 'open_project', { path: '/p' }],
      [() => b.setLive(true), 'set_live', { enabled: true }],
      [() => b.takePendingOpen(), 'take_pending_open'],
    ];
    for (const [call, name, args] of calls) {
      invoke.mockClear();
      await call();
      expect(invoke).toHaveBeenCalledTimes(1);
      expect(invoke.mock.calls[0][0]).toBe(name);
      if (args) expect(invoke.mock.calls[0][1]).toEqual(args);
    }
    const handler = () => undefined;
    await b.onOpenRequested(handler);
    await b.onWorkspaceChanged(handler);
    expect(listen.mock.calls.map((c) => c[0])).toEqual(['imt://open', 'imt://workspace-changed']);
  });
});

describe('provideToolsBackend', () => {
  afterEach(() => {
    delete (window as unknown as Record<string, unknown>)['__TAURI_INTERNALS__'];
    TestBed.resetTestingModule();
  });

  it('uses the mock in a plain browser and exposes the narrow interfaces', async () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), provideToolsBackend()] });
    const full = TestBed.inject(ToolsBackend);
    expect(TestBed.inject(PerfmonBackend)).toBe(full);
    expect(TestBed.inject(CollectBackend)).toBe(full);
    expect(TestBed.inject(LifecycleBackend)).toBe(full);
    expect(TestBed.inject(ProjectBackend)).toBe(full);
    // The mock gets the workspace summary of the app's backend for open_project.
    await full.saveProject('/demo/p.imtproj', 'P', {});
    expect((await full.openProject('/demo/p.imtproj')).summary.sources.length).toBeGreaterThan(0);
    expect(full).toBeDefined();
    expect(new MockToolsBackend()).toBeInstanceOf(MockToolsBackend);
  });

  it('reports failed commands to the FailureReporter', async () => {
    const backendFailed = vi.fn();
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), provideToolsBackend(), { provide: FailureReporter, useValue: { backendFailed, panelFailed: vi.fn() } }] });
    const tools = TestBed.inject(ToolsBackend);
    await expect(tools.openProject('/nowhere.imtproj')).rejects.toBeDefined();
    expect(backendFailed).toHaveBeenCalledWith('openProject', expect.anything());
  });

  it('uses Tauri inside the desktop shell', () => {
    (window as unknown as Record<string, unknown>)['__TAURI_INTERNALS__'] = {};
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), provideToolsBackend()] });
    expect(typeof TestBed.inject(ToolsBackend).perfmonPresets).toBe('function');
  });
});
