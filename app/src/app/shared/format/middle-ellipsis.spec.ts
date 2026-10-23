import { DEFAULT_KEEP_TAIL, ELLIPSIS, middleEllipsis, splitForEllipsis } from './middle-ellipsis';

const WINDOWS = 'C:\\ProgramData\\VeryLongVendorName\\CompanyA.Graphic.Builder.Service.Diagnostics.2026-10-23.log';
const POSIX = '/var/lib/very-long-vendor-name/company-a/builder/service.diagnostics.2026-10-23.log';

describe('middleEllipsis', () => {
  it('returns text that fits unchanged', () => {
    expect(middleEllipsis('app.log', 20)).toBe('app.log');
    expect(middleEllipsis('abcde', 5)).toBe('abcde');
    expect(middleEllipsis('', 5)).toBe('');
  });

  it('keeps the start and the file name of a Windows path', () => {
    const path = 'C:\\ProgramData\\VeryLongVendorName\\Builder.Service.Diagnostics.log';
    const out = middleEllipsis(path, 55);
    expect(out).toBe('C:\\ProgramData\\…\\Builder.Service.Diagnostics.log');
    expect(middleEllipsis(path, 45)).toBe('C:\\ProgramDa…\\Builder.Service.Diagnostics.log');
    expect(middleEllipsis(path, path.length)).toBe(path);
    expect(Array.from(middleEllipsis(path, 40)).length).toBeLessThanOrEqual(40);
  });

  it('keeps the whole file name of a POSIX path when it fits', () => {
    const out = middleEllipsis(POSIX, 50);
    expect(out).toBe('/var/lib/…/service.diagnostics.2026-10-23.log');
    expect(Array.from(middleEllipsis(POSIX, 40)).length).toBeLessThanOrEqual(40);
  });

  it('cuts text with no separator in the middle, keeping both ends', () => {
    const out = middleEllipsis('CompanyA.Graphic.Builder.Service.Diagnostics.log', 21);
    expect(out).toBe('CompanyA.G…ostics.log');
    expect(Array.from(out)).toHaveLength(21);
    expect(middleEllipsis('abcdefghij', 6)).toBe('abc…ij');
  });

  it('falls back to a middle cut when the file name alone is too long', () => {
    const out = middleEllipsis(`/data/${'x'.repeat(80)}.log`, 20);
    expect(Array.from(out)).toHaveLength(20);
    expect(out.endsWith('.log')).toBe(true);
    expect(out).toContain(ELLIPSIS);
  });

  it('handles very short widths', () => {
    expect(middleEllipsis('abcdef', 0)).toBe('');
    expect(middleEllipsis('abcdef', -3)).toBe('');
    expect(middleEllipsis('abcdef', Number.NaN)).toBe('');
    expect(middleEllipsis('abcdef', 1)).toBe(ELLIPSIS);
    expect(middleEllipsis('abcdef', 2)).toBe(`a${ELLIPSIS}`);
    expect(middleEllipsis('abcdef', 3)).toBe(`a${ELLIPSIS}f`);
    expect(middleEllipsis('abcdef', 2.9)).toBe(`a${ELLIPSIS}`);
  });

  it('never splits a surrogate pair', () => {
    const text = '😀'.repeat(10);
    const out = middleEllipsis(text, 5);
    expect(out).toBe(`😀😀${ELLIPSIS}😀😀`);
    expect(Array.from(middleEllipsis(`${text}\\file😀.log`, 9)).every((c) => c.length === 1 || c.length === 2)).toBe(true);
    expect(middleEllipsis('😀😀', 2)).toBe('😀😀');
  });

  it('never exceeds the width, whatever the input', () => {
    for (const text of [WINDOWS, POSIX, 'a'.repeat(300), '\\\\server\\share\\x', '/', '\\', 'a/b']) {
      for (let max = 0; max < 40; max++) expect(Array.from(middleEllipsis(text, max)).length).toBeLessThanOrEqual(Math.max(max, 0));
    }
  });
});

describe('splitForEllipsis', () => {
  it('keeps short text whole in the tail', () => {
    expect(splitForEllipsis('app.log')).toEqual({ head: '', tail: 'app.log' });
    expect(splitForEllipsis('')).toEqual({ head: '', tail: '' });
  });

  it('keeps the separator and file name of a path as the tail', () => {
    expect(splitForEllipsis(WINDOWS, 60)).toEqual({ head: 'C:\\ProgramData\\VeryLongVendorName', tail: '\\CompanyA.Graphic.Builder.Service.Diagnostics.2026-10-23.log' });
    expect(splitForEllipsis(POSIX, 60).tail).toBe('/service.diagnostics.2026-10-23.log');
  });

  it('keeps the last characters of a name with no separator or a name that is too long', () => {
    const name = 'CompanyA.Graphic.Builder.Service.Diagnostics.log';
    const parts = splitForEllipsis(name);
    expect(parts.tail).toBe(name.slice(-DEFAULT_KEEP_TAIL));
    expect(parts.head + parts.tail).toBe(name);
    const long = `/data/${'x'.repeat(80)}.log`;
    expect(splitForEllipsis(long, 10).tail).toHaveLength(10);
  });

  it('counts code points and treats a negative tail as no tail', () => {
    const text = '😀'.repeat(10);
    expect(splitForEllipsis(text, 3)).toEqual({ head: '😀'.repeat(7), tail: '😀'.repeat(3) });
    expect(splitForEllipsis('abc', -1)).toEqual({ head: 'abc', tail: '' });
  });
});
