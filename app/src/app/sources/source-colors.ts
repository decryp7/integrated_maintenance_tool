import { SourceColorMode } from '../settings/settings.store';

/**
 * Colour coding of sources and instances: pure helpers. Colours come from the theme's `--src-1`…`--src-4` tokens, a
 * categorical palette validated all-pairs for colour-vision deficiency and for 3:1 contrast on the row and panel
 * backgrounds of both themes (`source-colors.spec.ts`). Four colours separate safely; past four the colours repeat
 * with a different pattern (solid, dashed, dotted), so twelve sources stay distinct; beyond that the tooltip names it.
 */
export const SOURCE_COLOR_VARS = ['var(--src-1)', 'var(--src-2)', 'var(--src-3)', 'var(--src-4)'] as const;
export const SWATCH_PATTERNS = ['solid', 'dashed', 'dotted'] as const;
export type SwatchPattern = (typeof SWATCH_PATTERNS)[number];

/** How one source or instance is marked. `slot` is its stable number (0-based). */
export interface Swatch {
  slot: number;
  color: string;
  pattern: SwatchPattern;
}

/** The marker of slot `slot` (a source id, or an instance's assigned slot): colour first, then pattern. */
export function swatchAt(slot: number): Swatch {
  const n = Math.max(0, Math.floor(Number.isFinite(slot) ? slot : 0));
  const colors = SOURCE_COLOR_VARS.length;
  return {
    slot: n,
    color: SOURCE_COLOR_VARS[n % colors] ?? SOURCE_COLOR_VARS[0],
    pattern: SWATCH_PATTERNS[Math.floor(n / colors) % SWATCH_PATTERNS.length] ?? 'solid',
  };
}

/** CSS background of the vertical row stripe for `s`: solid, or dashes / dots down the row. */
export function stripeBackground(s: Swatch): string {
  switch (s.pattern) {
    case 'solid':
      return s.color;
    case 'dashed':
      return `repeating-linear-gradient(to bottom, ${s.color} 0 6px, transparent 6px 9px)`;
    case 'dotted':
      return `repeating-linear-gradient(to bottom, ${s.color} 0 3px, transparent 3px 6px)`;
  }
}

/**
 * Slots of instance keys, assigned in the order keys are first seen and never reassigned while the app runs:
 * an instance keeps its colour when others appear or go away (no recolouring by list position).
 */
export class InstanceSlots {
  private readonly slots = new Map<string, number>();

  slotOf(key: string): number {
    let slot = this.slots.get(key);
    if (slot === undefined) {
      slot = this.slots.size;
      this.slots.set(key, slot);
    }
    return slot;
  }
}

/** Labels of the source-colour modes (settings and the View menu). */
export const SOURCE_COLOR_LABELS: Record<SourceColorMode, string> = {
  source: $localize`:Source colour option@@sources.colors.mode.source:By file`,
  instance: $localize`:Source colour option@@sources.colors.mode.instance:By instance`,
  off: $localize`:Source colour option@@sources.colors.mode.off:Off`,
};
