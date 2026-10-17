import { Timeline } from '../../core/api.types';
import { timeTicks, TimeZoneMode } from '../../core/format';

/**
 * A pure serializer of the timeline model (lanes with density buckets and marker clusters, annotations,
 * time axis) into a standalone SVG document. It draws from the data, not the DOM, so exports look the same
 * at any window size; every text is escaped and every colour checked, and the result uses only elements
 * the engine's SVG sanitizer keeps (svg, g, rect, line, circle, text, title).
 */

export type SvgTheme = 'light' | 'dark';

export interface TimelineSvgAnnotation {
  from: number;
  to: number;
  label: string;
  /** #RGB / #RRGGBB(AA); anything else falls back to the theme's accent. */
  color?: string | null;
}

export interface TimelineSvgInput {
  timeline: Timeline;
  title: string;
  /** Marker colours by kind (from the summary's marker kinds). */
  markerColors?: Record<string, string | null | undefined>;
  annotations?: readonly TimelineSvgAnnotation[];
  theme: SvgTheme;
  tz: TimeZoneMode;
  /** Total width in px (default 1200, at least 400). */
  width?: number;
}

export interface SvgPalette {
  background: string;
  text: string;
  muted: string;
  grid: string;
  density: string;
  accent: string;
}

export const SVG_PALETTES: Record<SvgTheme, SvgPalette> = {
  light: { background: '#ffffff', text: '#1d2330', muted: '#5b6475', grid: '#e3e7ee', density: '#2f6fde', accent: '#7a5af8' },
  dark: { background: '#14171d', text: '#e6e9ef', muted: '#9aa3b2', grid: '#2c323d', density: '#7aa7ff', accent: '#b69cff' },
};

const LABEL_W = 190;
const RIGHT = 16;
const TITLE_H = 30;
const AXIS_H = 26;
const LANE_H = 24;
const ANNOTATION_H = 20;
const MAX_LABEL = 28;

/** Escapes text for SVG content and attribute values. */
export function escapeXml(text: string): string {
  return text.replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]!);
}

/** A colour safe to put in an attribute, or the fallback. */
export function safeColor(color: string | null | undefined, fallback: string): string {
  return color && /^#(?:[0-9a-f]{3}|[0-9a-f]{4}|[0-9a-f]{6}|[0-9a-f]{8})$/i.test(color.trim()) ? color.trim() : fallback;
}

function shorten(text: string, max = MAX_LABEL): string {
  return text.length > max ? `${text.slice(0, max - 1)}…` : text;
}

const n = (v: number) => (Math.round(v * 10) / 10).toString();

/** The timeline as an SVG document (no XML declaration; the engine adds one when saving). */
export function timelineToSvg(input: TimelineSvgInput): string {
  const t = input.timeline;
  const p = SVG_PALETTES[input.theme];
  const width = Math.max(400, Math.round(input.width ?? 1200));
  const plotW = width - LABEL_W - RIGHT;
  const annotations = input.annotations ?? [];
  const annH = annotations.length ? ANNOTATION_H : 0;
  const lanesTop = TITLE_H + AXIS_H + annH;
  const note = t.truncatedLanes > 0 ? 18 : 0;
  const height = lanesTop + Math.max(1, t.lanes.length) * LANE_H + note + 8;
  const span = Math.max(1, t.to - t.from);
  const x = (ts: number) => LABEL_W + Math.min(1, Math.max(0, (ts - t.from) / span)) * plotW;
  const font = `font-family="Helvetica, Arial, sans-serif"`;
  const out: string[] = [];

  out.push(`<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}">`);
  out.push(`<title>${escapeXml(input.title)}</title>`);
  out.push(`<rect x="0" y="0" width="${width}" height="${height}" fill="${p.background}"/>`);
  out.push(`<text x="12" y="20" font-size="14" font-weight="bold" fill="${p.text}" ${font}>${escapeXml(input.title)}</text>`);

  // Axis: tick lines across the lanes, labels above them.
  out.push('<g class="axis">');
  for (const tick of timeTicks(t.from, t.to, Math.max(2, Math.floor(plotW / 110)), input.tz)) {
    const tx = n(LABEL_W + tick.frac * plotW);
    out.push(`<line x1="${tx}" y1="${TITLE_H + AXIS_H - 6}" x2="${tx}" y2="${height - note - 4}" stroke="${p.grid}" stroke-width="1"/>`);
    out.push(`<text x="${tx}" y="${TITLE_H + 14}" font-size="10" text-anchor="middle" fill="${p.muted}" ${font}>${escapeXml(tick.label)}</text>`);
  }
  out.push('</g>');

  // Annotations: a band (or a tick for a point) in their own track.
  if (annotations.length) {
    out.push('<g class="annotations">');
    const y = TITLE_H + AXIS_H;
    for (const a of annotations) {
      const color = safeColor(a.color, p.accent);
      const x1 = x(a.from);
      const x2 = Math.max(x1 + 2, x(a.to));
      out.push(`<g><title>${escapeXml(a.label)}</title><rect x="${n(x1)}" y="${y + 3}" width="${n(x2 - x1)}" height="${ANNOTATION_H - 6}" rx="2" fill="${color}" fill-opacity="0.35" stroke="${color}"/></g>`);
    }
    out.push('</g>');
  }

  // Lanes: label, matched-density bars and marker clusters.
  out.push('<g class="lanes">');
  t.lanes.forEach((lane, i) => {
    const top = lanesTop + i * LANE_H;
    const max = Math.max(1, ...lane.density);
    const bw = plotW / Math.max(1, lane.density.length);
    out.push(`<g class="lane">`);
    out.push(`<line x1="${LABEL_W}" y1="${top + LANE_H}" x2="${width - RIGHT}" y2="${top + LANE_H}" stroke="${p.grid}" stroke-width="1"/>`);
    out.push(`<text x="10" y="${top + 16}" font-size="11" fill="${p.text}" ${font}><title>${escapeXml(lane.key)}</title>${escapeXml(shorten(lane.key))}</text>`);
    lane.density.forEach((d, b) => {
      if (d <= 0) return;
      const h = Math.max(1, (d / max) * (LANE_H - 8));
      out.push(`<rect x="${n(LABEL_W + b * bw)}" y="${n(top + LANE_H - 2 - h)}" width="${n(Math.max(1, bw - 0.5))}" height="${n(h)}" fill="${p.density}" fill-opacity="0.45"/>`);
    });
    for (const m of lane.markers) {
      const color = safeColor(input.markerColors?.[m.kind], p.muted);
      const label = `${m.kind}${m.count > 1 ? ` ×${m.count}` : ''}: ${m.message}`;
      out.push(`<circle cx="${n(x(m.ts))}" cy="${top + LANE_H / 2}" r="${m.count > 1 ? 5 : 4}" fill="${color}" stroke="${p.background}" stroke-width="1"><title>${escapeXml(label)}</title></circle>`);
    }
    out.push('</g>');
  });
  if (!t.lanes.length) out.push(`<text x="${LABEL_W}" y="${lanesTop + 16}" font-size="11" fill="${p.muted}" ${font}>No events in this range.</text>`);
  out.push('</g>');
  if (note) {
    out.push(`<text x="10" y="${height - 10}" font-size="10" fill="${p.muted}" ${font}>${t.truncatedLanes} more instance${t.truncatedLanes === 1 ? '' : 's'} not shown</text>`);
  }
  out.push('</svg>');
  return out.join('');
}
