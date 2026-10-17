import { SourceInfo } from '../core/api.types';

/** Rolled, dated or rotated files of one log (same `SourceInfo.group`), shown as one expandable row. */
export interface SourceGroup {
  key: string;
  /** File name of the log family, e.g. "app.log". */
  label: string;
  /** Members, newest first. */
  sources: SourceInfo[];
  records: number;
  newest: SourceInfo;
}

export type SourceItem = { kind: 'file'; source: SourceInfo } | { kind: 'group'; group: SourceGroup };

/** Newest first: the latest last event, then the highest id (the file created last). */
function newerFirst(a: SourceInfo, b: SourceInfo): number {
  return (b.to ?? Number.NEGATIVE_INFINITY) - (a.to ?? Number.NEGATIVE_INFINITY) || b.id - a.id;
}

/**
 * Collapses sources sharing a group into one item (at the position of the first member); sources alone in
 * their group, or without one, stay plain files. Pure.
 */
export function groupSources(sources: readonly SourceInfo[]): SourceItem[] {
  const members = new Map<string, SourceInfo[]>();
  for (const s of sources) {
    if (!s.group) continue;
    const list = members.get(s.group);
    if (list) list.push(s);
    else members.set(s.group, [s]);
  }
  const items: SourceItem[] = [];
  const emitted = new Set<string>();
  for (const s of sources) {
    const list = s.group ? members.get(s.group) : undefined;
    if (!list || list.length < 2) {
      items.push({ kind: 'file', source: s });
      continue;
    }
    if (emitted.has(s.group)) continue;
    emitted.add(s.group);
    const sorted = [...list].sort(newerFirst);
    items.push({
      kind: 'group',
      group: {
        key: s.group,
        label: s.group.split(/[\\/]/).filter(Boolean).pop() ?? s.group,
        sources: sorted,
        records: list.reduce((a, x) => a + x.records, 0),
        newest: sorted[0],
      },
    });
  }
  return items;
}
