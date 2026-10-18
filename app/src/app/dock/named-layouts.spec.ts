import { EMPTY_EXTRAS, SavedLayout, toEnvelope } from './layout-store';
import {
  cleanLayoutName,
  LocalStorageNamedLayoutStore,
  MAX_LAYOUT_NAME,
  MAX_NAMED_LAYOUTS,
  moveLayout,
  NamedLayout,
  NamedLayouts,
  NamedLayoutStore,
  parseNamedLayouts,
  removeLayout,
  renameLayout,
  serializeNamedLayouts,
  upsertLayout,
} from './named-layouts';
import { at } from '../../testing/must';

const grid = { grid: { root: { type: 'leaf', data: { views: [], id: '1' } }, height: 1, width: 1, orientation: 'HORIZONTAL' }, panels: {} } as never;
const saved = (): SavedLayout => ({ layout: grid, extras: { ...EMPTY_EXTRAS } });
const nl = (name: string, savedAt = 1): NamedLayout => ({ name, savedAt, saved: saved() });
const names = (l: readonly NamedLayout[]) => l.map((x) => x.name);
const entry = (name: unknown, over: Record<string, unknown> = {}) => ({ name, savedAt: 5, envelope: toEnvelope(saved()), ...over });

describe('cleanLayoutName', () => {
  it('trims and replaces control characters with spaces', () => {
    expect(cleanLayoutName('  My layout  ')).toBe('My layout');
    expect(cleanLayoutName('a\nb\tc\u007fd')).toBe('a b c d');
    expect(cleanLayoutName('\n\u0000x\n')).toBe('x');
  });

  it('truncates to 60 characters and trims again', () => {
    expect(cleanLayoutName('x'.repeat(100))).toBe('x'.repeat(MAX_LAYOUT_NAME));
    expect(cleanLayoutName('x'.repeat(59) + '   tail')).toBe('x'.repeat(59));
  });

  it('is null when nothing is left', () => {
    expect(cleanLayoutName('')).toBeNull();
    expect(cleanLayoutName(' \n\t ')).toBeNull();
  });
});

describe('upsertLayout', () => {
  it('appends new names', () => {
    expect(names(upsertLayout([nl('a')], nl('b')))).toEqual(['a', 'b']);
  });

  it('replaces a same name case-insensitively, in place', () => {
    const next = upsertLayout([nl('Work', 1), nl('b', 2)], nl('WORK', 9));
    expect(next.map((l) => [l.name, l.savedAt])).toEqual([
      ['WORK', 9],
      ['b', 2],
    ]);
  });

  it('drops the oldest past the cap', () => {
    const full = Array.from({ length: MAX_NAMED_LAYOUTS }, (_, i) => nl(`l${i}`));
    const next = upsertLayout(full, nl('new'));
    expect(next).toHaveLength(MAX_NAMED_LAYOUTS);
    expect(at(next, 0).name).toBe('l1');
    expect(at(next, next.length - 1).name).toBe('new');
    expect(upsertLayout(full, nl('L0'))).toHaveLength(MAX_NAMED_LAYOUTS);
  });

  it('does not mutate the input', () => {
    const list = [nl('a')];
    upsertLayout(list, nl('b'));
    expect(list).toHaveLength(1);
  });
});

describe('removeLayout', () => {
  it('removes by exact name', () => {
    expect(names(removeLayout([nl('a'), nl('b')], 'a'))).toEqual(['b']);
    expect(names(removeLayout([nl('a')], 'A'))).toEqual(['a']);
    expect(removeLayout([], 'a')).toEqual([]);
  });
});

describe('renameLayout', () => {
  const list = [nl('a'), nl('b')];

  it('renames and cleans the new name', () => {
    expect(names(renameLayout(list, 'a', '  c\n'))).toEqual(['c', 'b']);
  });

  it('refuses empty names and names taken by another layout, case-insensitively', () => {
    expect(names(renameLayout(list, 'a', '  '))).toEqual(['a', 'b']);
    expect(names(renameLayout(list, 'a', 'B'))).toEqual(['a', 'b']);
  });

  it('allows changing only the case of its own name', () => {
    expect(names(renameLayout(list, 'a', 'A'))).toEqual(['A', 'b']);
  });

  it('returns a copy and keeps the list for an unknown source', () => {
    const same = renameLayout(list, 'zzz', 'q');
    expect(names(same)).toEqual(['a', 'b']);
    expect(same).not.toBe(list);
  });
});

describe('moveLayout', () => {
  const list = [nl('a'), nl('b'), nl('c')];

  it('swaps with the neighbour', () => {
    expect(names(moveLayout(list, 'b', -1))).toEqual(['b', 'a', 'c']);
    expect(names(moveLayout(list, 'b', 1))).toEqual(['a', 'c', 'b']);
  });

  it('does nothing at the bounds or for unknown names', () => {
    expect(names(moveLayout(list, 'a', -1))).toEqual(['a', 'b', 'c']);
    expect(names(moveLayout(list, 'c', 1))).toEqual(['a', 'b', 'c']);
    expect(names(moveLayout(list, 'zzz', 1))).toEqual(['a', 'b', 'c']);
    expect(names(moveLayout(list, 'zzz', -1))).toEqual(['a', 'b', 'c']);
  });
});

describe('parseNamedLayouts', () => {
  const wrap = (layouts: unknown, version: unknown = 1) => ({ version, layouts });

  it('is empty for the wrong version or shape', () => {
    for (const raw of [null, undefined, 'x', 3, {}, wrap([entry('a')], 2), wrap([entry('a')], null), wrap('nope'), wrap({})]) {
      expect(parseNamedLayouts(raw)).toEqual([]);
    }
  });

  it('drops malformed entries', () => {
    const out = parseNamedLayouts(
      wrap([null, 'x', entry(5), entry('   '), entry('nover', { envelope: { layout: grid } }), entry('badenv', { envelope: null }), entry('good')]),
    );
    expect(names(out)).toEqual(['good']);
  });

  it('drops duplicate names case-insensitively, keeping the first', () => {
    expect(names(parseNamedLayouts(wrap([entry('Dup', { savedAt: 1 }), entry('dup', { savedAt: 2 }), entry('x')])))).toEqual(['Dup', 'x']);
  });

  it('cleans names and defaults a missing savedAt to 0', () => {
    const l = at(parseNamedLayouts(wrap([entry(' a\nb ', { savedAt: 'yesterday' })])), 0);
    expect(l.name).toBe('a b');
    expect(l.savedAt).toBe(0);
  });

  it('migrates v1 layout envelopes', () => {
    const l = at(parseNamedLayouts(wrap([entry('old', { envelope: { version: 1, layout: grid } })])), 0);
    expect(l.saved).toEqual({ layout: grid, extras: EMPTY_EXTRAS });
  });

  it('keeps at most the cap', () => {
    const many = Array.from({ length: MAX_NAMED_LAYOUTS + 5 }, (_, i) => entry(`l${i}`));
    expect(parseNamedLayouts(wrap(many))).toHaveLength(MAX_NAMED_LAYOUTS);
  });

  it('round-trips through serialize', () => {
    const list = [{ ...nl('a', 10), saved: { layout: grid, extras: { ...EMPTY_EXTRAS, pinnedTabs: ['p'] } } }, nl('b', 20)];
    expect(parseNamedLayouts(JSON.parse(serializeNamedLayouts(list)))).toEqual(list);
  });
});

describe('LocalStorageNamedLayoutStore', () => {
  afterEach(() => {
    vi.restoreAllMocks();
    localStorage.clear();
  });

  it('saves and loads through localStorage under its key', () => {
    const store = new LocalStorageNamedLayoutStore('test.layouts');
    expect(store.load()).toEqual([]);
    store.save([nl('a', 3)]);
    expect(localStorage.getItem('test.layouts')).toBeTruthy();
    expect(new LocalStorageNamedLayoutStore('test.layouts').load()).toEqual([nl('a', 3)]);
  });

  it('loads an empty list from corrupt JSON or foreign data', () => {
    localStorage.setItem('k', '{not json');
    expect(new LocalStorageNamedLayoutStore('k').load()).toEqual([]);
    localStorage.setItem('k', '{"version":9,"layouts":[]}');
    expect(new LocalStorageNamedLayoutStore('k').load()).toEqual([]);
  });

  it('never throws when storage does', () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('denied');
    });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('full');
    });
    const store = new LocalStorageNamedLayoutStore('k');
    expect(store.load()).toEqual([]);
    expect(() => store.save([nl('a')])).not.toThrow();
  });
});

describe('NamedLayouts', () => {
  class FakeStore implements NamedLayoutStore {
    saves: NamedLayout[][] = [];
    constructor(public initial: NamedLayout[] = []) {}
    load() {
      return this.initial;
    }
    save(list: readonly NamedLayout[]) {
      this.saves.push([...list]);
    }
  }

  it('starts from the stored list', () => {
    expect(names(new NamedLayouts(new FakeStore([nl('a')])).list())).toEqual(['a']);
  });

  it('saves under the cleaned name and persists', () => {
    const store = new FakeStore();
    const layouts = new NamedLayouts(store);
    expect(layouts.save('  Debug\n', saved(), 42)).toBe('Debug');
    expect(layouts.list()[0]).toMatchObject({ name: 'Debug', savedAt: 42 });
    expect(names(at(store.saves, 0))).toEqual(['Debug']);
  });

  it('refuses an empty name without touching the store', () => {
    const store = new FakeStore();
    const layouts = new NamedLayouts(store);
    expect(layouts.save('  ', saved())).toBeNull();
    expect(layouts.list()).toEqual([]);
    expect(store.saves).toEqual([]);
  });

  it('saving the same name again replaces it', () => {
    const layouts = new NamedLayouts(new FakeStore());
    layouts.save('a', saved(), 1);
    layouts.save('A', saved(), 2);
    expect(layouts.list().map((l) => [l.name, l.savedAt])).toEqual([['A', 2]]);
  });

  it('renames, reporting whether anything changed', () => {
    const store = new FakeStore([nl('a'), nl('b')]);
    const layouts = new NamedLayouts(store);
    expect(layouts.rename('a', 'c')).toBe(true);
    expect(names(layouts.list())).toEqual(['c', 'b']);
    expect(store.saves).toHaveLength(1);
    expect(layouts.rename('c', 'b')).toBe(false);
    expect(layouts.rename('c', ' ')).toBe(false);
    expect(layouts.rename('c', 'c')).toBe(false);
    expect(layouts.rename('zzz', 'q')).toBe(false);
    expect(store.saves).toHaveLength(1);
  });

  it('moves, removes, finds and indexes layouts', () => {
    const store = new FakeStore([nl('a'), nl('b'), nl('c')]);
    const layouts = new NamedLayouts(store);
    layouts.move('c', -1);
    expect(names(layouts.list())).toEqual(['a', 'c', 'b']);
    expect(layouts.at(1)?.name).toBe('a');
    expect(layouts.at(3)?.name).toBe('b');
    expect(layouts.at(0)).toBeUndefined();
    expect(layouts.at(4)).toBeUndefined();
    expect(layouts.find('c')?.name).toBe('c');
    expect(layouts.find('zzz')).toBeUndefined();
    layouts.remove('a');
    expect(names(layouts.list())).toEqual(['c', 'b']);
    expect(names(at(store.saves, store.saves.length - 1))).toEqual(['c', 'b']);
  });
});
