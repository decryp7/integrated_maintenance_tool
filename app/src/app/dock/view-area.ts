import type { DebriefView, ViewArea } from '../extensions/tokens';
import { viewKind } from './view-kind';

export type { ViewArea } from '../extensions/tokens';

/** The most tabs the analysis panel keeps; opening one more closes the least recently used inactive tab. */
export const ANALYSIS_TAB_CAP = 8;

/**
 * A view's home area: the one it declares, else derived from its kind and default position (for plugin views that
 * declare none): tool windows on the right are side tools, other tool windows status tools; documents at the bottom
 * open in the analysis panel, the others in the centre.
 */
export function viewArea(view: Pick<DebriefView, 'id' | 'defaultPosition' | 'area' | 'layout'> | undefined | null): ViewArea {
  if (!view) return 'document';
  if (view.layout) return view.layout.area;
  if (view.area) return view.area;
  if (viewKind(view) === 'tool') return view.defaultPosition === 'right' ? 'side' : 'status';
  return view.defaultPosition === 'bottom' ? 'analysis' : 'document';
}

/**
 * Which tab of a full analysis panel to close for a new one: the least recently used tab that is neither active nor
 * pinned (`mru` lists ids most recent first; ids not in it count as oldest). Null while the panel has room.
 */
export function analysisTabToClose(tabs: readonly string[], active: string | null, pinned: ReadonlySet<string>, mru: readonly string[], cap = ANALYSIS_TAB_CAP): string | null {
  if (tabs.length < cap) return null;
  const candidates = tabs.filter((id) => id !== active && !pinned.has(id));
  if (!candidates.length) return null;
  const rank = (id: string) => {
    const i = mru.indexOf(id);
    return i < 0 ? Number.POSITIVE_INFINITY : i;
  };
  return candidates.reduce((oldest, id) => (rank(id) > rank(oldest) ? id : oldest));
}
