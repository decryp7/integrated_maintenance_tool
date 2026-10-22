import {
  applyOverrides,
  conflictKey,
  findConflicts,
  MAX_OVERRIDES,
  normalizeBinding,
  recordKey,
  sameBinding,
  sanitizeOverrides,
  withOverride,
  withoutOverride,
} from './keybinding-overrides';

const key = (init: Partial<KeyboardEvent> & { key: string }) => ({ code: '', ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, ...init });

describe('normalizeBinding', () => {
  it('spells modifiers and keys one way', () => {
    expect(normalizeBinding('shift+mod+p')).toBe('Mod+Shift+P');
    expect(normalizeBinding('Alt+ArrowLeft')).toBe('Alt+Left');
    expect(normalizeBinding('ctrl + alt + h')).toBe('Ctrl+Alt+H');
    expect(normalizeBinding('f6')).toBe('F6');
    expect(normalizeBinding('Mod+/')).toBe('Mod+/');
    expect(normalizeBinding('Mod+ ')).toBeNull();
  });

  it('refuses what is not a binding', () => {
    for (const bad of ['', '+', 'Mod+', 'Hyper+K', 'Mod+Mod+K', 'Mod+Banana', 'F25', 'x'.repeat(50)]) expect(normalizeBinding(bad)).toBeNull();
    expect(normalizeBinding(5 as unknown as string)).toBeNull();
  });
});

describe('conflicts', () => {
  it('resolves Mod to the platform modifier', () => {
    expect(conflictKey('Mod+K', false)).toBe('Ctrl+k');
    expect(conflictKey('Mod+K', true)).toBe('Meta+k');
    expect(conflictKey('nonsense+', true)).toBeNull();
    expect(sameBinding('Mod+K', 'Ctrl+K', false)).toBe(true);
    expect(sameBinding('Mod+K', 'Ctrl+K', true)).toBe(false);
    expect(sameBinding('Mod+K', undefined)).toBe(false);
    expect(sameBinding('bad+', 'bad+')).toBe(false);
  });

  it('lists other commands and fixed keys on the same key, never the command itself', () => {
    const commands = [
      { id: 'a', title: 'A', keybinding: 'Mod+K' },
      { id: 'b', title: 'B', keybinding: 'Mod+Shift+K' },
      { id: 'c', title: 'C' },
      { id: 'd', title: 'D', keybinding: 'Ctrl+Alt+1' },
    ];
    const fixed = [{ binding: 'Mod+Enter', description: 'Run the query' }, { description: 'text only' }];
    expect(findConflicts('Ctrl+K', 'b', commands, fixed, false)).toEqual([{ kind: 'command', id: 'a', title: 'A' }]);
    expect(findConflicts('Mod+K', 'a', commands, fixed, false)).toEqual([]);
    expect(findConflicts('Ctrl+Enter', 'a', commands, fixed, false)).toEqual([{ kind: 'fixed', description: 'Run the query' }]);
    // The perspective switch (Ctrl+1..9) and the panel group focus (Ctrl+Alt+1..9) are told apart by Alt.
    expect(findConflicts('Mod+Alt+1', 'a', commands, fixed, false)).toEqual([{ kind: 'command', id: 'd', title: 'D' }]);
    expect(findConflicts('Mod+1', 'a', commands, fixed, false)).toEqual([]);
  });
});

describe('recordKey', () => {
  it('records a chord with the platform modifier as Mod', () => {
    expect(recordKey(key({ key: 'k', ctrlKey: true }), false)).toEqual({ kind: 'binding', binding: 'Mod+K' });
    expect(recordKey(key({ key: 'K', ctrlKey: true, shiftKey: true }), false)).toEqual({ kind: 'binding', binding: 'Mod+Shift+K' });
    expect(recordKey(key({ key: 'k', metaKey: true }), true)).toEqual({ kind: 'binding', binding: 'Mod+K' });
    expect(recordKey(key({ key: 'k', ctrlKey: true }), true)).toEqual({ kind: 'binding', binding: 'Ctrl+K' });
    expect(recordKey(key({ key: 'k', metaKey: true }), false)).toEqual({ kind: 'binding', binding: 'Meta+K' });
    expect(recordKey(key({ key: 'ArrowLeft', altKey: true }), false)).toEqual({ kind: 'binding', binding: 'Alt+Left' });
    expect(recordKey(key({ key: 'F6' }), false)).toEqual({ kind: 'binding', binding: 'F6' });
  });

  it('uses the physical letter where Option turns it into a symbol', () => {
    expect(recordKey(key({ key: 'å', code: 'KeyA', altKey: true }), true)).toEqual({ kind: 'binding', binding: 'Alt+A' });
  });

  it('cancels on Escape, clears on Backspace or Delete, waits on modifiers', () => {
    expect(recordKey(key({ key: 'Escape' }))).toEqual({ kind: 'cancel' });
    expect(recordKey(key({ key: 'Backspace' }))).toEqual({ kind: 'clear' });
    expect(recordKey(key({ key: 'Delete' }))).toEqual({ kind: 'clear' });
    expect(recordKey(key({ key: 'Control', ctrlKey: true }))).toEqual({ kind: 'wait' });
    expect(recordKey(key({ key: 'Shift', shiftKey: true }))).toEqual({ kind: 'wait' });
  });

  it('refuses keys that would fire while typing', () => {
    expect(recordKey(key({ key: 'a' }))).toEqual({ kind: 'invalid' });
    expect(recordKey(key({ key: 'A', shiftKey: true }))).toEqual({ kind: 'invalid' });
    expect(recordKey(key({ key: 'Tab' }))).toEqual({ kind: 'invalid' });
    expect(recordKey(key({ key: '+', ctrlKey: true }))).toEqual({ kind: 'invalid' });
    expect(recordKey(key({ key: 'Escape', ctrlKey: true }), false)).toEqual({ kind: 'binding', binding: 'Mod+Escape' });
  });
});

describe('merging overrides', () => {
  it('sets, clears and drops overrides that equal the declared key', () => {
    let o = withOverride({}, 'a', 'mod+j', 'Mod+K');
    expect(o).toEqual({ a: 'Mod+J' });
    o = withOverride(o, 'a', null, 'Mod+K');
    expect(o).toEqual({ a: null });
    expect(withOverride(o, 'a', 'Mod+K', 'Mod+K')).toEqual({});
    expect(withOverride({}, 'b', null, undefined)).toEqual({});
    expect(withOverride({}, 'b', 'Mod+J', undefined)).toEqual({ b: 'Mod+J' });
    expect(withOverride({}, 'b', 'bad+', undefined)).toEqual({ b: 'bad+' });
    expect(withoutOverride({ a: 'F1', b: null }, 'a')).toEqual({ b: null });
  });

  it('applies overrides without touching the other commands', () => {
    const cmds = [
      { id: 'a', keybinding: 'Mod+K' },
      { id: 'b', keybinding: 'Mod+J' },
      { id: 'c' },
      { id: 'd', keybinding: 'F1' },
    ];
    const out = applyOverrides(cmds, { a: 'Mod+L', b: null, c: 'F2' });
    expect(out).toEqual([{ id: 'a', keybinding: 'Mod+L' }, { id: 'b' }, { id: 'c', keybinding: 'F2' }, { id: 'd', keybinding: 'F1' }]);
    expect('keybinding' in out[1]!).toBe(false);
    expect(out[3]).toBe(cmds[3]);
    expect(cmds[0]!.keybinding).toBe('Mod+K');
  });
});

describe('sanitizeOverrides', () => {
  it('keeps valid entries and drops the rest', () => {
    expect(sanitizeOverrides({ a: 'mod+k', b: null, c: 'nonsense+', d: 5, '': 'F1', ['x'.repeat(300)]: 'F1' })).toEqual({ a: 'Mod+K', b: null });
    for (const bad of [null, undefined, 'x', 3, [1]]) expect(sanitizeOverrides(bad)).toEqual({});
  });

  it('is bounded and ignores a prototype key', () => {
    const many = Object.fromEntries(Array.from({ length: MAX_OVERRIDES + 50 }, (_, i) => [`c${i}`, 'F1']));
    expect(Object.keys(sanitizeOverrides(many)).length).toBe(MAX_OVERRIDES);
    expect(Object.keys(sanitizeOverrides(JSON.parse('{"__proto__":"F1","a":"F2"}')))).toEqual(['a']);
  });
});
