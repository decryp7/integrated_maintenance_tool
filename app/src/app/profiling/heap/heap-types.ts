import { formatExactBytes } from '../../shared/format/units';
import { ChangeDetectionStrategy, Component, computed, DestroyRef, effect, inject, input, linkedSignal, signal, untracked } from '@angular/core';
import { HeapSort, HeapTypeRow, HeapTypesPage } from '../../core/api.types';
import { formatCount } from '../../core/format';
import { AnalysisState } from '../../analysis/analysis-state';
import { AsyncResource } from '../../analysis/async-resource';
import { ProfilingBackend } from '../profiling-backend';
import { formatBytes, formatPct } from '../profiling-format';
import { PROFILING_STYLES } from '../profiling-styles';
import { HeapSelection } from './heap-selection';

const PAGE = 200;

/** Types of a snapshot by count, bytes or retained size; a type opens its instances. */
@Component({
  selector: 'imt-heap-types',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AnalysisState],
  template: `
    <div class="bar">
      <input type="search" placeholder="Search types…" i18n-placeholder="@@profiling.heapTypes.searchPlaceholder" aria-label="Search types" i18n-aria-label="@@profiling.heapTypes.searchLabel" (input)="onSearch($event)" />
      @if (res.data(); as d) { <span class="muted" i18n="@@profiling.heapTypes.summary">{{ count(d.totalRows) // i18n(ph: "types") }} types · {{ count(d.totalCount) // i18n(ph: "objects") }} objects · {{ bytes(d.totalBytes) // i18n(ph: "bytes") }}</span> }
    </div>
    <imt-analysis-state [loading]="res.loading()" [error]="res.error()" [retryable]="res.retryable()" [hasData]="!!res.data()" [empty]="res.data()?.totalRows === 0" emptyText="No types match." i18n-emptyText="@@profiling.heapTypes.empty" (retry)="res.retry()">
      <div class="body tight" tabindex="0" role="region" aria-label="Type rows" i18n-aria-label="@@profiling.heapTypes.rowsRegion">
        <table class="grid" aria-label="Types" i18n-aria-label="@@profiling.heapTypes.tableLabel">
          <thead>
            <tr>
              @for (c of cols; track c.id) {
                <th scope="col" [class.num]="c.id !== 'name'" [attr.aria-sort]="sort() === c.id ? (c.id === 'name' ? 'ascending' : 'descending') : null">
                  <button type="button" (click)="sort.set(c.id)" [disabled]="c.id === 'retained' && !res.data()?.hasGraph">{{ c.label }}{{ sort() === c.id ? (c.id === 'name' ? ' ▲' : ' ▼') : '' }}</button>
                </th>
              }
              <th scope="col" class="num" i18n="@@profiling.heapTypes.col.share">Share</th>
            </tr>
          </thead>
          <tbody>
            @for (r of rows(); track r.name) {
              <tr>
                <td class="ellipsis fn"><button class="link" type="button" [title]="instancesTitle(r.name)" (click)="sel.showInstances(r.name)">{{ r.name }}</button></td>
                <td class="num">{{ count(r.count) }}</td>
                <td class="num" [title]="exact(r.bytes)">{{ bytes(r.bytes) }}</td>
                <td class="num">{{ r.retained === null ? '—' : bytes(r.retained) }}</td>
                <td class="num"><span class="bar-cell"><span class="meter" aria-hidden="true"><span [style.width.%]="r.bytesPct"></span></span>{{ pct(r.bytesPct) }}</span></td>
              </tr>
            }
          </tbody>
        </table>
        @if (more() > 0) { <button class="btn more" type="button" (click)="loadMore()" i18n="@@profiling.heapTypes.loadMore">Load {{ more() // i18n(ph: "count") }} more</button> }
      </div>
    </imt-analysis-state>
  `,
  styles: [PROFILING_STYLES, `.tight { padding: 0; } .fn { max-width: 480px; } .more { margin: 8px 12px; }`],
})
export class HeapTypes {
  readonly snapshotId = input.required<number>();
  protected readonly sel = inject(HeapSelection);
  private readonly backend = inject(ProfilingBackend);
  protected readonly res = new AsyncResource<HeapTypesPage>();
  protected readonly sort = signal<HeapSort>('bytes');
  private readonly search = signal('');
  private readonly request = computed(() => ({ snapshotId: this.snapshotId(), sort: this.sort(), search: this.search() || null, offset: 0, limit: PAGE }));
  /** Rows paged in after the first page; a new request starts over. */
  private readonly extra = linkedSignal<HeapTypeRow[]>(() => {
    this.request();
    return [];
  });
  protected readonly cols: { id: HeapSort; label: string }[] = [
    { id: 'name', label: $localize`:@@profiling.heapTypes.col.type:Type` },
    { id: 'count', label: $localize`:@@profiling.heapTypes.col.count:Count` },
    { id: 'bytes', label: $localize`:@@profiling.heapTypes.col.bytes:Bytes` },
    { id: 'retained', label: $localize`:@@profiling.heapTypes.col.retained:Retained` },
  ];
  protected readonly count = formatCount;
  protected readonly bytes = formatBytes;
  protected readonly exact = formatExactBytes;
  protected readonly pct = formatPct;
  protected readonly rows = computed(() => [...(this.res.data()?.rows ?? []), ...this.extra()]);
  protected readonly more = computed(() => Math.max(0, (this.res.data()?.totalRows ?? 0) - this.rows().length));
  private timer: ReturnType<typeof setTimeout> | undefined;

  constructor() {
    effect(() => {
      const request = this.request();
      untracked(() => void this.res.load(() => this.backend.heapTypes(request)));
    });
    inject(DestroyRef).onDestroy(() => clearTimeout(this.timer));
  }

  protected instancesTitle(name: string): string {
    return $localize`:@@profiling.heapTypes.instancesOf:Instances of ${name}:name:`;
  }

  protected onSearch(e: Event): void {
    const v = (e.target as HTMLInputElement).value.trim();
    clearTimeout(this.timer);
    this.timer = setTimeout(() => this.search.set(v), 200);
  }

  protected async loadMore(): Promise<void> {
    const page = await this.backend.heapTypes({ snapshotId: this.snapshotId(), sort: this.sort(), search: this.search() || null, offset: this.rows().length, limit: PAGE }).catch(() => null);
    if (page) this.extra.update((x) => [...x, ...page.rows]);
  }
}
