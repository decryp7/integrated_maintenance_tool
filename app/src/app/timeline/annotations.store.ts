import { computed, effect, inject, Injectable, signal, untracked } from '@angular/core';
import { AnnotationDto, Query } from '../core/api.types';
import { ImtBackend } from '../core/backend';
import { describeError, Logger } from '../core/logger';
import { EMPTY_QUERY, WorkspaceStore } from '../core/workspace.store';
import { TimeCursor } from '../analysis/time-cursor';
import { ViewRequests } from '../core/view-requests';
import { ConsumerGate } from '../core/panel-visibility';
import { annotationKinds } from './annotation-layout';
import { TimelineZoom } from './zoom';
import { runKinds, runRange } from './run-layout';

/**
 * Plugin annotations for the visible time range. They are fetched again whenever the data changes (open,
 * reload, profile save, and each coalesced live-tail refresh, which bumps the data version) or the visible
 * range moves; responses to superseded requests are ignored. Kinds can be hidden from the legend. Runs between lifecycle
 * markers (annotations with `run`) are kept apart: they have their own legend toggles and click action. Nothing is
 * fetched while no view that shows them (`gate`: the timeline, the runs and report views) is open and visible; the
 * annotations catch up when one is shown again.
 */
@Injectable({ providedIn: 'root' })
export class TimelineAnnotations {
  private readonly backend = inject(ImtBackend);
  private readonly ws = inject(WorkspaceStore);
  private readonly logger = inject(Logger);
  private readonly zoom = inject(TimelineZoom);
  private readonly cursor = inject(TimeCursor);
  /** Asks for the events grid without depending on the dock. */
  private readonly views = inject(ViewRequests);

  readonly items = signal<AnnotationDto[]>([]);
  /** Why the last request failed (shown inline with Retry); null when fine. */
  readonly error = signal<string | null>(null);
  /** Kinds switched off in the legend. */
  readonly hidden = signal<ReadonlySet<string>>(new Set());
  /** Legend entries for the kinds present. */
  readonly kinds = computed(() => annotationKinds(this.plugin(), this.ws.markerKindMap()));

  /** Plugin annotations (everything but runs). */
  readonly plugin = computed(() => this.items().filter((a) => !a.run));
  /** Runs between lifecycle markers (`[[span]]`), drawn as bands behind the lane markers. */
  readonly runs = computed(() => this.items().filter((a) => !!a.run));
  /** Legend entries for the span kinds present. */
  readonly runKinds = computed(() => runKinds(this.runs()));
  /** The "Runs" legend toggle: draws every run kind or none. */
  readonly runsShown = signal(true);
  /** Span kinds switched off in the legend. */
  readonly hiddenRuns = signal<ReadonlySet<string>>(new Set());

  isRunKindShown(kind: string): boolean {
    return !this.hiddenRuns().has(kind);
  }

  toggleRuns(): void {
    this.runsShown.update((v) => !v);
  }

  /** Shows or hides one span kind. */
  toggleRunKind(kind: string): void {
    this.hiddenRuns.update((h) => {
      const next = new Set(h);
      if (!next.delete(kind)) next.add(kind);
      return next;
    });
  }

  /**
   * Filters to a run: its lane (and field value) over its time range, zooms there and selects its start event (its stop
   * for a stop without start) in the events grid.
   */
  showRun(a: AnnotationDto): void {
    const range = runRange(a);
    const query: Query = { ...EMPTY_QUERY, ...a.evidenceQuery, ...range, window: undefined };
    this.ws.patchQuery(query);
    if (this.ws.textInput() !== query.text) this.ws.setTextInput(query.text);
    this.zoom.apply(range);
    this.cursor.select(range);
    this.views.show('events');
    const event = a.run?.start ?? a.run?.stop;
    if (event) this.ws.jumpTo(event.id);
    this.logger.info('ui.timeline', 'Run shown', { Kind: a.kind, Status: a.run?.status ?? '' });
  }

  /** The views showing the annotations; nothing is fetched while none is visible. */
  readonly gate = new ConsumerGate();

  private seq = 0;
  private readonly retryNonce = signal(0);

  constructor() {
    effect(() => {
      if (!this.gate.active()) return;
      const t = this.ws.timeline();
      const version = this.ws.dataVersion();
      const hasData = !!this.ws.summary()?.sources.length;
      this.retryNonce();
      untracked(() => void this.load(hasData && t ? { from: t.from, to: t.to } : null, version));
    });
  }

  private lastKey = '';

  private async load(range: { from: number; to: number } | null, version: number): Promise<void> {
    const key = range ? `${range.from}:${range.to}:${version}:${this.retryNonce()}` : '';
    if (key === this.lastKey) return;
    this.lastKey = key;
    const mine = ++this.seq;
    if (!range) {
      this.items.set([]);
      this.error.set(null);
      return;
    }
    try {
      const items = await this.backend.annotations({ from: range.from, to: range.to });
      if (mine !== this.seq) return;
      this.items.set(Array.isArray(items) ? items : []);
      this.error.set(null);
    } catch (e) {
      if (mine !== this.seq) return;
      this.error.set(describeError(e));
      this.logger.warn('ui.timeline', 'Annotations could not be loaded', { Error: describeError(e) });
    }
  }

  retry(): void {
    this.retryNonce.update((n) => n + 1);
  }

  isShown(kind: string): boolean {
    return !this.hidden().has(kind);
  }

  /** Shows or hides one kind on the timeline. */
  toggle(kind: string): void {
    this.hidden.update((h) => {
      const next = new Set(h);
      if (!next.delete(kind)) next.add(kind);
      return next;
    });
  }

  /**
   * Applies an annotation's evidence: its query replaces the current one, the timeline zooms to the evidence
   * range and the events grid comes to the front.
   */
  showEvidence(a: AnnotationDto): void {
    const evidence = a.evidenceQuery;
    const from = evidence.from ?? a.from;
    const to = Math.max(evidence.to ?? a.to, from + 1);
    const query: Query = { ...EMPTY_QUERY, ...evidence, from, to, window: undefined };
    this.ws.patchQuery(query);
    if (this.ws.textInput() !== query.text) this.ws.setTextInput(query.text);
    this.zoom.apply({ from, to });
    this.cursor.select({ from, to });
    this.views.show('events');
    this.logger.info('ui.timeline', 'Annotation evidence shown', { Kind: a.kind, Owner: a.owner });
  }
}
