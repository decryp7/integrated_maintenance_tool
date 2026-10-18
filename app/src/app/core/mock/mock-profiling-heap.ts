import { HeapDiffPage, HeapDiffSort, HeapGrowthPage, HeapObjectRow, HeapObjectsPage, HeapPaths, HeapSort, HeapTypeRow, HeapTypesPage, PathStep } from '../api.types';
import { MockHeapData, MockObject } from './mock-profiling-data';

/**
 * Plausible stand-ins for the engine's heap queries over the synthetic snapshots (browser demo and
 * UI tests only). The graph is tree-shaped apart from extra referrers, so an object's first
 * referrer is its dominator.
 */

export interface HeapIndex {
  data: MockHeapData;
  children: Map<number, number[]>;
  retained: number[];
}

export function indexHeap(data: MockHeapData): HeapIndex {
  const children = new Map<number, number[]>();
  for (const o of data.objects) {
    const parent = o.from[0]?.id;
    if (parent !== undefined) (children.get(parent) ?? children.set(parent, []).get(parent)!).push(o.id);
  }
  const retained = data.objects.map((o) => o.shallow);
  // Objects are created after their dominator, so a reverse pass accumulates subtrees.
  for (let i = data.objects.length - 1; i > 0; i--) {
    const parent = data.objects[i]?.from[0]?.id;
    if (parent !== undefined) retained[parent] = (retained[parent] ?? 0) + (retained[i] ?? 0);
  }
  return { data, children, retained };
}

function typeTable(h: HeapIndex): Map<string, { count: number; bytes: number; retained: number }> {
  const m = new Map<string, { count: number; bytes: number; retained: number }>();
  for (const o of h.data.objects) {
    if (o.id === 0) continue;
    const t = m.get(o.type) ?? m.set(o.type, { count: 0, bytes: 0, retained: 0 }).get(o.type)!;
    t.count++;
    t.bytes += o.shallow;
    t.retained += h.retained[o.id] ?? 0;
  }
  return m;
}

const matches = (name: string, search: string | null | undefined) => !search?.trim() || name.toLowerCase().includes(search.trim().toLowerCase());

export function mockHeapTypes(h: HeapIndex, sort: HeapSort, search: string | null | undefined, offset: number, limit: number): HeapTypesPage {
  const table = typeTable(h);
  const totalBytes = [...table.values()].reduce((a, t) => a + t.bytes, 0);
  const totalCount = [...table.values()].reduce((a, t) => a + t.count, 0);
  const rows: HeapTypeRow[] = [...table.entries()]
    .filter(([name]) => matches(name, search))
    .map(([name, t]) => ({ name, count: t.count, bytes: t.bytes, retained: t.retained, bytesPct: totalBytes ? (t.bytes / totalBytes) * 100 : 0 }));
  const key: Record<HeapSort, (r: HeapTypeRow) => number> = { name: () => 0, count: (r) => r.count, bytes: (r) => r.bytes, retained: (r) => r.retained ?? 0 };
  rows.sort((a, b) => (sort === 'name' ? a.name.localeCompare(b.name) : key[sort](b) - key[sort](a) || a.name.localeCompare(b.name)));
  return { rows: rows.slice(offset, offset + limit), totalRows: rows.length, totalBytes, totalCount, hasGraph: true };
}

export function mockHeapDiff(before: HeapIndex, after: HeapIndex, sort: HeapDiffSort, search: string | null | undefined, offset: number, limit: number): HeapDiffPage {
  const a = typeTable(before);
  const b = typeTable(after);
  const names = new Set([...a.keys(), ...b.keys()]);
  const rows = [...names]
    .filter((n) => matches(n, search))
    .map((name) => {
      const x = a.get(name);
      const y = b.get(name);
      return {
        name,
        countBefore: x?.count ?? 0,
        countAfter: y?.count ?? 0,
        countDelta: (y?.count ?? 0) - (x?.count ?? 0),
        bytesBefore: x?.bytes ?? 0,
        bytesAfter: y?.bytes ?? 0,
        bytesDelta: (y?.bytes ?? 0) - (x?.bytes ?? 0),
        isNew: !x,
        isGone: !y,
      };
    });
  rows.sort((p, q) => (sort === 'name' ? p.name.localeCompare(q.name) : sort === 'countDelta' ? q.countDelta - p.countDelta : q.bytesDelta - p.bytesDelta) || p.name.localeCompare(q.name));
  const sum = (m: Map<string, { count: number; bytes: number }>, k: 'count' | 'bytes') => [...m.values()].reduce((s, t) => s + t[k], 0);
  return { rows: rows.slice(offset, offset + limit), totalRows: rows.length, bytesBefore: sum(a, 'bytes'), bytesAfter: sum(b, 'bytes'), countBefore: sum(a, 'count'), countAfter: sum(b, 'count') };
}

export function mockHeapGrowth(snaps: HeapIndex[], search: string | null | undefined, steadyOnly: boolean, offset: number, limit: number): HeapGrowthPage {
  if (snaps.length < 3) throw new Error('growth needs at least 3 snapshots');
  const tables = snaps.map(typeTable);
  const names = new Set(tables.flatMap((t) => [...t.keys()]));
  let rows = [...names]
    .filter((n) => matches(n, search))
    .map((name) => {
      const bytes = tables.map((t) => t.get(name)?.bytes ?? 0);
      const counts = tables.map((t) => t.get(name)?.count ?? 0);
      const n = bytes.length;
      const mx = (n - 1) / 2;
      const my = bytes.reduce((a, x) => a + x, 0) / n;
      const slopeBytes = bytes.reduce((a, y, i) => a + (i - mx) * (y - my), 0) / bytes.reduce((a, _, i) => a + (i - mx) ** 2, 0);
      const growthBytes = (bytes[n - 1] ?? 0) - (bytes[0] ?? 0);
      const steady = growthBytes > 0 && bytes.every((x, i) => i === 0 || x >= (bytes[i - 1] ?? 0));
      return { name, bytes, counts, slopeBytes, growthBytes, steady };
    });
  const steadyCount = rows.filter((r) => r.steady).length;
  if (steadyOnly) rows = rows.filter((r) => r.steady);
  rows.sort((a, b) => Number(b.steady) - Number(a.steady) || b.slopeBytes - a.slopeBytes || a.name.localeCompare(b.name));
  return { rows: rows.slice(offset, offset + limit), totalRows: rows.length, steadyCount };
}

function objectRow(h: HeapIndex, id: number): HeapObjectRow {
  const o = h.data.objects[id];
  if (!o) throw new Error(`object ${id} is not in this snapshot`);
  return { object: id, typeName: o.type, shallow: o.shallow, retained: h.retained[id] ?? 0, childCount: h.children.get(id)?.length ?? 0 };
}

const byRetained = (h: HeapIndex) => (a: number, b: number) => (h.retained[b] ?? 0) - (h.retained[a] ?? 0) || a - b;

export function mockHeapDominators(h: HeapIndex, parent: number | null, offset: number, limit: number): HeapObjectsPage {
  const ids = [...(h.children.get(parent ?? 0) ?? [])].sort(byRetained(h));
  return { rows: ids.slice(offset, offset + limit).map((id) => objectRow(h, id)), totalRows: ids.length, totalBytes: h.retained[0] ?? 0 };
}

export function mockHeapInstances(h: HeapIndex, typeName: string, offset: number, limit: number): HeapObjectsPage {
  const ids = h.data.objects.filter((o) => o.type === typeName && o.id !== 0).map((o) => o.id).sort(byRetained(h));
  return { rows: ids.slice(offset, offset + limit).map((id) => objectRow(h, id)), totalRows: ids.length, totalBytes: ids.reduce((a, id) => a + (h.data.objects[id]?.shallow ?? 0), 0) };
}

/** Shortest paths from a root-referenced object to `object`, one per referrer of the target. */
export function mockHeapPaths(h: HeapIndex, object: number, maxPaths: number): HeapPaths {
  const target: MockObject | undefined = h.data.objects[object];
  if (!target || object === 0) throw new Error(`object ${object} is not in this snapshot`);
  const chainFrom = (start: { id: number; edge: string }): PathStep[] => {
    if (start.id === 0) return [{ object, typeName: `${target.type} (${start.edge})`, edge: null }];
    const steps: PathStep[] = [{ object, typeName: target.type, edge: start.edge }];
    let cur = start;
    while (cur.id !== 0) {
      const o = h.data.objects[cur.id];
      const up = o?.from[0];
      if (!o || !up) break;
      const step: PathStep = { object: o.id, typeName: o.type, edge: up.id !== 0 ? up.edge : null };
      steps.unshift(step);
      // The first step's edge is null; the root reference becomes part of its label.
      if (up.id === 0) {
        steps[0] = { ...step, edge: null, typeName: `${o.type} (${up.edge})` };
        break;
      }
      cur = up;
    }
    return steps;
  };
  const paths = target.from.map(chainFrom).sort((a, b) => a.length - b.length).slice(0, maxPaths);
  return { paths, unreachable: paths.length === 0, truncated: target.from.length > maxPaths };
}
