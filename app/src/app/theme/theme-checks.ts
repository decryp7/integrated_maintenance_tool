/**
 * Quality gates on a theme's tokens (ADR 0018), pure so that a unit test can run them over every theme and scheme:
 * text contrast (WCAG 2.2 AA, 4.5:1) on every surface text is drawn on, non-text contrast (3:1) of marks, and the
 * categorical palettes (sources, chart series) under colour-vision deficiency. A plugin theme will go through the same
 * checks before it is accepted.
 */
import { contrastRatio, paletteProblems } from '../tools/monitor/cvd';
import { TokenSet } from './theme.types';

/** Minimum contrast of text (WCAG 1.4.3 AA). */
export const TEXT_CONTRAST = 4.5;
/** Minimum contrast of marks, glyphs and focus indicators (WCAG 1.4.11). */
export const GRAPHIC_CONTRAST = 3;

/** Text roles drawn on any panel surface. */
export const TEXT_ROLES = ['text', 'text-2', 'muted', 'faint', 'accent-text', 'ok', 'warn-text', 'danger-text', 'fatal-text'] as const;
/** The surfaces text roles are drawn on. */
export const TEXT_SURFACES = ['bg-0', 'bg-1', 'bg-2', 'bg-3', 'bg-hover', 'bg-active', 'bg-sel', 'tooltip-bg'] as const;
/** Text drawn on one particular surface: [text, surface]. */
export const TEXT_PAIRS: readonly (readonly [string, string])[] = [
  ['accent-ink', 'accent'],
  ['chip-text', 'chip-bg'],
  ['readout-text', 'readout-bg'],
  ['statusbar-text', 'statusbar-bg'],
  ['statusbar-strong', 'statusbar-bg'],
  ['text', 'titlebar-bg'],
  ['muted', 'titlebar-bg'],
  ['muted', 'activity-bg'],
  ['text', 'code-bg'],
  ['window-close-text', 'window-close-bg'],
];

/** Marks that must stand out from the lanes and rows they are drawn on. */
export const GRAPHIC_ROLES = ['mk-start', 'mk-stop', 'mk-fatal', 'mk-exception', 'mk-error', 'mk-warning', 'lvl-warn', 'lvl-error', 'lvl-fatal', 'focus-ring', 'brush-edge'] as const;
/** Where marks are drawn: panels, lanes, hovered and selected rows. */
export const GRAPHIC_SURFACES = ['bg-1', 'bg-hover', 'bg-sel', 'lane-bar'] as const;

/** Source colours (rows, Explorer, lanes) and where they are drawn. */
export const SOURCE_ROLES = ['src-1', 'src-2', 'src-3', 'src-4'] as const;
export const SOURCE_SURFACES = ['bg-1', 'bg-2', 'bg-3', 'bg-hover', 'bg-active', 'bg-sel'] as const;
/** Chart series colours and the chart surfaces. */
export const SERIES_ROLES = ['s1', 's2', 's3', 's4'] as const;
export const SERIES_SURFACES = ['bg-1', 'bg-2', 'bg-3'] as const;

function pairProblem(tokens: TokenSet, fg: string, bg: string, min: number): string | null {
  const a = tokens[fg];
  const b = tokens[bg];
  if (!a || !b) return `--${fg} on --${bg}: missing`;
  const r = contrastRatio(a, b);
  if (r === null) return `--${fg} (${a}) on --${bg} (${b}): not #rrggbb`;
  return r < min ? `--${fg} (${a}) on --${bg} (${b}) has contrast ${r.toFixed(2)}:1, below ${min}:1` : null;
}

/** Every text and mark pair below its minimum contrast (empty = the token set passes). */
export function contrastProblems(tokens: TokenSet): string[] {
  const out: (string | null)[] = [];
  for (const fg of TEXT_ROLES) for (const bg of TEXT_SURFACES) out.push(pairProblem(tokens, fg, bg, TEXT_CONTRAST));
  for (const [fg, bg] of TEXT_PAIRS) out.push(pairProblem(tokens, fg, bg, TEXT_CONTRAST));
  for (const fg of GRAPHIC_ROLES) for (const bg of GRAPHIC_SURFACES) out.push(pairProblem(tokens, fg, bg, GRAPHIC_CONTRAST));
  return out.filter((p): p is string => p !== null);
}

/**
 * Problems of the categorical palettes: every pair of source colours and of chart series colours separates under
 * protanopia, deuteranopia, tritanopia and normal vision, and each has 3:1 contrast on its surfaces.
 */
export function paletteProblemsOf(tokens: TokenSet): string[] {
  const pick = (names: readonly string[]) => names.map((n) => tokens[n] ?? `missing --${n}`);
  return [
    ...paletteProblems(pick(SOURCE_ROLES), pick(SOURCE_SURFACES)).map((p) => `sources: ${p}`),
    ...paletteProblems(pick(SERIES_ROLES), pick(SERIES_SURFACES)).map((p) => `series: ${p}`),
  ];
}
