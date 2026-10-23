import { generateDemo } from './mock-data';
import { LONG_FILE_NAME, LONG_POSIX_PATH, LONG_WINDOWS_PATH, withLongNames } from './mock-long-names';

describe('withLongNames', () => {
  it('adds very long paths and names to the demo without changing its records', () => {
    const demo = generateDemo();
    const long = withLongNames(demo);
    expect(LONG_WINDOWS_PATH.length).toBeGreaterThan(200);
    expect(long.records).toBe(demo.records);
    expect(long.sources[0]?.path).toBe(LONG_WINDOWS_PATH);
    expect(long.sources[0]?.name).toBe('CompanyA.Graphic.Builder.Service.Diagnostics.2026-10-23.log');
    expect(long.sources[1]?.path).toBe(LONG_POSIX_PATH);
    expect(long.sources.length).toBe(demo.sources.length);
    expect(long.unrecognized).toHaveLength(demo.unrecognized.length + 3);
    expect(long.unrecognized.some((u) => u.name === `${LONG_FILE_NAME}.bak`)).toBe(true);
  });
});
