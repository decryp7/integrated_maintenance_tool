import { patchState, signalStore, withMethods, withState } from '@ngrx/signals';
import { TimeZoneMode } from '../format';

export type ThemeMode = 'system' | 'dark' | 'light';

interface UiPrefsState {
  timezone: TimeZoneMode;
  alwaysShowLifecycle: boolean;
  theme: ThemeMode;
  /** Grid sort order (not persisted). */
  descending: boolean;
  extraColumns: string[];
}

const PREFS_KEY = 'imt.prefs.v1';
/** The preferences kept across sessions. */
export type Prefs = Pick<UiPrefsState, 'timezone' | 'theme' | 'extraColumns' | 'alwaysShowLifecycle'>;

export function loadPrefs(): Partial<Prefs> {
  try {
    return JSON.parse(localStorage.getItem(PREFS_KEY) ?? '{}') as Partial<Prefs>;
  } catch {
    return {};
  }
}

export function savePrefs(p: Prefs): void {
  try {
    localStorage.setItem(PREFS_KEY, JSON.stringify(p));
  } catch {
    /* storage unavailable: preferences are a convenience only */
  }
}

/**
 * View preferences: time zone, theme, extra grid columns, lifecycle markers and the grid's sort order. Saving
 * them is not this store's job (`providePrefsPersistence` does it), so it has no side effects of its own.
 */
export const UiPrefsStore = signalStore(
  { providedIn: 'root' },
  withState<UiPrefsState>(() => ({ timezone: 'utc', alwaysShowLifecycle: true, theme: 'system', descending: false, extraColumns: [], ...loadPrefs() })),
  withMethods((store) => ({
    setTimezone: (timezone: TimeZoneMode) => patchState(store, { timezone }),
    setAlwaysShowLifecycle: (alwaysShowLifecycle: boolean) => patchState(store, { alwaysShowLifecycle }),
    setTheme: (theme: ThemeMode) => patchState(store, { theme }),
    toggleSort: () => patchState(store, (s) => ({ descending: !s.descending })),
    addColumn(field: string) {
      if (!store.extraColumns().includes(field)) patchState(store, { extraColumns: [...store.extraColumns(), field] });
    },
    removeColumn: (field: string) => patchState(store, { extraColumns: store.extraColumns().filter((c) => c !== field) }),
    /** The preferences to persist. */
    snapshot: (): Prefs => ({
      timezone: store.timezone(),
      theme: store.theme(),
      extraColumns: store.extraColumns(),
      alwaysShowLifecycle: store.alwaysShowLifecycle(),
    }),
  })),
);
