import { DestroyRef, inject, Injectable, signal } from '@angular/core';
import { KeybindingOverrides, sanitizeOverrides } from './keybinding-overrides';

/** Where the user's key bindings live in the browser (UI preference, like the theme in `imt.prefs.v1`). */
export const KEYBINDINGS_KEY = 'imt.keybindings';
/** Schema version of the stored value; a different version is ignored (defaults apply). */
export const KEYBINDINGS_VERSION = 1;

/** Reads the stored overrides; anything missing, corrupt or of another version yields no overrides. Never throws. */
export function loadOverrides(storage: Pick<Storage, 'getItem'> | null = safeStorage()): KeybindingOverrides {
  try {
    const parsed = JSON.parse(storage?.getItem(KEYBINDINGS_KEY) ?? 'null') as { version?: unknown; overrides?: unknown } | null;
    return parsed?.version === KEYBINDINGS_VERSION ? sanitizeOverrides(parsed.overrides) : {};
  } catch {
    return {};
  }
}

/** Stores the overrides; a storage that is unavailable or full only costs the persistence. */
export function saveOverrides(overrides: KeybindingOverrides, storage: Pick<Storage, 'setItem' | 'removeItem'> | null = safeStorage()): void {
  try {
    if (Object.keys(overrides).length === 0) storage?.removeItem(KEYBINDINGS_KEY);
    else storage?.setItem(KEYBINDINGS_KEY, JSON.stringify({ version: KEYBINDINGS_VERSION, overrides }));
  } catch {
    /* storage unavailable: the bindings last for this session only */
  }
}

function safeStorage(): Storage | null {
  try {
    return localStorage;
  } catch {
    return null;
  }
}

/**
 * The user's key binding overrides: a versioned, persisted map of command id to binding (or to "no key"). The
 * command registry applies it, so menus, the palette, tooltips and the cheat sheet all show the effective keys.
 * Follows changes made in another window (the `storage` event).
 */
@Injectable({ providedIn: 'root' })
export class KeybindingOverridesStore {
  private readonly state = signal<KeybindingOverrides>(loadOverrides());
  readonly overrides = this.state.asReadonly();

  constructor() {
    if (typeof window === 'undefined') return;
    const onStorage = (e: StorageEvent) => {
      if (e.key === KEYBINDINGS_KEY || e.key === null) this.state.set(loadOverrides());
    };
    window.addEventListener('storage', onStorage);
    inject(DestroyRef).onDestroy(() => window.removeEventListener('storage', onStorage));
  }

  /** Replaces all overrides (sanitised) and persists them. */
  set(overrides: KeybindingOverrides): void {
    const clean = sanitizeOverrides(overrides);
    this.state.set(clean);
    saveOverrides(clean);
  }

  /** Back to the declared bindings. */
  resetAll(): void {
    this.set({});
  }
}
