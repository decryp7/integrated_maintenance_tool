import { SourceInfo } from '../core/api.types';

/**
 * One expandable row: rolled, dated or rotated files of one log (same `SourceInfo.group`), or the logical sources a
 * profile route split one file into (same `path`, `routedFrom` set; `split`).
 */
export interface SourceGroup {
  key: string;
  /** File name of the log family, e.g. "app.log", or of the split file. */
  label: string;
  /** Members: rotated files newest first; logical sources by name, the file's own source first. */
  sources: SourceInfo[];
  records: number;
  newest: SourceInfo;
  /** Logical sources of one file (ADR 0020) rather than files of one log. */
  split: boolean;
}

export type SourceItem = { kind: 'file'; source: SourceInfo } | { kind: 'group'; group: SourceGroup };

/** Newest first: the latest last event, then the highest id (the file created last). */
function newerFirst(a: SourceInfo, b: SourceInfo): number {
  return (b.to ?? Number.NEGATIVE_INFINITY) - (a.to ?? Number.NEGATIVE_INFINITY) || b.id - a.id;
}

/** The logical sources of each split file (by path), with the file's own source if it kept events. */
function splitFiles(sources: readonly SourceInfo[]): Map<string, SourceInfo[]> {
  const split = new Set(sources.filter((s) => s.routedFrom).map((s) => s.path));
  const byPath = new Map<string, SourceInfo[]>();
  for (const s of sources) {
    if (!split.has(s.path)) continue;
    const list = byPath.get(s.path);
    if (list) list.push(s);
    else byPath.set(s.path, [s]);
  }
  return byPath;
}

/**
 * Collapses sources sharing a group into one item (at the position of the first member), and the logical sources
 * of a split file into one item per file; sources alone in their group, or without one, stay plain files. Pure.
 */
export function groupSources(sources: readonly SourceInfo[]): SourceItem[] {
  const split = splitFiles(sources);
  const members = new Map<string, SourceInfo[]>();
  for (const s of sources) {
    if (!s.group || split.has(s.path)) continue;
    const list = members.get(s.group);
    if (list) list.push(s);
    else members.set(s.group, [s]);
  }
  const items: SourceItem[] = [];
  const emitted = new Set<string>();
  for (const s of sources) {
    const parts = split.get(s.path);
    if (parts) {
      if (emitted.has(`split:${s.path}`)) continue;
      emitted.add(`split:${s.path}`);
      const own = parts.find((p) => !p.routedFrom);
      const label = parts.find((p) => p.routedFrom)?.routedFrom ?? s.name;
      const sorted = [...parts].sort((a, b) => Number(!!a.routedFrom) - Number(!!b.routedFrom) || a.name.localeCompare(b.name));
      items.push({
        kind: 'group',
        group: {
          key: `split:${s.path}`,
          label: label.split(/[\\/]/).filter(Boolean).pop() ?? label,
          sources: sorted,
          records: parts.reduce((a, x) => a + x.records, 0),
          newest: own ?? sorted[0] ?? s,
          split: true,
        },
      });
      continue;
    }
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
        newest: sorted[0] ?? s,
        split: false,
      },
    });
  }
  return items;
}
