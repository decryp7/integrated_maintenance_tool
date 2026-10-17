import { ViewKind } from './view-kind';

/** One entry of the IDE navigator (Ctrl+Tab). */
export interface SwitcherItem {
  id: string;
  title: string;
  kind: ViewKind;
}

export interface SwitcherState {
  /** Most recently used first. */
  items: readonly SwitcherItem[];
  index: number;
}

/** Opens the navigator on the panel used before the current one (Ctrl+Tab), or the last one (Ctrl+Shift+Tab). */
export function openSwitcher(items: readonly SwitcherItem[], backwards = false): SwitcherState | null {
  if (!items.length) return null;
  const index = items.length === 1 ? 0 : backwards ? items.length - 1 : 1;
  return { items, index };
}

/** Moves the selection, wrapping around. */
export function cycle(state: SwitcherState, delta: number): SwitcherState {
  const n = state.items.length;
  return n ? { ...state, index: (((state.index + delta) % n) + n) % n } : state;
}

/** Selects an entry by id (pointer), keeping the selection when the id is unknown. */
export function selectId(state: SwitcherState, id: string): SwitcherState {
  const i = state.items.findIndex((x) => x.id === id);
  return i < 0 ? state : { ...state, index: i };
}

export function selected(state: SwitcherState): SwitcherItem | null {
  return state.items[state.index] ?? null;
}

/** Records a use: moves `id` to the front of the most-recently-used list. */
export function touchMru(mru: readonly string[], id: string, limit = 50): string[] {
  return [id, ...mru.filter((x) => x !== id)].slice(0, limit);
}

/** The navigator's entries: open panels in most-recently-used order, never-used ones after them. */
export function switcherItems(mru: readonly string[], open: readonly SwitcherItem[]): SwitcherItem[] {
  const byId = new Map(open.map((i) => [i.id, i]));
  const used = mru.filter((id) => byId.has(id)).map((id) => byId.get(id)!);
  return [...used, ...open.filter((i) => !mru.includes(i.id))];
}
