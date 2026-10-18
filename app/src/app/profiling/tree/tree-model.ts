import { TreeNode } from '../../core/api.types';

/** One loaded node of a lazily paged call tree. */
export interface TreeEntry {
  key: string;
  path: number[];
  node: TreeNode;
  /** Child keys loaded so far; null = never requested. */
  children: string[] | null;
  totalChildren: number;
  loading: boolean;
  error: string | null;
}

/** The (synthetic) root holding the top-level nodes. */
export const ROOT_KEY = '';

export const keyOf = (path: readonly number[]): string => path.join('/');

export type TreeRow =
  | { kind: 'node'; key: string; entry: TreeEntry; level: number; posinset: number; setsize: number; expanded: boolean; parent: string }
  | { kind: 'more'; key: string; parent: string; level: number; remaining: number }
  | { kind: 'status'; key: string; parent: string; level: number; loading: boolean; error: string | null };

/**
 * The visible rows of the tree (depth-first over expanded nodes), with "load more" and
 * loading/error rows. Pure: the treegrid renders exactly this list.
 */
export function flattenTree(entries: Readonly<Record<string, TreeEntry>>, expanded: ReadonlySet<string>): TreeRow[] {
  const rows: TreeRow[] = [];
  const walk = (parentKey: string, level: number) => {
    const parent = entries[parentKey];
    if (!parent) return;
    const kids = parent.children ?? [];
    kids.forEach((k, i) => {
      const entry = entries[k];
      if (!entry) return;
      const open = expanded.has(k) && entry.node.hasChildren;
      rows.push({ kind: 'node', key: k, entry, level, posinset: i + 1, setsize: parent.totalChildren, expanded: open, parent: parentKey });
      if (open) walk(k, level + 1);
    });
    if (parent.loading || parent.error) rows.push({ kind: 'status', key: `${parentKey}#status`, parent: parentKey, level, loading: parent.loading, error: parent.error });
    else if (parent.children && parent.totalChildren > kids.length) rows.push({ kind: 'more', key: `${parentKey}#more`, parent: parentKey, level, remaining: parent.totalChildren - kids.length });
  };
  walk(ROOT_KEY, 1);
  return rows;
}

/** What a key press in the treegrid does. */
export type TreeKeyAction =
  | { type: 'focus'; index: number }
  | { type: 'expand'; key: string }
  | { type: 'collapse'; key: string }
  | { type: 'activate'; index: number }
  | { type: 'hotPath'; key: string }
  | null;

/** WAI-ARIA treegrid keyboard model over the flattened rows. */
export function treeKey(rows: readonly TreeRow[], index: number, key: string): TreeKeyAction {
  if (!rows.length) return null;
  const i = Math.max(0, Math.min(rows.length - 1, index));
  const row = rows[i];
  if (!row) return null;
  switch (key) {
    case 'ArrowDown':
      return { type: 'focus', index: Math.min(rows.length - 1, i + 1) };
    case 'ArrowUp':
      return { type: 'focus', index: Math.max(0, i - 1) };
    case 'Home':
      return { type: 'focus', index: 0 };
    case 'End':
      return { type: 'focus', index: rows.length - 1 };
    case 'ArrowRight':
      if (row.kind !== 'node' || !row.entry.node.hasChildren) return null;
      return row.expanded ? { type: 'focus', index: Math.min(rows.length - 1, i + 1) } : { type: 'expand', key: row.key };
    case 'ArrowLeft': {
      if (row.kind === 'node' && row.expanded) return { type: 'collapse', key: row.key };
      const parent = rows.findIndex((r) => r.kind === 'node' && r.key === row.parent);
      return parent >= 0 ? { type: 'focus', index: parent } : null;
    }
    case 'Enter':
    case ' ':
      return { type: 'activate', index: i };
    case '*':
      return row.kind === 'node' ? { type: 'hotPath', key: row.key } : null;
    default:
      return null;
  }
}
