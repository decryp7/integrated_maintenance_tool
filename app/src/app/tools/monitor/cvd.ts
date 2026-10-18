/**
 * Colour-vision-deficiency check for categorical series colours: Machado, Oliveira & Fernandes (2009) simulation at
 * severity 1.0 and Euclidean distance in OKLab (×100), plus WCAG contrast. Pure; the thresholds follow the dataviz
 * palette validator. `palette.spec.ts` runs it over the theme's series tokens.
 */

/** Below this simulated ΔE two series colours would need a secondary encoding (a dash, a texture). */
export const CVD_TARGET = 8;
/** Minimum WCAG contrast of a series mark against the chart surface. */
export const MIN_CONTRAST = 3;
/** Below this unsimulated ΔE two colours are hard to tell apart even with full colour vision. */
export const NORMAL_FLOOR = 15;

export type CvdKind = 'protan' | 'deutan' | 'tritan';
export const CVD_KINDS: readonly CvdKind[] = ['protan', 'deutan', 'tritan'];

type Rgb = [number, number, number];
type Matrix = [Rgb, Rgb, Rgb];

const MACHADO: Record<CvdKind, Matrix> = {
  protan: [
    [0.152286, 1.052583, -0.204868],
    [0.114503, 0.786281, 0.099216],
    [-0.003882, -0.048116, 1.051998],
  ],
  deutan: [
    [0.367322, 0.860646, -0.227968],
    [0.280085, 0.672501, 0.047413],
    [-0.01182, 0.04294, 0.968881],
  ],
  tritan: [
    [1.255528, -0.076749, -0.178779],
    [-0.078411, 0.930809, 0.147602],
    [0.004733, 0.691367, 0.3039],
  ],
};

/** The deficiencies the separation gate uses by default, as the validator does (tritan is rare and only reported there). */
const GATED: CvdKind[] = ['protan', 'deutan'];

/** Linear RGB of a `#rrggbb` colour, or null for anything else. */
function linear(hex: string): Rgb | null {
  const m = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
  if (!m?.[1]) return null;
  const v = m[1];
  const ch = (i: number) => {
    const c = parseInt(v.slice(i, i + 2), 16) / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return [ch(0), ch(2), ch(4)];
}

function oklab([r, g, b]: Rgb): Rgb {
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  return [
    0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
    1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
    0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
  ];
}

function simulate(c: Rgb, kind: CvdKind): Rgb {
  const clamp = (x: number) => Math.max(0, Math.min(1, x));
  const row = (i: 0 | 1 | 2) => {
    const r = MACHADO[kind][i];
    return clamp(r[0] * c[0] + r[1] * c[1] + r[2] * c[2]);
  };
  return [row(0), row(1), row(2)];
}

/** OKLab ΔE×100 between two colours, under a simulated deficiency or normal vision; null if either is not `#rrggbb`. */
export function deltaE(a: string, b: string, kind?: CvdKind): number | null {
  const la = linear(a);
  const lb = linear(b);
  if (!la || !lb) return null;
  const pa = oklab(kind ? simulate(la, kind) : la);
  const pb = oklab(kind ? simulate(lb, kind) : lb);
  return 100 * Math.hypot(pa[0] - pb[0], pa[1] - pb[1], pa[2] - pb[2]);
}

/** The smallest ΔE under the given deficiencies, protanopia and deuteranopia by default (null when a colour cannot be read). */
export function worstCvdDelta(a: string, b: string, kinds: readonly CvdKind[] = GATED): number | null {
  let worst: number | null = null;
  for (const k of kinds) {
    const d = deltaE(a, b, k);
    if (d === null) return null;
    worst = worst === null ? d : Math.min(worst, d);
  }
  return worst;
}

/** Two series colours are hard to tell apart, with or without a deficiency (unreadable colours count as confusable). */
export function confusable(a: string, b: string, kinds: readonly CvdKind[] = GATED): boolean {
  const d = worstCvdDelta(a, b, kinds);
  const normal = deltaE(a, b);
  return d === null || normal === null || d < CVD_TARGET || normal < NORMAL_FLOOR;
}

/** WCAG 2 contrast ratio of two `#rrggbb` colours (1–21), or null when either cannot be read. */
export function contrastRatio(a: string, b: string): number | null {
  const la = linear(a);
  const lb = linear(b);
  if (!la || !lb) return null;
  const y = ([r, g, bl]: Rgb) => 0.2126 * r + 0.7152 * g + 0.0722 * bl;
  const [hi, lo] = [y(la), y(lb)].sort((p, q) => q - p) as [number, number];
  return (hi + 0.05) / (lo + 0.05);
}

/**
 * Every problem of a categorical palette on its surfaces: confusable pairs (all pairs, under every deficiency and
 * normal vision) and marks below {@link MIN_CONTRAST} on a surface. Empty = the palette passes.
 */
export function paletteProblems(colors: readonly string[], surfaces: readonly string[]): string[] {
  const out: string[] = [];
  colors.forEach((a, i) =>
    colors.slice(i + 1).forEach((b) => {
      if (confusable(a, b, CVD_KINDS)) out.push(`${a} and ${b} are confusable (CVD ΔE ${worstCvdDelta(a, b, CVD_KINDS)?.toFixed(1)}, normal ΔE ${deltaE(a, b)?.toFixed(1)})`);
    }),
  );
  for (const c of colors) {
    for (const s of surfaces) {
      const r = contrastRatio(c, s);
      if (r === null || r < MIN_CONTRAST) out.push(`${c} on ${s} has contrast ${r?.toFixed(2) ?? 'unreadable'}:1, below ${MIN_CONTRAST}:1`);
    }
  }
  return out;
}
