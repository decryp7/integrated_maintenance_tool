/** Styles shared by the "from data" dialogs (component-scoped, injected through `styles`). */
export const DIALOG_FORM_STYLES = `
  :host { display: flex; flex-direction: column; gap: 12px; }
  p { margin: 0; color: var(--text-2); line-height: 1.5; }
  .lbl { display: flex; flex-direction: column; gap: 4px; font-size: 11px; color: var(--muted); }
  .row { display: flex; gap: 12px; align-items: flex-end; flex-wrap: wrap; }
  .row > .lbl { flex: 1; min-width: 120px; }
  .check { display: flex; align-items: center; gap: 8px; color: var(--text-2); }
  .status { font-size: 12px; color: var(--muted); }
  .status strong { color: var(--text); }
  .err { color: var(--danger-text); font-size: 12px; }
  .foot { display: flex; justify-content: flex-end; gap: 8px; margin-top: 4px; }
  .box { padding: 8px 10px; border-radius: 6px; background: var(--code-bg); border: 1px solid var(--border); font-family: var(--font-mono); font-size: 11.5px; line-height: 1.5; white-space: pre-wrap; word-break: break-word; max-height: 140px; overflow: auto; }
  .box .k { color: var(--muted); }
  .hl { background: var(--accent-soft); color: var(--chip-text); border-radius: 3px; }
`;
