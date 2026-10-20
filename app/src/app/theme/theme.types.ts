/**
 * Themes are data (ADR 0018): an id, a name, a token set per colour scheme and density, and a closed set of traits.
 * Tokens become CSS custom properties on the root element; traits become `data-*` attributes there and a signal for
 * code that draws (timeline, charts). Components read only tokens and traits, never a theme id.
 */

/** The built-in themes. `precision` follows when the engine provides per-lane error counts (ADR 0018). */
export const THEME_IDS = ['calm', 'console'] as const;
export type ThemeId = (typeof THEME_IDS)[number];

/** A resolved colour scheme. */
export const SCHEMES = ['light', 'dark'] as const;
export type Scheme = (typeof SCHEMES)[number];

/** The user's scheme choice: follow the operating system, or a fixed scheme. */
export const SCHEME_MODES = ['system', 'light', 'dark'] as const;
export type SchemeMode = (typeof SCHEME_MODES)[number];

/** Row, lane and padding sizes, orthogonal to the theme. */
export const DENSITIES = ['comfortable', 'compact'] as const;
export type Density = (typeof DENSITIES)[number];

/** The typed switches a theme sets, with every value each may take (first = the fallback). */
export const TRAIT_VALUES = {
  /** Panels flush with 1 px borders, or soft cards. */
  surface: ['flat', 'cards'],
  /** How a run between lifecycle markers is drawn on its sub-track. */
  runs: ['line', 'pill', 'bracket'],
  /** How a lane's volume is drawn: a neutral area, an area in the lane's source colour, or a barcode. */
  laneFill: ['neutral', 'bySource', 'barcode'],
  /** A soft band behind each crash across the histogram and lanes (in addition to the crash rule). */
  crashBand: ['off', 'on'],
  /** A floating pill on the brush with the selected range and its length. */
  brushPill: ['off', 'on'],
  /** Findings as one sentence, or as severity chips. */
  findings: ['text', 'chips'],
  /** Figures in proportional or tabular (fixed-width) digits. */
  numerals: ['proportional', 'tabular'],
} as const;

export type TraitName = keyof typeof TRAIT_VALUES;
export type ThemeTraits = { readonly [K in TraitName]: (typeof TRAIT_VALUES)[K][number] };

/** CSS custom property values by name, without the leading `--`. */
export type TokenSet = Readonly<Record<string, string>>;

/** One built-in (or, later, plugin-contributed) theme. */
export interface ThemeDef {
  id: ThemeId;
  /** Display name (localised by the caller). */
  name: string;
  /** Tokens shared by both schemes: type, spacing, radii, motion, chrome sizes. */
  common: TokenSet;
  /** Colour roles per scheme. */
  schemes: Readonly<Record<Scheme, TokenSet>>;
  /** Row and lane sizes per density. */
  densities: Readonly<Record<Density, TokenSet>>;
  traits: ThemeTraits;
}
