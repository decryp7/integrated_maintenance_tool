import { InjectionToken, signal } from '@angular/core';
import { readLayout, SavedLayout, toEnvelope } from './layout-store';

/** A window layout the user saved by name (VS "Save Window Layout"), separate from project layouts. */
export interface NamedLayout {
  name: string;
  /** Epoch ms. */
  savedAt: number;
  saved: SavedLayout;
}

export const MAX_NAMED_LAYOUTS = 20;
export const MAX_LAYOUT_NAME = 60;
export const NAMED_LAYOUTS_KEY = 'imt.windowLayouts';
const VERSION = 1;

/** A layout name as typed: one line, trimmed, at most 60 characters; null when nothing is left. */
export function cleanLayoutName(name: string): string | null {
  // eslint-disable-next-line no-control-regex
  const n = name.replace(/[\u0000-\u001f\u007f]/g, ' ').trim().slice(0, MAX_LAYOUT_NAME).trim();
  return n || null;
}

/** Adds or replaces (same name, case-insensitive) a layout; the oldest go past the limit. */
export function upsertLayout(list: readonly NamedLayout[], layout: NamedLayout): NamedLayout[] {
  const key = layout.name.toLowerCase();
  const i = list.findIndex((l) => l.name.toLowerCase() === key);
  const next = i >= 0 ? list.map((l, j) => (j === i ? layout : l)) : [...list, layout];
  return next.slice(Math.max(0, next.length - MAX_NAMED_LAYOUTS));
}

export function removeLayout(list: readonly NamedLayout[], name: string): NamedLayout[] {
  return list.filter((l) => l.name !== name);
}

/** Renames; refuses (returns the list unchanged) when the new name is empty or taken by another layout. */
export function renameLayout(list: readonly NamedLayout[], from: string, to: string): NamedLayout[] {
  const name = cleanLayoutName(to);
  if (!name || list.some((l) => l.name !== from && l.name.toLowerCase() === name.toLowerCase())) return [...list];
  return list.map((l) => (l.name === from ? { ...l, name } : l));
}

/** Moves a layout up (-1) or down (+1); the first four have Ctrl+Alt+1..4. */
export function moveLayout(list: readonly NamedLayout[], name: string, delta: -1 | 1): NamedLayout[] {
  const i = list.findIndex((l) => l.name === name);
  const j = i + delta;
  if (i < 0 || j < 0 || j >= list.length) return [...list];
  const next = [...list];
  const a = next[i];
  const b = next[j];
  if (a === undefined || b === undefined) return next;
  next[i] = b;
  next[j] = a;
  return next;
}

/** Parses the stored list; malformed entries are dropped. */
export function parseNamedLayouts(raw: unknown): NamedLayout[] {
  const v = raw as { version?: unknown; layouts?: unknown } | null;
  if (!v || v.version !== VERSION || !Array.isArray(v.layouts)) return [];
  const out: NamedLayout[] = [];
  for (const l of v.layouts as unknown[]) {
    const e = l as { name?: unknown; savedAt?: unknown; envelope?: unknown } | null;
    const name = typeof e?.name === 'string' ? cleanLayoutName(e.name) : null;
    const saved = readLayout(e?.envelope);
    if (name && saved && !out.some((o) => o.name.toLowerCase() === name.toLowerCase())) {
      out.push({ name, savedAt: typeof e?.savedAt === 'number' ? e.savedAt : 0, saved });
    }
  }
  return out.slice(0, MAX_NAMED_LAYOUTS);
}

export function serializeNamedLayouts(list: readonly NamedLayout[]): string {
  return JSON.stringify({ version: VERSION, layouts: list.map((l) => ({ name: l.name, savedAt: l.savedAt, envelope: toEnvelope(l.saved) })) });
}

/** Where named layouts are kept. App-wide (every window and project sees the same list); never throws. */
export interface NamedLayoutStore {
  load(): NamedLayout[];
  save(list: readonly NamedLayout[]): void;
}

export class LocalStorageNamedLayoutStore implements NamedLayoutStore {
  constructor(private readonly key = NAMED_LAYOUTS_KEY) {}

  load(): NamedLayout[] {
    try {
      return parseNamedLayouts(JSON.parse(localStorage.getItem(this.key) ?? 'null'));
    } catch {
      return [];
    }
  }

  save(list: readonly NamedLayout[]): void {
    try {
      localStorage.setItem(this.key, serializeNamedLayouts(list));
    } catch {
      /* storage full or unavailable: the layouts stay for this run */
    }
  }
}

export const NAMED_LAYOUT_STORE = new InjectionToken<NamedLayoutStore>('NAMED_LAYOUT_STORE', {
  providedIn: 'root',
  factory: () => new LocalStorageNamedLayoutStore(),
});

/** The named layouts as a signal, persisted through the store. */
export class NamedLayouts {
  readonly list = signal<readonly NamedLayout[]>([]);

  constructor(private readonly store: NamedLayoutStore) {
    this.list.set(store.load());
  }

  private set(list: readonly NamedLayout[]): void {
    this.list.set(list);
    this.store.save(list);
  }

  /** Saves the layout under `name`; returns the cleaned name, or null when the name is empty. */
  save(name: string, saved: SavedLayout, now = Date.now()): string | null {
    const clean = cleanLayoutName(name);
    if (!clean) return null;
    this.set(upsertLayout(this.list(), { name: clean, savedAt: now, saved }));
    return clean;
  }

  remove(name: string): void {
    this.set(removeLayout(this.list(), name));
  }

  rename(from: string, to: string): boolean {
    const next = renameLayout(this.list(), from, to);
    const changed = next.some((l, i) => l.name !== this.list()[i]?.name);
    if (changed) this.set(next);
    return changed;
  }

  move(name: string, delta: -1 | 1): void {
    this.set(moveLayout(this.list(), name, delta));
  }

  find(name: string): NamedLayout | undefined {
    return this.list().find((l) => l.name === name);
  }

  /** Layout number `n` (1-based, Ctrl+Alt+n). */
  at(n: number): NamedLayout | undefined {
    return this.list()[n - 1];
  }
}
