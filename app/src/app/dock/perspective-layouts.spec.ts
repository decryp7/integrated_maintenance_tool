import type { DockviewApi } from 'dockview';
import { DebriefView } from '../extensions/tokens';
import { PANEL } from './dock-context';
import { DockExtras, EMPTY_EXTRAS } from './layout-store';
import { BUILT_IN_PERSPECTIVES, buildPerspective, PERSPECTIVE_REQUIRES, PERSPECTIVE_VIEWS } from './perspective-layouts';

interface Added {
  id: string;
  title: string;
  inactive?: boolean;
  position?: { referenceGroup?: string; direction: string };
  group: string;
}

function harness(registered: readonly string[], opts: { events?: boolean } = {}) {
  const added: Added[] = [];
  const sizes: Record<string, { width?: number; height?: number }> = {};
  let groups = 0;
  const api = {
    addPanel: (o: { id: string; title: string; inactive?: boolean; position?: { referenceGroup?: string; direction: string } }) => {
      const reference = o.position?.referenceGroup;
      const group = o.position?.direction === 'within' && reference ? reference : `g${++groups}`;
      added.push({ id: o.id, title: o.title, inactive: o.inactive, position: o.position, group });
      return { group: { id: group, api: { setSize: (s: { width?: number; height?: number }) => void (sizes[group] = s) } } };
    },
    getPanel: (id: string) => (opts.events !== false && id === PANEL.events ? { group: { id: 'events-group' } } : undefined),
  } as unknown as DockviewApi;
  let defaults = 0;
  const extras: DockExtras = { autoHidden: [], pinnedTabs: ['marker'], poppedOut: [] };
  const ctx = {
    api,
    buildDefault: () => (defaults++, extras),
    find: (id: string): DebriefView | undefined => (registered.includes(id) ? ({ id, title: `T:${id}`, icon: 'panel', component: async () => null as never } as DebriefView) : undefined),
  };
  return { ctx, added, sizes, extras, defaults: () => defaults };
}

const V = PERSPECTIVE_VIEWS;
const ALL = Object.values(V);

describe('built-in perspectives', () => {
  it('are four, in Ctrl+1..4 order, and only Profiling and Live need a view', () => {
    expect([...BUILT_IN_PERSPECTIVES]).toEqual(['investigate', 'profiling', 'live', 'diagnostics']);
    expect(PERSPECTIVE_REQUIRES).toEqual({ investigate: null, profiling: V.captures, live: V.tailStatus, diagnostics: null });
  });
});

describe('buildPerspective', () => {
  it('Investigate is the default arrangement and nothing else', () => {
    const h = harness(ALL);
    expect(buildPerspective('investigate', h.ctx)).toBe(h.extras);
    expect(h.defaults()).toBe(1);
    expect(h.added).toEqual([]);
  });

  it('Profiling is the captures list alone, narrow, with no auto-hidden tools', () => {
    const h = harness(ALL);
    expect(buildPerspective('profiling', h.ctx)).toEqual(EMPTY_EXTRAS);
    expect(h.defaults()).toBe(0);
    expect(h.added.map((a) => a.id)).toEqual([V.captures]);
    expect(h.sizes[must(h.added[0]).group]).toEqual({ width: 380 });
  });

  it('Profiling falls back to the default arrangement without the captures view', () => {
    const h = harness([]);
    expect(buildPerspective('profiling', h.ctx)).toBe(h.extras);
    expect(h.added).toEqual([]);
  });

  it('Live adds tail status and alerts as tabs below the events, status first', () => {
    const h = harness(ALL);
    expect(buildPerspective('live', h.ctx)).toBe(h.extras);
    expect(h.defaults()).toBe(1);
    expect(h.added.map((a) => [a.id, !!a.inactive, a.group])).toEqual([
      [V.tailStatus, false, 'g1'],
      [V.alerts, true, 'g1'],
    ]);
    expect(h.added[0]?.position).toEqual({ referenceGroup: 'events-group', direction: 'below' });
    expect(h.added[1]?.position).toEqual({ referenceGroup: 'g1', direction: 'within' });
    expect(h.sizes['g1']).toEqual({ height: 260 });
  });

  it('Live skips views that are not registered, and adds nothing when none are', () => {
    const only = harness([V.alerts]);
    buildPerspective('live', only.ctx);
    expect(only.added.map((a) => a.id)).toEqual([V.alerts]);
    expect(only.added[0]?.inactive).toBeUndefined();
    const none = harness([]);
    expect(buildPerspective('live', none.ctx)).toBe(none.extras);
    expect(none.added).toEqual([]);
    expect(none.sizes).toEqual({});
  });

  it('Diagnostics docks problems, jobs, error snapshots, store and plugin health in one group below the events', () => {
    const h = harness(ALL);
    buildPerspective('diagnostics', h.ctx);
    expect(h.added.map((a) => a.id)).toEqual([V.problems, V.jobs, V.errorSnapshots, V.store, V.plugins]);
    expect(new Set(h.added.map((a) => a.group)).size).toBe(1);
    expect(h.added.slice(1).every((a) => a.inactive)).toBe(true);
    expect(h.sizes['g1']).toEqual({ height: 260 });
  });

  it('Diagnostics opens the first registered view when some are missing', () => {
    const h = harness([V.store, V.plugins]);
    buildPerspective('diagnostics', h.ctx);
    expect(h.added.map((a) => [a.id, !!a.inactive])).toEqual([
      [V.store, false],
      [V.plugins, true],
    ]);
  });

  it('places the bottom group without a reference when the events panel is missing', () => {
    const h = harness(ALL, { events: false });
    buildPerspective('live', h.ctx);
    expect(h.added[0]?.position).toEqual({ direction: 'below' });
  });
});

function must<T>(v: T | undefined): T {
  if (v === undefined) throw new Error('missing');
  return v;
}
