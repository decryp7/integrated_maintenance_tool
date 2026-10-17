import { computed, effect, inject, Injectable, signal, untracked } from '@angular/core';
import { AnnotationDto, Query } from '../core/api.types';
import { ImtBackend } from '../core/backend';
import { describeError, Logger } from '../core/logger';
import { EMPTY_QUERY, WorkspaceStore } from '../core/workspace.store';
import { TimeCursor } from '../analysis/time-cursor';
import { DockService, PANEL } from '../dock/dock.service';
import { annotationKinds } from './annotation-layout';
import { TimelineZoom } from './zoom';

/**
 * Plugin annotations for the visible time range. They are fetched again whenever the data changes (open,
 * reload, profile save, and each coalesced live-tail refresh, which bumps the data version) or the visible
 * range moves; responses to superseded requests are ignored. Kinds can be hidden from the legend.
 */
@Injectable({ providedIn: 'root' })
export class TimelineAnnotations {
  private readonly backend = inject(ImtBackend);
  private readonly ws = inject(WorkspaceStore);
  private readonly logger = inject(Logger);
  private readonly zoom = inject(TimelineZoom);
  private readonly cursor = inject(TimeCursor);
  private readonly dock = inject(DockService);

  readonly items = signal<AnnotationDto[]>([]);
  /** Why the last request failed (shown inline with Retry); null when fine. */
  readonly error = signal<string | null>(null);
  /** Kinds switched off in the legend. */
  readonly hidden = signal<ReadonlySet<string>>(new Set());
  /** Legend entries for the kinds present. */
  readonly kinds = computed(() => annotationKinds(this.items(), this.ws.markerKindMap()));

  private seq = 0;
  private readonly retryNonce = signal(0);

  constructor() {
    effect(() => {
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
    this.dock.show(PANEL.events);
    this.logger.info('ui.timeline', 'Annotation evidence shown', { Kind: a.kind, Owner: a.owner });
  }
}
