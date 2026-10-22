import { patchState, signalStore, withMethods, withState } from '@ngrx/signals';
import { TimeZoneMode } from '../format';
import { isThemeId } from '../../theme/themes';
import { SCHEME_MODES, SchemeMode, ThemeId, UI_FONTS, UiFont } from '../../theme/theme.types';

export type { SchemeMode, ThemeId, UiFont } from '../../theme/theme.types';

interface UiPrefsState {
  timezone: TimeZoneMode;
  alwaysShowLifecycle: boolean;
  /** The look (ADR 0018): token set and traits. Null follows the system (High contrast when it asks for more contrast). */
  theme: ThemeId | null;
  /** Light, dark, or follow the operating system. */
  scheme: SchemeMode;
  /** The interface font: the operating system's (native feel, ADR 0028) or the theme's. */
  uiFont: UiFont;
  /** Grid sort order (not persisted). */
  descending: boolean;
  extraColumns: string[];
}

const PREFS_KEY = 'debrief.prefs.v1';
/** The preferences kept across sessions. */
export type Prefs = Pick<UiPrefsState, 'timezone' | 'theme' | 'scheme' | 'uiFont' | 'extraColumns' | 'alwaysShowLifecycle'>;

const DEFAULTS: Prefs = { timezone: 'utc', alwaysShowLifecycle: true, theme: null, scheme: 'system', uiFont: 'system', extraColumns: [] };

/** Narrows stored (untrusted) preferences: unknown or malformed values fall back to the defaults. */
export function sanitizePrefs(raw: unknown): Prefs {
  const r = typeof raw === 'object' && raw !== null && !Array.isArray(raw) ? (raw as Record<string, unknown>) : {};
  return {
    timezone: r['timezone'] === 'local' || r['timezone'] === 'utc' ? r['timezone'] : DEFAULTS.timezone,
    alwaysShowLifecycle: typeof r['alwaysShowLifecycle'] === 'boolean' ? r['alwaysShowLifecycle'] : DEFAULTS.alwaysShowLifecycle,
    theme: isThemeId(r['theme']) ? r['theme'] : null,
    scheme: isSchemeMode(r['scheme']) ? r['scheme'] : DEFAULTS.scheme,
    uiFont: isUiFont(r['uiFont']) ? r['uiFont'] : DEFAULTS.uiFont,
    extraColumns: Array.isArray(r['extraColumns']) ? r['extraColumns'].filter((c): c is string => typeof c === 'string') : DEFAULTS.extraColumns,
  };
}

/** Narrows untrusted input to an interface font choice. */
export function isUiFont(v: unknown): v is UiFont {
  return (UI_FONTS as readonly unknown[]).includes(v);
}

/** Narrows untrusted input to a scheme choice. */
export function isSchemeMode(v: unknown): v is SchemeMode {
  return (SCHEME_MODES as readonly unknown[]).includes(v);
}

export function loadPrefs(): Prefs {
  try {
    return sanitizePrefs(JSON.parse(localStorage.getItem(PREFS_KEY) ?? '{}'));
  } catch {
    return sanitizePrefs({});
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
 * View preferences: time zone, theme and colour scheme, lane colouring, extra grid columns, lifecycle markers and the
 * grid's sort order. Saving them is not this store's job (`providePrefsPersistence` does it), and applying the theme is
 * `ThemeService`'s, so it has no side effects of its own.
 */
export const UiPrefsStore = signalStore(
  { providedIn: 'root' },
  withState<UiPrefsState>(() => ({ ...loadPrefs(), descending: false })),
  withMethods((store) => ({
    setTimezone: (timezone: TimeZoneMode) => patchState(store, { timezone }),
    setAlwaysShowLifecycle: (alwaysShowLifecycle: boolean) => patchState(store, { alwaysShowLifecycle }),
    setTheme: (theme: ThemeId | null) => patchState(store, { theme }),
    setScheme: (scheme: SchemeMode) => patchState(store, { scheme }),
    setUiFont: (uiFont: UiFont) => patchState(store, { uiFont }),
    toggleSort: () => patchState(store, (s) => ({ descending: !s.descending })),
    addColumn(field: string) {
      if (!store.extraColumns().includes(field)) patchState(store, { extraColumns: [...store.extraColumns(), field] });
    },
    removeColumn: (field: string) => patchState(store, { extraColumns: store.extraColumns().filter((c) => c !== field) }),
    /** The preferences to persist. */
    snapshot: (): Prefs => ({
      timezone: store.timezone(),
      theme: store.theme(),
      scheme: store.scheme(),
      uiFont: store.uiFont(),
      extraColumns: store.extraColumns(),
      alwaysShowLifecycle: store.alwaysShowLifecycle(),
    }),
  })),
);
