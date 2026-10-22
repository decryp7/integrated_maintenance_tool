/** Layout shared by the Video panels; colours are the app's tokens (dark and light). */
export const VIDEO_STYLES = `
  :host { display: block; min-width: 0; color: var(--text-2); font-size: var(--fs-sm); }
  h3 { margin: 0 0 6px; font-size: var(--fs-xs); font-weight: 600; letter-spacing: 0.08em; text-transform: uppercase; color: var(--muted); }
  p { margin: 4px 0; }
  .muted { color: var(--muted); }
  .mono { font-family: var(--font-mono); }
  .row { display: flex; flex-wrap: wrap; align-items: center; gap: 6px 8px; margin: 6px 0; }
  .grow { flex: 1; min-width: 0; }
  label.field { display: inline-flex; align-items: center; gap: 6px; }
  input[type='text'], input[type='datetime-local'], input[type='number'], select, textarea {
    font: inherit; font-size: var(--fs-sm); color: var(--text); background: var(--bg-3); border: 1px solid var(--border-2); border-radius: 4px; padding: 2px 6px;
  }
  input[type='text'], input[type='datetime-local'], input[type='number'], select { height: 24px; }
  textarea { width: 100%; box-sizing: border-box; resize: vertical; min-height: 56px; }
  input:focus-visible, select:focus-visible, textarea:focus-visible, button:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: -1px; }
  .link { background: none; border: 0; padding: 0; font: inherit; color: var(--chip-text); cursor: pointer; }
  .link:hover { text-decoration: underline; }
  .card { border: 1px solid var(--border-2); border-radius: 6px; background: var(--bg-2); padding: 10px 12px; }
  .meter { height: 8px; border-radius: 4px; background: var(--lane-bar); overflow: hidden; }
  .meter > span { display: block; height: 100%; background: var(--accent); }
  .meter.full > span { background: var(--danger-text); }
  .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }
`;
