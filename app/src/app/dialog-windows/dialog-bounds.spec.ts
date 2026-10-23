import { DIALOG_BOUNDS_KEY, DIALOG_BOUNDS_VERSION, loadDialogBounds, rememberDialogBounds } from './dialog-bounds';

function store(initial?: string) {
  const data = new Map<string, string>(initial === undefined ? [] : [[DIALOG_BOUNDS_KEY, initial]]);
  return { getItem: (k: string) => data.get(k) ?? null, setItem: (k: string, v: string) => void data.set(k, v), data };
}

describe('remembered dialog bounds', () => {
  it('remembers the size and position per dialog id, versioned', () => {
    const s = store();
    rememberDialogBounds('settings', { x: 10, y: 20, width: 900, height: 600 }, s);
    rememberDialogBounds('about', { x: 1, y: 2, width: 700, height: 500 }, s);
    expect(loadDialogBounds(s)).toEqual({ settings: { x: 10, y: 20, width: 900, height: 600 }, about: { x: 1, y: 2, width: 700, height: 500 } });
    expect(JSON.parse(s.data.get(DIALOG_BOUNDS_KEY)!).version).toBe(DIALOG_BOUNDS_VERSION);
  });

  it('tolerates missing, corrupt, foreign-version and malformed data', () => {
    expect(loadDialogBounds(store())).toEqual({});
    expect(loadDialogBounds(store('{not json'))).toEqual({});
    expect(loadDialogBounds(store('null'))).toEqual({});
    expect(loadDialogBounds(store(JSON.stringify({ version: 99, bounds: { a: { x: 1, y: 1, width: 5, height: 5 } } })))).toEqual({});
    expect(loadDialogBounds(store(JSON.stringify({ version: 1, bounds: 'x' })))).toEqual({});
    const mixed = JSON.stringify({ version: 1, bounds: { ok: { x: 1, y: 2, width: 3, height: 4, extra: 1 }, bad: { x: 'a', y: 1, width: 1, height: 1 }, zero: { x: 0, y: 0, width: 0, height: 5 }, nan: null } });
    expect(loadDialogBounds(store(mixed))).toEqual({ ok: { x: 1, y: 2, width: 3, height: 4 } });
    expect(loadDialogBounds(null)).toEqual({});
  });

  it('never throws, and ignores nonsense bounds', () => {
    const s = store();
    rememberDialogBounds('x', { x: NaN, y: 0, width: 5, height: 5 }, s);
    expect(s.data.size).toBe(0);
    expect(() => rememberDialogBounds('x', { x: 1, y: 1, width: 5, height: 5 }, { getItem: () => null, setItem: () => { throw new Error('full'); } })).not.toThrow();
    expect(() => rememberDialogBounds('x', { x: 1, y: 1, width: 5, height: 5 }, null)).not.toThrow();
    expect(loadDialogBounds({ getItem: () => { throw new Error('blocked'); } })).toEqual({});
  });
});
