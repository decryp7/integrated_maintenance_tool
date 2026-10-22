import { TailFileChange, TailReport } from '../core/api.types';

/** A report with nothing in it (what an event without payload means: "something changed"). */
export function emptyTailReport(): TailReport {
  return {
    appended: 0,
    updated: 0,
    sourcesChanged: [],
    added: [],
    rotated: [],
    truncated: [],
    deleted: [],
    reimported: [],
    idsChangedFrom: null,
    newestTs: null,
    instancesChanged: [],
    fileStates: [],
    alertsRaised: 0,
    elapsedMs: 0,
  };
}

const union = <T>(a: readonly T[], b: readonly T[]): T[] => [...new Set([...a, ...b])];
const minOf = (a: number | null, b: number | null): number | null => (a === null ? b : b === null ? a : Math.min(a, b));
const maxOf = (a: number | null, b: number | null): number | null => (a === null ? b : b === null ? a : Math.max(a, b));

/** What an event without payload means: something changed, ids may have moved (refresh everything). */
export function unknownChange(): TailReport {
  return { ...emptyTailReport(), idsChangedFrom: 0 };
}

/** Per-file state changes of two refreshes: one entry per file, the later state wins. */
function mergeFileStates(a: readonly TailFileChange[], b: readonly TailFileChange[]): TailFileChange[] {
  if (!b.length) return [...a];
  const later = new Set(b.map((c) => c.sourceId));
  return [...a.filter((c) => !later.has(c.sourceId)), ...b];
}

/** Combines two reports into one that describes both refreshes (counts add up, sets unite, the earliest id change wins). */
export function mergeReports(a: TailReport, b: TailReport): TailReport {
  return {
    appended: a.appended + b.appended,
    updated: a.updated + b.updated,
    sourcesChanged: union(a.sourcesChanged, b.sourcesChanged),
    added: union(a.added, b.added),
    rotated: [...a.rotated, ...b.rotated],
    truncated: union(a.truncated, b.truncated),
    deleted: union(a.deleted, b.deleted),
    reimported: union(a.reimported, b.reimported),
    idsChangedFrom: minOf(a.idsChangedFrom, b.idsChangedFrom),
    newestTs: maxOf(a.newestTs, b.newestTs),
    instancesChanged: union(a.instancesChanged, b.instancesChanged),
    fileStates: mergeFileStates(a.fileStates, b.fileStates),
    alertsRaised: a.alertsRaised + b.alertsRaised,
    elapsedMs: a.elapsedMs + b.elapsedMs,
  };
}

/**
 * True when existing record ids or sources changed (not just appended to): rows already on screen may now be
 * different events, so views holding ids (the grid, a selection) must reload even when they are not following.
 */
export function isStructural(r: TailReport): boolean {
  return (
    r.idsChangedFrom !== null ||
    r.updated > 0 ||
    r.truncated.length > 0 ||
    r.deleted.length > 0 ||
    r.reimported.length > 0
  );
}

/** True when the sidebar's file list or states may have changed (new, rotated, truncated or deleted files). */
export function changesSources(r: TailReport): boolean {
  return r.added.length > 0 || r.rotated.length > 0 || r.truncated.length > 0 || r.deleted.length > 0 || r.reimported.length > 0;
}

/** What the UI must refresh for a (coalesced) report. Pure, so the refresh policy is unit-testable. */
export interface RefreshPlan {
  /** Fetch the summary (counts, extent, sources) again. Also refetches the timeline (it follows the summary). */
  summary: boolean;
  /** Metric charts in view. */
  charts: boolean;
  /** Facet counts (the caller throttles these further). */
  facets: boolean;
  /** The grid: reload rows in view (`follow`), only count new events (`count`), or nothing. */
  grid: 'follow' | 'count' | 'none';
  /** Tail status (per-file states, sidebar badges). */
  status: boolean;
}

/** Decides what to refresh. `following`: the grid follows new events; otherwise it shows a "N new events" pill. */
export function planRefresh(r: TailReport, following: boolean): RefreshPlan {
  const structural = isStructural(r);
  const data = structural || r.appended > 0 || r.added.length > 0 || r.rotated.length > 0;
  return {
    summary: data,
    charts: data,
    facets: data,
    grid: !data ? 'none' : following || structural ? 'follow' : 'count',
    status: true,
  };
}
