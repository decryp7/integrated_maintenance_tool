import { computed, inject, Injectable, InjectionToken, Signal } from '@angular/core';
import { Timeline } from '../core/api.types';
import { WorkspaceStore } from '../core/workspace.store';
import { TimeCursor } from '../analysis/time-cursor';
import { RangeZoom, TimelineZoom } from './zoom';

/**
 * What a timeline panel shows and drives. The main timeline is bound to the workspace (its filter, range and
 * instance filter); each additional timeline has its own state (`TimelineInstanceStore`). The panel component only
 * talks to this, so one component serves both.
 */
export interface TimelineSource {
  /** True for the main timeline, which is bound to the workspace filter. */
  readonly main: boolean;
  readonly title: Signal<string>;
  readonly timeline: Signal<Timeline | null>;
  readonly loading: Signal<boolean>;
  /** Why the last request failed (shown inline with Retry). */
  readonly error: Signal<string | null>;
  /** Marker kinds the timeline is restricted to (empty = none). */
  readonly markers: Signal<readonly string[]>;
  /** Lane keys the timeline is restricted to (empty = all). */
  readonly groups: Signal<readonly string[]>;
  /** Fields the lanes are formed by. */
  readonly groupBy: Signal<readonly string[]>;
  /** True while a range narrower than the data is shown (lane order is kept stable while zoomed). */
  readonly hasRange: Signal<boolean>;
  /** The zoom controller in effect: the workspace's when linked to the main timeline, else the panel's own. */
  readonly zoom: Signal<RangeZoom>;
  /** The time cursor in effect: shared with every time-based view when linked, else the panel's own. */
  readonly cursor: Signal<TimeCursor>;
  retry(): void;
  /** Horizontal resolution (plot width / 2). */
  setBuckets(buckets: number): void;
  /** Shows only events with this marker kind; null clears the restriction. */
  showOnlyMarker(kind: string | null): void;
  /** Adds or removes a lane from the lane filter. */
  toggleGroup(key: string): void;
}

export const TIMELINE_SOURCE = new InjectionToken<TimelineSource>('TIMELINE_SOURCE');

/** The main timeline: the workspace's timeline, filter, range (`TimelineZoom`) and the shared time cursor. */
@Injectable({ providedIn: 'root' })
export class MainTimelineSource implements TimelineSource {
  private readonly store = inject(WorkspaceStore);
  private readonly sharedZoom = inject(TimelineZoom);
  private readonly sharedCursor = inject(TimeCursor);

  readonly main = true;
  readonly title = computed(() => $localize`:@@timeline.defaultTitle:Timeline`);
  readonly timeline = this.store.timeline;
  readonly loading = this.store.timelineLoading;
  readonly error = this.store.timelineError;
  readonly markers = computed(() => this.store.query().markers);
  readonly groups = computed(() => this.store.query().groups);
  readonly groupBy = this.store.effectiveGroupBy;
  readonly hasRange = this.store.hasRange;
  readonly zoom = computed(() => this.sharedZoom as RangeZoom);
  readonly cursor = computed(() => this.sharedCursor);

  retry(): void {
    this.store.retryTimeline();
  }

  setBuckets(buckets: number): void {
    this.store.setBuckets(buckets);
  }

  showOnlyMarker(kind: string | null): void {
    this.store.patchQuery({ markers: kind ? [kind] : [] });
  }

  toggleGroup(key: string): void {
    this.store.toggleGroup(key);
  }
}
