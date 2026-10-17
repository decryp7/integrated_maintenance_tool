import { filterLines, LINE_HEIGHT, toLines, versionInfoText, VIEWPORT_HEIGHT, visibleRange } from './notices-text';

describe('notices text helpers', () => {
  it('splits lines with CRLF or LF', () => {
    expect(toLines('a\r\nb\nc')).toEqual(['a', 'b', 'c']);
  });

  it('filters case-insensitively and keeps everything for a blank query', () => {
    const lines = ['Serde 1.0', 'tokio 1.2', 'SERDE json'];
    expect(filterLines(lines, 'serde')).toEqual(['Serde 1.0', 'SERDE json']);
    expect(filterLines(lines, '   ')).toBe(lines);
    expect(filterLines(lines, 'nothing')).toEqual([]);
  });

  it('computes the rendered window with overscan and clamps to the list', () => {
    expect(visibleRange(0, 1000)).toEqual({ start: 0, end: Math.ceil(VIEWPORT_HEIGHT / LINE_HEIGHT) + 8 });
    const mid = visibleRange(LINE_HEIGHT * 100, 1000);
    expect(mid.start).toBe(92);
    expect(mid.end).toBe(100 + Math.ceil(VIEWPORT_HEIGHT / LINE_HEIGHT) + 8);
    expect(visibleRange(LINE_HEIGHT * 5000, 10)).toEqual({ start: 10, end: 10 });
    expect(visibleRange(-50, 3)).toEqual({ start: 0, end: 3 });
    expect(visibleRange(0, 0)).toEqual({ start: 0, end: 0 });
  });

  it('formats the version info, omitting what is unknown', () => {
    expect(versionInfoText('1.2.3', 'linux', { commit: 'abc123', date: '2026-01-01' })).toBe('IMT 1.2.3\nLicence: MIT OR Apache-2.0\nPlatform: linux\nCommit: abc123\nBuilt: 2026-01-01');
    expect(versionInfoText(null, null, null)).toBe('IMT unknown\nLicence: MIT OR Apache-2.0');
    expect(versionInfoText('1.0.0', null, { commit: 'unknown', date: 'unknown' })).toBe('IMT 1.0.0\nLicence: MIT OR Apache-2.0');
  });
});
