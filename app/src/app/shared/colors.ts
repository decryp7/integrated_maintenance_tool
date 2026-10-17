import { Level } from '../core/api.types';

/** Validated categorical slots; anything beyond folds into "Other". Never cycle. */
export const SERIES_VARS = ['var(--s1)', 'var(--s2)', 'var(--s3)', 'var(--s4)'];
export const OTHER_VAR = 'var(--s-other)';

export const LEVEL_VARS: Record<Level, string> = {
  Verbose: 'var(--lvl-verbose)',
  Debug: 'var(--lvl-debug)',
  Information: 'var(--lvl-info)',
  Warning: 'var(--lvl-warn)',
  Error: 'var(--lvl-error)',
  Fatal: 'var(--lvl-fatal)',
};

export const LEVEL_CODES: Record<Level, string> = {
  Verbose: 'VRB',
  Debug: 'DBG',
  Information: 'INF',
  Warning: 'WRN',
  Error: 'ERR',
  Fatal: 'FTL',
};

/** Histogram fill per level: Information recedes so problems stand out. */
export const HIST_VARS: Record<Level, string> = { ...LEVEL_VARS, Information: 'var(--hist-info)' };

export const MARKER_VARS: Record<string, string> = {
  start: 'var(--mk-start)',
  stop: 'var(--mk-stop)',
  fatal: 'var(--mk-fatal)',
  exception: 'var(--mk-exception)',
  error: 'var(--mk-error)',
  warning: 'var(--mk-warning)',
};

/** Colours for capture groups in the extractor preview. */
export const CAPTURE_VARS = ['var(--s1)', 'var(--s2)', 'var(--s4)', 'var(--s3)'];
