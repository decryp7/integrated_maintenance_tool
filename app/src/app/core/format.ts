/** Time, number and axis helpers shared by the timeline, grid and charts. */

export type TimeZoneMode = 'utc' | 'local';

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

function pad(n: number, w = 2): string {
  return String(n).padStart(w, '0');
}

interface Parts {
  y: number;
  mo: number;
  d: number;
  h: number;
  mi: number;
  s: number;
  ms: number;
}

function parts(ts: number, tz: TimeZoneMode): Parts {
  const d = new Date(ts);
  return tz === 'utc'
    ? {
        y: d.getUTCFullYear(),
        mo: d.getUTCMonth(),
        d: d.getUTCDate(),
        h: d.getUTCHours(),
        mi: d.getUTCMinutes(),
        s: d.getUTCSeconds(),
        ms: d.getUTCMilliseconds(),
      }
    : {
        y: d.getFullYear(),
        mo: d.getMonth(),
        d: d.getDate(),
        h: d.getHours(),
        mi: d.getMinutes(),
        s: d.getSeconds(),
        ms: d.getMilliseconds(),
      };
}

/** "10:41:22.318" */
export function formatTime(ts: number | null, tz: TimeZoneMode): string {
  if (ts === null) return '—';
  const p = parts(ts, tz);
  return `${pad(p.h)}:${pad(p.mi)}:${pad(p.s)}.${pad(p.ms, 3)}`;
}

/** "14 Oct 10:41:22" */
export function formatDateTime(ts: number | null, tz: TimeZoneMode, withMs = false): string {
  if (ts === null) return '—';
  const p = parts(ts, tz);
  const base = `${p.d} ${MONTHS[p.mo]} ${pad(p.h)}:${pad(p.mi)}:${pad(p.s)}`;
  return withMs ? `${base}.${pad(p.ms, 3)}` : base;
}

/** "14 Oct 2026 10:41:22.318 UTC" */
export function formatFull(ts: number | null, tz: TimeZoneMode): string {
  if (ts === null) return 'No timestamp';
  const p = parts(ts, tz);
  return `${p.d} ${MONTHS[p.mo]} ${p.y} ${pad(p.h)}:${pad(p.mi)}:${pad(p.s)}.${pad(p.ms, 3)} ${tzLabel(tz)}`;
}

export function tzLabel(tz: TimeZoneMode): string {
  if (tz === 'utc') return 'UTC';
  const off = -new Date().getTimezoneOffset();
  const sign = off >= 0 ? '+' : '-';
  return `UTC${sign}${pad(Math.floor(Math.abs(off) / 60))}:${pad(Math.abs(off) % 60)}`;
}

export function formatCount(n: number): string {
  return n.toLocaleString('en-US');
}

/** 1234 -> "1.2k" */
export function formatCompact(n: number): string {
  if (n < 1000) return String(n);
  if (n < 1_000_000) return `${(n / 1000).toFixed(n < 10_000 ? 1 : 0)}k`;
  return `${(n / 1_000_000).toFixed(1)}M`;
}

export function formatNumber(v: number): string {
  const a = Math.abs(v);
  if (a >= 1_000_000) return `${(v / 1_000_000).toFixed(1)}M`;
  if (a >= 10_000) return `${(v / 1000).toFixed(0)}k`;
  if (a >= 100) return v.toFixed(0);
  if (a >= 10) return v.toFixed(1);
  return v.toFixed(2).replace(/\.?0+$/, '') || '0';
}

export function formatDuration(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)} ms`;
  if (ms < 60_000) return `${+(ms / 1000).toFixed(1)} s`;
  if (ms < 3_600_000) return `${+(ms / 60_000).toFixed(1)} min`;
  if (ms < 86_400_000) return `${+(ms / 3_600_000).toFixed(1)} h`;
  return `${+(ms / 86_400_000).toFixed(1)} d`;
}

const SEC = 1000;
const MIN = 60 * SEC;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;
const STEPS = [
  1, 2, 5, 10, 20, 50, 100, 200, 500,
  SEC, 2 * SEC, 5 * SEC, 10 * SEC, 15 * SEC, 30 * SEC,
  MIN, 2 * MIN, 5 * MIN, 10 * MIN, 15 * MIN, 30 * MIN,
  HOUR, 2 * HOUR, 3 * HOUR, 6 * HOUR, 12 * HOUR,
  DAY, 2 * DAY, 7 * DAY, 14 * DAY, 30 * DAY,
];

export interface Tick {
  ts: number;
  frac: number;
  label: string;
}

/** Evenly spaced, human-aligned ticks between from and to. */
export function timeTicks(from: number, to: number, maxTicks: number, tz: TimeZoneMode): Tick[] {
  const span = to - from;
  if (span <= 0 || maxTicks < 1) return [];
  const step = STEPS.find((s) => span / s <= maxTicks) ?? STEPS[STEPS.length - 1];
  const offset = tz === 'utc' ? 0 : -new Date(from).getTimezoneOffset() * MIN;
  const first = Math.ceil((from + offset) / step) * step - offset;
  const ticks: Tick[] = [];
  for (let t = first; t <= to; t += step) {
    const p = parts(t, tz);
    let label: string;
    if (step >= DAY) label = `${p.d} ${MONTHS[p.mo]}`;
    else if (step >= MIN) label = p.h === 0 && p.mi === 0 ? `${p.d} ${MONTHS[p.mo]}` : `${pad(p.h)}:${pad(p.mi)}`;
    else if (step >= SEC) label = `${pad(p.h)}:${pad(p.mi)}:${pad(p.s)}`;
    else label = `${pad(p.s)}.${pad(p.ms, 3)}`;
    ticks.push({ ts: t, frac: (t - from) / span, label });
  }
  return ticks;
}

/** Parses "30s", "1m", "5m", "1h", "1d"; null for "auto"/invalid. */
export function parseBucket(s: string): number | null {
  const m = /^(\d+(?:\.\d+)?)\s*(ms|s|m|h|d)$/.exec(s.trim());
  if (!m) return null;
  const n = parseFloat(m[1]);
  const unit = { ms: 1, s: SEC, m: MIN, h: HOUR, d: DAY }[m[2] as 'ms' | 's' | 'm' | 'h' | 'd'];
  return n * unit;
}

export function clamp(v: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, v));
}

export function valueToString(v: unknown): string {
  if (v === null || v === undefined) return '';
  if (typeof v === 'string') return v;
  return JSON.stringify(v);
}
