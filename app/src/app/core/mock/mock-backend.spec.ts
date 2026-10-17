import { query, tinyBackend, tinyDataset } from '../../../testing/helpers';
import { MockBackend } from './mock-backend';
import { MOCK_PROFILE_TOML } from './mock-profile';

describe('MockBackend', () => {
  let backend: MockBackend;
  beforeEach(() => (backend = tinyBackend()));

  it('summarises the workspace', async () => {
    const s = await backend.workspaceSummary();
    expect(s.total).toBe(11);
    expect(s.sources.map((x) => [x.name, x.records])).toEqual([
      ['app.clef', 7],
      ['app.log', 4],
    ]);
    expect(s.sources[1].skipped).toBe(1);
    expect(s.levels).toMatchObject({ Information: 7, Warning: 1, Error: 1, Fatal: 1, Debug: 1, Verbose: 0 });
    expect(s.unrecognized).toHaveLength(1);
    expect(s.defaultGroupBy).toEqual(['MachineName', 'ProcessId']);
    expect(s.profileName).toContain('demo');
    expect(s.from).toBeLessThan(s.to!);
  });

  it('opens, removes, clears and reloads sources', async () => {
    let s = await backend.openPaths(['/x/extra.log', '/x/extra.log']);
    expect(s.sources).toHaveLength(3);
    const extra = s.sources[2];
    expect(extra.name).toBe('extra.log');
    expect(extra.records).toBeGreaterThan(10);
    s = await backend.openPaths(['/x/extra.log']);
    expect(s.sources).toHaveLength(3);
    s = await backend.removeSource(extra.id);
    expect(s.sources).toHaveLength(2);
    s = await backend.clearWorkspace();
    expect(s.total).toBe(0);
    expect(s.from).toBeNull();
    s = await backend.reloadWorkspace();
    expect(s.total).toBe(11);
  });

  it('answers queries', async () => {
    expect((await backend.queryPage({ query: query(), offset: 0, limit: 5, descending: false })).rows).toHaveLength(5);
    expect((await backend.queryTimeline({ query: query(), buckets: 20, alwaysShowLifecycle: true })).lanes).toHaveLength(3);
    expect((await backend.queryFacet({ query: query(), field: '@level', limit: 10 })).distinct).toBe(5);
    expect((await backend.queryChart({ query: query(), chartId: 'cpu', buckets: 20 })).series).toHaveLength(2);
    await expect(backend.queryChart({ query: query(), chartId: 'nope', buckets: 20 })).rejects.toMatch(/Unknown chart/);
  });

  it('validates profiles', async () => {
    expect(await backend.validateProfile(MOCK_PROFILE_TOML)).toEqual({ ok: true, error: null, line: null });
    const syntax = await backend.validateProfile('name = "x"\n[parse\n');
    expect(syntax.ok).toBe(false);
    expect(syntax.line).toBe(2);
    expect((await backend.validateProfile('[[extract]]\nregex = "x"')).error).toMatch(/missing a name/);
    expect((await backend.validateProfile('[[extract]]\nname = "a"\nregex = "("')).error).toMatch(/invalid regex/);
    expect((await backend.validateProfile('[[marker]]\nlabel = "x"')).error).toMatch(/missing a kind/);
    expect((await backend.validateProfile('[[chart]]\nid = "c"')).error).toMatch(/needs id and y/);
    const badY = await backend.validateProfile('[[chart]]\nid = "c"\ny = "median(x)"');
    expect(badY.error).toMatch(/avg\(field\)/);
    expect(badY.line).toBe(3);
  });

  it('gets, saves and rejects profiles', async () => {
    const doc = await backend.getProfile();
    expect(doc.path).toBeNull();
    expect(doc.toml).toBe(MOCK_PROFILE_TOML);
    const s = await backend.saveProfile('name = "Only markers"\n[[marker]]\nkind = "warning"\nwhen = [{ field = "@level", op = "eq", value = "Warning" }]\n');
    expect(s.profileName).toBe('Only markers');
    expect(s.markerKinds.map((k) => [k.kind, k.owner])).toEqual([['warning', 'profile']]);
    expect(s.charts).toEqual([]);
    expect((await backend.getProfile()).name).toBe('Only markers');
    await expect(backend.saveProfile('[[broken')).rejects.toBeTruthy();
    const unnamed = await backend.saveProfile('');
    expect(unnamed.profileName).toBe('Unnamed profile');
    expect((await backend.getProfile()).name).toBe('Unnamed profile');
  });

  it('previews extractors', async () => {
    const p = await backend.previewExtractor({ toml: MOCK_PROFILE_TOML, extractor: 'process-stats', query: query(), limit: 5 });
    expect(p.matches).toBe(2);
    expect((await backend.previewExtractor({ toml: MOCK_PROFILE_TOML, extractor: 'nope', query: query(), limit: 5 })).error).toMatch(/No extractor/);
    expect((await backend.previewExtractor({ toml: '[[x', extractor: 'a', query: query(), limit: 5 })).error).toBeTruthy();
  });

  it('imports and exports profiles', async () => {
    const doc = await backend.importProfile('/p/team.toml');
    expect(doc.path).toBe('/p/team.toml');
    const click = vi.fn();
    const createElement = document.createElement.bind(document);
    const spy = vi.spyOn(document, 'createElement').mockImplementation((tag: string) => {
      const node = createElement(tag);
      if (tag === 'a') node.click = click;
      return node;
    });
    URL.createObjectURL ??= () => 'blob:x';
    URL.revokeObjectURL ??= () => undefined;
    await backend.exportProfile('/p/out.toml', 'name = "x"');
    expect(click).toHaveBeenCalled();
    spy.mockRestore();
  });

  it('uses the demo dataset and simulated latency by default', async () => {
    vi.useFakeTimers();
    try {
      const b = new MockBackend();
      const pending = b.queryFacet({ query: query(), field: '@level', limit: 3 });
      await vi.advanceTimersByTimeAsync(100);
      expect((await pending).total).toBeGreaterThan(1000);
    } finally {
      vi.useRealTimers();
    }
  });

  it('turns thrown errors into rejected strings', async () => {
    const b = new MockBackend({ latency: false, dataset: tinyDataset });
    await expect(b.queryChart({ query: query(), def: { id: 'x', title: '', y: 'nope', bucket: 'auto', splitBy: [], type: 'line', unit: '' }, buckets: 10 })).rejects.toMatch(/Invalid y/);
  });

  describe('options and engine commands', () => {
    it('serves a synthetic total lazily and reports it in the summary', async () => {
      const b = new MockBackend({ latency: false, dataset: tinyDataset, syntheticTotal: 50_000_000 });
      const summary = await b.workspaceSummary();
      expect(summary.total).toBe(50_000_000);
      expect(summary.sources[0].records).toBe(50_000_000);
      expect(summary.sources[1].records).toBe(4);
      const page = await b.queryPage({ query: query(), offset: 49_999_998, limit: 500, descending: false });
      expect(page.rows.map((r) => r.id)).toEqual([49_999_998, 49_999_999]);
    });

    it('reports synthetic facets, corrupt markers and simulated failures', async () => {
      const b = new MockBackend({ latency: false, dataset: tinyDataset, syntheticFacetValues: 10_000, corruptMarkers: true, fail: { queryFacet: 1, workspaceSummary: 1, queryTimeline: 1 } });
      await expect(b.queryFacet({ query: query(), field: 'x', limit: 10 })).rejects.toMatch(/Simulated queryFacet failure/);
      const f = await b.queryFacet({ query: query(), field: 'x', limit: 5000 });
      expect(f.distinct).toBe(10_000);
      expect(f.values).toHaveLength(1000);
      await expect(b.workspaceSummary()).rejects.toMatch(/Simulated/);
      await expect(b.queryTimeline({ query: query(), buckets: 10, alwaysShowLifecycle: true })).rejects.toMatch(/Simulated/);
      expect((await b.workspaceSummary()).total).toBe(11);
      const page = await b.queryPage({ query: query(), offset: 0, limit: 2, descending: false });
      expect(page.rows[0].markers).toBeUndefined();
    });

    it('returns the lines around an event', async () => {
      const b = tinyBackend();
      const page = await b.queryPage({ query: query(), offset: 3, limit: 1, descending: false });
      const rec = page.rows[0];
      const ctx = await b.sourceContext({ recordId: rec.id, before: 2, after: 1 });
      expect(ctx.sourceName).toBe(rec.sourceName);
      expect(ctx.line).toBe(rec.line);
      expect(ctx.lines.map((l) => l.line)).toEqual([rec.line - 2, rec.line - 1, rec.line, rec.line + 1].filter((n) => n >= 1));
      expect(ctx.lines.find((l) => l.line === rec.line)?.text).toContain(rec.message);
      expect((await b.sourceContext({ recordId: 0, before: 99999, after: 0 })).lines).toHaveLength(1);
      expect((await b.sourceContext({ recordId: 0, before: 0, after: 999 })).lines).toHaveLength(501);
      await expect(b.sourceContext({ recordId: 9999, before: 1, after: 1 })).rejects.toMatch(/no longer/);
      const undated = (await b.queryPage({ query: query(), offset: 10, limit: 1, descending: false })).rows[0];
      expect(undated.ts).toBeNull();
      expect((await b.sourceContext({ recordId: undated.id, before: 0, after: 0 })).lines[0].text).toMatch(/^ \[/);
    });

    it('reads the head of workspace files only', async () => {
      const b = new MockBackend({ latency: false, dataset: tinyDataset, fail: { readFileHead: 1, previewParse: 1 } });
      await expect(b.readFileHead('/logs/system/export.reg')).rejects.toMatch(/Simulated/);
      expect(await b.readFileHead('/logs/system/export.reg')).toContain('[INF]');
      expect(await b.readFileHead('/logs/web01/app.clef', 10)).toHaveLength(10);
      await expect(b.readFileHead('/etc/passwd')).rejects.toMatch(/only files in the workspace/);
      await expect(b.previewParse({ toml: '', sample: '', fileName: '', limit: 1 })).rejects.toMatch(/Simulated/);
      expect((await b.previewParse({ toml: '', sample: 'x', fileName: '', limit: 1 })).total).toBe(0);
    });

    it('emits workspace changes while live is on, with a new event each time', async () => {
      const b = new MockBackend({ latency: false, dataset: tinyDataset, liveMs: 10 });
      const changed = vi.fn();
      const off = await b.onWorkspaceChanged(changed);
      expect(await b.setLive(true)).toBe(2);
      await vi.waitFor(() => expect(changed.mock.calls.length).toBeGreaterThanOrEqual(2));
      const summary = await b.workspaceSummary();
      expect(summary.total).toBeGreaterThan(11);
      expect((await b.queryPage({ query: query({ text: 'Live event' }), offset: 0, limit: 1, descending: true })).rows[0].message).toMatch(/Live event/);
      expect(await b.setLive(false)).toBe(0);
      const n = changed.mock.calls.length;
      await new Promise((r) => setTimeout(r, 50));
      expect(changed.mock.calls.length).toBe(n);
      off();
      await b.setLive(true);
      await new Promise((r) => setTimeout(r, 50));
      expect(changed.mock.calls.length).toBe(n);
      await b.setLive(false);
    });

    it('counts live events in a synthetic result and survives an empty workspace', async () => {
      const synthetic = new MockBackend({ latency: false, dataset: tinyDataset, syntheticTotal: 100, liveMs: 10 });
      await synthetic.setLive(true);
      await vi.waitFor(async () => expect((await synthetic.workspaceSummary()).total).toBeGreaterThan(100));
      await synthetic.setLive(false);
      const empty = new MockBackend({ latency: false, dataset: () => ({ sources: [], unrecognized: [], records: [] }), liveMs: 10 });
      const changed = vi.fn();
      await empty.onWorkspaceChanged(changed);
      await empty.setLive(true);
      await vi.waitFor(() => expect(changed).toHaveBeenCalled());
      expect((await empty.workspaceSummary()).total).toBe(0);
      await empty.setLive(false);
    });

    it('infers and explains regexes like the engine contract says', async () => {
      const r = await backend.inferRegex(['order 1 ok', 'order 22 ok']);
      expect(r.pattern).toBe('order (?P<n1>\\d+) ok');
      expect(r.groups).toEqual([{ name: 'n1', kind: 'number', examples: ['1', '22'] }]);
      expect(r.matches).toEqual([true, true]);
      await expect(backend.inferRegex([])).rejects.toContain('at least one');
      expect((await backend.explainRegex('ab\\d+')).map((n) => n.text)).toEqual(['Literal "ab"', 'One or more digits']);
      await expect(backend.explainRegex('(')).rejects.toContain('not a valid regex');
    });

    it('lists plugins with their status', async () => {
      expect((await backend.workspaceSummary()).plugins.map((p) => [p.id, p.status, p.failures, p.lastError])).toEqual([
        ['net.decryptology.registry', 'active', 0, null],
        ['net.decryptology.windows', 'active', 0, null],
        ['net.decryptology.web', 'active', 0, null],
        ['net.decryptology.otel', 'inactive', 0, null],
      ]);
      const otel = (await backend.workspaceSummary()).plugins.find((p) => p.status === 'inactive')!;
      expect(otel.inactiveReason).toContain('needs');
    });
  });
});
