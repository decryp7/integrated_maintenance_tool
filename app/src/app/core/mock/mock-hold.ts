/**
 * The one release switch of the mocks' holds (browser build and end-to-end tests only): a mock told to hold something
 * (`?mockHold=getProfile` holds an answer, `?mockJobsHold=ai` holds the end of a job) registers here, and the test
 * lets everything held go on with `globalThis.__debriefMockRelease()`, so it can see, and act on, the state before.
 */
const waiting = new Set<() => void>();

/** Calls `release` (once) when the test releases the holds; publishes the release hook. */
export function onMockRelease(release: () => void): void {
  waiting.add(release);
  (globalThis as Record<string, unknown>)['__debriefMockRelease'] = releaseMockHolds;
}

/** Releases every hold registered so far. */
export function releaseMockHolds(): void {
  const all = [...waiting];
  waiting.clear();
  for (const release of all) release();
}
