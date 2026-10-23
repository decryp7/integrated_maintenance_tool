import { readFileSync } from 'node:fs';
import { afterEach, describe, expect, it } from 'vitest';
import { setActiveLocale, SOURCE_LOCALE } from '../../i18n/locale';
import {
  formatBytes,
  formatCompact,
  formatCount,
  formatDecimal,
  formatDuration,
  formatEta,
  formatEventRate,
  formatExactBytes,
  formatNumber,
  formatPercent,
  formatConnections,
  formatRate,
  formatRelativeTime,
  NONE,
  percent,
  TransferMeter,
} from './units';

afterEach(() => setActiveLocale(SOURCE_LOCALE));

describe('formatBytes', () => {
  it('writes small sizes as bytes', () => {
    expect(formatBytes(0)).toBe('0 bytes');
    expect(formatBytes(1)).toBe('1 byte');
    expect(formatBytes(12)).toBe('12 bytes');
    expect(formatBytes(1023)).toBe('1,023 bytes');
    expect(formatBytes(0.4)).toBe('0 bytes');
  });

  it('scales in binary steps with short labels', () => {
    expect(formatBytes(1024)).toBe('1 KB');
    expect(formatBytes(850 * 1024)).toBe('850 KB');
    expect(formatBytes(1.5 * 1024)).toBe('1.5 KB');
    expect(formatBytes(1.2 * 1024 ** 3)).toBe('1.2 GB');
    expect(formatBytes(5 * 1024 ** 4)).toBe('5 TB');
    expect(formatBytes(3 * 1024 ** 5)).toBe('3 PB');
    expect(formatBytes(5000 * 1024 ** 5)).toBe('5,000 PB');
  });

  it('rolls over at rounding boundaries', () => {
    expect(formatBytes(1023.9 * 1024)).toBe('1 MB');
    expect(formatBytes(1023.4 * 1024)).toBe('1,023 KB');
    expect(formatBytes(1023.96 * 1024 ** 2)).toBe('1 GB');
    expect(formatBytes(9.96 * 1024)).toBe('10 KB');
  });

  it('shows a dash for anything that is not a size', () => {
    for (const v of [-1, Number.NaN, Infinity, null, undefined]) expect(formatBytes(v)).toBe(NONE);
    expect(formatExactBytes(-5)).toBe(NONE);
  });

  it('follows the locale', () => {
    setActiveLocale('de-DE');
    expect(formatBytes(1.5 * 1024)).toBe('1,5 KB');
    expect(formatExactBytes(1234567)).toBe('1.234.567 bytes');
  });

  it('gives the exact size for tooltips', () => {
    expect(formatExactBytes(1234567)).toBe('1,234,567 bytes');
    expect(formatExactBytes(1)).toBe('1 byte');
  });
});

describe('connections', () => {
  it('names the parallel connections only when there are several', () => {
    expect(formatConnections(4)).toBe('4 connections');
    expect(formatConnections(1)).toBe('');
    expect(formatConnections(0)).toBe('');
    expect(formatConnections(null)).toBe('');
    expect(formatConnections(Number.NaN)).toBe('');
  });
});

describe('rates and eta', () => {
  it('formats speed', () => {
    expect(formatRate(12.4 * 1024 ** 2)).toBe('12 MB/s');
    expect(formatRate(5.5 * 1024 ** 2)).toBe('5.5 MB/s');
    expect(formatRate(0)).toBe('');
    expect(formatRate(-1)).toBe('');
    expect(formatRate(Number.NaN)).toBe('');
    expect(formatRate(null)).toBe('');
  });

  it('formats event rates', () => {
    expect(formatEventRate(2.5)).toBe('2.5 events/s');
    expect(formatEventRate(1234)).toBe('1.2k events/s');
    expect(formatEventRate(0)).toBe('');
    expect(formatEventRate(undefined)).toBe('');
  });

  it('formats the time left', () => {
    expect(formatEta(null)).toBe('');
    expect(formatEta(-1)).toBe('');
    expect(formatEta(Infinity)).toBe('');
    expect(formatEta(0)).toBe('less than a minute left');
    expect(formatEta(59)).toBe('less than a minute left');
    expect(formatEta(60)).toBe('about 1 min left');
    expect(formatEta(130)).toBe('about 2 min left');
    expect(formatEta(3600)).toBe('about 1 h left');
    expect(formatEta(5400)).toBe('about 1 h 30 min left');
    expect(formatEta(3 * 86400)).toBe('about 3 d left');
  });
});

describe('formatDuration', () => {
  it('writes the two most significant units', () => {
    expect(formatDuration(0)).toBe('0 ms');
    expect(formatDuration(350)).toBe('350 ms');
    expect(formatDuration(999.6)).toBe('1 s');
    expect(formatDuration(4200)).toBe('4.2 s');
    expect(formatDuration(9990)).toBe('10 s');
    expect(formatDuration(42_000)).toBe('42 s');
    expect(formatDuration(59_600)).toBe('1 min');
    expect(formatDuration(60_000)).toBe('1 min');
    expect(formatDuration(190_000)).toBe('3 min 10 s');
    expect(formatDuration(7_500_000)).toBe('2 h 5 min');
    expect(formatDuration(7_200_000)).toBe('2 h');
    expect(formatDuration(97_200_000)).toBe('1 d 3 h');
    expect(formatDuration(86_400_000)).toBe('1 d');
  });

  it('shows a dash for bad input', () => {
    for (const v of [-1, Number.NaN, Infinity, null, undefined]) expect(formatDuration(v)).toBe(NONE);
  });

  it('uses the locale decimal separator', () => {
    setActiveLocale('de-DE');
    expect(formatDuration(4200)).toBe('4,2 s');
  });
});

describe('counts and percentages', () => {
  it('formats counts, compact counts and numbers', () => {
    expect(formatCount(12345)).toBe('12,345');
    expect(formatCount(Number.NaN)).toBe(NONE);
    expect(formatCompact(999)).toBe('999');
    expect(formatCompact(1234)).toBe('1.2k');
    expect(formatCompact(12345)).toBe('12k');
    expect(formatCompact(3_400_000)).toBe('3.4M');
    expect(formatCompact(Number.NaN)).toBe(NONE);
    expect(formatNumber(2_500_000)).toBe('2.5M');
    expect(formatNumber(25_000)).toBe('25k');
    expect(formatNumber(250.4)).toBe('250');
    expect(formatNumber(25.44)).toBe('25.4');
    expect(formatNumber(2.5)).toBe('2.5');
    expect(formatNumber(0)).toBe('0');
    expect(formatNumber(Infinity)).toBe(NONE);
    expect(formatDecimal(2.345, 2)).toBe('2.35');
    expect(formatDecimal(Number.NaN)).toBe(NONE);
  });

  it('formats percentages', () => {
    expect(formatPercent(42.4)).toBe('42%');
    expect(formatPercent(42.46, 1)).toBe('42.5%');
    expect(formatPercent(null)).toBe(NONE);
    expect(formatPercent(Number.NaN)).toBe(NONE);
    expect(percent(1, 0)).toBe(0);
    expect(percent(0, 10)).toBe(0);
    expect(percent(5, 10)).toBe(50);
    expect(percent(20, 10)).toBe(100);
  });
});

describe('formatRelativeTime', () => {
  const now = 10_000_000;
  it('words the age', () => {
    expect(formatRelativeTime(null, now)).toBe(NONE);
    expect(formatRelativeTime(Number.NaN, now)).toBe(NONE);
    expect(formatRelativeTime(now - 500, now)).toBe('just now');
    expect(formatRelativeTime(now + 5000, now)).toBe('just now');
    expect(formatRelativeTime(now - 12_000, now)).toMatch(/12 sec/);
    expect(formatRelativeTime(now - 300_000, now)).toMatch(/5 min/);
    expect(formatRelativeTime(now - 7_200_000, now)).toMatch(/2 hr/);
    expect(formatRelativeTime(now - 3 * 86_400_000, now)).toMatch(/3 days/);
  });
});

describe('TransferMeter', () => {
  const MB = 1024 * 1024;

  it('offers nothing before 2 s of samples', () => {
    const m = new TransferMeter();
    expect(m.rate).toBe(0);
    expect(m.etaSecs(100 * MB)).toBeNull();
    m.sample(0, 0);
    m.sample(MB, 1000);
    expect(m.rate).toBe(0);
    expect(m.etaSecs(100 * MB)).toBeNull();
  });

  it('reports a steady speed and the time left', () => {
    const m = new TransferMeter();
    for (let t = 0; t <= 4000; t += 500) m.sample((t / 1000) * 10 * MB, t);
    expect(m.rate).toBeCloseTo(10 * MB, -3);
    expect(m.etaSecs(100 * MB)).toBeCloseTo((100 - 40) / 10, 1);
  });

  it('smooths a spike instead of following it', () => {
    const m = new TransferMeter();
    for (let t = 0; t <= 4000; t += 500) m.sample((t / 1000) * 10 * MB, t);
    m.sample(40 * MB + 100 * MB, 4500);
    expect(m.rate).toBeGreaterThan(10 * MB);
    expect(m.rate).toBeLessThan(200 * MB);
  });

  it('has no ETA when stalled', () => {
    const m = new TransferMeter();
    m.sample(0, 0);
    for (let t = 1000; t <= 60_000; t += 1000) m.sample(MB, t);
    expect(m.etaSecs(100 * MB)).toBeNull();
  });

  it('ignores bad samples and repeated times, restarts on a decrease', () => {
    const m = new TransferMeter();
    m.sample(Number.NaN, 0);
    m.sample(0, 0);
    m.sample(MB, 0);
    m.sample(10 * MB, 3000);
    expect(m.rate).toBeGreaterThan(0);
    expect(m.etaSecs(0)).toBeNull();
    m.sample(MB, 4000);
    expect(m.rate).toBe(0);
    m.sample(MB, 3000);
    m.reset();
    expect(m.rate).toBe(0);
  });
});

describe('the table shared with Rust (debrief-plugin-api units)', () => {
  const table = JSON.parse(readFileSync('../crates/debrief-plugin-api/units-cases.json', 'utf8')) as Record<string, { in: unknown; out: string }[]>;
  const each = (key: string, f: (v: never) => string) => {
    expect((table[key] ?? []).length).toBeGreaterThan(0);
    for (const c of table[key] ?? []) expect(f(c.in as never), ).toBe(c.out);
  };

  it('formats every case exactly as the Rust side does', () => {
    each('bytes', (v: number) => formatBytes(v));
    each('durationMs', (v: number) => formatDuration(v));
    each('count', (v: number) => formatCount(v));
    each('percent', (v: [number, number]) => formatPercent(v[0], v[1]));
  });
});
