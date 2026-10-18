import { TestBed } from '@angular/core/testing';
import { AnalysisBackend } from '../core/analysis-backend';
import { BundleOptions, Query, Report, TileCfg } from '../core/api.types';
import { ImtBackend } from '../core/backend';
import { Logger } from '../core/logger';
import { MockBackend } from '../core/mock/mock-backend';
import { EMPTY_QUERY } from '../core/workspace.store';
import { MockAnalysisBackend, recordsFrom } from '../analysis/mock/mock-analysis-backend';
import { IMT_COMMANDS } from '../extensions/tokens';
import { MemoryLogger } from '../../testing/helpers';
import { mockDashboards, mockDashboardsError, mockTileError } from './mock/mock-dashboards';
import { mockRedactCfg, mockRedactionPreview, MockRedactor } from './mock/mock-redact';
import { MockShareBackend, mockTileQuery } from './mock/mock-share-backend';
import { ShareBackend, TauriShareBackend } from './share-backend';
import { createShareBackend, provideShare } from './share.plugin';
import { at } from '../../testing/must';

const invoke = vi.hoisted(() => vi.fn(async (..._args: unknown[]) => 'ok'));

const q: Query = { ...EMPTY_QUERY };

function tile(patch: Partial<TileCfg> & Pick<TileCfg, 'type'>): TileCfg {
  return { id: 't', title: '', x: 0, y: 0, w: 4, h: 3, ...patch };
}

function report(): Report {
  return { version: 1, id: 'r1', title: 'Outage', summary: 'mail a@b.io', createdAt: 1, updatedAt: 1, sections: [] };
}

describe('TauriShareBackend', () => {
  it('maps every method to its Tauri command', async () => {
    const b = new TauriShareBackend(invoke as never);
    const options: BundleOptions = { name: 'n', content: 'raw', query: q, redact: false, ui: null, reports: [] };
    const calls: [() => Promise<unknown>, string, unknown][] = [
      [() => b.setRedactedView(true), 'set_redacted_view', { enabled: true }],
      [() => b.previewRedaction({ sample: 's' }), 'preview_redaction', { request: { sample: 's' } }],
      [() => b.redactionSummary({ query: q }), 'redaction_summary', { request: { query: q } }],
      [() => b.dashboardTile({ tile: tile({ type: 'stat' }), query: q }), 'dashboard_tile', { request: { tile: tile({ type: 'stat' }), query: q } }],
      [() => b.exportReport({ report: report(), format: 'pdf', path: '/r.pdf', redact: true, theme: 'dark' }), 'export_report', { request: { report: report(), format: 'pdf', path: '/r.pdf', redact: true, theme: 'dark' } }],
      [() => b.saveVisual({ path: '/a.svg', kind: 'svg', data: '<svg/>' }), 'save_visual', { request: { path: '/a.svg', kind: 'svg', data: '<svg/>' } }],
      [() => b.estimateBundle(options), 'estimate_bundle', { options }],
      [() => b.createBundle(options, '/x.imtz'), 'create_bundle', { options, path: '/x.imtz' }],
      [() => b.openBundle('/x.imtz'), 'open_bundle', { path: '/x.imtz' }],
      [() => b.overview(q), 'overview', { request: { query: q } }],
    ];
    for (const [call, name, args] of calls) {
      invoke.mockClear();
      await call();
      expect(invoke.mock.calls[0]).toEqual([name, args]);
    }
  });
});

describe('provideShare', () => {
  it('provides the mock backend in a browser and registers its command batches', () => {
    const imt = new MockBackend({ latency: false });
    TestBed.configureTestingModule({
      providers: [
        { provide: Logger, useClass: MemoryLogger },
        { provide: ImtBackend, useValue: imt },
        { provide: AnalysisBackend, useValue: new MockAnalysisBackend(recordsFrom(imt)) },
        provideShare(),
      ],
    });
    expect(TestBed.inject(ShareBackend)).toBeDefined();
    expect(Array.isArray(TestBed.inject(IMT_COMMANDS))).toBe(true);
    expect(TestBed.runInInjectionContext(() => createShareBackend())).toBeDefined();
  });
});

describe('mock redaction', () => {
  it('reads the [redact] section with defaults and rejects what the engine rejects', () => {
    const cfg = mockRedactCfg({});
    expect(cfg.detectors).toContain('email');
    expect(cfg.detectors).not.toContain('guid');
    expect(cfg).toMatchObject({ mode: 'mask', fields: ['@message', '@exception'], onExport: false, rules: [] });
    expect(mockRedactCfg('[redact]\ndetectors = ["guid"]\nmode = "hash"\non_export = true').detectors).toEqual(['guid']);
    expect(() => mockRedactCfg({ redact: { detectors: ['nope'] } })).toThrow(/unknown variant/);
    expect(() => mockRedactCfg({ redact: { mode: 'blur' } })).toThrow(/unknown variant/);
    expect(() => mockRedactCfg({ redact: { rule: [{ name: '', regex: 'a' }] } })).toThrow(/needs a name/);
    expect(() => mockRedactCfg({ redact: { rule: [{ name: 'x', regex: '(' }] } })).toThrow(/invalid regex/);
  });

  it('masks, hashes and drops values, keeping keys and counting hits', () => {
    const text = 'mail a@b.io from 10.0.0.1 password=hunter2 card 4111 1111 1111 1111 bad 4111 1111 1111 1112 id 3F2504E0-4F89-11D3-9A0C-0305E82C3301';
    const mask = new MockRedactor(mockRedactCfg({ redact: { detectors: ['email', 'ipv4', 'password', 'credit-card', 'guid'] } }));
    const counts: Record<string, number> = {};
    expect(mask.apply(text, counts)).toBe('mail <email> from <ipv4> password=<password> card <card> bad 4111 1111 1111 1112 id <guid>');
    expect(counts).toEqual({ email: 1, ipv4: 1, password: 1, 'credit-card': 1, guid: 1 });
    expect(mask.hits(counts).map((h) => h.rule)).toEqual(['password', 'email', 'credit-card', 'ipv4', 'guid']);
    const hash = new MockRedactor(mockRedactCfg({ redact: { detectors: ['email'], mode: 'hash' } }), 's1');
    const [a, b, c] = hash.apply('a@b.io x@y.io a@b.io').split(' ');
    expect(a).toMatch(/^<email:[0-9a-f]{10}>$/);
    expect(a).toBe(c);
    expect(a).not.toBe(b);
    expect(new MockRedactor(hash.cfg, 's2').apply('a@b.io')).not.toBe(a);
    expect(new MockRedactor(mockRedactCfg({ redact: { detectors: ['email'], mode: 'drop' } })).apply('[a@b.io]')).toBe('[]');
    const rules = new MockRedactor(mockRedactCfg({ redact: { detectors: [], rule: [{ name: 'customer', regex: 'Customer (?P<value>\\w+)' }, { name: 'ticket', regex: 'TCK-(\\d+)', replace: 'TCK-#$1' }] } }));
    expect(rules.apply('Customer acme TCK-12')).toBe('Customer <customer> TCK-#12');
    expect(rules.empty).toBe(false);
    expect(new MockRedactor(mockRedactCfg({ redact: { detectors: [] } })).empty).toBe(true);
  });

  it('applies to the fields in scope (the template follows the message)', () => {
    const r = new MockRedactor(mockRedactCfg({ redact: { detectors: ['email'], fields: ['@message', 'User'] } }));
    expect(r.field('User', 'a@b.io')).toBe('<email>');
    expect(r.field('@template', 'a@b.io')).toBe('<email>');
    expect(r.field('Other', 'a@b.io')).toBe('a@b.io');
    expect(new MockRedactor(mockRedactCfg({ redact: { detectors: ['email'], fields: ['*'] } })).field('Other', 'a@b.io')).toBe('<email>');
  });

  it('previews against a model or TOML and reports errors', () => {
    expect(mockRedactionPreview({ redact: { detectors: ['email'] } }, 'a@b.io')).toEqual({ before: 'a@b.io', after: '<email>', hits: [{ rule: 'email', count: 1 }], error: null });
    const bad = mockRedactionPreview('[redact', 'x');
    expect(bad.after).toBe('x');
    expect(bad.error).toBeTruthy();
  });
});

describe('mock dashboards', () => {
  it('fills defaults and validates like the engine', () => {
    const dashboards = mockDashboards({ dashboard: [{ id: 'd', title: 'D', tile: [{ id: 'a', type: 'top', field: 'f', by: ['x'], limit: 3, query: 'q' }] }] });
    const d = at(dashboards, 0);
    expect(d).toEqual({ id: 'd', title: 'D', columns: 12, tile: [{ id: 'a', title: '', type: 'top', x: 0, y: 0, w: 4, h: 3, field: 'f', by: ['x'], limit: 3, query: 'q' }] });
    expect(mockDashboards({})).toEqual([]);
    expect(mockDashboards({ dashboard: [{}] })).toEqual([{ id: '', title: '', columns: 12, tile: [] }]);
    expect(mockDashboardsError([d])).toBeNull();
    expect(mockDashboardsError([d, d])).toMatch(/used twice/);
    expect(mockDashboardsError([{ ...d, id: ' ' }])).toMatch(/needs an id/);
    expect(mockDashboardsError([{ ...d, columns: 30 }])).toMatch(/columns/);
    expect(mockDashboardsError([{ ...d, tile: [at(d.tile, 0), at(d.tile, 0)] }])).toMatch(/tile id "a" is used twice/);
    expect(mockDashboardsError([{ ...d, tile: [{ ...at(d.tile, 0), field: '' }] }])).toMatch(/needs a `field`/);
    const cases: [Partial<TileCfg> & Pick<TileCfg, 'type'>, RegExp | null][] = [
      [{ type: 'stat', query: 'x | stats count()' }, null],
      [{ type: 'stat' }, /needs a query/],
      [{ type: 'stat', query: 'level = error' }, /stats/],
      [{ type: 'table' }, /needs a query/],
      [{ type: 'chart' }, /needs `chart`/],
      [{ type: 'chart', value: 'count()' }, null],
      [{ type: 'annotations', w: 0 }, /at least 1/],
      [{ type: 'annotations', x: 22 }, /columns/],
      [{ type: 'annotations', window: 'soon' }, /not a duration/],
      [{ type: 'nope' as never }, /unknown type/],
    ];
    for (const [patch, error] of cases) {
      const e = mockTileError(tile(patch));
      if (error) expect(e).toMatch(error);
      else expect(e).toBeNull();
    }
    expect(mockTileError({ ...tile({ type: 'top', field: 'f' }), id: '' })).toMatch(/needs an id/);
  });
});

describe('MockShareBackend', () => {
  let imt: MockBackend;
  let share: MockShareBackend;
  beforeEach(() => {
    imt = new MockBackend({ latency: false });
    share = new MockShareBackend(imt, new MockAnalysisBackend(recordsFrom(imt)));
  });

  it('switches the redacted view of the mock workspace (pages, facets, context, exports)', async () => {
    const listening = { ...q, text: 'listening' };
    const before = await imt.queryPage({ query: listening, offset: 0, limit: 1, descending: false });
    expect(before.rows[0]?.message).toContain('0.0.0.0');
    const on = await share.setRedactedView(true);
    expect(on.redactedView).toBe(true);
    const after = await imt.queryPage({ query: listening, offset: 0, limit: 1, descending: false });
    expect(after.rows[0]?.message).toContain('<ipv4>');
    expect(after.rows[0]?.fields['Address']).toContain('<ipv4>');
    const facet = await imt.queryFacet({ query: q, field: 'CorrelationId', limit: 10 });
    expect(facet.values).toEqual([{ value: '<guid>', count: expect.any(Number) }]);
    const ctx = await imt.sourceContext({ recordId: at(after.rows, 0).id, before: 0, after: 0 });
    expect(ctx.lines[0]?.text).toContain('<ipv4>');
    const exported = await imt.exportEvents({ query: listening, format: 'csv', path: 'x.csv', redact: true });
    expect(exported.redactions?.['ipv4']).toBeGreaterThan(0);
    // A profile without rules switches the view off; switching on then fails.
    await imt.saveProfile('name = "x"\n[redact]\ndetectors = []');
    expect((await imt.workspaceSummary()).redactedView).toBe(false);
    await expect(share.setRedactedView(true)).rejects.toThrow(/no redaction rules/);
    expect((await share.setRedactedView(false)).redactedView).toBe(false);
  });

  it('keeps redacting with the new rules after a profile change', async () => {
    await share.setRedactedView(true);
    const doc = await imt.getProfile();
    await imt.saveProfile(doc.toml.replace('mode = "mask"', 'mode = "drop"'));
    const page = await imt.queryPage({ query: { ...q, text: 'listening' }, offset: 0, limit: 1, descending: false });
    expect(at(page.rows, 0).message).not.toContain('0.0.0.0');
    expect(at(page.rows, 0).message).not.toContain('<ipv4>');
  });

  it('previews redaction and summarises a query', async () => {
    expect((await share.previewRedaction({ sample: '10.0.0.1' })).after).toBe('<ipv4>');
    expect((await share.previewRedaction({ sample: 'a@b.io', toml: '[redact]\ndetectors = []' })).after).toBe('a@b.io');
    const s = await share.redactionSummary({ query: q, samples: 2 });
    expect(s.changed).toBeGreaterThan(0);
    expect(s.samples).toHaveLength(2);
    expect(s.hits.find((h) => h.rule === 'guid')!.count).toBeGreaterThan(0);
  });

  it('evaluates every tile kind of the demo dashboard', async () => {
    const dashboards = (await imt.workspaceSummary()).dashboards;
    const health = at(dashboards, 0);
    expect(health.title).toBe('OrderService health');
    const kinds = new Set<string>();
    for (const t of health.tile) {
      const data = await share.dashboardTile({ tile: t, query: q });
      kinds.add(data.kind);
      if (data.kind === 'stat') expect(data.sparkline).toHaveLength(24);
    }
    expect([...kinds].sort()).toEqual(['alerts', 'chart', 'stat', 'table', 'top']);
    const inline = await share.dashboardTile({ tile: tile({ type: 'chart', value: 'count()', by: ['@level'], style: 'bar', window: '15m' }), query: q });
    expect(inline.kind === 'chart' && inline.data.def.type).toBe('bar');
    const events = await share.dashboardTile({ tile: tile({ type: 'table', query: 'level = error', limit: 2 }), query: q });
    expect(events).toMatchObject({ kind: 'table', columns: ['@t', '@level', '@message'], truncated: true });
    expect((await share.dashboardTile({ tile: tile({ type: 'annotations' }), query: q })).kind).toBe('annotations');
    await expect(share.dashboardTile({ tile: tile({ type: 'stat' }), query: q })).rejects.toThrow(/needs a query/);
    await expect(share.dashboardTile({ tile: tile({ type: 'table', query: '| sort' }), query: q })).rejects.toBeDefined();
    await expect(share.dashboardTile({ tile: tile({ type: 'stat', query: '| stats nope(x)' }), query: q })).rejects.toBeDefined();
    await expect(share.dashboardTile({ tile: tile({ type: 'annotations', window: '5x' }), query: q })).rejects.toThrow(/duration/);
    const empty = await share.dashboardTile({ tile: tile({ type: 'stat', query: 'nosuchword_xyz | stats count()' }), query: q });
    expect(empty.kind === 'stat' && (empty.value === 0 || empty.value === null)).toBe(true);
  });

  it('computes a stat sparkline from one workspace snapshot, bucket by bucket like a per-bucket query', async () => {
    const stat = tile({ type: 'stat', query: 'level >= error | stats count()' });
    const pages = vi.spyOn(imt, 'queryPage');
    const data = await share.dashboardTile({ tile: stat, query: q });
    // The total and the snapshot: not one full reload per bucket (that made a stat tile take seconds).
    expect(pages.mock.calls.length).toBeLessThanOrEqual(2);
    if (data.kind !== 'stat') throw new Error('not a stat tile');
    expect(data.sparkline).toHaveLength(24);
    const analysis = new MockAnalysisBackend(recordsFrom(imt));
    const picks = [0, 11, 23];
    for (const i of picks) {
      const [start] = at(data.sparkline, i);
      const end = i === 23 ? (await imt.workspaceSummary()).to! : at(data.sparkline, i + 1)[0] - 1;
      const out = await analysis.runQuery(mockTileQuery({ ...q, window: null, from: start, to: end }, stat.query));
      if ('error' in out || out.result.kind !== 'table') throw new Error('expected a table');
      expect(data.sparkline[i]?.[1]).toBe(out.result.rows[0]?.[0] ?? null);
    }
  });

  it('combines tile queries with the window query like the engine', () => {
    expect(mockTileQuery({ ...q, ql: 'a' }, 'b | stats count()').ql).toBe('a | where b | stats count()');
    expect(mockTileQuery({ ...q, ql: 'a' }, '| stats count()').ql).toBe('a | stats count()');
    expect(mockTileQuery({ ...q, ql: 'a' }, undefined).ql).toBe('a');
    expect(mockTileQuery(q, ' b ').ql).toBe('b');
  });

  it('records report and image exports and checks their paths', async () => {
    const r = await share.exportReport({ report: report(), format: 'html', path: '/r.html', redact: true, theme: 'light' });
    expect(r.bytes).toBeGreaterThan(0);
    expect(r.redaction.find((h) => h.rule === 'email')?.count).toBe(1);
    expect((await share.exportReport({ report: report(), format: 'pdf', path: '/r.pdf', redact: false, theme: 'dark' })).redaction).toEqual([]);
    await expect(share.exportReport({ report: { ...report(), id: '' }, format: 'html', path: '/r.html', redact: false, theme: 'light' })).rejects.toThrow(/needs an id/);
    await expect(share.exportReport({ report: report(), format: 'pdf', path: '/r.html', redact: false, theme: 'light' })).rejects.toThrow(/\.pdf/);
    await share.saveVisual({ path: '/c.png', kind: 'png', data: 'data:image/png;base64,AAAA' });
    await share.saveVisual({ path: '/c.svg', kind: 'svg', data: '<svg/>' });
    await expect(share.saveVisual({ path: '/c.png', kind: 'png', data: 'x' })).rejects.toThrow(/PNG/);
    await expect(share.saveVisual({ path: '/c.svg', kind: 'svg', data: '<p/>' })).rejects.toThrow(/svg/);
    await expect(share.saveVisual({ path: '/c.gif', kind: 'svg', data: '<svg/>' })).rejects.toThrow(/\.svg/);
    expect(share.written.map((w) => w.path)).toEqual(['/r.html', '/r.pdf', '/c.png', '/c.svg']);
    expect((globalThis as Record<string, unknown>)['__imtMockShareWritten']).toBe(share.written);
  });

  it('estimates, creates and opens bundles', async () => {
    const options: BundleOptions = { name: 'Case 42', content: 'events', query: { ...q, levels: ['Error'] }, redact: true, ui: { k: 1 }, reports: [report()] };
    const est = await share.estimateBundle(options);
    expect(est.events).toBeGreaterThan(0);
    expect(est.files).toBe(5);
    const raw = await share.estimateBundle({ ...options, content: 'raw', reports: [] });
    expect(raw.parts.map((p) => p.name)).toEqual(['profile', 'logs', 'reports']);
    const m = await share.createBundle(options, '/out/case.imtz');
    expect(m).toMatchObject({ format: 'imt-share', name: 'Case 42', redacted: true, content: 'events' });
    await expect(share.createBundle(options, '/out/case.zip')).rejects.toThrow(/\.imtz/);
    const opened = await share.openBundle('/out/case.imtz');
    expect(opened).toMatchObject({ readOnly: true, name: 'Case 42', ui: { k: 1 }, reports: [report()], bundle: m });
    const other = await share.openBundle('/demo/shared.imtz');
    expect(other).toMatchObject({ name: 'shared', bundle: null, reports: [], ui: null });
    await expect(share.openBundle('/demo/x.zip')).rejects.toThrow(/not an IMT bundle/);
    const plain = await share.createBundle({ ...options, redact: false, content: 'raw' }, '/out/raw.imtz');
    expect(plain.redaction).toEqual({});
    expect(plain.events).toBeNull();
  });

  it('refuses profiles with a broken [redact] section or dashboard, like the engine', async () => {
    const bad = await imt.validateProfile('[redact]\ndetectors = ["nope"]');
    expect(bad.ok).toBe(false);
    expect(bad.error).toMatch(/unknown variant/);
    const dash = await imt.validateProfile('[[dashboard]]\nid = "d"\ntitle = "D"\n[[dashboard.tile]]\nid = "t"\ntype = "stat"');
    expect(dash).toMatchObject({ ok: false, error: expect.stringContaining('needs a query') });
    expect((await imt.validateProfile('[redact]\ndetectors = ["email"]')).ok).toBe(true);
  });

  it('works over a non-mock workspace backend (no redacted view to switch)', async () => {
    const other = new MockShareBackend({ workspaceSummary: () => imt.workspaceSummary(), getProfile: () => imt.getProfile() } as unknown as ImtBackend, new MockAnalysisBackend(recordsFrom(imt)));
    expect((await other.setRedactedView(true)).redactedView).toBe(false);
  });
});
