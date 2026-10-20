/**
 * The "AI is browsing" banner over a browser window: the shell (`app/src-tauri/src/browser/driving.rs`) loads the
 * app with `#/ai-browsing?host=<label>` in a small window of its own, and `bootstrap.ts` boots the banner alone.
 */
export interface AiBrowsingRoute {
  /** Label of the IMT window whose browser the banner sits on. */
  host: string;
}

const LABEL = /^[a-z0-9][a-z0-9-]{0,63}$/i;

/** Parses the window's location hash into the banner route; null for any other window or anything malformed. */
export function parseAiBrowsingRoute(hash: string): AiBrowsingRoute | null {
  const match = /^#\/ai-browsing\?(.*)$/.exec(hash ?? '');
  if (!match) return null;
  const host = new URLSearchParams(match[1]).get('host') ?? '';
  return LABEL.test(host) ? { host } : null;
}
