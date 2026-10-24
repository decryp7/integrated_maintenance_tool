import { TestBed } from '@angular/core/testing';
import { all, el, settle, typeInto } from '../../testing/helpers';
import { LensCfg } from '../core/api.types';
import { WorkspaceStore } from '../core/workspace.store';
import { UI_STATE_CONTRIBUTIONS, UiStateRegistry } from '../lifecycle/ui-state';
import { mockLenses } from '../core/mock/mock-lenses';
import { HighlightRules } from '../projects/bookmarks/highlight-rules.store';
import { ProjectUiState } from '../projects/project-ui-state';
import { provideToolsTesting } from '../tools/mock/test-support';
import { UndoStack } from '../undo/undo-stack';
import { FilterSetStore } from './filter-set.store';
import { LensBar } from './lens-bar';
import { LENS_STATE_VERSION, lensFromProfile, MAX_SAVED, parseFilterSets, parseSavedLenses, versioned } from './lens-model';
import { LensStore } from './lens.store';
import { at } from '../../testing/must';

async function setup() {
  TestBed.configureTestingModule({
    providers: [...provideToolsTesting(), { provide: UI_STATE_CONTRIBUTIONS, multi: true, useFactory: () => TestBed.inject(ProjectUiState).contributions }],
  });
  // The workspace loads its summary when it is created (a start-up read, retried: ADR 0058): create it, then let it load.
  TestBed.inject(WorkspaceStore);
  await settle();
  return {
    workspace: TestBed.inject(WorkspaceStore),
    lenses: TestBed.inject(LensStore),
    sets: TestBed.inject(FilterSetStore),
    highlights: TestBed.inject(HighlightRules),
    registry: TestBed.inject(UiStateRegistry),
    undo: TestBed.inject(UndoStack),
  };
}

const cfg = (over: Partial<LensCfg> = {}): LensCfg => ({ id: 'x', title: 'X', ...over });

describe('lens model', () => {
  it('fills defaults for profile lenses', () => {
    expect(lensFromProfile(cfg())).toEqual({ id: 'x', name: 'X', description: '', query: { levels: [], text: '', regex: false, ql: '', markers: [], filters: [] }, columns: [], highlights: [], chart: null });
    const full = lensFromProfile(cfg({ description: 'd', levels: ['Error'], text: 't', regex: true, ql: 'q', markers: ['m'], filter: [{ field: 'f', op: 'eq', value: 'v' }], columns: ['c'], highlight: [{ field: 'f', op: 'eq', value: 'v', color: '#112233' }], chart: 'ch' }));
    expect(full).toMatchObject({ description: 'd', query: { levels: ['Error'], regex: true, ql: 'q' }, columns: ['c'], chart: 'ch' });
  });

  it('parses saved lenses defensively', () => {
    expect(parseSavedLenses(undefined)).toEqual([]);
    expect(parseSavedLenses([{ id: 'a' }])).toEqual([]);
    expect(parseSavedLenses({ version: LENS_STATE_VERSION + 1, items: [{ id: 'a', name: 'A', query: {} }] })).toEqual([]);
    expect(parseSavedLenses({ version: LENS_STATE_VERSION, items: 'nope' })).toEqual([]);
    const items = [
      null,
      { id: 1, name: 'bad id', query: {} },
      { id: 'a', name: 'A', query: {} },
      {
        id: 'b',
        name: 'B',
        description: 'd',
        query: {
          levels: ['Error', 'Loud', 5],
          text: 't',
          regex: true,
          ql: 'q',
          markers: ['m', 1],
          filters: [{ field: 'f', op: 'eq', value: 'v' }, { field: 'g', op: 'exists' }, { field: 'h', op: 'bogus' }, { op: 'eq' }, 'x'],
        },
        columns: ['c', 2],
        highlights: [{ field: 'f', op: 'eq', value: 'v', color: '#aabbcc' }, { field: 'f', op: 'exists', color: '#aabbcc' }, { field: 'f', op: 'eq', color: 'red' }, { field: 'f', op: 'matches', color: '#aabbcc' }, 3],
        chart: 'ch',
      },
    ];
    const lenses = parseSavedLenses(versioned(items));
    expect(lenses.map((l) => l.id)).toEqual(['a', 'b']);
    expect(at(lenses, 0)).toMatchObject({ query: { levels: [], text: '', regex: false, ql: '', markers: [], filters: [] }, columns: [], highlights: [], chart: null, description: '' });
    expect(at(lenses, 1)).toMatchObject({
      description: 'd',
      query: { levels: ['Error'], text: 't', regex: true, ql: 'q', markers: ['m'], filters: [{ field: 'f', op: 'eq', value: 'v' }, { field: 'g', op: 'exists' }] },
      columns: ['c'],
      highlights: [{ field: 'f', op: 'eq', value: 'v', color: '#aabbcc' }, { field: 'f', op: 'exists', value: '', color: '#aabbcc' }],
      chart: 'ch',
    });
    expect(parseSavedLenses(versioned(Array.from({ length: MAX_SAVED + 5 }, (_, i) => ({ id: `l${i}`, name: 'n', query: {} }))))).toHaveLength(MAX_SAVED);
  });

  it('parses saved filter sets defensively', () => {
    expect(parseFilterSets('x')).toEqual([]);
    const sets = parseFilterSets(versioned([{ id: 'a', name: 'A', filters: [{ field: 'f', op: 'eq', value: 'v' }, { op: 'eq' }], enabled: false }, { id: 'b', name: 'B' }, { id: 3 }, 7]));
    expect(sets).toEqual([
      { id: 'a', name: 'A', filters: [{ field: 'f', op: 'eq', value: 'v' }], enabled: false },
      { id: 'b', name: 'B', filters: [], enabled: true },
    ]);
  });

  it('mock lenses mirror the summary keys', () => {
    expect(mockLenses({})).toEqual([]);
    expect(mockLenses({ lens: [{ id: 'a', title: 'A', levels: ['Error'] }, {}] })).toEqual([{ id: 'a', title: 'A', levels: ['Error'] }, { id: '', title: '' }]);
  });
});

describe('FilterSetStore', () => {
  it('adds, toggles, renames and removes sets; only enabled sets contribute filters', async () => {
    const { sets, undo } = await setup();
    const a = sets.add(' Noise ', [{ field: 'SourceContext', op: 'contains', value: 'Http' }]);
    sets.add('', [{ field: '@level', op: 'eq', value: 'Error' }]);
    expect(sets.sets().map((s) => s.name)).toEqual(['Noise', 'Set 2']);
    expect(sets.enabledFilters()).toHaveLength(2);
    sets.toggle(a);
    expect(sets.enabledFilters().map((f) => f.field)).toEqual(['@level']);
    sets.rename(a, ' Chatty ');
    sets.rename(a, '  ');
    expect(at(sets.sets(), 0).name).toBe('Chatty');
    sets.remove(a);
    expect(sets.sets()).toHaveLength(1);
    undo.undo();
    expect(sets.sets()).toHaveLength(2);
  });

  it('the engine query carries the enabled sets next to the user filters', async () => {
    const { sets, workspace } = await setup();
    workspace.addFilter({ field: 'a', op: 'eq', value: '1' });
    expect(workspace.effectiveQuery().filters).toEqual([{ field: 'a', op: 'eq', value: '1' }]);
    const id = sets.add('S', [{ field: 'b', op: 'eq', value: '2' }]);
    expect(workspace.effectiveQuery().filters.map((f) => f.field)).toEqual(['a', 'b']);
    expect(workspace.query().filters).toHaveLength(1);
    sets.toggle(id);
    expect(workspace.effectiveQuery().filters.map((f) => f.field)).toEqual(['a']);
  });
});

describe('LensStore', () => {
  it('lists the profile lenses and applies one', async () => {
    const { lenses, workspace } = await setup();
    expect(lenses.builtIn().map((l) => l.id)).toEqual(['errors', 'warnings']);
    workspace.patchQuery({ from: 5, to: 9 });
    workspace.addFilter({ field: 'a', op: 'eq', value: 'x' });
    lenses.apply('warnings');
    expect(workspace.query()).toMatchObject({ levels: ['Warning', 'Error', 'Fatal'], filters: [], from: 5, to: 9 });
    expect(workspace.extraColumns()).toEqual(['SourceContext']);
    expect(lenses.active()?.name).toBe('Warnings and worse');
    lenses.apply('errors');
    expect(workspace.query().levels).toEqual(['Error', 'Fatal']);
    expect(workspace.extraColumns()).toEqual(['SourceContext']);
    lenses.apply('nope');
    expect(lenses.activeId()).toBe('errors');
  });

  it('saves the current view, switches back to it and deletes it', async () => {
    const { lenses, workspace, highlights } = await setup();
    workspace.setTextInput('timeout');
    workspace.patchQuery({ levels: ['Error'], ql: 'x > 1', markers: ['restart'], from: 1, to: 2 });
    workspace.addFilter({ field: 'a', op: 'eq', value: 'x' });
    workspace.setColumns(['A', 'B', 'A']);
    highlights.add({ field: '@level', op: 'eq', value: 'Error', color: '#f2545b' });
    const id = lenses.saveCurrent(' Mine ');
    expect(lenses.saveCurrent('')).toBeTruthy();
    expect(lenses.saved().map((l) => l.name)).toEqual(['Mine', 'Lens 2']);
    expect(lenses.saved()[0]).toMatchObject({ query: { text: 'timeout', levels: ['Error'], ql: 'x > 1' }, columns: ['A', 'B'], chart: null });
    expect(JSON.stringify(lenses.saved()[0])).not.toContain('"from"');

    workspace.clearQuery();
    workspace.setColumns([]);
    highlights.replace([]);
    lenses.apply(id);
    expect(workspace.query()).toMatchObject({ levels: ['Error'], ql: 'x > 1', markers: ['restart'], filters: [{ field: 'a', op: 'eq', value: 'x' }], from: null });
    expect(workspace.textInput()).toBe('timeout');
    expect(workspace.extraColumns()).toEqual(['A', 'B']);
    expect(highlights.items()).toHaveLength(1);
    lenses.apply(id);
    expect(highlights.items()).toHaveLength(1);

    lenses.rename(id, ' Renamed ');
    lenses.rename(id, '');
    expect(lenses.active()?.name).toBe('Renamed');
    lenses.remove(id);
    expect(lenses.activeId()).toBeNull();
    lenses.remove('unknown');
    expect(lenses.saved()).toHaveLength(1);
  });

  it('a saved lens never shadows a profile lens and a lens can bring a chart forward', async () => {
    const { lenses } = await setup();
    lenses.replace([{ ...lensFromProfile(cfg({ id: 'errors', title: 'Dup' })) }, { ...lensFromProfile(cfg({ id: 'c', title: 'C', chart: 'chart-9' })) }]);
    expect(lenses.all().map((l) => l.name)).toEqual(['Errors only', 'Warnings and worse', 'C']);
    lenses.apply('c');
    expect(lenses.focusChart()).toBe('chart-9');
    lenses.apply('errors');
    expect(lenses.focusChart()).toBeNull();
  });

  it('moves the filters into a filter set', async () => {
    const { lenses, workspace, sets } = await setup();
    expect(lenses.saveFiltersAsSet('x')).toBeNull();
    workspace.addFilter({ field: 'a', op: 'eq', value: '1' });
    const id = lenses.saveFiltersAsSet('Mine');
    expect(id).toBe(at(sets.sets(), 0).id);
    expect(workspace.query().filters).toEqual([]);
    expect(workspace.effectiveQuery().filters).toHaveLength(1);
  });
});

describe('lens and filter-set persistence', () => {
  it('round-trips through the project state and resets without it', async () => {
    const { lenses, sets, registry, workspace } = await setup();
    lenses.apply('errors');
    lenses.saveCurrent('Mine');
    sets.add('S', [{ field: 'a', op: 'eq', value: '1' }]);
    const ui = JSON.parse(JSON.stringify(registry.serialize()));
    expect(ui.lenses.version).toBe(LENS_STATE_VERSION);
    expect(ui.filterSets.items).toHaveLength(1);

    registry.restore({});
    expect(lenses.saved()).toEqual([]);
    expect(sets.sets()).toEqual([]);
    registry.restore(ui);
    expect(lenses.saved().map((l) => l.name)).toEqual(['Mine']);
    expect(sets.sets().map((s) => s.name)).toEqual(['S']);
    expect(workspace.effectiveQuery().filters).toHaveLength(1);

    registry.restore({ lenses: 'garbage', filterSets: { version: 99, items: [] } });
    expect(lenses.saved()).toEqual([]);
    expect(sets.sets()).toEqual([]);
  });
});

describe('LensBar', () => {
  it('switches lenses, saves lenses and toggles filter sets by chip and Alt+number', async () => {
    const { lenses, sets, workspace } = await setup();
    const fixture = TestBed.createComponent(LensBar);
    await settle(fixture);
    const button = () => el<HTMLButtonElement>(fixture, '[data-testid="lens-button"]');
    const label = () => el(fixture, '[data-testid="lens-name"]').textContent?.trim();
    const items = () => Array.from(document.body.querySelectorAll<HTMLButtonElement>('[data-testid="lens-menu"] button'));
    const item = (t: string) => items().find((b) => b.textContent?.includes(t)) as HTMLButtonElement;
    expect(label()).toBe('No lens');
    expect(fixture.nativeElement.querySelector('select')).toBeNull();
    button().click();
    await settle(fixture);
    expect(items().map((b) => b.textContent?.trim())).toEqual(['Errors only', 'Warnings and worse', 'Save current as lens…', 'Filters → set']);
    item('Errors only').click();
    await settle(fixture);
    expect(workspace.query().levels).toEqual(['Error', 'Fatal']);
    expect(lenses.activeId()).toBe('errors');
    expect(label()).toBe('Errors only');

    // Saving opens a small popover with the name; the saved lens becomes the current one and can be deleted from the menu.
    button().click();
    await settle(fixture);
    item('Save current as lens').click();
    await settle(fixture);
    const name = el<HTMLInputElement>(fixture, '[data-testid="lens-save-form"] input');
    typeInto(name, 'Mine');
    await settle(fixture);
    el<HTMLFormElement>(fixture, '[data-testid="lens-save-form"]').dispatchEvent(new Event('submit', { cancelable: true }));
    await settle(fixture);
    expect(lenses.saved().map((l) => l.name)).toEqual(['Mine']);
    expect(fixture.nativeElement.querySelector('[data-testid="lens-save-form"]')).toBeNull();
    expect(label()).toBe('Mine');
    button().click();
    await settle(fixture);
    item('Delete lens Mine').click();
    await settle(fixture);
    expect(lenses.saved()).toEqual([]);
    expect(label()).toBe('No lens');

    const openSetItem = async () => {
      button().click();
      await settle(fixture);
      return item('Filters → set');
    };
    expect((await openSetItem()).getAttribute('aria-disabled')).toBe('true');
    workspace.addFilter({ field: 'a', op: 'eq', value: '1' });
    await settle(fixture);
    expect(item('Filters → set').getAttribute('aria-disabled')).not.toBe('true');
    item('Filters → set').click();
    await settle(fixture);
    const chip = () => el<HTMLButtonElement>(fixture, '[data-testid="filter-set"] .toggle');
    expect(chip().getAttribute('aria-pressed')).toBe('true');
    chip().click();
    await settle(fixture);
    expect(chip().getAttribute('aria-pressed')).toBe('false');

    const key = (init: KeyboardEventInit) => {
      const e = new KeyboardEvent('keydown', { cancelable: true, ...init });
      document.dispatchEvent(e);
      return e.defaultPrevented;
    };
    expect(key({ altKey: true, code: 'Digit1' })).toBe(true);
    expect(at(sets.sets(), 0).enabled).toBe(true);
    expect(key({ altKey: true, code: 'Digit2' })).toBe(false);
    expect(key({ altKey: true, code: 'Digit0' })).toBe(false);
    expect(key({ altKey: true, code: 'KeyA' })).toBe(false);
    expect(key({ code: 'Digit1' })).toBe(false);
    expect(key({ altKey: true, ctrlKey: true, code: 'Digit1' })).toBe(false);
    expect(at(sets.sets(), 0).enabled).toBe(true);

    el<HTMLButtonElement>(fixture, '[data-testid="filter-set"] .x').click();
    await settle(fixture);
    expect(sets.sets()).toEqual([]);
  });

  it('titles the chips with their key only for the first nine', async () => {
    const { sets } = await setup();
    for (let i = 0; i < 10; i++) sets.add(`S${i}`, []);
    const fixture = TestBed.createComponent(LensBar);
    await settle(fixture);
    const titles = all<HTMLButtonElement>(fixture, '.toggle').map((b) => b.title);
    expect(titles[0]).toContain('Alt+1');
    expect(titles[8]).toContain('Alt+9');
    expect(titles[9]).not.toContain('Alt');
  });

  it('groups profile and saved lenses in the menu', async () => {
    const { lenses, workspace } = await setup();
    workspace.setTextInput('x');
    lenses.saveCurrent('Mine');
    const fixture = TestBed.createComponent(LensBar);
    await settle(fixture);
    el<HTMLButtonElement>(fixture, '[data-testid="lens-button"]').click();
    await settle(fixture);
    expect(Array.from(document.body.querySelectorAll('[data-testid="lens-menu"] .group')).map((g) => g.textContent?.trim())).toEqual(['Profile', 'Saved']);
  });
});

