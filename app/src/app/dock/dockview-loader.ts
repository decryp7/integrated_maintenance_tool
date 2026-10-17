/**
 * dockview is loaded on demand: only a window that shows the dock needs it (a pop-out panel window does not, which
 * keeps about 390 KB of script out of every pop-out).
 */
type Dockview = typeof import('dockview');

let loaded: Dockview | null = null;
let loading: Promise<Dockview> | null = null;

export function loadDockview(): Promise<Dockview> {
  loading ??= import('dockview').then((m) => (loaded = m));
  return loading;
}

/** The module once `loadDockview` resolved, else null. */
export function dockview(): Dockview | null {
  return loaded;
}
