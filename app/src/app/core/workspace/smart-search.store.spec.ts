import { TestBed } from '@angular/core/testing';
import { provideTinyBackendBare, query, settle, testLogger, tinyBackend } from '../../../testing/helpers';
import { MockBackend } from '../mock/mock-backend';
import { QueryStore, TEXT_DEBOUNCE_MS } from './query.store';
import { isWordSearch, SmartSearchStore } from './smart-search.store';
import { WorkspaceDataStore } from './workspace-data.store';

function setup(backend: MockBackend = tinyBackend()) {
  TestBed.configureTestingModule({ providers: provideTinyBackendBare(backend) });
  const smart = TestBed.inject(SmartSearchStore);
  return { smart, q: TestBed.inject(QueryStore), data: TestBed.inject(WorkspaceDataStore), backend };
}

describe('SmartSearchStore', () => {
  it('tells word searches apart', () => {
    expect(isWordSearch(query({ text: 'x', textMode: 'words' }))).toBe(true);
    expect(isWordSearch(query({ text: 'x', textMode: 'wordsOrSubstring' }))).toBe(true);
    expect(isWordSearch(query({ text: 'x' }))).toBe(false);
    expect(isWordSearch(query({ text: ' ', textMode: 'words' }))).toBe(false);
    expect(isWordSearch(query({ text: 'x', textMode: 'words', regex: true }))).toBe(false);
    expect(isWordSearch(query({ text: 'x', textMode: 'words', caseSensitive: true }))).toBe(false);
  });

  it('the search box asks for words, then adds the partial matches the scan found', async () => {
    vi.useFakeTimers();
    try {
      const { smart, q, backend } = setup();
      const scan = vi.spyOn(backend, 'searchPartial');
      await vi.runAllTimersAsync();
      q.setTextInput('imeout');
      await vi.advanceTimersByTimeAsync(TEXT_DEBOUNCE_MS + 1);
      await vi.runAllTimersAsync();
      TestBed.tick();
      await vi.runAllTimersAsync();
      expect(scan).toHaveBeenCalledTimes(1);
      expect(scan.mock.calls.at(0)?.[0]).toMatchObject({ text: 'imeout', textMode: 'words' });
      // `TimeoutException` holds "imeout" only inside a word: two partial matches, added to the query.
      expect(smart.added()).toBe(2);
      expect(smart.scanning()).toBe(false);
      expect(q.query().textMode).toBe('wordsOrSubstring');
      // Switching the mode is not a new search.
      TestBed.tick();
      await vi.runAllTimersAsync();
      expect(scan).toHaveBeenCalledTimes(1);
      // A new search starts over with words.
      q.setTextInput('time');
      await vi.advanceTimersByTimeAsync(TEXT_DEBOUNCE_MS + 1);
      TestBed.tick();
      await vi.runAllTimersAsync();
      expect(q.query().textMode).toBe('words');
      expect(smart.added()).toBe(0);
      // Clearing the search clears the note.
      q.clearQuery();
      TestBed.tick();
      expect(smart.added()).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it('a failed scan keeps the word matches and logs only the error code', async () => {
    const backend = tinyBackend({ fail: { searchPartial: 1 } });
    const { smart, q } = setup(backend);
    await settle();
    q.patchQuery({ text: 'imeout', textMode: 'words' });
    await settle();
    expect(smart.scanning()).toBe(false);
    expect(smart.added()).toBe(0);
    expect(q.query().textMode).toBe('words');
    const warn = testLogger().entries.find((e) => e.category === 'search');
    expect(warn?.message).toBe('Partial matches are unavailable');
    expect(JSON.stringify(warn?.props)).not.toContain('imeout');
  });

  it('a stale scan is ignored', async () => {
    const backend = tinyBackend();
    let release: (v: { added: number; complete: boolean }) => void = () => {};
    vi.spyOn(backend, 'searchPartial').mockImplementationOnce(() => new Promise((r) => (release = r)));
    const { smart, q } = setup(backend);
    await settle();
    q.patchQuery({ text: 'imeout', textMode: 'words' });
    await settle();
    expect(smart.scanning()).toBe(true);
    q.patchQuery({ text: 'cache', textMode: 'words' });
    await settle();
    release({ added: 99, complete: true });
    await settle();
    expect(smart.added()).toBe(0);
    expect(q.query().textMode).toBe('words');
  });
});
