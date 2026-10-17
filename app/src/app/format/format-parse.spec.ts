import { dateFormatPattern, matchedCount, parseLines, regexFromMarks, regexFromOutputTemplate, timestampPattern } from './format-parse';
import { TestBed } from '@angular/core/testing';
import { FILE_PREVIEW, SampleFilePreview, TauriFilePreview } from './file-preview';

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
    const { rows, error } = parseLines([LINE, '2026-10-15 01:02:03.004 +00:00 [ERR] Boom', '   at Foo()'], regex);
    expect(error).toBeNull();
    expect(rows[0]).toMatchObject({ matched: true, timestamp: '2026-10-14 10:41:02.114 +00:00', level: 'INF', message: 'Order 1042 accepted' });
    expect(rows[1]).toMatchObject({ matched: true, level: 'ERR', message: 'Boom' });
    expect(rows[2].matched).toBe(false);
    expect(matchedCount(rows)).toBe(2);
  });

  it('works without a level and with trailing literal text', () => {
    const line = '[10:41:02] hello (end)';
    const regex = regexFromMarks(line, { timestamp: { start: 1, end: 9 }, message: { start: 11, end: 16 } })!;
    expect(regex).toBe('^\\[(?P<timestamp>\\d{2}:\\d{2}:\\d{2})\\] (?P<message>.+?)\\ \\(end\\)$'.replace('\\ ', ' '));
    expect(parseLines([line], regex).rows[0]).toMatchObject({ matched: true, message: 'hello' });
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

describe('regexFromOutputTemplate', () => {
  it('converts a Serilog template and parses with it', () => {
    const template = '{Timestamp:yyyy-MM-dd HH:mm:ss.fff zzz} [{Level:u3}] {Message:lj}{NewLine}{Exception}';
    const regex = regexFromOutputTemplate(template)!;
    expect(parseLines([LINE], regex).rows[0]).toMatchObject({ matched: true, level: 'INF', message: 'Order 1042 accepted' });
  });

  it('handles other holes, long levels and formats without a pattern', () => {
    const regex = regexFromOutputTemplate('{Timestamp} {Level} <{SourceContext}> {Message}{Properties:j}')!;
    expect(parseLines(['2026-01-01 Information <Web> hi'], regex).rows[0]).toMatchObject({ matched: true, level: 'Information', message: 'hi' });
    expect(regexFromOutputTemplate('{Level} only')).toBeNull();
    expect(regexFromOutputTemplate('{Timestamp} no message')).toBeNull();
  });

  it('maps .NET date tokens', () => {
    expect(dateFormatPattern('yy-M-d h:mm tt')).toBe('\\d{2}-\\d{1,2}-\\d{1,2} \\d{1,2}:\\d{2} [AP]M');
    expect(dateFormatPattern('FFF zz')).toBe('\\d{3} [+-]\\d{2}(?::\\d{2})?');
  });
});

describe('parseLines', () => {
  it('reports an invalid regex', () => {
    const { rows, error } = parseLines(['a'], '(');
    expect(error).toBeTruthy();
    expect(rows).toEqual([{ line: 'a', matched: false }]);
  });

  it('copes with a regex without named groups', () => {
    expect(parseLines(['a'], 'a').rows[0]).toMatchObject({ matched: true, timestamp: undefined });
  });
});

describe('file previews', () => {
  it('returns sample lines in the browser demo', async () => {
    const lines = await new SampleFilePreview().firstLines('/x.log', 2);
    expect(lines).toHaveLength(2);
    expect(lines[0]).toMatch(/\[INF\]/);
  });

  it('is not supported on the desktop yet', async () => {
    await expect(new TauriFilePreview().firstLines()).rejects.toThrow(/not supported yet/);
  });
});

describe('FILE_PREVIEW', () => {
  afterEach(() => {
    delete (window as unknown as Record<string, unknown>)['__TAURI_INTERNALS__'];
    TestBed.resetTestingModule();
  });

  it('uses sample lines in a browser and the desktop stub inside Tauri', () => {
    expect(TestBed.inject(FILE_PREVIEW)).toBeInstanceOf(SampleFilePreview);
    TestBed.resetTestingModule();
    (window as unknown as Record<string, unknown>)['__TAURI_INTERNALS__'] = {};
    expect(TestBed.inject(FILE_PREVIEW)).toBeInstanceOf(TauriFilePreview);
  });
});
