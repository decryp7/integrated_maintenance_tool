/** Layout shared by the analysis views; colours come from the app's CSS tokens (dark and light). */
export const ANALYSIS_STYLES = `
  :host { flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column; background: var(--bg-1); color: var(--text-2); font-size: var(--fs-sm); }
  .bar { flex: none; display: flex; align-items: center; flex-wrap: wrap; gap: 8px 10px; padding: 6px 12px; border-bottom: 1px solid var(--border); }
  .spacer { flex: 1; }
  .body { flex: 1; min-height: 0; overflow: auto; padding: 8px 12px 12px; }
  .list { list-style: none; margin: 0; padding: 0; }
  .row { display: flex; align-items: center; gap: 10px; width: 100%; padding: 6px 12px; border: 0; border-bottom: 1px solid var(--border); background: transparent; color: inherit; font: inherit; text-align: left; cursor: pointer; }
  .row:hover { background: var(--bg-hover); }
  .row:focus-visible, .btn:focus-visible, .link:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: -2px; }
  .ellipsis { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
  .grow { flex: 1; min-width: 0; }
  .num { text-align: right; font-variant-numeric: tabular-nums; }
  .muted { color: var(--muted); }
  .link { background: none; border: 0; padding: 0; font: inherit; color: var(--chip-text); cursor: pointer; text-align: left; }
  .link:hover { text-decoration: underline; }
  table.grid { border-collapse: collapse; width: 100%; font-size: var(--fs-sm); }
  table.grid th, table.grid td { padding: 4px 10px; border-bottom: 1px solid var(--border); text-align: left; white-space: nowrap; }
  table.grid th { position: sticky; top: 0; background: var(--bg-2); color: var(--muted); font-weight: 600; z-index: 1; }
  table.grid th button { all: unset; cursor: pointer; display: inline-flex; gap: 4px; align-items: center; color: inherit; font-weight: inherit; }
  table.grid th button:focus-visible { outline: 2px solid var(--focus-ring); }
  table.grid td.num, table.grid th.num { text-align: right; }
  .card { border: 1px solid var(--border-2); border-radius: 6px; background: var(--bg-2); padding: 10px 12px; }
  .cards { display: grid; grid-template-columns: repeat(auto-fill, minmax(260px, 1fr)); gap: 10px; }
  h3.sec { margin: 14px 0 6px; font-size: var(--fs-sm); font-weight: 600; color: var(--muted); text-transform: none; }
  .bars { display: flex; height: 8px; border-radius: 4px; overflow: hidden; background: var(--lane-bar); min-width: 60px; }
  .bars span { display: block; height: 100%; }
  .lvl { font-family: var(--font-mono); font-size: var(--fs-2xs); padding: 1px 5px; border-radius: 3px; border: 1px solid var(--border-3); color: var(--text-2); }
  .lvl.Warning { color: var(--warn-text); border-color: var(--warn-text); }
  .lvl.Error, .lvl.Fatal { color: var(--danger-text); border-color: var(--danger-text); }
  .up { color: var(--danger-text); }
  .down { color: var(--ok); }
  .flat { color: var(--muted); }
`;
