import { TestBed } from '@angular/core/testing';
import { settle } from '../../testing/helpers';
import { AnalysisBackend } from '../core/analysis-backend';
import { SearchInputKind } from '../core/api.types';
import { WorkspaceStore } from '../core/workspace.store';
import { CHECK_DELAY_MS, QuerySession } from './query-session';
import { SearchField } from './search-field';
import { FakeContext, loadWorkspace, provideAnalysisTesting, stubBackend } from './test-setup';

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

describe('SearchField', () => {
  let field: SearchField;
  let store: WorkspaceStore;
  let session: QuerySession;
  let backend: AnalysisBackend;
  let classify: ReturnType<typeof vi.fn>;

  /** Real command context (the applied query lives on the workspace query); classification via the stub. */
  async function setup(classifier?: (text: string) => Promise<SearchInputKind>): Promise<void> {
    backend = stubBackend();
    const original = backend.classifySearch.bind(backend);
    classify = vi.fn(classifier ?? ((text: string) => original(text, [])));
    (backend as unknown as { classifySearch: unknown }).classifySearch = classify;
    TestBed.configureTestingModule({ providers: provideAnalysisTesting({ backend }) });
    store = await loadWorkspace();
    field = TestBed.inject(SearchField);
    session = TestBed.inject(QuerySession);
    field.start();
    await settle();
  }

  /** Sets what the workspace query holds from outside the field (undo, saved query, project). */
  async function outside(patch: { ql?: string; text?: string }): Promise<void> {
    store.restoreQuery({ ...store.query(), ql: patch.ql ?? '', text: patch.text ?? '' } as never);
    await settle();
  }

  afterEach(() => vi.useRealTimers());

  it('start() is idempotent', async () => {
    await setup();
    field.start();
    field.start();
    await outside({ text: 'abc' });
    expect(field.text()).toBe('abc');
  });

  describe('edit and classification', () => {
    beforeEach(() => setup());

    it('plain words stay text and apply as the message search', async () => {
      await field.edit('payment failed', 14);
      expect(field.kind()).toBe('text');
      expect(field.text()).toBe('payment failed');
      expect(store.textInput()).toBe('payment failed');
      expect(field.classifying()).toBe(false);
      expect(field.unrun()).toBe(false);
    });

    it('an empty text is text without asking the engine', async () => {
      await field.edit('   ', 3);
      expect(classify).not.toHaveBeenCalled();
      expect(field.kind()).toBe('text');
    });

    it('text with query structure is a query: shown, not applied until run', async () => {
      await field.edit('level >= warning', 16);
      expect(field.kind()).toBe('query');
      expect(field.unrun()).toBe(true);
      expect(session.applied()).toBe('');
      expect(store.textInput()).toBe('');
      expect(classify).toHaveBeenCalledWith('level >= warning', expect.anything());
    });

    it('becoming a query drops the message search that was applied', async () => {
      await field.edit('abc', 3);
      expect(store.textInput()).toBe('abc');
      await field.edit('level = error', 13);
      expect(store.textInput()).toBe('');
      expect(field.kind()).toBe('query');
    });

    it('a regular-expression search is always text', async () => {
      store.setRegex(true);
      await field.edit('level >= warning', 16);
      expect(classify).not.toHaveBeenCalled();
      expect(field.kind()).toBe('text');
      expect(store.textInput()).toBe('level >= warning');
    });

    it('a failing classification falls back to text', async () => {
      TestBed.resetTestingModule();
      await setup(() => Promise.reject(new Error('IPC down')));
      await field.edit('level >= warning', 16);
      expect(field.kind()).toBe('text');
      expect(field.classifying()).toBe(false);
    });

    it('editing clears a previous run error', async () => {
      session.runError.set('boom');
      await field.edit('x', 1);
      expect(session.runError()).toBeNull();
    });

    it('a stale classification is dropped', async () => {
      TestBed.resetTestingModule();
      const slow: Array<(k: SearchInputKind) => void> = [];
      await setup(() => new Promise<SearchInputKind>((r) => slow.push(r)));
      const first = field.edit('level >= a', 10);
      const second = field.edit('words', 5);
      expect(field.classifying()).toBe(true);
      slow[1]?.('text');
      await second;
      expect(field.kind()).toBe('text');
      expect(field.classifying()).toBe(false);
      slow[0]?.('query');
      await first;
      expect(field.kind()).toBe('text');
      expect(store.textInput()).toBe('words');
    });
  });

  describe('submit', () => {
    beforeEach(() => setup());

    it('returns "text" for a message search (it already applies)', async () => {
      await field.edit('hello', 5);
      expect(await field.submit()).toBe('text');
    });

    it('waits for the classification, then runs the query', async () => {
      void field.edit('level >= error', 14);
      expect(await field.submit()).toBe('events');
      expect(session.applied()).toBe('level >= error');
      expect(field.unrun()).toBe(false);
      // The run's own change to the workspace query is not mistaken for an outside one.
      await settle();
      expect(field.text()).toBe('level >= error');
      expect(field.kind()).toBe('query');
    });

    it('returns what the session says when the run fails', async () => {
      await field.edit('level >= error', 14);
      vi.spyOn(session, 'run').mockResolvedValue(null);
      expect(await field.submit()).toBeNull();
    });

    it('runs a table query without applying it', async () => {
      await field.edit('| stats count()', 14);
      vi.spyOn(session, 'run').mockResolvedValue('table');
      expect(await field.submit()).toBe('table');
    });
  });

  describe('clear, hidden text', () => {
    beforeEach(() => setup());

    it('clear empties the field and removes what it applied', async () => {
      await field.edit('level >= error', 14);
      await field.submit();
      session.error.set({ message: 'x', offset: 0, len: 1, suggestions: [] });
      session.completions.set([{ label: 'a', insert: 'a', kind: 'field' } as never]);
      field.clear();
      expect(field.text()).toBe('');
      expect(field.kind()).toBe('text');
      expect(session.error()).toBeNull();
      expect(session.completions()).toEqual([]);
      expect(session.applied()).toBe('');
      expect(store.textInput()).toBe('');
      await settle();
      expect(field.text()).toBe('');
    });

    it('clear cancels an in-flight classification', async () => {
      const pending = field.edit('level >= error', 14);
      field.clear();
      await pending;
      expect(field.kind()).toBe('text');
      expect(field.classifying()).toBe(false);
      expect(field.text()).toBe('');
    });

    it('hiddenText shows a message search that still applies under a query, and dropHiddenText removes it', async () => {
      expect(field.hiddenText()).toBeNull();
      store.patchQuery({ ql: 'level >= error', text: '' } as never);
      expect(field.hiddenText()).toBeNull();
      store.patchQuery({ ql: 'level >= error', text: 'old search' } as never);
      expect(field.hiddenText()).toBe('old search');
      store.patchQuery({ ql: '', text: 'old search' } as never);
      expect(field.hiddenText()).toBeNull();
      store.patchQuery({ ql: 'level >= error', text: 'old search' } as never);
      field.dropHiddenText();
      expect(store.textInput()).toBe('');
      await settle();
      // Dropping it is the field's own change: the field keeps showing nothing odd.
      expect(field.kind()).toBe('text');
      await wait(10);
    });
  });

  describe('following outside changes', () => {
    beforeEach(() => setup());

    it('a query set from outside shows in the field as a query', async () => {
      await outside({ ql: 'level >= error' });
      expect(field.kind()).toBe('query');
      expect(field.text()).toBe('level >= error');
      expect(field.unrun()).toBe(false);
      await wait(CHECK_DELAY_MS + 20);
    });

    it('does not reload the field when it already shows that query', async () => {
      await field.edit('level >= error', 14);
      const load = vi.spyOn(session, 'load');
      await outside({ ql: 'level >= error' });
      expect(load).not.toHaveBeenCalled();
      expect(field.kind()).toBe('query');
    });

    it('a message search set from outside shows as text', async () => {
      await outside({ text: 'timeout' });
      expect(field.kind()).toBe('text');
      expect(field.text()).toBe('timeout');
    });

    it('does not rewrite the field when it already shows that text', async () => {
      await field.edit('abc', 3);
      store.restoreQuery({ ...store.query(), text: 'abc' });
      await settle();
      expect(field.text()).toBe('abc');
    });

    it('switching from a query to a text drops the query (undo)', async () => {
      await outside({ ql: 'level >= error' });
      expect(field.kind()).toBe('query');
      await outside({ text: 'plain' });
      expect(field.kind()).toBe('text');
      expect(field.text()).toBe('plain');
      expect(session.applied()).toBe('');
    });

    it('switching from a text to a query drops the text', async () => {
      await outside({ text: 'plain' });
      await outside({ ql: 'level = fatal' });
      expect(field.kind()).toBe('query');
      expect(field.text()).toBe('level = fatal');
    });

    it('clearing the workspace query empties the field and its error', async () => {
      await outside({ ql: 'level >= error' });
      session.error.set({ message: 'x', offset: 0, len: 1, suggestions: [] });
      await outside({});
      expect(field.kind()).toBe('text');
      expect(field.text()).toBe('');
      expect(session.error()).toBeNull();
    });

    it('an outside change supersedes a classification in flight', async () => {
      TestBed.resetTestingModule();
      let release: ((k: SearchInputKind) => void) | undefined;
      await setup(() => new Promise<SearchInputKind>((r) => (release = r)));
      const pending = field.edit('level >= error', 14);
      await outside({ text: 'other' });
      expect(field.classifying()).toBe(false);
      release?.('query');
      await pending;
      expect(field.kind()).toBe('text');
      expect(field.text()).toBe('other');
    });
  });

  it('uses the injected context when given (no workspace dependency in the unit)', async () => {
    const ctx = new FakeContext();
    TestBed.configureTestingModule({ providers: provideAnalysisTesting({ context: ctx }) });
    await loadWorkspace();
    const f = TestBed.inject(SearchField);
    await f.edit('level >= error', 14);
    await f.submit();
    expect(ctx.last('setQuery')).toEqual([{ ql: 'level >= error' }]);
  });
});
