import { MockBackend } from './mock-backend';
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

  it('reads the long names switch', () => {
    expect(mockOptionsFromSearch('?mockLongNames=1')).toEqual({ longNames: true });
    expect(mockOptionsFromSearch('?mockLongNames=0')).toEqual({});
  });

  it('reads the commands to hold, ignoring unknown ones', () => {
    expect(mockOptionsFromSearch('?mockHold=getProfile,nonsense')).toEqual({ hold: ['getProfile'] });
    expect(mockOptionsFromSearch('?mockHold=')).toEqual({});
  });
});

describe('MockBackend hold', () => {
  const release = () => (globalThis as unknown as { __debriefMockRelease: () => void }).__debriefMockRelease();
  afterEach(() => delete (globalThis as Record<string, unknown>)['__debriefMockRelease']);

  it('answers a held command only once the test releases it, and later calls at once', async () => {
    const backend = new MockBackend({ latency: false, hold: ['getProfile'] });
    let answered = false;
    const first = backend.getProfile().then((doc) => ((answered = true), doc));
    await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
    expect(answered).toBe(false);
    release();
    expect((await first).toml).toContain('[');
    expect(answered).toBe(true);
    await expect(backend.getProfile()).resolves.toMatchObject({ path: null });
  });

  it('holds nothing by default and publishes no release hook', async () => {
    const backend = new MockBackend({ latency: false });
    await expect(backend.getProfile()).resolves.toMatchObject({ path: null });
    expect((globalThis as Record<string, unknown>)['__debriefMockRelease']).toBeUndefined();
  });
});
