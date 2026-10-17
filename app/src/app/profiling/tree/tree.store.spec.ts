import { TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';
import { FrameDto, HotPath, HotPathRequest, TreeNode, TreePage, TreeRequest } from '../../core/api.types';
import { FrameNames } from '../captures.store';
import { ProfilingBackend } from '../profiling-backend';
import { openDemoCaptures, setupProfiling } from '../testing/profiling-test-support';
import { ROOT_KEY } from './tree-model';
import { TREE_PAGE, TreeQuery, TreeStore } from './tree.store';

const frame = (id: number): FrameDto => ({ id, name: `F${id}`, module: null, file: null, line: null, kind: 'managed', isFramework: false });
const node = (id: number, hasChildren: boolean): TreeNode => ({ frame: frame(id), self: { timeNs: 1 }, total: { timeNs: 2 }, selfPct: 1, totalPct: 2, hasChildren });
const query = (patch: Partial<TreeQuery> = {}): TreeQuery => ({ profileId: 1, view: {}, metric: 'timeNs', ...patch });

/** A synthetic tree: `kids[pathKey]` = number of children, whose frame ids are 0..n-1 (shared by all levels). */
function fake(kids: Record<string, number>, overlap = 0) {
  const treeCalls: TreeRequest[] = [];
  const hotCalls: HotPathRequest[] = [];
  const failing = new Set<string>();
  const holds: Array<() => void> = [];
  let hold = false;
  let hotPath: HotPath | Error = { path: [], nodes: [] };
  const answer = (r: TreeRequest): TreePage => {
    const key = (r.path ?? []).join('/');
    const total = kids[key] ?? 0;
    const offset = Math.max(0, (r.offset ?? 0) - overlap);
    const n = Math.max(0, Math.min(r.limit ?? TREE_PAGE, total - offset));
    const nodes = Array.from({ length: n }, (_, i) => node(offset + i, kids[[...(r.path ?? []), offset + i].join('/')] !== undefined));
    return { nodes, totalChildren: total, grandTotal: { timeNs: 1000 }, metric: 'timeNs', measurement: 'sampled' };
  };
  const backend = {
    tree: vi.fn((r: TreeRequest) => {
      treeCalls.push(r);
      const key = (r.path ?? []).join('/');
      if (failing.delete(key)) return Promise.reject(`boom at ${key || 'root'}`);
      if (hold) return new Promise<TreePage>((resolve) => holds.push(() => resolve(answer(r))));
      return Promise.resolve(answer(r));
    }),
    hotPath: vi.fn((r: HotPathRequest) => {
      hotCalls.push(r);
      if (hotPath instanceof Error) {
        const message = hotPath.message;
        return hold ? new Promise<HotPath>((_, reject) => holds.push(() => reject(message))) : Promise.reject(message);
      }
      const p = hotPath;
      if (hold) return new Promise<HotPath>((resolve) => holds.push(() => resolve(p)));
      return Promise.resolve(p);
    }),
  } as unknown as ProfilingBackend;
  return {
    backend, treeCalls, hotCalls, failing, holds,
    holdBack: () => (hold = true),
    setHot: (h: HotPath | Error) => (hotPath = h),
  };
}

function setup(kids: Record<string, number>, overlap = 0) {
  const f = fake(kids, overlap);
  setupProfiling({ backend: f.backend });
  TestBed.configureTestingModule({ providers: [TreeStore] });
  return { f, store: TestBed.inject(TreeStore) };
}

const SMALL = { '': 3, '0': 2, '0/1': 2, '1': 1 };

describe('TreeStore', () => {
  it('setQuery loads the top level and remembers frame names', async () => {
    const { f, store } = setup(SMALL);
    await store.setQuery(query());
    expect(f.treeCalls).toEqual([{ profileId: 1, view: {}, metric: 'timeNs', direction: 'topDown', path: [], offset: 0, limit: TREE_PAGE }]);
    expect(store.root().children).toEqual(['0', '1', '2']);
    expect(store.root().totalChildren).toBe(3);
    expect(store.rows().every((r) => r.kind === 'node')).toBe(true);
    expect(store.rows()).toHaveLength(3);
    expect(store.grandTotal()).toEqual({ timeNs: 1000 });
    expect(store.measurement()).toBe('sampled');
    expect(TestBed.inject(FrameNames).name(1, 2)).toBe('F2');
  });

  it('the same query is a no-op, a new one starts over and null empties the tree', async () => {
    const { f, store } = setup(SMALL);
    await store.setQuery(query());
    await store.toggle('0');
    store.select('0');
    await store.setQuery(query());
    expect(f.treeCalls).toHaveLength(2);
    await store.setQuery(query({ metric: 'samples' }));
    expect(f.treeCalls).toHaveLength(3);
    expect(store.expanded()).toEqual([]);
    expect(store.selected()).toBeNull();
    expect(store.rows()).toHaveLength(3);
    await store.setQuery(null);
    expect(store.root().children).toBeNull();
    expect(store.rows()).toHaveLength(0);
    expect(store.grandTotal()).toBeNull();
    expect(f.treeCalls).toHaveLength(3);
  });

  it('expand, collapse and toggle', async () => {
    const { f, store } = setup(SMALL);
    await store.setQuery(query());
    expect(await store.expand('0')).toBe(true);
    expect(store.expanded()).toEqual(['0']);
    expect(store.rows().map((r) => (r.kind === 'node' ? r.entry.key : r.kind))).toEqual(['0', '0/0', '0/1', '1', '2']);
    // Already loaded: no new request, no duplicate expansion.
    const calls = f.treeCalls.length;
    expect(await store.expand('0')).toBe(true);
    expect(f.treeCalls).toHaveLength(calls);
    expect(store.expanded()).toEqual(['0']);
    store.collapse('0');
    expect(store.expanded()).toEqual([]);
    await store.toggle('0');
    expect(store.expanded()).toEqual(['0']);
    await store.toggle('0');
    expect(store.expanded()).toEqual([]);
    expect(await store.expand('nope')).toBe(false);
    expect(await store.expand(ROOT_KEY)).toBe(true);
    expect(store.expanded()).toEqual([]);
  });

  it('loadMore pages through a large child list without duplicates', async () => {
    const { f, store } = setup({ '': 450 });
    await store.setQuery(query());
    expect(store.root().children).toHaveLength(TREE_PAGE);
    expect(store.root().totalChildren).toBe(450);
    expect(await store.loadMore(ROOT_KEY)).toBe(true);
    expect(store.root().children).toHaveLength(400);
    expect(await store.loadMore(ROOT_KEY)).toBe(true);
    expect(store.root().children).toHaveLength(450);
    expect(f.treeCalls.map((c) => c.offset)).toEqual([0, 200, 400]);
    expect(new Set(store.root().children).size).toBe(450);
  });

  it('loadMore tolerates a page that repeats nodes and refuses unknown or busy nodes', async () => {
    const { f, store } = setup({ '': 5 });
    await store.setQuery(query());
    expect(await store.loadMore('nope')).toBe(false);
    f.holdBack();
    const a = store.loadMore(ROOT_KEY);
    expect(store.root().loading).toBe(true);
    expect(await store.loadMore(ROOT_KEY)).toBe(false);
    f.holds[0]();
    expect(await a).toBe(true);
    // The fake serves offset 5 as an empty page: nothing is duplicated.
    expect(store.root().children).toEqual(['0', '1', '2', '3', '4']);
  });

  it('keeps loaded descendants and does not duplicate a node the next page repeats', async () => {
    const { store } = setup({ '': 450, '199': 2 }, 1);
    await store.setQuery(query());
    await store.expand('199');
    expect(store.entries()['199'].children).toEqual(['199/0', '199/1']);
    await store.loadMore(ROOT_KEY);
    expect(store.root().children).toHaveLength(399);
    expect(new Set(store.root().children).size).toBe(399);
    expect(store.entries()['199'].children).toEqual(['199/0', '199/1']);
  });

  it('does nothing without a query', async () => {
    const { f, store } = setup(SMALL);
    expect(await store.loadMore(ROOT_KEY)).toBe(false);
    await store.expandHotPath();
    expect(f.hotCalls).toHaveLength(0);
    expect(await store.reveal([0])).toBe(false);
  });

  it('shows a per-node error and retry loads again', async () => {
    const { f, store } = setup(SMALL);
    await store.setQuery(query());
    f.failing.add('0');
    expect(await store.expand('0')).toBe(false);
    expect(store.root().error).toBeNull();
    const status = store.rows().find((r) => r.kind === 'status');
    expect(status).toMatchObject({ parent: '0', error: 'boom at 0', loading: false });
    expect(await store.retry('0')).toBe(true);
    expect(store.rows().map((r) => (r.kind === 'node' ? r.entry.key : r.kind))).toContain('0/1');
    expect(store.rows().some((r) => r.kind === 'status')).toBe(false);
  });

  it('records the error on the failing node', async () => {
    const { f, store } = setup(SMALL);
    f.failing.add('');
    await store.setQuery(query());
    expect(store.root().error).toContain('boom at root');
    expect(store.root().loading).toBe(false);
    expect(await store.retry(ROOT_KEY)).toBe(true);
    expect(store.root().error).toBeNull();
    expect(store.root().children).toHaveLength(3);
  });

  it('drops answers from an older query', async () => {
    const { f, store } = setup(SMALL);
    f.holdBack();
    const first = store.setQuery(query());
    const second = store.setQuery(query({ profileId: 2 }));
    f.holds[1]();
    await second;
    f.holds[0]();
    await first;
    expect(store.root().children).toEqual(['0', '1', '2']);
    expect(store.root().loading).toBe(false);
    expect(TestBed.inject(FrameNames).get(1, 0)).toBeNull();
    expect(TestBed.inject(FrameNames).get(2, 0)).not.toBeNull();
  });

  it('a new query while a hot path is in flight does not leave the tree busy (regression)', async () => {
    const { f, store } = setup(SMALL);
    await store.setQuery(query());
    f.holdBack();
    const hot = store.expandHotPath();
    expect(store.busy()).toBe(true);
    const next = store.setQuery(query({ profileId: 2 }));
    expect(store.busy()).toBe(false);
    f.holds.forEach((release) => release());
    await Promise.all([hot, next]);
    expect(store.busy()).toBe(false);
  });

  it('passes the bottom-up direction to the backend', async () => {
    const { f, store } = setup(SMALL);
    store.setDirection('bottomUp');
    expect(store.direction()).toBe('bottomUp');
    await store.setQuery(query());
    await store.expand('0');
    expect(f.treeCalls.map((c) => c.direction)).toEqual(['bottomUp', 'bottomUp']);
    f.setHot({ path: [0, 1], nodes: [node(0, true), node(1, true)] });
    await store.expandHotPath();
    expect(f.hotCalls[0].direction).toBe('bottomUp');
  });

  describe('reveal', () => {
    it('expands every prefix and selects the last node', async () => {
      const { f, store } = setup(SMALL);
      await store.setQuery(query());
      expect(await store.reveal([0, 1])).toBe(true);
      expect(store.expanded()).toEqual(['0']);
      expect(store.selected()).toBe('0/1');
      expect(store.notice()).toBeNull();
      expect(f.treeCalls.map((c) => (c.path ?? []).join('/'))).toEqual(['', '0']);
      expect(await store.reveal([0, 1, 0])).toBe(true);
      expect(store.expanded()).toEqual(['0', '0/1']);
      expect(store.selected()).toBe('0/1/0');
    });

    it('pages through the parent until the child appears', async () => {
      const { f, store } = setup({ '': 450 });
      await store.setQuery(query());
      expect(await store.reveal([430])).toBe(true);
      expect(f.treeCalls.map((c) => c.offset)).toEqual([0, 200, 400]);
      expect(store.selected()).toBe('430');
    });

    it('explains a path that is not in the tree and keeps the selection', async () => {
      const { store } = setup(SMALL);
      await store.setQuery(query());
      store.select('2');
      expect(await store.reveal([9])).toBe(false);
      expect(store.notice()).toContain('not in the tree');
      expect(store.selected()).toBe('2');
      store.dismissNotice();
      expect(store.notice()).toBeNull();
      expect(await store.reveal([0, 9])).toBe(false);
      expect(store.notice()).toContain('not in the tree');
      // A successful reveal clears the notice.
      expect(await store.reveal([1])).toBe(true);
      expect(store.notice()).toBeNull();
    });

    it('stops when the top level or a parent cannot be loaded', async () => {
      const { f, store } = setup({ '': 450, '3': 2 });
      f.failing.add('');
      await store.setQuery(query());
      f.failing.add('');
      expect(await store.reveal([0])).toBe(false);
      expect(await store.reveal([0])).toBe(true);
      f.failing.add('3');
      expect(await store.reveal([3, 1])).toBe(false);
      expect(store.root().children!.length).toBeGreaterThan(0);
    });

    it('stops when paging a parent fails', async () => {
      const { f, store } = setup({ '': 450 });
      await store.setQuery(query());
      f.failing.add('');
      expect(await store.reveal([300])).toBe(false);
      expect(store.root().error).toContain('boom');
    });
  });

  describe('expandHotPath', () => {
    it('expands along the engine path and selects the hot node', async () => {
      const { f, store } = setup(SMALL);
      await store.setQuery(query());
      f.setHot({ path: [0, 1], nodes: [node(0, true), node(1, true)] });
      await store.expandHotPath();
      expect(f.hotCalls).toEqual([{ profileId: 1, view: {}, metric: 'timeNs', direction: 'topDown', path: [] }]);
      expect(store.expanded()).toEqual(['0']);
      expect(store.selected()).toBe('0/1');
      expect(store.busy()).toBe(false);
      expect(store.notice()).toBeNull();
    });

    it('starts from a node and says so when nothing below is hot', async () => {
      const { f, store } = setup(SMALL);
      await store.setQuery(query());
      await store.expand('0');
      f.setHot({ path: [0], nodes: [node(0, true)] });
      await store.expandHotPath('0');
      expect(f.hotCalls[0].path).toEqual([0]);
      expect(store.notice()).toBe('Nothing below this node is hot enough to expand.');
      expect(store.busy()).toBe(false);
      f.setHot({ path: [], nodes: [] });
      await store.expandHotPath('unknown-key');
      expect(f.hotCalls[1].path).toEqual([]);
    });

    it('turns a backend error into a notice', async () => {
      const { f, store } = setup(SMALL);
      await store.setQuery(query());
      f.setHot(new Error('hot path unavailable'));
      await store.expandHotPath();
      expect(store.notice()).toContain('hot path unavailable');
      expect(store.busy()).toBe(false);
    });

    it('ignores an answer that arrives after the query changed', async () => {
      const { f, store } = setup(SMALL);
      await store.setQuery(query());
      f.setHot({ path: [0, 1], nodes: [node(0, true), node(77, true)] });
      f.holdBack();
      const hot = store.expandHotPath();
      const next = store.setQuery(query({ profileId: 2 }));
      f.holds[1]();
      await next;
      f.holds[0]();
      await hot;
      expect(store.expanded()).toEqual([]);
      expect(store.selected()).toBeNull();
      expect(TestBed.inject(FrameNames).get(1, 77)).toBeNull();
    });

    it('ignores an error that arrives after the query changed', async () => {
      const { f, store } = setup(SMALL);
      await store.setQuery(query());
      f.setHot(new Error('late'));
      f.holdBack();
      const hot = store.expandHotPath();
      const next = store.setQuery(query({ profileId: 2 }));
      f.holds[1]();
      await next;
      f.holds[0]();
      await hot;
      expect(store.notice()).toBeNull();
    });
  });

  it('works against the mock engine', async () => {
    setupProfiling();
    TestBed.configureTestingModule({ providers: [TreeStore] });
    await openDemoCaptures();
    const store = TestBed.inject(TreeStore);
    await store.setQuery({ profileId: 1, view: {}, metric: 'timeNs' });
    expect(store.rows().length).toBeGreaterThan(0);
    await store.expandHotPath();
    expect(store.selected()).not.toBeNull();
    expect(store.expanded().length).toBeGreaterThan(0);
    expect(TestBed.inject(ProfilingBackend)).toBeTruthy();
  });
});
