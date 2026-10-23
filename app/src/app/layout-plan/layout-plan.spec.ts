import { checkPlan, defaultViewLayout, fitAxis, LayoutArea, OpenView, planLayout, Size, ViewLayout } from './layout-plan';

const L = (area: LayoutArea, minW: number, minH: number, prefW: number, prefH: number, priority: number): ViewLayout => ({
  area,
  min: { width: minW, height: minH },
  pref: { width: prefW, height: prefH },
  priority,
});

/** The built-in values of ADR 0056 section 1 (the planner itself knows none of them). */
const V = {
  timeline: { id: 'timeline', row: 'timeline', layout: L('document', 560, 120, 560, 300, 90) },
  events: { id: 'events', row: 'events', layout: L('document', 560, 240, 560, 400, 100) },
  home: { id: 'home', fullHeight: true, layout: L('document', 420, 300, 700, 600, 60) },
  ai: { id: 'ai', layout: L('side', 320, 240, 372, 400, 70) },
  markers: { id: 'markers', row: 'analysis', layout: L('analysis', 560, 180, 560, 280, 50) },
  notes: { id: 'notes', layout: L('side', 320, 240, 372, 400, 50) },
  jobs: { id: 'jobs', layout: L('status', 320, 180, 480, 360, 20) },
} satisfies Record<string, OpenView>;

const widthOf = (plan: ReturnType<typeof planLayout>, id: string) => plan.columns.find((c) => c.views.includes(id))?.width;

describe('planLayout scenarios', () => {
  const set = (...ids: (keyof typeof V)[]): OpenView[] => ids.map((i) => V[i]);
  const table: { name: string; viewport: Size; open: OpenView[]; tabbed: string[]; hidden: string[]; widths: Record<string, number> }[] = [
    { name: 'events+timeline only', viewport: { width: 1440, height: 900 }, open: set('timeline', 'events'), tabbed: [], hidden: [], widths: { events: 1440 } },
    {
      name: 'events + assistant at 1440',
      viewport: { width: 1440, height: 900 },
      open: set('timeline', 'events', 'ai'),
      tabbed: [],
      hidden: [],
      widths: { ai: 372, events: 1068 },
    },
    {
      name: 'the reported scenario at 1440: Home gets its preferred width only if the grid keeps 560',
      viewport: { width: 1440, height: 900 },
      open: set('timeline', 'events', 'ai', 'home', 'markers'),
      tabbed: [],
      hidden: [],
      widths: { ai: 372, home: 508, events: 560 },
    },
    {
      name: 'the same at 1920 gives Home its preference',
      viewport: { width: 1920, height: 1080 },
      open: set('timeline', 'events', 'ai', 'home', 'markers'),
      tabbed: [],
      hidden: [],
      widths: { ai: 372, home: 700, events: 848 },
    },
    {
      name: 'at 1100 Home is tabbed into the events and the assistant stays',
      viewport: { width: 1100, height: 800 },
      open: set('timeline', 'events', 'ai', 'home'),
      tabbed: ['home'],
      hidden: [],
      widths: { ai: 372, events: 728 },
    },
    {
      name: 'at 800 the assistant auto-hides too',
      viewport: { width: 800, height: 700 },
      open: set('timeline', 'events', 'ai', 'home', 'notes'),
      tabbed: ['home'],
      hidden: ['ai', 'notes'],
      widths: { events: 800 },
    },
    {
      name: 'status tools never take room',
      viewport: { width: 1440, height: 900 },
      open: set('timeline', 'events', 'jobs'),
      tabbed: [],
      hidden: ['jobs'],
      widths: { events: 1440 },
    },
  ];
  for (const t of table) {
    it(t.name, () => {
      const plan = planLayout(t.viewport, t.open);
      expect(plan.violations).toEqual([]);
      expect(plan.tabbed.map((x) => x.id).sort()).toEqual([...t.tabbed].sort());
      expect(plan.hidden.map((x) => x.id).sort()).toEqual([...t.hidden].sort());
      for (const [id, w] of Object.entries(t.widths)) expect(widthOf(plan, id), id).toBe(w);
    });
  }

  it('tabbed views name the view they tab into', () => {
    const plan = planLayout({ width: 1100, height: 800 }, set('timeline', 'events', 'ai', 'home'));
    expect(plan.tabbed).toEqual([{ id: 'home', into: 'events' }]);
  });

  it('a short window tabs the lowest-priority row into the top row instead of squeezing it', () => {
    const plan = planLayout({ width: 1440, height: 420 }, set('timeline', 'events', 'markers'));
    expect(plan.violations).toEqual([]);
    expect(plan.tabbed.map((t) => t.id)).toContain('markers');
    for (const r of plan.columns[0]!.rows) expect(r.height).toBeGreaterThanOrEqual(r.minHeight);
  });

  it('shrinks lower priority first, down to its minimum, before dropping anything', () => {
    // 560 + 700 + 372 = 1632 > 1500: Home (60) gives up 132 before the assistant (70) or the grid (100) move.
    const plan = planLayout({ width: 1500, height: 900 }, set('timeline', 'events', 'ai', 'home'));
    expect(plan.tabbed).toEqual([]);
    expect(widthOf(plan, 'home')).toBe(568);
    expect(widthOf(plan, 'ai')).toBe(372);
    expect(plan.shrunk).toContain('home');
  });

  it('saved sizes replace the preferred ones but never go under the minimum', () => {
    const plan = planLayout({ width: 1920, height: 1080 }, set('timeline', 'events', 'ai'), { sizes: { ai: { width: 100 } } });
    expect(widthOf(plan, 'ai')).toBe(320);
    const wide = planLayout({ width: 1920, height: 1080 }, set('timeline', 'events', 'ai'), { sizes: { ai: { width: 500 } } });
    expect(widthOf(wide, 'ai')).toBe(500);
  });

  it('a viewport under the highest-priority view\'s minimum is reported as overflow, not squeezed silently', () => {
    const plan = planLayout({ width: 400, height: 700 }, set('timeline', 'events', 'ai'));
    expect(plan.overflow).toBe(true);
    expect(plan.hidden.map((h) => h.id)).toEqual(['ai']);
  });

  it('with no main column the highest-priority remaining column anchors the tabs', () => {
    const plan = planLayout({ width: 500, height: 700 }, set('home', 'ai'));
    expect(plan.tabbed).toEqual([{ id: 'home', into: 'ai' }]);
    expect(widthOf(plan, 'ai')).toBe(500);
  });

  it('an empty window plans nothing', () => {
    const plan = planLayout({ width: 1000, height: 700 }, []);
    expect(plan.columns).toEqual([]);
    expect(plan.violations).toEqual([]);
  });

  it('is deterministic: the order the views are given in does not matter', () => {
    const a = planLayout({ width: 1300, height: 800 }, set('timeline', 'events', 'ai', 'home', 'markers', 'notes'));
    const b = planLayout({ width: 1300, height: 800 }, set('notes', 'markers', 'home', 'ai', 'events', 'timeline'));
    expect(b).toEqual(a);
  });
});

describe('fitAxis', () => {
  it('gives equal priorities their cut in proportion to their slack', () => {
    const fit = fitAxis(
      [
        { key: 'a', min: 100, pref: 300, priority: 1 },
        { key: 'b', min: 100, pref: 500, priority: 1 },
        { key: 'c', min: 100, pref: 200, priority: 9 },
      ],
      800,
    );
    expect([...fit.sizes.values()].reduce((x, y) => x + y, 0)).toBe(800);
    // c is the anchor (highest priority): elastic, it counts at its minimum, so the others are cut in proportion to their slack.
    expect(fit.sizes.get('c')).toBe(100);
    expect(fit.sizes.get('a')).toBe(267);
    expect(fit.sizes.get('b')).toBe(433);
    expect(fit.sizes.get('a')).toBeGreaterThanOrEqual(100);
    expect(fit.sizes.get('b')).toBeGreaterThan(fit.sizes.get('a')!);
  });
});

describe('defaults for views that declare nothing', () => {
  it('are conservative per area', () => {
    expect(defaultViewLayout('side').min.width).toBe(320);
    expect(defaultViewLayout('analysis').min.height).toBe(180);
    expect(defaultViewLayout('document').min.width).toBe(420);
  });
});

describe('planLayout properties (random viewports and view sets)', () => {
  /** mulberry32, so a failure reproduces from the printed seed. */
  const rng = (seed: number) => () => {
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
  const pool = Object.values(V) as OpenView[];

  function sample(seed: number) {
    const r = rng(seed);
    const open = pool.filter(() => r() < 0.6);
    const viewport = { width: 700 + Math.floor(r() * 1900), height: 500 + Math.floor(r() * 940) };
    const saved = r() < 0.5 ? { sizes: Object.fromEntries(open.map((v) => [v.id, { width: 100 + Math.floor(r() * 900), height: 100 + Math.floor(r() * 600) }])) } : undefined;
    return { open, viewport, saved };
  }

  it('never puts a placed view under its minimum, never overlaps, never loses or duplicates a view', () => {
    for (let seed = 1; seed <= 400; seed++) {
      const { open, viewport, saved } = sample(seed);
      const plan = planLayout(viewport, open, saved);
      expect(checkPlan(plan, open), `seed ${seed}`).toEqual([]);
      expect(plan.violations, `seed ${seed}`).toEqual([]);
      for (const c of plan.columns) {
        for (const id of c.views) expect(c.width, `seed ${seed} ${id}`).toBeGreaterThanOrEqual(open.find((v) => v.id === id)!.layout.min.width);
      }
      expect(plan.columns.reduce((s, c) => s + c.width, 0), `seed ${seed}`).toBeLessThanOrEqual(viewport.width);
    }
  });

  it('is deterministic for the same input', () => {
    for (let seed = 1; seed <= 100; seed++) {
      const { open, viewport, saved } = sample(seed);
      expect(planLayout(viewport, [...open].reverse(), saved), `seed ${seed}`).toEqual(planLayout(viewport, open, saved));
    }
  });

  it('respects priorities: a displaced view is never of higher priority than one that stays placed in a column of the same kind', () => {
    for (let seed = 1; seed <= 400; seed++) {
      const { open, viewport } = sample(seed);
      const plan = planLayout(viewport, open);
      const prio = (id: string) => open.find((v) => v.id === id)!.layout.priority;
      const placed = plan.columns.flatMap((c) => c.views);
      for (const t of plan.tabbed) {
        const kind = (id: string) => (open.find((v) => v.id === id)!.layout.area === 'side' ? 'side' : 'doc');
        // A tabbed document never outranks the lowest-priority placed document column that is not the anchor of its own kind.
        const anchor = plan.columns[0]?.views[0];
        if (anchor && kind(t.id) === 'doc') expect(prio(t.id), `seed ${seed} ${t.id}`).toBeLessThanOrEqual(Math.max(...plan.columns.map((c) => Math.max(...c.views.map(prio)))));
      }
      // Auto-hidden side tools are never of higher priority than every placed side tool of the same column when something placed is lower.
      for (const h of plan.hidden.filter((x) => x.edge === 'right')) {
        const lowerPlacedSide = placed.filter((id) => open.find((v) => v.id === id)!.layout.area === 'side' && prio(id) < prio(h.id));
        expect(lowerPlacedSide, `seed ${seed} ${h.id}`).toEqual([]);
      }
    }
  });

  it('adding a lower-priority view never shrinks a higher-priority one below its preference unless the room is gone', () => {
    for (let seed = 1; seed <= 300; seed++) {
      const { open, viewport } = sample(seed);
      const extra: OpenView = { id: 'extra', layout: L('side', 320, 240, 372, 400, 1) };
      const before = planLayout(viewport, open);
      const after = planLayout(viewport, [...open, extra]);
      for (const c of before.columns) {
        const nowWidth = after.columns.find((a) => a.kind === c.kind)?.width;
        if (nowWidth === undefined || nowWidth >= c.width) continue;
        // It only shrank because the extra column needed the room, and only down to what its own preference or minimum allows.
        const pref = Math.max(...c.views.map((id) => open.find((v) => v.id === id)!.layout.pref.width));
        expect(nowWidth >= pref || after.columns.some((a) => a.views.includes('extra')) || after.hidden.some((h) => h.id === 'extra'), `seed ${seed}`).toBe(true);
      }
    }
  });
});
