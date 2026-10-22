import { InjectionToken } from '@angular/core';

/** Commit and build date of this build; `'unknown'` when the build did not record them (plain `ng build`, tests). */
export interface BuildInfo {
  readonly commit: string;
  readonly date: string;
}

export const UNKNOWN_BUILD: BuildInfo = { commit: 'unknown', date: 'unknown' };

/**
 * Replaced at build time by `scripts/build-info.mjs` (`ng build --define DEBRIEF_BUILD_INFO=…`, used by `npm run build`
 * and so by `tauri build`). Undeclared in builds that do not define it.
 */
declare const DEBRIEF_BUILD_INFO: unknown;

const COMMIT = /^[0-9a-f]{4,40}$/;
const DATE = /^\d{4}-\d{2}-\d{2}$/;

/** Turns whatever the build defined into a `BuildInfo`; anything malformed becomes `'unknown'`. */
export function parseBuildInfo(raw: unknown): BuildInfo {
  const o = raw && typeof raw === 'object' ? (raw as Record<string, unknown>) : {};
  const pick = (v: unknown, valid: RegExp) => (typeof v === 'string' && valid.test(v) ? v : 'unknown');
  return { commit: pick(o['commit'], COMMIT), date: pick(o['date'], DATE) };
}

/** The build's commit and date, from build-time data. Override in tests. */
export const BUILD_INFO = new InjectionToken<BuildInfo>('BUILD_INFO', {
  providedIn: 'root',
  factory: () => parseBuildInfo(typeof DEBRIEF_BUILD_INFO === 'undefined' ? undefined : DEBRIEF_BUILD_INFO),
});
