import { AutoHideItem, AutoHideState, autoHideReduce, clampSize, EMPTY_AUTO_HIDE, itemsOn, MAX_FLYOUT, MIN_FLYOUT, sanitizeAutoHide } from './auto-hide';
import { at } from '../../testing/must';

const item = (id: string, side: AutoHideItem['side'] = 'left', size = 300): AutoHideItem => ({
  id,
  title: id.toUpperCase(),
  side,
  size,
  slot: { referencePanel: null, direction: 'within' },
});
const state = (open: string | null = null, openedBy: AutoHideState['openedBy'] = null): AutoHideState => ({
  items: [item('a'), item('b', 'right')],
  open,
  openedBy,
});

describe('autoHideReduce', () => {
  it('unpin adds an item (closing any slide-out) and clamps its size', () => {
    const s = autoHideReduce(state('a', 'click'), { type: 'unpin', item: item('c', 'bottom', 5000) });
    expect(s.items.map((i) => i.id)).toEqual(['a', 'b', 'c']);
    expect(at(s.items, 2).size).toBe(MAX_FLYOUT);
    expect(s.open).toBeNull();
    expect(s.openedBy).toBeNull();
  });

  it('unpin replaces an existing id, moving it to the end', () => {
    const s = autoHideReduce(state(), { type: 'unpin', item: item('a', 'bottom', 400) });
    expect(s.items.map((i) => i.id)).toEqual(['b', 'a']);
    expect(s.items[1]).toMatchObject({ side: 'bottom', size: 400 });
  });

  it('pin removes the item; if it was open the slide-out closes too', () => {
    const s = autoHideReduce(state('a', 'click'), { type: 'pin', id: 'a' });
    expect(s).toEqual({ items: [item('b', 'right')], open: null, openedBy: null });
    const other = autoHideReduce(state('b', 'click'), { type: 'pin', id: 'a' });
    expect(other.open).toBe('b');
    expect(other.openedBy).toBe('click');
    expect(other.items.map((i) => i.id)).toEqual(['b']);
  });

  it('hover opens an item as a peek', () => {
    expect(autoHideReduce(state(), { type: 'hover', id: 'a' })).toMatchObject({ open: 'a', openedBy: 'hover' });
    expect(autoHideReduce(state('a', 'hover'), { type: 'hover', id: 'b' })).toMatchObject({ open: 'b', openedBy: 'hover' });
  });

  it('hover never overrides an item opened by click or keyboard', () => {
    const clicked = state('a', 'click');
    expect(autoHideReduce(clicked, { type: 'hover', id: 'b' })).toBe(clicked);
    const keyed = state('a', 'keyboard');
    expect(autoHideReduce(keyed, { type: 'hover', id: 'b' })).toBe(keyed);
  });

  it('click opens, and clicking an open item closes it', () => {
    const opened = autoHideReduce(state(), { type: 'click', id: 'a' });
    expect(opened).toMatchObject({ open: 'a', openedBy: 'click' });
    expect(autoHideReduce(opened, { type: 'click', id: 'a' })).toMatchObject({ open: null, openedBy: null });
    expect(autoHideReduce(opened, { type: 'click', id: 'b' })).toMatchObject({ open: 'b', openedBy: 'click' });
  });

  it('click on a peeking item makes it stay open', () => {
    expect(autoHideReduce(state('a', 'hover'), { type: 'click', id: 'a' })).toMatchObject({ open: 'a', openedBy: 'click' });
  });

  it('activate opens by keyboard, even over a click-opened item', () => {
    expect(autoHideReduce(state('a', 'click'), { type: 'activate', id: 'b' })).toMatchObject({ open: 'b', openedBy: 'keyboard' });
    expect(autoHideReduce(state('a', 'keyboard'), { type: 'activate', id: 'a' })).toMatchObject({ open: 'a', openedBy: 'keyboard' });
  });

  it('leave closes only a hover-opened item', () => {
    expect(autoHideReduce(state('a', 'hover'), { type: 'leave' })).toMatchObject({ open: null, openedBy: null });
    const clicked = state('a', 'click');
    expect(autoHideReduce(clicked, { type: 'leave' })).toBe(clicked);
    const keyed = state('a', 'keyboard');
    expect(autoHideReduce(keyed, { type: 'leave' })).toBe(keyed);
  });

  it.each(['escape', 'blur'] as const)('%s closes whatever is open', (type) => {
    for (const by of ['hover', 'click', 'keyboard'] as const) {
      expect(autoHideReduce(state('a', by), { type })).toMatchObject({ open: null, openedBy: null });
    }
    const idle = state();
    expect(autoHideReduce(idle, { type })).toBe(idle);
  });

  it('resize clamps the size of one item only', () => {
    expect(autoHideReduce(state(), { type: 'resize', id: 'a', size: 10 }).items.map((i) => i.size)).toEqual([MIN_FLYOUT, 300]);
    expect(autoHideReduce(state(), { type: 'resize', id: 'b', size: 500.4 }).items.map((i) => i.size)).toEqual([300, 500]);
  });

  it('ignores unknown ids', () => {
    const s = state('a', 'click');
    for (const type of ['pin', 'hover', 'click', 'activate'] as const) {
      expect(autoHideReduce(s, { type, id: 'zzz' })).toBe(s);
    }
    expect(autoHideReduce(s, { type: 'resize', id: 'zzz', size: 999 }).items).toEqual(s.items);
  });

  it('starts empty', () => {
    expect(EMPTY_AUTO_HIDE).toEqual({ items: [], open: null, openedBy: null });
  });
});

describe('itemsOn', () => {
  it('lists the items of one side in order', () => {
    const s: AutoHideState = { ...EMPTY_AUTO_HIDE, items: [item('a'), item('b', 'right'), item('c')] };
    expect(itemsOn(s, 'left').map((i) => i.id)).toEqual(['a', 'c']);
    expect(itemsOn(s, 'right').map((i) => i.id)).toEqual(['b']);
    expect(itemsOn(s, 'bottom')).toEqual([]);
  });
});

describe('clampSize', () => {
  it('rounds and clamps to the flyout range', () => {
    expect(clampSize(300.6)).toBe(301);
    expect(clampSize(0)).toBe(MIN_FLYOUT);
    expect(clampSize(-5)).toBe(MIN_FLYOUT);
    expect(clampSize(99999)).toBe(MAX_FLYOUT);
    expect(clampSize(Infinity)).toBe(320);
  });

  it('falls back to 320 for NaN', () => {
    expect(clampSize(NaN)).toBe(320);
  });
});

describe('sanitizeAutoHide', () => {
  it('is empty for anything that is not an array', () => {
    for (const raw of [null, undefined, 'x', 5, {}, { id: 'a' }]) expect(sanitizeAutoHide(raw)).toEqual([]);
  });

  it('keeps a well-formed item as is', () => {
    const good = { id: 'a', title: 'A', side: 'right', size: 400, slot: { referencePanel: 'p', direction: 'below', index: 2, size: 50 } };
    expect(sanitizeAutoHide([good])).toEqual([good]);
  });

  it('drops malformed entries and duplicate ids', () => {
    const out = sanitizeAutoHide([
      null,
      'x',
      { id: '', side: 'left' },
      { id: 5, side: 'left' },
      { id: 'a' },
      { id: 'a2', side: 'top' },
      { id: 'ok', side: 'left' },
      { id: 'ok', side: 'right' },
    ]);
    expect(out.map((i) => [i.id, i.side])).toEqual([['ok', 'left']]);
  });

  it('applies defaults for missing or invalid fields', () => {
    const [i] = sanitizeAutoHide([{ id: 'a', side: 'bottom', title: 7, size: 'big', slot: { referencePanel: 3, direction: 'diagonal', index: 'x', size: null } }]);
    expect(i).toEqual({ id: 'a', title: 'a', side: 'bottom', size: 320, slot: { referencePanel: null, direction: 'within' } });
  });

  it('clamps the size and tolerates a missing slot', () => {
    expect(sanitizeAutoHide([{ id: 'a', side: 'left', size: 5 }])[0]).toMatchObject({ size: MIN_FLYOUT, slot: { referencePanel: null, direction: 'within' } });
  });
});
