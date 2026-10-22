import { formatOpenedAt } from './recent-date';

describe('formatOpenedAt', () => {
  const now = new Date(2026, 9, 22, 15, 0).getTime();

  it('shows the time for today and the date for earlier days', () => {
    expect(formatOpenedAt(new Date(2026, 9, 22, 9, 5).getTime(), now, 'en-GB')).toBe('09:05');
    expect(formatOpenedAt(new Date(2026, 9, 14, 9, 5).getTime(), now, 'en-GB')).toBe('14 Oct 2026');
    expect(formatOpenedAt(new Date(2025, 9, 22, 9, 5).getTime(), now, 'en-GB')).toBe('22 Oct 2025');
  });

  it('shows nothing for a missing or invalid time', () => {
    expect(formatOpenedAt(0, now)).toBe('');
    expect(formatOpenedAt(Number.NaN, now)).toBe('');
  });

  it('defaults to the real clock', () => {
    expect(formatOpenedAt(Date.now())).toMatch(/\d/);
  });
});
