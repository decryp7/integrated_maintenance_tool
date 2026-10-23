import { computed, inject, Injectable } from '@angular/core';
import { Evidence, EvidenceView, FrameDto, HeapSummary, HotspotRow, MetricId, ProfileSummary, ProfileTimeRange } from '../core/api.types';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { CapturesStore } from './captures.store';
import { ProfilingBackend } from './profiling-backend';
import { isHeap, isProfile } from './profiling-format';
import { ProfileViewState } from './profile-view-state.store';

/** Dock view ids of the profiling views. */
export const PROFILING_VIEW_IDS = {
  captures: 'profiling-captures',
  hotspots: 'profiling-hotspots',
  tree: 'profiling-tree',
  inverted: 'profiling-inverted',
  butterfly: 'profiling-butterfly',
  flame: 'profiling-flame',
  compare: 'profiling-compare',
  threads: 'profiling-threads',
  findings: 'profiling-findings',
  heap: 'profiling-heap',
} as const;

const EVIDENCE_VIEW: Record<EvidenceView, string> = {
  hotspots: PROFILING_VIEW_IDS.hotspots,
  callTree: PROFILING_VIEW_IDS.tree,
  flame: PROFILING_VIEW_IDS.flame,
  butterfly: PROFILING_VIEW_IDS.butterfly,
  timeline: PROFILING_VIEW_IDS.threads,
  diff: PROFILING_VIEW_IDS.compare,
};

/** The active capture and effective metric, derived from the captures and the shared view state. */
@Injectable({ providedIn: 'root' })
export class ActiveProfile {
  private readonly captures = inject(CapturesStore);
  private readonly state = inject(ProfileViewState);

  readonly profile = computed<ProfileSummary | null>(() => {
    const c = this.captures.byId(this.state.profileId());
    return isProfile(c) ? c : null;
  });

  readonly heap = computed<HeapSummary | null>(() => {
    const c = this.captures.byId(this.state.heapId());
    return isHeap(c) ? c : null;
  });

  /** The chosen metric when the profile has it, else the profile's default. */
  readonly metric = computed<MetricId | null>(() => {
    const p = this.profile();
    if (!p) return null;
    const m = this.state.metric();
    return m && p.metrics.some((x) => x.id === m) ? m : p.defaultMetric;
  });
}

/** Moves between profiling views: evidence links, row actions, log timeline and moment links. */
@Injectable({ providedIn: 'root' })
export class ProfilingNavigator {
  private readonly ctx = inject(COMMAND_CONTEXT);
  private readonly state = inject(ProfileViewState);
  /** Optional: the log views embed profiling links without the profiling engine. */
  private readonly backend = inject(ProfilingBackend, { optional: true });

  show(viewId: string): void {
    this.ctx.showView(viewId);
  }

  openEvidence(e: Evidence): void {
    this.state.applyEvidence(e);
    this.show(EVIDENCE_VIEW[e.view]);
  }

  /** Opens `profileId` on a range (from the log timeline or a moment) and shows the hotspots. */
  openOnRange(profileId: number, range: ProfileTimeRange | null): void {
    this.state.setProfile(profileId);
    this.state.setRange(range);
    this.show(PROFILING_VIEW_IDS.hotspots);
  }

  openHeap(heapId: number): void {
    this.state.setHeap(heapId);
    this.show(PROFILING_VIEW_IDS.heap);
  }

  butterfly(frame: FrameDto): void {
    this.state.selectFrame(frame);
    this.show(PROFILING_VIEW_IDS.butterfly);
  }

  /**
   * The engine's hottest top-down path to a function in the current view; null when the function
   * is not in the view or the request failed (the caller then shows the view without a target).
   */
  private async hottestPath(frame: FrameDto, metric: MetricId | null): Promise<number[] | null> {
    const profileId = this.state.profileId();
    if (profileId === null || !this.backend) return null;
    try {
      const r = await this.backend.framePaths({ profileId, view: this.state.view(), frame: frame.id, metric, limit: 1 });
      return r.paths[0]?.path ?? null;
    } catch {
      return null;
    }
  }

  /** Shows a function in the call tree: its hottest call path is expanded down to the exact node, which is selected. */
  async showInTree(row: HotspotRow, metric: MetricId): Promise<void> {
    if (!row.frame) return;
    this.state.selectFrame(row.frame);
    const path = await this.hottestPath(row.frame, metric);
    if (path) this.state.revealPath(path, 'topDown');
    this.show(PROFILING_VIEW_IDS.tree);
  }

  /**
   * Zooms the flame graph to a function: the focus becomes its hottest call path. When the function
   * is not in the view, it is highlighted instead (as `showInFlame`).
   */
  async focusInFlame(frame: FrameDto, metric: MetricId | null): Promise<void> {
    const path = await this.hottestPath(frame, metric);
    if (!path) return this.showInFlame(frame);
    this.state.selectFrame(frame);
    this.state.setFlameSearch('');
    this.state.setFlameFocus(path);
    this.show(PROFILING_VIEW_IDS.flame);
  }

  /** Shows a function in the flame graph: every occurrence highlighted by the engine's search mask. */
  showInFlame(frame: FrameDto): void {
    this.state.selectFrame(frame);
    this.state.setFlameFocus([]);
    this.state.setFlameSearch(frame.name);
    this.show(PROFILING_VIEW_IDS.flame);
  }
}
