import {
  clamp,
  formatCompact,
  formatCount,
  formatDateTime,
  formatDuration,
  formatFull,
  formatNumber,
  formatTime,
  parseBucket,
  timeTicks,
  tzLabel,
  valueToString,
} from './format';

const TS = Date.UTC(2026, 9, 14, 10, 41, 22, 318);

describe('format', () => {
  it('formats times in UTC', () => {
    expect(formatTime(TS, 'utc')).toBe('10:41:22.318');
    expect(formatTime(null, 'utc')).toBe('—');
    expect(formatDateTime(TS, 'utc')).toBe('14 Oct 10:41:22');
    expect(formatDateTime(TS, 'utc', true)).toBe('14 Oct 10:41:22.318');
    expect(formatDateTime(null, 'utc')).toBe('—');
    expect(formatFull(TS, 'utc')).toBe('14 Oct 2026 10:41:22.318 UTC');
    expect(formatFull(null, 'utc')).toBe('No timestamp');
  });

  it('formats times in the local zone', () => {
    const d = new Date(TS);
    expect(formatTime(TS, 'local')).toBe(
      `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}:${String(d.getSeconds()).padStart(2, '0')}.318`,
    );
    expect(formatFull(TS, 'local')).toContain(tzLabel('local'));
  });

  it('labels time zones', () => {
    expect(tzLabel('utc')).toBe('UTC');
    expect(tzLabel('local')).toMatch(/^UTC[+-]\d{2}:\d{2}$/);
  });

  it('formats counts and numbers', () => {
    expect(formatCount(1234567)).toBe('1,234,567');
    expect(formatCompact(999)).toBe('999');
    expect(formatCompact(1234)).toBe('1.2k');
    expect(formatCompact(12345)).toBe('12k');
    expect(formatCompact(2_500_000)).toBe('2.5M');
    expect(formatNumber(2_500_000)).toBe('2.5M');
    expect(formatNumber(25_000)).toBe('25k');
    expect(formatNumber(250.4)).toBe('250');
    expect(formatNumber(25.44)).toBe('25.4');
    expect(formatNumber(2.5)).toBe('2.5');
    expect(formatNumber(2)).toBe('2');
    expect(formatNumber(0)).toBe('0');
  });

  it('formats durations', () => {
    expect(formatDuration(500)).toBe('500 ms');
    expect(formatDuration(1500)).toBe('1.5 s');
    expect(formatDuration(90_000)).toBe('1.5 min');
    expect(formatDuration(5_400_000)).toBe('1.5 h');
    expect(formatDuration(2 * 86_400_000)).toBe('2 d');
  });

  it('generates aligned time ticks', () => {
    const from = Date.UTC(2026, 9, 14, 9, 0, 0);
    const to = Date.UTC(2026, 9, 14, 12, 0, 0);
    const ticks = timeTicks(from, to, 7, 'utc');
    expect(ticks.map((t) => t.label)).toEqual(['09:00', '09:30', '10:00', '10:30', '11:00', '11:30', '12:00']);
    expect(ticks[0].frac).toBe(0);
    expect(ticks[6].frac).toBe(1);
  });

  it('chooses tick precision from the span', () => {
    const from = Date.UTC(2026, 9, 14, 10, 0, 0);
    expect(timeTicks(from, from + 10_000, 5, 'utc')[1].label).toMatch(/^10:00:0\d$/);
    expect(timeTicks(from, from + 400, 4, 'utc')[1].label).toMatch(/^\d{2}\.\d{3}$/);
    expect(timeTicks(from, from + 5 * 86_400_000, 5, 'utc')[0].label).toMatch(/^\d+ Oct$/);
    expect(timeTicks(Date.UTC(2026, 9, 14, 23, 0), Date.UTC(2026, 9, 15, 1, 0), 4, 'utc').map((t) => t.label)).toContain('15 Oct');
    expect(timeTicks(from, from, 5, 'utc')).toEqual([]);
    expect(timeTicks(from, from + 1000, 0, 'utc')).toEqual([]);
    expect(timeTicks(from, from + 3_600_000, 6, 'local').length).toBeGreaterThan(0);
  });

  it('parses bucket durations', () => {
    expect(parseBucket('30s')).toBe(30_000);
    expect(parseBucket('1m')).toBe(60_000);
    expect(parseBucket('2h')).toBe(7_200_000);
    expect(parseBucket('1d')).toBe(86_400_000);
    expect(parseBucket('250ms')).toBe(250);
    expect(parseBucket('auto')).toBeNull();
  });

  it('clamps and stringifies', () => {
    expect(clamp(5, 0, 3)).toBe(3);
    expect(clamp(-1, 0, 3)).toBe(0);
    expect(valueToString(null)).toBe('');
    expect(valueToString(undefined)).toBe('');
    expect(valueToString('x')).toBe('x');
    expect(valueToString(42)).toBe('42');
    expect(valueToString({ a: 1 })).toBe('{"a":1}');
  });
});
