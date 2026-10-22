import { Density, Scheme, ThemeDef, ThemeTraits, TokenSet, TRAIT_VALUES, TraitName, UiFont } from './theme.types';

/** Tokens every theme defines once for both schemes. */
export const COMMON_TOKENS = [
  'font-sans',
  'font-mono',
  'fs-2xs',
  'fs-xs',
  'fs-sm',
  'fs-md',
  'fs-lg',
  'fs-xl',
  'fs-hero',
  's-1',
  's-2',
  's-3',
  's-4',
  's-5',
  's-6',
  'r-1',
  'r-2',
  'r-3',
  'dur-fast',
  'dur-med',
  'dur-slow',
  'ease',
  'title-h',
  'status-h',
  'tab-h',
  'tl-label-w',
  'hist-h',
] as const;

/** Tokens every theme defines per density. */
export const DENSITY_TOKENS = ['row-h', 'lane-h', 'run-row-h'] as const;

/** Colour roles every theme defines per scheme. */
export const COLOR_TOKENS = [
  // Surfaces and structure
  'bg-0',
  'bg-1',
  'bg-2',
  'bg-3',
  'bg-hover',
  'bg-active',
  'bg-sel',
  'border',
  'border-2',
  'border-3',
  'grid-line',
  'row-line',
  // Text
  'text',
  'text-2',
  'muted',
  'faint',
  'accent',
  'accent-ink',
  'accent-text',
  'accent-soft',
  'focus-ring',
  'ok',
  'warn-text',
  'danger-text',
  'fatal-text',
  'chip-bg',
  'chip-border',
  'chip-text',
  // Chrome
  'titlebar-bg',
  'activity-bg',
  'tabs-bg',
  'statusbar-bg',
  'statusbar-text',
  'statusbar-strong',
  'tooltip-bg',
  'code-bg',
  'readout-bg',
  'readout-text',
  'window-close-bg',
  'window-close-text',
  'scrim',
  'shadow',
  'shadow-1',
  'shadow-pop',
  'shadow-drawer',
  // Levels and markers
  'lvl-verbose',
  'lvl-debug',
  'lvl-info',
  'lvl-warn',
  'lvl-error',
  'lvl-fatal',
  'row-hl-tint',
  'mk-start',
  'mk-stop',
  'mk-fatal',
  'mk-exception',
  'mk-error',
  'mk-warning',
  // Timeline
  'lane-bar',
  'lane-border',
  'density',
  'hist-info',
  'ring',
  'brush-fill',
  'brush-edge',
  'crosshair',
  'silent-hatch',
  'ann-critical',
  'ann-critical-border',
  'ann-warning',
  'ann-warning-border',
  'ann-info',
  'ann-info-border',
  // Categorical palettes: chart series and sources
  's1',
  's2',
  's3',
  's4',
  's-other',
  // Compare view: the A and B sides (an Okabe-Ito blue and orange, separable under every colour-vision deficiency)
  'cmp-a',
  'cmp-b',
  'src-1',
  'src-2',
  'src-3',
  'src-4',
] as const;

/** Every token a complete theme sets for one scheme and density. */
export const ALL_TOKENS: readonly string[] = [...COMMON_TOKENS, ...DENSITY_TOKENS, ...COLOR_TOKENS];

/** The tokens in effect for a scheme and density: common, then colours, then sizes (later sets win). */
export function resolveTokens(theme: ThemeDef, scheme: Scheme, density: Density): TokenSet {
  return { ...theme.common, ...theme.schemes[scheme], ...theme.densities[density] };
}

/** The names `theme` is missing for any scheme or density (empty for a complete theme). */
export function missingTokens(theme: ThemeDef): string[] {
  const out: string[] = [];
  const check = (set: TokenSet, names: readonly string[], where: string) => {
    for (const n of names) if (!set[n]) out.push(`${where}: --${n}`);
  };
  check(theme.common, COMMON_TOKENS, 'common');
  for (const [scheme, set] of Object.entries(theme.schemes)) check(set, COLOR_TOKENS, scheme);
  for (const [density, set] of Object.entries(theme.densities)) check(set, DENSITY_TOKENS, density);
  return out;
}

/**
 * The operating system's UI font (ADR 0028): Segoe UI Variable / Segoe UI on Windows, San Francisco on macOS, and the
 * desktop's font on Linux (`system-ui`, then GNOME's Cantarell, Ubuntu, Noto Sans). Only installed faces match, so the
 * order picks the right one on each system.
 */
export const SYSTEM_FONT_SANS =
  "'Segoe UI Variable Text', 'Segoe UI Variable', 'Segoe UI', -apple-system, BlinkMacSystemFont, system-ui, Cantarell, Ubuntu, 'Noto Sans', 'Liberation Sans', sans-serif";

/** The tokens with the interface font applied: `system` swaps the theme's sans face for the system one. */
export function withUiFont(tokens: TokenSet, font: UiFont): TokenSet {
  return font === 'system' ? { ...tokens, 'font-sans': SYSTEM_FONT_SANS } : tokens;
}

/** User choices that override a theme's traits (null keeps the theme's own). */
export interface TraitOverrides {
  /** "Colour lanes by source": on forces `laneFill: bySource`, off forces `neutral`. */
  lanesBySource: boolean | null;
}

/** The theme's traits with the user's overrides applied. */
export function effectiveTraits(theme: ThemeDef, o: TraitOverrides): ThemeTraits {
  if (o.lanesBySource === null) return theme.traits;
  return { ...theme.traits, laneFill: o.lanesBySource ? 'bySource' : 'neutral' };
}

/** The `data-*` attribute name of a trait (`laneFill` → `data-lane-fill`). */
export function traitAttribute(name: TraitName): string {
  return 'data-' + name.replace(/[A-Z]/g, (c) => '-' + c.toLowerCase());
}

/** Every trait as its root attribute, in a stable order. */
export function traitAttributes(traits: ThemeTraits): [string, string][] {
  return (Object.keys(TRAIT_VALUES) as TraitName[]).map((name) => [traitAttribute(name), traits[name]]);
}

/** A `12px` token as a number (0 when it is not a pixel length). */
export function px(value: string | undefined): number {
  const m = /^\s*(-?\d+(?:\.\d+)?)px\s*$/.exec(value ?? '');
  return m?.[1] ? Number(m[1]) : 0;
}
