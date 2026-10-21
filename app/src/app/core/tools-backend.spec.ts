import { TestBed } from '@angular/core/testing';
import { provideTinyBackend } from '../../testing/helpers';
import { MockToolsBackend } from '../tools/mock/mock-tools-backend';
import { FailureReporter } from './failure-reporter';
import { CollectBackend, EventLogBackend, LifecycleBackend, PerfmonBackend, PluginPackagesBackend, ProjectBackend, provideToolsBackend, TauriPluginPackagesBackend, TauriToolsBackend, ToolCatalogBackend, ToolsBackend } from './tools-backend';
import { MockPluginPackagesBackend } from '../plugins-panel/mock/mock-plugin-packages-backend';
import { CollectorTemplate } from './tools.types';
import { at } from '../../testing/must';

const invoke = vi.hoisted(() => vi.fn(async (..._args: unknown[]) => 'ok'));
const template: CollectorTemplate = { name: 'T', processes: [], presets: [], extraCounters: [], sampleIntervalSecs: 5, format: 'csv', outputDir: 'x', maxSizeMb: null, durationSecs: null };

describe('TauriToolsBackend', () => {
  it('maps every method to its Tauri command and arguments', async () => {
    const listen = vi.fn(async (_e: string, _h: () => void) => () => undefined);
    const b = new TauriToolsBackend(invoke as never, listen);
    const config = { targets: [{ name: 'a' }], intervalMs: 1000, cpu: 'machine' as const, counters: [], includeChildren: false };
    const calls: [() => Promise<unknown>, string, unknown?][] = [
      [() => b.toolsList(), 'tools_list'],
      [() => b.toolInvoke('perfmon.sets', 'outputs', { name: 'T' }), 'tool_invoke', { tool: 'perfmon.sets', op: 'outputs', args: { name: 'T' } }],
      [() => b.toolInvoke('elevation', 'info'), 'tool_invoke', { tool: 'elevation', op: 'info', args: {} }],
      [() => b.perfmonPresets(), 'tool_invoke', { tool: 'perfmon.templates', op: 'presets', args: {} }],
      [() => b.perfmonGenerate(template, '/x.xml'), 'tool_invoke', { tool: 'perfmon.templates', op: 'generate', args: { template, xmlPath: '/x.xml' } }],
      [() => b.perfmonGenerate(template, '/x.xml', true), 'tool_invoke', { tool: 'perfmon.templates', op: 'generate', args: { template, xmlPath: '/x.xml', overwrite: true } }],
      [() => b.perfmonInstall(template), 'tool_invoke', { tool: 'perfmon.sets', op: 'install', args: { template } }],
      [() => b.perfmonControl('T', 'start'), 'tool_invoke', { tool: 'perfmon.sets', op: 'control', args: { name: 'T', action: 'start' } }],
      [() => b.perfmonOutputs('T'), 'tool_invoke', { tool: 'perfmon.sets', op: 'outputs', args: { name: 'T' } }],
      [() => b.monitorStart(config, '/d'), 'tool_invoke', { tool: 'process-monitor', op: 'start', args: { config, dir: '/d' } }],
      [() => b.monitorStop(), 'tool_invoke', { tool: 'process-monitor', op: 'stop', args: {} }],
      [() => b.monitorStatus(), 'tool_invoke', { tool: 'process-monitor', op: 'status', args: {} }],
      [() => b.collectDefaultPlan(), 'tool_invoke', { tool: 'diagnostics', op: 'default-plan', args: {} }],
      [() => b.collectDiagnostics('{}', '/o'), 'tool_invoke', { tool: 'diagnostics', op: 'collect', args: { plan: '{}', outDir: '/o' } }],
      [() => b.collectDiagnostics('{}', '/o', true), 'tool_invoke', { tool: 'diagnostics', op: 'collect', args: { plan: '{}', outDir: '/o', overwrite: true } }],
      [() => b.eventLogChannels(), 'tool_invoke', { tool: 'event-log-follower', op: 'channels', args: {} }],
      [() => b.eventLogValidate({ channels: ['System'], backfill: '1h' }), 'tool_invoke', { tool: 'event-log-follower', op: 'validate', args: { channels: ['System'], backfill: '1h' } }],
      [() => b.eventLogStart({ channels: ['System'], resumeFromBookmark: false, dir: '/e' }), 'tool_invoke', { tool: 'event-log-follower', op: 'start', args: { channels: ['System'], resumeFromBookmark: false, dir: '/e' } }],
      [() => b.eventLogStop(), 'tool_invoke', { tool: 'event-log-follower', op: 'stop', args: {} }],
      [() => b.eventLogPause(), 'tool_invoke', { tool: 'event-log-follower', op: 'pause', args: {} }],
      [() => b.eventLogResume(), 'tool_invoke', { tool: 'event-log-follower', op: 'resume', args: {} }],
      [() => b.eventLogStatus(), 'tool_invoke', { tool: 'event-log-follower', op: 'status', args: {} }],
      [() => b.elevation(), 'elevation'],
      [() => b.restartElevated(true), 'restart_elevated', { reopenSession: true }],
      [() => b.appStatus(), 'app_status'],
      [() => b.restartApp(true, false), 'restart_app', { safeMode: true, reopenSession: false }],
      [() => b.pluginReset('p'), 'plugin_reset', { id: 'p' }],
      [() => b.pluginSetEnabled('p', false), 'plugin_set_enabled', { id: 'p', enabled: false }],
      [() => b.saveSession({ a: 1 }), 'save_session', { ui: { a: 1 } }],
      [() => b.lastSession(), 'last_session'],
      [() => b.saveProject('/p', 'n', { a: 1 }), 'save_project', { path: '/p', name: 'n', ui: { a: 1 } }],
      [() => b.openProject('/p'), 'open_project', { path: '/p' }],
      [() => b.takePendingOpen(), 'take_pending_open'],
    ];
    for (const [call, name, args] of calls) {
      invoke.mockClear();
      await call();
      expect(invoke).toHaveBeenCalledTimes(1);
      expect(at(invoke.mock.calls, 0)[0]).toBe(name);
      if (args) expect(at(invoke.mock.calls, 0)[1]).toEqual(args);
    }
    const handler = () => undefined;
    await b.onOpenRequested(handler);
    expect(listen.mock.calls.map((c) => c[0])).toEqual(['imt://open']);
    expect('setLive' in b || 'onWorkspaceChanged' in b).toBe(false);
  });
});

describe('TauriPluginPackagesBackend', () => {
  it('maps every method to its plugin_package command and arguments', async () => {
    const b = new TauriPluginPackagesBackend(invoke as never);
    const calls: [() => Promise<unknown>, string, unknown?][] = [
      [() => b.listPackages(), 'plugin_packages_list'],
      [() => b.readPackage('/p/a.imtplugin'), 'plugin_package_inspect', { path: '/p/a.imtplugin' }],
      [() => b.installPackage('/p/a.imtplugin', 'ab', ['network']), 'plugin_package_install', { path: '/p/a.imtplugin', digest: 'ab', approved: ['network'] }],
      [() => b.removePackage('x'), 'plugin_package_remove', { id: 'x' }],
      [() => b.setPackageEnabled('x', true), 'plugin_package_set_enabled', { id: 'x', enabled: true }],
      [() => b.setPackagePermissions('x', []), 'plugin_package_set_permissions', { id: 'x', granted: [] }],
    ];
    for (const [call, name, args] of calls) {
      invoke.mockClear();
      await call();
      expect(invoke).toHaveBeenCalledTimes(1);
      expect(at(invoke.mock.calls, 0)[0]).toBe(name);
      if (args) expect(at(invoke.mock.calls, 0)[1]).toEqual(args);
    }
    expect(new TauriPluginPackagesBackend()).toBeInstanceOf(PluginPackagesBackend);
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
    expect(TestBed.inject(EventLogBackend)).toBe(full);
    expect(TestBed.inject(LifecycleBackend)).toBe(full);
    expect(TestBed.inject(ProjectBackend)).toBe(full);
    expect(TestBed.inject(ToolCatalogBackend)).toBe(full);
    // The mock gets the workspace summary of the app's backend for open_project.
    await full.saveProject('/demo/p.imtz', 'P', {});
    expect((await full.openProject('/demo/p.imtz')).summary.sources.length).toBeGreaterThan(0);
    expect(full).toBeDefined();
    expect(new MockToolsBackend()).toBeInstanceOf(MockToolsBackend);
    // Plugin packages: the mock in a browser.
    expect((await TestBed.inject(PluginPackagesBackend).listPackages()).plugins.length).toBe(1);
    expect(new MockPluginPackagesBackend()).toBeInstanceOf(PluginPackagesBackend);
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
    expect(typeof TestBed.inject(PluginPackagesBackend).listPackages).toBe('function');
  });
});
