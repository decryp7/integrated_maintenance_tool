/**
 * The built-in marker icon set (names must match `MARKER_ICONS` in crates/debrief-core/src/profile.rs).
 * Simple stroke icons on a 24 grid, drawn with `currentColor`; each is a single SVG path string.
 */
export interface BuiltinIcon {
  name: string;
  d: string;
  /** Extra search words. */
  keywords: string;
}

const c = (cx: number, cy: number, r: number) => `M${cx - r} ${cy}a${r} ${r} 0 1 0 ${2 * r} 0a${r} ${r} 0 1 0 ${-2 * r} 0`;

export const BUILTIN_ICONS: readonly BuiltinIcon[] = [
  { name: 'alert', d: 'M12 3.5L2.8 19.5h18.4zM12 9.5v4.5M12 17v.01', keywords: 'warning error problem triangle' },
  { name: 'bug', d: 'M8.5 9a3.5 3.5 0 0 1 7 0v5.5a3.5 3.5 0 0 1-7 0zM8.5 11H4.5M15.5 11h4M8.5 15H5M15.5 15H19M9.5 6L7.5 3.5M14.5 6l2-2.5M12 9v10', keywords: 'defect insect issue' },
  { name: 'flag', d: 'M5 21V4M5 4h12l-2.5 4.5L17 13H5', keywords: 'milestone note' },
  { name: 'star', d: 'M12 3l2.7 5.6 6.1.9-4.4 4.3 1 6.1L12 17l-5.4 2.9 1-6.1L3.2 9.5l6.1-.9z', keywords: 'favourite favorite important' },
  { name: 'pin', d: `M12 21s-6.5-6-6.5-11a6.5 6.5 0 0 1 13 0c0 5-6.5 11-6.5 11z${c(12, 10, 2.5)}`, keywords: 'location map place' },
  { name: 'bookmark', d: 'M6.5 3.5h11v17.5L12 16.5 6.5 21z', keywords: 'save remember' },
  { name: 'info', d: `${c(12, 12, 9)}M12 11v6M12 7.5v.01`, keywords: 'information note help' },
  { name: 'check', d: `${c(12, 12, 9)}M8 12.5l3 3 5-6`, keywords: 'ok success done tick' },
  { name: 'x-circle', d: `${c(12, 12, 9)}M9 9l6 6M15 9l-6 6`, keywords: 'fail failure cancel close' },
  { name: 'user', d: `${c(12, 8, 4)}M4.5 20.5a7.5 7.5 0 0 1 15 0`, keywords: 'person account login' },
  { name: 'users', d: `${c(9, 8.5, 3.5)}M2.5 20a6.5 6.5 0 0 1 13 0M16 5.5a3.2 3.2 0 0 1 0 6.2M18 14a6 6 0 0 1 3.5 6`, keywords: 'people team group' },
  { name: 'lock', d: 'M5.5 11h13v9.5h-13zM8 11V8a4 4 0 0 1 8 0v3M12 15v2', keywords: 'security private auth' },
  { name: 'key', d: `${c(8, 15.5, 4)}M11 12.5L20 3.5M16 7.5l2.5 2.5M18.5 5l2 2`, keywords: 'secret token credential' },
  { name: 'shield', d: 'M12 3l8 3v5.5c0 5-3.5 8.3-8 9.5-4.5-1.2-8-4.5-8-9.5V6zM8.5 12l2.5 2.5 4.5-5', keywords: 'security protect firewall' },
  { name: 'database', d: 'M4 6c0-1.7 3.6-3 8-3s8 1.3 8 3-3.6 3-8 3-8-1.3-8-3zM4 6v12c0 1.7 3.6 3 8 3s8-1.3 8-3V6M4 12c0 1.7 3.6 3 8 3s8-1.3 8-3', keywords: 'sql db storage' },
  { name: 'network', d: 'M9.5 3h5v5h-5zM3 16h5v5H3zM16 16h5v5h-5zM12 8v4M5.5 16v-4h13v4', keywords: 'connection topology tcp' },
  { name: 'cloud', d: 'M7 18a4.5 4.5 0 0 1-.6-8.96A6 6 0 0 1 17.9 9.2 4.4 4.4 0 0 1 17 18z', keywords: 'azure aws remote' },
  { name: 'server', d: 'M3.5 4h17v6h-17zM3.5 14h17v6h-17zM7 7h.01M7 17h.01', keywords: 'host machine node' },
  { name: 'credit-card', d: 'M3 6h18v12H3zM3 10h18M6.5 15h3', keywords: 'payment billing' },
  { name: 'cart', d: `M2.5 4h3l2.6 11h10.4l2-8H6.3${c(10, 19.5, 1)}${c(17, 19.5, 1)}`, keywords: 'order shop checkout' },
  { name: 'mail', d: 'M3 5.5h18v13H3zM3.5 6.5L12 13l8.5-6.5', keywords: 'email smtp message' },
  { name: 'clock', d: `${c(12, 12, 9)}M12 7v5l3.5 2`, keywords: 'time schedule' },
  { name: 'timer', d: `M9.5 2.5h5${c(12, 14, 7.5)}M12 10v4.5M18 6.5l1.5-1.5`, keywords: 'timeout slow duration' },
  { name: 'rocket', d: `M12 2.5c3.5 2 5 6 4.5 10.5H7.5C7 8.5 8.5 4.5 12 2.5zM7.5 13L5 17h4.5M16.5 13L19 17h-4.5${c(12, 9, 1.5)}M10.5 17c.3 2 .8 3 1.5 4 .7-1 1.2-2 1.5-4`, keywords: 'deploy release launch' },
  { name: 'power', d: 'M12 3v8.5M7 6.2a8 8 0 1 0 10 0', keywords: 'shutdown on off' },
  { name: 'restart', d: 'M19.5 12a7.5 7.5 0 1 1-2.2-5.3M19.5 4v4.5H15', keywords: 'reboot reload retry' },
  { name: 'cpu', d: 'M6.5 6.5h11v11h-11zM9.5 9.5h5v5h-5zM9 3v3.5M15 3v3.5M9 17.5V21M15 17.5V21M3 9h3.5M3 15h3.5M17.5 9H21M17.5 15H21', keywords: 'processor load' },
  { name: 'memory', d: 'M2.5 7.5h19v9h-19zM6 16.5V19M10 16.5V19M14 16.5V19M18 16.5V19M6 11h3M11 11h3M16 11h2', keywords: 'ram heap leak' },
  { name: 'disk', d: 'M3.5 12l3-7h11l3 7v6.5h-17zM3.5 12h17M16.5 15.5h.01M13 15.5h.01', keywords: 'drive storage io' },
  { name: 'file', d: 'M6 3h8l4 4v14H6zM14 3v4h4M9 12h6M9 16h6', keywords: 'document log' },
  { name: 'download', d: 'M12 4v11M7.5 10.5L12 15l4.5-4.5M4.5 19.5h15', keywords: 'receive pull' },
  { name: 'upload', d: 'M12 15V4M7.5 8.5L12 4l4.5 4.5M4.5 19.5h15', keywords: 'send push' },
  { name: 'link', d: 'M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1', keywords: 'url reference chain' },
  { name: 'zap', d: 'M13 2.5L4.5 13.5H12l-1 8 8.5-11H12z', keywords: 'lightning power spike' },
  { name: 'fire', d: 'M12 21c-3.9 0-6.5-2.6-6.5-6 0-3 2-4.8 3.5-7 .5 1.6 1.4 2.5 2.4 3 0-3 1.4-5.6 3.6-7.5.3 3.3 4 5.4 4 11.5 0 3.4-2.6 6-6.5 6z', keywords: 'hot critical burn' },
  { name: 'heart', d: 'M12 20.5S3.5 15 3.5 8.8A4.6 4.6 0 0 1 12 6.6a4.6 4.6 0 0 1 8.5 2.2C20.5 15 12 20.5 12 20.5z', keywords: 'health like' },
];

const BY_NAME = new Map(BUILTIN_ICONS.map((i) => [i.name, i]));

export function builtinIcon(name: string | null | undefined): BuiltinIcon | undefined {
  return name ? BY_NAME.get(name) : undefined;
}

/** Icons whose name or keywords contain every word of `query` (all icons for an empty query). */
export function searchIcons(query: string): BuiltinIcon[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return [...BUILTIN_ICONS];
  return BUILTIN_ICONS.filter((i) => words.every((w) => i.name.includes(w) || i.keywords.includes(w)));
}
