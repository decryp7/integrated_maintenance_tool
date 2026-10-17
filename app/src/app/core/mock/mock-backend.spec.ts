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
    expect(s.markerKinds.map((k) => k.kind)).toEqual(['warning']);
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
});
