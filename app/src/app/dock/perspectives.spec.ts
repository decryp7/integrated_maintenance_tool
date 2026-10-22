import { TestBed } from '@angular/core/testing';
import { at, must } from '../../testing/must';
import { FakeDock, provideTinyBackendBare } from '../../testing/helpers';
import { provideWorkspace } from '../core/workspace/provide-workspace';
import { WorkspaceStore } from '../core/workspace.store';
import { DEBRIEF_VIEWS, DebriefView } from '../extensions/tokens';
import { DockService } from './dock.service';
import { EMPTY_EXTRAS, SavedLayout, toEnvelope } from './layout-store';
import { NAMED_LAYOUT_STORE, NamedLayout, NamedLayoutStore } from './named-layouts';
import { perspectiveActivityItems, perspectiveMenu } from './perspective-activity';
import {
  LocalStoragePerspectiveStore,
  parsePerspectiveState,
  PERSPECTIVE_KEYS,
  PERSPECTIVE_STORE,
  PerspectiveState,
  PerspectiveStore,
  Perspectives,
  PERSPECTIVES_KEY,
  serializePerspectiveState,
} from './perspectives';

const grid = (id: string) => ({ grid: { root: { type: 'leaf', data: { views: [], id } }, height: 1, width: 1, orientation: 'HORIZONTAL' }, panels: {} }) as never;
const layout = (id: string): SavedLayout => ({ layout: grid(id), extras: { ...EMPTY_EXTRAS } });
const view = (id: string): DebriefView => ({ id, title: id, icon: 'panel', component: async () => null as never });

class DockStub extends FakeDock {
  current: SavedLayout | null = layout('current');
  applied: SavedLayout[] = [];
  built: string[] = [];
  applyOk = true;
  currentLayout = () => this.current;
  applyLayout = (l: SavedLayout) => {
    this.applied.push(l);
    return this.applyOk;
  };
  buildPerspective = (id: string) => void this.built.push(id);
}

class MemoryPerspectiveStore implements PerspectiveStore {
  saves: PerspectiveState[] = [];
  constructor(public state: PerspectiveState = { active: 'investigate', layouts: {} }) {}
  load = () => this.state;
  save = (s: PerspectiveState) => void (this.saves.push(s), (this.state = s));
}

class MemoryNamed implements NamedLayoutStore {
  constructor(public list: NamedLayout[] = []) {}
  load = () => this.list;
  save = (l: readonly NamedLayout[]) => void (this.list = [...l]);
}

function setup(opts: { views?: string[]; state?: PerspectiveState; named?: NamedLayout[] } = {}) {
  const dock = new DockStub();
  const store = new MemoryPerspectiveStore(opts.state);
  const ids = opts.views ?? ['profiling-captures', 'tail-status'];
  TestBed.configureTestingModule({
    providers: [
      provideWorkspace(),
      ...provideTinyBackendBare(),
      { provide: DockService, useValue: dock },
      { provide: DEBRIEF_VIEWS, multi: true, useValue: ids.map(view) },
      { provide: PERSPECTIVE_STORE, useValue: store },
      { provide: NAMED_LAYOUT_STORE, useValue: new MemoryNamed(opts.named) },
    ],
  });
  return { dock, store, p: TestBed.inject(Perspectives), status: TestBed.inject(WorkspaceStore) };
}

const names = (p: Perspectives) => p.list().map((x) => x.id);
const mine = (name = 'Mine'): NamedLayout => ({ name, savedAt: 1, saved: layout(name) });

describe('Perspectives list', () => {
  it('offers the built-in perspectives whose required view exists', () => {
    expect(names(setup().p)).toEqual(['investigate', 'profiling', 'live', 'diagnostics']);
  });

  it('hides perspectives whose required view is absent', () => {
    expect(names(setup({ views: ['tail-status'] }).p)).toEqual(['investigate', 'live', 'diagnostics']);
    TestBed.resetTestingModule();
    expect(names(setup({ views: ['profiling-captures'] }).p)).toEqual(['investigate', 'profiling', 'diagnostics']);
    TestBed.resetTestingModule();
    expect(names(setup({ views: [] }).p)).toEqual(['investigate', 'diagnostics']);
  });

  it('appends saved layouts as perspectives after the built-in ones', () => {
    const { p } = setup({ named: [mine()] });
    expect(must(p.list().at(-1))).toMatchObject({ id: 'saved:Mine', name: 'Mine', builtIn: false });
    expect(p.list().filter((x) => x.builtIn)).toHaveLength(4);
    expect(p.at(5)?.id).toBe('saved:Mine');
    expect(p.at(6)).toBeUndefined();
    expect(p.at(0)).toBeUndefined();
  });

  it('every built-in has a name, icon and description', () => {
    for (const info of setup().p.list()) {
      expect(info.name).toBeTruthy();
      expect(info.icon).toBeTruthy();
      expect(info.description).toBeTruthy();
    }
  });
});

describe('Perspectives active', () => {
  it('is the remembered one, numbered from 1', () => {
    const { p } = setup({ state: { active: 'live', layouts: {} } });
    expect(p.active().id).toBe('live');
    expect(p.activeNumber()).toBe(3);
  });

  it('falls back to the first perspective when the remembered one is gone', () => {
    const { p } = setup({ state: { active: 'saved:Gone', layouts: {} } });
    expect(p.active().id).toBe('investigate');
    expect(p.activeNumber()).toBe(1);
  });

  it('falls back when the remembered built-in is hidden', () => {
    const { p } = setup({ views: [], state: { active: 'profiling', layouts: {} } });
    expect(p.active().id).toBe('investigate');
  });
});

describe('Perspectives.switchTo', () => {
  it('builds a built-in perspective, persists the choice and announces it', () => {
    const { p, dock, store, status } = setup();
    expect(p.switchTo('live')).toBe(true);
    expect(dock.built).toEqual(['live']);
    expect(p.active().id).toBe('live');
    expect(must(store.saves.at(-1)).active).toBe('live');
    expect(status.notice()).toBe('Perspective: Live');
  });

  it('is a no-op for the active perspective and refuses an unknown id', () => {
    const { p, dock, status } = setup();
    expect(p.switchTo('investigate')).toBe(true);
    expect(dock.built).toEqual([]);
    expect(status.notice()).toBeNull();
    expect(p.switchTo('nope')).toBe(false);
    expect(p.switchTo('saved:Nope')).toBe(false);
    expect(p.active().id).toBe('investigate');
  });

  it('is quiet on request', () => {
    const { p, status } = setup();
    p.switchTo('live', { quiet: true });
    expect(status.notice()).toBeNull();
  });

  it('remembers the arrangement of the perspective being left and restores it on return', () => {
    const { p, dock, store } = setup();
    dock.current = layout('investigate-as-left');
    p.switchTo('live');
    expect(store.state.layouts['investigate']).toEqual(layout('investigate-as-left'));
    dock.current = layout('live-as-left');
    p.switchTo('investigate');
    expect(dock.applied).toEqual([layout('investigate-as-left')]);
    expect(dock.built).toEqual(['live']);
    p.switchTo('live');
    expect(dock.applied.at(-1)).toEqual(layout('live-as-left'));
    expect(dock.built).toEqual(['live']);
  });

  it('rebuilds when the remembered arrangement cannot be applied', () => {
    const { p, dock } = setup({ state: { active: 'live', layouts: { investigate: layout('x') } } });
    dock.applyOk = false;
    p.switchTo('investigate');
    expect(dock.built).toEqual(['investigate']);
  });

  it('remembers nothing while the dock has no layout', () => {
    const { p, dock, store } = setup();
    dock.current = null;
    p.switchTo('live');
    expect(store.state.layouts).toEqual({});
  });

  it('applies a saved perspective through its window layout', () => {
    const { p, dock } = setup({ named: [mine()] });
    p.switchTo('saved:Mine');
    expect(dock.applied).toEqual([layout('Mine')]);
    expect(p.active().name).toBe('Mine');
  });

  it('builds Investigate when a saved perspective cannot be applied', () => {
    const { p, dock, status } = setup({ named: [mine()] });
    dock.applyOk = false;
    p.switchTo('saved:Mine');
    expect(dock.built).toEqual(['investigate']);
    expect(status.error()).toContain('could not be applied');
  });

  it('switchToAt switches by number and reports a missing one', () => {
    const { p, dock } = setup();
    expect(p.switchToAt(2)).toBe(true);
    expect(dock.built).toEqual(['profiling']);
    expect(p.switchToAt(PERSPECTIVE_KEYS)).toBe(false);
    expect(p.switchToAt(0)).toBe(false);
  });
});

describe('Perspectives.reset', () => {
  it('forgets the remembered arrangement and rebuilds the active perspective', () => {
    const { p, dock, store, status } = setup({ state: { active: 'live', layouts: { live: layout('l'), investigate: layout('i') } } });
    p.reset();
    expect(dock.built).toEqual(['live']);
    expect(Object.keys(store.state.layouts)).toEqual(['investigate']);
    expect(status.notice()).toBe('Perspective Live reset');
  });
});

describe('Perspectives.suggest', () => {
  it('switches once per window, quietly, with a notice that offers Undo', () => {
    const { p, dock, status } = setup();
    p.suggest('profiling');
    expect(p.active().id).toBe('profiling');
    expect(dock.built).toEqual(['profiling']);
    expect(status.notice()).toBe('Switched to the Profiling perspective');
    expect(status.noticeAction()?.label).toBe('Undo');
    status.runNoticeAction();
    expect(p.active().id).toBe('investigate');
    expect(status.notice()).toBeNull();
    // Not again: the user went back on purpose.
    p.suggest('profiling');
    expect(p.active().id).toBe('investigate');
  });

  it('does nothing from the perspective itself or for one that does not exist', () => {
    const { p, dock } = setup({ state: { active: 'profiling', layouts: {} } });
    p.suggest('profiling');
    p.suggest('saved:none');
    expect(dock.built).toEqual([]);
    // Being on it did not use up the suggestion.
    p.switchTo('live');
    p.suggest('profiling');
    expect(p.active().id).toBe('profiling');
  });

  it('does nothing when the perspective is hidden', () => {
    const { p, dock } = setup({ views: [] });
    p.suggest('profiling');
    expect(dock.built).toEqual([]);
  });
});

describe('Perspectives saved ones', () => {
  it('saveCurrent saves the arrangement as a perspective and makes it active', () => {
    const { p, store } = setup({ state: { active: 'live', layouts: { 'saved:Mine': layout('stale'), live: layout('l') } } });
    expect(p.saveCurrent('  Mine  ')).toBe('Mine');
    expect(p.active().id).toBe('saved:Mine');
    expect(p.activeNumber()).toBe(5);
    // The stale remembered arrangement is gone: the saved layout is the arrangement.
    expect(Object.keys(store.state.layouts)).toEqual(['live']);
    expect(store.state.active).toBe('saved:Mine');
  });

  it('saveCurrent refuses an empty name and a dock without layout', () => {
    const { p, dock } = setup();
    expect(p.saveCurrent('   ')).toBeNull();
    dock.current = null;
    expect(p.saveCurrent('x')).toBeNull();
    expect(p.active().id).toBe('investigate');
  });

  it('rename keeps the remembered arrangement and the active choice', () => {
    const { p, store } = setup({ named: [mine('Old')], state: { active: 'saved:Old', layouts: { 'saved:Old': layout('mem') } } });
    expect(p.rename('Old', 'New')).toBe(true);
    expect(p.active().id).toBe('saved:New');
    expect(store.state.active).toBe('saved:New');
    expect(Object.keys(store.state.layouts)).toEqual(['saved:New']);
  });

  it('rename of an inactive perspective leaves the active one alone', () => {
    const { p, store } = setup({ named: [mine('Old')], state: { active: 'live', layouts: {} } });
    expect(p.rename('Old', 'New')).toBe(true);
    expect(p.active().id).toBe('live');
    expect(store.state.active).toBe('live');
  });

  it('rename is refused for unknown or empty names', () => {
    const { p } = setup({ named: [mine('Old')] });
    expect(p.rename('Missing', 'New')).toBe(false);
    expect(p.rename('Old', '  ')).toBe(false);
    expect(p.rename('Old', 'Old')).toBe(false);
  });

  it('remove drops its remembered arrangement and names Investigate active when it was the active one', () => {
    const { p, store } = setup({ named: [mine()], state: { active: 'saved:Mine', layouts: { 'saved:Mine': layout('x'), live: layout('l') } } });
    p.remove('Mine');
    expect(p.active().id).toBe('investigate');
    expect(store.state.active).toBe('investigate');
    expect(Object.keys(store.state.layouts)).toEqual(['live']);
    expect(p.list().some((x) => !x.builtIn)).toBe(false);
  });

  it('remove of another perspective keeps the active one', () => {
    const { p, store } = setup({ named: [mine()], state: { active: 'live', layouts: {} } });
    p.remove('Mine');
    expect(store.state.active).toBe('live');
  });

  it('move reorders saved perspectives', () => {
    const { p } = setup({ named: [mine('A'), mine('B')] });
    p.move('B', -1);
    expect(p.list().slice(4).map((x) => x.name)).toEqual(['B', 'A']);
    p.move('B', 1);
    expect(p.list().slice(4).map((x) => x.name)).toEqual(['A', 'B']);
  });
});

describe('parsePerspectiveState / serializePerspectiveState', () => {
  it('round-trips', () => {
    const state: PerspectiveState = { active: 'live', layouts: { live: layout('l') } };
    expect(parsePerspectiveState(JSON.parse(serializePerspectiveState(state)))).toEqual(state);
  });

  it.each([null, undefined, 'x', 7, [], {}, { version: 2, active: 'live' }, { version: '1' }])('falls back for %j', (raw) => {
    expect(parsePerspectiveState(raw)).toEqual({ active: 'investigate', layouts: {} });
  });

  it('defaults a bad active and drops malformed layouts', () => {
    const parsed = parsePerspectiveState({ version: 1, active: 5, layouts: { good: toEnvelope(layout('g')), bad: 'x', worse: { version: 99, layout: {} }, nul: null } });
    expect(parsed.active).toBe('investigate');
    expect(Object.keys(parsed.layouts)).toEqual(['good']);
    expect(parsePerspectiveState({ version: 1, active: '' }).active).toBe('investigate');
  });

  it('ignores layouts that are not an object', () => {
    expect(parsePerspectiveState({ version: 1, active: 'live', layouts: [toEnvelope(layout('a'))] })).toEqual({ active: 'live', layouts: {} });
    expect(parsePerspectiveState({ version: 1, active: 'live', layouts: 'x' })).toEqual({ active: 'live', layouts: {} });
  });
});

describe('LocalStoragePerspectiveStore', () => {
  beforeEach(() => localStorage.clear());

  it('saves and loads', () => {
    const s = new LocalStoragePerspectiveStore();
    expect(s.load()).toEqual({ active: 'investigate', layouts: {} });
    s.save({ active: 'live', layouts: { live: layout('l') } });
    expect(JSON.parse(must(localStorage.getItem(PERSPECTIVES_KEY))).version).toBe(1);
    expect(new LocalStoragePerspectiveStore().load().active).toBe('live');
  });

  it('survives corrupt data and unavailable storage', () => {
    localStorage.setItem(PERSPECTIVES_KEY, '{nope');
    expect(new LocalStoragePerspectiveStore().load().active).toBe('investigate');
    const spies = [
      vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
        throw new Error('blocked');
      }),
      vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
        throw new Error('quota');
      }),
    ];
    const s = new LocalStoragePerspectiveStore();
    expect(s.load()).toEqual({ active: 'investigate', layouts: {} });
    expect(() => s.save({ active: 'live', layouts: {} })).not.toThrow();
    spies.forEach((x) => x.mockRestore());
  });

  it('is the default PERSPECTIVE_STORE', () => {
    TestBed.configureTestingModule({});
    expect(TestBed.inject(PERSPECTIVE_STORE)).toBeInstanceOf(LocalStoragePerspectiveStore);
  });
});

describe('perspective activity', () => {
  it('lists the perspectives with the active one checked, Ctrl+n keys, then Save and Manage', () => {
    const { p } = setup({ named: [mine()], state: { active: 'live', layouts: {} } });
    const menu = perspectiveMenu(p);
    expect(menu.map((m) => m.id)).toEqual(['perspective:investigate', 'perspective:profiling', 'perspective:live', 'perspective:diagnostics', 'perspective:saved:Mine', 'perspective:save', 'perspective:manage']);
    expect(menu.filter((m) => m.checked).map((m) => m.id)).toEqual(['perspective:live']);
    expect(at(menu, 0).keybinding).toBe('Mod+1');
    expect(at(menu, 4).keybinding).toBe('Mod+5');
    expect(at(menu, 2).run).toEqual({ command: 'window.perspective.3' });
    expect(at(menu, 5).run).toEqual({ command: 'window.layout.save' });
    expect(at(menu, 6).run).toEqual({ command: 'window.layout.manage' });
  });

  it('shows at most nine perspectives', () => {
    const { p } = setup({ named: Array.from({ length: 12 }, (_, i) => mine(`L${i}`)) });
    const menu = perspectiveMenu(p);
    expect(menu.filter((m) => m.keybinding)).toHaveLength(PERSPECTIVE_KEYS);
    expect(menu).toHaveLength(PERSPECTIVE_KEYS + 2);
  });

  it('is one activity-bar item whose menu is built lazily', () => {
    const { p } = setup();
    const item = at(perspectiveActivityItems(p), 0);
    expect(item.id).toBe('perspectives');
    const run = item.run;
    expect('menu' in run && run.menu().length).toBeGreaterThan(2);
  });
});
