import { Perspectives } from '../dock/perspectives';
import { Component, input, InputSignal, Injector, runInInjectionContext } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { firstValueFrom } from 'rxjs';
import { provideFakeDock, provideTinyBackendBare, provideTinyBackend, settle, FakeDock } from '../../testing/helpers';
import { RecordView } from '../core/api.types';
import { WorkspaceStore } from '../core/workspace.store';
import {
  COMMAND_CONTEXT,
  CommandContext,
  CommandRegistry,
  EVENT_ID_FORMATTER,
  FieldFormatters,
  findFormatter,
  flattenContributions,
  formatKeybinding,
  fuzzyFilter,
  fuzzyScore,
  DEBRIEF_COMMANDS,
  DEBRIEF_FIELD_FORMATTERS,
  DEBRIEF_INSPECTOR_SECTIONS,
  DEBRIEF_VIEWS,
  DebriefCommand,
  DebriefEvents,
  DebriefView,
  InspectorSections,
  isViewAvailable,
  JUMP_TIMEOUT_MS,
  matchesKeybinding,
  provideDebriefPlugin,
  ViewRegistry,
  WorkspaceCommandContext,
} from './index';

const key = (init: KeyboardEventInit & { key: string }) => new KeyboardEvent('keydown', { cancelable: true, ...init });

describe('fuzzy matching', () => {
  it('scores in-order subsequences and rejects others', () => {
    expect(fuzzyScore('', 'anything')).toBe(0);
    expect(fuzzyScore('xyz', 'Open files')).toBeNull();
    expect(fuzzyScore('of', 'Open files')).not.toBeNull();
    // consecutive and word-start matches rank higher
    expect(fuzzyScore('open', 'Open files')!).toBeGreaterThan(fuzzyScore('opfs', 'Open files')!);
    expect(fuzzyScore('fi', 'Open files')!).toBeGreaterThan(fuzzyScore('fe', 'Open files')!);
  });

  it('filters and ranks, keeping order for an empty query', () => {
    const items = ['Open folder', 'Close workspace', 'Open files'];
    expect(fuzzyFilter(items, '  ', (s) => s)).toEqual(items);
    expect(fuzzyFilter(items, 'cw', (s) => s)).toEqual(['Close workspace']);
    expect(fuzzyFilter(items, 'open', (s) => s)).toEqual(['Open files', 'Open folder']);
    expect(fuzzyFilter(items, 'zzz', (s) => s)).toEqual([]);
  });
});

describe('keybindings', () => {
  it('matches modifiers, Mod per platform and arrow aliases', () => {
    expect(matchesKeybinding('Mod+K', key({ key: 'k', ctrlKey: true }), false)).toBe(true);
    expect(matchesKeybinding('Mod+K', key({ key: 'k', metaKey: true }), true)).toBe(true);
    expect(matchesKeybinding('Mod+K', key({ key: 'k', metaKey: true }), false)).toBe(false);
    expect(matchesKeybinding('Mod+K', key({ key: 'k' }), false)).toBe(false);
    expect(matchesKeybinding('Mod+Shift+P', key({ key: 'P', ctrlKey: true, shiftKey: true }), false)).toBe(true);
    expect(matchesKeybinding('Alt+Left', key({ key: 'ArrowLeft', altKey: true }), false)).toBe(true);
    expect(matchesKeybinding('Alt+Right', key({ key: 'ArrowRight', altKey: true }), false)).toBe(true);
    expect(matchesKeybinding('Ctrl+Meta+X', key({ key: 'x', ctrlKey: true, metaKey: true }), false)).toBe(true);
    expect(matchesKeybinding('', key({ key: 'k' }), false)).toBe(false);
  });

  it('detects the platform by default', () => {
    expect(typeof matchesKeybinding('Mod+K', key({ key: 'k' }))).toBe('boolean');
    expect(formatKeybinding('Mod+K')).toMatch(/K/);
  });

  it('formats hints for macOS and other platforms', () => {
    expect(formatKeybinding('Mod+Shift+P', true)).toBe('⌘⇧P');
    expect(formatKeybinding('Mod+Shift+P', false)).toBe('Ctrl+Shift+P');
    expect(formatKeybinding('Alt+Left', false)).toBe('Alt+←');
    expect(formatKeybinding('Alt+Right', true)).toBe('⌥→');
    expect(formatKeybinding('Mod+F5', false)).toBe('Ctrl+F5');
  });
});

describe('contributions', () => {
  it('flattens batches and single items', () => {
    expect(flattenContributions<number>(null)).toEqual([]);
    expect(flattenContributions<number>([[1, 2], 3, [4]])).toEqual([1, 2, 3, 4]);
  });

  it('provideDebriefPlugin registers each kind once per batch and skips empty ones', () => {
    const view: DebriefView = { id: 'v', title: 'V', icon: 'panel', component: async () => Component as never };
    const command: DebriefCommand = { id: 'c', title: 'C', run: () => undefined };
    const formatter = { field: 'F', format: () => 'x' };
    @Component({ selector: 'debrief-test-section', template: '' })
    class Section {
      readonly record = input.required<RecordView>();
    }
    TestBed.configureTestingModule({
      providers: [
        provideDebriefPlugin({ views: [view], commands: [command], fieldFormatters: [formatter], inspectorSections: [{ id: 's', title: 'S', order: 1, component: Section }] }),
        provideDebriefPlugin({}),
      ],
    });
    expect(flattenContributions(TestBed.inject(DEBRIEF_VIEWS))).toEqual([view]);
    expect(flattenContributions(TestBed.inject(DEBRIEF_COMMANDS))).toEqual([command]);
    expect(flattenContributions(TestBed.inject(DEBRIEF_FIELD_FORMATTERS))).toEqual([formatter]);
    expect(flattenContributions(TestBed.inject(DEBRIEF_INSPECTOR_SECTIONS))).toHaveLength(1);
    expect(TestBed.inject(DEBRIEF_VIEWS, null)).toBeTruthy();
  });
});

describe('ViewRegistry', () => {
  const mk = (id: string, when?: DebriefView['when']): DebriefView => ({ id, title: id, icon: 'panel', component: async () => Component as never, when });

  function registry(views: DebriefView[]) {
    TestBed.configureTestingModule({
      providers: [provideTinyBackendBare(), provideFakeDock(), { provide: DEBRIEF_VIEWS, multi: true, useValue: views }, { provide: DEBRIEF_VIEWS, multi: true, useValue: [mk('dup')] }],
    });
    return TestBed.inject(ViewRegistry);
  }

  it('dedupes by id and finds views', () => {
    const r = registry([mk('a'), mk('dup'), mk('dup')]);
    expect(r.all.map((v) => v.id)).toEqual(['a', 'dup']);
    expect(r.find('a')?.id).toBe('a');
    expect(r.find('zzz')).toBeUndefined();
  });

  it('offers conditional views only when their condition holds for the summary', async () => {
    const r = registry([mk('always'), mk('csv', (s) => s.sources.some((x) => x.importer === 'csv')), mk('json', (s) => s.sources.some((x) => x.importer === 'json')), mk('boom', () => { throw new Error('x'); })]);
    expect(r.available().map((v) => v.id)).toEqual(['always', 'dup']);
    TestBed.inject(WorkspaceStore);
    await settle();
    expect(r.available().map((v) => v.id)).toEqual(['always', 'json', 'dup']);
  });

  it('isViewAvailable handles missing summary and missing condition', () => {
    expect(isViewAvailable(mk('x'), null)).toBe(true);
    expect(isViewAvailable(mk('x', () => true), null)).toBe(false);
  });

  it('isViewAvailable honours enabled (a feature that may be absent), whatever the workspace', () => {
    expect(isViewAvailable({ ...mk('x'), enabled: () => false }, null)).toBe(false);
    expect(isViewAvailable({ ...mk('x'), enabled: () => true }, null)).toBe(true);
    expect(
      isViewAvailable(
        {
          ...mk('x'),
          enabled: () => {
            throw new Error('broken');
          },
        },
        null,
      ),
    ).toBe(false);
  });
});

describe('CommandRegistry', () => {
  let ran: string[];
  let ctx: CommandContext & { summary: unknown };

  function setup(commands: DebriefCommand[]) {
    ran = [];
    ctx = { summary: null, openPaths: async () => undefined, setQuery: () => undefined, addFilter: () => undefined, selectRecord: () => undefined, jumpTo: async () => undefined, showView: () => undefined, notifyError: () => undefined };
    TestBed.configureTestingModule({
      providers: [provideTinyBackendBare(), provideFakeDock(), { provide: COMMAND_CONTEXT, useValue: ctx }, { provide: DEBRIEF_COMMANDS, multi: true, useValue: commands }],
    });
    return { registry: TestBed.inject(CommandRegistry), store: TestBed.inject(WorkspaceStore) };
  }
  const cmd = (id: string, extra: Partial<DebriefCommand> = {}): DebriefCommand => ({ id, title: `Do ${id}`, run: () => void ran.push(id), ...extra });

  it('lists, dedupes, searches with the category and respects `when`', () => {
    const { registry } = setup([cmd('a'), cmd('a'), cmd('b', { category: 'File', title: 'Open things' }), cmd('hidden', { when: () => false }), cmd('boom', { when: () => { throw new Error('x'); } })]);
    expect(registry.all().map((c) => c.id)).toEqual(['a', 'b', 'hidden', 'boom']);
    expect(registry.available().map((c) => c.id)).toEqual(['a', 'b']);
    expect(registry.search('open file').map((c) => c.id)).toEqual([]);
    expect(registry.search('file: open').map((c) => c.id)).toEqual(['b']);
    expect(registry.search('').map((c) => c.id)).toEqual(['a', 'b']);
    expect(registry.find('b')?.title).toBe('Open things');
  });

  it('runs commands and reports failures to the user', async () => {
    const { registry, store } = setup([cmd('ok'), cmd('bad', { run: () => { throw new Error('nope'); } }), cmd('async-bad', { run: async () => Promise.reject('plain') }), cmd('off', { when: () => false })]);
    expect(await registry.run('ok')).toBe(true);
    expect(ran).toEqual(['ok']);
    expect(await registry.run('missing')).toBe(false);
    expect(await registry.run('off')).toBe(false);
    expect(await registry.run('bad')).toBe(false);
    expect(store.error()).toBe('Do bad failed: nope');
    expect(await registry.run('async-bad')).toBe(false);
    expect(store.error()).toBe('Do async-bad failed: plain');
  });

  it('runs the command bound to a key', () => {
    const { registry } = setup([cmd('k', { keybinding: 'Ctrl+J' }), cmd('plain')]);
    const hit = key({ key: 'j', ctrlKey: true });
    expect(registry.runForKey(hit)).toBe(true);
    expect(hit.defaultPrevented).toBe(true);
    expect(ran).toEqual(['k']);
    expect(registry.runForKey(key({ key: 'q' }))).toBe(false);
  });
});

describe('FieldFormatters', () => {
  const record = { id: 1, fields: {} } as unknown as RecordView;

  it('picks the first matching formatter by name or pattern', () => {
    const exact = { field: 'A', format: () => 'exact' };
    const re = { field: /^A/, format: () => 'regex' };
    expect(findFormatter([re, exact], 'A')).toBe(re);
    expect(findFormatter([exact, re], 'A')).toBe(exact);
    expect(findFormatter([exact], 'B')).toBeUndefined();
  });

  it('formats EventId objects and falls back to plain strings', () => {
    TestBed.configureTestingModule({
      providers: [
        { provide: DEBRIEF_FIELD_FORMATTERS, multi: true, useValue: [EVENT_ID_FORMATTER] },
        { provide: DEBRIEF_FIELD_FORMATTERS, multi: true, useValue: [{ field: 'Bad', format: () => { throw new Error('x'); } }, { field: 'Upper', format: (v: unknown) => String(v).toUpperCase() }] },
      ],
    });
    const f = TestBed.inject(FieldFormatters);
    expect(f.format('EventId', { Id: 10100, Name: 'OrderPlaced' }, record)).toBe('10100 · OrderPlaced');
    expect(f.format('EventId', { Id: 7 }, record)).toBe('7');
    expect(f.format('EventId', 42, record)).toBe('42');
    expect(f.format('Upper', 'abc', record)).toBe('ABC');
    expect(f.format('Bad', 'raw', record)).toBe('raw');
    expect(f.format('Other', { a: 1 }, record)).toBe('{"a":1}');
  });

  it('works without any registered formatter', () => {
    TestBed.configureTestingModule({});
    expect(TestBed.inject(FieldFormatters).format('X', 5, record)).toBe('5');
  });
});

describe('InspectorSections', () => {
  @Component({ selector: 'debrief-test-a', template: '' })
  class A {
    readonly record: InputSignal<RecordView> = input.required<RecordView>();
  }

  it('orders sections and filters them per record', () => {
    TestBed.configureTestingModule({
      providers: [
        {
          provide: DEBRIEF_INSPECTOR_SECTIONS,
          multi: true,
          useValue: [
            { id: 'late', title: 'Late', order: 200, component: A },
            { id: 'early', title: 'Early', order: 100, component: A },
            { id: 'warn', title: 'Warn only', order: 150, component: A, when: (r: RecordView) => r.level === 'Warning' },
            { id: 'boom', title: 'Boom', order: 160, component: A, when: () => { throw new Error('x'); } },
          ],
        },
      ],
    });
    const s = TestBed.inject(InspectorSections);
    expect(s.forRecord({ level: 'Information' } as RecordView).map((x) => x.id)).toEqual(['early', 'late']);
    expect(s.forRecord({ level: 'Warning' } as RecordView).map((x) => x.id)).toEqual(['early', 'warn', 'late']);
  });

  it('is empty without contributions', () => {
    TestBed.configureTestingModule({});
    expect(TestBed.inject(InspectorSections).forRecord({} as RecordView)).toEqual([]);
  });
});

describe('DebriefEvents and the command context', () => {
  function setup() {
    const dock = new FakeDock();
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), provideFakeDock(dock)] });
    return { dock, store: TestBed.inject(WorkspaceStore), events: TestBed.inject(DebriefEvents) };
  }

  it('exposes read-only signals and observables over the store', async () => {
    const { store, events } = setup();
    const workspace = firstValueFrom(events.workspaceChanged);
    await settle();
    expect((await workspace).total).toBe(11);
    expect(events.workspace()?.total).toBe(11);

    const query = firstValueFrom(events.querySelectionChanged);
    store.patchQuery({ text: 'boom' });
    await settle();
    expect((await query).text === 'boom' || events.query().text === 'boom').toBe(true);
    expect(events.query().text).toBe('boom');

    const selected = [] as (RecordView | null)[];
    events.recordSelected.subscribe((r) => selected.push(r));
    const row = { id: 3 } as RecordView;
    store.select(row);
    await settle();
    expect(events.selectedRecord()).toBe(row);
    expect(selected.at(-1)).toBe(row);

    let saved = 0;
    events.profileSaved.subscribe(() => saved++);
    await settle();
    expect(saved).toBe(0);
    store.profileSaved(store.summary()!);
    await settle();
    expect(saved).toBe(1);
  });

  it('lets plugins act only through the documented context methods', async () => {
    const { dock, store } = setup();
    await settle();
    const ctx = TestBed.inject(WorkspaceCommandContext);
    expect(ctx.summary?.total).toBe(11);
    await ctx.openPaths(['/extra/new.log']);
    expect(store.summary()?.sources.some((s) => s.path === '/extra/new.log')).toBe(true);
    ctx.setQuery({ text: 'x' });
    expect(store.query().text).toBe('x');
    ctx.addFilter({ field: 'MachineName', op: 'eq', value: 'WEB01' });
    expect(store.query().filters).toHaveLength(1);
    ctx.selectRecord({ id: 9 } as RecordView);
    expect(store.selected()?.id).toBe(9);
    ctx.showView('registry');
    expect(dock.calls).toEqual(['show:registry']);
    ctx.notifyError('bad');
    expect(store.error()).toBe('bad');
    expect(TestBed.inject(COMMAND_CONTEXT)).toBeInstanceOf(WorkspaceCommandContext);
  });

  it('suggestPerspective hands the id to Perspectives; an unknown perspective changes nothing', async () => {
    const { dock } = setup();
    await settle();
    const perspectives = TestBed.inject(Perspectives);
    const suggest = vi.spyOn(perspectives, 'suggest');
    const ctx = TestBed.inject(WorkspaceCommandContext);
    const before = perspectives.active().id;
    ctx.suggestPerspective('no-such-perspective');
    expect(suggest).toHaveBeenCalledWith('no-such-perspective');
    expect(perspectives.active().id).toBe(before);
    expect(dock.calls).toEqual([]);
    ctx.suggestPerspective('profiling');
    expect(suggest).toHaveBeenLastCalledWith('profiling');
  });

  it('exposes the display timezone as a signal', () => {
    const { store, events } = setup();
    store.setTimezone('local');
    expect(events.timezone()).toBe('local');
    store.setTimezone('utc');
    expect(events.timezone()).toBe('utc');
  });

  it('jumpTo asks the grid for the record and resolves when it is selected or reported missing', async () => {
    const { store } = setup();
    await settle();
    const ctx = TestBed.inject(WorkspaceCommandContext);
    let done = false;
    const found = ctx.jumpTo(4).then(() => (done = true));
    expect(store.anchor()?.id).toBe(4);
    await settle();
    expect(done).toBe(false);
    store.anchorResolved({ id: 4 } as RecordView);
    await found;
    expect(store.selected()?.id).toBe(4);

    const missing = ctx.jumpTo(99);
    expect(store.anchor()?.id).toBe(99);
    store.anchorResolved(null);
    await missing;
    expect(store.anchorMissed()).toBe(true);
  });

  it('jumpTo resolves after a timeout when no grid handles the anchor', async () => {
    const { store } = setup();
    await settle();
    vi.useFakeTimers();
    try {
      const ctx = TestBed.inject(WorkspaceCommandContext);
      let done = false;
      void ctx.jumpTo(5).then(() => (done = true));
      TestBed.tick();
      await vi.advanceTimersByTimeAsync(JUMP_TIMEOUT_MS + 1);
      expect(done).toBe(true);
      expect(store.anchor()?.id).toBe(5);
    } finally {
      vi.useRealTimers();
    }
  });

  it('StoreDebriefEvents can be created in an injection context', () => {
    setup();
    expect(runInInjectionContext(TestBed.inject(Injector), () => TestBed.inject(DebriefEvents))).toBeTruthy();
  });
});
