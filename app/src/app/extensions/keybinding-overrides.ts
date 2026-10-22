import { isMac } from './keybinding';
import { DebriefCommand } from './tokens';

/**
 * Pure helpers of user key bindings: canonical form, recording from a key event, conflict detection and merging
 * overrides over the bindings commands declare. No framework, no storage (see `KeybindingOverridesStore`).
 *
 * An override maps a command id to a binding, or to `null` for "no key". Commands without an override keep the
 * binding they declare. Bindings use the syntax of `matchesKeybinding`: `Mod+Shift+P`, `Alt+Left`, `F6`.
 */

/** Overrides by command id; `null` = the user removed the binding. */
export type KeybindingOverrides = Readonly<Record<string, string | null>>;

/** Most overrides kept (the command list is far smaller; this bounds corrupt or hostile storage). */
export const MAX_OVERRIDES = 1000;
const MAX_ID_LENGTH = 200;

const MODIFIERS = ['Mod', 'Ctrl', 'Alt', 'Shift', 'Meta'] as const;

const NAMED_KEYS: Readonly<Record<string, string>> = {
  left: 'Left',
  arrowleft: 'Left',
  right: 'Right',
  arrowright: 'Right',
  up: 'Up',
  arrowup: 'Up',
  down: 'Down',
  arrowdown: 'Down',
  space: 'Space',
  ' ': 'Space',
  enter: 'Enter',
  tab: 'Tab',
  escape: 'Escape',
  esc: 'Escape',
  home: 'Home',
  end: 'End',
  pageup: 'PageUp',
  pagedown: 'PageDown',
  delete: 'Delete',
  del: 'Delete',
  insert: 'Insert',
  backspace: 'Backspace',
};

/** Keys that never make a binding on their own. */
const MODIFIER_KEYS = new Set(['control', 'shift', 'alt', 'meta', 'altgraph', 'os', 'capslock', 'numlock', 'scrolllock', 'fn', 'dead', 'process', 'unidentified', 'hyper', 'super']);

/** The canonical spelling of a key name, or null when a binding cannot use it. */
function canonicalKey(key: string): string | null {
  const lower = key.toLowerCase();
  const named = NAMED_KEYS[lower];
  if (named) return named;
  if (/^f([1-9]|1\d|2[0-4])$/.test(lower)) return lower.toUpperCase();
  return [...key].length === 1 && key !== '+' && key.trim() !== '' ? key.toUpperCase() : null;
}

/**
 * The canonical form of a binding (`mod+shift+p` becomes `Mod+Shift+P`: modifiers in a fixed order, named keys spelled
 * one way), or null when it is not a valid binding.
 */
export function normalizeBinding(binding: string): string | null {
  if (typeof binding !== 'string' || binding.length === 0 || binding.length > 40) return null;
  const parts = binding.split('+').map((p) => p.trim());
  const keyPart = parts.pop();
  if (!keyPart) return null;
  const key = canonicalKey(keyPart);
  if (!key) return null;
  const mods = new Set<string>();
  for (const p of parts) {
    const m = MODIFIERS.find((x) => x.toLowerCase() === p.toLowerCase());
    if (!m || mods.has(m)) return null;
    mods.add(m);
  }
  return [...MODIFIERS.filter((m) => mods.has(m)), key].join('+');
}

/**
 * What decides whether two bindings fire on the same key press: `Mod` is resolved to the platform's modifier, so
 * `Mod+K` and `Ctrl+K` clash on Windows and Linux but not on macOS. Null for an invalid binding.
 */
export function conflictKey(binding: string, mac = isMac()): string | null {
  const canon = normalizeBinding(binding);
  if (!canon) return null;
  const parts = canon.split('+');
  const key = parts.pop() ?? '';
  const mods = new Set(parts.map((m) => (m === 'Mod' ? (mac ? 'Meta' : 'Ctrl') : m)));
  return [...MODIFIERS.filter((m) => mods.has(m)), key.toLowerCase()].join('+');
}

/** Do two bindings fire on the same key press? */
export function sameBinding(a: string | null | undefined, b: string | null | undefined, mac = isMac()): boolean {
  if (!a || !b) return false;
  const ka = conflictKey(a, mac);
  return ka !== null && ka === conflictKey(b, mac);
}

/** Result of reading a key press while the user records a binding. */
export type Recorded =
  | { kind: 'binding'; binding: string }
  | { kind: 'cancel' }
  | { kind: 'clear' }
  /** Only modifiers so far: keep waiting. */
  | { kind: 'wait' }
  /** A key that cannot be bound alone (a letter without Ctrl/Alt, Tab ...): tell the user. */
  | { kind: 'invalid' };

/** The part of a keyboard event the recorder reads. */
export type KeyInput = Pick<KeyboardEvent, 'key' | 'code' | 'ctrlKey' | 'metaKey' | 'altKey' | 'shiftKey'>;

/**
 * Reads a key press as a binding. Escape cancels and Backspace or Delete (alone) clear the binding, as in other
 * editors; bare keys other than F1-F24 are refused because they would fire while the user types.
 */
export function recordKey(e: KeyInput, mac = isMac()): Recorded {
  const name = e.key.toLowerCase();
  if (MODIFIER_KEYS.has(name)) return { kind: 'wait' };
  const bare = !e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey;
  if (bare && name === 'escape') return { kind: 'cancel' };
  if (bare && (name === 'backspace' || name === 'delete')) return { kind: 'clear' };
  // Letters come from the key itself; the physical key only helps where Alt/Option turns them into symbols.
  const letter = /^Key([A-Z])$/.exec(e.code ?? '');
  const raw = !/^[a-z]$/i.test(e.key) && letter?.[1] && e.altKey ? letter[1] : e.key;
  const key = canonicalKey(raw);
  if (!key) return { kind: 'invalid' };
  const primary = mac ? e.metaKey : e.ctrlKey;
  const secondary = mac ? e.ctrlKey : e.metaKey;
  const mods = [primary ? 'Mod' : '', secondary ? (mac ? 'Ctrl' : 'Meta') : '', e.altKey ? 'Alt' : '', e.shiftKey ? 'Shift' : ''].filter(Boolean);
  const isFunctionKey = /^F\d+$/.test(key);
  if (!isFunctionKey && !primary && !secondary && !e.altKey) return { kind: 'invalid' };
  const binding = normalizeBinding([...mods, key].join('+'));
  return binding ? { kind: 'binding', binding } : { kind: 'invalid' };
}

/** What already uses a binding. */
export type Occupant = { kind: 'command'; id: string; title: string } | { kind: 'fixed'; description: string };

/** A key the app handles outside the command registry. */
export interface FixedBinding {
  binding?: string;
  description: string;
}

/** Everything that already fires on `binding`: other commands (by their effective binding) and fixed keys. */
export function findConflicts(
  binding: string,
  forId: string,
  commands: readonly Pick<DebriefCommand, 'id' | 'title' | 'keybinding'>[],
  fixed: readonly FixedBinding[],
  mac = isMac(),
): Occupant[] {
  const out: Occupant[] = [];
  for (const c of commands) if (c.id !== forId && sameBinding(c.keybinding, binding, mac)) out.push({ kind: 'command', id: c.id, title: c.title });
  for (const f of fixed) if (sameBinding(f.binding, binding, mac)) out.push({ kind: 'fixed', description: f.description });
  return out;
}

/**
 * `overrides` with `id` bound to `binding` (`null` = no key). An override equal to the declared default is dropped,
 * so setting the default by hand leaves no trace.
 */
export function withOverride(overrides: KeybindingOverrides, id: string, binding: string | null, declared: string | undefined): KeybindingOverrides {
  const rest = withoutOverride(overrides, id);
  const canon = binding === null ? null : (normalizeBinding(binding) ?? binding);
  const isDefault = canon === null ? !declared : canon === (declared ? normalizeBinding(declared) : null);
  return isDefault ? rest : { ...rest, [id]: canon };
}

/** `overrides` without `id`. */
export function withoutOverride(overrides: KeybindingOverrides, id: string): KeybindingOverrides {
  const rest = { ...overrides };
  delete rest[id];
  return rest;
}

/** The commands with the user's bindings applied: an override replaces the declared binding, `null` removes it. */
export function applyOverrides<T extends Pick<DebriefCommand, 'id' | 'keybinding'>>(commands: readonly T[], overrides: KeybindingOverrides): T[] {
  return commands.map((c) => {
    if (!Object.hasOwn(overrides, c.id)) return c;
    const o = overrides[c.id];
    if (o === null || o === undefined) {
      const rest = { ...c };
      delete rest.keybinding;
      return rest;
    }
    return { ...c, keybinding: o };
  });
}

/** Narrows stored (untrusted) overrides: invalid ids and bindings are dropped, the count is capped. */
export function sanitizeOverrides(raw: unknown): KeybindingOverrides {
  if (typeof raw !== 'object' || raw === null || Array.isArray(raw)) return {};
  const out: Record<string, string | null> = {};
  for (const [id, v] of Object.entries(raw)) {
    if (Object.keys(out).length >= MAX_OVERRIDES) break;
    if (!id || id.length > MAX_ID_LENGTH || id === '__proto__') continue;
    if (v === null) out[id] = null;
    else if (typeof v === 'string') {
      const b = normalizeBinding(v);
      if (b) out[id] = b;
    }
  }
  return out;
}
