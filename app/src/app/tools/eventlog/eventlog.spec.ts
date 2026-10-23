import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, settle, text, typeInto } from '../../../testing/helpers';
import { at, must } from '../../../testing/must';
import { EventLogBackend } from '../../core/tools-backend';
import { EventLogRun } from '../../core/tools.types';
import { WorkspaceStore } from '../../core/workspace.store';
import { LifecycleService } from '../../lifecycle/lifecycle.service';
import { LiveTail } from '../../workspace/live-tail';
import { MOCK_DISABLED_CHANNEL } from '../mock/mock-event-log';
import { MockToolsOptions } from '../mock/mock-tools-backend';
import { mockTools, provideToolsTesting } from '../mock/test-support';
import { DEFAULT_EVENT_LOG_DRAFT, eventLogProblems, filterChannels, MAX_EVENT_CHANNELS, MAX_EVENT_XPATH, toEventLogQuery, toEventLogStart, toggled } from './eventlog-model';
import { EVENT_LOG_POLL_MS, EventLogView } from './eventlog-view';
import { EventLogStore } from './eventlog.store';

async function setup(options: Partial<MockToolsOptions> = {}) {
  TestBed.configureTestingModule({ providers: [...provideToolsTesting(options), { provide: EVENT_LOG_POLL_MS, useValue: 5 }] });
  await TestBed.inject(LifecycleService).init();
  const fixture = TestBed.createComponent(EventLogView);
  await settle(fixture);
  return { fixture, store: TestBed.inject(EventLogStore), tools: mockTools(), workspace: TestBed.inject(WorkspaceStore) };
}

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

describe('event log model', () => {
  it('reports what blocks a run', () => {
    expect(eventLogProblems(DEFAULT_EVENT_LOG_DRAFT)).toEqual(['Choose at least one channel to follow.']);
    const many = Array.from({ length: MAX_EVENT_CHANNELS + 1 }, (_, i) => `c${i}`);
    expect(eventLogProblems({ ...DEFAULT_EVENT_LOG_DRAFT, channels: many, levels: [], xpath: 'x'.repeat(MAX_EVENT_XPATH + 1) })).toEqual([
      `Follow at most ${MAX_EVENT_CHANNELS} channels at once.`,
      'Choose at least one level.',
      `The XPath filter is longer than ${MAX_EVENT_XPATH} characters.`,
    ]);
  });

  it('builds the validate and start arguments', () => {
    const d = { ...DEFAULT_EVENT_LOG_DRAFT, channels: ['System'], levels: ['error' as const], xpath: '  ', backfill: '1h' as const };
    expect(toEventLogQuery(d)).toEqual({ channels: ['System'], levels: ['error'], backfill: '1h' });
    expect(toEventLogQuery({ ...d, xpath: ' *[System[EventID=1]] ' }).xpath).toBe('*[System[EventID=1]]');
    expect(toEventLogStart(d)).toEqual({ channels: ['System'], levels: ['error'], backfill: '1h', resumeFromBookmark: true });
    expect(toEventLogStart({ ...d, resume: false, dir: ' /o ' })).toMatchObject({ resumeFromBookmark: false, dir: '/o' });
  });

  it('filters channels and toggles entries', () => {
    const channels = [
      { name: 'System', needsAdmin: false },
      { name: 'Microsoft-Windows-PowerShell/Operational', needsAdmin: false },
    ];
    expect(filterChannels(channels, ' power ').map((c) => c.name)).toEqual(['Microsoft-Windows-PowerShell/Operational']);
    expect(filterChannels(channels, '')).toHaveLength(2);
    expect(toggled(['a', 'b'], 'a', true)).toEqual(['b', 'a']);
    expect(toggled(['a', 'b'], 'a', false)).toEqual(['b']);
  });
});

describe('EventLogStore', () => {
  it('loads the channels and the status', async () => {
    const { store } = await setup();
    expect(store.channels().length).toBeGreaterThan(5);
    expect(store.running()).toBe(false);
    expect(store.canRun()).toBe(true);
    expect(store.unsupported()).toBeNull();
    store.setSearch('sec');
    expect(store.visibleChannels().map((c) => c.name)).toEqual(['Security']);
  });

  it('validates, clears the result on edit, and starts: opens the folder and turns live tail on', async () => {
    const { store, tools, workspace } = await setup();
    store.toggleChannel('System', true);
    store.toggleChannel(MOCK_DISABLED_CHANNEL, true);
    store.toggleLevel('verbose', true);
    store.patchDraft({ backfill: '24h' });
    await store.validate();
    expect(store.validation()?.channels.map((c) => c.ok)).toEqual([true, false]);
    expect(store.validation()?.backfillXpath).toContain('86400000');
    store.toggleChannel(MOCK_DISABLED_CHANNEL, false);
    expect(store.validation()).toBeNull();

    const sources = workspace.summary()?.sources.length ?? 0;
    await store.start();
    expect(store.error()).toBeNull();
    expect(store.running()).toBe(true);
    expect(store.dir()).toBe('/demo/eventlog/run-1');
    expect(tools.calls).toEqual(['eventLogStart:/demo/eventlog/run-1']);
    expect(TestBed.inject(LiveTail).enabled()).toBe(true);
    expect(workspace.summary()?.sources.length).toBe(sources + 1);
    expect(store.notice()).toContain('Following started');
    const before = store.run()?.events ?? 0;
    await store.poll();
    expect(store.run()?.events).toBeGreaterThan(before);

    await store.pause();
    expect(store.paused()).toBe(true);
    await store.resume();
    expect(store.paused()).toBe(false);
    await store.stop();
    expect(store.running()).toBe(false);
    expect(store.run()?.events).toBeGreaterThan(0);
    expect(store.notice()).toContain('positions are saved');

    // The next run continues where this one stopped.
    await store.start();
    expect(store.resumed()).toEqual(['System']);
    expect(store.notice()).toContain('Continuing from the last position of System');
  });

  it('shows why Security cannot be read and which chosen channels need administrator rights', async () => {
    const { store } = await setup();
    store.toggleChannel('Security', true);
    expect(store.needsAdmin()).toEqual(['Security']);
    await store.start();
    expect(store.error()).toContain('Access is denied');
    expect(store.running()).toBe(false);
    store.dismissError();
    expect(store.error()).toBeNull();
  });

  it('needs no administrator note when elevated', async () => {
    const { store } = await setup({ elevated: true });
    store.toggleChannel('Security', true);
    expect(store.needsAdmin()).toEqual([]);
  });

  it('keeps the previous folder when stop finds nothing and reports polling failures', async () => {
    const { store } = await setup();
    await store.stop();
    expect(store.dir()).toBeNull();
    vi.spyOn(TestBed.inject(EventLogBackend), 'eventLogStatus').mockRejectedValueOnce(new Error('tool timed out'));
    await store.poll();
    expect(store.error()).toBe('tool timed out');
  });

  it('does not ask for channels where the follower is unsupported', async () => {
    const { store, fixture } = await setup({ platform: 'macos' });
    expect(store.canRun()).toBe(false);
    expect(store.channels()).toEqual([]);
    expect(store.error()).toBeNull();
    expect(el(fixture, '[data-testid=eventlog-unsupported]').textContent).toContain('only available on Windows');
  });
});

describe('EventLogView', () => {
  it('picks channels, validates, starts, polls the status and stops', async () => {
    const { fixture } = await setup();
    expect(el(fixture, '[data-testid=eventlog-status]').textContent).toContain('Stopped');
    expect(buttonByText(fixture, 'Start').disabled).toBe(true);
    // No validation message before the user has touched the form.
    expect(text(fixture)).not.toContain('Choose at least one channel');

    typeInto(el(fixture, 'input[aria-label="Search channels"]'), 'zzz');
    await settle(fixture);
    expect(text(fixture)).toContain('No channel matches the search.');
    expect(text(fixture)).toContain('Choose at least one channel');
    typeInto(el(fixture, 'input[aria-label="Search channels"]'), '');
    await settle(fixture);
    const channel = (name: string) => must(all<HTMLLabelElement>(fixture, '.picker label').find((l) => l.textContent?.includes(name))).querySelector('input') as HTMLInputElement;
    channel('System').click();
    channel('Application').click();
    await settle(fixture);
    expect(el(fixture, '[data-testid=eventlog-selected]').textContent).toContain('2 selected');
    expect(text(fixture)).not.toContain('Choose at least one channel');

    const levels = all<HTMLInputElement>(fixture, '[aria-label=Levels] input');
    at(levels, 4).click();
    typeInto(el(fixture, 'textarea'), '*[System[EventID=7036]]');
    typeInto(el(fixture, 'select'), '1h', 'change');
    await settle(fixture);
    buttonByText(fixture, 'Validate').click();
    await settle(fixture);
    const validation = el(fixture, '[data-testid=eventlog-validation]').textContent ?? '';
    expect(validation).toContain('Live query: *[(*[System[EventID=7036]])]');
    expect(validation).toContain('Backfill query:');
    expect(validation).toContain('System OK');

    el<HTMLInputElement>(fixture, 'debrief-switch input').click();
    typeInto(at(all<HTMLInputElement>(fixture, 'input.mono'), 0), '/out');
    await settle(fixture);
    buttonByText(fixture, 'Start').click();
    await settle(fixture);
    expect(el(fixture, '[data-testid=eventlog-status]').textContent).toContain('Running');
    expect(el(fixture, '[data-testid=eventlog-status]').textContent).toContain('/out');
    const events = () => Number(el(fixture, '[data-testid=eventlog-events]').textContent?.replace(/,/g, ''));
    const first = events();
    await wait(30);
    await settle(fixture);
    expect(events()).toBeGreaterThan(first);
    expect(el(fixture, '[data-testid=eventlog-last]').textContent).toContain('Application');
    expect(text(fixture)).toContain('Live');

    buttonByText(fixture, 'Pause').click();
    await settle(fixture);
    expect(el(fixture, '[data-testid=eventlog-status]').textContent).toContain('Paused');
    buttonByText(fixture, 'Resume').click();
    await settle(fixture);
    buttonByText(fixture, 'Stop').click();
    await settle(fixture);
    expect(el(fixture, '[data-testid=eventlog-status]').textContent).toContain('Stopped');
    el<HTMLButtonElement>(fixture, 'button[aria-label="Refresh status"]').click();
    await settle(fixture);
  });

  it('shows retrying channels with their errors', async () => {
    const { fixture, store } = await setup();
    store.toggleChannel(MOCK_DISABLED_CHANNEL, true);
    await store.start();
    await settle(fixture);
    expect(el(fixture, '[data-testid=eventlog-progress]').textContent).toContain('Retrying');
    expect(el(fixture, '[data-testid=eventlog-errors]').textContent).toContain('disabled');
    expect(text(fixture)).toContain('No events written yet.');
    await store.stop();
  });

  it('offers Restart as administrator for Security, before and after a denied start', async () => {
    const { fixture, store, tools } = await setup();
    store.toggleChannel('Security', true);
    await settle(fixture);
    expect(el(fixture, '[data-testid=eventlog-admin-note]').textContent).toContain('Security needs administrator rights');
    buttonByText(fixture, 'Start').click();
    await settle(fixture);
    expect(el(fixture, '[role=alert]').textContent).toContain('Access is denied');
    const restart = all<HTMLButtonElement>(fixture, 'button').filter((b) => b.textContent?.includes('Restart as administrator'));
    expect(restart).toHaveLength(2);
    at(restart, 0).click();
    await settle(fixture);
    expect(tools.calls.some((c) => c.startsWith('restartElevated'))).toBe(true);
    buttonByText(fixture, 'Dismiss').click();
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('[role=alert]')).toBeNull();
  });

  it('shows the last event without an id and the clear button', async () => {
    const { fixture, store } = await setup();
    store.toggleChannel('System', true);
    await settle(fixture);
    buttonByText(fixture, 'Clear').click();
    await settle(fixture);
    expect(store.draft().channels).toEqual([]);
    const run: EventLogRun = {
      running: false, paused: false, dir: '/x', events: 1, eventsPerSecond: 0.5, channels: [], errors: [{ at: 0, channel: null, message: 'boom' }],
      lastEvent: { ts: null, channel: 'System', provider: 'P', eventId: null, level: 'Information' }, unreadable: 0, queued: 0, queueCapacity: 10, xpath: '*',
    };
    vi.spyOn(TestBed.inject(EventLogBackend), 'eventLogStatus').mockResolvedValue(run);
    await store.poll();
    await settle(fixture);
    expect(el(fixture, '[data-testid=eventlog-last]').textContent).toContain('ID —');
    expect(el(fixture, '[data-testid=eventlog-errors]').textContent).toContain('boom');
  });
});
