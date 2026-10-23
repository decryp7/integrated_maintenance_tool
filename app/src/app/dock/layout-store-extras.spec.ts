import { EMPTY_EXTRAS, ENVELOPE_VERSION, readLayout, sanitizeExtras, toEnvelope } from './layout-store';
import { at } from '../../testing/must';

const layout = { grid: { root: { type: 'leaf', data: { views: [], id: '1' } }, height: 1, width: 1, orientation: 'HORIZONTAL' }, panels: {} } as never;

describe('sanitizeExtras', () => {
  it('returns empty extras (a fresh copy) for non-objects', () => {
    for (const raw of [null, undefined, 'x', 3, [], [1]]) {
      const e = sanitizeExtras(raw);
      expect(e).toEqual(EMPTY_EXTRAS);
      expect(e).not.toBe(EMPTY_EXTRAS);
    }
    expect(sanitizeExtras({})).toEqual(EMPTY_EXTRAS);
  });

  it('keeps unique, non-empty string pinned tab ids only', () => {
    expect(sanitizeExtras({ pinnedTabs: ['a', 'a', '', 5, null, 'b'] }).pinnedTabs).toEqual(['a', 'b']);
    expect(sanitizeExtras({ pinnedTabs: 'a' }).pinnedTabs).toEqual([]);
  });

  it('drops malformed and duplicate popped-out panels', () => {
    const e = sanitizeExtras({ poppedOut: [null, 'x', { id: '' }, { id: 4 }, { id: 'p' }, { id: 'p', monitor: 'M' }, { id: 'q' }] });
    expect(e.poppedOut.map((p) => p.id)).toEqual(['p', 'q']);
    expect(at(e.poppedOut, 0).monitor).toBeNull();
    expect(sanitizeExtras({ poppedOut: 'p' }).poppedOut).toEqual([]);
  });

  it('keeps finite bounds and turns invalid ones into null', () => {
    const bounds = (b: unknown) => at(sanitizeExtras({ poppedOut: [{ id: 'p', bounds: b }] }).poppedOut, 0).bounds;
    expect(bounds({ x: 1, y: -2, width: 300, height: 200, extra: 9 })).toEqual({ x: 1, y: -2, width: 300, height: 200 });
    expect(bounds({ x: NaN, y: 0, width: 1, height: 1 })).toBeNull();
    expect(bounds({ x: 0, y: 0, width: Infinity, height: 1 })).toBeNull();
    expect(bounds({ x: '0', y: 0, width: 1, height: 1 })).toBeNull();
    expect(bounds({ x: 0, y: 0, width: 1 })).toBeNull();
    expect(bounds('big')).toBeNull();
    expect(bounds(undefined)).toBeNull();
  });

  it('sanitizes the slot of a popped-out panel', () => {
    const slot = (s: unknown) => at(sanitizeExtras({ poppedOut: [{ id: 'p', slot: s, monitor: 'DP-1' }] }).poppedOut, 0);
    expect(slot({ referencePanel: 'r', direction: 'above', index: 2, size: 120 })).toMatchObject({
      monitor: 'DP-1',
      slot: { referencePanel: 'r', direction: 'above', index: 2, size: 120 },
    });
    expect(slot({ referencePanel: 1, direction: 'sideways', index: NaN, size: '5' }).slot).toEqual({ referencePanel: null, direction: 'within' });
    expect(slot(undefined).slot).toEqual({ referencePanel: null, direction: 'within' });
    expect(slot([]).slot).toEqual({ referencePanel: null, direction: 'within' });
  });

  it('sanitizes the auto-hidden items too', () => {
    const e = sanitizeExtras({ autoHidden: [{ id: 'a', side: 'left' }, { id: 'b', side: 'nope' }] });
    expect(e.autoHidden.map((i) => i.id)).toEqual(['a']);
  });
});

describe('readLayout', () => {
  it('reads v2 with sanitized extras', () => {
    const saved = readLayout({ version: 2, layout, extras: { pinnedTabs: ['a', 'a'] } });
    expect(saved?.layout).toBe(layout);
    expect(saved?.extras).toEqual({ autoHidden: [], pinnedTabs: ['a'], poppedOut: [] });
    expect(readLayout({ version: 2, layout })?.extras).toEqual(EMPTY_EXTRAS);
  });

  it('is null for unknown versions and non-objects', () => {
    for (const version of [0, 3, '2', null, undefined]) expect(readLayout({ version, layout })).toBeNull();
    for (const raw of [null, undefined, 'x', 2, [], { version: 2 }, { version: 2, layout: 'x' }, { version: 2, layout: [] }]) {
      expect(readLayout(raw)).toBeNull();
    }
  });
});

describe('toEnvelope', () => {
  it('stamps the current version and round-trips through readLayout', () => {
    const saved = { layout, extras: { autoHidden: [], pinnedTabs: ['a'], poppedOut: [] } };
    const env = toEnvelope(saved);
    expect(env).toEqual({ version: ENVELOPE_VERSION, layout, extras: saved.extras });
    expect(readLayout(JSON.parse(JSON.stringify(env)))).toEqual(saved);
  });
});

describe('sanitizeExtras auto-hide sides', () => {
  it('keeps the remembered side per pane, and loads layouts saved without it', () => {
    expect(sanitizeExtras({ autoHideSides: { inspector: 'bottom', x: 'top' } }).autoHideSides).toEqual({ inspector: 'bottom' });
    expect(sanitizeExtras({ autoHidden: [], pinnedTabs: [], poppedOut: [] }).autoHideSides).toBeUndefined();
  });
});

describe('sanitizeExtras: displaced views and user sizes (ADR 0056)', () => {
  it('keeps well-formed records, drops malformed and duplicate ones, and tolerates their absence', () => {
    const e = sanitizeExtras({
      displaced: [
        { id: 'home', slot: { referencePanel: null, direction: 'within', edge: 'right', size: 700 }, into: 'events' },
        { id: 'home', into: 'x' },
        { id: 'ai-chat', slot: 7, into: null },
        { slot: {} },
        'junk',
      ],
      sizes: { 'ai-chat': { width: 372, height: -4 }, bad: { width: 'x' }, worse: 5, home: { height: 600.4 } },
    });
    expect(e.displaced?.map((d) => d.id)).toEqual(['home', 'ai-chat']);
    expect(e.displaced?.[0]?.into).toBe('events');
    expect(e.displaced?.[1]?.into).toBeNull();
    expect(e.sizes).toEqual({ 'ai-chat': { width: 372 }, home: { height: 600.4 } });
    const none = sanitizeExtras({});
    expect(none.displaced).toBeUndefined();
    expect(none.sizes).toBeUndefined();
  });
});
