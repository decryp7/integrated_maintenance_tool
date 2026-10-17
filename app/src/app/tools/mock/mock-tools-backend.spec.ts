import { CollectorTemplate } from '../../core/tools.types';
import { DEFAULT_PLAN } from '../collector/plan-model';
import { MOCK_DEFAULTS, MockToolsBackend, MockToolsOptions, mockManifest, mockXml, optionsFromSearch } from './mock-tools-backend';

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
    expect(optionsFromSearch('?mock-safe=1&mock-crash=1&mock-elevated=1&mock-os=mac&mock-denied=1&mock-latency=5')).toEqual({
      safeMode: true,
      previousCrash: true,
      elevated: true,
      platform: 'macos',
      denied: true,
      latency: 5,
    });
    expect(optionsFromSearch('?mock-os=linux').platform).toBe('linux');
    expect(optionsFromSearch('?mock-os=windows').platform).toBe('windows');
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
    expect((await b.perfmonPresets()).map((p) => p.id)).toEqual(['cpu', 'memory', 'disk']);
  });

  it('validates templates like the engine', async () => {
    const b = make();
    await expect(b.perfmonGenerate({ ...template, name: ' ' })).rejects.toThrow('name');
    await expect(b.perfmonGenerate({ ...template, sampleIntervalSecs: 0 })).rejects.toThrow('interval');
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
    await expect(make({ platform: 'macos' }).perfmonInstall(template)).rejects.toThrow('only be managed on Windows');
    await expect(make({ platform: 'macos' }).perfmonControl('x', 'query')).rejects.toThrow('only be managed on Windows');
    await expect(make({ platform: 'macos' }).restartElevated([])).rejects.toThrow('only available on Windows');
    await expect(make({ denied: true }).perfmonInstall(template)).rejects.toThrow('Access is denied');
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
    await expect(b.monitorStart({ targets: [], intervalMs: 500, cpu: 'machine', counters: [], includeChildren: false })).rejects.toThrow('target');
    const cfg = { targets: [{ name: 'a' }], intervalMs: 500, cpu: 'machine' as const, counters: [], includeChildren: false };
    expect(await b.monitorStart(cfg)).toBe('/demo/monitor/run-1');
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
    const r = await b.collectDiagnostics(JSON.stringify({ ...plan, logs: [{ path: 'C:\\L' }], msinfo: true }), '/out');
    expect(r.bundle).toBe('/out/imt-diagnostics.imtz');
    expect(r.manifest.items[0].origin).toBe('C:\\L');
    expect(r.manifest.skipped).toEqual([]);
    expect(mockManifest(DEFAULT_PLAN).skipped).toHaveLength(1);
    expect(mockManifest(DEFAULT_PLAN).items[0].origin).toBe('C:\\logs');
    await expect(b.collectDiagnostics('{nope', '/out')).rejects.toThrow('not valid JSON');
    await expect(b.collectDiagnostics('{}', ' ')).rejects.toThrow('output folder');
  });

  it('reports lifecycle state from its options', async () => {
    const b = make({ safeMode: true, previousCrash: true, elevated: true });
    expect(await b.elevation()).toEqual({ elevated: true, canRestartElevated: true, platform: 'windows' });
    expect(await make({ platform: 'linux' }).elevation()).toEqual({ elevated: false, canRestartElevated: false, platform: 'linux' });
    expect(await b.appStatus()).toMatchObject({ safeMode: true, previousCrash: true, sessionAvailable: true });
    expect(await b.lastSession()).toBe('/demo/session/last-session.imtproj');
    const c = make();
    expect(await c.appStatus()).toMatchObject({ sessionAvailable: false });
    expect(await c.lastSession()).toBeNull();
    await c.saveSession({ a: 1 });
    expect(await c.lastSession()).toContain('last-session');
    expect((await c.appStatus()).sessionAvailable).toBe(true);
    await c.restartElevated(['/s']);
    await c.restartApp(true, ['/s']);
    await c.pluginReset('p');
    expect(await c.pluginSetEnabled('p', false)).toBe(true);
    expect(await c.pluginSetEnabled('p', true)).toBe(true);
    expect(c.calls).toEqual(['saveSession', 'restartElevated:/s', 'restartApp:true:/s', 'pluginReset:p', 'pluginSetEnabled:p:false', 'pluginSetEnabled:p:true']);
  });

  it('round-trips projects and the session', async () => {
    const b = make();
    await b.saveProject('/p/a.imtproj', 'A', { bookmarks: [1] });
    const opened = await b.openProject('/p/a.imtproj');
    expect(opened).toMatchObject({ name: 'A', ui: { bookmarks: [1] }, missing: [] });
    expect((await b.openProject('/p/missing.imtproj').catch(() => null))).toBeNull();
    await b.saveProject('/p/missing.imtproj', 'M', {});
    expect((await b.openProject('/p/missing.imtproj')).missing).toHaveLength(1);
    await b.saveSession({ q: 1 });
    expect((await b.openProject('/x/last-session.imtproj')).ui).toEqual({ q: 1 });
    await expect(b.openProject('/nope.imtproj')).rejects.toThrow('not an IMT project');
    expect(await b.takePendingOpen()).toEqual([]);
    (await b.onOpenRequested())();
  });

  it('rejects open_project when no workspace summary source is wired', async () => {
    const b = new MockToolsBackend({ ...MOCK_DEFAULTS });
    await b.saveProject('/p.imtproj', 'P', {});
    await expect(b.openProject('/p.imtproj')).rejects.toThrow('no workspace summary');
  });
});
