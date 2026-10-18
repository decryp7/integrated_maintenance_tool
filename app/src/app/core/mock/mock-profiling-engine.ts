import {
  Butterfly,
  ButterflyRow,
  DiffSort,
  FlameGraph,
  FlameRect,
  FrameDto,
  FramePaths,
  FunctionDiffPage,
  FunctionDiffRow,
  HotPath,
  HotspotRow,
  HotspotsPage,
  MetricId,
  MetricValues,
  ProfileFinding,
  ProfileGroupBy,
  ProfileSortSpec,
  ProfileView,
  ThreadTimeline,
  TreeDirection,
  TreeNode,
  TreePage,
} from '../api.types';
import { MockProfileData, MockSample } from './mock-profiling-data';
import { ProfilingFailure } from '../../profiling/profiling-errors';

/**
 * Plausible stand-ins for the engine's profile queries over the synthetic data. Only for the
 * browser demo and UI tests: the reference behaviour is `crates/imt-profile`, not this file.
 */

type Values = MetricValues;

function add(into: Values, v: Values): void {
  for (const k of Object.keys(v) as MetricId[]) {
    if (k === 'maxCallNs') into[k] = Math.max(into[k] ?? 0, v[k] ?? 0);
    else into[k] = (into[k] ?? 0) + (v[k] ?? 0);
  }
}

const zero = (p: MockProfileData): Values => Object.fromEntries(p.metrics.map((m) => [m.id, 0]));

function wildcard(pattern: string): RegExp {
  return new RegExp('^' + pattern.split('*').map((s) => s.replace(/[.+?^${}()|[\]\\]/g, '\\$&')).join('.*') + '$', 'i');
}

/** Applies just-my-code and recursion folding to one stack (root first). */
export function transformStack(stack: number[], p: MockProfileData, view: ProfileView): number[] {
  let out = stack;
  const jmc = view.justMyCode;
  if (jmc && jmc.enabled !== false) {
    const patterns = (jmc.frameworkPatterns ?? []).map(wildcard);
    const isFw = (id: number) => {
      const f = p.frames.find((x) => x.id === id);
      if (!f) return false;
      return (jmc.useFrameFlags !== false && f.isFramework) || patterns.some((r) => r.test(f.name) || (f.module !== null && r.test(f.module)));
    };
    out = out.filter((id, i) => {
      const prev = out[i - 1];
      return !(isFw(id) && prev !== undefined && isFw(prev));
    });
  }
  if (view.recursion === 'direct') out = out.filter((id, i) => i === 0 || out[i - 1] !== id);
  else if (view.recursion === 'all') {
    const folded: number[] = [];
    for (const id of out) {
      const at = folded.indexOf(id);
      if (at >= 0) folded.length = at + 1;
      else folded.push(id);
    }
    out = folded;
  }
  return out;
}

/** Samples inside the view's range and threads, with transformed stacks. */
export function viewSamples(p: MockProfileData, view: ProfileView = {}): MockSample[] {
  const r = view.range;
  const threads = view.threads?.length ? new Set(view.threads) : null;
  return p.samples
    .filter((s) => (!r || (s.t >= r.fromNs && s.t <= r.toNs)) && (!threads || threads.has(s.thread)))
    .map((s) => ({ ...s, stack: transformStack(s.stack, p, view) }));
}

export function grandTotal(p: MockProfileData, samples: MockSample[]): Values {
  const g = zero(p);
  for (const s of samples) add(g, s.v);
  return g;
}

interface Node {
  frame: number;
  self: Values;
  total: Values;
  children: Map<number, Node>;
}

function buildTree(p: MockProfileData, samples: MockSample[], direction: TreeDirection): Node {
  const root: Node = { frame: -1, self: zero(p), total: zero(p), children: new Map() };
  for (const s of samples) {
    const stack = direction === 'bottomUp' ? [...s.stack].reverse() : s.stack;
    let node = root;
    add(root.total, s.v);
    for (const f of stack) {
      let child = node.children.get(f);
      if (!child) node.children.set(f, (child = { frame: f, self: zero(p), total: zero(p), children: new Map() }));
      add(child.total, s.v);
      node = child;
    }
    add(direction === 'bottomUp' ? (root.children.get(stack[0] ?? -1) ?? root).self : node.self, s.v);
  }
  return root;
}

function nodeAt(root: Node, path: number[]): Node {
  let node = root;
  for (const f of path) {
    const next = node.children.get(f);
    if (!next) throw new Error(`no call-tree node at path [${path.join(', ')}] in this view`);
    node = next;
  }
  return node;
}

const pct = (v: number | undefined, total: number | undefined) => (total ? ((v ?? 0) / total) * 100 : 0);

export function frameOf(p: MockProfileData, id: number): FrameDto {
  const f = p.frames.find((x) => x.id === id);
  if (!f) throw new Error(`unknown frame ${id}`);
  return f;
}

function treeNode(p: MockProfileData, n: Node, g: Values, metric: MetricId): TreeNode {
  return { frame: frameOf(p, n.frame), self: n.self, total: n.total, selfPct: pct(n.self[metric], g[metric]), totalPct: pct(n.total[metric], g[metric]), hasChildren: n.children.size > 0 };
}

function sortedChildren(n: Node, metric: MetricId): Node[] {
  return [...n.children.values()].sort((a, b) => (b.total[metric] ?? 0) - (a.total[metric] ?? 0) || a.frame - b.frame);
}

export function mockTree(p: MockProfileData, view: ProfileView | undefined, direction: TreeDirection, path: number[], metric: MetricId, offset: number, limit: number): TreePage {
  const samples = viewSamples(p, view);
  const g = grandTotal(p, samples);
  const node = nodeAt(buildTree(p, samples, direction), path);
  const kids = sortedChildren(node, metric);
  return { nodes: kids.slice(offset, offset + limit).map((n) => treeNode(p, n, g, metric)), totalChildren: kids.length, grandTotal: g, metric, measurement: measurement(p, metric) };
}

export function mockHotPath(p: MockProfileData, view: ProfileView | undefined, direction: TreeDirection, path: number[], metric: MetricId, minFraction: number): HotPath {
  const samples = viewSamples(p, view);
  const g = grandTotal(p, samples);
  let node = nodeAt(buildTree(p, samples, direction), path);
  const full = [...path];
  const nodes: TreeNode[] = [];
  for (;;) {
    const next = sortedChildren(node, metric)[0];
    if (!next || (next.total[metric] ?? 0) < minFraction * (g[metric] ?? 0)) break;
    full.push(next.frame);
    nodes.push(treeNode(p, next, g, metric));
    node = next;
  }
  return { path: full, nodes };
}

export function measurement(p: MockProfileData, metric: MetricId) {
  return p.metrics.find((m) => m.id === metric)?.measurement ?? 'sampled';
}

function groupKey(p: MockProfileData, f: FrameDto, by: ProfileGroupBy, thread: number): { key: string; name: string } {
  switch (by) {
    case 'module':
      return { key: f.module ?? '(unknown)', name: f.module ?? '(unknown module)' };
    case 'type': {
      const t = f.name.includes('.') ? f.name.slice(0, f.name.lastIndexOf('.')) : f.name;
      return { key: t, name: t };
    }
    case 'namespace': {
      const parts = f.name.split('.');
      const ns = parts.length > 2 ? parts.slice(0, -2).join('.') : (f.module ?? f.name);
      return { key: ns, name: ns };
    }
    case 'thread': {
      const t = p.threads.find((x) => x.id === thread);
      return { key: `t${thread}`, name: t?.name ?? `Thread ${thread}` };
    }
    default:
      return { key: `f${f.id}`, name: f.name };
  }
}

export function hotspotRows(p: MockProfileData, view: ProfileView | undefined, groupBy: ProfileGroupBy, metric: MetricId): { rows: HotspotRow[]; grand: Values; samples: number } {
  const samples = viewSamples(p, view);
  const grand = grandTotal(p, samples);
  const rows = new Map<string, HotspotRow>();
  for (const s of samples) {
    const seen = new Set<string>();
    s.stack.forEach((id, i) => {
      const f = frameOf(p, id);
      const { key, name } = groupKey(p, f, groupBy, s.thread);
      let row = rows.get(key);
      if (!row) {
        row = { key, name, module: groupBy === 'function' ? f.module : null, frame: groupBy === 'function' ? f : null, thread: groupBy === 'thread' ? s.thread : null, self: zero(p), total: zero(p), selfPct: 0, totalPct: 0, avgPerCallNs: null, maxPerCallNs: null };
        rows.set(key, row);
      }
      if (!seen.has(key)) {
        seen.add(key);
        add(row.total, s.v);
      }
      if (i === s.stack.length - 1) add(row.self, s.v);
    });
  }
  for (const r of rows.values()) {
    r.selfPct = pct(r.self[metric], grand[metric]);
    r.totalPct = pct(r.total[metric], grand[metric]);
    if (r.total.calls) {
      r.avgPerCallNs = (r.total.timeNs ?? 0) / r.total.calls;
      r.maxPerCallNs = r.total.maxCallNs ?? null;
    }
  }
  return { rows: [...rows.values()], grand, samples: samples.length };
}

export function mockHotspots(
  p: MockProfileData,
  view: ProfileView | undefined,
  groupBy: ProfileGroupBy,
  metric: MetricId,
  sort: ProfileSortSpec | null | undefined,
  search: string | null | undefined,
  offset: number,
  limit: number,
): HotspotsPage {
  const { rows, grand, samples } = hotspotRows(p, view, groupBy, metric);
  const needle = search?.trim().toLowerCase();
  let list = needle ? rows.filter((r) => r.name.toLowerCase().includes(needle) || (r.module ?? '').toLowerCase().includes(needle)) : rows;
  const by = sort?.metric === undefined ? metric : sort.metric;
  const of = sort?.of ?? 'self';
  const desc = sort?.descending ?? by !== null;
  if (of === 'avgPerCall' && by !== null) {
    if (!p.metrics.some((m) => m.id === 'calls')) throw new ProfilingFailure('unsupported', 'this profile has no call counts, so there is no average per call');
    const avg = (r: HotspotRow) => (r.total.calls ? (r.total[by] ?? 0) / r.total.calls : null);
    list = [...list].sort((a, b) => {
      const x = avg(a);
      const y = avg(b);
      // Rows without calls have no average: last in either direction.
      if (x === null || y === null) return (x === null ? 1 : 0) - (y === null ? 1 : 0) || a.name.localeCompare(b.name);
      return (desc ? y - x : x - y) || a.name.localeCompare(b.name);
    });
  } else {
    const key = of === 'avgPerCall' ? 'total' : of;
    list = [...list].sort((a, b) => {
      const c = by === null ? a.name.localeCompare(b.name) : (a[key][by] ?? 0) - (b[key][by] ?? 0) || b.name.localeCompare(a.name);
      return desc ? -c : c;
    });
  }
  return { rows: list.slice(offset, offset + limit), totalRows: list.length, grandTotal: grand, metric, measurement: measurement(p, metric), samplesInView: samples };
}

export function mockButterfly(p: MockProfileData, view: ProfileView | undefined, frameId: number, metric: MetricId, limit: number): Butterfly {
  const { rows } = hotspotRows(p, view, 'function', metric);
  const fn = rows.find((r) => r.frame?.id === frameId);
  if (!fn) throw new Error(`function ${frameId} has no samples in this view`);
  const callers = new Map<number | null, Values>();
  const callees = new Map<number, Values>();
  for (const s of viewSamples(p, view)) {
    const i = s.stack.indexOf(frameId);
    if (i < 0) continue;
    const caller = i === 0 ? null : (s.stack[i - 1] ?? null);
    add(callers.get(caller) ?? callers.set(caller, zero(p)).get(caller)!, s.v);
    const callee = s.stack[i + 1];
    if (callee !== undefined) add(callees.get(callee) ?? callees.set(callee, zero(p)).get(callee)!, s.v);
  }
  const total = fn.total[metric] ?? 0;
  const toRows = (m: Map<number | null, Values>): ButterflyRow[] =>
    [...m.entries()]
      .map(([id, values]) => ({ frame: id === null ? null : frameOf(p, id), values, pct: pct(values[metric], total) }))
      .sort((a, b) => (b.values[metric] ?? 0) - (a.values[metric] ?? 0))
      .slice(0, limit);
  return { function: fn, callers: toRows(callers), callees: toRows(callees as Map<number | null, Values>), metric };
}

export interface FlameParams {
  direction: TreeDirection;
  focus: number[];
  viewport: { start: number; end: number };
  widthPx: number;
  minWidthPx: number;
  maxDepth: number;
  metric: MetricId;
  order: 'name' | 'value';
  search: string | null;
  searchRegex: boolean;
  maxRects: number;
}

function matcher(search: string | null, regex: boolean): ((name: string) => boolean) | null {
  if (!search) return null;
  if (regex) {
    let re: RegExp;
    try {
      re = new RegExp(search, 'i');
    } catch {
      throw new Error(`invalid search pattern: ${search}`);
    }
    return (n) => re.test(n);
  }
  const needle = search.toLowerCase();
  return (n) => n.toLowerCase().includes(needle);
}

/** Lays the tree out as the engine does: depth 0 is the focus (or the roots), fractions of the width. */
export function mockFlame(p: MockProfileData, view: ProfileView | undefined, params: FlameParams, baseline?: MockProfileData): FlameGraph {
  const samples = viewSamples(p, view);
  const g = grandTotal(p, samples);
  const root = buildTree(p, samples, params.direction);
  const metric = params.metric;
  const focusNode = nodeAt(root, params.focus);
  const top = params.focus.length ? [focusNode] : sortedChildren(root, metric);
  const focusValue = params.focus.length ? (focusNode.total[metric] ?? 0) : (root.total[metric] ?? 0);
  const match = matcher(params.search, params.searchRegex);
  const span = params.viewport.end - params.viewport.start;
  const rects: FlameRect[] = [];
  let truncated = false;
  let maxDepth = 0;
  let matchedValue = 0;

  // Baseline shares by name path, for the differential graph.
  let baseShare: ((names: string[]) => number) | null = null;
  let baseGrand = 0;
  if (baseline) {
    const bs = viewSamples(baseline, view);
    const bRoot = buildTree(baseline, bs, 'topDown');
    baseGrand = bRoot.total[metric] ?? 0;
    baseShare = (names) => {
      let node: Node | undefined = bRoot;
      for (const name of names) {
        node = [...(node?.children.values() ?? [])].find((c) => frameOf(baseline, c.frame).name === name);
        if (!node) return 0;
      }
      return baseGrand ? (node.total[metric] ?? 0) / baseGrand : 0;
    };
  }
  const prefix = params.focus.slice(0, -1).map((id) => frameOf(p, id).name);

  const place = (nodes: Node[], x0: number, depth: number, names: string[], inMatch: boolean, parent: number | null) => {
    const ordered = params.order === 'value' ? sortedChildren({ children: new Map(nodes.map((n) => [n.frame, n])) } as Node, metric) : [...nodes].sort((a, b) => frameOf(p, a.frame).name.localeCompare(frameOf(p, b.frame).name));
    let x = x0;
    for (const n of ordered) {
      const value = n.total[metric] ?? 0;
      const wAbs = focusValue ? value / focusValue : 0;
      const name = frameOf(p, n.frame).name;
      const matched = !!match && match(name);
      if (matched && !inMatch) matchedValue += value;
      const xs = (x - params.viewport.start) / span;
      const ws = wAbs / span;
      if (ws * params.widthPx >= params.minWidthPx && xs + ws > 0 && xs < 1) {
        if (rects.length >= params.maxRects) truncated = true;
        else {
          const rect: FlameRect = { frame: n.frame, parent, depth, x: xs, w: ws, value, selfValue: n.self[metric] ?? 0, matched };
          if (baseShare) {
            const path = [...names, name];
            const b = baseShare(path);
            const share = g[metric] ? value / g[metric]! : 0;
            rect.delta = share - b;
            rect.baselineValue = b * (g[metric] ?? 0);
          }
          const index = rects.push(rect) - 1;
          maxDepth = Math.max(maxDepth, depth);
          if (depth + 1 < params.maxDepth) place([...n.children.values()], x, depth + 1, [...names, name], inMatch || matched, index);
        }
      }
      x += wAbs;
    }
  };
  place(top, 0, 0, prefix, false, null);
  const used = new Set(rects.map((r) => r.frame));
  return {
    rects,
    frames: p.frames.filter((f) => used.has(f.id)),
    focusFrames: params.focus.map((id) => frameOf(p, id)),
    focusValue,
    grandTotal: g[metric] ?? 0,
    matchedValue,
    matchedPct: pct(matchedValue, focusValue),
    truncated,
    maxDepth,
    metric,
    measurement: measurement(p, metric),
  };
}

/** Hottest top-down paths to the function of `frameId` (same name and module); nested occurrences belong to the outer one. */
export function mockFramePaths(p: MockProfileData, view: ProfileView | undefined, frameId: number, metric: MetricId, limit: number): FramePaths {
  const target = frameOf(p, frameId);
  const same = (id: number) => {
    const f = frameOf(p, id);
    return f.name === target.name && f.module === target.module;
  };
  const samples = viewSamples(p, view);
  const g = grandTotal(p, samples);
  const found: { path: number[]; value: number }[] = [];
  const walk = (node: Node, path: number[]) => {
    for (const child of node.children.values()) {
      const at = [...path, child.frame];
      if (same(child.frame)) found.push({ path: at, value: child.total[metric] ?? 0 });
      else walk(child, at);
    }
  };
  walk(buildTree(p, samples, 'topDown'), []);
  found.sort((a, b) => b.value - a.value);
  return {
    paths: found.slice(0, limit).map((f) => ({ path: f.path, frames: f.path.map((id) => frameOf(p, id)), value: f.value, pct: pct(f.value, g[metric]) })),
    totalPaths: found.length,
    metric,
  };
}

export function mockFunctionDiff(base: MockProfileData, target: MockProfileData, view: ProfileView | undefined, groupBy: ProfileGroupBy, metric: MetricId, sort: DiffSort, search: string | null | undefined, offset: number, limit: number): FunctionDiffPage {
  const b = hotspotRows(base, view, groupBy, metric);
  const t = hotspotRows(target, view, groupBy, metric);
  const key = (r: HotspotRow) => `${r.name}|${r.module ?? ''}`;
  const keys = new Map<string, { b?: HotspotRow; t?: HotspotRow }>();
  for (const r of b.rows) keys.set(key(r), { b: r });
  for (const r of t.rows) keys.set(key(r), { ...keys.get(key(r)), t: r });
  const side = (r: HotspotRow | undefined, g: Values) => ({
    self: r?.self[metric] ?? 0,
    total: r?.total[metric] ?? 0,
    selfPct: pct(r?.self[metric], g[metric]),
    totalPct: pct(r?.total[metric], g[metric]),
  });
  const needle = search?.trim().toLowerCase();
  let rows: FunctionDiffRow[] = [...keys.values()].map(({ b: br, t: tr }) => {
    const any = (tr ?? br)!;
    const bs = side(br, b.grand);
    const ts = side(tr, t.grand);
    const deltaSelfPct = ts.selfPct - bs.selfPct;
    return {
      key: key(any),
      name: any.name,
      module: any.module,
      targetFrame: tr?.frame?.id ?? null,
      baseline: bs,
      target: ts,
      deltaSelfPct,
      deltaTotalPct: ts.totalPct - bs.totalPct,
      status: !br ? 'new' : !tr ? 'removed' : Math.abs(deltaSelfPct) < 0.05 && Math.abs(ts.totalPct - bs.totalPct) < 0.05 ? 'same' : 'changed',
    };
  });
  if (needle) rows = rows.filter((r) => r.name.toLowerCase().includes(needle));
  const cmp: Record<DiffSort, (a: FunctionDiffRow, b: FunctionDiffRow) => number> = {
    absDeltaSelf: (x, y) => Math.abs(y.deltaSelfPct) - Math.abs(x.deltaSelfPct),
    absDeltaTotal: (x, y) => Math.abs(y.deltaTotalPct) - Math.abs(x.deltaTotalPct),
    deltaSelf: (x, y) => y.deltaSelfPct - x.deltaSelfPct,
    deltaTotal: (x, y) => y.deltaTotalPct - x.deltaTotalPct,
    name: (x, y) => x.name.localeCompare(y.name),
  };
  rows.sort((x, y) => cmp[sort](x, y) || x.name.localeCompare(y.name));
  return { rows: rows.slice(offset, offset + limit), totalRows: rows.length, baselineTotal: b.grand[metric] ?? 0, targetTotal: t.grand[metric] ?? 0, metric };
}

export function mockThreads(p: MockProfileData, view: ProfileView | undefined, buckets: number, metric: MetricId, kinds: string[] | null, maxEvents: number, startedMs: number | null): ThreadTimeline {
  const fromNs = view?.range?.fromNs ?? 0;
  const toNs = view?.range?.toNs ?? p.durationNs;
  const bucketNs = Math.max(1, Math.ceil((toNs - fromNs) / buckets));
  const lanes = new Map<number, number[]>();
  for (const s of viewSamples(p, view)) {
    const lane = lanes.get(s.thread) ?? lanes.set(s.thread, new Array<number>(buckets).fill(0)).get(s.thread)!;
    const i = Math.min(buckets - 1, Math.floor((s.t - fromNs) / bucketNs));
    lane[i] = (lane[i] ?? 0) + (s.v[metric] ?? 0);
  }
  const threadSet = view?.threads?.length ? new Set(view.threads) : null;
  const events = p.events.filter((e) => e.startNs + e.durNs >= fromNs && e.startNs <= toNs && (!kinds || kinds.includes(e.kind)) && (!threadSet || e.thread === null || threadSet.has(e.thread)));
  return {
    fromNs,
    toNs,
    bucketNs,
    threads: [...lanes.entries()]
      .map(([id, b]) => ({ thread: p.threads.find((t) => t.id === id)!, total: b.reduce((a, x) => a + x, 0), buckets: b }))
      .sort((a, b) => b.total - a.total),
    events: events.slice(0, maxEvents),
    eventsTruncated: events.length > maxEvents,
    metric,
    startedMs,
  };
}

/** Top-down path (root first) to the first stack that contains `frameId`. */
function pathTo(p: MockProfileData, view: ProfileView | undefined, frameId: number): number[] {
  const s = viewSamples(p, view).find((x) => x.stack.includes(frameId));
  return s ? s.stack.slice(0, s.stack.indexOf(frameId) + 1) : [];
}

export function mockFindings(id: number, p: MockProfileData, view: ProfileView | undefined, baseline: { id: number; data: MockProfileData } | null): ProfileFinding[] {
  const metric: MetricId = p.metrics.some((m) => m.id === 'timeNs') ? 'timeNs' : 'samples';
  const { rows, grand } = hotspotRows(p, view, 'function', metric);
  const evidence = (view_: ProfileFinding['evidence']['view'], frame: FrameDto | null, extra: Partial<ProfileFinding['evidence']> = {}) => ({
    view: view_,
    profileId: id,
    baselineId: null,
    frame,
    path: frame ? pathTo(p, view, frame.id) : [],
    metric,
    range: view?.range ?? null,
    threads: view?.threads ?? null,
    eventKind: null,
    ...extra,
  });
  const out: ProfileFinding[] = [];
  const gc = rows.find((r) => r.frame?.id === 13);
  if (gc && gc.totalPct > 5) {
    out.push({ kind: 'gcPressure', severity: 'warning', title: 'GC pressure', detail: `Garbage collection takes ${gc.totalPct.toFixed(1)}% of the time; look at the allocation hot spots.`, valuePct: gc.totalPct, evidence: evidence('timeline', null, { eventKind: 'gc' }) });
  }
  const lock = rows.find((r) => r.frame?.id === 10);
  if (lock && lock.totalPct > 5) {
    out.push({ kind: 'lockContention', severity: 'critical', title: 'Lock contention in OrderRepository.Save', detail: `Threads wait on Monitor.Enter for ${lock.totalPct.toFixed(1)}% of the time.`, valuePct: lock.totalPct, evidence: evidence('callTree', lock.frame) });
  }
  const thrower = rows.find((r) => r.frame?.id === 15);
  if (thrower && (grand.exceptions ?? 0) > 0) {
    out.push({ kind: 'exceptionHotPath', severity: 'warning', title: 'Exceptions on a hot path', detail: `${grand.exceptions} exceptions thrown from OrderValidator.ThrowInvalid during requests.`, valuePct: thrower.totalPct, evidence: evidence('butterfly', thrower.frame) });
  }
  const top = [...rows].sort((a, b) => b.selfPct - a.selfPct).filter((r) => r.selfPct >= 5).slice(0, 2);
  top.forEach((r, i) => out.push({ kind: 'topSelf', severity: 'info', title: `Top self time: ${r.name}`, detail: `${r.selfPct.toFixed(1)}% of the time is spent in this function itself.`, valuePct: r.selfPct, evidence: evidence(i === 0 ? 'hotspots' : 'flame', r.frame) }));
  if (baseline) {
    const diff = mockFunctionDiff(baseline.data, p, view, 'function', 'samples', 'deltaSelf', null, 0, 3);
    for (const r of diff.rows.filter((x) => x.deltaSelfPct > 5)) {
      const frame = r.targetFrame === null ? null : frameOf(p, r.targetFrame);
      out.unshift({ kind: 'regression', severity: 'critical', title: `Regression: ${r.name}`, detail: `Self share grew by ${r.deltaSelfPct.toFixed(1)} percentage points against the baseline.`, valuePct: r.deltaSelfPct, evidence: evidence('diff', frame, { baselineId: baseline.id }) });
    }
  }
  const rank = { critical: 0, warning: 1, info: 2 };
  return out.sort((a, b) => rank[a.severity] - rank[b.severity]);
}
