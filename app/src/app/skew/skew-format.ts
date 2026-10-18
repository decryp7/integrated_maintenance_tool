import { localizeDecimal } from '../i18n/locale';
/** Display formatting and input parsing of clock offsets (the engine validates and applies them). */

const UNITS: Record<string, number> = { ms: 1, s: 1000, sec: 1000, m: 60_000, min: 60_000, h: 3_600_000, d: 86_400_000 };

/** `+2.35 s`, `-1 h 2 min 3.5 s`, `0 s`: an offset as people read it (mirrors the engine's wording). */
export function formatOffset(ms: number): string {
  if (!Number.isFinite(ms) || ms === 0) return '0 s';
  const sign = ms < 0 ? '-' : '+';
  let rest = Math.abs(Math.round(ms));
  const parts: string[] = [];
  for (const [unit, size] of [['d', 86_400_000], ['h', 3_600_000], ['min', 60_000]] as const) {
    if (rest >= size) {
      parts.push(`${Math.floor(rest / size)} ${unit}`);
      rest %= size;
    }
  }
  if (rest > 0 || !parts.length) parts.push(`${localizeDecimal(String(Number((rest / 1000).toFixed(3))))} s`);
  return `${sign}${parts.join(' ')}`;
}

/**
 * Parses what the user typed: a signed number in seconds (`2.35`, `-1.5`), or with units
 * (`+2350ms`, `-1h 2min 3.5s`, `90 s`). Returns milliseconds, or null when it is not an offset.
 */
export function parseOffset(text: string): number | null {
  const t = text.trim().toLowerCase();
  if (!t) return null;
  const sign = t.startsWith('-') ? -1 : 1;
  const body = t.replace(/^[+-]\s*/, '');
  if (/^\d+(\.\d+)?$/.test(body)) return sign * Math.round(Number(body) * 1000);
  const re = /(\d+(?:\.\d+)?)\s*(ms|sec|min|s|m|h|d)\b/g;
  let total = 0;
  let consumed = '';
  for (const m of body.matchAll(re)) {
    total += Number(m[1]) * (UNITS[m[2] ?? ''] ?? 0);
    consumed += m[0];
  }
  if (!consumed || consumed.replace(/\s/g, '') !== body.replace(/\s/g, '')) return null;
  return sign * Math.round(total);
}

/** "86 %" for a 0..1 confidence. */
export function formatConfidence(c: number): string {
  return `${Math.round(Math.max(0, Math.min(1, c)) * 100)} %`;
}

/** A word for a confidence, so the number is not read on its own. */
export function confidenceLabel(c: number): 'high' | 'medium' | 'low' {
  return c >= 0.8 ? 'high' : c >= 0.5 ? 'medium' : 'low';
}
