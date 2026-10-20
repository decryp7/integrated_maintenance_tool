import { CALM } from './themes/calm.theme';
import { CONSOLE } from './themes/console.theme';
import { THEME_IDS, ThemeDef, ThemeId } from './theme.types';

/** The default theme. */
export const DEFAULT_THEME_ID: ThemeId = 'calm';

/** The built-in themes, in menu order. */
export const BUILT_IN_THEMES: readonly ThemeDef[] = [CALM, CONSOLE];

/** Narrows untrusted input (stored preferences, another window) to a theme id. */
export function isThemeId(v: unknown): v is ThemeId {
  return (THEME_IDS as readonly unknown[]).includes(v);
}

/** The theme with `id`, or the default one for an unknown id. */
export function themeById(id: string): ThemeDef {
  return BUILT_IN_THEMES.find((t) => t.id === id) ?? CALM;
}
