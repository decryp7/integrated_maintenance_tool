/** Styles shared by the "from data" dialogs (component-scoped, injected through `styles`). */
export const DIALOG_FORM_STYLES = `
  :host { display: flex; flex-direction: column; gap: 12px; }
  p { margin: 0; color: var(--text-2); line-height: 1.5; }
  .lbl { display: flex; flex-direction: column; gap: 4px; font-size: var(--fs-xs); color: var(--muted); }
  .row { display: flex; gap: 12px; align-items: flex-end; flex-wrap: wrap; }
  .row > .lbl { flex: 1; min-width: 120px; }
  /* 24px rows keep stacked 13px checkboxes and radios 24px apart (WCAG 2.5.8 spacing). */
  .check { display: flex; align-items: center; gap: 8px; min-height: 24px; color: var(--text-2); }
  .status { font-size: var(--fs-sm); color: var(--muted); }
  .status strong { color: var(--text); }
  .err { color: var(--danger-text); font-size: var(--fs-sm); }
  .foot { display: flex; justify-content: flex-end; gap: 8px; margin-top: 4px; }
  .box { padding: 8px 10px; border-radius: 6px; background: var(--code-bg); border: 1px solid var(--border); font-family: var(--font-mono); font-size: var(--fs-xs); line-height: 1.5; white-space: pre-wrap; word-break: break-word; max-height: 140px; overflow: auto; }
  .box .k { color: var(--muted); }
  .hl { background: var(--accent-soft); color: var(--chip-text); border-radius: 3px; }
`;
