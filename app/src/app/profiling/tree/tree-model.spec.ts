import { describe, expect, it } from 'vitest';
import { FrameDto, TreeNode } from '../../core/api.types';
import { flattenTree, keyOf, ROOT_KEY, TreeEntry, TreeRow, treeKey } from './tree-model';

const frame = (id: number): FrameDto => ({ id, name: `f${id}`, module: null, file: null, line: null, kind: 'managed', isFramework: false });
const node = (id: number, hasChildren: boolean): TreeNode => ({ frame: frame(id), self: {}, total: {}, selfPct: 0, totalPct: 0, hasChildren });
function entry(path: number[], hasChildren: boolean, patch: Partial<TreeEntry> = {}): TreeEntry {
  return { key: keyOf(path), path, node: node(path[path.length - 1] ?? -1, hasChildren), children: null, totalChildren: 0, loading: false, error: null, ...patch };
}
const root = (children: string[] | null, patch: Partial<TreeEntry> = {}) => entry([], false, { key: ROOT_KEY, children, totalChildren: children?.length ?? 0, ...patch });

describe('flattenTree', () => {
  it('keyOf joins paths', () => {
    expect(keyOf([])).toBe('');
    expect(keyOf([1, 2, 3])).toBe('1/2/3');
  });

  it('returns nothing without a root entry', () => {
    expect(flattenTree({}, new Set())).toEqual([]);
  });

  it('lists top-level nodes collapsed, with position and set size', () => {
    const rows = flattenTree({ '': root(['1', '2']), '1': entry([1], true), '2': entry([2], false) }, new Set());
    expect(rows.map((r) => r.key)).toEqual(['1', '2']);
    const r0 = rows[0];
    expect(r0.kind === 'node' && [r0.level, r0.posinset, r0.setsize, r0.expanded, r0.parent]).toEqual([1, 1, 2, false, '']);
  });

  it('descends into expanded nodes that have children and loaded entries', () => {
    const entries = {
      '': root(['1', '2', 'gone']),
      '1': entry([1], true, { children: ['1/5'], totalChildren: 1 }),
      '1/5': entry([1, 5], false),
      '2': entry([2], false, { children: ['2/9'] }),
      '2/9': entry([2, 9], false),
    };
    const rows = flattenTree(entries, new Set(['1', '2']));
    // '2' is expanded but has no children; 'gone' has no entry.
    expect(rows.map((r) => r.key)).toEqual(['1', '1/5', '2']);
    const child = rows[1];
    expect(child.kind === 'node' && [child.level, child.parent]).toEqual([2, '1']);
    const two = rows[2];
    expect(two.kind === 'node' && two.expanded).toBe(false);
  });

  it('adds a load-more row when only part of the children are loaded', () => {
    const rows = flattenTree({ '': root(['1'], { totalChildren: 5 }), '1': entry([1], false) }, new Set());
    expect(rows[1]).toEqual({ kind: 'more', key: '#more', parent: '', level: 1, remaining: 4 });
  });

  it('adds no load-more row while children were never requested', () => {
    expect(flattenTree({ '': root(null, { totalChildren: 3 }) }, new Set())).toEqual([]);
  });

  it('adds loading and error status rows instead of load-more', () => {
    const loading = flattenTree({ '': root(['1'], { loading: true, totalChildren: 9 }), '1': entry([1], false) }, new Set());
    expect(loading[1]).toEqual({ kind: 'status', key: '#status', parent: '', level: 1, loading: true, error: null });
    const failed = flattenTree({ '': root(null, { error: 'boom' }) }, new Set());
    expect(failed).toEqual([{ kind: 'status', key: '#status', parent: '', level: 1, loading: false, error: 'boom' }]);
  });

  it('puts the status row of an expanded child after its nodes at the deeper level', () => {
    const entries = {
      '': root(['1']),
      '1': entry([1], true, { children: [], loading: true }),
    };
    const rows = flattenTree(entries, new Set(['1']));
    expect(rows.map((r) => r.kind)).toEqual(['node', 'status']);
    expect(rows[1].level).toBe(2);
  });
});

describe('treeKey', () => {
  const entries = {
    '': root(['1', '2'], { totalChildren: 3 }),
    '1': entry([1], true, { children: ['1/5'], totalChildren: 1 }),
    '1/5': entry([1, 5], false),
    '2': entry([2], false),
  };
  const rows = (open: string[] = []): TreeRow[] => flattenTree(entries, new Set(open));

  it('does nothing on an empty tree', () => {
    expect(treeKey([], 0, 'ArrowDown')).toBeNull();
  });

  it('moves focus with Up/Down/Home/End and clamps at the edges', () => {
    const r = rows(['1']); // 1, 1/5, 2, more
    expect(r).toHaveLength(4);
    expect(treeKey(r, 0, 'ArrowDown')).toEqual({ type: 'focus', index: 1 });
    expect(treeKey(r, 3, 'ArrowDown')).toEqual({ type: 'focus', index: 3 });
    expect(treeKey(r, 2, 'ArrowUp')).toEqual({ type: 'focus', index: 1 });
    expect(treeKey(r, 0, 'ArrowUp')).toEqual({ type: 'focus', index: 0 });
    expect(treeKey(r, 2, 'Home')).toEqual({ type: 'focus', index: 0 });
    expect(treeKey(r, 0, 'End')).toEqual({ type: 'focus', index: 3 });
  });

  it('clamps an out-of-range index', () => {
    const r = rows();
    expect(treeKey(r, 99, 'ArrowUp')).toEqual({ type: 'focus', index: r.length - 2 });
    expect(treeKey(r, -5, 'ArrowDown')).toEqual({ type: 'focus', index: 1 });
  });

  it('ArrowRight expands, then steps into the first child; leaves and non-nodes do nothing', () => {
    expect(treeKey(rows(), 0, 'ArrowRight')).toEqual({ type: 'expand', key: '1' });
    expect(treeKey(rows(['1']), 0, 'ArrowRight')).toEqual({ type: 'focus', index: 1 });
    expect(treeKey(rows(), 1, 'ArrowRight')).toBeNull(); // leaf
    expect(treeKey(rows(), 2, 'ArrowRight')).toBeNull(); // more row
  });

  it('ArrowLeft collapses an expanded node, otherwise goes to the parent', () => {
    const r = rows(['1']);
    expect(treeKey(r, 0, 'ArrowLeft')).toEqual({ type: 'collapse', key: '1' });
    expect(treeKey(r, 1, 'ArrowLeft')).toEqual({ type: 'focus', index: 0 });
    expect(treeKey(r, 2, 'ArrowLeft')).toBeNull(); // top level: parent is the synthetic root
    expect(treeKey(rows(), 0, 'ArrowLeft')).toBeNull();
  });

  it('Enter and space activate the focused row', () => {
    expect(treeKey(rows(), 1, 'Enter')).toEqual({ type: 'activate', index: 1 });
    expect(treeKey(rows(), 0, ' ')).toEqual({ type: 'activate', index: 0 });
  });

  it("'*' expands the hot path of a node only", () => {
    expect(treeKey(rows(), 0, '*')).toEqual({ type: 'hotPath', key: '1' });
    expect(treeKey(rows(), 2, '*')).toBeNull();
  });

  it('ignores other keys', () => {
    expect(treeKey(rows(), 0, 'x')).toBeNull();
  });
});
