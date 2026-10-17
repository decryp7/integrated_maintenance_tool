import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, settle, text, typeInto } from '../../../testing/helpers';
import { RecordView } from '../../core/api.types';
import { HighlightRule } from '../../core/tools.types';
import { WorkspaceStore } from '../../core/workspace.store';
import { UI_STATE_CONTRIBUTIONS, UiStateRegistry } from '../../lifecycle/ui-state';
import { provideToolsTesting } from '../../tools/mock/test-support';
import { ProjectUiState } from '../project-ui-state';
import { BookmarkList } from './bookmark-list';
import { BookmarkStore, parseBookmarks } from './bookmark.store';
import { BookmarksView } from './bookmarks-view';
import { fieldValue, HighlightRules, matchesRule, parseHighlightRules } from './highlight';
import { HighlightEditor } from './highlight-editor';
import { newId } from './ids';
import { SavedQueries } from './saved-queries';
import { parseSavedQueries, SavedQueryStore } from './saved-query.store';

const record = (over: Partial<RecordView> = {}): RecordView => ({
  id: 7, ts: 1_700_000_000_000, level: 'Error', message: 'Failed to capture payment', template: null, exception: 'TimeoutException', fields: { ElapsedMs: 5000, Machine: 'WEB01', Empty: '' },
  source: 0, sourceName: 'app.clef', line: 4, group: 'g', markers: [], ...over,
});
const rule = (over: Partial<HighlightRule>): HighlightRule => ({ id: 'r', field: '@level', op: 'eq', value: 'Error', color: '#f00', enabled: true, ...over });

async function setup() {
  TestBed.configureTestingModule({
    providers: [...provideToolsTesting(), { provide: UI_STATE_CONTRIBUTIONS, multi: true, useFactory: () => TestBed.inject(ProjectUiState).contributions }],
  });
  await settle();
  return { store: TestBed.inject(WorkspaceStore), bookmarks: TestBed.inject(BookmarkStore), queries: TestBed.inject(SavedQueryStore), highlights: TestBed.inject(HighlightRules), registry: TestBed.inject(UiStateRegistry) };
}

describe('ids', () => {
  it('are unique', () => {
    expect(new Set(Array.from({ length: 50 }, () => newId('x'))).size).toBe(50);
  });
});

describe('BookmarkStore', () => {
  it('bookmarks the selected record and the current range, with notes', async () => {
    const { store, bookmarks } = await setup();
    expect(bookmarks.addSelected()).toBe(false);
    expect(bookmarks.addRange()).toBe(false);
    store.select(record({ message: 'x'.repeat(100) }));
    store.setRange(10, 20);
    expect(bookmarks.addSelected('look here')).toBe(true);
    expect(bookmarks.addRange()).toBe(true);
    const [a, b] = bookmarks.items();
    expect(a).toMatchObject({ kind: 'record', recordId: 7, note: 'look here', level: 'Error' });
    expect(a.label).toHaveLength(81);
    expect(b).toMatchObject({ kind: 'range', from: 10, to: 20 });
    bookmarks.setNote(b.id, 'window');
    expect(bookmarks.items()[1].note).toBe('window');
    bookmarks.remove(a.id);
    expect(bookmarks.items()).toHaveLength(1);
  });

  it('jumps to a record or applies a range', async () => {
    const { store, bookmarks } = await setup();
    store.select(record());
    store.setRange(5, 9);
    bookmarks.addSelected();
    bookmarks.addRange();
    store.resetRange();
    bookmarks.jump(bookmarks.items()[0]);
    expect(store.anchor()?.id).toBe(7);
    bookmarks.jump(bookmarks.items()[1]);
    expect(store.query()).toMatchObject({ from: 5, to: 9 });
    bookmarks.jump({ id: 'x', kind: 'record', note: '', label: '', createdAt: 0 });
    bookmarks.jump({ id: 'y', kind: 'range', note: '', label: '', createdAt: 0 });
    expect(store.query()).toMatchObject({ from: 5, to: 9 });
  });

  it('parses saved bookmarks defensively', () => {
    expect(parseBookmarks('x')).toEqual([]);
    expect(
      parseBookmarks([
        null,
        { id: 1 },
        { id: 'a', kind: 'zzz' },
        { id: 'b', kind: 'record' },
        { id: 'c', kind: 'range', from: 1 },
        { id: 'd', kind: 'record', recordId: 3, note: 5, label: 'l', createdAt: 'x' },
        { id: 'e', kind: 'range', from: 1, to: 2, note: 'n', label: 'r', createdAt: 9 },
      ]),
    ).toEqual([
      { id: 'd', kind: 'record', recordId: 3, note: '', label: 'l', createdAt: 0, ts: null, level: undefined },
      { id: 'e', kind: 'range', from: 1, to: 2, note: 'n', label: 'r', createdAt: 9 },
    ]);
  });
});

describe('SavedQueryStore', () => {
  it('saves the current query and re-applies it', async () => {
    const { store, queries } = await setup();
    store.patchQuery({ text: 'timeout', levels: ['Error'] }); store.setTextInput('timeout');
    queries.saveCurrent('  Timeouts ');
    queries.saveCurrent('');
    expect(queries.items().map((q) => q.name)).toEqual(['Timeouts', 'Query 2']);
    store.clearQuery();
    queries.apply(queries.items()[0].id);
    expect(store.query()).toMatchObject({ text: 'timeout', levels: ['Error'] });
    expect(store.textInput()).toBe('timeout');
    queries.apply('missing');
    queries.rename(queries.items()[1].id, ' Renamed ');
    queries.rename(queries.items()[1].id, '  ');
    expect(queries.items()[1].name).toBe('Renamed');
    queries.remove(queries.items()[0].id);
    expect(queries.items()).toHaveLength(1);
  });

  it('saves the search text that is typed but not applied yet', async () => {
    const { store, queries } = await setup();
    store.setTextInput('typed just now');
    queries.saveCurrent('Fast');
    expect(queries.items()[0].query).toMatchObject({ text: 'typed just now' });
  });

  it('parses saved queries defensively', () => {
    expect(parseSavedQueries(null)).toEqual([]);
    const parsed = parseSavedQueries([{ id: 'a', name: 'n', query: { text: 't', junk: 1 } }, { id: 'b' }, null, { id: 'c', name: 'x', query: 'str' }]);
    expect(parsed).toHaveLength(1);
    expect(parsed[0].query).toMatchObject({ text: 't' });
  });
});

describe('highlight rules', () => {
  it('reads fields and pseudo-fields', () => {
    const r = record();
    expect(fieldValue(r, '@level')).toBe('Error');
    expect(fieldValue(r, '@message')).toBe(r.message);
    expect(fieldValue(r, '@exception')).toBe('TimeoutException');
    expect(fieldValue(record({ exception: null }), '@exception')).toBeUndefined();
    expect(fieldValue(r, '@source')).toBe('app.clef');
    expect(fieldValue(r, 'Machine')).toBe('WEB01');
  });

  it('evaluates every condition', () => {
    const r = record();
    const m = (over: Partial<HighlightRule>) => matchesRule(rule(over), r);
    expect(m({ op: 'eq', field: 'Machine', value: 'WEB01' })).toBe(true);
    expect(m({ op: 'eq', field: 'Machine', value: 'WEB02' })).toBe(false);
    expect(m({ op: 'ne', field: 'Machine', value: 'WEB02' })).toBe(true);
    expect(m({ op: 'contains', field: '@message', value: 'PAYMENT' })).toBe(true);
    expect(m({ op: 'notContains', field: '@message', value: 'payment' })).toBe(false);
    expect(m({ op: 'gt', field: 'ElapsedMs', value: '1000' })).toBe(true);
    expect(m({ op: 'lt', field: 'ElapsedMs', value: '1000' })).toBe(false);
    expect(m({ op: 'gt', field: 'Machine', value: '1' })).toBe(false);
    expect(m({ op: 'gt', field: 'ElapsedMs', value: ' ' })).toBe(false);
    expect(m({ op: 'exists', field: 'Machine' })).toBe(true);
    expect(m({ op: 'exists', field: 'Empty' })).toBe(false);
    expect(m({ op: 'exists', field: 'Nope' })).toBe(false);
    expect(m({ op: 'missing', field: 'Nope' })).toBe(true);
    expect(m({ op: 'missing', field: 'Machine' })).toBe(false);
    expect(m({ op: 'eq', field: 'Nope', value: 'x' })).toBe(false);
    expect(m({ op: 'ne', field: 'Nope', value: 'x' })).toBe(true);
    expect(m({ op: 'notContains', field: 'Nope', value: 'x' })).toBe(true);
    expect(m({ op: 'contains', field: 'Nope', value: 'x' })).toBe(false);
  });

  it('publishes enabled rules and colours rows by the first match', async () => {
    const { highlights } = await setup();
    highlights.add({ field: '@level', op: 'eq', value: 'Fatal', color: '#a' });
    highlights.add({ field: '@level', op: 'eq', value: 'Error', color: '#b' });
    highlights.add({ field: 'Machine', op: 'eq', value: 'WEB01', color: '#c' });
    expect(highlights.rules()).toHaveLength(3);
    expect(highlights.colorFor(record())).toBe('#b');
    highlights.update(highlights.items()[1].id, { enabled: false });
    expect(highlights.rules()).toHaveLength(2);
    expect(highlights.colorFor(record())).toBe('#c');
    expect(highlights.colorFor(record({ fields: {} }))).toBeNull();
    highlights.remove(highlights.items()[2].id);
    expect(highlights.colorFor(record())).toBeNull();
  });

  it('parses saved rules defensively', () => {
    expect(parseHighlightRules(5)).toEqual([]);
    expect(parseHighlightRules([null, { id: 'a', field: 'f', op: 'bogus', color: 'c' }, { id: 'b', field: 'f', op: 'eq', value: 3, color: 'c', enabled: false }, { id: 'c', field: 'f', op: 'missing', color: 'x' }])).toEqual([
      { id: 'b', field: 'f', op: 'eq', value: '', color: 'c', enabled: false },
      { id: 'c', field: 'f', op: 'missing', value: '', color: 'x', enabled: true },
    ]);
  });
});

describe('ProjectUiState', () => {
  it('saves and restores bookmarks, queries and highlight rules, and clears them when absent', async () => {
    const { registry, bookmarks, queries, highlights, store } = await setup();
    store.select(record());
    bookmarks.addSelected('n');
    queries.saveCurrent('q');
    highlights.add({ field: 'f', op: 'exists', value: '', color: '#1' });
    const ui = JSON.parse(JSON.stringify(registry.serialize()));
    expect(Object.keys(ui)).toEqual(expect.arrayContaining(['bookmarks', 'savedQueries', 'highlightRules']));
    bookmarks.replace([]);
    queries.replace([]);
    highlights.replace([]);
    registry.restore(ui);
    expect(bookmarks.items()).toHaveLength(1);
    expect(queries.items()).toHaveLength(1);
    expect(highlights.items()).toHaveLength(1);
    registry.restore({ version: 1 });
    expect(bookmarks.items()).toEqual([]);
    expect(queries.items()).toEqual([]);
    expect(highlights.items()).toEqual([]);
  });
});

describe('bookmark views', () => {
  it('lists bookmarks with notes and jumps, deletes and adds from the toolbar', async () => {
    const { store, bookmarks } = await setup();
    const f = TestBed.createComponent(BookmarkList);
    await settle(f);
    expect(buttonByText(f, 'Bookmark selected event').disabled).toBe(true);
    expect(buttonByText(f, 'Bookmark time range').disabled).toBe(true);
    expect(text(f)).toContain('No bookmarks yet');
    store.select(record());
    store.setRange(1_700_000_000_000, 1_700_000_060_000);
    await settle(f);
    buttonByText(f, 'Bookmark selected event').click();
    buttonByText(f, 'Bookmark time range').click();
    await settle(f);
    expect(all(f, '.item')).toHaveLength(2);
    expect(text(f)).toContain('→');
    typeInto(el(f, 'input[aria-label="Note for Failed to capture payment"]'), 'root cause', 'change');
    expect(bookmarks.items()[0].note).toBe('root cause');
    el<HTMLButtonElement>(f, 'button[aria-label="Jump to bookmark Failed to capture payment"]').click();
    expect(store.anchor()?.id).toBe(7);
    el<HTMLButtonElement>(f, 'button[aria-label="Delete bookmark Failed to capture payment"]').click();
    await settle(f);
    expect(all(f, '.item')).toHaveLength(1);
  });

  it('saves and applies queries from the panel', async () => {
    const { store, queries } = await setup();
    const f = TestBed.createComponent(SavedQueries);
    await settle(f);
    expect(text(f)).toContain('No saved queries');
    store.patchQuery({ text: 'abc' }); store.setTextInput('abc');
    typeInto(el(f, 'input[aria-label="Saved query name"]'), 'My query');
    await settle(f);
    buttonByText(f, 'Save current query').click();
    await settle(f);
    expect(queries.items()[0].name).toBe('My query');
    expect(el<HTMLInputElement>(f, 'input').value).toBe('');
    typeInto(el(f, 'input[aria-label="Saved query name"]'), 'Second');
    await settle(f);
    el(f, 'input[aria-label="Saved query name"]').dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter' }));
    await settle(f);
    expect(queries.items()).toHaveLength(2);
    store.clearQuery();
    el<HTMLButtonElement>(f, 'button[aria-label="Apply query My query"]').click();
    expect(store.query().text).toBe('abc');
    el<HTMLButtonElement>(f, 'button[aria-label="Delete query My query"]').click();
    await settle(f);
    expect(queries.items()).toHaveLength(1);
  });

  it('adds, toggles and removes highlight rules', async () => {
    const { highlights } = await setup();
    const f = TestBed.createComponent(HighlightEditor);
    await settle(f);
    expect(text(f)).toContain('No highlight rules');
    expect(buttonByText(f, 'Add rule').disabled).toBe(true);
    typeInto(el(f, 'input[aria-label="Highlight field"]'), '@level');
    typeInto(el(f, 'select[aria-label="Highlight condition"]'), 'eq', 'change');
    await settle(f);
    typeInto(el(f, 'input[aria-label="Highlight value"]'), 'Error');
    el<HTMLButtonElement>(f, 'button[aria-label="Colour #e9ae3f"]').click();
    buttonByText(f, 'Add rule').click();
    await settle(f);
    expect(highlights.items()[0]).toMatchObject({ field: '@level', op: 'eq', value: 'Error', color: '#e9ae3f' });
    typeInto(el(f, 'input[aria-label="Highlight field"]'), 'Machine');
    typeInto(el(f, 'select[aria-label="Highlight condition"]'), 'exists', 'change');
    await settle(f);
    expect(f.nativeElement.querySelector('input[aria-label="Highlight value"]')).toBeNull();
    buttonByText(f, 'Add rule').click();
    await settle(f);
    expect(highlights.items()[1]).toMatchObject({ op: 'exists', value: '' });
    expect(text(f)).toContain('does not equal'.slice(0, 0) + 'equals');
    expect(all(f, 'datalist option').length).toBeGreaterThan(4);
    el<HTMLInputElement>(f, 'input[aria-label="Enable rule @level"]').click();
    expect(highlights.items()[0].enabled).toBe(false);
    el<HTMLButtonElement>(f, 'button[aria-label="Delete rule @level"]').click();
    await settle(f);
    expect(highlights.items()).toHaveLength(1);
  });

  it('composes the three sections', async () => {
    await setup();
    const f = TestBed.createComponent(BookmarksView);
    await settle(f);
    expect(all(f, 'h2').map((h) => h.textContent)).toEqual(['Bookmarks and notes', 'Saved queries', 'Highlight rules']);
  });
});
