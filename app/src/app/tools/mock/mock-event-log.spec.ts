import { EventLogRun, isEventLogRun } from '../../core/tools.types';
import { MOCK_DISABLED_CHANNEL, MOCK_SECURITY_DENIED, MockEventLog, mockXpath } from './mock-event-log';
import { MOCK_DEFAULTS, MockToolsBackend, MockToolsOptions } from './mock-tools-backend';
import { isAccessDenied } from '../tool-errors';

function make(rights = { elevated: false, denied: false }) {
  return new MockEventLog(() => rights);
}

async function runOf(log: MockEventLog): Promise<EventLogRun> {
  const s = await log.status();
  if (!isEventLogRun(s)) throw new Error('not running');
  return s;
}

describe('mockXpath', () => {
  it('composes levels, the backfill window and the user filter like the engine', () => {
    expect(mockXpath(undefined, 'none', undefined)).toBe('*');
    expect(mockXpath(['critical', 'error', 'warning', 'information', 'verbose'], 'none', '  ')).toBe('*');
    expect(mockXpath(['information', 'critical'], 'none', undefined)).toBe('*[System[(Level=1 or Level=0 or Level=4)]]');
    expect(mockXpath(['error'], '1h', 'System[EventID=7036]')).toBe(
      '*[System[(Level=2)] and System[TimeCreated[timediff(@SystemTime) <= 3600000]] and (System[EventID=7036])]',
    );
    expect(mockXpath([], '7d', undefined)).toBe('*[System[TimeCreated[timediff(@SystemTime) <= 604800000]]]');
  });
});

describe('MockEventLog', () => {
  it('offers the standard and operational channels, Security needing administrator rights', async () => {
    const channels = await make().channels();
    expect(channels.slice(0, 5).map((c) => c.name)).toEqual(['System', 'Application', 'Security', 'Setup', 'ForwardedEvents']);
    expect(channels.filter((c) => c.needsAdmin).map((c) => c.name)).toEqual(['Security']);
  });

  it('validates each channel: Security is denied without elevation, ForwardedEvents is disabled', async () => {
    const v = await make().validate({ channels: ['System', 'Security', MOCK_DISABLED_CHANNEL, 'System'], backfill: '24h', levels: ['error'] });
    expect(v.xpath).toBe('*[System[(Level=2)]]');
    expect(v.backfillXpath).toContain('86400000');
    expect(v.channels.map((c) => [c.name, c.ok])).toEqual([
      ['System', true],
      ['Security', false],
      [MOCK_DISABLED_CHANNEL, false],
    ]);
    expect(isAccessDenied(v.channels[1]?.error)).toBe(true);
    expect((await make({ elevated: true, denied: false }).validate({ channels: ['Security'] })).channels[0]).toEqual({ name: 'Security', ok: true, error: null });
    expect((await make().validate({ channels: ['Microsoft-Windows-Nope/Operational'] })).channels[0]?.error).toContain('could not be found');
    expect((await make().validate({ channels: ['System'] })).backfillXpath).toBeNull();
  });

  it('rejects arguments the engine would reject', async () => {
    const log = make();
    await expect(log.validate({ channels: [] })).rejects.toThrow('choose at least one');
    await expect(log.validate({ channels: ['Bogus'] })).rejects.toThrow('not a channel the follower reads');
    await expect(log.validate({ channels: ['System'], xpath: '*[System[EventID=1]' })).rejects.toThrow('brackets');
    await expect(log.validate({ channels: ['System'], xpath: ']*[' })).rejects.toThrow('brackets');
    await expect(log.start({ channels: [] })).rejects.toThrow('choose at least one');
  });

  it('runs once at a time, advances on every status read, pauses and resumes', async () => {
    const log = make();
    expect(await log.status()).toEqual({ running: false, dir: null });
    await expect(log.pause()).rejects.toThrow('not running');
    const started = await log.start({ channels: ['System', 'Application'], backfill: '1h' });
    expect(started).toEqual({ dir: '/demo/eventlog/run-1', xpath: '*', resumed: [] });
    await expect(log.start({ channels: ['System'] })).rejects.toThrow('already running');
    const first = await runOf(log);
    expect(first.channels.map((c) => c.state)).toEqual(['live', 'live']);
    expect(first.events).toBe(6);
    expect(first.lastEvent?.channel).toBe('Application');
    expect((await runOf(log)).events).toBe(12);
    const paused = await log.pause();
    expect(isEventLogRun(paused) && paused.paused).toBe(true);
    expect((await runOf(log)).events).toBe(12);
    // Resuming answers with a fresh status, which advances again.
    expect(await log.resume()).toMatchObject({ paused: false, events: 18 });
    expect((await runOf(log)).events).toBe(24);
    expect(await log.stop()).toBe('/demo/eventlog/run-1');
    expect(await log.stop()).toBeNull();
  });

  it('continues from saved positions unless asked to start afresh', async () => {
    const log = make();
    await log.start({ channels: ['System'] });
    await log.status();
    await log.stop();
    expect((await log.start({ channels: ['System', 'Setup'], dir: ' /out ' })).resumed).toEqual(['System']);
    await log.stop();
    expect((await log.start({ channels: ['System'], resumeFromBookmark: false })).resumed).toEqual([]);
  });

  it('refuses Security without elevation, everything in denied mode, and retries a disabled channel', async () => {
    const start = (log: MockEventLog) => log.start({ channels: ['Security'] });
    const err = await start(make()).catch((e: Error) => e.message);
    expect(err).toContain(MOCK_SECURITY_DENIED);
    expect(isAccessDenied(err as string)).toBe(true);
    await expect(make({ elevated: false, denied: true }).start({ channels: ['System'] })).rejects.toThrow('Access is denied');
    await expect(start(make({ elevated: true, denied: true }))).resolves.toMatchObject({ dir: '/demo/eventlog/run-1' });

    const log = make();
    await log.start({ channels: [MOCK_DISABLED_CHANNEL, 'Microsoft-Windows-Nope/Operational'] });
    const run = await runOf(log);
    expect(run.channels.map((c) => c.state)).toEqual(['retrying', 'retrying']);
    expect(run.errors.map((e) => e.channel)).toEqual(['Microsoft-Windows-Nope/Operational', MOCK_DISABLED_CHANNEL]);
    expect(run.events).toBe(0);
    expect(run.lastEvent).toBeNull();
  });
});

describe('MockToolsBackend event log', () => {
  const backend = (o: Partial<MockToolsOptions> = {}) => new MockToolsBackend({ ...MOCK_DEFAULTS, ...o });

  it('goes through tool_invoke and records the start', async () => {
    const b = backend();
    expect((await b.eventLogChannels()).length).toBeGreaterThan(5);
    expect((await b.eventLogValidate({ channels: ['System'] })).xpath).toBe('*');
    expect((await b.eventLogStart({ channels: ['System'] })).dir).toBe('/demo/eventlog/run-1');
    expect(b.calls).toEqual(['eventLogStart:/demo/eventlog/run-1']);
    expect(await b.eventLogPause()).toMatchObject({ paused: true });
    expect(await b.eventLogResume()).toMatchObject({ paused: false });
    expect(await b.eventLogStatus()).toMatchObject({ running: true });
    expect(await b.eventLogStop()).toBe('/demo/eventlog/run-1');
  });

  it('is unsupported away from Windows and refuses Security unless elevated', async () => {
    await expect(backend({ platform: 'linux' }).eventLogChannels()).rejects.toThrow('only available on Windows');
    await expect(backend().eventLogStart({ channels: ['Security'] })).rejects.toThrow('Access is denied');
    await expect(backend({ elevated: true }).eventLogStart({ channels: ['Security'] })).resolves.toBeDefined();
  });
});
