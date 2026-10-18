import { computed, inject } from '@angular/core';
import { patchState, signalStore, withComputed, withMethods, withState } from '@ngrx/signals';
import { Measurement, MetricId, MetricValues, ProfileView, TreeDirection, TreeNode } from '../../core/api.types';
import { errorText } from '../../analysis/async-resource';
import { FrameNames } from '../captures.store';
import { ProfilingBackend } from '../profiling-backend';
import { flattenTree, keyOf, ROOT_KEY, TreeEntry } from './tree-model';

/** Children fetched per request. */
export const TREE_PAGE = 200;

export interface TreeQuery {
  profileId: number;
  view: ProfileView;
  metric: MetricId;
}

interface TreeShape {
  direction: TreeDirection;
  entries: Record<string, TreeEntry>;
  expanded: string[];
  selected: string | null;
  grandTotal: MetricValues | null;
  measurement: Measurement | null;
  /** Message when a reveal or hot path could not be followed. */
  notice: string | null;
  busy: boolean;
}

const rootEntry = (): TreeEntry => ({ key: ROOT_KEY, path: [], node: null as unknown as TreeNode, children: null, totalChildren: 0, loading: false, error: null });

/**
 * A lazily paged call tree (top-down) or inverted tree (bottom-up): children are requested from
 * the engine when a node is expanded; "expand hottest path" and reveals follow engine paths.
 */
export const TreeStore = signalStore(
  withState<TreeShape>({ direction: 'topDown', entries: { [ROOT_KEY]: rootEntry() }, expanded: [], selected: null, grandTotal: null, measurement: null, notice: null, busy: false }),
  withComputed((s) => ({
    rows: computed(() => flattenTree(s.entries(), new Set(s.expanded()))),
    // The root entry is always present (every reset puts a fresh one in).
    root: computed((): TreeEntry => s.entries()[ROOT_KEY] ?? rootEntry()),
  })),
  withMethods((s) => {
    const backend = inject(ProfilingBackend);
    const names = inject(FrameNames);
    let query: TreeQuery | null = null;
    let queryKey = '';
    let generation = 0;

    const patchEntry = (key: string, patch: Partial<TreeEntry>) => {
      const cur = s.entries()[key];
      if (cur) patchState(s, { entries: { ...s.entries(), [key]: { ...cur, ...patch } } });
    };

    /** Loads the next page of children of `key`. Resolves false when it failed or the query changed. */
    const loadChildren = async (key: string): Promise<boolean> => {
      const q = query;
      const entry = s.entries()[key];
      if (!q || !entry || entry.loading) return false;
      const mine = generation;
      const offset = entry.children?.length ?? 0;
      patchEntry(key, { loading: true, error: null });
      try {
        const page = await backend.tree({ profileId: q.profileId, view: q.view, metric: q.metric, direction: s.direction(), path: entry.path, offset, limit: TREE_PAGE });
        if (mine !== generation) return false;
        names.remember(q.profileId, page.nodes.map((n) => n.frame));
        const entries = { ...s.entries() };
        const keys = [...(entry.children ?? [])];
        for (const node of page.nodes) {
          const path = [...entry.path, node.frame.id];
          const k = keyOf(path);
          entries[k] = { key: k, path, node, children: entries[k]?.children ?? null, totalChildren: entries[k]?.totalChildren ?? 0, loading: false, error: null };
          if (!keys.includes(k)) keys.push(k);
        }
        entries[key] = { ...(entries[key] ?? entry), children: keys, totalChildren: page.totalChildren, loading: false, error: null };
        patchState(s, { entries, grandTotal: page.grandTotal, measurement: page.measurement });
        return true;
      } catch (e) {
        if (mine === generation) patchEntry(key, { loading: false, error: errorText(e) });
        return false;
      }
    };

    const expand = async (key: string): Promise<boolean> => {
      const entry = s.entries()[key];
      if (!entry) return false;
      if (key !== ROOT_KEY && !s.expanded().includes(key)) patchState(s, { expanded: [...s.expanded(), key] });
      return entry.children === null ? loadChildren(key) : true;
    };

    /** Expands every prefix of `path` (loading pages as needed) and selects its last node. */
    const reveal = async (path: number[]): Promise<boolean> => {
      if (!(await expand(ROOT_KEY))) return false;
      for (let i = 1; i <= path.length; i++) {
        const k = keyOf(path.slice(0, i));
        let parent = s.entries()[keyOf(path.slice(0, i - 1))];
        // Page through the parent until the child appears (bounded by the parent's child count).
        while (parent && !s.entries()[k] && parent.children && parent.children.length < parent.totalChildren) {
          if (!(await loadChildren(parent.key))) return false;
          parent = s.entries()[parent.key];
        }
        if (!s.entries()[k]) {
          patchState(s, { notice: $localize`:@@profiling.tree.notInTree:That path is not in the tree under the current view (range, threads, just my code or recursion).` });
          return false;
        }
        if (i < path.length && !(await expand(k))) return false;
      }
      patchState(s, { selected: keyOf(path), notice: null });
      return true;
    };

    return {
      setDirection: (direction: TreeDirection) => patchState(s, { direction }),
      /** A new query (profile, view or metric) starts over from the top level. */
      async setQuery(q: TreeQuery | null): Promise<void> {
        const k = JSON.stringify(q);
        if (k === queryKey) return;
        queryKey = k;
        query = q;
        generation++;
        patchState(s, { entries: { [ROOT_KEY]: rootEntry() }, expanded: [], selected: null, notice: null, grandTotal: null, busy: false });
        if (q) await loadChildren(ROOT_KEY);
      },
      loadMore: (key: string) => loadChildren(key),
      retry: (key: string) => loadChildren(key),
      expand,
      collapse: (key: string) => patchState(s, { expanded: s.expanded().filter((k) => k !== key) }),
      async toggle(key: string): Promise<void> {
        if (s.expanded().includes(key)) patchState(s, { expanded: s.expanded().filter((k) => k !== key) });
        else await expand(key);
      },
      select: (key: string | null) => patchState(s, { selected: key }),
      reveal,
      /** Asks the engine for the hottest path below `key` (the top when empty) and expands along it. */
      async expandHotPath(key: string = ROOT_KEY): Promise<void> {
        const q = query;
        if (!q) return;
        const mine = generation;
        const from = s.entries()[key]?.path ?? [];
        patchState(s, { busy: true, notice: null });
        try {
          const hot = await backend.hotPath({ profileId: q.profileId, view: q.view, metric: q.metric, direction: s.direction(), path: from });
          if (mine !== generation) return;
          names.remember(q.profileId, hot.nodes.map((n) => n.frame));
          if (hot.path.length > from.length) await reveal(hot.path);
          else patchState(s, { notice: $localize`:@@profiling.tree.notHot:Nothing below this node is hot enough to expand.` });
        } catch (e) {
          if (mine === generation) patchState(s, { notice: errorText(e) });
        } finally {
          if (mine === generation) patchState(s, { busy: false });
        }
      },
      dismissNotice: () => patchState(s, { notice: null }),
    };
  }),
);

export type TreeStoreType = InstanceType<typeof TreeStore>;
