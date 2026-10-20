import { computed, linkedSignal, Signal } from '@angular/core';
import { Lane, Timeline } from '../core/api.types';

/** Lanes in display order, with the rank of every lane key seen in the full (unzoomed) view. */
export interface OrderedLanes {
  lanes: readonly Lane[];
  order: ReadonlyMap<string, number>;
}

/**
 * Orders lanes by their position in the full (unzoomed) view, without mutating anything. With `reset` (nothing is
 * zoomed) the given lanes define the order; otherwise lanes keep their earlier slot and new ones follow by first event.
 */
export function orderLanes(lanes: readonly Lane[], previous: ReadonlyMap<string, number>, reset: boolean): OrderedLanes {
  if (reset) return { lanes, order: new Map(lanes.map((l, i) => [l.key, i])) };
  const rank = (l: Lane) => previous.get(l.key) ?? Number.MAX_SAFE_INTEGER;
  return { lanes: [...lanes].sort((a, b) => rank(a) - rank(b) || a.first - b.first), order: previous };
}

/**
 * The lanes of `timeline` in a stable order: zooming (`hasRange`) never reshuffles them. The order of the full view
 * is carried from one value to the next by the linked signal, so no computed mutates state.
 */
export function stableLanes(timeline: Signal<Timeline | null>, hasRange: Signal<boolean>): Signal<readonly Lane[]> {
  const ordered = linkedSignal<{ lanes: readonly Lane[]; reset: boolean }, OrderedLanes>({
    source: () => ({ lanes: timeline()?.lanes ?? [], reset: !hasRange() }),
    computation: ({ lanes, reset }, previous) => orderLanes(lanes, previous?.value.order ?? new Map(), reset),
  });
  return computed(() => ordered().lanes);
}
