import { TestBed } from '@angular/core/testing';
import type { DockviewApi } from 'dockview';
import { provideDebriefBuiltins } from '../builtin/provide-builtins';
import { BUILTIN_VIEWS } from '../builtin/builtin-views';
import { HOME_VIEWS } from '../home/home.plugin';
import { provideTinyBackendBare, tinyBackend } from '../../testing/helpers';
import { provideWorkspace } from '../core/workspace/provide-workspace';
import { DebriefView } from '../extensions/tokens';
import { defaultViewLayout } from '../layout-plan/layout-plan';
import { DockContext } from './dock-context';
import { DockLayout } from './dock-layout';
import { viewLayout, viewRow } from './view-layout';

const AI: DebriefView = { id: 'ai-chat', title: 'AI', icon: 'puzzle', defaultPosition: 'right', layout: { area: 'side', min: { width: 320, height: 240 }, pref: { width: 372, height: 400 }, priority: 70 }, component: async () => class {} };

interface FakeGroup {
  id: string;
  width: number;
  height: number;
  panels: { id: string }[];
  element: HTMLElement;
  api: { location: { type: string }; setConstraints: ReturnType<typeof vi.fn>; setSize: ReturnType<typeof vi.fn> };
}

/** A fake dock: one group per panel id (ids joined by "+" share a group), with sizes. */
function fakeDock(width: number, height: number, spec: Record<string, { width: number; height: number }>) {
  const groups: FakeGroup[] = [];
  const panels = new Map<string, { id: string; group: FakeGroup; api: { moveTo: ReturnType<typeof vi.fn>; setActive: ReturnType<typeof vi.fn> } }>();
  for (const [ids, size] of Object.entries(spec)) {
    const group: FakeGroup = { id: `g${groups.length}`, ...size, panels: [], element: document.createElement('div'), api: { location: { type: 'grid' }, setConstraints: vi.fn(), setSize: vi.fn() } };
    groups.push(group);
    for (const id of ids.split('+')) {
      group.panels.push({ id });
      panels.set(id, { id, group, api: { moveTo: vi.fn(), setActive: vi.fn() } });
    }
  }
  const api = { width, height, groups, panels: [...panels.values()], getPanel: (id: string) => panels.get(id), removePanel: vi.fn() } as unknown as DockviewApi;
  return { api, groups, panels };
}

function setup() {
  TestBed.configureTestingModule({
    providers: [provideDebriefBuiltins([...BUILTIN_VIEWS, ...HOME_VIEWS, AI]), provideWorkspace(), ...provideTinyBackendBare(tinyBackend())],
  });
  return TestBed.inject(DockLayout);
}

describe('view layouts', () => {
  it('every built-in and core view declares a layout with min <= pref and a priority', () => {
    for (const v of [...BUILTIN_VIEWS, ...HOME_VIEWS, AI]) {
      expect(v.layout, v.id).toBeDefined();
      expect(v.layout!.min.width, v.id).toBeLessThanOrEqual(v.layout!.pref.width);
      expect(v.layout!.min.height, v.id).toBeLessThanOrEqual(v.layout!.pref.height);
      expect(v.layout!.priority, v.id).toBeGreaterThan(0);
    }
  });

  it('a view that declares nothing gets its area\'s default', () => {
    expect(viewLayout({ id: 'x', defaultPosition: 'center' })).toEqual(defaultViewLayout('document'));
    expect(viewLayout({ id: 'inspector-like', defaultPosition: 'center', area: 'side' }).min.width).toBe(320);
    expect(viewLayout(undefined)).toEqual(defaultViewLayout('document'));
  });

  it('rows: analysis shares one row, events and timeline have their own, other documents sit beside the timeline', () => {
    expect(viewRow({ id: 'markers', layout: defaultViewLayout('analysis') })).toBe('analysis');
    expect(viewRow({ id: 'events' })).toBe('events');
    expect(viewRow({ id: 'metrics' })).toBe('timeline');
  });
});

describe('DockLayout.fit', () => {
  it('constrains every group from the views it holds (events 560, assistant 320), also without a measured size', () => {
    const layout = setup();
    const { api, groups } = fakeDock(0, 0, { events: { width: 0, height: 0 }, 'ai-chat': { width: 0, height: 0 } });
    expect(layout.fit(api)).toBeNull();
    expect(groups[0]!.api.setConstraints).toHaveBeenCalledWith({ minimumWidth: 560, minimumHeight: 240 });
    expect(groups[1]!.api.setConstraints).toHaveBeenCalledWith({ minimumWidth: 320 });
    // The declared minimum is readable from outside (the scenario tests assert against it).
    expect(groups[0]!.element.dataset['minWidth']).toBe('560');
    expect(groups[1]!.element.dataset['minWidth']).toBe('320');
  });

  it('tabs Home into the events group when the window has no room for it, and never squeezes the assistant', () => {
    const layout = setup();
    const { api, panels, groups } = fakeDock(1100, 800, { 'timeline+events': { width: 400, height: 800 }, 'ai-chat': { width: 372, height: 800 }, home: { width: 328, height: 800 } });
    const plan = layout.fit(api, { sizes: true, fresh: ['home'] })!;
    expect(plan.tabbed).toEqual([{ id: 'home', into: 'events' }]);
    expect(panels.get('home')!.api.moveTo).toHaveBeenCalledWith({ group: groups[0], position: 'center' });
    expect(panels.get('home')!.api.setActive).toHaveBeenCalled();
    expect(plan.violations).toEqual([]);
  });

  it('a view that yields without being opened just now joins as a background tab: the tab in front stays in front', () => {
    const layout = setup();
    const { api, panels, groups } = fakeDock(1100, 800, { 'timeline+events': { width: 400, height: 800 }, 'ai-chat': { width: 372, height: 800 }, home: { width: 328, height: 800 } });
    const eventsActive = { api: { setActive: vi.fn() } };
    (groups[0] as unknown as { activePanel: unknown }).activePanel = eventsActive;
    layout.fit(api, { sizes: true });
    expect(panels.get('home')!.api.setActive).not.toHaveBeenCalled();
    expect(eventsActive.api.setActive).toHaveBeenCalled();
  });

  it('gives a new Home column its preferred width at 1920 and shrinks only what the plan says at 1440', () => {
    const layout = setup();
    const wide = fakeDock(1920, 1080, { 'timeline+events': { width: 1000, height: 1080 }, 'ai-chat': { width: 372, height: 1080 }, home: { width: 548, height: 1080 } });
    layout.fit(wide.api, { sizes: true, fresh: ['home'] });
    expect(wide.groups[2]!.api.setSize).toHaveBeenCalledWith({ width: 700 });
    const narrow = fakeDock(1440, 900, { 'timeline+events': { width: 520, height: 900 }, 'ai-chat': { width: 372, height: 900 }, home: { width: 548, height: 900 } });
    const plan = layout.fit(narrow.api, { sizes: true, fresh: ['home'] })!;
    expect(plan.tabbed).toEqual([]);
    expect(narrow.groups[2]!.api.setSize).toHaveBeenCalledWith({ width: 508 });
  });

  it('auto-hides a side tool to the right edge when there is no room left even after tabbing', () => {
    const layout = setup();
    const { api, groups } = fakeDock(800, 700, { 'timeline+events': { width: 428, height: 700 }, 'ai-chat': { width: 372, height: 700 } });
    const ctx = TestBed.inject(DockContext);
    ctx.api = api;
    const plan = layout.fit(api, { fresh: ['ai-chat'] })!;
    expect(plan.hidden).toEqual([{ id: 'ai-chat', edge: 'right' }]);
    expect(api.removePanel).toHaveBeenCalled();
    expect(ctx.autoHide().items.map((i) => i.id)).toEqual(['ai-chat']);
    expect(ctx.autoHide().open).toBe('ai-chat');
    expect(groups.length).toBe(2);
  });

  it('needsFit: false for a sound layout, true for a group under its minimum or a view that must yield', () => {
    const layout = setup();
    const ok = fakeDock(1440, 900, { 'timeline+events': { width: 1068, height: 900 }, 'ai-chat': { width: 372, height: 900 } });
    expect(layout.needsFit(ok.api, layout.plan(ok.api)!)).toBe(false);
    const small = fakeDock(1440, 900, { 'timeline+events': { width: 1180, height: 900 }, 'ai-chat': { width: 200, height: 900 } });
    expect(layout.needsFit(small.api, layout.plan(small.api)!)).toBe(true);
    const tight = fakeDock(800, 900, { 'timeline+events': { width: 428, height: 900 }, 'ai-chat': { width: 372, height: 900 } });
    expect(layout.needsFit(tight.api, layout.plan(tight.api)!)).toBe(true);
  });
});

describe('DockPersistence.hold (a layout fitted to a small window is not written over the saved one)', () => {
  it('skips saves while held and saves again after release', async () => {
    vi.useFakeTimers();
    try {
      TestBed.configureTestingModule({ providers: [provideDebriefBuiltins(), provideWorkspace(), ...provideTinyBackendBare(tinyBackend())] });
      const { DockPersistence } = await import('./dock-persistence');
      const { LAYOUT_STORE } = await import('./layout-store');
      const store = { load: () => null, save: vi.fn(), clear: vi.fn() };
      TestBed.overrideProvider(LAYOUT_STORE, { useValue: store });
      const persistence = TestBed.inject(DockPersistence);
      TestBed.inject(DockContext).api = { toJSON: () => ({ grid: {}, panels: {} }) } as unknown as DockviewApi;
      persistence.hold();
      persistence.schedule();
      vi.advanceTimersByTime(1000);
      expect(store.save).not.toHaveBeenCalled();
      persistence.release();
      persistence.schedule();
      vi.advanceTimersByTime(1000);
      expect(store.save).toHaveBeenCalledTimes(1);
    } finally {
      vi.useRealTimers();
    }
  });
});
