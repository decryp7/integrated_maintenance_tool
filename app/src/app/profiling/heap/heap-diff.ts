import { ChangeDetectionStrategy, Component, computed, effect, inject, signal, untracked } from '@angular/core';
import { HeapDiffPage, HeapDiffSort, HeapSummary } from '../../core/api.types';
import { formatCount } from '../../core/format';
import { AnalysisState } from '../../analysis/analysis-state';
import { AsyncResource } from '../../analysis/async-resource';
import { CapturesStore } from '../captures.store';
import { ProfilingBackend } from '../profiling-backend';
import { formatBytes, formatDelta } from '../profiling-format';
import { PROFILING_STYLES } from '../profiling-styles';
import { HeapSelection } from './heap-selection';

/** Snapshots oldest first (by time taken, then id). */
export function byTaken(heaps: readonly HeapSummary[]): HeapSummary[] {
  return [...heaps].sort((a, b) => (a.takenMs ?? 0) - (b.takenMs ?? 0) || a.id - b.id);
}

/** Two snapshots compared per type, biggest growth first; new and gone types are flagged in words. */
@Component({
  selector: 'imt-heap-diff',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AnalysisState],
  template: `
    <div class="bar">
      <label class="inline">Before
        <select aria-label="Before snapshot" (change)="before.set(num($event))">
          @for (h of heaps(); track h.id) { <option [value]="h.id" [selected]="h.id === pair()?.before">{{ h.name }}</option> }
        </select>
      </label>
      <label class="inline">After
        <select aria-label="After snapshot" (change)="after.set(num($event))">
          @for (h of heaps(); track h.id) { <option [value]="h.id" [selected]="h.id === pair()?.after">{{ h.name }}</option> }
        </select>
      </label>
      <label class="inline">Sort
        <select aria-label="Sort by" (change)="sort.set($any($event.target).value)">
          <option value="bytesDelta" [selected]="sort() === 'bytesDelta'">Bytes growth</option>
          <option value="countDelta" [selected]="sort() === 'countDelta'">Count growth</option>
          <option value="name" [selected]="sort() === 'name'">Name</option>
        </select>
      </label>
      @if (res.data(); as d) { <span class="muted">{{ bytes(d.bytesBefore) }} → {{ bytes(d.bytesAfter) }} ({{ delta(d.bytesAfter - d.bytesBefore, 'bytes') }})</span> }
    </div>
    @if (heaps().length < 2) {
      <div class="state" role="status">Open a second heap snapshot to compare.</div>
    } @else {
      <imt-analysis-state [loading]="res.loading()" [error]="res.error()" [hasData]="!!res.data()" [empty]="res.data()?.totalRows === 0" (retry)="res.retry()">
        <div class="body tight" tabindex="0" role="region" aria-label="Comparison rows">
          <table class="grid" aria-label="Snapshot comparison, sorted by growth">
            <thead><tr><th scope="col">Type</th><th scope="col">Status</th><th scope="col" class="num">Count</th><th scope="col" class="num">Δ count</th><th scope="col" class="num">Bytes</th><th scope="col" class="num">Δ bytes</th></tr></thead>
            <tbody>
              @for (r of res.data()?.rows ?? []; track r.name) {
                <tr>
                  <td class="ellipsis fn"><button class="link" type="button" (click)="sel.showInstances(r.name)">{{ r.name }}</button></td>
                  <td>@if (r.isNew) { <span class="tag new">new</span> } @else if (r.isGone) { <span class="tag gone">gone</span> }</td>
                  <td class="num">{{ count(r.countBefore) }} → {{ count(r.countAfter) }}</td>
                  <td class="num" [class.up]="r.countDelta > 0" [class.down]="r.countDelta < 0">{{ delta(r.countDelta, 'count') }}</td>
                  <td class="num">{{ bytes(r.bytesBefore) }} → {{ bytes(r.bytesAfter) }}</td>
                  <td class="num" [class.up]="r.bytesDelta > 0" [class.down]="r.bytesDelta < 0"><span aria-hidden="true">{{ r.bytesDelta > 0 ? '▲' : r.bytesDelta < 0 ? '▼' : '' }}</span> {{ delta(r.bytesDelta, 'bytes') }}</td>
                </tr>
              }
            </tbody>
          </table>
        </div>
      </imt-analysis-state>
    }
  `,
  styles: [PROFILING_STYLES, `.tight { padding: 0; } .fn { max-width: 420px; } .state { padding: 24px 12px; color: var(--muted); text-align: center; }`],
})
export class HeapDiff {
  protected readonly sel = inject(HeapSelection);
  private readonly captures = inject(CapturesStore);
  private readonly backend = inject(ProfilingBackend);
  protected readonly res = new AsyncResource<HeapDiffPage>();
  protected readonly before = signal<number | null>(null);
  protected readonly after = signal<number | null>(null);
  protected readonly sort = signal<HeapDiffSort>('bytesDelta');
  protected readonly heaps = computed(() => byTaken(this.captures.heaps()));
  protected readonly count = formatCount;
  protected readonly bytes = formatBytes;
  protected readonly delta = formatDelta;

  /** Defaults: the two newest snapshots. */
  protected readonly pair = computed(() => {
    const hs = this.heaps();
    if (hs.length < 2) return null;
    const ids = hs.map((h) => h.id);
    const before = this.before() !== null && ids.includes(this.before()!) ? this.before()! : hs[hs.length - 2].id;
    const after = this.after() !== null && ids.includes(this.after()!) ? this.after()! : hs[hs.length - 1].id;
    return { before, after };
  });

  constructor() {
    effect(() => {
      const p = this.pair();
      const sort = this.sort();
      untracked(() => (p ? void this.res.load(() => this.backend.heapDiff({ baselineId: p.before, targetId: p.after, sort, limit: 500 })) : this.res.reset()));
    });
  }

  protected num(e: Event): number {
    return Number((e.target as HTMLSelectElement).value);
  }
}
