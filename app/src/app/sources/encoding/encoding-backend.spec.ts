import { TestBed } from '@angular/core/testing';
import { MemoryLogger, tinyBackend } from '../../../testing/helpers';
import { ImtBackend } from '../../core/backend';
import { Logger } from '../../core/logger';
import { mockJobs } from '../../jobs/mock-jobs';
import { EncodingBackend, TauriEncodingBackend } from './encoding-backend';
import { MOCK_ENCODINGS, MockEncodingBackend } from './mock/mock-encoding-backend';
import type { MockDataset } from '../../core/mock/mock-data';
import { tinyDataset } from '../../../testing/helpers';

function withEncodings(): MockDataset {
  const d = tinyDataset();
  return {
    ...d,
    sources: d.sources.map((s, i) => (i === 0 ? { ...s, encoding: { label: 'Shift_JIS', confidence: 84, origin: 'detected' as const } } : s)),
  };
}

describe('TauriEncodingBackend', () => {
  it('maps every method to its command', async () => {
    const calls: [string, unknown][] = [];
    const call = vi.fn(async (cmd: string, args?: unknown) => {
      calls.push([cmd, args]);
      return null;
    });
    const b = new TauriEncodingBackend(call as never);
    await b.options();
    await b.setSourceEncoding(3, 'GBK');
    await b.setSourceEncoding(4, null);
    expect(calls).toEqual([
      ['list_encodings', undefined],
      ['set_source_encoding', { sourceId: 3, encoding: 'GBK' }],
      ['set_source_encoding', { sourceId: 4, encoding: null }],
    ]);
  });

  it('is the mock in a browser, with failures logged', async () => {
    TestBed.configureTestingModule({ providers: [{ provide: Logger, useFactory: () => new MemoryLogger() }, { provide: ImtBackend, useValue: tinyBackend() }] });
    const backend = TestBed.inject(EncodingBackend);
    expect((await backend.options()).map((o) => o.label)).toContain('Shift_JIS');
    await expect(backend.setSourceEncoding(99, 'GBK')).rejects.toBeDefined();
    expect((TestBed.inject(Logger) as MemoryLogger).entries.some((e) => e.level === 'error' || e.level === 'warn')).toBe(true);
  });
});

describe('MockEncodingBackend', () => {
  it('offers the engine\'s encodings', async () => {
    const options = await new MockEncodingBackend(tinyBackend()).options();
    expect(options).toEqual(MOCK_ENCODINGS);
    expect(options.length).toBe(21);
    expect(options.map((o) => o.label)).toEqual(expect.arrayContaining(['UTF-8', 'UTF-16LE', 'UTF-16BE', 'windows-1252', 'Shift_JIS', 'gb18030', 'ISO-8859-15']));
  });

  it('records a choice on the source (manual, 100) as a job, and "auto" or null detects again', async () => {
    const imt = tinyBackend({ dataset: withEncodings });
    const b = new MockEncodingBackend(imt);
    const job = await b.setSourceEncoding(0, 'windows-1252');
    expect((await mockJobs().wait(job)).state).toBe('done');
    expect((await imt.workspaceSummary()).sources[0]?.encoding).toEqual({ label: 'windows-1252', confidence: 100, origin: 'manual' });
    expect(b.chosen.get(0)).toBe('windows-1252');

    await b.setSourceEncoding(0, null);
    expect((await imt.workspaceSummary()).sources[0]?.encoding?.origin).toBe('detected');
    await b.setSourceEncoding(0, 'GBK');
    await b.setSourceEncoding(0, 'auto');
    expect((await imt.workspaceSummary()).sources[0]?.encoding?.origin).toBe('detected');
    expect(b.chosen.get(0)).toBeNull();
  });

  it('rejects an unknown source, a binary source, an unknown encoding and a non-mock workspace', async () => {
    const imt = tinyBackend({ dataset: withEncodings });
    const b = new MockEncodingBackend(imt);
    await expect(b.setSourceEncoding(99, 'GBK')).rejects.toMatchObject({ code: 'notFound' });
    await expect(b.setSourceEncoding(1, 'GBK')).rejects.toMatchObject({ code: 'invalid' });
    await expect(b.setSourceEncoding(0, 'klingon')).rejects.toMatchObject({ code: 'invalid' });
    await expect(new MockEncodingBackend({} as ImtBackend).setSourceEncoding(0, 'GBK')).rejects.toMatchObject({ code: 'invalid' });
  });
});
