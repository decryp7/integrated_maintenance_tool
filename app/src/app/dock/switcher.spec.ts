import { cycle, openSwitcher, selected, selectId, SwitcherItem, switcherItems, touchMru } from './switcher';

const it_ = (id: string): SwitcherItem => ({ id, title: id, kind: 'document' });
const items = ['a', 'b', 'c'].map(it_);

describe('openSwitcher', () => {
  it('is null without panels', () => {
    expect(openSwitcher([])).toBeNull();
    expect(openSwitcher([], true)).toBeNull();
  });

  it('starts on the previous panel going forward and on the last going backward', () => {
    expect(openSwitcher(items)?.index).toBe(1);
    expect(openSwitcher(items, true)?.index).toBe(2);
  });

  it('stays on the only panel', () => {
    expect(openSwitcher([it_('a')])?.index).toBe(0);
    expect(openSwitcher([it_('a')], true)?.index).toBe(0);
  });
});

describe('cycle', () => {
  const at = (index: number) => ({ items, index });

  it('wraps around both ways', () => {
    expect(cycle(at(2), 1).index).toBe(0);
    expect(cycle(at(0), -1).index).toBe(2);
    expect(cycle(at(1), 1).index).toBe(2);
    expect(cycle(at(1), -4).index).toBe(0);
  });

  it('leaves an empty switcher alone', () => {
    const empty = { items: [], index: 0 };
    expect(cycle(empty, 1)).toBe(empty);
  });
});

describe('selectId and selected', () => {
  it('selects by id and keeps the selection for unknown ids', () => {
    const s = { items, index: 0 };
    expect(selectId(s, 'c').index).toBe(2);
    expect(selectId(s, 'zzz')).toBe(s);
  });

  it('returns the selected item, or null when out of range', () => {
    expect(selected({ items, index: 1 })).toBe(items[1]);
    expect(selected({ items, index: 9 })).toBeNull();
    expect(selected({ items: [], index: 0 })).toBeNull();
  });
});

describe('touchMru', () => {
  it('moves an id to the front without duplicating it', () => {
    expect(touchMru(['a', 'b', 'c'], 'c')).toEqual(['c', 'a', 'b']);
    expect(touchMru(['a'], 'z')).toEqual(['z', 'a']);
    expect(touchMru([], 'a')).toEqual(['a']);
  });

  it('drops the oldest past the limit and does not mutate the input', () => {
    const mru = ['a', 'b', 'c'];
    expect(touchMru(mru, 'd', 3)).toEqual(['d', 'a', 'b']);
    expect(mru).toEqual(['a', 'b', 'c']);
  });
});

describe('switcherItems', () => {
  it('orders open panels by use, then never-used ones in their own order', () => {
    expect(switcherItems(['c', 'a'], items).map((i) => i.id)).toEqual(['c', 'a', 'b']);
    expect(switcherItems([], items).map((i) => i.id)).toEqual(['a', 'b', 'c']);
  });

  it('ignores used ids that are no longer open', () => {
    expect(switcherItems(['gone', 'b'], items).map((i) => i.id)).toEqual(['b', 'a', 'c']);
    expect(switcherItems(['a'], [])).toEqual([]);
  });
});
