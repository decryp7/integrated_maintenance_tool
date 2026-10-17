import { T0, SEC, tinyDataset } from '../../../testing/helpers';
import { MockBackend } from './mock-backend';
import { mockAnnotations, MOCK_ANNOTATION_OWNER } from './mock-annotations';
import { effectiveMarkers, MOCK_PLUGIN_KINDS } from './mock-plugin-markers';
import { MockRecord } from './mock-data';

const key = (r: MockRecord) => `${r.fields['MachineName']} · ${r.fields['ProcessId']}`;

describe('mockAnnotations', () => {
  const records = tinyDataset().records;

  it('derives uptime spans, machine-wide downtime and crash points like the Windows annotators', () => {
    const all = mockAnnotations(records, key);
    expect(all.every((a) => a.owner === MOCK_ANNOTATION_OWNER && a.kind.startsWith(`${MOCK_ANNOTATION_OWNER}/`))).toBe(true);
    const crash = all.find((a) => a.kind.endsWith('/app-crash'))!;
    expect([crash.from, crash.to, crash.instance, crash.severity]).toEqual([T0 + 240 * SEC, T0 + 240 * SEC, 'WEB01 · 100', 'critical']);
    const down = all.find((a) => a.kind.endsWith('/downtime'))!;
    expect([down.from, down.to, down.instance, down.on]).toEqual([T0 + 240 * SEC, T0 + 300 * SEC, null, { field: 'MachineName', value: 'WEB01' }]);
    expect(down.evidenceQuery.filters).toEqual([{ field: 'MachineName', op: 'eq', value: 'WEB01' }]);
    const running = all.filter((a) => a.kind.endsWith('/service-running'));
    expect(running.map((a) => [a.instance, a.severity])).toEqual([
      ['WEB01 · 100', 'warning'],
      ['WEB02 · 300', 'info'],
      ['WEB01 · 200', 'warning'],
    ]);
    // By start time.
    expect(all.map((a) => a.from)).toEqual([...all.map((a) => a.from)].sort((a, b) => a - b));
  });

  it('filters by overlap with the requested range and by instance', () => {
    const early = mockAnnotations(records, key, { to: T0 + 10 * SEC });
    expect(early.map((a) => a.instance)).toEqual(['WEB01 · 100']);
    expect(mockAnnotations(records, key, { from: T0 + 1000 * SEC })).toEqual([]);
    const web02 = mockAnnotations(records, key, { instance: 'WEB02 · 300' });
    expect(web02.every((a) => a.instance === 'WEB02 · 300' || a.instance === null)).toBe(true);
    expect(web02.some((a) => a.instance === null)).toBe(true);
    expect(mockAnnotations([], key)).toEqual([]);
  });

  it('is served by the mock backend, which can also fail on demand', async () => {
    const b = new MockBackend({ latency: false, dataset: tinyDataset, fail: { annotations: 1 } });
    await expect(b.annotations({})).rejects.toContain('Simulated annotations failure');
    expect((await b.annotations({})).length).toBeGreaterThan(0);
    const metrics = await b.pluginEvents();
    expect(metrics[0]).toMatchObject({ plugin: MOCK_ANNOTATION_OWNER, dropped: 0 });
  });
});

describe('effectiveMarkers', () => {
  it('puts profile rules first, then plugin kinds, and applies overrides', () => {
    const profile = {
      marker: [{ kind: 'mine', label: 'Mine', when: [{ field: '@level', op: 'eq' as const, value: 'Error' }] }],
      marker_override: [
        { kind: 'net.decryptology.windows/service-stop', enabled: false },
        { kind: 'net.decryptology.windows/app-crash', color: '#00F', badge: 'X', shape: 'square', label: 'Crash!', description: 'restyled', icon: 'bug' },
        { kind: 'not.installed/x', enabled: false },
      ],
    };
    const m = effectiveMarkers(profile);
    expect(m.map((x) => [x.info.kind, x.info.owner])).toEqual([
      ['mine', 'profile'],
      ['net.decryptology.windows/app-crash', 'net.decryptology.windows'],
    ]);
    expect(m[1].info).toMatchObject({ color: '#00F', badge: 'X', shape: 'square', label: 'Crash!', description: 'restyled', icon: 'bug', role: 'crash', lifecycle: true });
    expect(m[1].info.when).toEqual([{ field: '@level', op: 'eq', value: 'Fatal' }]);
    expect(m[0].info.when).toEqual([]);
    expect(effectiveMarkers({}, []).length).toBe(0);
    expect(effectiveMarkers({}).length).toBe(MOCK_PLUGIN_KINDS.length);
    // A restyle without values keeps the plugin's style.
    expect(effectiveMarkers({ marker_override: [{ kind: 'net.decryptology.windows/app-crash' }] })[0].info.color).toBe('#E5484D');
  });

  it('marks records with plugin kinds in the mock backend', async () => {
    const b = new MockBackend({ latency: false, dataset: tinyDataset });
    const s = await b.workspaceSummary();
    expect(s.markerKinds.filter((k) => k.owner !== 'profile').map((k) => k.role)).toEqual(['crash', 'stop']);
    const page = await b.queryPage({ query: { levels: ['Fatal'], text: '', regex: false, caseSensitive: false, filters: [], groupBy: [], groups: [], markers: [] }, offset: 0, limit: 5, descending: false });
    expect(page.rows[0].markers).toContain('net.decryptology.windows/app-crash');
  });
});
