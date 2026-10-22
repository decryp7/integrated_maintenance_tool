/**
 * The one place that turns quantities into text: byte sizes, transfer speed, remaining time, durations, counts,
 * percentages and relative times. Pure functions; numbers follow the active locale (`Intl`), words go through
 * `$localize`. Unknown, negative or non-finite input shows as "—" (`NONE`), except where the value is optional
 * decoration (a rate or an ETA), which returns '' so callers can drop the whole fragment.
 *
 * Byte sizes use binary steps (1024) with the short labels KB / MB / GB / TB, as Windows Explorer does, so a file
 * shown in Debrief matches what the file manager shows. Keep exact values in a tooltip (`formatExactBytes`).
 */
import { activeLocale, groupedNumber, plural } from '../../i18n/locale';

/** Shown for a value that is missing or not a usable number. */
export const NONE = '—';

export interface Units {
  ms: string;
  s: string;
  min: string;
  h: string;
  d: string;
  thousand: string;
  million: string;
}
let units: Units | null = null;

/** Unit abbreviations the UI writes after numbers, translated once on first use. */
export function unitLabels(): Units {
  return (units ??= {
    ms: $localize`:Abbreviation for milliseconds, after a duration@@core.format.unit.ms:ms`,
    s: $localize`:Abbreviation for seconds, after a duration@@core.format.unit.s:s`,
    min: $localize`:Abbreviation for minutes, after a duration@@core.format.unit.min:min`,
    h: $localize`:Abbreviation for hours, after a duration@@core.format.unit.h:h`,
    d: $localize`:Abbreviation for days, after a duration@@core.format.unit.d:d`,
    thousand: $localize`:Suffix for thousands in compact numbers, as in 1.2k@@core.format.suffix.thousand:k`,
    million: $localize`:Suffix for millions in compact numbers, as in 2.5M@@core.format.suffix.million:M`,
  });
}

const formats = new Map<string, Intl.NumberFormat>();
/** A cached `Intl.NumberFormat` of the active locale with at most `max` fraction digits. */
function nf(max: number): Intl.NumberFormat {
  const key = `${activeLocale()}|${max}`;
  let f = formats.get(key);
  if (!f) {
    f = new Intl.NumberFormat(activeLocale(), { maximumFractionDigits: max });
    formats.set(key, f);
  }
  return f;
}

/** A number with at most `digits` decimals in the active locale (no trailing zeros). */
export function formatDecimal(n: number, digits = 1): string {
  return Number.isFinite(n) ? nf(digits).format(n) : NONE;
}

/** 12345 -> "12,345" (grouping of the active locale). */
export function formatCount(n: number): string {
  return Number.isFinite(n) ? groupedNumber(n) : NONE;
}

/** 1234 -> "1.2k": only where space is tight. */
export function formatCompact(n: number): string {
  if (!Number.isFinite(n)) return NONE;
  const u = unitLabels();
  const a = Math.abs(n);
  if (a < 1000) return String(n);
  if (a < 1_000_000) return nf(a < 10_000 ? 1 : 0).format(n / 1000) + u.thousand;
  return nf(1).format(n / 1_000_000) + u.million;
}

/** A measured value at a glance: 2.5M, 25k, 250, 25.4, 2.5. */
export function formatNumber(v: number): string {
  if (!Number.isFinite(v)) return NONE;
  const a = Math.abs(v);
  const u = unitLabels();
  if (a >= 1_000_000) return nf(1).format(v / 1_000_000) + u.million;
  if (a >= 10_000) return nf(0).format(v / 1000) + u.thousand;
  if (a >= 100) return String(Math.round(v));
  if (a >= 10) return nf(1).format(v);
  return nf(2).format(v);
}

const BYTE_UNITS = ['KB', 'MB', 'GB', 'TB', 'PB'] as const;

function plainBytes(n: number): string {
  return plural(n, {
    one: $localize`:@@core.units.bytes.one:${n}:count: byte`,
    other: $localize`:@@core.units.bytes.other:${groupedNumber(n)}:count: bytes`,
  });
}

function usable(n: number | null | undefined): n is number {
  return typeof n === 'number' && Number.isFinite(n) && n >= 0;
}

function roundSize(value: number): number {
  return value < 10 ? Math.round(value * 10) / 10 : Math.round(value);
}

/** "12 bytes", "1 byte", "850 KB", "1.2 GB". 1023.9 KB rounds up to "1 MB", not "1,024 KB". */
export function formatBytes(bytes: number | null | undefined): string {
  if (!usable(bytes)) return NONE;
  if (bytes < 1024) return plainBytes(Math.round(bytes));
  let value = bytes / 1024;
  let i = 0;
  while (i < BYTE_UNITS.length - 1 && roundSize(value) >= 1024) {
    value /= 1024;
    i++;
  }
  return `${nf(value < 10 ? 1 : 0).format(value)} ${BYTE_UNITS[i]}`;
}

/** The exact size for a tooltip: "1,234,567 bytes". */
export function formatExactBytes(bytes: number | null | undefined): string {
  return usable(bytes) ? plainBytes(Math.round(bytes)) : NONE;
}

/** "12.4 MB/s"; '' while unknown or idle. */
export function formatRate(bytesPerSec: number | null | undefined): string {
  if (!usable(bytesPerSec) || bytesPerSec <= 0) return '';
  const size = formatBytes(bytesPerSec);
  return $localize`:@@core.units.perSecond:${size}:size:/s`;
}

/** "250 events/s", "1.2k events/s"; '' while unknown or idle. */
export function formatEventRate(perSec: number | null | undefined): string {
  if (!usable(perSec) || perSec <= 0) return '';
  const n = perSec < 10 ? formatDecimal(perSec, 1) : formatCompact(Math.round(perSec));
  return $localize`:@@core.units.eventsPerSecond:${n}:count: events/s`;
}

/**
 * "about 2 min left", "less than a minute left"; '' while unknown. Rounded to the unit people read, so it does not
 * flicker second by second.
 */
export function formatEta(secs: number | null | undefined): string {
  if (!usable(secs)) return '';
  if (secs < 60) return $localize`:@@core.units.eta.underMinute:less than a minute left`;
  const minutes = Math.round(secs / 60);
  if (minutes < 60) return $localize`:@@core.units.eta.minutes:about ${minutes}:minutes: min left`;
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  if (h >= 24) {
    const d = Math.floor(h / 24);
    return $localize`:@@core.units.eta.days:about ${d}:days: d left`;
  }
  return m
    ? $localize`:@@core.units.eta.hoursMinutes:about ${h}:hours: h ${m}:minutes: min left`
    : $localize`:@@core.units.eta.hours:about ${h}:hours: h left`;
}

/** "350 ms", "4.2 s", "42 s", "3 min 10 s", "2 h 5 min", "1 d 3 h". */
export function formatDuration(ms: number | null | undefined): string {
  if (!usable(ms)) return NONE;
  const u = unitLabels();
  if (ms < 999.5) return `${Math.round(ms)} ${u.ms}`;
  if (ms < 9950) return `${nf(1).format(ms / 1000)} ${u.s}`;
  const total = Math.round(ms / 1000);
  const s = total % 60;
  const min = Math.floor(total / 60) % 60;
  const h = Math.floor(total / 3600) % 24;
  const d = Math.floor(total / 86400);
  const parts: string[] = [];
  if (d) parts.push(`${nf(0).format(d)} ${u.d}`, ...(h ? [`${h} ${u.h}`] : []));
  else if (h) parts.push(`${h} ${u.h}`, ...(min ? [`${min} ${u.min}`] : []));
  else if (min) parts.push(`${min} ${u.min}`, ...(s ? [`${s} ${u.s}`] : []));
  else parts.push(`${s} ${u.s}`);
  return parts.join(' ');
}

/** A percentage given as 0..100: "42%", "42.5%" (`digits` decimals at most). */
export function formatPercent(pct: number | null | undefined, digits = 0): string {
  if (pct === null || pct === undefined || !Number.isFinite(pct)) return NONE;
  return new Intl.NumberFormat(activeLocale(), { style: 'percent', maximumFractionDigits: digits }).format(pct / 100);
}

/** Progress as a whole percentage 0..100 for a bar; 0 while the total is unknown. */
export function percent(done: number, total: number): number {
  if (!(total > 0) || !(done > 0)) return 0;
  return Math.min(100, Math.round((done / total) * 100));
}

/** "just now", "12 sec. ago", "5 min. ago", "2 hr. ago", "3 days ago" (`Intl.RelativeTimeFormat`, short style). */
export function formatRelativeTime(ts: number | null | undefined, now: number): string {
  if (ts === null || ts === undefined || !Number.isFinite(ts) || !Number.isFinite(now)) return NONE;
  const s = Math.max(0, Math.round((now - ts) / 1000));
  if (s < 2) return $localize`:@@core.units.justNow:just now`;
  const rtf = new Intl.RelativeTimeFormat(activeLocale(), { numeric: 'auto', style: 'short' });
  if (s < 60) return rtf.format(-s, 'second');
  if (s < 3600) return rtf.format(-Math.floor(s / 60), 'minute');
  if (s < 86400) return rtf.format(-Math.floor(s / 3600), 'hour');
  return rtf.format(-Math.floor(s / 86400), 'day');
}

/** Exponentially smoothed transfer speed from (bytes, time) samples, and the time left from it. */
export class TransferMeter {
  /** Time constant of the moving average (ms): the speed follows the last few seconds. */
  static readonly WINDOW_MS = 3000;
  /** Samples must span at least this long before a rate and an ETA are offered (ms). */
  static readonly MIN_SPAN_MS = 2000;

  private first: { bytes: number; at: number } | null = null;
  private last: { bytes: number; at: number } | null = null;
  private ema = 0;

  /** Adds a sample (cumulative `bytes` at `at` ms). A decrease in bytes or time restarts the meter. */
  sample(bytes: number, at: number): void {
    if (!Number.isFinite(bytes) || !Number.isFinite(at)) return;
    if (!this.last || bytes < this.last.bytes || at < this.last.at) {
      this.first = this.last = { bytes, at };
      this.ema = 0;
      return;
    }
    const dt = at - this.last.at;
    if (dt <= 0) return;
    const inst = ((bytes - this.last.bytes) * 1000) / dt;
    // The first interval seeds the average; later ones blend in with weight dt / (dt + window).
    const alpha = this.ema === 0 ? 1 : dt / (dt + TransferMeter.WINDOW_MS);
    this.ema = this.ema * (1 - alpha) + inst * alpha;
    this.last = { bytes, at };
  }

  /** Bytes per second, or 0 until the samples span at least `MIN_SPAN_MS`. */
  get rate(): number {
    if (!this.first || !this.last || this.last.at - this.first.at < TransferMeter.MIN_SPAN_MS) return 0;
    return this.ema;
  }

  /** Seconds left for `total` bytes; null while the speed is unknown or about zero. */
  etaSecs(total: number): number | null {
    const r = this.rate;
    if (!this.last || r < 1 || !(total > 0)) return null;
    return Math.max(0, (total - this.last.bytes) / r);
  }

  reset(): void {
    this.first = this.last = null;
    this.ema = 0;
  }
}
