import { Level } from '../../core/api.types';
import {
  EventLogBackfill,
  EventLogChannel,
  EventLogChannelStatus,
  EventLogErrorEntry,
  EventLogLevel,
  EventLogQuery,
  EventLogRun,
  EventLogStartArgs,
  EventLogStarted,
  EventLogStatus,
  EventLogValidation,
} from '../../core/tools.types';

/** The channels a demo Windows machine offers (standard ones first, like the engine). */
export const MOCK_EVENT_CHANNELS: readonly EventLogChannel[] = [
  { name: 'System', needsAdmin: false },
  { name: 'Application', needsAdmin: false },
  { name: 'Security', needsAdmin: true },
  { name: 'Setup', needsAdmin: false },
  { name: 'ForwardedEvents', needsAdmin: false },
  { name: 'Microsoft-Windows-PowerShell/Operational', needsAdmin: false },
  { name: 'Microsoft-Windows-TaskScheduler/Operational', needsAdmin: false },
  { name: 'Microsoft-Windows-Windows Defender/Operational', needsAdmin: false },
  { name: 'Microsoft-Windows-WindowsUpdateClient/Operational', needsAdmin: false },
];

/** The text Windows gives when a channel needs administrator rights (matches `isAccessDenied`). */
export const MOCK_SECURITY_DENIED = 'Access is denied. Reading the Security channel requires administrator rights.';
/** The channel that fails validation on the demo machine. */
export const MOCK_DISABLED_CHANNEL = 'ForwardedEvents';
const DISABLED_ERROR = 'The channel is disabled: no event subscription forwards events to it. Enable one in Event Viewer or choose another channel.';

const LEVEL_PREDICATE: Record<EventLogLevel, string> = {
  critical: 'Level=1',
  error: 'Level=2',
  warning: 'Level=3',
  information: 'Level=0 or Level=4',
  verbose: 'Level=5',
};
const BACKFILL_MS: Record<Exclude<EventLogBackfill, 'none'>, number> = { '1h': 3_600_000, '24h': 86_400_000, '7d': 604_800_000 };
const ALL_LEVELS = Object.keys(LEVEL_PREDICATE) as EventLogLevel[];
const QUEUE_CAPACITY = 10_000;
/** Events each channel writes per status read, so the demo advances without timers. */
const EVENTS_PER_READ = 3;
const PROVIDERS: Record<string, string> = { System: 'Service Control Manager', Application: 'Application Error', Security: 'Microsoft-Windows-Security-Auditing' };

/** A plausible composed query (the real one is built by the engine). */
export function mockXpath(levels: EventLogLevel[] | undefined, backfill: EventLogBackfill, filter: string | undefined): string {
  const set = ALL_LEVELS.filter((l) => levels?.includes(l));
  const parts = [
    set.length && set.length < ALL_LEVELS.length ? `System[(${set.map((l) => LEVEL_PREDICATE[l]).join(' or ')})]` : null,
    backfill !== 'none' ? `System[TimeCreated[timediff(@SystemTime) <= ${BACKFILL_MS[backfill]}]]` : null,
    filter?.trim() ? `(${filter.trim()})` : null,
  ].filter((p): p is string => p !== null);
  return parts.length ? `*[${parts.join(' and ')}]` : '*';
}

function parseQuery(q: EventLogQuery): void {
  if (!q.channels.length) throw new Error('choose at least one event log channel');
  for (const c of q.channels) {
    if (!MOCK_EVENT_CHANNELS.some((m) => m.name === c) && !/^Microsoft-Windows-[^/]+\/Operational$/.test(c)) {
      throw new Error(`"${c}" is not a channel the follower reads: choose System, Application, Security, Setup, ForwardedEvents or a Microsoft-Windows-*/Operational channel`);
    }
  }
  const filter = q.xpath?.trim() ?? '';
  let depth = 0;
  for (const ch of filter) {
    depth += ch === '[' ? 1 : ch === ']' ? -1 : 0;
    if (depth < 0) break;
  }
  if (depth !== 0) throw new Error('the XPath filter is not valid: its brackets do not balance');
}

const LEVEL_OF: Level[] = ['Information', 'Warning', 'Error'];

/**
 * The Event Log follower of the mock tools backend: one run at a time, Security needs administrator rights,
 * ForwardedEvents fails validation, and every status read advances the counters.
 */
export class MockEventLog {
  private run: EventLogRun | null = null;
  private runs = 0;
  private reads = 0;
  /** Channels with a saved position (a later run continues from them). */
  private readonly bookmarks = new Set<string>();

  /** `elevated` grants the Security channel; `denied` makes every start fail with missing rights. */
  constructor(private readonly rights: () => { elevated: boolean; denied: boolean }) {}

  async channels(): Promise<EventLogChannel[]> {
    return MOCK_EVENT_CHANNELS.map((c) => ({ ...c }));
  }

  private probe(channel: string): string | null {
    if (channel === 'Security' && !this.rights().elevated) return MOCK_SECURITY_DENIED;
    if (channel === MOCK_DISABLED_CHANNEL) return DISABLED_ERROR;
    if (!MOCK_EVENT_CHANNELS.some((m) => m.name === channel)) return `The specified channel could not be found: ${channel}`;
    return null;
  }

  async validate(q: EventLogQuery): Promise<EventLogValidation> {
    parseQuery(q);
    const backfill = q.backfill ?? 'none';
    return {
      xpath: mockXpath(q.levels, 'none', q.xpath),
      backfillXpath: backfill === 'none' ? null : mockXpath(q.levels, backfill, q.xpath),
      channels: [...new Set(q.channels)].map((name) => {
        const error = this.probe(name);
        return { name, ok: error === null, error };
      }),
    };
  }

  async start(args: EventLogStartArgs): Promise<EventLogStarted> {
    if (this.run) throw new Error('the event log follower is already running');
    parseQuery(args);
    const { elevated, denied } = this.rights();
    if (denied && !elevated) throw new Error('Access is denied. Administrator rights are required.');
    const channels = [...new Set(args.channels)];
    if (channels.includes('Security') && !elevated) throw new Error(`Security: ${MOCK_SECURITY_DENIED}`);
    const dir = args.dir?.trim() || `/demo/eventlog/run-${++this.runs}`;
    const resumed = args.resumeFromBookmark === false ? [] : channels.filter((c) => this.bookmarks.has(c));
    const xpath = mockXpath(args.levels, 'none', args.xpath);
    const backfill = (args.backfill ?? 'none') !== 'none';
    this.reads = 0;
    this.run = {
      running: true,
      paused: false,
      dir,
      events: 0,
      eventsPerSecond: 0,
      lastEvent: null,
      channels: channels.map((channel) => ({ channel, state: backfill ? 'backfill' : 'starting', events: 0, lastRecordId: null, error: null })),
      errors: [],
      unreadable: 0,
      queued: 0,
      queueCapacity: QUEUE_CAPACITY,
      xpath,
    };
    // A channel that cannot be read retries (and says why) instead of failing the run, like the engine.
    for (const c of this.run.channels) {
      const error = this.probe(c.channel);
      if (error) this.fail(c, error);
    }
    return { dir, xpath, resumed };
  }

  private fail(c: EventLogChannelStatus, message: string): void {
    c.state = 'retrying';
    c.error = message;
    const entry: EventLogErrorEntry = { at: Date.now(), channel: c.channel, message };
    this.run?.errors.unshift(entry);
    this.run?.errors.splice(20);
  }

  async stop(): Promise<string | null> {
    const run = this.run;
    if (!run) return null;
    // Positions are saved on stop, so the next run can continue from them.
    for (const c of run.channels) if (c.lastRecordId !== null) this.bookmarks.add(c.channel);
    this.run = null;
    return run.dir;
  }

  private requireRun(): EventLogRun {
    if (!this.run) throw new Error('the event log follower is not running');
    return this.run;
  }

  async pause(): Promise<EventLogStatus> {
    this.requireRun().paused = true;
    return this.status();
  }

  async resume(): Promise<EventLogStatus> {
    this.requireRun().paused = false;
    return this.status();
  }

  /** Each read advances a running, unpaused follower: backfill and starting channels go live, events are written. */
  async status(): Promise<EventLogStatus> {
    const run = this.run;
    if (!run) return { running: false, dir: null };
    if (!run.paused) this.advance(run);
    return structuredClone(run);
  }

  private advance(run: EventLogRun): void {
    this.reads++;
    let written = 0;
    for (const c of run.channels) {
      if (c.state === 'retrying' || c.state === 'stopped') continue;
      c.state = 'live';
      c.events += EVENTS_PER_READ;
      c.lastRecordId = 1000 + c.events;
      written += EVENTS_PER_READ;
      run.lastEvent = {
        ts: Date.now(),
        channel: c.channel,
        provider: PROVIDERS[c.channel] ?? c.channel.replace(/\/Operational$/, ''),
        eventId: 7036 + (this.reads % 3),
        level: LEVEL_OF[this.reads % LEVEL_OF.length] ?? 'Information',
      };
    }
    run.events += written;
    run.eventsPerSecond = written;
    run.queued = written ? this.reads % 4 : 0;
  }
}
