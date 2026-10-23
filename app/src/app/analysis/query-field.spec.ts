import { Component, signal } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { By } from '@angular/platform-browser';
import { patchState } from '@ngrx/signals';
import { all, el, settle } from '../../testing/helpers';
import { at } from '../../testing/must';
import { AiLauncher } from '../ai/ai-launcher';
import { AiStatusStore } from '../ai/ai-status.store';
import { AnalysisBackend } from '../core/analysis-backend';
import { QueryProgress, StreamedQueryOutcome } from '../core/api.types';
import { WorkspaceStore } from '../core/workspace.store';
import { SmartSearchStore } from '../core/workspace/smart-search.store';
import { SavedQueryStore } from '../projects/bookmarks/saved-query.store';
import { CommandRegistry } from '../extensions/command-registry';
import { CHECK_DELAY_MS, QuerySession } from './query-session';
import { focusSoon, MAX_FIELD_LINES, progressText, QueryField, QueryStatus } from './query-field';
import { SearchField } from './search-field';
import { FakeContext, MemoryHistory, loadWorkspace, provideAnalysisTesting } from './test-setup';

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

function key(input: HTMLElement, k: string, init: KeyboardEventInit = {}): KeyboardEvent {
  const e = new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true, ...init });
  input.dispatchEvent(e);
  return e;
}

/** The filter bar's pair: the field and the status line under it (the shell wires `pick` to `useSuggestion`). */
@Component({
  selector: 'debrief-test-host',
  imports: [QueryField, QueryStatus],
  template: `<debrief-query-field #f /><debrief-query-status (pick)="f.useSuggestion($event)" />`,
})
class Host {}

describe('QueryField', () => {
  let fixture: ComponentFixture<Host>;
  let fieldCmp: QueryField;
  let statusCmp: QueryStatus;
  let ctx: FakeContext;
  let history: MemoryHistory;
  let input: HTMLTextAreaElement;
  let session: QuerySession;
  let field: SearchField;
  let store: WorkspaceStore;

  const refresh = () => settle(fixture);

  const type = async (text: string, caret = text.length) => {
    input.focus();
    input.dispatchEvent(new Event('focus'));
    input.value = text;
    input.setSelectionRange(caret, caret);
    input.dispatchEvent(new Event('input', { bubbles: true }));
    await wait(CHECK_DELAY_MS + 80);
    await refresh();
  };

  const statusEl = () => el(fixture, 'debrief-query-status');
  const statusText = () => statusEl().textContent ?? '';

  const create = async (providers = provideAnalysisTesting({ context: ctx, history })) => {
    TestBed.configureTestingModule({ providers });
    store = await loadWorkspace();
    fixture = TestBed.createComponent(Host);
    session = TestBed.inject(QuerySession);
    field = TestBed.inject(SearchField);
    fixture.detectChanges();
    fieldCmp = fixture.debugElement.query(By.directive(QueryField)).componentInstance;
    statusCmp = fixture.debugElement.query(By.directive(QueryStatus)).componentInstance;
    input = el<HTMLTextAreaElement>(fixture, 'textarea');
  };

  beforeEach(async () => {
    ctx = new FakeContext();
    history = new MemoryHistory(['stats-old', 'level >= fatal']);
    await create();
  });

  afterEach(() => vi.useRealTimers());

  it('is an accessible combobox in text mode with plain-search controls', () => {
    expect(input.getAttribute('role')).toBe('combobox');
    expect(input.getAttribute('aria-label')).toContain('Search messages');
    expect(input.getAttribute('aria-expanded')).toBe('false');
    expect(input.getAttribute('aria-invalid')).toBeNull();
    expect(input.placeholder).toBe('Search or query…');
    expect(fixture.nativeElement.querySelector('.mirror')).toBeNull();
    expect(fixture.nativeElement.querySelector('[data-testid=query-run]')).toBeNull();
    expect(all(fixture, '.toggle')).toHaveLength(2);
  });

  it('typing words searches the messages (text mode)', async () => {
    await type('payment');
    expect(field.kind()).toBe('text');
    expect(store.textInput()).toBe('payment');
    expect(el<HTMLButtonElement>(fixture, '[data-testid=query-save]').disabled).toBe(false);
  });

  it('the save button is disabled for an empty field', () => {
    expect(el<HTMLButtonElement>(fixture, '[data-testid=query-save]').disabled).toBe(true);
  });

  describe('regex and match-case toggles', () => {
    it('toggle the workspace query and swap the placeholder', async () => {
      const [regex, matchCase] = all<HTMLButtonElement>(fixture, '.toggle') as [HTMLButtonElement, HTMLButtonElement];
      regex.click();
      matchCase.click();
      await refresh();
      expect(store.query().regex).toBe(true);
      expect(store.query().caseSensitive).toBe(true);
      expect(regex.getAttribute('aria-pressed')).toBe('true');
      expect(input.placeholder).toBe('Regular expression');
    });

    it('an invalid regular expression marks the field invalid', async () => {
      store.setRegex(true);
      await type('(unclosed');
      store.patchQuery({ text: '(unclosed' });
      await refresh();
      expect(input.getAttribute('aria-invalid')).toBe('true');
      expect(el(fixture, '.box').classList.contains('invalid')).toBe(true);
      store.patchQuery({ text: 'fine' });
      await refresh();
      expect(input.getAttribute('aria-invalid')).toBeNull();
      store.patchQuery({ text: '(?P<n>a)' });
      await refresh();
      expect(input.getAttribute('aria-invalid')).toBeNull();
    });

    it('are hidden while the field holds a query', async () => {
      await type('level >= warning');
      expect(all(fixture, '.toggle')).toHaveLength(0);
      expect(el<HTMLButtonElement>(fixture, '[data-testid=query-run]').textContent).toContain('Run');
    });
  });

  it('shows how many partial matches the scan added, in text mode only', async () => {
    patchState(TestBed.inject(SmartSearchStore) as never, { added: 1 });
    await refresh();
    expect(el(fixture, '[data-testid=partial-note]').textContent).toContain('+1');
    expect(el(fixture, '[data-testid=partial-note]').getAttribute('title')).toContain('1 more partial match added');
    expect(el(fixture, '[data-testid=partial-note]').getAttribute('aria-label')).toContain('1 more partial match added');
    patchState(TestBed.inject(SmartSearchStore) as never, { added: 3 });
    await refresh();
    expect(el(fixture, '[data-testid=partial-note]').getAttribute('title')).toContain('3 more partial matches added');
    await type('level >= warning');
    expect(fixture.nativeElement.querySelector('[data-testid=partial-note]')).toBeNull();
  });

  describe('query mode', () => {
    it('highlights the typed query', async () => {
      await type('level >= warning | stats count()');
      const mirror = el(fixture, '.mirror');
      expect(mirror.getAttribute('aria-hidden')).toBe('true');
      expect(mirror.textContent).toBe('level >= warning | stats count()');
      expect(mirror.querySelector('.kw')?.textContent).toBe('stats');
      expect(mirror.querySelector('.fn')?.textContent).toBe('count');
      expect(input.classList.contains('see-through')).toBe(true);
    });

    it('underlines the error span and explains it under the bar', async () => {
      await type('level >= warning | frob');
      expect(el(fixture, '.err').textContent).toBe('frob');
      expect(input.getAttribute('aria-invalid')).toBe('true');
      expect(input.getAttribute('aria-describedby')).toBe('ql-error');
      expect(el(fixture, '.box').classList.contains('invalid')).toBe(true);
      const message = statusEl().querySelector('#ql-error');
      expect(message?.textContent).toContain("Unknown stage 'frob'");
      expect(message?.textContent).toContain('column 20');
      expect(message?.getAttribute('role')).toBe('alert');
    });

    it('marks a zero-length error position with a caret', async () => {
      await type('level =');
      expect(el(fixture, '.err-caret')).toBeTruthy();
      expect(statusText()).toContain('Expected a value');
    });

    it('marks the previous stats table as stale while the text has an error', async () => {
      session.table.set({ kind: 'table', columns: ['n'], rows: [[1]] });
      session.tableText.set('| stats count()');
      await type('| stats count()');
      expect(session.tableStale()).toBeNull();
      await type('| stats count( by');
      expect(session.tableStale()).toBe('error');
    });

    it('the mirror follows the field scroll', async () => {
      await type('level >= warning | stats count()');
      input.scrollLeft = 0;
      input.dispatchEvent(new Event('scroll'));
      input.dispatchEvent(new Event('keyup'));
      input.dispatchEvent(new Event('click'));
      fixture.detectChanges();
      expect((el(fixture, '.shift') as HTMLElement).style.transform).toContain('translate(');
    });
  });

  describe('completions', () => {
    it('suggests and accepts one with the keyboard (Tab)', async () => {
      await type('level >= error | st');
      const options = all(fixture, '[role=option]');
      expect(options.length).toBeGreaterThan(0);
      expect(input.getAttribute('aria-expanded')).toBe('true');
      expect(input.getAttribute('aria-controls')).toBe('ql-completions');
      expect(input.getAttribute('aria-activedescendant')).toBe('ql-opt-0');
      expect(options[0]?.getAttribute('aria-selected')).toBe('true');
      if (options.length > 1) {
        key(input, 'ArrowDown');
        fixture.detectChanges();
        expect(input.getAttribute('aria-activedescendant')).toBe('ql-opt-1');
        key(input, 'ArrowUp');
        key(input, 'ArrowUp');
        fixture.detectChanges();
        expect(input.getAttribute('aria-activedescendant')).toBe(`ql-opt-${options.length - 1}`);
        key(input, 'ArrowDown');
      }
      const e = key(input, 'Tab');
      await refresh();
      expect(e.defaultPrevented).toBe(true);
      expect(input.value).toMatch(/^level >= error \| st\w+/);
      expect(session.text()).toBe(input.value);
    });

    it('accepts with Enter without running, or with a click, putting the caret inside count()', async () => {
      await type('level >= error | st');
      const before = input.value;
      key(input, 'Enter');
      await refresh();
      expect(input.value).not.toBe(before);
      expect(ctx.calls.some((c) => c.fn === 'setQuery')).toBe(false);

      await type('x | stats co');
      const count = all(fixture, '[role=option]').find((o) => o.textContent?.includes('count') && !o.textContent.includes('count_'));
      expect(count).toBeTruthy();
      count!.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, cancelable: true }));
      count!.click();
      await refresh();
      expect(input.value).toBe('x | stats count()');
      expect(input.selectionStart).toBe(16);
    });

    it('Shift+Enter does not accept a completion', async () => {
      await type('x | stats co');
      const e = key(input, 'Enter', { shiftKey: true });
      expect(e.defaultPrevented).toBe(false);
      expect(input.value).toBe('x | stats co');
    });

    it('Escape closes the popup, Ctrl+Space reopens it, blur closes it', async () => {
      await type('x | stats co');
      expect(all(fixture, '[role=option]').length).toBeGreaterThan(0);
      const esc = key(input, 'Escape');
      fixture.detectChanges();
      expect(esc.defaultPrevented).toBe(true);
      expect(all(fixture, '[role=option]')).toHaveLength(0);
      expect(field.text()).toBe('x | stats co');
      const space = key(input, ' ', { ctrlKey: true });
      expect(space.defaultPrevented).toBe(true);
      await wait(30);
      await refresh();
      expect(all(fixture, '[role=option]').length).toBeGreaterThan(0);
      input.dispatchEvent(new Event('blur'));
      fixture.detectChanges();
      expect(all(fixture, '[role=option]')).toHaveLength(0);
    });

    it('clicking outside closes popups', async () => {
      await type('x | stats co');
      document.body.dispatchEvent(new Event('pointerdown', { bubbles: true }));
      fixture.detectChanges();
      expect(all(fixture, '[role=option]')).toHaveLength(0);
    });

    it('arrow keys with Alt do not move through completions', async () => {
      await type('x | stats co');
      const e = key(input, 'ArrowDown', { altKey: true });
      expect(e.defaultPrevented).toBe(true);
    });
  });

  describe('running', () => {
    it('Ctrl+Enter, Cmd+Enter, plain Enter and the Run button run the query', async () => {
      await type('level >= error');
      session.dismissCompletions();
      const ctrl = key(input, 'Enter', { ctrlKey: true });
      expect(ctrl.defaultPrevented).toBe(true);
      await wait(30);
      expect(ctx.last('setQuery')).toEqual([{ ql: 'level >= error' }]);
      ctx.calls.length = 0;
      key(input, 'Enter', { metaKey: true });
      await wait(30);
      expect(ctx.last('setQuery')).toEqual([{ ql: 'level >= error' }]);
      ctx.calls.length = 0;
      key(input, 'Enter');
      await wait(30);
      expect(ctx.calls.filter((c) => c.fn === 'setQuery')).toHaveLength(1);
      ctx.calls.length = 0;
      el<HTMLButtonElement>(fixture, 'button.run').click();
      await wait(30);
      expect(ctx.last('setQuery')).toEqual([{ ql: 'level >= error' }]);
    });

    it('the Run button is highlighted while the query is not the applied one', async () => {
      await type('level >= error');
      expect(el(fixture, 'button.run').classList.contains('pending')).toBe(true);
    });

    it('Enter in a message search runs nothing', async () => {
      await type('hello');
      key(input, 'Enter');
      await wait(30);
      expect(ctx.calls.some((c) => c.fn === 'setQuery')).toBe(false);
    });

    it('Shift+Enter inserts a new line instead of running', async () => {
      await type('level >= error');
      const e = key(input, 'Enter', { shiftKey: true });
      expect(e.defaultPrevented).toBe(false);
      await wait(30);
      expect(ctx.calls.some((c) => c.fn === 'setQuery')).toBe(false);
    });

    it('a stats query opens the results view instead of filtering', async () => {
      await type('| stats count() by level');
      session.dismissCompletions();
      key(input, 'Enter', { ctrlKey: true });
      await wait(40);
      expect(ctx.last('showView')).toEqual(['query-results']);
      expect(ctx.calls.some((c) => c.fn === 'setQuery')).toBe(false);
    });

    it('shows run errors under the bar', async () => {
      await type('level >= error');
      vi.spyOn(session, 'run').mockImplementation(async () => {
        session.runError.set('IPC down');
        return null;
      });
      el<HTMLButtonElement>(fixture, 'button.run').click();
      await refresh();
      expect(statusText()).toContain('IPC down');
      expect(input.getAttribute('aria-describedby')).toBe('ql-error');
    });

    it('shows progress and a Cancel button while a query runs; Cancel and Esc stop it', async () => {
      const backend = TestBed.inject(AnalysisBackend);
      let progress: ((p: QueryProgress) => void) | undefined;
      let finish: ((o: StreamedQueryOutcome) => void) | undefined;
      vi.spyOn(backend, 'runQueryStreaming').mockImplementation((_q, _id, onProgress) => {
        progress = onProgress;
        return new Promise((r) => (finish = r));
      });
      const cancel = vi.spyOn(backend, 'cancelQuery').mockImplementation(async () => (finish?.({ cancelled: true }), true));
      expect(fixture.nativeElement.querySelector('[data-testid=query-cancel]')).toBeNull();
      await type('level >= warning');
      session.dismissCompletions();
      el<HTMLButtonElement>(fixture, 'button.run').click();
      await refresh();
      expect(statusText()).toContain('Running…');
      progress?.({ stage: 'evaluating', done: 250, total: 1000 });
      await refresh();
      expect(statusText()).toContain('Evaluating 250 of 1,000 events (25%)');
      expect(statusEl().querySelector('progress')?.getAttribute('aria-label')).toBe('Query progress');
      el<HTMLButtonElement>(fixture, '[data-testid=query-cancel]').click();
      await refresh();
      expect(cancel).toHaveBeenCalledTimes(1);
      await wait(10);
      await refresh();
      expect(statusEl().querySelector('[data-testid=query-cancelled]')?.textContent).toContain('Query cancelled.');
      expect(fixture.nativeElement.querySelector('[data-testid=query-cancel]')).toBeNull();

      // Esc cancels too (when no completion popup is open).
      el<HTMLButtonElement>(fixture, 'button.run').click();
      await refresh();
      const esc = key(input, 'Escape');
      await refresh();
      expect(esc.defaultPrevented).toBe(true);
      expect(cancel).toHaveBeenCalledTimes(2);
      expect(session.running()).toBe(false);
    });

    it('Escape with text and nothing else going on clears the field', async () => {
      await type('level >= error');
      session.dismissCompletions();
      const esc = key(input, 'Escape');
      expect(esc.defaultPrevented).toBe(true);
      await refresh();
      expect(field.text()).toBe('');
      expect(field.kind()).toBe('text');
      const again = key(input, 'Escape');
      expect(again.defaultPrevented).toBe(false);
    });

    it('shows "Cancelling…" while the cancel is in flight', async () => {
      await type('level >= warning');
      session.running.set(true);
      session.cancelling.set(true);
      await refresh();
      expect(statusText()).toContain('Cancelling…');
      expect(el<HTMLButtonElement>(fixture, '[data-testid=query-cancel]').disabled).toBe(true);
    });

    it('formats progress', () => {
      expect(progressText(null)).toBeNull();
      expect(progressText({ stage: 'matching', done: 0, total: 0 })).toBeNull();
      expect(progressText({ stage: 'matching', done: 5, total: 10 })).toBe('Matching 5 of 10 events (50%)');
      expect(progressText({ stage: 'evaluating', done: 12, total: 10 })).toContain('(100%)');
    });
  });

  describe('history', () => {
    it('is offered in a menu, picks an entry and clears', async () => {
      const historyBtn = el<HTMLButtonElement>(fixture, '[data-testid=query-history]');
      expect(historyBtn.disabled).toBe(false);
      historyBtn.click();
      fixture.detectChanges();
      expect(historyBtn.getAttribute('aria-expanded')).toBe('true');
      const items = all<HTMLButtonElement>(fixture, '.history .item');
      expect(items.map((i) => i.textContent?.trim())).toEqual(['stats-old', 'level >= fatal', 'Clear history']);
      at(items, 1).click();
      await wait(20);
      await refresh();
      expect(input.value).toBe('level >= fatal');
      expect(field.text()).toBe('level >= fatal');
      expect(all(fixture, '.history')).toHaveLength(0);

      historyBtn.click();
      fixture.detectChanges();
      all<HTMLButtonElement>(fixture, '.history .item').pop()!.click();
      fixture.detectChanges();
      expect(session.history.entries()).toEqual([]);
      expect(historyBtn.disabled).toBe(true);
      expect(all(fixture, '.history')).toHaveLength(0);
    });

    it('clicking outside the history menu closes it', () => {
      el<HTMLButtonElement>(fixture, '[data-testid=query-history]').click();
      fixture.detectChanges();
      expect(all(fixture, '.history')).toHaveLength(1);
      document.body.dispatchEvent(new Event('pointerdown', { bubbles: true }));
      fixture.detectChanges();
      expect(all(fixture, '.history')).toHaveLength(0);
    });

    it('Alt+arrows walk through it', async () => {
      const walk = async (k: string) => {
        key(input, k, { altKey: true });
        await wait(10);
        await refresh();
      };
      await walk('ArrowUp'); // not in the history: the newest
      expect(session.text()).toBe('stats-old');
      await walk('ArrowUp');
      expect(session.text()).toBe('level >= fatal');
      await walk('ArrowUp'); // oldest already
      expect(session.text()).toBe('level >= fatal');
      await walk('ArrowDown');
      expect(session.text()).toBe('stats-old');
      await walk('ArrowDown'); // newest already
      expect(session.text()).toBe('stats-old');
      await type('unrelated');
      await walk('ArrowDown'); // not in history, newer: nothing
      expect(session.text()).toBe('unrelated');
    });

    it('Alt+arrows do nothing with an empty history', async () => {
      session.history.clear();
      await type('x');
      key(input, 'ArrowUp', { altKey: true });
      expect(session.text()).toBe('x');
    });
  });

  describe('focus', () => {
    it('focuses the input when the focus command fires', async () => {
      input.blur();
      session.requestFocus();
      fixture.detectChanges();
      await settle(fixture);
      expect(document.activeElement).toBe(input);
    });

    it('focuses when the command ran before the field was created, once', async () => {
      fixture.destroy();
      session.requestFocus();
      fixture = TestBed.createComponent(Host);
      fixture.detectChanges();
      await settle(fixture);
      input = el<HTMLTextAreaElement>(fixture, 'textarea');
      expect(document.activeElement).toBe(input);
      input.blur();
      fixture.destroy();
      fixture = TestBed.createComponent(Host);
      fixture.detectChanges();
      await settle(fixture);
      expect(document.activeElement).not.toBe(el(fixture, 'textarea'));
    });

    it('focuses on a "focus the search" request from the workspace', async () => {
      input.blur();
      store.focusSearch();
      fixture.detectChanges();
      await wait(20);
      expect(document.activeElement).toBe(input);
    });

    it('ignores a stale focus request', () => {
      vi.useFakeTimers();
      session.requestFocus();
      vi.advanceTimersByTime(6000);
      expect(session.takeFocusRequest()).toBe(false);
    });

    it('focusSoon retries while the panel is not visible yet', async () => {
      const e = document.createElement('input');
      let calls = 0;
      e.focus = () => void calls++;
      focusSoon(e, 2);
      await wait(180);
      expect(calls).toBe(3);
    });
  });

  describe('multi-line growth', () => {
    it('grows while focused and shrinks on blur', async () => {
      expect(MAX_FIELD_LINES).toBeGreaterThan(1);
      await type('level >= error\nand source = web');
      await wait(5);
      expect(el(fixture, '.box').classList.contains('expanded')).toBe(true);
      expect(input.classList.contains('wrap')).toBe(true);
      input.dispatchEvent(new Event('blur'));
      await wait(5);
      fixture.detectChanges();
      expect(el(fixture, '.box').classList.contains('expanded')).toBe(false);
      expect(input.style.height).toBe('28px');
    });
  });

  describe('long single-line text', () => {
    it('shows its start again on blur and keeps the full text as the tooltip', async () => {
      await type('level >= error and source = web');
      let scrollLeft = 120;
      Object.defineProperty(input, 'scrollLeft', { get: () => scrollLeft, set: (v: number) => (scrollLeft = v), configurable: true });
      expect(input.title).toBe('');
      input.dispatchEvent(new Event('blur'));
      await wait(5);
      fixture.detectChanges();
      expect(scrollLeft).toBe(0);
      expect(input.title).toBe('level >= error and source = web');
    });
  });

  describe('useSuggestion', () => {
    it('replaces the unknown field with the chosen one, via the did-you-mean button', async () => {
      await type('MachinName = 5 and level >= error');
      expect(statusText()).toContain('Unknown field "MachinName"');
      el<HTMLButtonElement>(fixture, '[data-testid=did-you-mean] button').click();
      await wait(CHECK_DELAY_MS + 80);
      await refresh();
      expect(input.value).toBe('MachineName = 5 and level >= error');
      expect(input.selectionStart).toBe('MachineName'.length);
      expect(session.error()).toBeNull();
    });

    it('does nothing without an error', () => {
      fieldCmp.useSuggestion('x');
      expect(field.text()).toBe('');
    });
  });

  describe('save', () => {
    it('saves the current search as a saved query and offers to show the list', async () => {
      const run = vi.fn(async () => true);
      TestBed.inject(CommandRegistry).run = run as never;
      await type('payment');
      el<HTMLButtonElement>(fixture, '[data-testid=query-save]').click();
      const saved = TestBed.inject(SavedQueryStore).items();
      expect(saved).toHaveLength(1);
      expect(store.notice()).toContain(saved[0]!.name);
      store.noticeAction()?.run();
      expect(run).toHaveBeenCalledWith('savedQueries.show', 'button');
    });
  });

  describe('Ask AI', () => {
    it('is shown only when AI is available and opens the query dialog', async () => {
      expect(fixture.nativeElement.querySelector('[data-testid=ai-ask-query]')).toBeNull();
      TestBed.resetTestingModule();
      const askQuery = vi.fn();
      await create([...provideAnalysisTesting({ context: ctx, history }), { provide: AiStatusStore, useValue: { available: signal(true) } }, { provide: AiLauncher, useValue: { askQuery } }]);
      el<HTMLButtonElement>(fixture, '[data-testid=ai-ask-query]').click();
      expect(askQuery).toHaveBeenCalled();
    });
  });

  describe('QueryStatus', () => {
    beforeEach(async () => {
      TestBed.resetTestingModule();
      await create(provideAnalysisTesting({ history }));
    });

    it('renders nothing for a message search', async () => {
      await type('hello');
      expect(statusEl().classList.contains('empty')).toBe(true);
      expect(statusText().trim()).toBe('');
    });

    it('shows the applied query with its match count and clears it', async () => {
      await type('level >= error');
      expect(statusEl().classList.contains('empty')).toBe(true);
      session.dismissCompletions();
      key(input, 'Enter', { ctrlKey: true });
      await wait(60);
      await refresh();
      expect((store.query() as { ql?: string }).ql).toBe('level >= error');
      const applied = statusEl().querySelector('[data-testid=query-applied]');
      expect(applied?.textContent).toContain('Filtering with');
      expect(applied?.textContent).toMatch(/\d+ events/);
      (applied!.querySelector('button') as HTMLButtonElement).click();
      await wait(20);
      await refresh();
      expect((store.query() as { ql?: string }).ql).toBe('');
      expect(statusEl().querySelector('[data-testid=query-applied]')).toBeNull();
      expect(field.text()).toBe('');
    });

    it('an error without a position has no column', async () => {
      session.error.set({ message: 'bad thing', offset: 0, len: 0, suggestions: [] });
      field.kind.set('query');
      await refresh();
      expect(statusText()).toContain('Bad thing');
      expect(statusText()).not.toContain('column');
    });

    it('forwards a picked suggestion', async () => {
      const picked: string[] = [];
      statusCmp.pick.subscribe((n) => picked.push(n));
      await type('MachinName = 5 and level >= error');
      el<HTMLButtonElement>(fixture, '[data-testid=did-you-mean] button').click();
      expect(picked).toEqual(['MachineName']);
    });
  });
});
