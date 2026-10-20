import * as mocks from '../mock-backends';
import { browserMocks, installBrowserMocks, loadBrowserMocks } from './mock-registry';

describe('mock registry', () => {
  afterEach(() => installBrowserMocks(mocks));

  it('fails fast with a message when the mocks were not loaded', () => {
    installBrowserMocks(null as never);
    expect(() => browserMocks()).toThrow(/loadBrowserMocks\(\) before bootstrapping/);
  });

  it('loads the mock backends on demand', async () => {
    installBrowserMocks(null as never);
    await loadBrowserMocks();
    expect(browserMocks().MockBackend).toBe(mocks.MockBackend);
  });
});
