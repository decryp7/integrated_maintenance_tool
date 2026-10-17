import { toJsRegex } from '../core/profile.model';
import { regexFromMarks, timestampPattern } from './format-parse';

/** Applies a generated pattern the way the engine would (the Rust regex dialect, translated for JS). */
function groups(line: string, regex: string): Record<string, string> | null {
  return toJsRegex(regex).exec(line)?.groups ?? null;
}

const LINE = '2026-10-14 10:41:02.114 +00:00 [INF] Order 1042 accepted';

function marks(line = LINE) {
  const ts = '2026-10-14 10:41:02.114 +00:00';
  const lvl = 'INF';
  const msg = 'Order 1042 accepted';
  return {
    timestamp: { start: line.indexOf(ts), end: line.indexOf(ts) + ts.length },
    level: { start: line.indexOf(lvl), end: line.indexOf(lvl) + lvl.length },
    message: { start: line.indexOf(msg), end: line.indexOf(msg) + msg.length },
  };
}

describe('regexFromMarks', () => {
  it('builds a regex that parses the sample and similar lines', () => {
    const regex = regexFromMarks(LINE, marks())!;
    expect(regex).toBe('^(?P<timestamp>\\d{4}-\\d{2}-\\d{2} \\d{2}:\\d{2}:\\d{2}\\.\\d{3} \\+\\d{2}:\\d{2}) \\[(?P<level>[A-Za-z]+)\\] (?P<message>.*)');
    expect(groups(LINE, regex)).toMatchObject({ timestamp: '2026-10-14 10:41:02.114 +00:00', level: 'INF', message: 'Order 1042 accepted' });
    expect(groups('2026-10-15 01:02:03.004 +00:00 [ERR] Boom', regex)).toMatchObject({ level: 'ERR', message: 'Boom' });
    expect(groups('   at Foo()', regex)).toBeNull();
  });

  it('works without a level and with trailing literal text', () => {
    const line = '[10:41:02] hello (end)';
    const regex = regexFromMarks(line, { timestamp: { start: 1, end: 9 }, message: { start: 11, end: 16 } })!;
    expect(regex).toBe('^\\[(?P<timestamp>\\d{2}:\\d{2}:\\d{2})\\] (?P<message>.+?)\\ \\(end\\)$'.replace('\\ ', ' '));
    expect(groups(line, regex)).toMatchObject({ message: 'hello' });
  });

  it('uses \\w+ for non-alphabetic levels', () => {
    const line = '1 2 L1 msg';
    const regex = regexFromMarks(line, { timestamp: { start: 0, end: 1 }, level: { start: 4, end: 6 }, message: { start: 7, end: 10 } })!;
    expect(regex).toContain('(?P<level>\\w+)');
  });

  it('rejects incomplete, overlapping or out-of-range marks', () => {
    expect(regexFromMarks(LINE, {})).toBeNull();
    expect(regexFromMarks(LINE, { timestamp: { start: 0, end: 4 } })).toBeNull();
    expect(regexFromMarks(LINE, { timestamp: { start: 0, end: 10 }, message: { start: 5, end: 20 } })).toBeNull();
    expect(regexFromMarks('ab', { timestamp: { start: 0, end: 1 }, message: { start: 1, end: 9 } })).toBeNull();
    expect(regexFromMarks(LINE, { timestamp: { start: 3, end: 3 }, message: { start: 5, end: 9 } })).toBeNull();
  });

  it('describes timestamps by digit runs', () => {
    expect(timestampPattern('10:41')).toBe('\\d{2}:\\d{2}');
  });
});
