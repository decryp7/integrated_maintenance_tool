import type { ImtView } from '../extensions/tokens';

/**
 * Visual Studio's two kinds of panels: tool windows dock around the edges (and can auto-hide); documents live in
 * the central document well and are what you work on.
 */
export type ViewKind = 'tool' | 'document';

/**
 * Views that are tool windows: inspectors, lists and status panels you keep beside your work. Everything else
 * (timeline, events, query results, profiling views, dashboards, reports, editors) is a document.
 */
export const TOOL_VIEWS: ReadonlySet<string> = new Set([
  'inspector',
  'live-alerts',
  'tail-status',
  'overview',
  'saved-queries',
  'notes',
  'problems',
  'markers',
  'plugin-health',
  'error-snapshots',
  'profiling-captures',
  'profiling-butterfly',
  'profiling-findings',
  'collection-runs',
]);

/** Views that are documents even though they open on the right or at the bottom by default. */
const DOCUMENT_VIEWS: ReadonlySet<string> = new Set(['events', 'query', 'query-results', 'report', 'anomalies', 'patterns', 'exceptions', 'profiling-threads']);

/** Is the view a tool window or a document? Known ids first; otherwise right/bottom views are tools. */
export function viewKind(view: Pick<ImtView, 'id' | 'defaultPosition'> | undefined | null): ViewKind {
  if (!view) return 'document';
  if (TOOL_VIEWS.has(view.id)) return 'tool';
  if (DOCUMENT_VIEWS.has(view.id)) return 'document';
  return view.defaultPosition === 'right' ? 'tool' : 'document';
}
