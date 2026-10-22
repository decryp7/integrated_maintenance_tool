/** One group of a release's notes (Added, Changed, Fixed…). */
export interface ChangelogGroup {
  readonly title: string;
  readonly items: readonly string[];
}

/** One release of CHANGELOG.md, as `scripts/whats-new.mjs` generated it at build time. */
export interface ChangelogRelease {
  readonly version: string;
  /** `YYYY-MM-DD`, or empty when the heading had none. */
  readonly date: string;
  readonly groups: readonly ChangelogGroup[];
}

/** The release notes of `version`, or null when the changelog has none for it. */
export function releaseFor(changelog: readonly ChangelogRelease[], version: string | null | undefined): ChangelogRelease | null {
  return (version && changelog.find((r) => r.version === version)) || null;
}

/** What the start-up check decides. */
export type WhatsNewDecision = 'show' | 'remember' | 'nothing';

/**
 * Whether to open the panel at start-up. `seen` is the last version the user was shown (null on a first run, which only
 * records the version: a fresh install has nothing "new"); a version without notes shows nothing.
 */
export function decideWhatsNew(seen: string | null, version: string | null | undefined, hasNotes: boolean): WhatsNewDecision {
  if (!version || seen === version) return 'nothing';
  if (seen === null) return 'remember';
  return hasNotes ? 'show' : 'remember';
}
