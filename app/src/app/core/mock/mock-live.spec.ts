import { settle, tinyDataset } from '../../../testing/helpers';
import { LiveAlert, TailReport } from '../api.types';
import { EMPTY_QUERY } from '../workspace.store';
import { MockBackend } from './mock-backend';
import { emptyReport, LEGACY_LIVE, MockAlertMutes, MockLiveSim, mockFamily, mockGroupOf, mockResolveLiveOptions, mockResolveWindow, mockWatchMode } from './mock-live';
import { mockDurationMs, validateLiveSection } from './mock-live-profile';
import { at } from '../../../testing/must';
import { mockJobs } from '../../jobs/mock-jobs';

describe('mock live tail helpers', () => {
  it('derives log families like the engine', () => {
    for (const [name, fam] of [
      ['app.log', 'app.log'],
      ['app.log.1', 'app.log'],
      ['app.log.2.gz', 'app.log'],
      ['log-20261017.txt', 'log.txt'],
      ['log-20261017_001.txt', 'log.txt'],
      ['app.2026-10-17.0.log', 'app.log'],
      ['app.1.log', 'app.log'],
      ['log_001.txt', 'log.txt'],
      ['web01.log', 'web01.log'],
      ['20261017.log', '20261017.log'],
      ['x.zip', 'x'],
      ['.gz', '.gz'],
    ] as [string, string][]) {
      expect(mockFamily(name)).toBe(fam);
    }
    expect(mockGroupOf('/logs/app.log.1')).toBe('/logs/app.log');
    expect(mockGroupOf('C:\\logs\\app.log.1')).toBe('C:\\logs/app.log');
    expect(mockGroupOf('app.log.1')).toBe('app.log');
  });

  it('resolves a live window into from (and keeps a later from)', () => {
    const q = { ...EMPTY_QUERY, window: { kind: 'live' as const, lastMs: 1000 } };
    expect(mockResolveWindow(q, 10_000)).toEqual({ ...EMPTY_QUERY, from: 9_000 });
    expect(mockResolveWindow({ ...q, from: 9_500 }, 10_000).from).toBe(9_500);
    expect(mockResolveWindow(EMPTY_QUERY, 1)).toBe(EMPTY_QUERY);
    expect(mockResolveWindow(q, null).from).toBeGreaterThan(Date.now() - 2000);
  });

  it('parses durations and validates [live] with the engine messages', () => {
    expect(mockDurationMs('60s')).toBe(60_000);
    expect(mockDurationMs('1.5h')).toBe(5_400_000);
    expect(mockDurationMs('250')).toBe(250);
    expect(mockDurationMs('5 weeks')).toBeNull();
    expect(mockDurationMs('soon')).toBeNull();
    expect(mockDurationMs('0s')).toBeNull();
    const lineOf = () => 3;
    const check = (live: unknown) => validateLiveSection({ live } as never, lineOf)?.error ?? null;
    expect(validateLiveSection({}, lineOf)).toBeNull();
    expect(check({ alerts: ['error_spike', 'Memory-Growth'], clock: 'wall', rule: [{ name: 'a', when: 'level = "Error"', threshold: 2, window: '1m', severity: 'critical' }] })).toBeNull();
    expect(check('x')).toBe('live: expected a table');
    expect(check({ alerts: 'x' })).toContain('expected a list');
    expect(check({ alerts: ['bogus'] })).toContain('unknown detector "bogus"');
    expect(check({ clock: 'moon' })).toBe('live.clock must be "data" or "wall", not "moon"');
    expect(check({ rule: 'x' })).toContain('expected [[live.rule]]');
    expect(check({ rule: [{ name: 'a', when: '' }] })).toBe('live.rule "a": `when` is empty');
    expect(check({ rule: [{ when: 'level = = 3' }] })).toMatch(/^live\.rule "rule 1": /);
    expect(check({ rule: [{ name: 'a', when: 'level = "Error"', threshold: -1 }] })).toContain('threshold');
    expect(check({ rule: [{ name: 'a', when: 'level = "Error"', window: 'soon' }] })).toBe('live.rule "a".window: "soon" is not a duration (e.g. "60s", "5m", "1h")');
    expect(check({ rule: [{ name: 'a', when: 'level = "Error"', severity: 'meh' }] })).toBe('live.rule "a".severity must be critical, warning or info');
    expect(check({ cooldown: 'later' })).toContain('live.cooldown');
    expect(mockWatchMode(false, undefined)).toBe('off');
    expect(mockWatchMode(true, { watch: 'polling' })).toBe('polling');
    expect(mockWatchMode(true, { watch: 'auto' })).toBe('events');
  });
});

describe('MockLiveSim', () => {
  const sources = () => tinyDataset().sources.concat({ id: 5, path: '/r/export.reg', name: 'export.reg', importer: 'registry', skipped: 0 });

  it('appends at the configured rate, rotates, raises alerts and reports file status', () => {
    let next = 50;
    const sim = new MockLiveSim({ tickMs: 250, ratePerSec: 40, rotateEveryMs: 500, alertEveryMs: 500 }, () => next++);
    const list = sources();
    const last = () => undefined;
    const b1 = sim.tick(list, 1_000, last, 100);
    expect(b1.records.length).toBe(10 + 1);
    expect(b1.records.every((r) => r.ts! > 1_000 && r.source !== 5)).toBe(true);
    expect(b1.rotations).toEqual([]);
    expect(b1.alerts).toEqual([]);
    const b2 = sim.tick(list, 1_250, last, 200);
    expect(b2.rotations.length).toBe(1);
    const rot = at(b2.rotations, 0);
    expect(rot.to).toBe(`${rot.from}.1`);
    expect(rot.replacement).toMatchObject({ id: 50, path: rot.from });
    expect(b2.alerts.length).toBe(1);
    expect(b2.alerts[0]?.evidenceQuery.levels).toEqual(['Error', 'Fatal']);
    list.push(rot.replacement);
    const b3 = sim.tick(list, 1_500, last, 300);
    expect(b3.records.some((r) => r.source === 50)).toBe(true);
    expect(b3.records.some((r) => r.source === rot.sourceId)).toBe(false);
    const st = sim.status(list, () => 3, 'events', 99);
    expect(st.files.find((f) => f.sourceId === rot.sourceId)!.state).toEqual({ kind: 'rotated' });
    expect(st.files.find((f) => f.sourceId === 5)).toMatchObject({ mode: 'off', state: { kind: 'whole' } });
    expect(st.totals).toMatchObject({ files: list.length, records: 99 });
    expect(st.totals.linesPerSec).toBeGreaterThan(0);
    // Alerts cycle through kinds.
    sim.tick(list, 1_750, last, 400);
    const kinds: LiveAlert['kind'][] = [];
    for (let i = 0; i < 6; i++) kinds.push(...sim.tick(list, 2_000 + i * 250, last, 500).alerts.map((a) => a.kind));
    expect(new Set(kinds)).toEqual(new Set(['rule', 'silence', 'errorSpike']));
    sim.idle();
    expect(sim.status(list, () => 0, 'off', 0).totals.linesPerSec).toBe(0);
    expect(sim.produced).toBeGreaterThan(30);
  });

  it('copies the instance fields of the file it writes to, and follows a source opened at its end', () => {
    const sim = new MockLiveSim({ ...LEGACY_LIVE, tickMs: 100 }, () => 99);
    const list = sources();
    const prev = tinyDataset().records[0];
    sim.follow(1);
    const b = sim.tick(list, null, (id) => (id === 1 ? prev : undefined), 0);
    expect(b.records).toHaveLength(1);
    expect(b.records[0]).toMatchObject({ source: 1, message: 'Live event 1', fields: { Live: true } });
    const realistic = new MockLiveSim({ tickMs: 1000, ratePerSec: 1, rotateEveryMs: 0, alertEveryMs: 0 }, () => 99);
    const r = realistic.tick(list, 5, () => prev, 0);
    expect(r.records[0]?.fields).toMatchObject({ MachineName: 'WEB01', ProcessId: 100, Live: true });
    expect(r.records.at(-1)!.message).toMatch(/^Process stats CPU/);
    expect(realistic.tick([at(list, 2)], 5, () => undefined, 0).records).toEqual([]);
    expect(emptyReport().idsChangedFrom).toBeNull();
  });
});

describe('MockBackend live tail', () => {
  it('emits realistic reports and alerts, rotates, records set_live calls and reports tail status', async () => {
    const b = new MockBackend({ latency: false, dataset: tinyDataset, liveMs: 20, liveRate: 200, liveRotateMs: 40, liveAlertMs: 40, exposeLiveCalls: true });
    const reports: (TailReport | null)[] = [];
    const alerts: LiveAlert[] = [];
    const off = await b.onWorkspaceChanged((r) => reports.push(r));
    const offAlerts = await b.onLiveAlert((a) => alerts.push(a));
    const before = (await b.workspaceSummary()).total;
    expect(await b.setLive(true, { watch: 'polling' })).toBe(2);
    expect((globalThis as Record<string, unknown>)['__debriefMockLiveCalls']).toBe(b.liveCalls);
    await vi.waitFor(() => expect(reports.length).toBeGreaterThanOrEqual(3));
    await vi.waitFor(() => expect(alerts.length).toBeGreaterThanOrEqual(1));
    await b.setLive(false);
    off();
    offAlerts();
    expect(b.liveCalls).toEqual([{ enabled: true, options: { watch: 'polling' } }, { enabled: false, options: null }]);
    const r = reports[0]!;
    expect(r.appended).toBeGreaterThan(0);
    expect(r.newestTs).not.toBeNull();
    expect(reports.some((x) => x!.rotated.length > 0 && x!.added.length > 0)).toBe(true);
    // Rotations report the per-file state changes: the old file rotated, the new one tailing.
    const rotation = reports.find((x) => x!.rotated.length > 0)!;
    expect(rotation.fileStates).toEqual([
      { sourceId: at(rotation.rotated, 0).sourceId, state: { kind: 'rotated' } },
      { sourceId: rotation.added[0], state: { kind: 'tailing' } },
    ]);
    // Alerts carry distinct stable ids.
    expect(alerts.every((a) => a.id.startsWith(`${a.kind}:`))).toBe(true);
    expect(new Set(alerts.map((a) => a.id)).size).toBe(alerts.length);
    const summary = await b.workspaceSummary();
    expect(summary.total).toBeGreaterThan(before);
    expect(summary.sources.some((s) => s.path.endsWith('.1'))).toBe(true);
    const groups = new Map<string, number>();
    for (const s of summary.sources) groups.set(s.group, (groups.get(s.group) ?? 0) + 1);
    expect([...groups.values()].some((n) => n > 1)).toBe(true);
    const status = await b.tailStatus();
    expect(status.files.some((f) => f.state.kind === 'rotated')).toBe(true);
    expect(status.files.every((f) => f.mode === 'off')).toBe(true);
    await b.setLive(true, { watch: 'polling' });
    expect((await b.tailStatus()).files[0]?.mode).toBe('polling');
    await b.setLive(false);
  });

  it('reports the live state with resolved options, like live_state', async () => {
    const b = new MockBackend({ latency: false, dataset: tinyDataset, liveMs: 100_000 });
    expect(await b.liveState()).toEqual({ enabled: false, options: mockResolveLiveOptions(undefined) });
    await b.setLive(true, { watch: 'polling', pollIntervalMs: 5, detectEveryMs: 1e9 });
    const on = await b.liveState();
    expect(on.enabled).toBe(true);
    expect(on.options).toMatchObject({ watch: 'polling', pollIntervalMs: 100, detectEveryMs: 600_000, carryFlushMs: 500, deletedFiles: 'keep' });
    await b.setLive(false);
    expect((await b.liveState()).enabled).toBe(false);
    expect(mockResolveLiveOptions({ detectEveryMs: -3 })).toMatchObject({ detectEveryMs: 0 });
  });

  it('mutes alerts by kind or rule until they expire', () => {
    let now = 1_000;
    const m = new MockAlertMutes(() => now);
    m.mute({ kind: 'crash' }, 2_000);
    m.mute({ rule: 'pay' }, 3_000);
    expect(m.covers({ kind: 'crash', rule: null })).toBe(true);
    expect(m.covers({ kind: 'rule', rule: 'pay' })).toBe(true);
    expect(m.covers({ kind: 'rule', rule: 'other' })).toBe(false);
    expect(m.covers({ kind: 'rule', rule: null })).toBe(false);
    now = 2_000;
    expect(m.covers({ kind: 'crash', rule: null })).toBe(false);
    m.unmute({ rule: 'pay' });
    expect(m.covers({ kind: 'rule', rule: 'pay' })).toBe(false);
    expect(new MockAlertMutes().covers({ kind: 'silence', rule: null })).toBe(false);
  });

  it('keeps the legacy one-event-per-tick simulation and the synthetic mode', async () => {
    const b = new MockBackend({ latency: false, dataset: tinyDataset, liveMs: 10 });
    const reports: (TailReport | null)[] = [];
    await b.onWorkspaceChanged((r) => reports.push(r));
    await b.setLive(true);
    await vi.waitFor(() => expect(reports.length).toBeGreaterThanOrEqual(2));
    await b.setLive(false);
    expect(reports[0]).toMatchObject({ appended: 1, alertsRaised: 0 });
    const page = await b.queryPage({ query: { ...EMPTY_QUERY, text: 'Live event' }, offset: 0, limit: 5, descending: false });
    expect(page.rows[0]?.message).toBe('Live event 1');
    const synthetic = new MockBackend({ latency: false, syntheticTotal: 100, liveMs: 10 });
    const s: (TailReport | null)[] = [];
    await synthetic.onWorkspaceChanged((r) => s.push(r));
    await synthetic.setLive(true);
    await vi.waitFor(() => expect(s.length).toBeGreaterThanOrEqual(1));
    await synthetic.setLive(false);
    expect((await synthetic.workspaceSummary()).total).toBeGreaterThan(100);
  });

  it('resolves live windows in every query and opens files at their end', async () => {
    const b = new MockBackend({ latency: false, dataset: tinyDataset });
    const all = (await b.queryPage({ query: EMPTY_QUERY, offset: 0, limit: 100, descending: false })).total;
    const windowed = { ...EMPTY_QUERY, window: { kind: 'live' as const, lastMs: 150_000 } };
    const page = await b.queryPage({ query: windowed, offset: 0, limit: 100, descending: false });
    expect(page.total).toBeLessThan(all);
    expect(page.rows.every((r) => r.ts !== null && r.ts >= page.rows.at(-1)!.ts! - 150_000)).toBe(true);
    const tl = await b.queryTimeline({ query: windowed, buckets: 20, alwaysShowLifecycle: false });
    expect(tl.to - tl.from).toBeLessThanOrEqual(150_000);
    expect((await b.queryFacet({ query: windowed, field: '@level', limit: 10 })).total).toBe(page.total);
    const chart = await b.queryChart({ query: windowed, chartId: null, def: { id: 'x', title: 'x', y: 'count()', bucket: 'auto', splitBy: [], type: 'bar', unit: '' }, buckets: 10 });
    expect(chart.to - chart.from).toBeLessThanOrEqual(150_000);
    expect((await mockJobs().wait(await b.exportEvents({ query: windowed, format: 'csv', path: '/x.csv' }))).message).toBe(`Exported ${page.total} events.`);
    await mockJobs().wait(await b.saveProfile('name = "w"\n[live]\nclock = "wall"\n'));
    expect((await b.queryPage({ query: windowed, offset: 0, limit: 1, descending: false })).total).toBe(0);

    const summary = await b.openPathsLive(['/var/log/new.log', '/var/log/new.log'], true);
    const added = summary.sources.filter((s) => s.path === '/var/log/new.log');
    expect(added).toHaveLength(1);
    expect(added[0]).toMatchObject({ records: 0, group: '/var/log/new.log', name: 'new.log' });
    expect((await b.openPathsLive(['/var/log/history.log'], false)).sources.find((s) => s.path === '/var/log/history.log')!.records).toBeGreaterThan(0);
    expect((await b.validateProfile('[live]\nclock = "moon"\n')).error).toContain('live.clock');
    await settle();
  });
});
