import { ANALYSIS_STYLES } from '../analysis/analysis-styles';

/** Layout shared by the profiling views (on top of the analysis views' layout); colours are app tokens. */
export const PROFILING_STYLES =
  ANALYSIS_STYLES +
  `
  .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }
  select, input[type='search'], input[type='text'], input[type='number'] { font: inherit; font-size: var(--fs-sm); color: var(--text); background: var(--bg-3); border: 1px solid var(--border-2); border-radius: 4px; padding: 2px 6px; height: 24px; }
  select:focus-visible, input:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: -1px; }
  label.inline { display: inline-flex; align-items: center; gap: 5px; white-space: nowrap; }
  .pill { display: inline-flex; align-items: center; gap: 4px; padding: 1px 7px; border-radius: 10px; border: 1px solid var(--border-3); font-size: var(--fs-xs); color: var(--text-2); white-space: nowrap; }
  .pill button { all: unset; cursor: pointer; padding: 0 2px; color: var(--muted); }
  .pill button:focus-visible { outline: 2px solid var(--focus-ring); }
  .tag { font-family: var(--font-mono); font-size: var(--fs-2xs); padding: 0 4px; border-radius: 3px; border: 1px solid var(--border-3); color: var(--text-2); white-space: nowrap; }
  .tag.new, .tag.steady { color: var(--danger-text); border-color: var(--danger-text); }
  .tag.removed, .tag.gone { color: var(--ok); border-color: var(--ok); }
  .bar-cell { display: flex; align-items: center; gap: 6px; }
  .meter { flex: none; width: 60px; height: 6px; border-radius: 3px; background: var(--lane-bar); overflow: hidden; }
  .meter span { display: block; height: 100%; background: var(--s1); }
  .notice { padding: 4px 12px; font-size: var(--fs-sm); color: var(--muted); border-bottom: 1px solid var(--border); }
  tr.sel td { background: var(--bg-sel); }
  tr.clickable { cursor: pointer; }
  tr.clickable:hover td { background: var(--bg-hover); }
  .actions { display: inline-flex; gap: 2px; }
  .mini { all: unset; cursor: pointer; font-size: var(--fs-xs); padding: 0 5px; border-radius: 3px; color: var(--chip-text); border: 1px solid transparent; }
  .mini:hover { border-color: var(--border-3); }
  .mini:focus-visible { outline: 2px solid var(--focus-ring); }
`;
