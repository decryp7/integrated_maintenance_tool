import { TestBed } from '@angular/core/testing';
import { all, el, provideTinyBackend, settle } from '../../testing/helpers';
import { SourceInfo, TimestampIssue } from '../core/api.types';
import { FormatWizard } from '../format/format-wizard';
import { DialogService } from '../ui/dialog.service';
import { TimestampWarning, timestampWarningText } from './timestamp-warning';

const ISSUE: TimestampIssue = {
  unparsedLines: 1204,
  totalLines: 1300,
  examples: ['Sat Nov 08 00:04:37 2025,INFO,started'],
  exampleLines: [2],
  suggestion: { format: '%a %b %d %H:%M:%S %Y', sample: 'Sat Nov 08 00:04:37 2025', parserSnippet: '', parsedSamples: 20, checkedSamples: 20 },
};

function source(timestampIssue: TimestampIssue | null | undefined): SourceInfo {
  return { id: 3, path: '/logs/sync.csv', name: 'sync.csv', group: 'sync.csv', importer: 'csv', fileKind: 'text', records: 1300, dropped: 0, skipped: 0, error: null, from: null, to: null, timestampIssue };
}

describe('timestampWarningText', () => {
  it('counts the lines and quotes the first example, cut to 60 characters', () => {
    expect(timestampWarningText(ISSUE)).toBe('Timestamps not recognised in 1,204 lines (e.g. “Sat Nov 08 00:04:37 2025,INFO,started”)');
    const long = timestampWarningText({ ...ISSUE, examples: ['x'.repeat(80)] });
    expect(long).toContain(`“${'x'.repeat(60)}…”`);
    expect(timestampWarningText({ ...ISSUE, examples: [] })).toBe('Timestamps not recognised in 1,204 lines');
  });
});

describe('TimestampWarning', () => {
  beforeEach(() => TestBed.configureTestingModule({ providers: provideTinyBackend() }));

  async function mount(s: SourceInfo) {
    const fixture = TestBed.createComponent(TimestampWarning);
    fixture.componentRef.setInput('source', s);
    await settle(fixture);
    return fixture;
  }

  it('shows nothing when the timestamps are fine', async () => {
    expect(all(await mount(source(null)), 'button')).toHaveLength(0);
    expect(all(await mount(source(undefined)), 'button')).toHaveLength(0);
  });

  it('is a labelled button that opens the format wizard on the proposed format', async () => {
    const fixture = await mount(source(ISSUE));
    const button = el<HTMLButtonElement>(fixture, '[data-testid=timestamp-warning]');
    expect(button.getAttribute('aria-label')).toBe(`${timestampWarningText(ISSUE)}. Click to fix the timestamp format.`);
    expect(button.title).toBe(button.getAttribute('aria-label'));
    button.click();
    await vi.waitFor(() => expect(TestBed.inject(DialogService).stack()).toHaveLength(1));
    const [open] = TestBed.inject(DialogService).stack();
    expect(open?.options.title).toBe('Fix timestamps');
    expect(open?.options.inputs).toEqual({ path: '/logs/sync.csv', name: 'sync.csv', timestampFormat: '%a %b %d %H:%M:%S %Y' });
    expect(open?.component).toBe(FormatWizard);
  });

  it('opens the wizard empty when the engine had no format to propose', async () => {
    const fixture = await mount(source({ ...ISSUE, suggestion: null }));
    el<HTMLButtonElement>(fixture, '[data-testid=timestamp-warning]').click();
    await vi.waitFor(() => expect(TestBed.inject(DialogService).stack()).toHaveLength(1));
    expect(TestBed.inject(DialogService).stack()[0]?.options.inputs).toMatchObject({ timestampFormat: null });
  });
});
