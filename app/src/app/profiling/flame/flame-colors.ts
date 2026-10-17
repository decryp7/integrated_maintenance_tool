import { FrameDto, FrameKind } from '../../core/api.types';
import { FlameModel } from './flame-model';

/** How rectangles are coloured. Identity is also in the labels and the legend, never colour alone. */
export type ColorBy = 'kind' | 'module';

/** Colours resolved from the theme tokens at draw time (dark and light). */
export interface FlamePalette {
  /** The validated categorical slots --s1..--s4 (fixed order, never cycled). */
  slots: string[];
  other: string;
  /** Diverging poles and neutral midpoint: red = more in the target, blue = less. */
  more: string;
  less: string;
  neutral: string;
  /** Ink for labels on dark and on light fills. */
  inkDark: string;
  inkLight: string;
  outline: string;
}

/** CSS tokens the palette reads (`resolvePalette`). */
export const PALETTE_TOKENS = { slots: ['--s1', '--s2', '--s3', '--s4'], other: '--s-other', more: '--lvl-error', less: '--s1', neutral: '--s-other', outline: '--text' } as const;

/** Fallbacks (the dark theme's values) when tokens are unavailable, e.g. in tests. */
export const FALLBACK_PALETTE: FlamePalette = {
  slots: ['#3987e5', '#d95926', '#199e70', '#9085e9'],
  other: '#5b6478',
  more: '#f2545b',
  less: '#3987e5',
  neutral: '#5b6478',
  inkDark: '#0b0d12',
  inkLight: '#ffffff',
  outline: '#e3e6ee',
};

export function resolvePalette(read: (token: string) => string): FlamePalette {
  const get = (token: string, fallback: string) => read(token).trim() || fallback;
  return {
    slots: PALETTE_TOKENS.slots.map((t, i) => get(t, FALLBACK_PALETTE.slots[i])),
    other: get(PALETTE_TOKENS.other, FALLBACK_PALETTE.other),
    more: get(PALETTE_TOKENS.more, FALLBACK_PALETTE.more),
    less: get(PALETTE_TOKENS.less, FALLBACK_PALETTE.less),
    neutral: get(PALETTE_TOKENS.neutral, FALLBACK_PALETTE.neutral),
    inkDark: FALLBACK_PALETTE.inkDark,
    inkLight: FALLBACK_PALETTE.inkLight,
    outline: get(PALETTE_TOKENS.outline, FALLBACK_PALETTE.outline),
  };
}

/** Frame kinds in slot order: managed, native, kernel; unknown is "other". */
export const KIND_SLOT: Record<FrameKind, number | null> = { managed: 0, native: 1, kernel: 3, unknown: null };
export const KIND_LABEL: Record<FrameKind, string> = { managed: 'Managed', native: 'Native', kernel: 'Kernel', unknown: 'Unknown' };

/**
 * Sticky module → slot assignment: modules keep the slot they got (colour follows the entity, not
 * its rank); new modules take a free slot in value order; the rest fold into "Other".
 */
export function stickySlots(previous: ReadonlyMap<string, number>, modulesByValue: readonly string[], slots = 4): Map<string, number> {
  const out = new Map(previous);
  const used = new Set(out.values());
  for (const m of modulesByValue) {
    if (out.has(m)) continue;
    let free = 0;
    while (free < slots && used.has(free)) free++;
    if (free >= slots) break;
    out.set(m, free);
    used.add(free);
  }
  return out;
}

/** Modules of the visible rectangles ordered by summed self value (largest first). */
export function modulesByValue(model: FlameModel): string[] {
  const sums = new Map<string, number>();
  model.rects.forEach((r) => {
    const m = model.frames.get(r.frame)?.module ?? '';
    if (m) sums.set(m, (sums.get(m) ?? 0) + r.selfValue);
  });
  return [...sums.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).map(([m]) => m);
}

export function frameColor(frame: FrameDto | undefined, by: ColorBy, slots: ReadonlyMap<string, number>, p: FlamePalette): string {
  if (!frame) return p.other;
  const slot = by === 'kind' ? KIND_SLOT[frame.kind] : frame.module !== null ? (slots.get(frame.module) ?? null) : null;
  return slot === null ? p.other : p.slots[slot];
}

function parseHex(hex: string): [number, number, number] | null {
  const m = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return null;
  const n = parseInt(m[1], 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

const toHex = (c: number[]) => '#' + c.map((x) => Math.round(x).toString(16).padStart(2, '0')).join('');

/** Linear mix of two #rrggbb colours (non-hex inputs return `b` at t ≥ 0.5, else `a`). */
export function mix(a: string, b: string, t: number): string {
  const x = parseHex(a);
  const y = parseHex(b);
  if (!x || !y) return t >= 0.5 ? b : a;
  return toHex(x.map((v, i) => v + (y[i] - v) * Math.max(0, Math.min(1, t))));
}

/**
 * Diverging colour of a differential rectangle: neutral grey at no change, towards red when the
 * target's share grew and blue when it shrank, saturating at `maxAbs`.
 */
export function diffColor(delta: number | undefined, maxAbs: number, p: FlamePalette): string {
  if (delta === undefined || !maxAbs || Math.abs(delta) < 1e-4) return p.neutral;
  return mix(p.neutral, delta > 0 ? p.more : p.less, Math.min(1, Math.abs(delta) / maxAbs));
}

/** Label ink with enough contrast on `bg` (relative luminance threshold). */
export function inkFor(bg: string, p: FlamePalette): string {
  const c = parseHex(bg);
  if (!c) return p.inkLight;
  const [r, g, b] = c.map((v) => {
    const s = v / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  });
  const lum = 0.2126 * r + 0.7152 * g + 0.0722 * b;
  return lum > 0.25 ? p.inkDark : p.inkLight;
}

/** Largest |delta| of the visible rectangles (the legend's scale end). */
export function maxAbsDelta(model: FlameModel): number {
  return model.rects.reduce((m, r) => Math.max(m, Math.abs(r.delta ?? 0)), 0);
}
