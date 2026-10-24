import type { DebriefView } from '../extensions/tokens';
import { defaultViewLayout, ViewLayout } from '../layout-plan/layout-plan';
import { viewArea } from './view-area';

/** The view's declared constraints, else the conservative default for its area (plugin views that declare none). */
export function viewLayout(view: Pick<DebriefView, 'id' | 'defaultPosition' | 'area' | 'layout'> | undefined | null): ViewLayout {
  return view?.layout ?? defaultViewLayout(viewArea(view));
}

/**
 * The row of the main column a view shares with others: analysis views share one row below the grid, the timeline and
 * the events grid have a row each, and every other document sits beside the timeline (its row).
 */
export function viewRow(view: Pick<DebriefView, 'id' | 'defaultPosition' | 'area' | 'layout'>): string {
  if (viewArea(view) === 'analysis') return 'analysis';
  return view.id === 'events' ? 'events' : 'timeline';
}
