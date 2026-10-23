import { EventLogBackfill, EventLogChannel, EventLogLevel, EventLogQuery, EventLogStartArgs } from '../../core/tools.types';

/** Limits the engine enforces (debrief-plugin-windows `eventlog::query`); checked early here for a quick message. */
export const MAX_EVENT_CHANNELS = 32;
export const MAX_EVENT_XPATH = 2048;

/** The follower form. */
export interface EventLogDraft {
  channels: string[];
  levels: EventLogLevel[];
  /** Optional user XPath filter. */
  xpath: string;
  backfill: EventLogBackfill;
  /** Continue from the saved positions instead of starting afresh. */
  resume: boolean;
  /** Output folder; empty for Debrief's data folder. */
  dir: string;
}

export const DEFAULT_EVENT_LOG_DRAFT: EventLogDraft = {
  channels: [],
  levels: ['critical', 'error', 'warning', 'information'],
  xpath: '',
  backfill: 'none',
  resume: true,
  dir: '',
};

export const LEVEL_LABEL: Record<EventLogLevel, string> = {
  critical: $localize`:@@tools.eventlog.level.critical:Critical`,
  error: $localize`:@@tools.eventlog.level.error:Error`,
  warning: $localize`:@@tools.eventlog.level.warning:Warning`,
  information: $localize`:@@tools.eventlog.level.information:Information`,
  verbose: $localize`:@@tools.eventlog.level.verbose:Verbose`,
};

export const BACKFILL_LABEL: Record<EventLogBackfill, string> = {
  none: $localize`:@@tools.eventlog.backfill.none:Only new events`,
  '1h': $localize`:@@tools.eventlog.backfill.1h:The last hour`,
  '24h': $localize`:@@tools.eventlog.backfill.24h:The last 24 hours`,
  '7d': $localize`:@@tools.eventlog.backfill.7d:The last 7 days`,
};

/** Messages for a form that cannot be sent yet (the engine re-validates everything). */
export function eventLogProblems(d: EventLogDraft): string[] {
  const problems: string[] = [];
  if (!d.channels.length) problems.push($localize`:@@tools.eventlog.problem.noChannel:Choose at least one channel to follow.`);
  if (d.channels.length > MAX_EVENT_CHANNELS) problems.push($localize`:@@tools.eventlog.problem.tooManyChannels:Follow at most ${MAX_EVENT_CHANNELS}:max: channels at once.`);
  if (!d.levels.length) problems.push($localize`:@@tools.eventlog.problem.noLevel:Choose at least one level.`);
  if (d.xpath.length > MAX_EVENT_XPATH) problems.push($localize`:@@tools.eventlog.problem.xpathLong:The XPath filter is longer than ${MAX_EVENT_XPATH}:max: characters.`);
  return problems;
}

/** The `validate` arguments of the form. */
export function toEventLogQuery(d: EventLogDraft): EventLogQuery {
  const xpath = d.xpath.trim();
  return {
    channels: [...d.channels],
    levels: [...d.levels],
    ...(xpath ? { xpath } : {}),
    backfill: d.backfill,
  };
}

/** The `start` arguments of the form. */
export function toEventLogStart(d: EventLogDraft): EventLogStartArgs {
  const dir = d.dir.trim();
  return { ...toEventLogQuery(d), resumeFromBookmark: d.resume, ...(dir ? { dir } : {}) };
}

/** Channels whose name contains the search text (case-insensitive). */
export function filterChannels(channels: readonly EventLogChannel[], search: string): EventLogChannel[] {
  const q = search.trim().toLowerCase();
  return q ? channels.filter((c) => c.name.toLowerCase().includes(q)) : [...channels];
}

/** Adds or removes one entry, keeping the order of first selection. */
export function toggled<T>(list: readonly T[], item: T, on: boolean): T[] {
  const without = list.filter((x) => x !== item);
  return on ? [...without, item] : without;
}
