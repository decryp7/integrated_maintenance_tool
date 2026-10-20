import { describe, expect, it } from 'vitest';
import { failureHeadline, hostOf, offersManualDownload } from './manual-download-view';

describe('manual download view', () => {
  it('offers the manual download for network failures only', () => {
    for (const c of ['tls', 'proxyAuth', 'proxy', 'offline', 'timeout', 'httpStatus', 'network'] as const) {
      expect(offersManualDownload(c)).toBe(true);
      expect(failureHeadline(c)).not.toBe('');
    }
    for (const c of ['disk', 'integrity', 'gated', 'cancelled', 'noRuntime'] as const) {
      expect(offersManualDownload(c)).toBe(false);
      expect(failureHeadline(c)).toBe('');
    }
    expect(offersManualDownload(null)).toBe(false);
    expect(offersManualDownload(undefined)).toBe(false);
    expect(failureHeadline('tls')).toContain('intercepts HTTPS');
    expect(failureHeadline('proxyAuth')).toContain('sign-in');
  });

  it('names the host of a download address', () => {
    expect(hostOf('https://huggingface.co/a/b?download=true')).toBe('huggingface.co');
    expect(hostOf('not a url')).toBe('');
  });
});
