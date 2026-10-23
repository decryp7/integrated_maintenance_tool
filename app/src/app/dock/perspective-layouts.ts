import type { DockviewApi } from 'dockview';
import type { DebriefView } from '../extensions/tokens';
import { PANEL } from './dock-context';
import { DockExtras } from './layout-store';

/** The built-in perspectives, in their Ctrl+1…4 order. */
export const BUILT_IN_PERSPECTIVES = ['investigate', 'profiling', 'live', 'diagnostics'] as const;
export type BuiltInPerspective = (typeof BUILT_IN_PERSPECTIVES)[number];

/** View ids the built-in perspectives arrange (owned by their features; listed here as data, not imported). */
export const PERSPECTIVE_VIEWS = {
  captures: 'profiling-captures',
  hotspots: 'profiling-hotspots',
  flame: 'profiling-flame',
  findings: 'profiling-findings',
  tailStatus: 'tail-status',
  alerts: 'live-alerts',
  problems: 'problems',
  jobs: 'jobs',
  errorSnapshots: 'error-snapshots',
  store: 'store',
  plugins: 'plugins',
} as const;

/** The view a perspective needs to be offered (absent features hide their perspective). */
export const PERSPECTIVE_REQUIRES: Readonly<Record<BuiltInPerspective, string | null>> = {
  investigate: null,
  profiling: PERSPECTIVE_VIEWS.captures,
  live: PERSPECTIVE_VIEWS.tailStatus,
  diagnostics: null,
};

/** What the builders need from the dock: the default arrangement and the view registry. */
export interface PerspectiveBuildContext {
  api: DockviewApi;
  /** Builds Investigate (the timeline over the events, the inspector auto-hidden) and returns its extras. */
  buildDefault(): DockExtras;
  find(id: string): DebriefView | undefined;
}

const LANDING_WIDTH = 380;
const BOTTOM_HEIGHT = 260;

/** Adds the views that exist as tabs of one new group at `direction` of the reference group, the first one active. */
function tabbed(ctx: PerspectiveBuildContext, ids: readonly string[], position: { referenceGroup?: string; direction: 'below' | 'right' | 'left' }, size?: { width?: number; height?: number }): void {
  const views = ids.map((id) => ctx.find(id)).filter((v): v is DebriefView => !!v);
  const first = views[0];
  if (!first) return;
  const panel = ctx.api.addPanel({ id: first.id, component: first.id, title: first.title, position: position.referenceGroup ? { referenceGroup: position.referenceGroup, direction: position.direction } : { direction: position.direction } });
  for (const v of views.slice(1)) ctx.api.addPanel({ id: v.id, component: v.id, title: v.title, inactive: true, position: { referenceGroup: panel.group.id, direction: 'within' } });
  if (size) panel.group.api.setSize(size);
}

/**
 * Builds a built-in perspective from scratch into an empty dock and returns its extras:
 * - Investigate: the timeline over the events (the default);
 * - Profiling: the captures list as the landing view (open a capture or profile a process; the hotspots and flame
 *   graph open beside it once a capture is open);
 * - Live: Investigate with tail status and alerts in the analysis panel below the events;
 * - Diagnostics: Investigate with problems, jobs, error snapshots, the store and plugin health docked below.
 */
export function buildPerspective(id: BuiltInPerspective, ctx: PerspectiveBuildContext): DockExtras {
  if (id === 'profiling') {
    const captures = ctx.find(PERSPECTIVE_VIEWS.captures);
    if (!captures) return ctx.buildDefault();
    const panel = ctx.api.addPanel({ id: captures.id, component: captures.id, title: captures.title });
    panel.group.api.setSize({ width: LANDING_WIDTH });
    return { autoHidden: [], pinnedTabs: [], poppedOut: [] };
  }
  const extras = ctx.buildDefault();
  const events = ctx.api.getPanel(PANEL.events);
  const below = events ? { referenceGroup: events.group.id, direction: 'below' as const } : { direction: 'below' as const };
  if (id === 'live') tabbed(ctx, [PERSPECTIVE_VIEWS.tailStatus, PERSPECTIVE_VIEWS.alerts], below, { height: BOTTOM_HEIGHT });
  if (id === 'diagnostics') {
    const v = PERSPECTIVE_VIEWS;
    tabbed(ctx, [v.problems, v.jobs, v.errorSnapshots, v.store, v.plugins], below, { height: BOTTOM_HEIGHT });
  }
  return extras;
}
