import { mockNotifications, MockNotifications, resetMockNotifications } from '../../notifications/mock-notifications';
import { MockBackend } from './mock-backend';
import { MockSource } from './mock-data';
import { timestampNotice, withTimestampIssue } from './mock-timestamps';

const SOURCE: MockSource = { id: 7, path: '/logs/legacy-no-timestamps.csv', name: 'legacy-no-timestamps.csv', importer: 'text', skipped: 0 };
const RECORD = { id: 0, ts: 1, level: 'Information' as const, message: 'm', template: null, exception: null, fields: {}, source: 7, line: 1, markers: [] };

describe('mock timestamp diagnosis', () => {
  beforeEach(() => resetMockNotifications());

  it('leaves other files alone', () => {
    const out = withTimestampIssue('/logs/app.log', { ...SOURCE, path: '/logs/app.log' }, [RECORD]);
    expect(out.source.timestampIssue).toBeUndefined();
    expect(out.records[0]?.ts).toBe(1);
    expect(timestampNotice(out.source)).toBeNull();
  });

  it('flags a "no-timestamps" file: no timestamps, examples and a proposed format', () => {
    const out = withTimestampIssue(SOURCE.path, SOURCE, [RECORD, { ...RECORD, line: 2 }]);
    expect(out.source.importer).toBe('csv');
    expect(out.records.every((r) => r.ts === null)).toBe(true);
    expect(out.source.timestampIssue).toMatchObject({ unparsedLines: 2, totalLines: 2, suggestion: { format: '%a %b %d %H:%M:%S %Y' } });
    expect(out.source.timestampIssue?.examples).toHaveLength(3);
    expect(out.source.timestampIssue?.exampleLines).toEqual([2, 3, 4]);
    expect(out.source.timestampIssue?.suggestion).toMatchObject({ parsedSamples: 3, checkedSamples: 3 });
    const notice = timestampNotice(out.source)!;
    expect(notice).toMatchObject({ severity: 'warning', title: 'Timestamps not recognised in legacy-no-timestamps.csv', once: true });
    expect(notice.primary).toEqual({ type: 'fixTimestamps', path: SOURCE.path, format: '%a %b %d %H:%M:%S %Y' });
    expect(notice.body).toContain('The format %a %b %d %H:%M:%S %Y reads them.');
    const none = timestampNotice({ ...out.source, timestampIssue: { ...out.source.timestampIssue!, suggestion: null } })!;
    expect(none.primary).toMatchObject({ format: null });
    expect(none.body).not.toContain('The format');
  });

  it('a "once" message is posted once', () => {
    const centre = new MockNotifications();
    const notice = timestampNotice(withTimestampIssue(SOURCE.path, SOURCE, [RECORD]).source)!;
    const first = centre.post(notice);
    const again = centre.post(notice);
    expect(again).toBe(first);
    expect(again.count).toBe(1);
  });

  it('the mock backend shows the issue in the summary and posts one message per file', async () => {
    const backend = new MockBackend();
    const summary = await backend.openPaths(['/logs/legacy-no-timestamps.csv']);
    const s = summary.sources.find((x) => x.path === '/logs/legacy-no-timestamps.csv')!;
    expect(s.timestampIssue?.suggestion?.format).toBe('%a %b %d %H:%M:%S %Y');
    expect(s.from).toBeNull();
    expect(summary.sources.filter((x) => x.timestampIssue).length).toBe(1);
    await backend.openPaths(['/logs/legacy-no-timestamps.csv']);
    const { notices } = await mockNotifications().list();
    expect(notices.filter((n) => n.dedupeKey === 'import:timestamps:/logs/legacy-no-timestamps.csv')).toHaveLength(1);
  });
});
