import { FILE_EXISTS_SUFFIX } from '../../core/api.types';
import { CollectorTemplate } from '../../core/tools.types';
import { tinyDataset } from '../../../testing/helpers';
import { MockBackend } from '../../core/mock/mock-backend';
import { DEFAULT_PLAN } from '../collector/plan-model';
import {
  MOCK_DEFAULTS,
  MockToolsBackend,
  MockToolsOptions,
  mockManifest,
  mockXml,
  optionsFromSearch,
} from './mock-tools-backend';
import { at } from '../../../testing/must';

const template: CollectorTemplate = {
  name: 'IMT',
  processes: ['svc'],
  presets: ['cpu'],
  extraCounters: ['\\Memory\\Pages/sec'],
  sampleIntervalSecs: 5,
  format: 'csv',
  outputDir: 'C:\\Logs\\',
  maxSizeMb: null,
  durationSecs: null,
};

function make(o: Partial<MockToolsOptions> = {}) {
  return new MockToolsBackend({ ...MOCK_DEFAULTS, ...o }, async () => ({ sources: [] }) as never);
}

describe('optionsFromSearch', () => {
  it('reads the mock-* switches', () => {
    expect(optionsFromSearch('')).toEqual(MOCK_DEFAULTS);
    expect(
      optionsFromSearch(
        '?mock-safe=1&mock-crash=1&mock-elevated=1&mock-os=mac&mock-denied=1&mock-latency=5',
      ),
    ).toEqual({
      safeMode: true,
      previousCrash: true,
      elevated: true,
      platform: 'macos',
      denied: true,
      latency: 5,
      toolsPlugin: 'active',
    });
    expect(optionsFromSearch('?mock-os=linux').platform).toBe('linux');
    expect(optionsFromSearch('?mock-os=windows').platform).toBe('windows');
    expect(optionsFromSearch('?mock-tools=quarantined').toolsPlugin).toBe('quarantined');
    expect(optionsFromSearch('?mock-tools=bogus').toolsPlugin).toBe('active');
  });
});

describe('MockToolsBackend tools', () => {
  it('lists the tools plugin with platform and plugin availability', async () => {
    const win = await make().toolsList();
    expect(win.map((t) => t.id)).toEqual([
      'perfmon.templates',
      'perfmon.sets',
      'process-monitor',
      'diagnostics',
      'elevation',
      'event-log-follower',
      'dumps.capture',
    ]);
    expect(
      win.filter((t) => t.id !== 'dumps.capture').every(
        (t) => t.availability.state === 'available' && t.plugin === 'net.decryptology.tools',
      ),
    ).toBe(true);
    const mac = await make({ platform: 'macos' }).toolsList();
    expect(mac[1]?.availability).toEqual({
      state: 'unsupported',
      reason: 'Performance Monitor collector sets is only available on Windows (this is macOS)',
    });
    expect(mac[0]?.availability.state).toBe('available');
    const quarantined = await make({ toolsPlugin: 'quarantined' }).toolsList();
    expect(quarantined[0]?.availability).toEqual({
      state: 'pluginUnavailable',
      reason: expect.stringContaining('repeated crashes'),
    });
    expect((await make({ toolsPlugin: 'inactive' }).toolsList())[2]?.availability).toEqual({
      state: 'pluginUnavailable',
      reason: expect.stringContaining('inactive'),
    });
    expect(await make({ toolsPlugin: 'disabled' }).toolsList()).toEqual([]);
    expect(await make({ safeMode: true }).toolsList()).toEqual([]);
  });

  it('invokes the operations it knows and refuses the rest with the reason', async () => {
    const b = make();
    expect(await b.toolInvoke('process-monitor', 'status')).toEqual({ running: false, dir: null });
    expect(((await b.toolInvoke('perfmon.templates', 'presets')) as unknown[]).length).toBe(3);
    expect(await b.toolInvoke('diagnostics', 'default-plan')).toContain('redact');
    expect(await b.toolInvoke('elevation', 'info')).toEqual({
      elevated: false,
      canRestartElevated: true,
      platform: 'windows',
    });
    await expect(b.toolInvoke('elevation', 'dance')).rejects.toThrow('has no operation "dance"');
    await expect(b.toolInvoke('nope', 'x')).rejects.toThrow(
      'no installed plugin provides the tool "nope"',
    );
    await expect(make({ safeMode: true }).toolInvoke('elevation', 'info')).rejects.toThrow(
      'no installed plugin',
    );
    await expect(
      make({ toolsPlugin: 'quarantined' }).toolInvoke('elevation', 'info'),
    ).rejects.toThrow('repeated crashes');
    await expect(make({ platform: 'linux' }).toolInvoke('perfmon.sets', 'outputs')).rejects.toThrow(
      'this is Linux',
    );
  });
});

describe('MockToolsBackend', () => {
  it('generates XML with counters per process and escapes names', async () => {
    const b = make();
    const g = await b.perfmonGenerate({ ...template, name: 'A&B', processes: [] });
    expect(g.xml).toContain('<Name>A&amp;B</Name>');
    expect(g.xml).toContain('\\Process(*)\\% Processor Time');
    expect(g.commands[0]).toContain('logman import -n "A&B"');
    expect(mockXml(template)).toContain('\\Memory\\Pages/sec');
    expect((await b.perfmonGenerate(template, '/x.xml')).path).toBe('/x.xml');
    // Like the engine: the file exists now; replaced only with `overwrite`; no path writes nothing.
    await expect(b.perfmonGenerate(template, '/x.xml')).rejects.toThrow(
      `/x.xml${FILE_EXISTS_SUFFIX}`,
    );
    expect((await b.perfmonGenerate(template, '/x.xml', true)).path).toBe('/x.xml');
    await b.perfmonGenerate(template);
    await b.perfmonGenerate(template);
    expect((await b.perfmonPresets()).map((p) => p.id)).toEqual(['cpu', 'memory', 'disk']);
  });

  it('validates templates like the engine', async () => {
    const b = make();
    await expect(b.perfmonGenerate({ ...template, name: ' ' })).rejects.toThrow('name');
    await expect(b.perfmonGenerate({ ...template, sampleIntervalSecs: 0 })).rejects.toThrow(
      'interval',
    );
    await expect(b.perfmonGenerate({ ...template, maxSizeMb: 5 })).rejects.toThrow('BLG');
  });

  it('installs, controls and lists the output of a collector set', async () => {
    const b = make();
    const installed = await b.perfmonInstall(template);
    expect(installed.status).toBe('stopped');
    expect((await b.perfmonControl('IMT', 'start'))?.status).toBe('running');
    expect((await b.perfmonControl('IMT', 'query'))?.status).toBe('running');
    expect((await b.perfmonControl('IMT', 'stop'))?.status).toBe('stopped');
    expect(await b.perfmonOutputs('IMT')).toHaveLength(1);
    expect(await b.perfmonControl('IMT', 'delete')).toBeNull();
    await expect(b.perfmonControl('IMT', 'query')).rejects.toThrow('not found');
    await expect(b.perfmonOutputs('IMT')).rejects.toThrow('not found');
  });

  it('fails like Windows tooling when not on Windows or without rights', async () => {
    await expect(make({ platform: 'macos' }).perfmonInstall(template)).rejects.toThrow(
      'only be managed on Windows',
    );
    await expect(make({ platform: 'macos' }).perfmonControl('x', 'query')).rejects.toThrow(
      'only be managed on Windows',
    );
    await expect(make({ platform: 'macos' }).restartElevated(false)).rejects.toThrow(
      'only available on Windows',
    );
    await expect(make({ denied: true }).perfmonInstall(template)).rejects.toThrow(
      'Access is denied',
    );
    const ok = make({ denied: true, elevated: true });
    await ok.perfmonInstall(template);
    const b = make();
    await b.perfmonInstall(template);
    (b as unknown as { options: MockToolsOptions }).options.denied = true;
    await expect(b.perfmonControl('IMT', 'start')).rejects.toThrow('Access is denied');
    expect((await b.perfmonControl('IMT', 'query'))?.name).toBe('IMT');
  });

  it('runs one process monitor at a time', async () => {
    const b = make();
    await expect(
      b.monitorStart({
        targets: [],
        intervalMs: 500,
        cpu: 'machine',
        counters: [],
        includeChildren: false,
      }),
    ).rejects.toThrow('target');
    const cfg = {
      targets: [{ name: 'a' }],
      intervalMs: 500,
      cpu: 'machine' as const,
      counters: [],
      includeChildren: false,
    };
    expect(b.monitorConfig).toBeNull();
    expect(await b.monitorStart(cfg)).toBe('/demo/monitor/run-1');
    expect(b.monitorConfig).toEqual(cfg);
    await expect(b.monitorStart(cfg)).rejects.toThrow('already running');
    expect(await b.monitorStatus()).toEqual({ running: true, dir: '/demo/monitor/run-1' });
    expect(await b.monitorStop()).toBe('/demo/monitor/run-1');
    expect(await b.monitorStop()).toBeNull();
    expect(await b.monitorStart(cfg, '/mine')).toBe('/mine');
  });

  it('collects a bundle from a plan and rejects bad input', async () => {
    const b = make({ latency: 1 });
    const plan = JSON.parse(await b.collectDefaultPlan());
    expect(plan.name).toBe(DEFAULT_PLAN.name);
    const r = await b.collectDiagnostics(
      JSON.stringify({ ...plan, logs: [{ path: 'C:\\L' }], msinfo: true }),
      '/out',
    );
    expect(r.bundle).toBe('/out/imt-diagnostics.imtz');
    expect(r.manifest.items[0]?.origin).toBe('C:\\L');
    expect(r.manifest.skipped).toEqual([]);
    expect(mockManifest(DEFAULT_PLAN).skipped).toHaveLength(1);
    expect(mockManifest(DEFAULT_PLAN).items[0]?.origin).toBe('C:\\logs');
    // The same bundle again: refused unless confirmed.
    await expect(b.collectDiagnostics(JSON.stringify(plan), '/out')).rejects.toThrow(
      `/out/imt-diagnostics.imtz${FILE_EXISTS_SUFFIX}`,
    );
    expect((await b.collectDiagnostics(JSON.stringify(plan), '/out', true)).bundle).toBe(
      '/out/imt-diagnostics.imtz',
    );
    await expect(b.collectDiagnostics('{nope', '/out')).rejects.toThrow('not valid JSON');
    await expect(b.collectDiagnostics('{}', ' ')).rejects.toThrow('output folder');
  });

  it('reports lifecycle state from its options', async () => {
    const b = make({ safeMode: true, previousCrash: true, elevated: true });
    expect(await b.elevation()).toEqual({
      elevated: true,
      canRestartElevated: true,
      platform: 'windows',
    });
    expect(await make({ platform: 'linux' }).elevation()).toEqual({
      elevated: false,
      canRestartElevated: false,
      platform: 'linux',
    });
    expect(await b.appStatus()).toMatchObject({
      safeMode: true,
      previousCrash: true,
      sessionAvailable: true,
    });
    expect(await b.lastSession()).toBe('/demo/session/last-session.imtproj');
    const c = make();
    expect(await c.appStatus()).toMatchObject({ sessionAvailable: false });
    expect(await c.lastSession()).toBeNull();
    await c.saveSession({ a: 1 });
    expect(await c.lastSession()).toContain('last-session');
    expect((await c.appStatus()).sessionAvailable).toBe(true);
    await c.restartElevated(true);
    await c.restartApp(true, true);
    await c.pluginReset('p');
    expect(await c.pluginSetEnabled('p', false)).toBe(true);
    expect(await c.pluginSetEnabled('p', true)).toBe(true);
    expect(c.calls).toEqual([
      'saveSession',
      'restartElevated:session',
      'restartApp:true:session',
      'pluginReset:p',
      'pluginSetEnabled:p:false',
      'pluginSetEnabled:p:true',
    ]);
  });

  it('round-trips projects and the session', async () => {
    const b = make();
    await b.saveProject('/p/a.imtz', 'A', { bookmarks: [1] });
    const opened = await b.openProject('/p/a.imtz');
    expect(opened).toMatchObject({ name: 'A', ui: { bookmarks: [1] }, missing: [] });
    expect(await b.openProject('/p/missing.imtz').catch(() => null)).toBeNull();
    await b.saveProject('/p/missing.imtz', 'M', {});
    expect((await b.openProject('/p/missing.imtz')).missing).toHaveLength(1);
    await b.saveSession({ q: 1 });
    expect((await b.openProject('/x/last-session.imtproj')).ui).toEqual({ q: 1 });
    await expect(b.openProject('/nope.imtz')).rejects.toThrow('not an IMT project');
    expect(await b.takePendingOpen()).toEqual([]);
    (await b.onOpenRequested())();
  });

  it('keeps clock offsets in projects and the session, by source path, like the engine', async () => {
    const imt = new MockBackend({ latency: false, dataset: tinyDataset, pluginMarkerKinds: [], memoryStoreFeatures: true });
    const b = new MockToolsBackend(
      { ...MOCK_DEFAULTS },
      () => imt.workspaceSummary(),
      (source, offsetMs) => imt.setSourceOffset({ kind: 'manual', source, offsetMs }),
    );
    const sources = (await imt.workspaceSummary()).sources;
    const first = at(sources, 0);
    const second = at(sources, 1);
    await imt.setSourceOffset({ kind: 'manual', source: first.id, offsetMs: 2350 });
    await b.saveProject('/p/skew.imtz', 'Skew', {});
    await b.saveSession({});
    // Changed after saving: reopening restores the saved offsets (and clears ones the project did not have).
    await imt.setSourceOffset({ kind: 'manual', source: first.id, offsetMs: 0 });
    await imt.setSourceOffset({ kind: 'manual', source: second.id, offsetMs: -100 });
    const opened = await b.openProject('/p/skew.imtz');
    const offsets = (s: { sources: { path: string; offsetMs?: number }[] }) =>
      Object.fromEntries(s.sources.map((x) => [x.path, x.offsetMs ?? 0]));
    expect(offsets(opened.summary)[first.path]).toBe(2350);
    expect(offsets(opened.summary)[second.path]).toBe(0);
    await imt.setSourceOffset({ kind: 'manual', source: first.id, offsetMs: 5 });
    expect(offsets((await b.openProject('/x/last-session.imtproj')).summary)[first.path]).toBe(
      2350,
    );
    // Nothing to change: the summary is returned as is; without an offset setter offsets are left alone.
    expect(offsets((await b.openProject('/p/skew.imtz')).summary)[first.path]).toBe(2350);
    const plain = new MockToolsBackend({ ...MOCK_DEFAULTS }, () => imt.workspaceSummary());
    await plain.saveProject('/p/plain.imtz', 'P', {});
    await imt.setSourceOffset({ kind: 'manual', source: first.id, offsetMs: 7 });
    expect(offsets((await plain.openProject('/p/plain.imtz')).summary)[first.path]).toBe(7);
  });

  it('saves projects without offsets when the workspace summary fails', async () => {
    const b = new MockToolsBackend({ ...MOCK_DEFAULTS });
    await expect(b.saveProject('/p/x.imtz', 'X', {})).resolves.toEqual({
      path: '/p/x.imtz',
      name: 'X',
    });
  });

  it('rejects open_project when no workspace summary source is wired', async () => {
    const b = new MockToolsBackend({ ...MOCK_DEFAULTS });
    await b.saveProject('/p.imtz', 'P', {});
    await expect(b.openProject('/p.imtz')).rejects.toThrow('no workspace summary');
  });
});
