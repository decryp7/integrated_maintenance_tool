import { ChangeDetectionStrategy, Component, computed, effect, inject, input, output, signal, untracked } from '@angular/core';
import { Facet } from '../core/api.types';
import { ImtBackend } from '../core/backend';
import { formatCount } from '../core/format';
import { describeError } from '../core/logger';
import { WorkspaceStore } from '../core/workspace.store';
import { LiveFeed } from '../live/live-feed';
import { ClickOutside } from '../shared/click-outside';
import { Icon } from '../shared/icon';
import { VirtualList } from './virtual-list';

/** The engine returns at most this many values per facet (and reports the true distinct count). */
export const FACET_LIMIT = 1000;

export interface FacetFilterEvent {
  op: 'eq' | 'ne';
  value: string;
}

/**
 * Popover with the most frequent values of one field in the current results: search-as-you-type,
 * a virtualised list (a field can have thousands of values) and actions to filter, group, add a column or chart.
 * A failed request shows an inline error with Retry.
 */
@Component({
  selector: 'imt-facet-popover',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, VirtualList],
  // `closed` fires on a click outside or Escape.
  hostDirectives: [{ directive: ClickOutside, outputs: ['imtClickOutside: closed'] }],
  host: { class: 'popover facet', '[style.top.px]': 'top()', '[style.left.px]': 'left()' },
  template: `
    <div class="facet-head">
      <span class="mono">{{ field() }}</span>
      <span class="faint" data-testid="facet-summary">{{ summary() }}</span>
    </div>
    <div class="facet-actions">
      <button class="btn" (click)="groupBy.emit(false)" i18n="@@workspace.facet.groupBy">Group by</button>
      <button class="btn" (click)="groupBy.emit(true)" i18n="@@workspace.facet.addToGroup">Add to group</button>
      <button class="btn" (click)="addColumn.emit()" i18n="@@workspace.facet.addColumn">Add column</button>
      @if (numeric()) {
        <button class="btn" (click)="chart.emit()" i18n="@@workspace.facet.chart">Chart this field</button>
      }
    </div>
    @if (error(); as e) {
      <div class="err" role="alert">
        <span i18n="@@workspace.facet.loadFailed">Could not load the values: {{ e }}</span>
        <button class="btn" (click)="load()" i18n="@@workspace.facet.retry">Retry</button>
      </div>
    } @else if (!facet()) {
      <div class="faint" role="status" i18n="@@workspace.facet.loading">Loading…</div>
    } @else {
      <input class="input" type="search" placeholder="Search values" i18n-placeholder="@@workspace.facet.searchPlaceholder" aria-label="Search values" i18n-aria-label="@@workspace.facet.search" [value]="search()" (input)="search.set($any($event.target).value)" />
      @if (shown().length) {
        <imt-virtual-list [items]="shown()" [itemHeight]="26" [maxHeight]="260" label="Values" i18n-label="@@workspace.facet.values">
          <ng-template let-v>
            <div class="value">
              <span class="mono ellipsis" [title]="v.value">{{ v.value || emptyValue }}</span>
              <span class="bar"><span [style.width.%]="share(v.count)"></span></span>
              <span class="mono faint">{{ count(v.count) }}</span>
              <button class="icon-btn" title="Only this value" i18n-title="@@workspace.facet.only" [attr.aria-label]="filterLabel('eq', v.value)" (click)="filter.emit({ op: 'eq', value: v.value })"><imt-icon name="plus" [size]="12" /></button>
              <button class="icon-btn" title="Exclude this value" i18n-title="@@workspace.facet.exclude" [attr.aria-label]="filterLabel('ne', v.value)" (click)="filter.emit({ op: 'ne', value: v.value })"><imt-icon name="minus" [size]="12" /></button>
            </div>
          </ng-template>
        </imt-virtual-list>
      } @else {
        <div class="faint">@if (search()) { <ng-container i18n="@@workspace.facet.noMatch">No value matches the search.</ng-container> } @else { <ng-container i18n="@@workspace.facet.noValues">No values in the current results.</ng-container> }</div>
      }
    }
  `,
  styles: `
    :host { position: fixed; width: 340px; display: flex; flex-direction: column; gap: 8px; max-height: 460px; }
    .facet-head { display: flex; justify-content: space-between; gap: 8px; }
    .facet-actions { display: flex; gap: 6px; flex-wrap: wrap; }
    .err { display: flex; align-items: center; justify-content: space-between; gap: 8px; color: var(--danger-text); font-size: 12px; }
    .value { width: 100%; display: grid; grid-template-columns: minmax(0, 1fr) 60px 48px 22px 22px; align-items: center; gap: 6px; }
    .value .mono { font-size: 11px; }
    .value .icon-btn { width: 22px; height: 22px; }
    .bar { height: 4px; background: var(--border-2); border-radius: 2px; overflow: hidden; }
    .bar span { display: block; height: 4px; background: var(--accent); border-radius: 2px; }
  `,
})
export class FacetPopover {
  private readonly backend = inject(ImtBackend);
  private readonly store = inject(WorkspaceStore);

  readonly field = input.required<string>();
  readonly top = input(0);
  readonly left = input(0);
  readonly numeric = input(false);
  readonly groupBy = output<boolean>();
  readonly addColumn = output<void>();
  readonly chart = output<void>();
  readonly filter = output<FacetFilterEvent>();

  protected readonly count = formatCount;
  protected readonly emptyValue = $localize`:@@workspace.facet.empty:(empty)`;
  protected readonly facet = signal<Facet | null>(null);
  protected readonly error = signal<string | null>(null);
  protected readonly search = signal('');
  private seq = 0;

  protected readonly shown = computed(() => {
    const values = this.facet()?.values ?? [];
    const needle = this.search().trim().toLowerCase();
    return needle ? values.filter((v) => v.value.toLowerCase().includes(needle)) : values;
  });

  protected readonly summary = computed(() => {
    const f = this.facet();
    if (this.error()) return $localize`:@@workspace.facet.unavailable:unavailable`;
    if (!f) return '…';
    const loaded = f.values.length;
    const distinct = f.distinct >= 10000 ? '10,000+' : formatCount(f.distinct);
    const parts = [$localize`:@@workspace.facet.summary:${distinct}:count: values in current results`];
    if (this.search().trim()) parts.push($localize`:@@workspace.facet.matches:${formatCount(this.shown().length)}:count: match`);
    else if (loaded < f.distinct) parts.push($localize`:@@workspace.facet.topShown:top ${formatCount(loaded)}:count: shown`);
    return parts.join(' · ');
  });

  constructor() {
    effect(() => {
      this.field();
      this.store.effectiveQuery();
      untracked(() => void this.load());
    });
    // Live tail: counts refresh in place (throttled by the feed), without flashing back to "…".
    const feed = inject(LiveFeed);
    effect(() => {
      const version = feed.facetVersion();
      untracked(() => {
        if (version > 0 && this.facet()) void this.load(true);
      });
    });
  }

  protected filterLabel(op: 'eq' | 'ne', value: string): string {
    const field = this.field();
    return op === 'eq'
      ? $localize`:@@workspace.facet.filterEq:Filter ${field}:field: equals ${value}:value:`
      : $localize`:@@workspace.facet.filterNe:Filter ${field}:field: not equal ${value}:value:`;
  }

  protected share(n: number): number {
    const total = this.facet()?.total ?? 0;
    return total ? (n / total) * 100 : 0;
  }

  protected async load(quiet = false): Promise<void> {
    const mine = ++this.seq;
    this.error.set(null);
    if (!quiet) this.facet.set(null);
    try {
      const facet = await this.backend.queryFacet({ query: this.store.effectiveQuery(), field: this.field(), limit: FACET_LIMIT });
      if (mine === this.seq) this.facet.set(facet);
    } catch (e) {
      if (mine === this.seq) this.error.set(describeError(e));
    }
  }
}
