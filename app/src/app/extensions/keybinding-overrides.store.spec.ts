import { TestBed } from '@angular/core/testing';
import { KEYBINDINGS_KEY, KEYBINDINGS_VERSION, KeybindingOverridesStore, loadOverrides, saveOverrides } from './keybinding-overrides.store';

describe('keybinding overrides storage', () => {
  beforeEach(() => localStorage.clear());
  afterEach(() => vi.restoreAllMocks());

  it('round-trips versioned overrides', () => {
    saveOverrides({ a: 'Mod+K', b: null });
    expect(JSON.parse(localStorage.getItem(KEYBINDINGS_KEY)!)).toEqual({ version: KEYBINDINGS_VERSION, overrides: { a: 'Mod+K', b: null } });
    expect(loadOverrides()).toEqual({ a: 'Mod+K', b: null });
    saveOverrides({});
    expect(localStorage.getItem(KEYBINDINGS_KEY)).toBeNull();
  });

  it('falls back to the defaults for missing, corrupt or foreign data', () => {
    expect(loadOverrides()).toEqual({});
    for (const raw of ['{not json', '[1]', 'null', '"x"', JSON.stringify({ version: 99, overrides: { a: 'F1' } }), JSON.stringify({ overrides: { a: 'F1' } }), JSON.stringify({ version: 1, overrides: 'x' })]) {
      localStorage.setItem(KEYBINDINGS_KEY, raw);
      expect(loadOverrides()).toEqual({});
    }
    localStorage.setItem(KEYBINDINGS_KEY, JSON.stringify({ version: 1, overrides: { a: 'F1', b: 'junk+', c: 7 } }));
    expect(loadOverrides()).toEqual({ a: 'F1' });
  });

  it('survives storage that throws or is missing', () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('denied');
    });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('quota');
    });
    expect(loadOverrides()).toEqual({});
    expect(() => saveOverrides({ a: 'F1' })).not.toThrow();
    expect(loadOverrides(null)).toEqual({});
    expect(() => saveOverrides({ a: 'F1' }, null)).not.toThrow();
  });
});

describe('KeybindingOverridesStore', () => {
  beforeEach(() => localStorage.clear());

  it('starts from the stored overrides, persists changes and resets', () => {
    localStorage.setItem(KEYBINDINGS_KEY, JSON.stringify({ version: 1, overrides: { a: 'F2' } }));
    const store = TestBed.inject(KeybindingOverridesStore);
    expect(store.overrides()).toEqual({ a: 'F2' });
    store.set({ b: 'mod+j', c: 'junk+' });
    expect(store.overrides()).toEqual({ b: 'Mod+J' });
    expect(loadOverrides()).toEqual({ b: 'Mod+J' });
    store.resetAll();
    expect(store.overrides()).toEqual({});
    expect(localStorage.getItem(KEYBINDINGS_KEY)).toBeNull();
  });

  it('follows another window', () => {
    const store = TestBed.inject(KeybindingOverridesStore);
    localStorage.setItem(KEYBINDINGS_KEY, JSON.stringify({ version: 1, overrides: { z: 'F9' } }));
    window.dispatchEvent(new StorageEvent('storage', { key: KEYBINDINGS_KEY }));
    expect(store.overrides()).toEqual({ z: 'F9' });
    window.dispatchEvent(new StorageEvent('storage', { key: 'other' }));
    expect(store.overrides()).toEqual({ z: 'F9' });
    localStorage.clear();
    window.dispatchEvent(new StorageEvent('storage', { key: null }));
    expect(store.overrides()).toEqual({});
  });
});
