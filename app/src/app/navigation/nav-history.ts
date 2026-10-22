/** A place the user can go back to: an event they looked at, a time range they zoomed to, a view they brought forward. */
export type NavPlace = { kind: 'event'; id: number } | { kind: 'range'; from: number | null; to: number | null } | { kind: 'view'; id: string };

/** Places kept; the oldest are dropped beyond this. */
export const NAV_LIMIT = 50;
/** Range changes closer together than this (ms) are one place: a zoom or pan is a stream of small steps. */
export const NAV_MERGE_MS = 1200;

export function samePlace(a: NavPlace, b: NavPlace): boolean {
  if (a.kind !== b.kind) return false;
  if (a.kind === 'event') return a.id === (b as typeof a).id;
  if (a.kind === 'range') return a.from === (b as typeof a).from && a.to === (b as typeof a).to;
  return a.id === (b as typeof a).id;
}

/**
 * Back/forward through places, like a browser: recording a new place drops the places ahead of the current one, an
 * unchanged place is ignored, and range places recorded in quick succession replace each other. Pure: the clock is
 * passed in.
 */
export class NavHistory {
  private places: NavPlace[] = [];
  private index = -1;
  private lastAt = 0;

  get canBack(): boolean {
    return this.index > 0;
  }

  get canForward(): boolean {
    return this.index >= 0 && this.index < this.places.length - 1;
  }

  get size(): number {
    return this.places.length;
  }

  get current(): NavPlace | null {
    return this.places[this.index] ?? null;
  }

  /** Records `place` as the new current one. Returns false when it was already the current place. */
  record(place: NavPlace, now: number): boolean {
    const cur = this.current;
    if (cur && samePlace(cur, place)) return false;
    if (cur?.kind === 'range' && place.kind === 'range' && now - this.lastAt <= NAV_MERGE_MS && this.index === this.places.length - 1) {
      this.places[this.index] = place;
    } else {
      this.places = [...this.places.slice(0, this.index + 1), place].slice(-NAV_LIMIT);
      this.index = this.places.length - 1;
    }
    this.lastAt = now;
    return true;
  }

  /** The index of the nearest place in direction `dir` that `skip` does not reject, or -1 when there is none. */
  private target(dir: -1 | 1, skip?: (place: NavPlace) => boolean): number {
    for (let i = this.index + dir; i >= 0 && i < this.places.length; i += dir) {
      const place = this.places[i];
      if (place && !skip?.(place)) return i;
    }
    return -1;
  }

  /** Whether `back(skip)` / `forward(skip)` would move: false when every place that way is rejected. */
  canGo(dir: -1 | 1, skip?: (place: NavPlace) => boolean): boolean {
    return this.target(dir, skip) >= 0;
  }

  /**
   * Moves back to the nearest place `skip` does not reject and returns it (null when there is none and nothing moves).
   * `skip` lets the caller pass places that would change nothing visible, such as the view that is already in front.
   */
  back(skip?: (place: NavPlace) => boolean): NavPlace | null {
    return this.step(-1, skip);
  }

  forward(skip?: (place: NavPlace) => boolean): NavPlace | null {
    return this.step(1, skip);
  }

  private step(dir: -1 | 1, skip?: (place: NavPlace) => boolean): NavPlace | null {
    const i = this.target(dir, skip);
    if (i < 0) return null;
    this.index = i;
    return this.current;
  }

  clear(): void {
    this.places = [];
    this.index = -1;
  }
}
