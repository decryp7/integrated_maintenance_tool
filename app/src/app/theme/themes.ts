import { CALM } from './themes/calm.theme';
import { CONSOLE } from './themes/console.theme';
import { CONTRAST } from './themes/contrast.theme';
import { PRECISION } from './themes/precision.theme';
import { THEME_IDS, ThemeDef, ThemeId } from './theme.types';

/** The default theme. */
export const DEFAULT_THEME_ID: ThemeId = 'calm';

/** The built-in themes, in menu order. */
export const BUILT_IN_THEMES: readonly ThemeDef[] = [CALM, CONSOLE, PRECISION, CONTRAST];

/** Narrows untrusted input (stored preferences, another window) to a theme id. */
export function isThemeId(v: unknown): v is ThemeId {
  return (THEME_IDS as readonly unknown[]).includes(v);
}

/**
 * The theme in effect for a stored preference: the chosen one, or (null = follow the system) High contrast when the
 * operating system asks for more contrast, the default theme otherwise.
 */
export function resolveThemeId(pref: ThemeId | null, systemWantsContrast: boolean): ThemeId {
  return pref ?? (systemWantsContrast ? 'contrast' : DEFAULT_THEME_ID);
}

/** The theme with `id`, or the default one for an unknown id. */
export function themeById(id: string): ThemeDef {
  return BUILT_IN_THEMES.find((t) => t.id === id) ?? CALM;
}
