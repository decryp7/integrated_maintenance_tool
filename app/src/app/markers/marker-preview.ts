import { DestroyRef, effect, inject, Injectable, signal, Signal, untracked } from '@angular/core';
import { Query, RecordView } from '../core/api.types';
import { DebriefBackend } from '../core/backend';
import { Condition } from '../core/profile.model';
import { EMPTY_QUERY } from '../core/workspace.store';
import { asFilters } from '../context/rule-builders';

export const PREVIEW_SAMPLES = 20;
const STRIP_BUCKETS = 48;

export interface MarkerPreviewResult {
  count: number;
  samples: RecordView[];
  /** Matches per time bucket, for the mini strip. */
  strip: number[];
  /** Conditions the preview cannot express as a query filter (`in`). */
  ignored: number;
}

export interface PreviewSpec {
  conditions: readonly Condition[];
  /** Query-language expression; when set it replaces the conditions in the preview. */
  ql?: string;
}

/** Counts, samples and a time strip of the events matching a rule: read-only queries to the engine. */
@Injectable({ providedIn: 'root' })
export class MarkerPreviewService {
  private readonly backend = inject(DebriefBackend);

  queryFor(spec: PreviewSpec): Query {
    const ql = spec.ql?.trim();
    return ql ? { ...EMPTY_QUERY, ql } : { ...EMPTY_QUERY, filters: asFilters(spec.conditions) };
  }

  async preview(spec: PreviewSpec): Promise<MarkerPreviewResult> {
    const query = this.queryFor(spec);
    const [page, timeline] = await Promise.all([
      this.backend.queryPage({ query, offset: 0, limit: PREVIEW_SAMPLES, descending: false, anchor: null }),
      this.backend.queryTimeline({ query, buckets: STRIP_BUCKETS, alwaysShowLifecycle: false }),
    ]);
    const strip = timeline.histogram.map((b) => b.reduce((a, c) => a + c, 0));
    return { count: page.total, samples: page.rows, strip, ignored: spec.ql?.trim() ? 0 : spec.conditions.filter((c) => c.op === 'in').length };
  }
}

export interface PreviewState {
  result: MarkerPreviewResult | null;
  error: string | null;
  loading: boolean;
}

/** Debounced live preview signal for changing specs. Call in an injection context. */
export function livePreview(spec: () => PreviewSpec, delayMs = 250): Signal<PreviewState> {
  const service = inject(MarkerPreviewService);
  const out = signal<PreviewState>({ result: null, error: null, loading: true });
  let seq = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  effect(() => {
    const current = spec();
    clearTimeout(timer);
    const mine = ++seq;
    untracked(() => out.update((s) => ({ ...s, loading: true })));
    timer = setTimeout(async () => {
      try {
        const result = await service.preview(current);
        if (mine === seq) out.set({ result, error: null, loading: false });
      } catch (e) {
        if (mine === seq) out.set({ result: null, error: e instanceof Error ? e.message : String(e), loading: false });
      }
    }, delayMs);
  });
  inject(DestroyRef).onDestroy(() => clearTimeout(timer));
  return out;
}
