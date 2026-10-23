/**
 * Token-themed look for the number input and segmented buttons, shared by the settings screens and the tool forms:
 * a segmented button that is selected is a tint with accent text, never a
 * second filled primary. Plain CSS so the elements keep their native roles, keyboard and form behaviour.
 */
export const THEMED_CONTROLS = `
  input[type='number'] { font-variant-numeric: tabular-nums; }
  .seg > .btn[aria-pressed='true'], .seg > .btn[aria-checked='true'] { background: var(--chip-bg); border-color: var(--chip-border); color: var(--chip-text); font-weight: 600; }
`;
