import { RecordView } from '../../core/api.types';
import { valueToString } from '../../core/format';

export type RegistryNodeKind = 'hive' | 'key' | 'value';

export interface RegistryNode {
  /** Unique within the tree; stable across rebuilds. */
  id: string;
  kind: RegistryNodeKind;
  label: string;
  /** `Hive\Key[\ValueName]`, what "copy path" copies. */
  path: string;
  children: RegistryNode[];
  /** The record behind a value (or a key line), selected in the inspector. */
  record?: RecordView;
  valueType?: string;
  /** Data rendered on one line. */
  data?: string;
  /** True for a key (not a value) that the export deletes (`[-HKEY...\\Key]`). */
  deleted?: boolean;
}

/** A visible row of the flattened tree, with the ARIA tree metadata. */
export interface FlatNode {
  node: RegistryNode;
  level: number;
  posInSet: number;
  setSize: number;
  expandable: boolean;
  expanded: boolean;
}

/** Maximum rows read from the backend; more are ignored with a notice. */
export const REGISTRY_ROW_CAP = 50_000;

function text(v: unknown): string {
  return typeof v === 'string' ? v : v === undefined || v === null ? '' : valueToString(v);
}

function isDeletedKeyLine(r: RecordView): boolean {
  return r.fields['ValueName'] === undefined && r.fields['ValueType'] === 'DELETE';
}

/** One-line rendering of a value's data (REG_MULTI_SZ arrays and multi-line strings are joined with " | "). */
export function dataText(v: unknown): string {
  if (Array.isArray(v)) return v.map((x) => text(x)).join(' | ');
  return text(v).replace(/\r?\n/g, ' | ');
}

/**
 * Builds Hive → Key segments → values from records produced by the "registry" importer.
 * Rows without a `Hive` or `Key` are ignored. Siblings are sorted: keys first, then values, by name.
 */
export function buildRegistryTree(rows: readonly RecordView[]): RegistryNode[] {
  const hives = new Map<string, RegistryNode>();
  const keyNode = (hive: string, key: string): RegistryNode => {
    let node: RegistryNode = hives.get(hive) ?? { id: hive, kind: 'hive', label: hive, path: hive, children: [] };
    hives.set(hive, node);
    for (const segment of key.split('\\').filter(Boolean)) {
      const id = `${node.id}\\${segment}`;
      const parent: RegistryNode = node;
      node = parent.children.find((c) => c.id === id) ?? { id, kind: 'key', label: segment, path: id, children: [] };
      if (!parent.children.includes(node)) parent.children.push(node);
    }
    return node;
  };

  for (const r of rows) {
    const hive = text(r.fields['Hive']);
    if (!hive) continue;
    const key = text(r.fields['Key']);
    const parent = keyNode(hive, key);
    const name = r.fields['ValueName'];
    if (name === undefined) {
      parent.record = r;
      if (isDeletedKeyLine(r)) parent.deleted = true;
      continue;
    }
    const label = text(name);
    const id = `${parent.id}#${label}`;
    if (parent.children.some((c) => c.id === id)) continue;
    parent.children.push({
      id,
      kind: 'value',
      label,
      path: `${parent.path}\\${label}`,
      children: [],
      record: r,
      valueType: text(r.fields['ValueType']),
      data: dataText(r.fields['Data']),
    });
  }

  const sort = (nodes: RegistryNode[]): void => {
    nodes.sort((a, b) => (a.kind === b.kind ? a.label.localeCompare(b.label) : a.kind === 'value' ? 1 : -1));
    nodes.forEach((n) => sort(n.children));
  };
  const roots = [...hives.values()];
  sort(roots);
  return roots;
}

function matches(node: RegistryNode, needle: string): boolean {
  return node.label.toLowerCase().includes(needle) || (node.data ?? '').toLowerCase().includes(needle) || (node.valueType ?? '').toLowerCase().includes(needle);
}

/** Ids of nodes that match the filter themselves or contain a match (these stay visible). */
function visibleIds(roots: readonly RegistryNode[], needle: string): Set<string> {
  const keep = new Set<string>();
  const visit = (n: RegistryNode): boolean => {
    let any = matches(n, needle);
    for (const c of n.children) any = visit(c) || any;
    if (any) keep.add(n.id);
    return any;
  };
  roots.forEach(visit);
  return keep;
}

/**
 * Flattens the tree to the rows currently visible. With a filter, only matching nodes and their
 * ancestors remain, and ancestors are shown expanded so the matches are visible.
 */
export function flattenTree(roots: readonly RegistryNode[], expanded: ReadonlySet<string>, filter = ''): FlatNode[] {
  const needle = filter.trim().toLowerCase();
  const keep = needle ? visibleIds(roots, needle) : null;
  const out: FlatNode[] = [];
  const walk = (nodes: readonly RegistryNode[], level: number): void => {
    const shown = keep ? nodes.filter((n) => keep.has(n.id)) : nodes;
    shown.forEach((node, i) => {
      const children = keep ? node.children.filter((c) => keep.has(c.id)) : node.children;
      const expandable = children.length > 0;
      const isExpanded = expandable && (keep ? true : expanded.has(node.id));
      out.push({ node, level, posInSet: i + 1, setSize: shown.length, expandable, expanded: isExpanded });
      if (isExpanded) walk(node.children, level + 1);
    });
  };
  walk(roots, 1);
  return out;
}

/** Ids of every node that has children (used to expand all). */
export function expandableIds(roots: readonly RegistryNode[]): string[] {
  const ids: string[] = [];
  const visit = (n: RegistryNode) => {
    if (n.children.length) ids.push(n.id);
    n.children.forEach(visit);
  };
  roots.forEach(visit);
  return ids;
}
