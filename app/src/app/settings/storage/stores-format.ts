import { MAX_AUTO_CLEAN_DAYS, StoreFolder, StoreFolderState } from './storage.types';

/** The state's label in the UI language. */
export function stateLabel(state: StoreFolderState): string {
  switch (state) {
    case 'open':
      return $localize`:Store state@@settings.storage.stores.state.open:Open`;
    case 'session':
      return $localize`:Store state@@settings.storage.stores.state.session:Kept for session restore`;
    case 'running':
      return $localize`:Store state@@settings.storage.stores.state.running:In use by another Debrief`;
    case 'orphaned':
      return $localize`:Store state@@settings.storage.stores.state.orphaned:Orphaned`;
  }
}

/** Why a store that is not orphaned cannot be selected (a tooltip); null for an orphaned one. */
export function notSelectableReason(state: StoreFolderState): string | null {
  switch (state) {
    case 'open':
      return $localize`:@@settings.storage.stores.why.open:A window has this store open.`;
    case 'session':
      return $localize`:@@settings.storage.stores.why.session:An autosaved session reopens this store; it is kept so the session can be restored.`;
    case 'running':
      return $localize`:@@settings.storage.stores.why.running:Another Debrief is using this store.`;
    case 'orphaned':
      return null;
  }
}

/** A store folder's name short enough for a table cell: the start and end of a long one. Pure. */
export function shortStoreName(id: string, max = 20): string {
  if (id.length <= max) return id;
  const tail = Math.floor((max - 1) / 3);
  return `${id.slice(0, max - 1 - tail)}…${id.slice(id.length - tail)}`;
}

/** The orphaned stores among `folders` (the only ones that can be cleaned up). */
export function orphaned(folders: readonly StoreFolder[]): StoreFolder[] {
  return folders.filter((f) => f.state === 'orphaned');
}

/** `ids` that are still orphaned in `folders`, in the order of `ids` (a selection that survives a refresh). */
export function stillOrphaned(ids: readonly string[], folders: readonly StoreFolder[]): string[] {
  const ok = new Set(orphaned(folders).map((f) => f.id));
  return ids.filter((id) => ok.has(id));
}

/** The days an input holds: a whole number from 0 to 365, or the message to show. The engine checks again. */
export function parseAutoCleanDays(raw: string): { days: number } | { error: string } {
  const text = raw.trim();
  if (!/^\d+$/.test(text)) return { error: $localize`:@@settings.storage.stores.policy.notNumber:Enter a whole number of days, from 0 to ${MAX_AUTO_CLEAN_DAYS}:max:.` };
  const days = Number(text);
  if (days > MAX_AUTO_CLEAN_DAYS) return { error: $localize`:@@settings.storage.stores.policy.tooMany:At most ${MAX_AUTO_CLEAN_DAYS}:max: days.` };
  return { days };
}
