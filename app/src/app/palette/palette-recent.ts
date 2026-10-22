import { Injectable, signal } from '@angular/core';

const KEY = 'imt.palette.recent.v1';
/** How many recently run commands the empty palette lists first. */
export const MAX_RECENT = 8;

/**
 * Commands most often reached for when the palette opens empty, after the recently used ones (a first run has none).
 * Ids that are not registered or not available are skipped.
 */
export const COMMON_COMMANDS: readonly string[] = [
  'file.openFolder',
  'file.openFiles',
  'analyze.search',
  'query.focus',
  'anomalies.show',
  'moment.analyse',
  'project.save',
  'settings.open',
];

/** The commands last run from the palette, newest first; kept in this browser's storage (a convenience: it may be lost). */
@Injectable({ providedIn: 'root' })
export class PaletteRecent {
  readonly ids = signal<readonly string[]>(read());

  add(id: string): void {
    const next = [id, ...this.ids().filter((x) => x !== id)].slice(0, MAX_RECENT);
    this.ids.set(next);
    try {
      localStorage.setItem(KEY, JSON.stringify(next));
    } catch {
      /* storage unavailable: the list lasts for this session */
    }
  }
}

function read(): readonly string[] {
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(KEY) ?? '[]');
    return Array.isArray(parsed) ? parsed.filter((x): x is string => typeof x === 'string').slice(0, MAX_RECENT) : [];
  } catch {
    return [];
  }
}
