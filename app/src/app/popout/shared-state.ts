import { Injector } from '@angular/core';
import { Query, RecordView } from '../core/api.types';
import { DateFormat, TimeZoneMode } from '../core/format';
import { ThemeMode, WorkspaceStore } from '../core/workspace.store';
import { TimeCursor } from '../analysis/time-cursor';
import { sanitizeQuery } from '../lifecycle/core-ui-state';
import { LiveFollow, sanitizeFollow } from '../live/live-follow.store';
import { GridDensity, SettingsStore } from '../settings/settings.store';
import { LiveTail } from '../workspace/live-tail';
import { SyncedValue } from './ui-sync';

const THEMES: readonly ThemeMode[] = ['system', 'dark', 'light'];
const ZONES: readonly TimeZoneMode[] = ['utc', 'local'];
const DATE_FORMATS: readonly DateFormat[] = ['readable', 'iso'];
const DENSITIES: readonly GridDensity[] = ['compact', 'comfortable'];

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null && !Array.isArray(v);
}

/** A query from another window: known keys only, plus the query-language filter and the live window when set. */
export function sharedQuery(v: unknown): Query {
  const q: Query = sanitizeQuery(v);
  if (!isRecord(v)) return q;
  if (typeof v['ql'] === 'string') q.ql = v['ql'];
  if (isRecord(v['window'])) q.window = v['window'] as unknown as Query['window'];
  return q;
}

/** A selected record from another window, or null. */
export function sharedRecord(v: unknown): RecordView | null {
  return isRecord(v) && typeof v['id'] === 'number' && typeof v['message'] === 'string' ? (v as unknown as RecordView) : null;
}

export function sharedRange(v: unknown): { from: number; to: number } | null {
  return isRecord(v) && typeof v['from'] === 'number' && typeof v['to'] === 'number' ? { from: v['from'], to: v['to'] } : null;
}

/**
 * The interaction state a host and its pop-outs share: the query, selection and jumps, time cursor and brushed
 * range, live follow, theme and display settings, plus what only the host decides (data versions, live tail on/off).
 */
export function sharedUiState(injector: Injector): SyncedValue[] {
  const store = injector.get(WorkspaceStore);
  const cursor = injector.get(TimeCursor);
  const follow = injector.get(LiveFollow);
  const live = injector.get(LiveTail);
  const settings = injector.get(SettingsStore);
  return [
    { key: 'query', read: () => store.query(), apply: (v) => store.restoreQuery(sharedQuery(v)) },
    { key: 'selected', read: () => store.selected(), apply: (v) => store.select(sharedRecord(v)) },
    {
      key: 'jump',
      read: () => store.anchor(),
      apply: (v) => {
        if (isRecord(v) && typeof v['id'] === 'number') store.jumpTo(v['id']);
      },
    },
    { key: 'theme', read: () => store.theme(), apply: (v) => THEMES.includes(v as ThemeMode) && store.setTheme(v as ThemeMode) },
    { key: 'timezone', read: () => store.timezone(), apply: (v) => ZONES.includes(v as TimeZoneMode) && store.setTimezone(v as TimeZoneMode) },
    { key: 'lifecycle', read: () => store.alwaysShowLifecycle(), apply: (v) => typeof v === 'boolean' && store.setAlwaysShowLifecycle(v) },
    {
      key: 'sort',
      read: () => store.descending(),
      apply: (v) => {
        if (typeof v === 'boolean' && v !== store.descending()) store.toggleSort();
      },
    },
    {
      key: 'columns',
      read: () => store.extraColumns(),
      apply: (v) => {
        if (!Array.isArray(v)) return;
        const want = v.filter((c): c is string => typeof c === 'string');
        for (const c of store.extraColumns()) if (!want.includes(c)) store.removeColumn(c);
        for (const c of want) store.addColumn(c);
      },
    },
    { key: 'cursor', read: () => cursor.hovered(), apply: (v) => cursor.hover(typeof v === 'number' ? v : null) },
    { key: 'range', read: () => cursor.selection(), apply: (v) => cursor.select(sharedRange(v)) },
    {
      key: 'follow',
      read: () => ({ follow: follow.follow(), preset: follow.preset(), customMinutes: follow.customMinutes(), paused: follow.paused() }),
      apply: (v) => {
        follow.restore(sanitizeFollow(v));
        if (isRecord(v) && v['paused'] === true) follow.pause();
      },
    },
    { key: 'dateFormat', read: () => settings.dateFormat(), apply: (v) => DATE_FORMATS.includes(v as DateFormat) && settings.setDateFormat(v as DateFormat) },
    { key: 'density', read: () => settings.density(), apply: (v) => DENSITIES.includes(v as GridDensity) && settings.setDensity(v as GridDensity) },
    {
      key: 'live',
      direction: 'fromHost',
      read: () => live.enabled(),
      apply: (v) => {
        if (v === true && !live.enabled()) void live.restore();
        else if (v === false) live.enabled.set(false);
      },
    },
    {
      key: 'data',
      direction: 'fromHost',
      read: () => store.dataVersion() + store.profileVersion() * 1e6,
      apply: () => void store.refreshSummary().catch(() => undefined),
    },
  ];
}
