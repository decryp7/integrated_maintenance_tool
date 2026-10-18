/** Height of one activity-bar button, in px. */
export const ACTIVITY_BUTTON_PX = 36;
/** Gap between two activity-bar buttons, in px. */
export const ACTIVITY_GAP_PX = 6;
/** The bar's vertical padding (top and bottom together), in px. */
export const ACTIVITY_PADDING_PX = 16;

/**
 * How many of the `top` buttons fit in a bar `height` px tall that also holds `bottom` buttons. When not all fit,
 * one slot goes to the More button, so the result is the number shown before it. An unknown height (0, e.g. before
 * layout) shows every button.
 */
export function fittingTopItems(top: number, bottom: number, height: number): number {
  if (!(height > 0)) return top;
  // n buttons take n * (button + gap) - gap; the spacer between the sections may shrink to nothing.
  const slots = Math.floor((height - ACTIVITY_PADDING_PX + ACTIVITY_GAP_PX) / (ACTIVITY_BUTTON_PX + ACTIVITY_GAP_PX)) - bottom;
  if (top <= slots) return top;
  return Math.max(0, slots - 1);
}
