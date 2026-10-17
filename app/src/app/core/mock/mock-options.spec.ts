import { mockOptionsFromSearch } from './mock-options';

describe('mockOptionsFromSearch', () => {
  it('has no options by default', () => {
    expect(mockOptionsFromSearch('')).toEqual({});
    expect(mockOptionsFromSearch('?x=1')).toEqual({});
  });

  it('reads the switches', () => {
    expect(mockOptionsFromSearch('?mockTotal=50000000&mockFacet=10000&mockLiveMs=300&mockCorrupt=markers&mockFail=queryPage:2,queryFacet,nonsense:3')).toEqual({
      syntheticTotal: 50_000_000,
      syntheticFacetValues: 10_000,
      liveMs: 300,
      corruptMarkers: true,
      fail: { queryPage: 2, queryFacet: 1 },
    });
  });

  it('ignores malformed values', () => {
    expect(mockOptionsFromSearch('?mockTotal=-5&mockFacet=abc&mockLiveMs=0&mockCorrupt=other&mockFail=queryPage:x')).toEqual({ fail: { queryPage: 1 } });
  });
});
