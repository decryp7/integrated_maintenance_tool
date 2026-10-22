/** Fixed row height (px) of the legal-text viewer; rows never wrap, so the list can be virtualised. */
export const LINE_HEIGHT = 18;
/** Height (px) of the viewer's viewport. */
export const VIEWPORT_HEIGHT = 320;
const OVERSCAN = 8;

/** Splits a text file into display lines (CRLF tolerant). */
export function toLines(text: string): string[] {
  return text.split(/\r?\n/);
}

/** Lines containing `query` (case-insensitive); all lines for a blank query. */
export function filterLines(lines: readonly string[], query: string): readonly string[] {
  const q = query.trim().toLowerCase();
  return q ? lines.filter((l) => l.toLowerCase().includes(q)) : lines;
}

/** The slice [start, end) of rows to render for a scroll offset, with a little overscan. */
export function visibleRange(scrollTop: number, total: number, viewport = VIEWPORT_HEIGHT, lineHeight = LINE_HEIGHT): { start: number; end: number } {
  const first = Math.floor(Math.max(0, scrollTop) / lineHeight);
  const start = Math.max(0, first - OVERSCAN);
  const end = Math.min(total, first + Math.ceil(viewport / lineHeight) + OVERSCAN);
  return { start: Math.min(start, total), end: Math.max(end, Math.min(start, total)) };
}

/** What "Copy version info" puts on the clipboard. */
export function versionInfoText(version: string | null, platform: string | null, build: { commit?: string; date?: string } | null): string {
  const rows = [`Debrief ${version ?? 'unknown'}`, 'Licence: MIT OR Apache-2.0'];
  if (platform) rows.push(`Platform: ${platform}`);
  const known = (v: string | undefined): v is string => !!v && v !== 'unknown';
  if (known(build?.commit)) rows.push(`Commit: ${build.commit}`);
  if (known(build?.date)) rows.push(`Built: ${build.date}`);
  return rows.join('\n');
}
