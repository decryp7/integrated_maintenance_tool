import type { MockNewNotice } from '../../notifications/mock-notifications';
import { TimestampIssue } from '../api.types';
import { MockRecord, MockSource } from './mock-data';

/** Opened paths with this in their name stand for a file whose timestamps the engine cannot read. */
export const MOCK_NO_TIMESTAMPS = 'no-timestamps';

const CTIME_FORMAT = '%a %b %d %H:%M:%S %Y';

/**
 * Mimics the engine's timestamp diagnosis (`debrief_core::timestamp_issue`) for the browser demo: a path naming
 * `no-timestamps` imports as a CSV whose events have no timestamp, with ctime-style examples and the format the
 * engine would propose. Mock only; the engine decides this for real.
 */
export function withTimestampIssue(path: string, source: MockSource, records: MockRecord[]): { source: MockSource; records: MockRecord[] } {
  if (!path.toLowerCase().includes(MOCK_NO_TIMESTAMPS)) return { source, records };
  const examples = ['Sat Nov 08 00:04:37 2025,INFO,Sync started', 'Sat Nov 08 00:04:38 2025,WARN,Slow page 3', 'Sat Nov 08 00:04:39 2025,INFO,Page 4'];
  const issue: TimestampIssue = {
    unparsedLines: records.length,
    totalLines: records.length,
    examples,
    exampleLines: [2, 3, 4],
    suggestion: {
      format: CTIME_FORMAT,
      sample: 'Sat Nov 08 00:04:37 2025',
      parserSnippet: `[format]\ntimestamp_formats = ["${CTIME_FORMAT}"]\n`,
      parsedSamples: examples.length,
      checkedSamples: examples.length,
    },
  };
  return { source: { ...source, importer: 'csv', timestampIssue: issue }, records: records.map((r) => ({ ...r, ts: null })) };
}

/** The message the engine posts for such a source (`workspace_commands::timestamp_notice`). */
export function timestampNotice(source: MockSource): MockNewNotice | null {
  const issue = source.timestampIssue;
  if (!issue) return null;
  const format = issue.suggestion?.format ?? null;
  const body = `${issue.unparsedLines} of ${issue.totalLines} events have no readable timestamp, so they are missing from the timeline.${format ? ` The format ${format} reads them.` : ''}`;
  return {
    source: 'import',
    severity: 'warning',
    title: `Timestamps not recognised in ${source.name}`,
    body,
    primary: { type: 'fixTimestamps', path: source.path, format },
    dedupeKey: `import:timestamps:${source.path}`,
    once: true,
    link: { path: source.path },
  };
}
