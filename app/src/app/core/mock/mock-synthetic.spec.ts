import { FACET_LIMIT_MAX, syntheticFacet, syntheticPage, syntheticRow } from './mock-synthetic';
import { query } from '../../../testing/helpers';

const req = (p: Partial<Parameters<typeof syntheticPage>[0]> = {}) => ({ query: query(), offset: 0, limit: 5, descending: false, ...p });

describe('synthetic rows', () => {
  it('generates rows by index', () => {
    expect(syntheticRow(0)).toMatchObject({ id: 0, level: 'Error', line: 1, markers: [] });
    expect(syntheticRow(7).level).toBe('Warning');
    expect(syntheticRow(8).level).toBe('Information');
    expect(syntheticRow(39_999_999).message).toBe('Synthetic event 39999999');
  });

  it('pages ascending and descending', () => {
    const asc = syntheticPage(req({ offset: 100 }), 50_000_000);
    expect(asc.total).toBe(50_000_000);
    expect(asc.rows.map((r) => r.id)).toEqual([100, 101, 102, 103, 104]);
    const desc = syntheticPage(req({ offset: 0, descending: true }), 50_000_000);
    expect(desc.rows.map((r) => r.id)).toEqual([49_999_999, 49_999_998, 49_999_997, 49_999_996, 49_999_995]);
  });

  it('clamps offsets and stops at the end', () => {
    expect(syntheticPage(req({ offset: 1e9 }), 10).rows.map((r) => r.id)).toEqual([9]);
    expect(syntheticPage(req({ offset: -4, limit: 2 }), 10).rows.map((r) => r.id)).toEqual([0, 1]);
    expect(syntheticPage(req({ limit: -1 }), 10).rows).toEqual([]);
    expect(syntheticPage(req(), 0)).toEqual({ total: 0, offset: 0, rows: [], anchorIndex: null });
  });

  it('resolves anchors to their position in either direction', () => {
    const a = syntheticPage(req({ anchor: 40_000_123, limit: 1 }), 50_000_000);
    expect(a.anchorIndex).toBe(40_000_123);
    expect(a.rows[0].id).toBe(40_000_123);
    const d = syntheticPage(req({ anchor: 40_000_123, limit: 1, descending: true }), 50_000_000);
    expect(d.anchorIndex).toBe(9_999_876);
    expect(d.rows[0].id).toBe(40_000_123);
    expect(syntheticPage(req({ anchor: 10 }), 10).anchorIndex).toBeNull();
    expect(syntheticPage(req({ anchor: -1 }), 10).anchorIndex).toBeNull();
    expect(syntheticPage(req({ anchor: 1.5 }), 10).anchorIndex).toBeNull();
  });

  it('centres a multi-row anchor page on the anchor', () => {
    const a = syntheticPage(req({ anchor: 100, limit: 10 }), 1000);
    expect(a.offset).toBe(95);
    expect(a.rows[5].id).toBe(100);
  });
});

describe('synthetic facets', () => {
  it('reports all distinct values but returns at most 1000, as the engine does', () => {
    const f = syntheticFacet({ query: query(), field: 'Host', limit: 5000 }, 10_000);
    expect(f.distinct).toBe(10_000);
    expect(f.values).toHaveLength(FACET_LIMIT_MAX);
    expect(f.values[0]).toEqual({ value: 'Host-value-00000', count: 10_000 });
    expect(syntheticFacet({ query: query(), field: 'Host', limit: 3 }, 10_000).values).toHaveLength(3);
    expect(syntheticFacet({ query: query(), field: 'Host', limit: 50 }, 4).values).toHaveLength(4);
  });
});
