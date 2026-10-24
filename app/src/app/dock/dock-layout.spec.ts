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

  it('a view opened in the background (auto-open) that has no room never takes the front from the view it joins', () => {
    const layout = setup();
    const { api, panels, groups } = fakeDock(1100, 800, { 'timeline+events': { width: 400, height: 800 }, 'ai-chat': { width: 372, height: 800 }, home: { width: 328, height: 800 } });
    const eventsActive = { api: { setActive: vi.fn() } };
    (groups[0] as unknown as { activePanel: unknown }).activePanel = eventsActive;
    layout.fit(api, { sizes: true, fresh: ['home'], background: true });
    expect(panels.get('home')!.api.moveTo).toHaveBeenCalled();
    expect(panels.get('home')!.api.setActive).not.toHaveBeenCalled();
    expect(eventsActive.api.setActive).toHaveBeenCalled();
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

describe('displaced views and grow-back', () => {
  const grid = () => fakeDock(1100, 800, { 'timeline+events': { width: 400, height: 800 }, 'ai-chat': { width: 372, height: 800 }, home: { width: 328, height: 800 } });

  it('records where a view was and what it joined when the plan tabs it away', () => {
    const layout = setup();
    const { api } = grid();
    const slot = { referencePanel: null, direction: 'within' as const, edge: 'right' as const, size: 700 };
    layout.fit(api, { sizes: true, capture: (id) => (id === 'home' ? slot : null) });
    expect(TestBed.inject(DockContext).displaced.get('home')).toEqual({ id: 'home', slot, into: 'events' });
  });

  it('records a view opened just now as a guest of its host too, so the next plan does not take the host for a full-height column', () => {
    const layout = setup();
    const { api, groups } = grid();
    layout.fit(api, { sizes: true, fresh: ['home'] });
    expect(TestBed.inject(DockContext).displaced.get('home')?.into).toBe('events');
    // The panel now lives in the events group (as dockview would have moved it): planning again leaves the timeline alone.
    groups[0]!.panels.push({ id: 'home' });
    expect(layout.plan(api)!.tabbed.map((t) => t.id)).toEqual(['home']);
  });

  it('a second timeline is stacked below the first in a row of its own, never side by side with it', () => {
    const layout = setup();
    const { api } = fakeDock(1440, 1000, { timeline: { width: 1440, height: 300 }, 'timeline#2': { width: 1440, height: 300 }, events: { width: 1440, height: 400 } });
    const plan = layout.plan(api)!;
    expect(plan.violations).toEqual([]);
    expect(plan.tabbed).toEqual([]);
    expect(plan.columns[0]!.rows.map((r) => r.key).sort()).toEqual(['events', 'timeline', 'timeline#2']);
  });

  it('records an auto-hidden side tool with no host', () => {
    const layout = setup();
    const { api } = fakeDock(800, 700, { 'timeline+events': { width: 428, height: 700 }, 'ai-chat': { width: 372, height: 700 } });
    TestBed.inject(DockContext).api = api;
    layout.fit(api);
    expect(TestBed.inject(DockContext).displaced.get('ai-chat')?.into).toBeNull();
  });

  it('a displaced view is a grow-back candidate only once there is room plus the margin, so it cannot flap', () => {
    const layout = setup();
    const ctx = TestBed.inject(DockContext);
    // Home tabbed into events; the dock is as wide as the threshold (1300) and then margin wider (1364).
    for (const [width, expected] of [[1300, []], [1363, []], [1364, ['home']]] as const) {
      const { api } = fakeDock(width, 900, { 'timeline+events+home': { width: 900, height: 900 }, 'ai-chat': { width: 372, height: 900 } });
      ctx.displaced.clear();
      ctx.displaced.set('home', { id: 'home', slot: { referencePanel: null, direction: 'within' }, into: 'events' });
      expect(layout.growBackIds(api), `${width}`).toEqual([...expected]);
    }
  });

  it('restoreOrder puts a view after the view its slot is relative to, and survives a cycle', () => {
    const layout = setup();
    const d = TestBed.inject(DockContext).displaced;
    const at = (id: string, ref: string | null) => d.set(id, { id, slot: { referencePanel: ref, direction: 'within' }, into: null });
    at('home', 'registry');
    at('registry', 'ai-chat');
    at('ai-chat', null);
    expect(layout.restoreOrder(['home', 'registry', 'ai-chat'])).toEqual(['ai-chat', 'registry', 'home']);
    at('a', 'b');
    at('b', 'a');
    expect(layout.restoreOrder(['a', 'b'])).toEqual(['a', 'b']);
  });

  it('a view tabbed in by the plan does not turn its host into a full-height column', () => {
    const layout = setup();
    const { api } = fakeDock(1500, 900, { 'timeline+events+home': { width: 1000, height: 900 }, 'ai-chat': { width: 372, height: 900 } });
    TestBed.inject(DockContext).displaced.set('home', { id: 'home', slot: { referencePanel: null, direction: 'within' }, into: 'events' });
    const open = layout.openViews(api);
    expect(open.find((v) => v.id === 'events')?.fullHeight).toBeUndefined();
    expect(open.find((v) => v.id === 'home')?.fullHeight).toBe(true);
  });

  it('moveBack: a column at the window edge is re-added by the caller, a neighbour slot is a move, a moved or missing view is stale', () => {
    const layout = setup();
    const { api, panels } = fakeDock(1500, 900, { 'timeline+events+home': { width: 1100, height: 900 }, 'ai-chat': { width: 372, height: 900 } });
    expect(layout.moveBack(api, { id: 'home', slot: { referencePanel: null, direction: 'within', edge: 'right' }, into: 'events' })).toBe('readd');
    expect(layout.moveBack(api, { id: 'home', slot: { referencePanel: 'ai-chat', direction: 'left' }, into: 'events' })).toBe('moved');
    expect(panels.get('home')!.api.moveTo).toHaveBeenCalledWith({ group: panels.get('ai-chat')!.group, position: 'left' });
    expect(layout.moveBack(api, { id: 'home', slot: { referencePanel: null, direction: 'within' }, into: 'ai-chat' })).toBe('stale');
    expect(layout.moveBack(api, { id: 'gone', slot: { referencePanel: null, direction: 'within' }, into: 'events' })).toBe('stale');
  });

  it('remembers dragged sizes as the user\'s wish and uses them (never below the minimum) as hints', () => {
    const layout = setup();
    const ctx = TestBed.inject(DockContext);
    const { api } = fakeDock(1920, 1080, { 'timeline+events': { width: 1200, height: 1080 }, 'ai-chat': { width: 500, height: 1080 } });
    layout.rememberSizes(api);
    expect(ctx.userSizes['ai-chat']).toEqual({ width: 500, height: 1080 });
    expect(layout.hints().sizes?.['ai-chat']?.width).toBe(500);
  });

  it('the Explorer verdict: hides when the plan has no room, shows again only with the margin', () => {
    const layout = setup();
    const narrow = fakeDock(400, 900, { 'timeline+events': { width: 400, height: 900 } });
    expect(layout.explorerVerdict(narrow.api, true, 300)).toBe('hide');
    const roomy = fakeDock(1200, 900, { 'timeline+events': { width: 1200, height: 900 } });
    expect(layout.explorerVerdict(roomy.api, true, 300)).toBe('keep');
    // Closed: the dock has the Explorer's room too; it needs 300 + 560 + the margin.
    expect(layout.explorerVerdict(fakeDock(923, 900, { 'timeline+events': { width: 923, height: 900 } }).api, false, 300)).toBe('keep');
    expect(layout.explorerVerdict(fakeDock(925, 900, { 'timeline+events': { width: 925, height: 900 } }).api, false, 300)).toBe('show');
  });
});
