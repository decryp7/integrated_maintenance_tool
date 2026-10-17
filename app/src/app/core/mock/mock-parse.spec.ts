import { dateFormatPattern, mockLevel, mockPreviewParse, mockTimestamp, regexFromOutputTemplate } from './mock-parse';
import { toJsRegex } from '../profile.model';

const LINE = '2026-10-14 10:41:02.114 +00:00 [INF] Order 1042 accepted';
const TEMPLATE = '{Timestamp:yyyy-MM-dd HH:mm:ss.fff zzz} [{Level:u3}] {Message:lj}{NewLine}{Exception}';
const SAMPLE = [LINE, '2026-10-14 10:41:03.201 +00:00 [ERR] Capture failed', '   at Payments.Capture()', '  at Other()', '', '2026-10-14 10:41:04.007 +00:00 [WRN] Slow'].join('\n');

const req = (toml: string, sample = SAMPLE, limit = 50) => ({ toml, sample, fileName: 'x.log', limit });

describe('regexFromOutputTemplate (mock only)', () => {
  it('converts a Serilog template', () => {
    expect(toJsRegex(regexFromOutputTemplate(TEMPLATE)!).exec(LINE)?.groups).toMatchObject({ level: 'INF', message: 'Order 1042 accepted' });
  });

  it('handles other holes, long levels and templates without a pattern', () => {
    const regex = regexFromOutputTemplate('{Timestamp} {Level} <{SourceContext}> {Message}{Properties:j}')!;
    expect(toJsRegex(regex).exec('2026-01-01 Information <Web> hi')?.groups).toMatchObject({ level: 'Information', message: 'hi' });
    expect(regexFromOutputTemplate('{Level} only')).toBeNull();
    expect(regexFromOutputTemplate('{Timestamp} no message')).toBeNull();
  });

  it('maps .NET date tokens', () => {
    expect(dateFormatPattern('yy-M-d h:mm tt')).toBe('\\d{2}-\\d{1,2}-\\d{1,2} \\d{1,2}:\\d{2} [AP]M');
    expect(dateFormatPattern('FFF zz')).toBe('\\d{3} [+-]\\d{2}(?::\\d{2})?');
  });
});

describe('mock levels and timestamps', () => {
  it('maps level words', () => {
    expect(['VRB', 'dbg', 'WRN', 'ERR', 'FTL', 'INF', undefined].map(mockLevel)).toEqual(['Verbose', 'Debug', 'Warning', 'Error', 'Fatal', 'Information', 'Information']);
  });

  it('parses timestamps with and without a zone', () => {
    expect(mockTimestamp('2026-10-14 10:41:02.114 +00:00')).toBe(Date.UTC(2026, 9, 14, 10, 41, 2, 114));
    expect(mockTimestamp('2026-10-14 10:41:02')).toBe(Date.UTC(2026, 9, 14, 10, 41, 2));
    expect(mockTimestamp('nonsense')).toBeNull();
    expect(mockTimestamp(undefined)).toBeNull();
  });
});

describe('mockPreviewParse', () => {
  it('parses lines with the profile regexes, attaching continuation lines', () => {
    const toml = `[parse]\nregexes = ['^(?P<timestamp>\\d{4}-\\d{2}-\\d{2} \\S+ \\S+) \\[(?P<level>\\w+)\\] (?P<message>.*)$']\n`;
    const r = mockPreviewParse(req(toml));
    expect(r).toMatchObject({ importer: 'text', total: 3, skipped: 0, dropped: 0, error: null });
    expect(r.records[1]).toMatchObject({ line: 2, level: 'Error', message: 'Capture failed', exception: '   at Payments.Capture()\n  at Other()' });
    expect(r.records[2]).toMatchObject({ line: 6, level: 'Warning' });
  });

  it('parses with output templates and honours the limit', () => {
    const r = mockPreviewParse(req(`[parse]\noutput_templates = ["${TEMPLATE.replace(/"/g, '\\"')}"]\n`, SAMPLE, 1));
    expect(r.total).toBe(3);
    expect(r.records).toHaveLength(1);
  });

  it('reports lines before the first event as skipped, and nothing parsed without patterns', () => {
    const r = mockPreviewParse(req(`[parse]\noutput_templates = ["${TEMPLATE.replace(/"/g, '\\"')}"]\n`, `junk\n${LINE}`));
    expect(r).toMatchObject({ total: 1, skipped: 1 });
    expect(mockPreviewParse(req('name = "x"', 'a\nb'))).toMatchObject({ importer: null, total: 0, skipped: 2, error: null });
  });

  it('reports invalid TOML and invalid regexes', () => {
    expect(mockPreviewParse(req('= nope')).error).toBeTruthy();
    expect(mockPreviewParse(req(`[parse]\nregexes = ['(']\n`)).error).toMatch(/Invalid regex/);
  });
});
