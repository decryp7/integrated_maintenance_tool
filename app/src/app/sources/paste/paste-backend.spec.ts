import { TestBed } from '@angular/core/testing';
import { MemoryLogger, tinyBackend } from '../../../testing/helpers';
import { DebriefBackend } from '../../core/backend';
import { Logger } from '../../core/logger';
import { MockPasteBackend } from './mock/mock-paste-backend';
import { PasteBackend, TauriPasteBackend } from './paste-backend';

describe('TauriPasteBackend', () => {
  it('maps pasteLog to paste_log with { text, name }', async () => {
    const call = vi.fn(async () => ({}));
    const b = new TauriPasteBackend(call as never);
    await b.pasteLog('hello', 'web01');
    await b.pasteLog('hi', null);
    expect(call.mock.calls).toEqual([
      ['paste_log', { text: 'hello', name: 'web01' }],
      ['paste_log', { text: 'hi', name: null }],
    ]);
  });

  it('is the mock in a browser, with failures logged', async () => {
    TestBed.configureTestingModule({ providers: [{ provide: Logger, useFactory: () => new MemoryLogger() }, { provide: DebriefBackend, useValue: tinyBackend() }] });
    const backend = TestBed.inject(PasteBackend);
    expect((await backend.pasteLog('one line', 'x')).sources.some((s) => s.name === 'pasted/x.log')).toBe(true);
    await expect(backend.pasteLog('   ', null)).rejects.toBeDefined();
    expect((TestBed.inject(Logger) as MemoryLogger).entries.some((e) => e.level === 'error' || e.level === 'warn')).toBe(true);
  });
});

describe('MockPasteBackend', () => {
  it('adds a source with the lines as events and a safe name', async () => {
    const debrief = tinyBackend();
    const b = new MockPasteBackend(debrief);
    const summary = await b.pasteLog('2026-10-14 ERROR boom\nplain line\nWARN careful\n\n', 'C:\\x\\web01 errors');
    const source = summary.sources.find((s) => s.name === 'pasted/web01 errors.log');
    expect(source?.records).toBe(3);
    expect(summary.total).toBeGreaterThanOrEqual(3);
  });

  it('makes a taken name unique and names an unnamed paste', async () => {
    const b = new MockPasteBackend(tinyBackend());
    await b.pasteLog('a', 'dup');
    const second = await b.pasteLog('b', 'dup');
    const third = await b.pasteLog('c', null);
    expect(second.sources.map((s) => s.name)).toContain('pasted/dup (2).log');
    expect(third.sources.map((s) => s.name)).toContain('pasted/Pasted log.log');
  });

  it('refuses empty text, text over 8 MiB, and a non-mock workspace', async () => {
    const b = new MockPasteBackend(tinyBackend());
    await expect(b.pasteLog('  \n ', null)).rejects.toMatchObject({ code: 'invalid' });
    await expect(b.pasteLog('x'.repeat(8 * 1024 * 1024 + 1), null)).rejects.toMatchObject({ code: 'invalid', message: expect.stringContaining('8 MiB') });
    await expect(new MockPasteBackend({} as DebriefBackend).pasteLog('a', null)).rejects.toMatchObject({ code: 'invalid' });
  });
});
