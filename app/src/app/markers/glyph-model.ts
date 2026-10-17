/** Pure glyph logic: which glyph a marker gets, badge geometry and colours, SVG sanitising. */
import { builtinIcon } from './builtin-icons';
import { CustomIcon, MARKER_SHAPES, MarkerShape } from '../core/api.types';
import { GlyphStyle } from './markers.types';

export const DEFAULT_COLOR = '#8a91a3';
export const MAX_BADGE_CHARS = 3;
const PNG_PREFIX = 'data:image/png;base64,';

export type GlyphSpec =
  | { type: 'shape'; kind: string; color: string | null }
  | { type: 'badge'; text: string; shape: MarkerShape; fill: string; ink: string }
  | { type: 'icon'; name: string; d: string; color: string }
  | { type: 'svg'; markup: string; color: string }
  | { type: 'png'; src: string };

// ---- colours ----------------------------------------------------------------------------------

/** `#abc` / `#aabbcc` to lower-case `#rrggbb`; null for anything else. */
export function normalizeHex(value: string | null | undefined): string | null {
  const m = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.exec((value ?? '').trim());
  if (!m) return null;
  const h = m[1].length === 3 ? [...m[1]].map((x) => x + x).join('') : m[1];
  return `#${h.toLowerCase()}`;
}

function luminance(hex: string): number {
  const channel = (i: number) => {
    const v = parseInt(hex.slice(1 + i * 2, 3 + i * 2), 16) / 255;
    return v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4);
  };
  return 0.2126 * channel(0) + 0.7152 * channel(1) + 0.0722 * channel(2);
}

/** WCAG contrast ratio (1..21) between two #rrggbb colours. */
export function contrastRatio(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

/** Black or white, whichever reads better on `fill`; the better of the two is always at least 4.5:1. */
export function inkFor(fill: string): string {
  return contrastRatio(fill, '#000000') >= contrastRatio(fill, '#ffffff') ? '#000000' : '#ffffff';
}

// ---- badges -----------------------------------------------------------------------------------

/** Visible badge text: trimmed, at most three characters; null when empty. */
export function badgeText(value: string | null | undefined): string | null {
  const t = [...(value ?? '').trim()].slice(0, MAX_BADGE_CHARS).join('');
  return t || null;
}

export function badgeShape(value: string | null | undefined): MarkerShape {
  return MARKER_SHAPES.find((s) => s === value) ?? 'circle';
}

/** SVG markup attributes for a badge outline on the 24 grid. */
export function shapeGeometry(shape: MarkerShape): { tag: 'circle' | 'rect' | 'polygon'; attrs: Record<string, string> } {
  switch (shape) {
    case 'square':
      return { tag: 'rect', attrs: { x: '1.5', y: '1.5', width: '21', height: '21', rx: '4' } };
    case 'diamond':
      return { tag: 'polygon', attrs: { points: '12,0.8 23.2,12 12,23.2 0.8,12' } };
    case 'hexagon':
      return { tag: 'polygon', attrs: { points: '12,1 21.5,6.5 21.5,17.5 12,23 2.5,17.5 2.5,6.5' } };
    default:
      return { tag: 'circle', attrs: { cx: '12', cy: '12', r: '11' } };
  }
}

/** Font size on the 24 grid so up to three characters fit the shape. */
export function badgeFontSize(chars: number, shape: MarkerShape): number {
  const base = shape === 'diamond' ? [12, 9.5, 7.5] : [14, 11.5, 9];
  return base[Math.min(Math.max(chars, 1), 3) - 1];
}

// ---- resolution -------------------------------------------------------------------------------

/** Decides what to draw: badge, then icon (built-in or custom), then the kind's default shape. */
export function resolveGlyph(style: GlyphStyle, icons: readonly CustomIcon[] = []): GlyphSpec {
  const color = normalizeHex(style.color);
  const text = badgeText(style.badge);
  if (text) {
    const fill = color ?? DEFAULT_COLOR;
    return { type: 'badge', text, shape: badgeShape(style.shape), fill, ink: inkFor(fill) };
  }
  const stroke = color ?? 'currentColor';
  if (style.svg && sanitizeSvg(style.svg)) return { type: 'svg', markup: style.svg, color: stroke };
  if (style.pngDataUrl?.startsWith(PNG_PREFIX)) return { type: 'png', src: style.pngDataUrl };
  const builtin = builtinIcon(style.icon);
  if (builtin) return { type: 'icon', name: builtin.name, d: builtin.d, color: stroke };
  const custom = style.icon ? icons.find((i) => i.name === style.icon) : undefined;
  if (custom?.svg) return { type: 'svg', markup: custom.svg, color: stroke };
  if (custom?.pngDataUrl?.startsWith(PNG_PREFIX)) return { type: 'png', src: custom.pngDataUrl };
  return { type: 'shape', kind: style.kind ?? '', color: style.color ?? null };
}

// ---- svg sanitising (defence in depth; the engine already sanitised) ---------------------------

const SVG_NS = 'http://www.w3.org/2000/svg';
const ELEMENTS = new Set(['svg', 'g', 'path', 'circle', 'ellipse', 'rect', 'line', 'polyline', 'polygon', 'title']);
const ATTRIBUTES = new Set([
  'viewBox', 'd', 'cx', 'cy', 'r', 'rx', 'ry', 'x', 'y', 'x1', 'y1', 'x2', 'y2', 'width', 'height', 'points', 'fill', 'fill-rule', 'fill-opacity',
  'stroke', 'stroke-width', 'stroke-linecap', 'stroke-linejoin', 'stroke-opacity', 'stroke-dasharray', 'clip-rule', 'opacity', 'transform',
]);
const UNSAFE_VALUE = /url\s*\(|javascript:|data:|expression\s*\(|<|&#|@import/i;
const MAX_NODES = 400;
const MAX_DEPTH = 8;

/**
 * Rebuilds SVG markup as a detached element containing only allowlisted elements and attributes,
 * or null when it is not a usable `<svg>`. The result is created with createElementNS, never via
 * innerHTML, so nothing from the source string can execute.
 */
export function sanitizeSvg(markup: string, doc: Document = document): SVGSVGElement | null {
  let parsed: Document;
  try {
    parsed = new DOMParser().parseFromString(markup, 'image/svg+xml');
  } catch {
    return null;
  }
  const root = parsed.documentElement;
  if (!root || root.localName !== 'svg' || parsed.getElementsByTagName('parsererror').length) return null;
  let budget = MAX_NODES;
  const copy = (src: Element, depth: number): Element | null => {
    if (!ELEMENTS.has(src.localName) || depth > MAX_DEPTH || --budget < 0) return null;
    const out = doc.createElementNS(SVG_NS, src.localName);
    for (const attr of Array.from(src.attributes)) {
      if (ATTRIBUTES.has(attr.name) && !UNSAFE_VALUE.test(attr.value)) out.setAttribute(attr.name, attr.value);
    }
    if (src.localName === 'title') out.textContent = (src.textContent ?? '').slice(0, 200);
    else for (const child of Array.from(src.children)) {
      const c = copy(child, depth + 1);
      if (c) out.appendChild(c);
    }
    return out;
  };
  const clean = copy(root, 0) as SVGSVGElement | null;
  if (!clean) return null;
  if (!clean.hasAttribute('viewBox')) clean.setAttribute('viewBox', '0 0 24 24');
  clean.removeAttribute('width');
  clean.removeAttribute('height');
  clean.setAttribute('width', '100%');
  clean.setAttribute('height', '100%');
  clean.setAttribute('aria-hidden', 'true');
  clean.setAttribute('focusable', 'false');
  return clean;
}
