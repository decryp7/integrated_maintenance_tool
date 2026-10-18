import { ComponentFixture, TestBed } from '@angular/core/testing';
import { all, el, settle, typeInto } from '../../testing/helpers';
import { MemoryHistory, FakeContext, loadWorkspace, provideAnalysisTesting } from './test-setup';
import { CHECK_DELAY_MS, QuerySession } from './query-session';
import { focusSoon, progressText, QueryView } from './query-view';
import { AnalysisBackend } from '../core/analysis-backend';
import { StreamedQueryOutcome, QueryProgress } from '../core/api.types';
import { at } from '../../testing/must';

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

function key(input: HTMLElement, k: string, init: KeyboardEventInit = {}): KeyboardEvent {
  const e = new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true, ...init });
  input.dispatchEvent(e);
  return e;
}

describe('QueryView', () => {
  let fixture: ComponentFixture<QueryView>;
  let ctx: FakeContext;
  let history: MemoryHistory;
  let input: HTMLInputElement;
  let session: QuerySession;

  const type = async (text: string, caret = text.length) => {
    input.focus();
    input.value = text;
    input.setSelectionRange(caret, caret);
    input.dispatchEvent(new Event('input', { bubbles: true }));
    await wait(CHECK_DELAY_MS + 80);
    await settle(fixture);
  };

  beforeEach(async () => {
    ctx = new FakeContext();
    history = new MemoryHistory(['stats-old', 'level >= fatal']);
    TestBed.configureTestingModule({ providers: provideAnalysisTesting({ context: ctx, history }) });
    await loadWorkspace();
    fixture = TestBed.createComponent(QueryView);
    session = TestBed.inject(QuerySession);
    fixture.detectChanges();
    input = el<HTMLInputElement>(fixture, 'input');
  });

  it('is an accessible combobox with a live status line', () => {
    expect(input.getAttribute('role')).toBe('combobox');
    expect(input.getAttribute('aria-label')).toBe('Query');
    expect(input.getAttribute('aria-expanded')).toBe('false');
    expect(el(fixture, '.status').getAttribute('role')).toBe('status');
    expect(el(fixture, '.mirror').getAttribute('aria-hidden')).toBe('true');
  });

  it('highlights the typed query', async () => {
    await type('level >= warning | stats count()');
    const mirror = el(fixture, '.mirror');
    expect(mirror.textContent).toBe('level >= warning | stats count()');
    expect(mirror.querySelector('.kw')?.textContent).toBe('stats');
    expect(mirror.querySelector('.fn')?.textContent).toBe('count');
  });

  it('underlines the error span and explains it', async () => {
    await type('level >= warning | frob');
    expect(el(fixture, '.err').textContent).toBe('frob');
    expect(input.getAttribute('aria-invalid')).toBe('true');
    expect(input.getAttribute('aria-describedby')).toBe('ql-error');
    expect(el(fixture, '#ql-error').textContent).toContain("unknown stage 'frob'");
    expect(el(fixture, '#ql-error').textContent).toContain('column 20');
    expect(el(fixture, '.editor').classList.contains('invalid')).toBe(true);
  });

  it('marks a zero-length error position with a caret', async () => {
    await type('level =');
    expect(el(fixture, '.err-caret')).toBeTruthy();
    expect(el(fixture, '#ql-error').textContent).toContain('expected a value');
  });

  it('suggests completions and accepts one with the keyboard', async () => {
    await type('lev');
    const options = all(fixture, '[role=option]');
    expect(options.map((o) => o.textContent)).toEqual(expect.arrayContaining([expect.stringContaining('level')]));
    expect(input.getAttribute('aria-expanded')).toBe('true');
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
    await settle(fixture);
    expect(e.defaultPrevented).toBe(true);
    expect(input.value).toBe('level');
    expect(session.text()).toBe('level');
  });

  it('accepts with Enter or a click, and puts the caret inside count()', async () => {
    await type('lev');
    key(input, 'Enter');
    expect(input.value).toBe('level');
    expect(ctx.calls.some((c) => c.fn === 'setQuery')).toBe(false);

    await type('x | stats co');
    const count = all(fixture, '[role=option]').find((o) => o.textContent?.includes('count') && !o.textContent.includes('count_'))!;
    count.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, cancelable: true }));
    count.click();
    await settle(fixture);
    expect(input.value).toBe('x | stats count()');
    expect(input.selectionStart).toBe(16);
  });

  it('Escape closes the popup, Ctrl+Space reopens it', async () => {
    await type('lev');
    expect(all(fixture, '[role=option]').length).toBeGreaterThan(0);
    key(input, 'Escape');
    fixture.detectChanges();
    expect(all(fixture, '[role=option]')).toHaveLength(0);
    key(input, 'Escape');
    key(input, ' ', { ctrlKey: true });
    await wait(30);
    await settle(fixture);
    expect(all(fixture, '[role=option]').length).toBeGreaterThan(0);
    input.dispatchEvent(new Event('blur'));
    fixture.detectChanges();
    expect(all(fixture, '[role=option]')).toHaveLength(0);
  });

  it('runs on Ctrl+Enter, Cmd+Enter and the Run button; plain Enter runs when no popup', async () => {
    await type('level >= error');
    session.dismissCompletions();
    key(input, 'Enter', { ctrlKey: true });
    await wait(20);
    expect(ctx.last('setQuery')).toEqual([{ ql: 'level >= error' }]);
    ctx.calls.length = 0;
    key(input, 'Enter', { metaKey: true });
    await wait(20);
    expect(ctx.last('setQuery')).toEqual([{ ql: 'level >= error' }]);
    ctx.calls.length = 0;
    key(input, 'Enter');
    await wait(20);
    expect(ctx.calls.filter((c) => c.fn === 'setQuery')).toHaveLength(1);
    ctx.calls.length = 0;
    el<HTMLButtonElement>(fixture, 'button.primary').click();
    await wait(20);
    expect(ctx.last('setQuery')).toEqual([{ ql: 'level >= error' }]);
  });

  it('a stats query opens the results view instead of filtering', async () => {
    await type('| stats count() by level');
    session.dismissCompletions();
    key(input, 'Enter', { ctrlKey: true });
    await wait(30);
    expect(ctx.last('showView')).toEqual(['query-results']);
    expect(ctx.calls.some((c) => c.fn === 'setQuery')).toBe(false);
  });

  it('keeps a history, offers it in a menu and walks it with Alt+arrows', async () => {
    const historyBtn = all<HTMLButtonElement>(fixture, 'button').find((b) => b.textContent?.trim() === 'History')!;
    expect(historyBtn.disabled).toBe(false);
    historyBtn.click();
    fixture.detectChanges();
    const items = all<HTMLButtonElement>(fixture, '.history .item');
    expect(items.map((i) => i.textContent?.trim())).toEqual(['stats-old', 'level >= fatal', 'Clear history']);
    at(items, 1).click();
    await settle(fixture);
    expect(input.value).toBe('level >= fatal');
    expect(all(fixture, '.history')).toHaveLength(0);

    key(input, 'ArrowUp', { altKey: true }); // older: none
    await settle(fixture);
    expect(session.text()).toBe('level >= fatal');
    key(input, 'ArrowDown', { altKey: true }); // newer
    expect(session.text()).toBe('stats-old');
    key(input, 'ArrowDown', { altKey: true }); // newest already
    expect(session.text()).toBe('stats-old');
    key(input, 'ArrowUp', { altKey: true });
    expect(session.text()).toBe('level >= fatal');
    session.text.set('unrelated');
    key(input, 'ArrowDown', { altKey: true });
    expect(session.text()).toBe('unrelated');
    key(input, 'ArrowUp', { altKey: true });
    expect(session.text()).toBe('stats-old');

    historyBtn.click();
    fixture.detectChanges();
    all<HTMLButtonElement>(fixture, '.history .item').pop()!.click();
    fixture.detectChanges();
    expect(session.history.entries()).toEqual([]);
    expect(historyBtn.disabled).toBe(true);
    session.text.set('x');
    key(input, 'ArrowUp', { altKey: true });
    expect(session.text()).toBe('x');
  });

  it('focuses the input when the focus command fires', async () => {
    input.blur();
    session.requestFocus();
    fixture.detectChanges();
    await settle(fixture);
    expect(document.activeElement).toBe(input);
  });

  it('focuses when the command ran before the view was created, once', async () => {
    fixture.destroy();
    session.requestFocus();
    fixture = TestBed.createComponent(QueryView);
    fixture.detectChanges();
    await settle(fixture);
    input = el<HTMLInputElement>(fixture, 'input');
    expect(document.activeElement).toBe(input);
    input.blur();
    fixture.destroy();
    fixture = TestBed.createComponent(QueryView);
    fixture.detectChanges();
    await settle(fixture);
    expect(document.activeElement).not.toBe(el(fixture, 'input'));
  });

  it('ignores a stale focus request', () => {
    vi.useFakeTimers();
    session.requestFocus();
    vi.advanceTimersByTime(6000);
    expect(session.takeFocusRequest()).toBe(false);
    vi.useRealTimers();
  });

  it('retries focusing while the panel is not visible yet', async () => {
    const el = document.createElement('input');
    let calls = 0;
    el.focus = () => void calls++;
    focusSoon(el, 2);
    await wait(180);
    expect(calls).toBe(3);
  });

  it('shows the applied query with its match count and clears it', async () => {
    // Real context: the applied ql lives on the workspace query.
    TestBed.resetTestingModule();
    TestBed.configureTestingModule({ providers: provideAnalysisTesting({ history }) });
    const store = await loadWorkspace();
    fixture = TestBed.createComponent(QueryView);
    session = TestBed.inject(QuerySession);
    fixture.detectChanges();
    input = el<HTMLInputElement>(fixture, 'input');
    expect(el(fixture, '.status').textContent).toContain('to run');
    await type('level >= error');
    session.dismissCompletions();
    key(input, 'Enter', { ctrlKey: true });
    await wait(50);
    await settle(fixture);
    expect((store.query() as { ql?: string }).ql).toBe('level >= error');
    expect(el(fixture, '.applied').textContent).toContain('Filtering with');
    expect(el(fixture, '.applied').textContent).toMatch(/\d+ events/);
    el<HTMLButtonElement>(fixture, '.applied button').click();
    await settle(fixture);
    expect((store.query() as { ql?: string }).ql).toBe('');
    expect(all(fixture, '.applied')).toHaveLength(0);
  });

  it('shows run errors', async () => {
    await type('x');
    vi.spyOn(session, 'run').mockImplementation(async () => {
      session.runError.set('IPC down');
      return null;
    });
    el<HTMLButtonElement>(fixture, 'button.primary').click();
    await settle(fixture);
    expect(el(fixture, '.err-msg').textContent).toContain('IPC down');
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
    el<HTMLButtonElement>(fixture, 'button.primary').click();
    await settle(fixture);
    expect(el<HTMLButtonElement>(fixture, 'button.primary').disabled).toBe(true);
    expect(el(fixture, '.status').textContent).toContain('Running…');
    progress?.({ stage: 'evaluating', done: 250, total: 1000 });
    await settle(fixture);
    expect(el(fixture, '.status').textContent).toContain('Evaluating 250 of 1,000 events (25%)');
    expect(el<HTMLProgressElement>(fixture, 'progress').getAttribute('aria-label')).toBe('Query progress');
    el<HTMLButtonElement>(fixture, '[data-testid=query-cancel]').click();
    await settle(fixture);
    expect(cancel).toHaveBeenCalledTimes(1);
    expect(el(fixture, '[data-testid=query-cancelled]').textContent).toContain('Query cancelled.');
    expect(fixture.nativeElement.querySelector('.err-msg')).toBeNull();
    expect(fixture.nativeElement.querySelector('[data-testid=query-cancel]')).toBeNull();

    // Esc cancels too (when no completion popup is open).
    el<HTMLButtonElement>(fixture, 'button.primary').click();
    await settle(fixture);
    const esc = key(input, 'Escape');
    await settle(fixture);
    expect(esc.defaultPrevented).toBe(true);
    expect(cancel).toHaveBeenCalledTimes(2);
    expect(session.running()).toBe(false);
  });

  it('formats progress', () => {
    expect(progressText(null)).toBeNull();
    expect(progressText({ stage: 'matching', done: 0, total: 0 })).toBeNull();
    expect(progressText({ stage: 'matching', done: 5, total: 10 })).toBe('Matching 5 of 10 events (50%)');
    expect(progressText({ stage: 'evaluating', done: 12, total: 10 })).toContain('(100%)');
  });

  it('clicking outside closes popups', async () => {
    await type('lev');
    document.body.dispatchEvent(new Event('pointerdown', { bubbles: true }));
    fixture.detectChanges();
    expect(all(fixture, '[role=option]')).toHaveLength(0);
    typeInto(input, 'lev');
  });
});
