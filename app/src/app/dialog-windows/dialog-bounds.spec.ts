import { DIALOG_BOUNDS_KEY, DIALOG_BOUNDS_VERSION, loadDialogBounds, rememberDialogBounds } from './dialog-bounds';

function store(initial?: string) {
  const data = new Map<string, string>(initial === undefined ? [] : [[DIALOG_BOUNDS_KEY, initial]]);
  return { getItem: (k: string) => data.get(k) ?? null, setItem: (k: string, v: string) => void data.set(k, v), data };
}

describe('remembered dialog places', () => {
  it('remembers the size and the offset from the parent per dialog id, versioned', () => {
    const s = store();
    rememberDialogBounds('settings', { dx: 10, dy: 20, width: 900, height: 600 }, s);
    rememberDialogBounds('about', { dx: -1, dy: 2, width: 700, height: 500 }, s);
    expect(loadDialogBounds(s)).toEqual({ settings: { dx: 10, dy: 20, width: 900, height: 600 }, about: { dx: -1, dy: 2, width: 700, height: 500 } });
    expect(JSON.parse(s.data.get(DIALOG_BOUNDS_KEY)!).version).toBe(DIALOG_BOUNDS_VERSION);
  });

  it('tolerates missing, corrupt, old (absolute bounds), foreign-version and malformed data', () => {
    expect(loadDialogBounds(store())).toEqual({});
    expect(loadDialogBounds(store('{not json'))).toEqual({});
    expect(loadDialogBounds(store('null'))).toEqual({});
    // Version 1 held absolute screen bounds: they are dropped, not reused on whatever display they point at.
    expect(loadDialogBounds(store(JSON.stringify({ version: 1, bounds: { a: { x: 3000, y: 1, width: 500, height: 500 } } })))).toEqual({});
    expect(loadDialogBounds(store(JSON.stringify({ version: 99, bounds: { a: { dx: 1, dy: 1, width: 5, height: 5 } } })))).toEqual({});
    expect(loadDialogBounds(store(JSON.stringify({ version: DIALOG_BOUNDS_VERSION, bounds: 'x' })))).toEqual({});
    const mixed = JSON.stringify({
      version: DIALOG_BOUNDS_VERSION,
      bounds: { ok: { dx: 1, dy: 2, width: 3, height: 4, extra: 1 }, abs: { x: 1, y: 2, width: 3, height: 4 }, bad: { dx: 'a', dy: 1, width: 1, height: 1 }, zero: { dx: 0, dy: 0, width: 0, height: 5 }, nan: null },
    });
    expect(loadDialogBounds(store(mixed))).toEqual({ ok: { dx: 1, dy: 2, width: 3, height: 4 } });
    expect(loadDialogBounds(null)).toEqual({});
  });

  it('never throws, and ignores nonsense', () => {
    const s = store();
    rememberDialogBounds('x', { dx: NaN, dy: 0, width: 5, height: 5 }, s);
    expect(s.data.size).toBe(0);
    expect(() => rememberDialogBounds('x', { dx: 1, dy: 1, width: 5, height: 5 }, { getItem: () => null, setItem: () => { throw new Error('full'); } })).not.toThrow();
    expect(() => rememberDialogBounds('x', { dx: 1, dy: 1, width: 5, height: 5 }, null)).not.toThrow();
    expect(loadDialogBounds({ getItem: () => { throw new Error('blocked'); } })).toEqual({});
  });
});
