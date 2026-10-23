import { Dialog, DialogRef } from '@angular/cdk/dialog';
import { TestBed } from '@angular/core/testing';
import { el, all, MemoryLogger, provideTinyBackend, tinyBackend, typeInto } from '../../testing/helpers';
import { at, must } from '../../testing/must';
import { appConfig } from '../app.config';
import { isActivityTracked } from '../core/activity/backend-activity';
import { DocHit, KindHits, SearchAllReply } from '../core/api.types';
import { CmdError } from '../core/cmd-error';
import { Logger } from '../core/logger';
import { WorkspaceStore } from '../core/workspace.store';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { CommandContext, flattenContributions, DEBRIEF_COMMANDS, DebriefCommand } from '../extensions/tokens';
import { NOTES_VIEW } from '../notes/note-actions';
import { ExceptionFocus } from '../analysis/exception-focus';
import { KnowledgeBackend } from '../ai/knowledge/knowledge-backend';
import { KnowledgeEditor } from '../ai/knowledge/knowledge-editor';
import { DialogService } from '../ui/dialog.service';
import { MockAnalysisBackend, MockKnowledgeBackend, MockNotesBackend, recordsFrom } from '../mock-backends';
import { EMPTY_QUERY } from '../core/workspace/query.store';
import { mockSnippet, MockSearchEverythingBackend } from './mock/mock-search-everything-backend';
import { provideSearchEverything, SEARCH_EVERYTHING_COMMAND, SEARCH_EVERYTHING_KEYBINDING, searchEverythingCommands } from './provide-search-everything';
import { flatRows, KIND_ORDER, outcomeNotice, resultGroups } from './search-everything.model';
import { SearchEverythingBackend, TauriSearchEverythingBackend } from './search-everything-backend';
import { SearchEverythingLauncher } from './search-everything-launcher';
import { EVENTS_VIEW, EXCEPTIONS_VIEW, SearchEverythingNavigator } from './search-everything-navigator';
import { SearchEverythingPalette } from './search-everything-palette';
import { SEARCH_DEBOUNCE_MS, SearchEverythingStore } from './search-everything.store';

const none = <T>(): KindHits<T> => ({ total: 0, items: [], outcome: { kind: 'complete' } });
const part = (text: string, mark = false) => ({ text, mark });
const doc = (id: string, patch: Partial<DocHit> = {}): DocHit => ({ id, scope: 'project', title: `Title ${id}`, event: null, snippet: [part('the '), part('timeout', true)], ...patch });

function reply(patch: Partial<SearchAllReply> = {}): SearchAllReply {
  return { events: none(), sources: none(), fields: none(), markers: none(), exceptions: none(), notes: none(), knowledge: none(), ...patch };
}

const full = reply({
  events: { total: 3, items: [{ id: 4, ts: 1000, level: 'Error', source: 'app.clef', snippet: [part('Failed: '), part('timeout', true)] }], outcome: { kind: 'complete' } },
  sources: { total: 1, items: [{ id: 1, name: 'app.log', path: '/logs/timeout/app.log', records: 4, snippet: [part('/logs/'), part('timeout', true)] }], outcome: { kind: 'complete' } },
  fields: {
    total: 2,
    items: [
      { field: 'TimeoutMs', value: null, count: 9, snippet: [part('Timeout', true), part('Ms')] },
      { field: 'Reason', value: 'timeout', count: 2, snippet: [part('timeout', true)] },
    ],
    outcome: { kind: 'timedOut' },
  },
  markers: { total: 1, items: [{ kind: 'timeout', snippet: [part('timeout', true)] }], outcome: { kind: 'complete' } },
  exceptions: { total: 1, items: [{ fingerprint: 'fp1', typeName: 'TimeoutException', count: 12, sample: 9, snippet: [part('Timeout', true), part('Exception: gateway')] }], outcome: { kind: 'complete' } },
  notes: { total: 1, items: [doc('n1', { event: 4 })], outcome: { kind: 'complete' } },
  knowledge: { total: 1, items: [doc('k1', { scope: 'global' })], outcome: { kind: 'failed', message: 'busy' } },
});

class FakeBackend extends SearchEverythingBackend {
  readonly calls: string[] = [];
  answer: (text: string) => Promise<SearchAllReply | null> = async () => full;
  searchEverything(request: { text: string }) {
    this.calls.push(request.text);
    return this.answer(request.text);
  }
}

const ctx = () => ({ notifyError: vi.fn(), showView: vi.fn(), jumpTo: vi.fn(async (_id: number) => undefined), setQuery: vi.fn(), addFilter: vi.fn() });

function provide(backend: SearchEverythingBackend = new FakeBackend(), context = ctx()) {
  TestBed.configureTestingModule({ providers: [...provideTinyBackend(), { provide: SearchEverythingBackend, useValue: backend }, { provide: COMMAND_CONTEXT, useValue: context as unknown as CommandContext }] });
  return { backend, context };
}

describe('search everything backend', () => {
  it('maps to the Tauri command', async () => {
    const invoke = vi.fn(async () => null);
    await new TauriSearchEverythingBackend(invoke as never).searchEverything({ text: 'timeout' });
    expect(invoke).toHaveBeenCalledWith('search_everything', { request: { text: 'timeout' } });
  });

  it('is the mock behind the shared proxy in a browser', () => {
    TestBed.configureTestingModule({ providers: provideTinyBackend() });
    const backend = TestBed.inject(SearchEverythingBackend);
    expect(isActivityTracked(backend)).toBe(true);
  });

  it('the mock searches the mock workspace by words', async () => {
    const workspace = tinyBackend();
    const notes = new MockNotesBackend();
    const knowledge = new MockKnowledgeBackend();
    const analysis = new MockAnalysisBackend(recordsFrom(workspace));
    const mock = new MockSearchEverythingBackend(workspace, { notes: () => notes, knowledge: () => knowledge, analysis: () => analysis });
    expect(await mock.searchEverything({ text: ' -- ' })).toBeNull();
    await expect(mock.searchEverything({ text: 'x'.repeat(257) })).rejects.toBeInstanceOf(CmdError);
    const found = must(await mock.searchEverything({ text: 'web02' }));
    expect(found.sources.items.map((s) => s.id)).toEqual([1]);
    expect(found.sources.items[0]?.snippet.filter((p) => p.mark).map((p) => p.text)).toEqual(['web02']);
    const fields = must(await mock.searchEverything({ text: 'elapsed', limit: 1 }));
    expect(fields.fields.items.map((f) => f.field)).toEqual(['ElapsedMs']);
    const events = must(await mock.searchEverything({ text: 'payment' }));
    expect(events.events.total).toBe(1);
    expect(at(events.events.items, 0).level).toBe('Error');
    // Notes and knowledge files, like the engine: every word, in the title or the text.
    await notes.addNote({ anchor: { kind: 'project' }, text: 'Gateway retries\nThey start after the deploy.' });
    await knowledge.createKnowledge('global', 'Glossary', 'ACME-GW is the payment gateway.');
    const docs = must(await mock.searchEverything({ text: 'gateway deploy' }));
    expect(docs.notes.items.map((n) => [n.id, n.title, n.scope])).toEqual([['n1', 'Gateway retries', 'project']]);
    expect(docs.knowledge.total).toBe(0);
    const glossary = must(await mock.searchEverything({ text: 'glossary payment' }));
    expect(glossary.knowledge.items.map((k) => [k.title, k.scope])).toEqual([['Glossary', 'global']]);
    expect(glossary.notes.total).toBe(0);
    // Values of fields with few distinct values.
    const summary = await workspace.workspaceSummary();
    const field = must(summary.fields.find((f) => !f.numeric && f.distinct > 0 && f.distinct <= 500));
    const facet = await workspace.queryFacet({ query: { ...EMPTY_QUERY }, field: field.name, limit: 500 });
    const value = must(facet.values[0]).value;
    const valueHits = must(await mock.searchEverything({ text: value })).fields.items.filter((f) => f.value === value && f.field === field.name);
    expect(valueHits.length, `${field.name} = ${value}`).toBe(1);
    // Exception groups by type, message or top frame.
    const group = must((await analysis.queryExceptions({ ...EMPTY_QUERY })).result[0]);
    const byType = must(await mock.searchEverything({ text: group.typeName }));
    expect(byType.exceptions.items.map((x) => x.fingerprint)).toContain(group.fingerprint);
    expect(mockSnippet('A  timeout,\nTIMEOUT', ['timeout'])).toEqual([part('A '), part('timeout', true), part(', '), part('TIMEOUT', true)]);
  });
});

describe('search everything model', () => {
  it('groups the kinds in order with their notices, and only those with something to show', () => {
    const groups = resultGroups(full, 'timeout');
    expect(groups.map((g) => g.kind)).toEqual(KIND_ORDER);
    expect(groups.map((g) => g.title)).toEqual(['Events', 'Sources', 'Fields', 'Markers', 'Exceptions', 'Notes', 'Knowledge']);
    expect(groups.find((g) => g.kind === 'fields')?.notice).toMatch(/time ran out/);
    expect(groups.find((g) => g.kind === 'knowledge')?.notice).toBe('Could not search: busy');
    expect(resultGroups(null, 'x')).toEqual([]);
    expect(resultGroups(reply(), 'x')).toEqual([]);
    expect(resultGroups(reply({ notes: { ...none(), outcome: { kind: 'timedOut' } } }), 'x').map((g) => g.kind)).toEqual(['notes']);
    expect(outcomeNotice({ kind: 'complete' })).toBeNull();
  });

  it('builds rows that say where they go', () => {
    const rows = flatRows(resultGroups(full, 'timeout'));
    expect(rows.map((r) => r.target)).toEqual([
      { kind: 'event', id: 4 },
      { kind: 'allEvents', text: 'timeout' },
      { kind: 'source', id: 1 },
      { kind: 'field', field: 'TimeoutMs', value: null },
      { kind: 'field', field: 'Reason', value: 'timeout' },
      { kind: 'marker', marker: 'timeout' },
      { kind: 'exception', fingerprint: 'fp1', sample: 9 },
      { kind: 'note', id: 'n1', event: 4 },
      { kind: 'knowledge', scope: 'global', id: 'k1' },
    ]);
    expect(new Set(rows.map((r) => r.key)).size).toBe(rows.length);
    const byKey = (key: string) => must(rows.find((r) => r.key === key));
    expect(byKey('event:4')).toMatchObject({ detail: 'Error · app.clef', ts: 1000 });
    expect(at(byKey('events:all').snippet, 0).text).toBe('Show all 3 matching events in the grid');
    expect(byKey('field:Reason:timeout').snippet).toEqual([part('Reason = '), part('timeout', true)]);
    expect(byKey('source:1').detail).toBe('4 events');
    expect(byKey('exception:fp1').detail).toBe('12 events');
    expect(byKey('knowledge:global:k1').detail).toBe('Title k1 · Global');
    expect(byKey('notes:project:n1').detail).toBe('Title n1');
    // A doc without a snippet shows its title; all events are not offered when every match is shown.
    const bare = reply({ events: { ...full.events, total: 1 }, knowledge: { total: 1, items: [doc('k2', { snippet: [] })], outcome: { kind: 'complete' } } });
    const bareRows = flatRows(resultGroups(bare, 'x'));
    expect(bareRows.some((r) => r.target.kind === 'allEvents')).toBe(false);
    expect(must(bareRows.find((r) => r.key === 'knowledge:project:k2'))).toMatchObject({ snippet: [part('Title k2')], detail: 'Title k2 · Project' });
  });
});

describe('search everything store', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('searches after a pause in typing and drops answers to older texts', async () => {
    const { backend } = provide();
    const fake = backend as FakeBackend;
    let release: (r: SearchAllReply | null) => void = () => undefined;
    fake.answer = (text) => (text === 'tim' ? new Promise((r) => (release = r)) : Promise.resolve(full));
    const store = TestBed.inject(SearchEverythingStore);
    store.setText('t');
    store.setText('tim');
    await vi.advanceTimersByTimeAsync(SEARCH_DEBOUNCE_MS);
    expect(fake.calls).toEqual(['tim']);
    expect(store.loading()).toBe(true);
    store.setText('timeout');
    await vi.advanceTimersByTimeAsync(SEARCH_DEBOUNCE_MS);
    expect(store.searched()).toBe('timeout');
    release(reply());
    await vi.advanceTimersByTimeAsync(0);
    expect(store.reply()).toBe(full);
    expect(store.rows().length).toBe(9);
    expect(store.loading()).toBe(false);

    store.setText('   ');
    await vi.advanceTimersByTimeAsync(SEARCH_DEBOUNCE_MS);
    expect(store.reply()).toBeNull();
    expect(fake.calls).toEqual(['tim', 'timeout']);
    store.reset();
    expect(store.text()).toBe('');
  });

  it('shows a failure and retries', async () => {
    const { backend } = provide();
    const fake = backend as FakeBackend;
    fake.answer = () => Promise.reject(new CmdError({ code: 'invalid', message: 'search for at most 256 characters', retryable: false }));
    const store = TestBed.inject(SearchEverythingStore);
    store.setText('x');
    await store.searchNow();
    expect(store.error()).toBe('search for at most 256 characters');
    fake.answer = () => Promise.reject(new Error('gone'));
    await store.searchNow();
    expect(store.error()).toBe('gone');
    fake.answer = () => Promise.reject('plain');
    await store.searchNow();
    expect(store.error()).toBe('plain');
    fake.answer = async () => full;
    await store.searchNow();
    expect(store.error()).toBeNull();
    expect(store.groups().length).toBe(7);
  });
});

describe('search everything navigation', () => {
  it('goes where each result points', () => {
    const { context } = provide();
    const dialogs = TestBed.inject(DialogService);
    vi.spyOn(dialogs, 'open').mockImplementation(() => undefined as never);
    const nav = TestBed.inject(SearchEverythingNavigator);
    nav.go({ kind: 'event', id: 4 });
    expect(context.jumpTo).toHaveBeenLastCalledWith(4);
    expect(context.showView).toHaveBeenLastCalledWith(EVENTS_VIEW);
    nav.go({ kind: 'source', id: 1 });
    expect(context.setQuery).toHaveBeenLastCalledWith({ sources: [1] });
    nav.go({ kind: 'field', field: 'TimeoutMs', value: null });
    expect(context.addFilter).toHaveBeenLastCalledWith({ field: 'TimeoutMs', op: 'exists' });
    nav.go({ kind: 'field', field: 'Reason', value: 'timeout' });
    expect(context.addFilter).toHaveBeenLastCalledWith({ field: 'Reason', op: 'eq', value: 'timeout' });
    nav.go({ kind: 'marker', marker: 'crash' });
    expect(context.setQuery).toHaveBeenLastCalledWith({ markers: ['crash'] });
    context.jumpTo.mockClear();
    nav.go({ kind: 'note', id: 'n2', event: null });
    expect(context.showView).toHaveBeenLastCalledWith(NOTES_VIEW);
    expect(context.jumpTo).not.toHaveBeenCalled();
    nav.go({ kind: 'note', id: 'n1', event: 9 });
    expect(context.jumpTo).toHaveBeenLastCalledWith(9);
    context.jumpTo.mockClear();
    nav.go({ kind: 'exception', fingerprint: 'fp1', sample: 9 });
    expect(context.showView).toHaveBeenLastCalledWith(EXCEPTIONS_VIEW);
    expect(TestBed.inject(ExceptionFocus).fingerprint()).toBe('fp1');
    expect(context.jumpTo).toHaveBeenLastCalledWith(9);
    context.jumpTo.mockClear();
    nav.go({ kind: 'exception', fingerprint: 'fp2', sample: null });
    expect(TestBed.inject(ExceptionFocus).fingerprint()).toBe('fp2');
    expect(context.jumpTo).not.toHaveBeenCalled();
    nav.go({ kind: 'allEvents', text: 'timeout' });
    expect(TestBed.inject(WorkspaceStore).textInput()).toBe('timeout');
  });
});

describe('search everything knowledge results', () => {
  it('open the file in the knowledge editor, listing the files first when they were not listed yet', async () => {
    const { context } = provide();
    const backend = TestBed.inject(KnowledgeBackend);
    const made = await backend.createKnowledge('global', 'Glossary', 'ACME-GW');
    const open = vi.spyOn(TestBed.inject(DialogService), 'open').mockImplementation(() => undefined as never);
    const nav = TestBed.inject(SearchEverythingNavigator);
    nav.go({ kind: 'knowledge', scope: 'global', id: made.id });
    await vi.waitFor(() => expect(open).toHaveBeenCalledTimes(1));
    expect(open).toHaveBeenCalledWith(KnowledgeEditor, expect.objectContaining({ inputs: expect.objectContaining({ file: expect.objectContaining({ id: made.id, name: 'Glossary', scope: 'global' }), scope: 'global' }) }));
    // A file that is gone opens nothing.
    nav.go({ kind: 'knowledge', scope: 'project', id: 'missing' });
    await new Promise((r) => setTimeout(r));
    expect(open).toHaveBeenCalledTimes(1);
    expect(context.notifyError).toHaveBeenCalledWith('This knowledge file no longer exists.');
  });
});

describe('search everything palette', () => {
  async function palette(backend = new FakeBackend()) {
    const close = vi.fn();
    const { context } = provide(backend);
    TestBed.overrideProvider(DialogRef, { useValue: { close } });
    const f = TestBed.createComponent(SearchEverythingPalette);
    f.detectChanges();
    return { f, close, context, backend };
  }

  it('lists the results by kind with the words marked and goes to the chosen one', async () => {
    const { f, close, context } = await palette();
    const input = el<HTMLInputElement>(f, '[data-testid=search-everything-input]');
    expect(input.getAttribute('role')).toBe('combobox');
    typeInto(input, 'timeout');
    await TestBed.inject(SearchEverythingStore).searchNow();
    f.detectChanges();
    expect(all(f, '[role=option]').length).toBe(9);
    expect(all(f, '[role=group]').length).toBe(7);
    expect(el(f, '[data-testid=search-group-events] mark').textContent).toBe('timeout');
    expect(el(f, '[data-testid=search-group-fields]').textContent).toContain('time ran out');
    expect(input.getAttribute('aria-activedescendant')).toBe(at(all(f, '[role=option]'), 0).id);
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown' }));
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown' }));
    f.detectChanges();
    expect(at(all(f, '[role=option]'), 2).getAttribute('aria-selected')).toBe('true');
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter' }));
    expect(close).toHaveBeenCalled();
    expect(context.setQuery).toHaveBeenCalledWith({ sources: [1] });
    // A click chooses too.
    at(all(f, '[role=option]'), 0).dispatchEvent(new MouseEvent('mousemove'));
    at(all(f, '[role=option]'), 0).click();
    expect(context.jumpTo).toHaveBeenCalledWith(4);
  });

  it('searches at once on Enter before the pause ended, says when nothing matches and offers Retry after a failure', async () => {
    const backend = new FakeBackend();
    backend.answer = async () => reply();
    const { f } = await palette(backend);
    const input = el<HTMLInputElement>(f, '[data-testid=search-everything-input]');
    typeInto(input, 'nothing');
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter' }));
    await new Promise((r) => setTimeout(r));
    f.detectChanges();
    expect(el(f, '[data-testid=search-everything-empty]').textContent).toContain('nothing');
    backend.answer = () => Promise.reject(new Error('the store is busy'));
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter' }));
    await new Promise((r) => setTimeout(r));
    f.detectChanges();
    expect(el(f, '[data-testid=search-everything-error]').textContent).toBe('the store is busy');
    backend.answer = async () => full;
    must(all<HTMLButtonElement>(f, 'button').find((b) => b.textContent?.includes('Retry'))).click();
    await new Promise((r) => setTimeout(r));
    f.detectChanges();
    expect(all(f, '[role=option]').length).toBe(9);
    expect(f.nativeElement.getAttribute('data-activity-trigger')).toBe('palette');
  });
});

describe('search everything command and launcher', () => {
  it('opens one palette in a CDK dialog on Mod+Shift+F, and focuses it again when open', () => {
    provide();
    const launcher = TestBed.inject(SearchEverythingLauncher);
    const dialog = TestBed.inject(Dialog);
    const command = at(searchEverythingCommands(launcher), 0);
    expect(command).toMatchObject({ id: SEARCH_EVERYTHING_COMMAND, keybinding: 'Mod+Shift+F' });
    command.run({} as CommandContext);
    expect(launcher.isOpen).toBe(true);
    expect(dialog.openDialogs.length).toBe(1);
    expect(document.querySelector('debrief-search-everything')).not.toBeNull();
    launcher.open();
    expect(dialog.openDialogs.length).toBe(1);
    launcher.close();
    expect(launcher.isOpen).toBe(false);
    launcher.close();
  });

  it('is the only command of the app on its key', () => {
    TestBed.configureTestingModule({ providers: [...appConfig.providers, { provide: Logger, useFactory: () => new MemoryLogger() }] });
    const commands = flattenContributions<DebriefCommand>(TestBed.inject(DEBRIEF_COMMANDS, null) ?? []);
    const bound = commands.filter((c) => c.keybinding?.toLowerCase() === SEARCH_EVERYTHING_KEYBINDING.toLowerCase());
    expect(bound.map((c) => c.id)).toEqual([SEARCH_EVERYTHING_COMMAND]);
    expect(typeof provideSearchEverything).toBe('function');
  });
});
