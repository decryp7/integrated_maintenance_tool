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

  it('offers the old in-memory store features only when asked', () => {
    expect(mockOptionsFromSearch('?mockMemoryFeatures=1')).toEqual({ memoryStoreFeatures: true });
    expect(mockOptionsFromSearch('?mockMemoryFeatures=yes')).toEqual({});
  });

  it('ignores malformed values', () => {
    expect(mockOptionsFromSearch('?mockTotal=-5&mockFacet=abc&mockLiveMs=0&mockCorrupt=other&mockFail=queryPage:x')).toEqual({ fail: { queryPage: 1 } });
  });

  it('reads the live simulation switches', () => {
    expect(mockOptionsFromSearch('?mockLive=1')).toEqual({ liveRate: 20, exposeLiveCalls: true });
    expect(mockOptionsFromSearch('?mockLiveRate=50&mockLiveRotateMs=0&mockLiveAlertMs=1500&mockLiveMs=100')).toEqual({
      liveMs: 100,
      liveRate: 50,
      exposeLiveCalls: true,
      liveRotateMs: 0,
      liveAlertMs: 1500,
    });
    expect(mockOptionsFromSearch('?mockLive=1&mockLiveRotateMs=x')).toEqual({ liveRate: 20, exposeLiveCalls: true });
    expect(mockOptionsFromSearch('?mockLive=2')).toEqual({});
  });
});
